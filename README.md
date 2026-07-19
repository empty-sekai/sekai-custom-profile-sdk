# Sekai Custom Profile SDK

[简体中文](README.md) | [English](README.en.md)

Sekai Custom Profile SDK 是面向 Project SEKAI（PJSK）自定义名片的非官方开发工具包。它提供游戏结构兼容的编辑文档、事务与导出，以及 native/browser 渲染、动态场景、交互区域、字体 atlas 和资源 provider。

富文本与排版实现针对 PJSK 使用的 Unity TextMesh Pro 数据、标签和运行时行为建立兼容模型，覆盖本仓库已经建模和验证的标签、排版、材质与动态语义。尚未建模的游戏行为或后续游戏更新可能与客户端不同，因此不承诺完整复刻游戏的渲染逻辑或最终像素。

renderer 是 SDK 内的一项能力。仓库包含两个面向不同运行环境的渲染适配器：

- native CPU/Skia renderer，适合服务端、CLI 与离线任务；
- browser WebGL2 renderer，使用 Rust/WASM、FreeType 与 SDF atlas 在浏览器中绘制和更新场景。

两者共享与后端无关的 Rust semantic core。浏览器包采用独立的 Rust/WASM + WebGL2 执行路径，native adapter 采用 Rust + Skia 执行路径。

## 设计边界

Rust/WASM 负责：

- 游戏结构兼容的 authoring document、事务、手势、undo/redo、checkpoint 与导出；
- 玩家名片与 masterdata 解析；
- TMP 富文本解析与排版；
- 动态公式和可变场景状态；
- FreeType 字体度量、glyph SDF 生成与 atlas 放置；
- 稳定的 layer、command、control 与 interaction ID；
- layer tree、source content、resolved parameters、bounds、quad、matrix 和 hit geometry。

浏览器主线程负责：

- 调用应用提供的异步字体、本地化与图片资源 provider；
- 对资源请求去重、限流和复用 decoded image；
- 把 atlas、command buffer 与紧凑状态表上传到 WebGL2；
- 在 context restore 后恢复 GPU 资源。

调用方负责：

- 提供 profile、字体、masterdata 和图片素材；
- 决定资源来自网络、本地文件、IndexedDB、签名请求、鉴权接口或其他实现；
- 实现 hover、click、selection、editing、复制、跳转和 DOM/SVG overlay。

字体度量、富文本解析和 glyph 像素由调用方提供并锁定 source hash 的字体 bytes、FreeType、TMP layout 与 SDF pipeline 共同确定。

## Workspace

| Crate | 职责 |
| --- | --- |
| `allium-renderer-core` | 后端无关的场景 schema、profile resolution、动态、稳定 ID、mask、control 与 interaction geometry |
| `allium-renderer` | native CPU/Skia adapter 与可复用 native renderer 组件 |
| `allium-renderer-host` | native host 工具和 JSON masterdata provider |
| `allium-renderer-cli` | `render-card` CLI 与 NDJSON 常驻服务模式 |
| `allium-renderer-wasm` | minimal FreeType WASM、stateful worker protocol、WebGL2 runtime、缓存与 Scene Workbench |

## 浏览器快速开始

0.3 的 browser SDK 以 `BrowserAuthoringClient` 和 `BrowserRenderer` 为入口。调用方通过 authoring API 编辑游戏结构文档，并通过必填的 `ResourceProvider` 用任意异步规则解释 SDK 提供的语义 descriptor。

```ts
import {
  BrowserRenderer,
  type FontProvider,
  type ResourceProvider,
} from "@empty-sekai/sekai-custom-profile-sdk";

const fontProvider: FontProvider = {
  async provide({ region, family }, { signal }) {
    const bytes = await loadApplicationFont({ region, family, signal });
    return bytes ? { bytes } : null;
  },
};

const resourceProvider: ResourceProvider = {
  cacheIdentity(descriptor) {
    return `catalog-2026-07:${descriptor.id}`;
  },

  async provide(descriptor, { signal }) {
    const request = await resolveApplicationResource(descriptor);
    if (!request) return null;

    const response = await fetch(request, { signal, cache: "default" });
    if (!response.ok) return null;
    return { source: await response.blob() };
  },
};

const renderer = await BrowserRenderer.create({
  canvas: document.querySelector("canvas")!,
  region: "en",
  resourceProvider,
  fontProvider,
});

const masterData = await renderer.loadMasterData(
  "latest",
  ({ table, region, revision }, { signal }) =>
    loadApplicationMasterData({ table, region, revision, signal }),
);
const scene = await renderer.createProfileScene({
  masterData,
  documentKey: "profile-preview",
  card,
  profile,
  frameMode: "animate",
});

scene.draw();
```

完整的接入流程、provider contract、缓存与交互说明见 [browser package README](crates/allium-renderer-wasm/README.md)。

## 浏览器中实际发生什么

创建一个 profile scene 时，runtime 按以下顺序工作：

1. worker 中的 WASM 读取 profile、card 与 masterdata，依次生成本场景 localization、font、glyph、layout 和 resource demand；
2. TypeScript 通过调用方的有界异步 provider 返回不可变本地化文本、字体 bytes 和图片资源快照；字体也可由调用方提前直接注册；
3. `ResourceProvider` 返回 `Blob`、`ArrayBuffer`、`Uint8Array` 或可直接上传的 `TexImageSource`；
4. WASM 根据已锁定 source hash 的字体完成 FreeType 度量、TMP layout、glyph SDF 与 atlas placement；
5. semantic core 生成 authored layer tree、commands、controls、interaction regions 和动态状态；
6. WebGL2 上传 atlas、图片、geometry 与状态 buffer，并绘制场景；
7. `advance()`、layer mask、tab 和 scroll 复用既有 timeline、layout 与 atlas，只增量修改相关状态；
8. `dump()` 与 `stats()` 提供可检查的语义数据和有界 telemetry。

单项图片资源缺失或解码失败会产生 warning，并使用透明占位继续构建场景。schema、ABI、内存预算等内部契约错误仍会显式失败。

## 状态、分层与交互

公开 layer 与游戏名片中的 authored element 一一对应；shape、glyph、mask 等 draw primitive 作为所属 layer 的 command 暴露。

```ts
await scene.advance(tick);
await scene.setLayerVisible(layerId, false);
await scene.setLayerMasks(layerTableRevision, overrides);
await scene.setTab(controlId, value);
await scene.scrollBy(controlId, delta);
scene.draw();

const dump = await scene.dump();
```

layer 显隐复用当前动态 timeline、layout cache、glyph atlas 与图片缓存。renderer 提供区域、control binding、resolved data 和 geometry；应用据此决定点击后打开角色、活动、卡面、剧情或其他资料页面。

## Native renderer

native adapter 用于服务端静态图片、CLI 和应用自定义 scene。资源字节与 masterdata 始终由 host 提供；`skia` feature 启用 native production preset。

```sh
cargo test --workspace --all-features
cargo run --release --bin render-card -- \
  --masterdata ./masterdata \
  --card ./card.json \
  --profile ./profile.json \
  --assets-dir ./assets \
  --font-dir ./fonts \
  --format png \
  -o output.png
```

## 缓存与资源所有权

- 字体 bytes 与逻辑 family 由调用方直接注册或通过任意异步 `FontProvider` 提供，并作为字体解析和 glyph cache identity 的唯一来源；
- glyph session atlas 在同一 worker 内跨 scene 复用，并受页数与内存预算约束；
- 可选 IndexedDB glyph cache 只保存不透明、带版本校验的 glyph record；
- encoded image 的持久化完全属于 `ResourceProvider`，可使用浏览器 HTTP cache、Cache Storage、IndexedDB 或应用自己的存储；
- renderer 内部只保留有界的 decoded image session cache 与 context-local GPU texture；
- WebGL context restore 直接重传保留的 atlas、image 和 buffer 数据，继续复用已完成的 TMP layout 与 SDF 结果。

## Debug 与 telemetry

`scene.dump()` 包含 authored layer tree、source content、resolved parameters、bounds、quad、matrix、hit geometry、commands、masks、controls 和 interaction regions。

`scene.stats()` 与 `renderer.stats()` 提供 worker、glyph、cache、atlas、texture、buffer、frame timing 和 context recovery 指标。telemetry 采用固定保留上限和 privacy-safe schema，只记录运行时计数、耗时与资源规模。

## 构建与验证

项目使用固定的 container toolchain 构建 FreeType、Skia 与 Emscripten 目标。

```sh
cargo test --workspace --all-features

cd crates/allium-renderer-wasm
npm run build
npm run typecheck
npm run test:gates
npm run verify:wasm:runtime
npm run measure:wasm:size
```

release gates 覆盖 ABI/schema、shared-core source consistency、TMP debug parity、glyph SDF parity、完整 profile command coverage、atlas/cache budget、worker singleflight、context restoration、bounded telemetry、npm contents 与 native/browser release matrix。

## License

仓库采用 AGPL-3.0-only。browser npm package 另含 `crates/allium-renderer-wasm/LICENSE-EXCEPTION` 中的有限 browser linking exception。修改 SDK、服务端使用和非浏览器使用仍受完整 AGPL 约束，包括网络交互场景下的源代码提供义务。
