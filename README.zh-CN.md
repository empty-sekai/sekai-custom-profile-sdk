# Sekai Custom Profile SDK

[English](README.md) | 简体中文

Sekai Custom Profile SDK 是面向 Project SEKAI（PJSK）自定义名片的非官方开发工具包。
它编辑与游戏结构兼容的名片文档，并在 CPU 上原生渲染，或在浏览器中用 WebGL2 渲染。

富文本与排版针对 PJSK 使用的 Unity TextMesh Pro 数据、标签和运行时行为建立兼容模型，
覆盖本仓库已实现并测试的标签、排版、材质与动态语义。模型未覆盖的游戏行为以及后续游戏更新
可能与客户端不同，因此 SDK 不保证与游戏的渲染逻辑或最终像素一致。

## 在线演示

[profile.mizore.art](https://profile.mizore.art) 基于浏览器渲染器构建，可在浏览器中编辑并
预览自定义名片。

![名片编辑器文本工具](docs/images/editor-text-tool.png)

## 功能

- 与游戏结构兼容的编辑文档，支持事务、手势、撤销/重做、checkpoint 与导出。
- 与后端无关的 Rust core，统一负责 profile 与 masterdata 解析、动态、稳定 ID、mask、
  control 与交互几何，两个渲染器共用。
- 兼容 TextMesh Pro 的富文本解析与排版，使用 FreeType 度量与 SDF 字形。
- 原生 CPU 渲染器，适合服务端、命令行工具与离线任务：输出 PNG 与 JPEG 页面、整份 profile
  批量渲染，以及动画导出（GIF、WebP、APNG、MP4）。
- WebGL2 浏览器渲染器，由 worker 中的 Rust/WASM 驱动，支持动态场景、图层显隐、tab、
  滚动与命中几何。
- 字体、本地化与图片 provider 由宿主定义：每项资源来自哪里由应用决定。
- 可检查的场景 dump 与有界 telemetry。

## 包

| 包 | 内容 |
| --- | --- |
| npm [`@empty-sekai/sekai-custom-profile-sdk`](crates/sekai-profile-wasm/README.zh-CN.md) | 浏览器渲染器与编辑客户端：TypeScript 封装与 WASM 运行时 |
| crate `sekai-profile-core` | 与后端无关的场景 schema、profile 解析、编辑文档、动态与交互几何 |
| crate `sekai-profile` | 原生 CPU 渲染器、JSON masterdata provider、素材存储与图集构建 |
| crate `sekai-profile-cli` | `sekai-profile` 命令：`render`、`prepare` 与 `serve` |
| crate `sekai-profile-wasm` | npm 包的源码（不作为 crate 发布） |

每个 [GitHub Release](https://github.com/empty-sekai/sekai-custom-profile-sdk/releases)
提供 Linux x86_64 与 aarch64、Windows x86_64、macOS aarch64 的 `sekai-profile` 可执行文件，
浏览器构建产物（`dist/`）的 zip 包，以及 `SHA256SUMS`。

## 快速上手

### 浏览器

```sh
npm install @empty-sekai/sekai-custom-profile-sdk
```

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
  // 完整的 Profile API 响应。页面取自 userCustomProfileCards，
  // 按 seq 升序：默认第一页，或由 pageIndex 指定。
  profile,
  frameMode: "animate",
});

scene.draw();
```

`loadApplicationFont`、`resolveApplicationResource` 与 `loadApplicationMasterData`
代表应用自己的加载逻辑。provider 契约、编辑、交互与缓存见
[浏览器包 README](crates/sekai-profile-wasm/README.zh-CN.md)。

### Rust

```toml
[dependencies]
sekai-profile = { git = "https://github.com/empty-sekai/sekai-custom-profile-sdk", tag = "v<version>" }
serde_json = "1"
```

```rust
use std::path::Path;
use std::sync::Arc;

use sekai_profile::profile::ProfileData;
use sekai_profile::region::Region;
use sekai_profile::types::UserCustomProfileCard;
use sekai_profile::{AssetStore, CustomProfileRenderer, JsonMasterDataProvider, RenderConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    sekai_profile::set_font_dirs(["fonts"]);
    let masterdata =
        JsonMasterDataProvider::from_dir(Path::new("masterdata"))?.with_region(Region::Jp);
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
    Ok(())
}
```

图集由 `sekai-profile prepare` 或 `sekai_profile::prepare::prepare_atlas_dir` 生成。
编码、配置、批量渲染、动画与缓存键见[原生渲染器](docs/native-renderer.md)（英文）。

### 命令行

```sh
sekai-profile prepare --masterdata ./masterdata --font-dir ./fonts \
  --assets-dir ./assets --atlas-dir ./atlases

sekai-profile render --profile ./profile.json --masterdata ./masterdata \
  --assets-dir ./assets --font-dir ./fonts --atlas-dir ./atlases -o card.png
```

`render` 会先补齐当前页需要的字形，单次渲染可以不先运行 `prepare`。`serve` 以 NDJSON
从 stdin 接收渲染请求。详见[命令行](docs/cli.md)（英文）。

## 文档

`docs/` 下的文档为英文。

- [Architecture](docs/architecture.md)：职责划分、场景创建流程、图层与交互、缓存、调试与 telemetry。
- [Native renderer](docs/native-renderer.md)：Rust 库 API、纯 CPU 画布、Cargo feature 与环境变量。
- [Command line](docs/cli.md)：`render`、`prepare`、`serve`，以及 masterdata 与素材来源。
- [Fonts, atlases and assets](docs/fonts-and-assets.md)：图集目录、字体文件、masterdata 表、
  收藏品类型与素材 key。
- [Development](docs/development.md)：构建、测试、CI、提交约定与发版。
- [浏览器包](crates/sekai-profile-wasm/README.zh-CN.md)：npm 包 API。
- [Demo](crates/sekai-profile-wasm/demo/README.md)：浏览器参考应用（英文）。

## 从源码构建

Rust crate 需要 FreeType（经 pkg-config 查找，或启用 `bundled-freetype` feature）；WASM 运行时
用 `crates/sekai-profile-wasm/build.sh` 在容器中构建。详见[开发文档](docs/development.md)（英文）。

## License

仓库采用 AGPL-3.0-only（[LICENSE](LICENSE)）。npm 包另附一项有限的附加许可，允许把未修改的包
与在浏览器中运行的前端代码链接（[LICENSE-EXCEPTION](crates/sekai-profile-wasm/LICENSE-EXCEPTION)）。
修改后的构建、服务端使用以及其他任何非浏览器使用仍受完整 AGPL 约束，包括网络交互场景下的源代码
提供义务。
