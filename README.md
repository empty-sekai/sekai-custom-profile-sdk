# Sekai Custom Profile SDK

[简体中文](README.md) | [English](README.en.md)

Sekai Custom Profile SDK 是面向 Project SEKAI（PJSK）自定义名片的非官方开发工具包。它提供游戏结构兼容的编辑文档、事务与导出，以及 native/browser 渲染、动态场景、交互区域、字体 atlas 和资源 provider。

富文本与排版实现针对 PJSK 使用的 Unity TextMesh Pro 数据、标签和运行时行为建立兼容模型，覆盖本仓库已经建模和验证的标签、排版、材质与动态语义。尚未建模的游戏行为或后续游戏更新可能与客户端不同，因此不承诺完整复刻游戏的渲染逻辑或最终像素。

renderer 是 SDK 内的一项能力。仓库包含两个面向不同运行环境的渲染适配器：

- native CPU renderer，适合服务端、CLI 与离线任务；
- browser WebGL2 renderer，使用 Rust/WASM、FreeType 与 SDF atlas 在浏览器中绘制和更新场景。

两者共享与后端无关的 Rust semantic core。浏览器包采用独立的 Rust/WASM + WebGL2 执行路径，native adapter 采用 Rust 自有的 CPU 光栅执行路径。

## 在线演示

[profile.mizore.art](https://profile.mizore.art) 基于本 SDK 的 browser renderer 构建，可在浏览器中编辑并预览自定义名片：

![名片编辑器文本工具](docs/images/editor-text-tool.png)

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
| `sekai-profile-core` | 后端无关的场景 schema、profile resolution、动态、稳定 ID、mask、control 与 interaction geometry |
| `sekai-profile` | native CPU 渲染器、JSON masterdata provider 与 SDF 图集构建 |
| `sekai-profile-cli` | `sekai-profile` 命令行：`render`、`prepare` 与 NDJSON 常驻服务 `serve` |
| `sekai-profile-wasm` | minimal FreeType WASM、stateful worker protocol、WebGL2 runtime、缓存与 Scene Workbench |

## 浏览器快速开始

browser SDK 以 `BrowserAuthoringClient` 和 `BrowserRenderer` 为入口。调用方通过 authoring API 编辑游戏结构文档，并通过必填的 `ResourceProvider` 用任意异步规则解释 SDK 提供的语义 descriptor。

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
  // 完整的 profile API 响应；名片文档取自 userCustomProfileCards
  //（按 seq 升序的页序，默认第一页，可用 pageIndex 选择），与 profile 数据保证同源。
  profile,
  frameMode: "animate",
});

scene.draw();
```

完整的接入流程、provider contract、缓存与交互说明见 [browser package README](crates/sekai-profile-wasm/README.md)。

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

native adapter 用 Rust 自有的 CPU 光栅绘制名片，用于服务端静态图片、命令行和应用自定义 scene。
资源字节与 masterdata 始终由调用方提供。文字与图形按 SDF 绘制，需要对应字体族的文字图集和
shape 图集；`prepare` 从字体文件、masterdata 与 shape 素材生成它们。

### 命令行

GitHub Release 提供 Linux（x86_64、aarch64）、Windows x86_64 与 macOS aarch64 的
`sekai-profile` 可执行文件。

```sh
sekai-profile render   --profile ./profile.json   --masterdata ./masterdata   --assets-dir ./assets   --font-dir ./fonts   --atlas-dir ./atlases   -o card.png
```

`render` 默认渲染 profile 的 `userCustomProfileCards` 中按游戏顺序的第一页；`--page N`
（从 1 起）或 `--seq S` 选择其他页，`--card` 改用单独的名片 JSON。输出格式跟随 `-o` 的扩展名
（`.png`、`.jpg`），也可用 `--format` 指定；`--transparent` 输出透明背景 PNG，`--quality`
设置 JPEG 质量。渲染前会把这一页需要的字形和图形补进图集目录（`--atlas-dir`，默认 `./atlases`）：
已有图集保留并按需扩充，之后的渲染直接复用；`--no-prepare` 直接用现有图集渲染。

`prepare` 为 masterdata 引用的全部字体族和 shape 生成完整图集，适合在常驻服务启动前一次性准备；
`--for <profile.json>`（可重复）只覆盖这些 profile 的文字。没有字体文件的字体族会被列出。

```sh
sekai-profile prepare --masterdata ./masterdata --font-dir ./fonts   --assets-dir ./assets --atlas-dir ./atlases
```

`serve` 逐行读取 stdin 上的 NDJSON 请求并逐行回应，字体、masterdata、图集与素材缓存跨请求常驻；
协议见 `crates/sekai-profile-cli/src/serve.rs`。

masterdata 来自目录（`--masterdata`）或 URL 前缀（`--masterdata-url`）。素材来自目录
（`--assets-dir`，key 为相对路径去掉 `.png`）或 URL 前缀（`--assets-url`、`--static-url`），
本地缺失的素材才走网络。`--region` 选择区服（默认 `cn`），`--executor` 选择 SDF 执行器
（默认 `auto`：CPU 支持 AVX-512 时用 SIMD，否则用标量）。

### 在 Rust 中使用

```toml
[dependencies]
sekai-profile = { git = "https://github.com/empty-sekai/sekai-custom-profile-sdk", tag = "v<version>" }
```

```rust
use std::path::Path;
use std::sync::Arc;

use sekai_profile::profile::ProfileData;
use sekai_profile::region::Region;
use sekai_profile::types::UserCustomProfileCard;
use sekai_profile::{AssetStore, CustomProfileRenderer, JsonMasterDataProvider, RenderConfig};

sekai_profile::set_font_dirs(["fonts"]);
let masterdata = JsonMasterDataProvider::from_dir(Path::new("masterdata"))?.with_region(Region::Jp);
let assets = AssetStore::new(256);
assets.load_png_dir(Path::new("assets"))?;
let renderer = CustomProfileRenderer::new(Arc::new(masterdata))
    .with_assets(Arc::new(assets))
    .with_atlas_dir("atlases")?;

let response: serde_json::Value = serde_json::from_slice(&std::fs::read("profile.json")?)?;
let pages: Vec<UserCustomProfileCard> =
    serde_json::from_value(response["userCustomProfileCards"].clone())?;
let first = sekai_profile::core::profile_source::profile_page(&pages, 0).expect("a page");
let profile = ProfileData::from_json(&response);

let page = renderer.render(&first.custom_profile_card, Some(&profile), &RenderConfig::default())?;
std::fs::write("card.png", page.to_png()?)?;
```

`render` 返回预乘 RGBA 的 `RenderedPage`；`to_png` 输出直通 alpha 的 PNG，启用 `jpeg-turbo`
feature 时 `to_jpeg` 输出 JPEG，常驻服务可用 `CustomProfileRenderer::encode_jpeg` 复用编码缓冲。
`RenderConfig::default()` 按 CPU 自动选择 SDF 执行器、白色背景；`Background::Transparent`
保留透明背景。

### 渲染配置

| 字段 | 取值 | 默认 | 说明 |
| --- | --- | --- | --- |
| `executor` | `Auto`、`Simd`、`Scalar` | `Auto` | `Auto` 在 CPU 支持时走 `Simd`。CPU 缺 AVX-512 F / BW / VBMI 或 FMA 时指定 `Simd` 报错。 |
| `background` | `White`、`Transparent` | `White` | 透明页只能编码成 PNG。 |
| `tile_width` / `tile_height` | 8 到 128 的 2 的幂 | `32` | SDF 执行器按这个尺寸分块。 |
| `pixel_occlusion` | `Off`、`Measure`、`Cull` | `Off` | `Measure` 统计被后续不透明图片完全覆盖的 SDF 像素数并写进 telemetry；`Cull` 跳过这些像素的绘制。两者都不改变页面最终像素。 |

配置无法满足时 `render` 返回 `RenderError`：`InvalidTileSize` 带出被拒的尺寸，`SimdUnavailable`
表示该 CPU 跑不了指定执行器，`Render` 承载渲染过程本身的错误文案。用
`CustomProfileRenderer::capabilities` 可以预先查看该渲染器会怎么做：本 CPU 能否跑 SIMD 包执行器、
装了几份文字图集，以及形状图集、render-object store、回退字形缓存是否在位。

`render_batch` 由一次编译出的批量结果渲染一个 profile 的全部页面，返回 `RenderedBatch`，并带回
`CompiledProfileBatch` 与预取的 render object，便于一次性渲染与缓存整个 profile。`render_animation`
渲染动画而非页面，`animation_preflight` 在实际绘制任何一帧之前报告该动画需要什么。两者都接收
`AnimationDocument`，即场景稳定 id 所依据的 key 与固定文字本地化所用的区服。两者与
`sekai_profile::animation` 中的函数失败时都返回 `AnimationError`：`OutputBudgetExceeded` 与
`ExportMemoryExceeded` 表示产物或导出占用的内存超出预设的预算，`Render` 带出渲染本身的 `RenderError`，
其余变体分别指出出错的预设、编码器、产物校验或导出步骤。

### 缓存已渲染的页面

`CustomProfileRenderer::cache_identity` 返回一个键，凡影响像素的因素变化它都会变：`RenderConfig`、
每一份已安装图集的 manifest、已发布的 render-object store。服务可用它作为已编码页面的缓存键，
换图集或换 store 后不会取回旧页。多次渲染共用同一个钉住的 store 时用
`cache_identity_pinned` 配 `RenderObjectGenerationPin`。

### 图集目录

`with_atlas_dir` 安装的目录结构如下，由 `sekai_profile::prepare::prepare_atlas_dir`
（即 `sekai-profile prepare`）生成：

```text
atlases/text/<字体族>/manifest.json   每个字体族一个文字图集
atlases/tiers/<名称>/manifest.json    可选，同一字体更大字号的图集
atlases/shape/manifest.json           shape 图集
atlases/fallback/<字体 SHA-256>/      后备字体的字形缓存，渲染时按需生成
```

文字图集记录生成它的字体文件 SHA-256，安装时字体目录中的同名字体必须与之一致，因为超出图集字号的
字形在渲染时从该字体生成。安装了 `Source Han Sans SC` 字体时，游戏字体缺失的字符从它生成并缓存在
`fallback/` 下。`CustomProfileRenderer::text_codepoints` 给出一张名片各字体族需要的码位，
`PrepareOptions::codepoints` 据此只生成这些字形。`build-sdf-atlas`、`build-shape-sdf-atlas`
可单独构建一个图集；`build-render-object-store` 生成可选的预解码纹理库（`--render-objects`）。

字体目录（`--font-dir` / `SEKAI_PROFILE_FONT_DIR`）按文件名查找字体族：

| 字体族（国服名） | 文件 |
|---|---|
| `FOT-RodinNTLGPro-DB`（`FZLanTingHei-DB-GBK`） | `FOT-RodinNTLGPro-DB.ttf` 或 `.otf` |
| `FOT-SkipProN-B`（`FZZhengHei-EB-GBK`） | `FOT-SkipProN-B.otf` |
| `FOT-PopHappinessStd-EB`（`FZShaoEr-M11-JF`） | `FOT-PopHappinessStd-EB.otf` |
| `FOT-YurukaStd-UB` | `FOT-YurukaStd-UB.otf` |
| `Source Han Sans SC`（后备字体） | `SourceHanSansSC-Medium.otf` 或 `SourceHanSansSC-Regular.otf` |

国服 masterdata 的字体名按上表括号中的名字解析，文件相同。御神签所用字体见下文。

`customProfileCharacterIconResources`、`customProfileMaterialResources`、
`customProfileUserInterfaceIconResources` 只有部分区服提供，`omikujis` 只有御神签收藏品读取；
四张表即 `sekai_profile::masterdata::OPTIONAL_TABLES`。`--masterdata` 与 `--masterdata-url` 存在时加载；
缺失时 `characterIcons`、`materials`、`userInterfaceIcons` 与御神签收藏品不绘制，也不计入缺表告警。

收藏品按 `customProfileCollectionResources` 行的 `customProfileResourceCollectionType` 绘制：
`can_badge`（罐徽章）以带光照的徽章材质绘制原图，另需静态素材 `ui/sekai_badge_normal`
（法线贴图，已列入 CLI 静态清单）；`omikuji` 行指向签的预制体，元素的 `targetId` 选出
`omikujis` 行，native 与浏览器渲染器按该行绘制签面图、运势图，以及用 FreeType 覆盖率（而非
SDF）绘制的竖排标题、总评与三段说明；两者共用 core 的版式与 FreeType 字形设置。签面与运势图
作为普通素材请求；签文使用字体资源 `FOT-Omikuji`（2022 年签）或 `FOT-UDMinchoPro-B`
（2023–2025 年签）。该字体没有字形的字符，取自第一个有该字形的后备 face，字号相同、不加粗：
先 `Roboto`，再按 master data 表所属区服选取的 Noto Sans CJK face。御神签页面需要签文字体与
两个后备字体，缺任何一个都渲染失败：

| Family | 文件 | Face |
|---|---|---|
| `FOT-Omikuji`（2022 年签） | `FOT-Omikuji.otf` | 0 |
| `FOT-UDMinchoPro-B`（2023–2025 年签） | `FOT-UDMinchoPro-B.otf` | 0 |
| `Roboto` | `Roboto-Regular.ttf` | 0 |
| `Noto Sans CJK SC`（区服 `cn`、`en` 及其他） | `NotoSansCJK-Regular.ttc` | 2 |
| `Noto Sans CJK TC`（区服 `tw`） | `NotoSansCJK-Regular.ttc` | 3 |
| `Noto Sans CJK JP`（区服 `jp`） | `NotoSansCJK-Regular.ttc` | 0 |
| `Noto Sans CJK KR`（区服 `kr`） | `NotoSansCJK-Regular.ttc` | 1 |

native 在字体目录中查找各文件并打开表中的 face；浏览器经 `FontProvider` 按 family 取字体文件。
两个后备字体文件为 Android 14 原生系统字体：

- `Roboto-Regular.ttf`，SHA-256 `9ca9debb09459bf4e3e7f826f5cd0f35f253902b85684921fce2ba3f28dd0f50`；
- `NotoSansCJK-Regular.ttc`，SHA-256 `39fb47c543da50618ab99e8b9e5529e54566bdbef41719308165975f627d5c93`。

`targetId` 缺失或指向不存在的行、以及未收录的签预制体都不绘制，也不请求素材。其余类型与缺省值按
普通图片绘制。

`--assets-url` 默认保持通用的 `flat` 规则（`/<key>.png`）。只有资源源采用游戏解包目录时，
调用方才应显式传 `--asset-url-layout game-assets`；该模式通过 shared core 的 canonical
映射解析 `bonds_honor/character` 与 `bonds_honor/word`，不改变 renderer asset key。

称号的资源语义由 `sekai-profile-core` 单一维护，native 与 WASM 不得各自猜测：国服
`limitevent` 的 `honor_top_*` 在未显式指定背景时使用共享
`honor_bg_event_cheerteam` degree 层；`bondsHonorViewType` 是可组合标志，字符串中可同时
包含 `reverse` 与 `unit_virtual_singer`。native adapter 只调用 core 规则函数，避免 native
渲染、离线目录生成器和浏览器 resolver 产生不同资源键。

### CPU 画布

`sekai_profile::profile_compositor::canvas` 提供有界的纯标签画布。`Canvas` 与 profile
合成器共用图片采样、source-over 混合和语义 shape 光栅器，支持裁切/缩放的 PNG 图片、矩形裁剪、
矩形、圆角矩形、椭圆、二次曲线描边和 FreeType 纯文本标签。`Font` 借用字体 bytes，调用方可以
共享同一份内存。TMP 富文本仍走 profile 管线。

像素缓冲为预乘 RGBA，`encode_png` 输出直通 alpha 的 PNG，`crop`
把区域复制成新画布而不重新编码。画布与解码后的图片缓冲上限 128 MiB，PNG 输入上限 32 MiB；
总的缓存与并发预算由调用方负责。

`plain_canvas` 示例接收一个字体文件和输出 PNG 路径。它的输出只演示功能，不作为名片一致性或
视觉验收基线。

### 环境变量

| 变量 | 作用 |
|---|---|
| `SEKAI_PROFILE_FONT_DIR` | native 字体目录，排在 `sekai_profile::set_font_dirs` 给出的目录（命令行的 `--font-dir`）之后查找；之后读取 `FONT_DIR`，最后查找 `assets/fonts` |
| `SEKAI_PROFILE_ATLAS_DIR` | 命令行 `--atlas-dir` 的默认值（`atlases`） |
| `SEKAI_PROFILE_SDF_EDT` | 设为 `1`–`4` 时字形 SDF 改用距离变换生成，值为超采样倍数；未设置或其他值使用解析法 |
| `SEKAI_PROFILE_REALTIME_EDT_THREADS` | `parallel` feature 下为图集缺失的字形即时生成 SDF 的线程数，`1`–`4`，默认 `2` |
| `SEKAI_PROFILE_DEBUG_TMP_PROBE`、`SEKAI_PROFILE_DEBUG_TEXT_CODEPOINTS` | 设为 `1`、`true` 或 `yes` 时输出 TMP 排版探针与文本码位的 `tracing` debug 日志 |

### 字体目录

字体族按文件名解析。查找顺序是 `sekai_profile::set_font_dirs` 给出的目录、`SEKAI_PROFILE_FONT_DIR`、
`FONT_DIR`，最后是 `assets/fonts`。图集记录了构建它的字体文件的 SHA-256，同名文件必须与之一致，
因此 `set_font_dirs` 要在 `with_atlas_dir` 之前调用。

`with_atlas_dir` 接受一个并不存在的目录且不会报错，这时渲染器没有装任何图集，启动不会失败但画不出东西。
先构建图集，或在装好后检查 `capabilities().text_atlases`。

## 从 0.7 迁移

| 0.7 | 现在 |
| --- | --- |
| `render_animation(card, profile, document_key, region, preset, config)` | `render_animation(card, profile, AnimationDocument { key, region }, preset, config)`；`render_animation_pinned` 与 `animation_preflight` 接收同样的 `AnimationDocument` |
| 动画 API 返回 `Result<_, String>` | 返回 `Result<_, AnimationError>`；传给 `encode_rgba_frames` 的取帧闭包也返回 `AnimationError`，自身的错误用 `AnimationError::FrameSource` 包装 |
| `profile_compositor::render_authored_profile_into_{simd,scalar}(scene, store, text_atlases, md, assets, kind, index, pixels, width, height)` | `(scene, CompositorSources { store, text, assets }, kind, index, RgbaSurface::new(pixels, width, height)?)`；绘制 SDF 文字时 `text` 为 `Some(SemanticSdfContext { text_atlases, md })` |
| `profile_compositor::render_authored_image_into_simd(scene, store, kind, index, pixels, width, height)` | `(scene, store, kind, index, RgbaSurface::new(pixels, width, height)?)` |
| 长度不符的目标缓冲区 | 由 `RgbaSurface::new` 以 `ProfileCompositorError::SurfaceLength` 拒绝 |
| `SdfDrawCommand::from_atlas_glyph(kind, atlas_set, glyph, atlas_point_size, atlas_spread, baseline_origin, font_size, local_to_device, material)` | `from_atlas_glyph(kind, SdfGlyphAtlas { atlas_set, point_size, spread }, glyph, SdfGlyphPlacement { baseline_origin, font_size, local_to_device }, material)` |
| `core::profile_resolve::compile_profile_scene_with_localizations(…, locale, resource_metadata, line_indent, localized_text)` | `(…, ProfileLocalization { locale, localized_text }, resource_metadata, line_indent)` |

## 从 0.6 迁移

0.6 的 crate 名为 `sekai-profile-renderer`、`sekai-profile-renderer-host` 与
`sekai-profile-renderer-core`，现在分别是 `sekai-profile`（并入了 host crate）、`sekai-profile` 与
`sekai-profile-core`。

| 0.6 | 0.7 |
| --- | --- |
| `ProfileBackendConfig` | `RenderConfig` |
| `ProfileBackendConfig::surface` | 已删除；CPU 光栅路径是唯一的 surface |
| `TextSdfExecutor` 与 `ShapeSdfExecutor` | 合并为一个 `SdfExecutor` |
| `TextSdfExecutor::LegacySkia` | 已删除 |
| `ShapeSdfExecutor::Auto` | `SdfExecutor::Auto` |
| `ScalarOracle` | `SdfExecutor::Scalar` |
| `ProfileJpegEncoder::Skia` | 已删除；改用 `JpegOptions` 配 `JpegEncoder::Standard` |
| `ProfileJpegEncoder::LibJpegTurbo` | `JpegEncoder::Standard` |
| `ProfileJpegEncoder::LibJpegTurboAvx512Yuv420` | `JpegEncoder::Avx512Yuv420` |
| `BackendFallbackPolicy` | 已删除；执行器不可用即报错 |
| `pixel_occlusion_dry_run` | `PixelOcclusion::Measure` |
| `pixel_occlusion_execute` | `PixelOcclusion::Cull` |
| `collect_telemetry` | 已删除；遥测始终采集 |
| `render_page_with_backend(_generation)` | `render_pinned` |
| `render_profile_batch_with_backend(_generation)` | `render_batch_pinned` |
| `render_animation_with_profile_backend` | `render_animation_pinned` |
| `animation_preflight_with_profile` | `animation_preflight` |
| `profile_backend_capabilities()` | `capabilities()` |
| `profile_backend_cache_identity(_for_generation)` | `cache_identity(_pinned)` |
| `prewarm_profile_backend_resources()` | `prewarm()` |
| `ProfileRenderTelemetry` | `RenderTelemetry`，schema `allium.profile-render.telemetry.v4` |

渲染入口返回 `RenderedPage` 而不是已编码字节，由调用方用 `encode_png` 或 `encode_jpeg` 编码。
0.6 缓存的页面不会被复用：缓存键前缀由 `profile-backend-` 改为 `profile-render-`，遥测 schema 同步变更。

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

项目使用固定的 container toolchain 构建 FreeType 与 Emscripten 目标；native 构建需要 FreeType（pkg-config 可找到，或启用 `bundled-freetype`），`animation-export` 另需 libx264。

```sh
cargo test --workspace --all-features

cd crates/sekai-profile-wasm
npm run build
npm run typecheck
npm run test:gates
npm run verify:wasm:runtime
npm run measure:wasm:size
```

release gates 覆盖 ABI/schema、shared-core source consistency、TMP debug parity、glyph SDF parity、完整 profile command coverage、atlas/cache budget、worker singleflight、context restoration、bounded telemetry、npm contents 与 native/browser release matrix。

## License

仓库采用 AGPL-3.0-only。browser npm package 另含 `crates/sekai-profile-wasm/LICENSE-EXCEPTION` 中的有限 browser linking exception。修改 SDK、服务端使用和非浏览器使用仍受完整 AGPL 约束，包括网络交互场景下的源代码提供义务。
