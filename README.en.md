# Sekai Custom Profile SDK

[简体中文](README.md) | [English](README.en.md)

Sekai Custom Profile SDK is an unofficial development kit for Project SEKAI (PJSK) custom profiles. It provides game-structure-compatible authoring documents, transactions, and exports together with native/browser rendering, dynamic scenes, interaction geometry, font atlases, and resource providers.

Rich text and layout follow a compatibility model for the Unity TextMesh Pro data, tags, and runtime behavior used by PJSK. It covers the tags, layout, material, and dynamic semantics modeled and verified by this repository. Unmodeled game behavior and later game updates may differ from the client, so complete reproduction of the game's rendering logic or final pixels is not guaranteed.

Rendering is one capability of the SDK. The repository contains two rendering adapters for different environments:

- a native CPU renderer for servers, CLI tools, and offline jobs;
- a browser WebGL2 renderer backed by Rust/WASM, FreeType, and glyph SDF atlases.

Both adapters share a backend-independent Rust semantic core. The browser package uses a dedicated Rust/WASM and WebGL2 execution path, while the native adapter uses its own Rust CPU raster path.

## Live demo

[profile.mizore.art](https://profile.mizore.art) is built on the SDK's browser renderer and lets you edit and preview custom profile cards in the browser:

![Card editor text tool](docs/images/editor-text-tool.png)

## Responsibilities

Rust/WASM owns:

- game-structure-compatible authoring documents, transactions, gestures, undo/redo, checkpoints, and exports;
- profile-card and masterdata resolution;
- TMP rich-text parsing and layout;
- dynamic formulas and mutable scene state;
- FreeType metrics, glyph SDF generation, and atlas placement;
- stable layer, command, control, and interaction IDs;
- the layer tree, source content, resolved parameters, bounds, quads, matrices, and hit geometry.

The browser main thread owns:

- calling the application's asynchronous font, localization, and image-resource providers;
- deduplicating, scheduling, and retaining decoded image resources;
- uploading atlases, command buffers, and compact state tables to WebGL2;
- restoring GPU resources after WebGL context loss.

The host application owns:

- profile data, fonts, masterdata, and image assets;
- whether resources come from the network, local files, IndexedDB, signed requests, authenticated APIs, or another source;
- hover, click, selection, editing, copying, navigation, and DOM/SVG overlays.

Host-provided font bytes with fixed source hashes, FreeType metrics, TMP layout, and the SDF pipeline jointly define text geometry and glyph pixels.

## Workspace

| Crate | Responsibility |
| --- | --- |
| `sekai-profile-core` | Backend-independent scene schema, profile resolution, dynamics, stable IDs, masks, controls, and interaction geometry |
| `sekai-profile` | Native CPU renderer, JSON masterdata provider, and SDF atlas building |
| `sekai-profile-cli` | The `sekai-profile` command line: `render`, `prepare`, and the NDJSON service `serve` |
| `sekai-profile-wasm` | Minimal FreeType WASM, stateful worker protocol, WebGL2 runtime, caches, and Scene Workbench |

## Browser quick start

The browser SDK starts with `BrowserAuthoringClient` and `BrowserRenderer`. Authoring APIs edit game-compatible documents, while a required `ResourceProvider` interprets SDK-provided semantic descriptors with arbitrary asynchronous host logic.

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
  // Full profile API response; the card document is derived from
  // userCustomProfileCards (pages in ascending seq order: the first page, or
  // pageIndex) and shares its source.
  profile,
  frameMode: "animate",
});

scene.draw();
```

See the [browser package README](crates/sekai-profile-wasm/README.en.md) for the complete integration flow, provider contract, caching model, and interaction API.

## What happens when a scene is created

1. WASM in the worker reads the profile, card, and masterdata and emits scene-local localization, font, glyph, layout, and resource demand in order.
2. TypeScript obtains immutable localized text, font bytes, and image-resource snapshots through bounded host providers. The host may also register fonts directly before scene creation.
3. The `ResourceProvider` returns a `Blob`, `ArrayBuffer`, `Uint8Array`, or directly uploadable `TexImageSource`.
4. WASM performs FreeType measurement, TMP layout, glyph SDF generation, and atlas placement from fonts with fixed source hashes.
5. The semantic core produces the authored layer tree, commands, controls, interaction regions, and dynamic state.
6. WebGL2 uploads atlases, images, geometry, and state buffers and draws the scene.
7. `advance()`, layer masks, tabs, and scrolling reuse the existing timeline, layout, and atlas while updating only the affected state.
8. `dump()` and `stats()` expose inspectable semantic data and bounded telemetry.

A missing or undecodable image produces a warning and a transparent placeholder, allowing the remaining scene to continue. Schema, ABI, and memory-budget contract violations fail explicitly.

## State, layers, and interaction

Public layers correspond to game-authored card elements. Shapes, glyphs, masks, and other draw primitives are commands owned by those layers.

```ts
await scene.advance(tick);
await scene.setLayerVisible(layerId, false);
await scene.setLayerMasks(layerTableRevision, overrides);
await scene.setTab(controlId, value);
await scene.scrollBy(controlId, delta);
scene.draw();

const dump = await scene.dump();
```

Layer visibility reuses the active dynamic timeline, layout cache, glyph atlas, and image cache. The renderer exposes regions, control bindings, resolved data, and geometry; the application uses them to open a character, event, card, story, or another data page.

## Native renderer

The native adapter draws cards with Rust's own CPU rasterizer for still images, the command
line, and application-specific scenes. Resource bytes and masterdata are always provided by the
caller. Text and shapes are drawn as signed distance fields and need a text atlas for each font
family and a shape atlas; `prepare` builds them from the font files, the masterdata, and the
shape sprites.

### Command line

GitHub Releases provide `sekai-profile` executables for Linux (x86_64, aarch64), Windows x86_64,
and macOS aarch64.

```sh
sekai-profile render   --profile ./profile.json   --masterdata ./masterdata   --assets-dir ./assets   --font-dir ./fonts   --atlas-dir ./atlases   -o card.png
```

`render` draws the first page of the profile's `userCustomProfileCards` in the order the game
shows them; `--page N` (counted from 1) or `--seq S` selects another page, and `--card` takes a
separate card JSON instead. The output format follows the extension of `-o` (`.png`, `.jpg`) or
`--format`; `--transparent` writes a PNG with a transparent background and `--quality` sets the
JPEG quality. Before rendering, the glyphs and shapes the page needs are added to the atlas
directory (`--atlas-dir`, default `./atlases`): existing atlases are kept and extended as needed,
and later renders reuse them. `--no-prepare` renders with the atlases as they are.

`prepare` builds complete atlases for every font family and shape the masterdata names, which
suits preparing once before starting a long-running service; `--for <profile.json>` (repeatable)
covers only the text of those profiles. Font families without a font file are listed.

```sh
sekai-profile prepare --masterdata ./masterdata --font-dir ./fonts   --assets-dir ./assets --atlas-dir ./atlases
```

`serve` reads NDJSON requests from stdin and answers each on its own line, keeping fonts,
masterdata, atlases, and the asset cache across requests; the protocol is described in
`crates/sekai-profile-cli/src/serve.rs`.

Masterdata comes from a directory (`--masterdata`) or a URL prefix (`--masterdata-url`). Assets
come from a directory (`--assets-dir`, keyed by relative path without `.png`) or URL prefixes
(`--assets-url`, `--static-url`); only assets missing locally are fetched. `--region` selects the
game server (default `cn`) and `--executor` the SDF executor (default `auto`: SIMD when the CPU
supports AVX-512, scalar otherwise).

### Using the library

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

`render` returns a `RenderedPage` of premultiplied RGBA; `to_png` writes a straight-alpha PNG,
`to_jpeg` writes a JPEG when the `jpeg-turbo` feature is enabled, and long-running services can
use `CustomProfileRenderer::encode_jpeg` to reuse the encoder buffers. `RenderConfig::default()`
picks the SDF executor for the CPU and a white background; `Background::Transparent` keeps the
background transparent.

### Render configuration

| Field | Values | Default | Notes |
| --- | --- | --- | --- |
| `executor` | `Auto`, `Simd`, `Scalar` | `Auto` | `Auto` runs `Simd` when the CPU has it. `Simd` is an error on a CPU without AVX-512 F, BW and VBMI plus FMA. |
| `background` | `White`, `Transparent` | `White` | A transparent page encodes to PNG only. |
| `tile_width` / `tile_height` | A power of two from 8 to 128 | `32` | The SDF executor works on tiles of this size. |
| `pixel_occlusion` | `Off`, `Measure`, `Cull` | `Off` | `Measure` counts the SDF pixels that later opaque images cover and reports the count in the telemetry; `Cull` skips drawing them. Neither changes which pixels the page ends up with. |

`render` returns `RenderError` when the config cannot be honoured: `InvalidTileSize` names the
rejected pair, `SimdUnavailable` says the executor the config asked for is not what this CPU
runs, and `Render` carries a message from the render itself. Check what a renderer will do with
`CustomProfileRenderer::capabilities`, which reports whether this CPU runs the packet executor,
how many text atlases are installed, and whether a shape atlas, a render-object store and a
fallback glyph cache are present.

`render_batch` renders every page of a profile from one compiled batch and returns a
`RenderedBatch` alongside the `CompiledProfileBatch` and any prepared render objects, which lets
a caller render and cache a whole profile from a single compile. `render_animation` renders an
animation instead of a page, and `animation_preflight` reports what such a render needs before
any frames are drawn. Both take an `AnimationDocument`: the key the scene's stable ids derive
from and the region its fixed text is localized for. Both, like the functions in
`sekai_profile::animation`, return `AnimationError` on failure: `OutputBudgetExceeded` and
`ExportMemoryExceeded` mean the artifact or the memory the export holds outgrew the preset's
budget, `Render` carries the `RenderError` of the render itself, and the other variants name the
preset, encoder, artifact check or export step that failed.

### Caching rendered pages

`CustomProfileRenderer::cache_identity` returns a key that changes whenever anything that
affects the pixels changes: the `RenderConfig`, every installed atlas manifest and the published
render-object store. A service can use it as the cache key for encoded pages, so a changed atlas
or a new store never serves a stale page. Use `cache_identity_pinned` with a
`RenderObjectGenerationPin` when several renders share one pinned store.

### Atlas directory

`with_atlas_dir` installs a directory laid out as follows, which
`sekai_profile::prepare::prepare_atlas_dir` (that is, `sekai-profile prepare`) writes:

```text
atlases/text/<family>/manifest.json    one text atlas per font family
atlases/tiers/<name>/manifest.json     optional atlases of the same faces at larger sizes
atlases/shape/manifest.json            the shape atlas
atlases/fallback/<font SHA-256>/       glyphs of the fallback face, generated while rendering
```

A text atlas records the SHA-256 of the font file it was built from, and the font installed
under the same name must match it, because glyphs larger than the atlas size are generated from
that font while rendering. When `Source Han Sans SC` is installed, characters the game fonts lack
are generated from it and cached under `fallback/`. `CustomProfileRenderer::text_codepoints`
gives the codepoints a card needs per font family, and `PrepareOptions::codepoints` builds only
those glyphs. `build-sdf-atlas` and `build-shape-sdf-atlas` build a single atlas;
`build-render-object-store` builds an optional store of pre-decoded textures (`--render-objects`).

The font directory (`--font-dir` / `SEKAI_PROFILE_FONT_DIR`) is searched by file name:

| Font family (CN server name) | File |
|---|---|
| `FOT-RodinNTLGPro-DB` (`FZLanTingHei-DB-GBK`) | `FOT-RodinNTLGPro-DB.ttf` or `.otf` |
| `FOT-SkipProN-B` (`FZZhengHei-EB-GBK`) | `FOT-SkipProN-B.otf` |
| `FOT-PopHappinessStd-EB` (`FZShaoEr-M11-JF`) | `FOT-PopHappinessStd-EB.otf` |
| `FOT-YurukaStd-UB` | `FOT-YurukaStd-UB.otf` |
| `Source Han Sans SC` (fallback face) | `SourceHanSansSC-Medium.otf` or `SourceHanSansSC-Regular.otf` |

CN masterdata resolves its font names to the names in parentheses, which use the same files. The
omikuji fonts are listed below.

`customProfileCharacterIconResources`, `customProfileMaterialResources`, and
`customProfileUserInterfaceIconResources` are shipped by some regions only, and `omikujis` is
read only by omikuji collections; the four tables are `sekai_profile::masterdata::OPTIONAL_TABLES`.
`--masterdata` and `--masterdata-url` load them when present; without them the
`characterIcons`, `materials`, and `userInterfaceIcons` elements and omikuji collections are
not drawn and no missing-table warning is reported.

Collections follow the `customProfileResourceCollectionType` of their
`customProfileCollectionResources` row: `can_badge` rows draw their image through a lit badge
material that also needs the static asset `ui/sekai_badge_normal` (a normal map, listed in the
CLI's static manifest); `omikuji` rows name a fortune-slip prefab, and the element's
`targetId` picks the `omikujis` row it shows. The native and browser renderers draw the slip's
cover and fortune images and its vertical title, summary and description texts, which are
rendered from FreeType coverage rather than SDF glyphs; both use the core layout and FreeType
glyph settings. The two images are requested as ordinary assets. The texts use the font asset
`FOT-Omikuji` (2022 slips) or `FOT-UDMinchoPro-B` (2023–2025 slips). A character that font has
no glyph for comes from the first fallback face that has one, at the same pixel size and without
emboldening: `Roboto`, then the Noto Sans CJK face of the region the master-data tables belong
to. An omikuji page needs its slip font and both fallback fonts, and fails to render without any
of them:

| Family | File | Face |
|---|---|---|
| `FOT-Omikuji` (2022 slips) | `FOT-Omikuji.otf` | 0 |
| `FOT-UDMinchoPro-B` (2023–2025 slips) | `FOT-UDMinchoPro-B.otf` | 0 |
| `Roboto` | `Roboto-Regular.ttf` | 0 |
| `Noto Sans CJK SC` (region `cn`, `en`, any other) | `NotoSansCJK-Regular.ttc` | 2 |
| `Noto Sans CJK TC` (region `tw`) | `NotoSansCJK-Regular.ttc` | 3 |
| `Noto Sans CJK JP` (region `jp`) | `NotoSansCJK-Regular.ttc` | 0 |
| `Noto Sans CJK KR` (region `kr`) | `NotoSansCJK-Regular.ttc` | 1 |

The native renderer finds each file in the font directory and opens the face listed; the browser
renderer requests each family's file from its `FontProvider`. The two fallback files are the
stock Android 14 system fonts:

- `Roboto-Regular.ttf`, SHA-256 `9ca9debb09459bf4e3e7f826f5cd0f35f253902b85684921fce2ba3f28dd0f50`;
- `NotoSansCJK-Regular.ttc`, SHA-256 `39fb47c543da50618ab99e8b9e5529e54566bdbef41719308165975f627d5c93`.

A missing `targetId`, one naming no row, and a slip prefab that is not laid out draw nothing and
request nothing. Every other type, and a missing one, draws as a plain image.

`--assets-url` keeps the generic `flat` rule (`/<key>.png`) by default. Pass
`--asset-url-layout game-assets` only when the source follows the extracted game asset
layout. That explicit mode reuses the shared core canonical mapping for
`bonds_honor/character` and `bonds_honor/word` without changing renderer asset keys.

Honor asset semantics have one source of truth in `sekai-profile-core`; native and WASM
backends must not infer them independently. CN `limitevent` `honor_top_*` entries without
an explicit background use the shared `honor_bg_event_cheerteam` degree layer.
`bondsHonorViewType` is composable and may contain both `reverse` and
`unit_virtual_singer`. Native adapters call the core rule functions so native rendering,
offline catalog generation, and browser resolution use identical keys.

### Plain CPU canvas

`sekai_profile::profile_compositor::canvas` exposes a bounded plain-label canvas.
`Canvas` reuses the profile compositor's image sampling, source-over blending, and semantic shape
rasterizer. It supports cropped/scaled PNG images, rectangular clipping, rectangles, rounded
rectangles, ellipses, quadratic strokes, and FreeType plain labels. `Font` borrows font bytes, so
callers can keep one shared allocation. Rich TMP text still uses the profile pipeline.

Pixel buffers are premultiplied RGBA; `encode_png`
converts them to a straight-alpha PNG, and `crop` copies a region into a new canvas without
re-encoding. Canvas and decoded image buffers are limited to 128 MiB; encoded PNG inputs are
limited to 32 MiB. Callers remain responsible for aggregate cache and concurrency budgets.

Run the `plain_canvas` example with a font file and an output PNG path. Its output is a functional
demonstration, not a profile-parity or visual acceptance baseline.

### Environment variables

| Variable | Effect |
|---|---|
| `SEKAI_PROFILE_FONT_DIR` | Native font directory, searched after the directories given to `sekai_profile::set_font_dirs` (`--font-dir` on the command line). `FONT_DIR` is read next, then `assets/fonts` |
| `SEKAI_PROFILE_ATLAS_DIR` | Default of the command line's `--atlas-dir` (`atlases`) |
| `SEKAI_PROFILE_SDF_EDT` | `1`–`4` generates glyph SDFs with a distance transform at that supersampling factor; unset or any other value uses the analytic method |
| `SEKAI_PROFILE_REALTIME_EDT_THREADS` | With the `parallel` feature, the number of threads that generate SDFs for glyphs missing from the atlas, `1`–`4`, default `2` |
| `SEKAI_PROFILE_DEBUG_TMP_PROBE`, `SEKAI_PROFILE_DEBUG_TEXT_CODEPOINTS` | `1`, `true`, or `yes` emits `tracing` debug logs for TMP layout probes and text code points |

### Font directories

Font families are resolved by file name. `sekai_profile::set_font_dirs` sets the directories
searched first, then `SEKAI_PROFILE_FONT_DIR`, then `FONT_DIR`, then `assets/fonts`. An atlas
records the SHA-256 of the font it was built from and the file found under that family's name
must match it, so `set_font_dirs` runs before `with_atlas_dir`.

`with_atlas_dir` accepts a directory that does not exist and installs no atlas, leaving a
renderer that draws nothing but cannot fail at startup. Build the atlases first, or check
`capabilities().text_atlases` after installing them.

## Migrating from 0.6

0.6 named the crates `sekai-profile-renderer`, `sekai-profile-renderer-host` and
`sekai-profile-renderer-core`; they are now `sekai-profile` (which absorbed the host crate),
`sekai-profile` and `sekai-profile-core`.

| 0.6 | 0.7 |
| --- | --- |
| `ProfileBackendConfig` | `RenderConfig` |
| `ProfileBackendConfig::surface` | Removed; the CPU raster path is the only surface |
| `TextSdfExecutor` and `ShapeSdfExecutor` | One `SdfExecutor` for both |
| `TextSdfExecutor::LegacySkia` | Removed |
| `ShapeSdfExecutor::Auto` | `SdfExecutor::Auto` |
| `ScalarOracle` | `SdfExecutor::Scalar` |
| `ProfileJpegEncoder::Skia` | Removed; use `JpegOptions` with `JpegEncoder::Standard` |
| `ProfileJpegEncoder::LibJpegTurbo` | `JpegEncoder::Standard` |
| `ProfileJpegEncoder::LibJpegTurboAvx512Yuv420` | `JpegEncoder::Avx512Yuv420` |
| `BackendFallbackPolicy` | Removed; an unavailable executor is an error |
| `pixel_occlusion_dry_run` | `PixelOcclusion::Measure` |
| `pixel_occlusion_execute` | `PixelOcclusion::Cull` |
| `collect_telemetry` | Removed; telemetry is always collected |
| `render_page_with_backend(_generation)` | `render_pinned` |
| `render_profile_batch_with_backend(_generation)` | `render_batch_pinned` |
| `render_animation_with_profile_backend` | `render_animation_pinned` |
| `animation_preflight_with_profile` | `animation_preflight` |
| `profile_backend_capabilities()` | `capabilities()` |
| `profile_backend_cache_identity(_for_generation)` | `cache_identity(_pinned)` |
| `prewarm_profile_backend_resources()` | `prewarm()` |
| `ProfileRenderTelemetry` | `RenderTelemetry`, schema `allium.profile-render.telemetry.v4` |

The render entry points return a `RenderedPage` instead of encoded bytes, so a caller encodes
with `encode_png` or `encode_jpeg`. Cached pages from 0.6 are not reused: the cache key prefix
changed from `profile-backend-` to `profile-render-`, and the telemetry schema with it.

## Cache and resource ownership

- Font bytes and logical families supplied directly or through an arbitrary asynchronous `FontProvider` are the authoritative source for font resolution and glyph cache identity.
- The bounded glyph session atlas is reused across scenes owned by the same worker.
- Optional IndexedDB persistence stores opaque, version-validated glyph records only.
- Persistent encoded-image caching belongs entirely to the `ResourceProvider`, which may use HTTP caching, Cache Storage, IndexedDB, or application storage.
- The renderer retains only a bounded decoded-image session cache and context-local GPU textures.
- Context restoration reuploads retained atlas, image, and buffer data while reusing completed TMP layout and SDF results.

## Debugging and telemetry

`scene.dump()` includes the authored layer tree, source content, resolved parameters, bounds, quads, matrices, hit geometry, commands, masks, controls, and interaction regions.

`scene.stats()` and `renderer.stats()` report worker, glyph, cache, atlas, texture, buffer, frame-timing, and context-recovery metrics. Telemetry has fixed retention limits and excludes player text, font bytes, and asset payloads.

## Building and verification

The project uses a pinned container toolchain for FreeType and Emscripten targets; native builds need FreeType (found through pkg-config, or the `bundled-freetype` feature), and `animation-export` also needs libx264.

```sh
cargo test --workspace --all-features

cd crates/sekai-profile-wasm
npm run build
npm run typecheck
npm run test:gates
npm run verify:wasm:runtime
npm run measure:wasm:size
```

Release gates cover ABI/schema compatibility, shared-core source consistency, TMP debug parity, glyph SDF parity, complete-profile command coverage, atlas and cache budgets, worker singleflight, context restoration, bounded telemetry, npm contents, and the native/browser release matrix.

## License

The repository is AGPL-3.0-only. The browser npm package also includes the limited browser linking exception in `crates/sekai-profile-wasm/LICENSE-EXCEPTION`. Modified SDK builds, server use, and non-browser use remain subject to the full AGPL, including its network-use source requirement.
