# allium-renderer

自定义名片的渲染层。输入玩家 profile JSON，输出名片图片（JPEG/WebP）。

**在线 Demo**：<https://empty-sekai.github.io/allium-renderer/>（浏览器 wasm 分层预览，资源自备）

## 仓库结构

Cargo workspace，四个 crate：

| Crate | 职责 |
| --- | --- |
| `crates/allium-renderer` | 渲染引擎（本文档主体） |
| `crates/allium-renderer-host` | CLI / wasm 共享宿主层：`JsonMasterDataProvider` |
| `crates/allium-renderer-cli` | `render-card` 二进制：单次渲染 + `--serve` NDJSON 常驻模式 |
| `crates/allium-renderer-wasm` | 浏览器 wasm 导出层（emscripten C ABI） |

## 渲染路径

两条路径共享部分底层模块（文本 SDF、素材解码、图形绘制）：

- **名片渲染**：Profile JSON → `CustomProfileCard` → `flatten_and_sort()` 按 layer 排序提取 `RenderElement` 数组（文本、形状、卡牌成员、图章、称号等 12 种元素类型），逐元素做 `canvas.translate/rotate/scale` 后分发到对应绘制函数（`draw_text` / `draw_shape` / widget 绘制）。
- **图元组合**：应用自身设计的场景（排行榜预览、卡牌缩略图、采集地图），通过 `Renderable::compose()` 产出 `SceneTree`（纯数据，无 Skia 依赖），再由执行器走 Skia 光栅化并编码。`compose` 阶段可脱离 Skia 单独测试。

## 文本渲染

名片文本还原游戏 TextMeshPro 的 SDF 渲染效果：

- 逐字形通过 FreeType 提取 glyph metrics（与 TMP FontEngine 同源），生成 SDF 轮廓纹理
- 解析 TMP 富文本标签：`<size>` `<scale>` `<rotate>` `<pos>` `<voffset>` `<color>` `<cspace>` `<line-height>` `<b>` `<i>`，应用至逐字排版与光栅化
- SDF 光栅化为逐像素软件路径：从完整 CTM 取逆矩阵，设备像素反变换到 local 坐标 → 距离场双线性采样 → face/underlay 合成 → 超采样累加。按像素行切分到专用 rayon 线程池并行执行，结果以 identity 矩阵贴回设备表面

## 入口

`CustomProfileRenderer` 是名片渲染的高层 API：

- `render_page(&self, card)` / `render_page_with_profile(...)` → 白底 JPEG 字节
- `render_page_png_transparent_with_profile(...)` → 透明底 PNG 字节
- `render_by_seq(&self, cards, page)` → 按 seq 选页渲染

底层 `render_document::render_document()` 接收 `WidgetDocument` + `RenderContext`，纯同步，不做调度，由调用方决定线程与并发策略。

## 两级线程模型

- **外层（串行）**：单个渲染任务全程单线程，一次画一张图，控制内存峰值与调度公平性。由调用方提供，不在本 crate 内。
- **内层（并行）**：`sdf` 模块将字形轮廓光栅化为像素位图时，各行独立只读共享状态，切分到专用 rayon 线程池（池内线程命名 `raster-*`）并行执行。

专用池不复用 rayon 全局池，线程数由 `ALLIUM_RASTER_THREADS` 控制（默认 2）。外层保证至多一个渲染在跑，池进程内全局共享。设为 1 时并行迭代退化为串行，无需单独代码路径。

> 默认配置为 1 个渲染线程 + 2 个光栅化线程。逐像素数学与串行版逐字节一致，输出不随线程数变化。

## 模块

| 模块 | 职责 |
| --- | --- |
| `renderer` | `CustomProfileRenderer` 高层 API |
| `elements` | `RenderElement` 枚举、`flatten_and_sort`、逐元素分发绘制 |
| `text` | TMP 富文本解析、字体测量、逐字排版、SDF 文字绘制 |
| `sdf` | SDF 字形轮廓的逐像素光栅化（需 `skia` feature；`parallel` 开启时按行并行） |
| `widgets` | 控件绘制（面板、徽章、缩略图等）与主题 |
| `widget_node` | `WidgetDocument` / `WidgetNode` 前后端合约 |
| `personal_profile` | 平台个人资料默认图渲染（需 `scenes` feature） |
| `ranking` | 排行榜预览图渲染（需 `scenes` feature） |
| `mysekai_harvest` | 采集地图渲染（需 `scenes` feature） |
| `primitives` | 图元定义（`SceneTree`） |
| `traits` | `Renderable` trait 与 `RenderOutput` |
| `masterdata` | 游戏数据解析（字体/颜色/称号） |
| `assets` | 素材 LRU 内存缓存 |
| `init` | 启动初始化（字体安装） |
| `executor` | 线程池隔离示例（已弃用，需 `executor` feature） |
| `transform` | Unity 坐标 → Skia 坐标转换、四元数旋转解析 |
| `profile` | 玩家数据模型（跨管线共享） |

## Features

| Feature | 说明 |
| --- | --- |
| `skia` | native 生产预设：`skia-core` + `parallel` + `scenes` + `executor`，skia-safe 启用 `textlayout/svg/vulkan/webp`（skia-binaries 预编译组合，勿增减） |
| `skia-minimal` | wasm 最小集：`skia-core` + skia-safe `webp`，无 GPU/textlayout/svg，skia 需源码编译 |
| `parallel` | rayon 光栅化线程池。关闭时逐行串行执行，输出逐字节不变 |
| `scenes` | 非名片场景：`ranking` / `mysekai_harvest` / `personal_profile` |
| `executor` | 已弃用的 `RenderExecutor`（依赖 tokio），仅 `render-deck` 示例使用 |
| `dev` | `skia` + `tracing-subscriber`，供 `tools/` 下的诊断 bin 使用 |

`skia-core` 是内部实现 gate，请通过 `skia` 或 `skia-minimal` 启用。
不启用任何 feature 时 `compose` 等纯数据路径可独立编译与测试。

## 构建

依赖系统 freetype（通过 `freetype-rs` / `pkg-config`）。建议在带有 freetype、pkg-config 的 Linux 环境或容器中构建。

```bash
cargo build -p allium-renderer --features skia
```

## 使用

资源（masterdata 表、字体、素材）一律由调用方提供，本仓库不内嵌任何游戏数据。
素材分两类，各取自不同基址：

- **静态素材**（边框、图标、遮罩、徽章）随引擎发布、不随版本变化，列在内嵌清单
  （`static_manifest.rs` / `static-manifest.js`）里；命中清单的 key 走静态基址。
- **动态素材**（卡面、图章、缩略图）随游戏版本变化，其余 key 走动态基址。

判定不能按 key 首段前缀——`honor/` 等前缀既含静态边框（`honor/frame_degree_*`）
又含动态图（`honor/<abn>/degree_*`），只有清单成员判定才区分得开。

### npm 包（浏览器 wasm）

```bash
npm install @empty-sekai/renderer-wasm
```

浏览器中推荐用 Worker 入口，skia 光栅化在 Worker 内同步执行，不阻塞 UI：

```ts
import { AlliumWorkerClient } from "@empty-sekai/renderer-wasm/worker";

const client = await AlliumWorkerClient.spawn({
  workerUrl: new URL("@empty-sekai/renderer-wasm/worker.js", import.meta.url),
  moduleUrl: new URL("@empty-sekai/renderer-wasm/allium_renderer_wasm.js", import.meta.url).href,
});

const jpeg = await client.render({ cardJson, masterData, fonts, assets });
// masterData: Record<table, json>，fonts/assets: Record<name|key, Uint8Array>
```

主线程直接驱动（注意 skia 同步阻塞）：

```ts
import createAlliumRenderer from "@empty-sekai/renderer-wasm/allium_renderer_wasm.js";
import { AlliumRenderer, ImageFormat } from "@empty-sekai/renderer-wasm";

const r = await AlliumRenderer.create(createAlliumRenderer);
r.registerFont("FZLanTingHei-DB-GBK", await fetchBytes("/fonts/lanting.ttf"));
for (const [name, json] of tables) r.loadMasterData(name, json);
r.init();
for (const key of r.collectAssetKeys(cardJson)) {
  r.putAsset(key, await fetchBytes(assetUrl(key)));
}
const jpeg = r.render(cardJson, ImageFormat.Jpeg);
```

包不内嵌字体 / masterdata / 素材，全部由使用方注入。完整 demo 见
`crates/allium-renderer-wasm/demo/`（在线版见顶部链接）。

### CLI（`render-card`）

单次渲染一张名片：

```bash
render-card --masterdata <dir> --card <card.json> -o out.jpg \
    [--profile <profile.json>] [--assets-dir <dir>] [--font-dir <dir>] \
    [--format jpeg|png|png-transparent] [--page <seq>]
```

masterdata 与素材都可改从 URL 前缀按需拉取（本地缺什么拉什么，并发 + 指数退避重试）。
三个 URL 各是纯前缀，程序只在后面接 `/<table>.json` 或 `/<key>.png`，不插入
region / latest 等任何子路径，兼容任意镜像布局：

```bash
render-card \
    --masterdata-url https://your-mirror.example.com/masterdata \
    --assets-url     https://your-mirror.example.com/assets \
    --static-url     https://your-mirror.example.com/assets/static \
    --card card.json -o out.jpg
```

`--serve` 常驻模式：stdin/stdout 走 NDJSON，请求严格串行，字体 / masterdata /
glyph 缓存 / 素材跨请求常驻，日志只走 stderr：

```bash
render-card --serve --masterdata <dir> [--assets-dir <dir>] [--font-dir <dir>]
# 每行一个请求：
#   {"id":1,"method":"render","params":{"card":{...},"output":"out.jpg"}}
#   {"id":2,"method":"reload_masterdata","params":{"dir":"..."}}
#   {"id":3,"method":"ping"}
#   {"id":4,"method":"shutdown"}
```

完整参数见 `render-card --help`。

## 许可证

[AGPL-3.0-only](./LICENSE)。Copyright (C) allium-renderer contributors。
