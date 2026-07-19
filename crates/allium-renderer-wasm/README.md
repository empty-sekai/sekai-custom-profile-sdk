# @empty-sekai/renderer-wasm

[简体中文](README.md) | [English](README.en.md)

`@empty-sekai/renderer-wasm` 是面向 Project SEKAI（PJSK）自定义名片场景的 WebGL2 browser runtime。

TMP 富文本与排版针对 PJSK 使用的 Unity TextMesh Pro 数据和行为提供兼容模型，覆盖当前已经建模和验证的标签、排版、材质与动态语义。尚未建模的游戏行为或后续游戏更新可能与客户端不同，因此不承诺完整复刻游戏的渲染逻辑或最终像素。

Rust/WASM 负责 profile resolution、TMP 富文本、layout、动态公式、稳定语义 ID、glyph demand、FreeType 字体度量、glyph SDF 与 atlas placement；TypeScript 负责 worker、异步资源调度、缓存 I/O 和 GPU resource orchestration；WebGL2 消费 semantic command stream 和紧凑状态表完成绘制。

0.2 提供 stateful scene API，执行路径由 Rust/WASM semantic runtime、dedicated worker、FreeType/SDF atlas 与 WebGL2 renderer 组成。

## 运行要求

- WebGL2；
- ES modules 与 Web Workers；
- 可选 IndexedDB，用于持久化不透明 glyph record；
- 调用方提供的 profile/card JSON、masterdata、字体 provider 或预注册字体，以及 `ResourceProvider`。

字体、玩家数据、masterdata 和图片素材由调用方提供。字体度量、富文本 layout 与 glyph 像素由调用方提供并锁定 source hash 的字体 bytes、FreeType、TMP parser 和 SDF pipeline 共同确定。

## 安装

```sh
npm install @empty-sekai/renderer-wasm
```

默认情况下，worker、Emscripten glue 和 WASM 文件从 package entry 相邻的 `dist/` 路径加载。若 bundler 或部署结构不同，可显式传入 `workerUrl`、`moduleUrl` 和 `wasmUrl`。

## 最小接入

```ts
import {
  BrowserRenderer,
  type FontProvider,
  type ResourceProvider,
} from "@empty-sekai/renderer-wasm";

const fontProvider: FontProvider = {
  async provide({ region, family }, { signal }) {
    const bytes = await loadApplicationFont({ region, family, signal });
    return bytes ? { bytes } : null;
  },
};

const resourceProvider: ResourceProvider = {
  async provide(descriptor, { signal }) {
    const request = await resolveResourceRequest(descriptor);
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

// 始终导出游戏名片原始尺寸 1830×812；内部会先绘制并同步复制当前帧。
const png = await scene.exportPng();
```

## 自定义名片编辑文档

`BrowserAuthoringClient` 在 dedicated worker 中持有游戏兼容的编辑文档。导入完整公开 Profile 时只提取 `userCustomProfileCards`；导出不包含 worker handle、稳定元素 ID、选中状态或其他浏览器元数据。

```ts
import { BrowserAuthoringClient, type AuthoringCommand } from "@empty-sekai/renderer-wasm";

const authoring = await BrowserAuthoringClient.create();
const document = profile
  ? await authoring.importProfile(profile)
  : await authoring.createBlank();

const command: AuthoringCommand = {
  kind: "set_transform",
  id: selectedElementId,
  position: [120, -40, 0],
};
const delta = await document.apply(command);

await document.undo();
await document.redo();
const gameDocument = await document.export();

await document.destroy();
authoring.destroy();
```

编辑核心最多保留 150 个历史事务，并校验每页 150 个元素的上限、12 个游戏数组、有限数值、图层顺序以及 `objectData` 边界。创建、复制、删除、变换、锁定、可见性、参数和图层命令都返回增量变化；调用方应把这些变化用于现有 scene 和界面状态，而不是在 TypeScript 中维护第二套命令历史。

## 完整执行流程

`createProfileScene()` 会依次完成以下语义解析、资源准备与 GPU 初始化流程：

1. worker 向 WASM 发送 card、profile、locale 与 masterdata session；
2. shared semantic core 收集本场景的 localization demand，调用方 provider 返回不可变文本快照；
3. WASM 用该文本快照解析 authored elements 与 components，并输出实际使用的 font family demand；
4. 主线程通过有界队列调用可选 `FontProvider`，对返回 bytes 计算 hash，并把 family→hash 锁定在 renderer 生命周期内；调用方也可以在创建 scene 前直接 `registerFont()`；
5. WASM 输出完整 glyph demand、layout request 与稳定 resource descriptor；
6. 主线程按 descriptor stable ID 去重，并通过有界队列调用 `ResourceProvider`；
7. WASM 使用已注册字体执行 FreeType measurement、TMP layout、glyph SDF generation 和 atlas placement；
8. core 编译 authored layer tree、semantic commands、control bindings、interaction regions 与初始动态状态；
9. WebGL2 上传 atlas pages、decoded images、geometry、command state 与 layer mask buffer；
10. scene 返回后，调用方决定何时 `draw()`、推进 timeline、切换 layer、处理交互或导出 dump。

glyph atlas 按实际 glyph demand 创建。单个图片资源缺失、provider 抛错或图片解码失败会产生 warning，并使用透明占位继续构建其余 scene；schema、ABI、内存预算和 scene contract 错误会显式失败。

## ResourceProvider

renderer 输出并消费语义 descriptor；调用方负责把 descriptor 映射到 URL、CDN、对象存储、文件、manifest、鉴权请求或其他资源系统。

```ts
export type ResourceDescriptor = {
  id: string;
  namespace: string;
  key: string;
  role: string;
  provenance: Record<string, unknown>;
  expectedSize?: { width: number; height: number };
};

export interface ResourceProvider {
  provide(
    descriptor: ResourceDescriptor,
    context: { signal: AbortSignal },
  ): Promise<{
    source: Blob | ArrayBuffer | Uint8Array | TexImageSource;
  } | null>;

  cacheIdentity?(descriptor: ResourceDescriptor): string | null;
}
```

`namespace`、`key` 和 `role` 表达 renderer 语义；调用方可将其映射到任意资源命名和存储规则。

### 任意异步来源

一个 provider 可以独立组合任意资源规则：

```ts
const provider: ResourceProvider = {
  cacheIdentity(descriptor) {
    return `${assetRevision}:${descriptor.id}`;
  },

  async provide(descriptor, { signal }) {
    const memoryHit = memoryImages.get(descriptor.id);
    if (memoryHit) return { source: memoryHit };

    const stored = await assetDatabase.get(assetRevision, descriptor.id);
    if (stored) return { source: stored };

    const request = await applicationAssetRouter.resolve(descriptor);
    if (!request) return null;

    const response = await authenticatedFetch(request, { signal });
    if (!response.ok) return null;

    const blob = await response.blob();
    await assetDatabase.put(assetRevision, descriptor.id, blob);
    return { source: blob };
  },
};
```

同一个 provider 可以从本地静态文件、HTTP cache、Cache Storage、IndexedDB、用户选择的文件、Service Worker、签名 URL、GraphQL、私有鉴权接口或已经解码的 `ImageBitmap` 返回资源。路径解释与失效策略始终由 provider 掌握。

### 并发、去重与失败语义

- `resourceConcurrency` 默认为 8，必须是正整数；
- descriptor 在单次 acquire 中按 stable `id` 去重；
- 同一 renderer 内的并发 scene 请求共享 in-flight singleflight；
- 每个等待者独立响应自己的取消信号；仅当全部等待者都取消时才取消底层 provider 请求；
- decoded cache hit 直接复用 session lease，provider 并发槽继续服务实际加载请求；
- `AbortSignal` 会传给 provider；
- provider 返回 `null`、抛错或图片解码失败时，该项变为透明占位并记录 warning；
- pinned lease 耗尽 decoded image hard budget 时会产生显式资源错误。

`cacheIdentity()` 用于声明 decoded session cache 的身份。它应包含会改变图片内容的 catalog revision、region、用户或鉴权域；若省略则使用 descriptor stable ID。encoded asset 是否持久化、保存多久以及如何失效，完全由 provider 决定。

## LocalizationProvider

General 标题、tab、难度名等 renderer-owned UI 文本通过稳定 localization key 请求；玩家名称、签名和其他用户内容保持 profile 原文。调用方可以用任意规则解析 `region + locale + key`：

```ts
const renderer = await BrowserRenderer.create({
  canvas,
  region: "tw",
  resourceProvider,
  localizationProvider: {
    async provide({ region, locale, key }, { signal }) {
      return applicationMessages.resolve({ region, locale, key, signal });
    },
  },
  localizationConcurrency: 8,
});
```

WASM 先收集当前 scene 实际使用且已去重的 key，主线程有界并发调用 provider，并把完整 `key → UTF-8 value` 作为不可变快照送回 WASM。缺少必需 key 时 scene 创建显式失败，不跨区服猜测文案。未提供 `LocalizationProvider` 时使用 masterdata/runtime 自带的本地化来源。

## Masterdata

`loadMasterData()` 接受调用方实现的任意异步 table loader。调用方完整定义 table 的命名、位置、传输、鉴权与缓存方式：

```ts
const masterData = await renderer.loadMasterData(
  "catalog-2026-07",
  async ({ table, region, revision }, { signal }) => {
    const request = await applicationMasterDataRouter.resolve({ table, region, revision });
    return applicationMasterDataLoader.load(request, { signal });
  },
  { concurrency: 4 },
);
```

需要逐表控制生命周期时，可直接驱动 session：

```ts
const masterData = await renderer.createMasterData("catalog-2026-07");

for (const table of masterData.requiredTables) {
  await masterData.putTable(table, await applicationMasterData.load(table));
}

await masterData.seal();
```

`seal()` 后该 session 才能创建 scene。销毁 renderer 前应销毁不再使用的 masterdata session。

## 字体

字体 bytes 始终由调用方提供。`family` 是 masterdata 和文本 layer 使用的 opaque 逻辑身份，并原样传给调用方的字体解析规则。

推荐在 `BrowserRenderer.create()` 中提供 `FontProvider`。WASM 只请求当前 scene 实际使用的 family，主线程按 `fontConcurrency`（默认 4）有界并发调用 provider：

```ts
const renderer = await BrowserRenderer.create({
  canvas,
  region: "cn",
  resourceProvider,
  fontProvider: {
    async provide({ region, family }, { signal }) {
      const bytes = await applicationFonts.resolve({ region, family, signal });
      return bytes ? { bytes } : null;
    },
  },
  fontConcurrency: 4,
});
```

provider 可以使用用户选择的文件、内存、IndexedDB、应用资源包、网络请求或任意其他实现。返回值会复制成该次解析的不可变 byte snapshot；缺少当前 scene 必需的 family 时 scene 创建显式失败。

需要完全手动管理时，也可以在创建 scene 前直接注册：

```ts
const bytes = await file.arrayBuffer();

await renderer.registerFont({ family: "FOT-RodinNTLGPro-DB", bytes });
await renderer.registerFont({ family: "Application Alias", bytes });
```

同一字体文件可以注册多个逻辑 alias。renderer 会对 source bytes 计算 hash，并把 region、family、source hash 和字符共同纳入 glyph identity。

同一 renderer 生命周期内，首次成功注册会锁定一个 family 对应的 source hash。用相同 bytes 重复注册是幂等操作；用不同 bytes 覆盖同一 family 会返回 `FONT_IDENTITY_CONFLICT`。需要切换字体版本时应创建新的 renderer，使 font snapshot、glyph identity、atlas 和持久缓存边界保持明确。

## 可选预生成 Atlas 包（0.2.1）

默认仍按 scene 的实际 glyph demand 生成 SDF，并使用同源 IndexedDB glyph cache。预生成 atlas 不会自动下载。调用方可以直接使用本地/HTTP provider，也可以通过显式用户操作把完整 atlas 安装到同源 IndexedDB，随后让多个 renderer、编辑器和查看页共享同一份安装结果。

```ts
import {
  BrowserRenderer,
  createHttpPrebuiltSdfAtlasProvider,
  createOriginPrebuiltSdfAtlasPackage,
} from "@empty-sekai/renderer-wasm";

const source = createHttpPrebuiltSdfAtlasProvider("/font-atlases");
const atlasPackage = createOriginPrebuiltSdfAtlasPackage({
  namespace: "cn-6.0.0-font-atlas-v1",
  source,
});

// 只应从明确的用户操作调用。manifest 在全部页写入成功后才可见。
await atlasPackage.install([
  "FZLanTingHei-DB-GBK",
  "FZZhengHei-EB-GBK",
  "FZShaoEr-M11-JF",
], {
  concurrency: 4,
  requestPersistence: true,
  onProgress(progress) {
    updateDownloadProgress(progress.completedPages / progress.totalPages);
  },
});

const renderer = await BrowserRenderer.create({
  canvas,
  region: "cn",
  resourceProvider,
  fontProvider,
  prebuiltSdfAtlasProvider: atlasPackage.provider,
});
```

`atlasPackage.provider` 只读取已经完整安装的 family；未安装、安装中、IndexedDB 不可用或已移除时，manifest 返回 `null`，scene 自动回到按需 glyph 生成。安装前会检查浏览器报告的剩余 quota，每个页校验 SHA-256，失败时清理本次未完成 family。`requestPersistence` 默认关闭；renderer 不会在没有用户操作时请求持久存储权限。atlas package 不保存 profile、文本、用户 ID、layout 或 scene dump。

不需要浏览器持久化时，可以把 `createHttpPrebuiltSdfAtlasProvider()` 的结果直接传给 `BrowserRenderer.create()`，由应用自己的本地文件、Service Worker 或 HTTP cache 管理生命周期。

## Scene 状态

```ts
await scene.advance(tick);
await scene.setLayerVisible(layerId, false);
await scene.setLayerMasks(layerTableRevision, [
  { layerId, visible: true },
]);
await scene.setTab(controlId, value);
await scene.setScrollOffset(controlId, offset);
await scene.scrollBy(controlId, delta);
scene.draw();
```

这些 mutation 复用 authored layer table、timeline、layout、glyph atlas 与 persistent glyph cache，只更新 WASM scene state 和紧凑 GPU buffer。layer 显隐作用于 render mask，并保持当前动态播放位置。

公开 layer 对应游戏名片 authored element。shape、glyph、mask 和其他 draw primitive 作为 command 归属于各自 authored layer。

## Dump、分层与交互

```ts
const dump = await scene.dump();
```

dump 包含：

- stable layer/glyph/command/control/region ID；
- layer table 与 parent tree；
- source content 和 resolved parameters；
- authored visibility、render mask 与动态状态；
- bounds、quad、matrix、clip 与 hit geometry；
- semantic commands 与 component controls；
- interaction regions 和剥离 TMP 后识别的连续数字区域。

滚动组件分别暴露固定 viewport、随 offset 移动的 content 和随比例移动的 thumb region。调用方可在 viewport/content/thumb 上处理滚轮，并根据 thumb 拖动结果调用 `setScrollOffset()`；renderer 负责 clamp、状态 patch 与命中几何更新，不接管指针行为。

产品交互由调用方实现；renderer 提供 `capabilities`、`resolved_data`、control binding 和 geometry，用于构建：

- 称号、角色、卡面、活动、歌曲或剧情资料跳转；
- tab 切换与滚动；
- 数字复制；
- hover card、selection、editing；
- DOM/SVG accessibility overlay。

动画过程中保持稳定的 layer tree DOM 与 layer row，并按 tick 更新画布、overlay 和必要的动态详情。

## 缓存模型

| 层级 | 生命周期 | 所有者 | 内容 |
| --- | --- | --- | --- |
| provider persistent cache | 由应用决定 | 调用方 | encoded image 或应用自定义记录 |
| decoded image cache | renderer session | TypeScript runtime | 有界 decoded `TexImageSource` lease |
| glyph persistent cache | browser origin | renderer | 不透明、版本校验的 glyph record |
| glyph session atlas | worker session | WASM | atlas pages、placement、lease 与 revision |
| GPU texture/buffer | WebGL context | WebGL2 runtime | atlas、image、geometry 与状态 buffer |

`sdf.persistence` 默认为 `"origin"`，使用 IndexedDB；`"memory-only"` 保留 session cache。IndexedDB record 由 opaque glyph identity、版本信息、metrics 与 R8 SDF payload 组成。

```ts
const scene = await renderer.createProfileScene({
  masterData,
  documentKey: "preview",
  card,
  sdf: { persistence: "memory-only" },
});
```

WebGL context restore 会重传保留的 atlas/image/buffer state，并复用已完成的 TMP parsing、layout 与 SDF generation 结果。

## Debug 与 telemetry

```ts
const renderer = await BrowserRenderer.create({
  canvas,
  region: "en",
  resourceProvider,
  telemetry: { level: "trace", maxSamples: 240 },
});

const sceneStats = scene.stats();
const rendererStats = await renderer.stats();
```

telemetry 覆盖 worker request；字体 provider 的请求、并发峰值、bytes、失败和耗时；本地化 provider 的请求、解析数、并发峰值、失败和耗时；resource provider 的实际调用数、并发峰值、已知 encoded bytes、失败、取消与累计解析耗时，并同时报告 decoded session cache 的 entries、bytes、pin、hit、load 和 eviction。它还覆盖 glyph generation、persistent cache hit/miss、atlas pages、texture uploads、GPU buffers、frame timing 与 context recovery。`summary` 保留聚合值；`trace` 在固定上限内保留 raw samples；`off` 仅提供即时运行状态。

telemetry 采用 privacy-safe schema，仅记录运行时计数、耗时、容量与恢复状态。GPU timing 在 unavailable 或 disjoint 状态下使用 `null` 表达测量状态。

## 生命周期与取消

```ts
const controller = new AbortController();

const pendingScene = renderer.createProfileScene({
  masterData,
  documentKey: "preview",
  card,
  signal: controller.signal,
});

controller.abort();

await pendingScene; // rejects if creation was still in progress

const scene = await renderer.createProfileScene({ masterData, documentKey: "preview", card });
await scene.destroy();
await masterData.destroy();
renderer.destroy();
```

`signal` 只取消仍在进行的 scene creation；已创建 scene 通过 `destroy()` 释放。销毁 scene 会释放图片 lease、atlas lease、GPU texture 与 core scene。`renderer.destroy()` 会终止 worker，并取消仍在进行的资源获取。

## 从源码构建

使用仓库提供的 container toolchain；它固定 FreeType、Emscripten 与 Rust target 环境。

```sh
npm run build
npm run typecheck
npm run test:gates
npm run verify:wasm:runtime
npm run measure:wasm:size
npm run audit:public
```

FreeType 只启用 runtime 所需的 TrueType、CFF、SFNT、PS auxiliary/name 和 smooth raster modules。CPU EDT 是 production glyph SDF backend；analytic backend 仅用于显式 debug 与 parity 分析。

## License

AGPL-3.0-only，并附带 `LICENSE-EXCEPTION` 中的 browser linking exception。exception 适用于未修改 package 的浏览器使用；修改 renderer、服务端使用和非浏览器使用仍受完整 AGPL 约束，包括网络交互场景下的源代码提供义务。
