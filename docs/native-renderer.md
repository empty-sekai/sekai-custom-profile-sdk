# Native renderer

This page covers the `sekai-profile` Rust library: building a
`CustomProfileRenderer`, rendering and encoding pages, render configuration
and errors, batches, animations, cache keys, the plain CPU canvas, Cargo
features and environment variables.

The renderer draws cards with its own CPU rasterizer. Text and shapes are
drawn as signed distance fields sampled from prebuilt atlases (see
[fonts, atlases and assets](fonts-and-assets.md)); masterdata, assets and
fonts are always supplied by the caller. The [command line](cli.md) is a thin
layer over this library.

## Dependency

```toml
[dependencies]
sekai-profile = { git = "https://github.com/empty-sekai/sekai-custom-profile-sdk", tag = "v<version>" }
serde_json = "1"
```

Building the crate needs FreeType; see [development](development.md#prerequisites).

## Rendering a page

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

- `set_font_dirs` sets the directories font files are looked up in. Call it
  before `with_atlas_dir`, which checks every text atlas against the font
  file of its family.
- `JsonMasterDataProvider::from_dir` reads `<dir>/<table>.json` for every
  table it knows. A missing required table is logged and skipped, and the
  elements that use it are not drawn; `missing_tables` lists them. A
  directory without any known table is an error. `empty` and `insert_table`
  build a provider table by table, and `with_region` sets the region
  (default `Region::Cn`).
- `AssetStore::new` takes the cache budget in MiB. `load_png_dir` keys each
  PNG by its path relative to the directory without `.png`;
  `put` adds one asset.
- `profile_page` picks a page in the order the game shows them, by
  ascending `seq`.
- `ProfileData::from_json` reads the player data of a Profile API response.

Any type that implements `masterdata::MasterDataProvider` can stand in for
`JsonMasterDataProvider`. `swap_masterdata` replaces the masterdata of a
running renderer; renders already in progress keep the masterdata they
started with. One renderer serves any number of concurrent renders.

The `ordered_pages` example in `crates/sekai-profile/examples/` renders every
page of a card list with the same calls.

## Rendered pages and encoding

`render` returns a `RenderedPage`: premultiplied RGBA8 pixels
(`premultiplied_rgba`), its size, its background, and the `telemetry` of the
render.

- `to_png` encodes a straight-alpha PNG.
- `to_jpeg(quality)` encodes a baseline JPEG; it needs the `jpeg-turbo`
  feature.
- `CustomProfileRenderer::encode_png` and `encode_jpeg` consume the page,
  return an `EncodedPage` with the encoding time added to its telemetry, and
  keep the pixel buffer, and for JPEG the encoder scratch, for the next
  render. They suit long-running services. `encode_jpeg` takes
  `JpegOptions`: `quality` (1 to 100, default 90) and `encoder`
  (`JpegEncoder::Standard`, or `JpegEncoder::Avx512Yuv420` on an x86-64 CPU
  with AVX-512).

A page with a transparent background encodes to PNG only.

## Render configuration

| Field | Values | Default | Notes |
| --- | --- | --- | --- |
| `executor` | `Auto`, `Simd`, `Scalar` | `Auto` | `Auto` runs `Simd` when the CPU supports it. `Simd` needs an x86-64 CPU with AVX-512 F, BW and VBMI, and FMA. |
| `background` | `White`, `Transparent` | `White` | A transparent page encodes to PNG only. |
| `tile_width` / `tile_height` | A power of two from 8 to 128 | `32` | The SDF executor works on tiles of this size. |
| `pixel_occlusion` | `Off`, `Measure`, `Cull` | `Off` | `Measure` counts the SDF pixels that later opaque images of the render-object store cover and reports the count in the telemetry; `Cull` skips drawing them. Neither changes the rendered pixels. |

`sdf_simd_available()` reports whether this CPU runs `SdfExecutor::Simd`.

## Errors and capabilities

`render` returns `RenderError`:

- `InvalidTileSize` names the rejected tile size;
- `SimdUnavailable` means the config asks for `Simd` on a CPU without it;
- `Render` carries the message of a failure during the render itself.

`CustomProfileRenderer::capabilities` returns `RenderCapabilities`: whether
this CPU runs the SIMD executor, how many text atlases are installed
(counting each family and sampling tier), and whether a shape atlas, a
render-object store and a fallback glyph cache are installed.

`with_atlas_dir` accepts a directory that does not exist and then installs
no text or shape atlas. Build the atlases first, or check
`capabilities().text_atlases` after installing them.

`validate_card` lists the colours, fonts, shapes and icons a card refers to
that the masterdata does not define. `prewarm` touches every installed
resource once so the first render does not pay for page faults and
allocations.

## Rendering every page of a profile

`render_batch(batch_key, locale, pages, profile, config)` takes
`(seq, &CustomProfileCard)` pairs, resolves all pages in one pass and
generates missing fallback glyphs once for all of them. It returns a
`RenderedBatch` holding the `CompiledProfileBatch`, the prepared render
objects when a store is installed, and each `RenderedPage` with its `seq` in
ascending `seq` order.

## Animations

With the `animation-export` feature, `render_animation` renders a card's
animation and `animation_preflight` reports whether a card animates without
drawing any frame. Both take an `AnimationDocument`:

- `key`: the key the scene's stable IDs derive from; the same card under the
  same key builds the same scene;
- `region`: the region code, such as `cn`, the scene is resolved for. It
  selects the locale of the fixed text in general panels.

`sekai_profile::animation::resolve_preset` turns a preset name into a
`ResolvedAnimationPreset`: the output format, frame rate, duration, size
limit and byte budgets. `AnimationFormat` is `Gif`, `Webp`, `Apng` or `Mp4`.
A static card returns `animated: false` without an artifact.

`render_animation`, `animation_preflight` and the functions of
`sekai_profile::animation` return `AnimationError` on failure. Among its
variants, `OutputBudgetExceeded` and `ExportMemoryExceeded` mean the
artifact or the memory the export holds outgrew the preset's budget,
`Render` carries the `RenderError` of the render itself, and the others name
the preset, encoder, artifact check or export step that failed.

The `animation_export` example runs cards through this pipeline.

## Caching rendered pages

`CustomProfileRenderer::cache_identity(config)` returns a key that changes
whenever anything that affects the pixels changes: the `RenderConfig`, the
manifest of every installed atlas and the published render-object store. A
service can use it as the cache key for encoded pages, so a changed atlas or
store never serves a stale page.

When several renders must see the same render-object store, take a
`RenderObjectGenerationPin` with `pin_render_object_generation` (or build one
with `RenderObjectGenerationPin::new`) and pass it to `render_pinned`,
`render_batch_pinned`, `render_animation_pinned` and
`cache_identity_pinned`.

## Render-object stores

A render-object store holds pre-decoded premultiplied RGBA images in
memory-mapped pages. Install one with `with_render_object_store`; images it
lacks are decoded from the asset store. `build-render-object-store` writes
stores, and `render_object::MappedRenderObjectStore::open` opens one by its
manifest.

## Plain CPU canvas

`sekai_profile::profile_compositor::canvas` provides a bounded canvas for
plain labels and graphics. `Canvas` reuses the profile compositor's image
sampling, source-over blending and shape rasterizer. It draws cropped and
scaled PNG images, rectangular clips, rectangles, rounded rectangles,
ellipses, quadratic strokes and FreeType plain-text labels. `Font` borrows
the font bytes, so callers can share one allocation. Rich TMP text still
goes through the profile pipeline.

Pixel buffers are premultiplied RGBA. `encode_png` writes a straight-alpha
PNG, and `crop` copies a region into a new canvas without re-encoding.
Canvas and decoded image buffers are limited to 128 MiB and encoded PNG
inputs to 32 MiB; aggregate cache and concurrency budgets are up to the
caller.

The `plain_canvas` example takes a font file and an output PNG path:

```sh
cargo run -p sekai-profile --example plain_canvas -- <font-file> <output.png>
```

## Cargo features

| Feature | Effect |
| --- | --- |
| `bundled-freetype` | Builds the bundled FreeType sources instead of finding FreeType through pkg-config |
| `jpeg-turbo` | JPEG encoding with the bundled libjpeg-turbo (`to_jpeg`, `encode_jpeg`) |
| `parallel` | Rasterizes on a dedicated rayon thread pool; the output is byte-identical to a serial build |
| `animation-export` | Animation rendering and the GIF, WebP, APNG and MP4 encoders; links libx264 |
| `dev` | Diagnostics for the SDF generators, used by the `compare_sdf` example |

No feature is enabled by default.

## Environment variables

| Variable | Effect |
| --- | --- |
| `SEKAI_PROFILE_FONT_DIR` | Font directory searched after the directories given to `set_font_dirs` |
| `FONT_DIR` | Font directory searched after `SEKAI_PROFILE_FONT_DIR`; `assets/fonts` is searched last |
| `SEKAI_PROFILE_SDF_EDT` | `1` to `4` generates glyph SDFs with a distance transform at that supersampling factor; unset or any other value uses the analytic method |
| `SEKAI_PROFILE_REALTIME_EDT_THREADS` | With the `parallel` feature, the number of threads that generate SDFs for glyphs missing from the atlas: `1` to `4`, default `2` |
| `SEKAI_PROFILE_DEBUG_TMP_PROBE`, `SEKAI_PROFILE_DEBUG_TEXT_CODEPOINTS` | `1`, `true` or `yes` emits `tracing` debug events for TMP layout probes and text codepoints |

The command line also reads `SEKAI_PROFILE_ATLAS_DIR`; see [command line](cli.md).
