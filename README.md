# Allium Renderer

Allium Renderer resolves and renders profile-card scenes from application-provided profile data, masterdata, fonts, and assets.

The repository contains two adapters over one shared semantic core:

- a native CPU renderer for server and CLI workloads;
- a WebGL2 browser renderer with minimal FreeType WASM.

The browser package does not compile or expose the native Skia renderer. Rust/WASM owns semantic resolution, TMP parsing, layout, dynamic formulas, glyph demand, FreeType measurement, SDF generation, and atlas placement. WebGL2 consumes the resulting command and state buffers.

## Workspace

| Crate | Purpose |
| --- | --- |
| `allium-renderer-core` | Backend-independent scene schema, profile resolution, layout state, dynamics, stable IDs, masks, controls, and interaction geometry |
| `allium-renderer` | Native CPU/Skia adapter and reusable renderer components |
| `allium-renderer-host` | Shared native host utilities and JSON masterdata provider |
| `allium-renderer-cli` | Native `render-card` command and long-running NDJSON service mode |
| `allium-renderer-wasm` | Minimal FreeType WASM, stateful worker protocol, WebGL2 runtime, caches, and Scene Workbench |

## Browser renderer

Version 0.2 is a breaking replacement for the 0.1 CPU-image WASM API. It has one public entrypoint, `BrowserRenderer`, and one production backend, WebGL2. There is no Skia WASM artifact, encoded-image API, legacy image-layer worker, or compatibility shim.

The host supplies profile/card JSON and fonts. Masterdata and assets use configurable URL providers; the defaults use the Empty Sekai CDN. The host does not parse TMP, enumerate glyphs, build an atlas, or precompute dynamic frames.

```ts
import { BrowserRenderer } from "@empty-sekai/renderer-wasm";

const renderer = await BrowserRenderer.create({ canvas, region: "en" });
await renderer.registerFont({ family: "Profile Font 1", bytes: fontBytes });

const masterData = await renderer.loadMasterData("latest");
const scene = await renderer.createProfileScene({
  masterData,
  documentKey: "profile-preview",
  card,
  profile,
});

scene.draw();
```

The stateful API exposes authored layers, source content, resolved parameters, bounds, quads, matrices, hit geometry, component controls, masks, and privacy-safe telemetry. The host owns navigation, hover, copying, selection, editing, and DOM/SVG overlays.

See [the browser package README](crates/allium-renderer-wasm/README.md), [runtime architecture](docs/browser-runtime.md), [scene schema](docs/scene-schema.md), [resource providers](docs/resource-providers.md), [debugging and telemetry](docs/debug-and-telemetry.md), and [the 0.2 migration guide](docs/migrating-to-0.2.md).

## Native renderer

The native adapter remains available for server-side still images and application-specific scenes. It uses the same semantic core contracts wherever profile-card behavior overlaps with the browser runtime. Native drawing stays outside the browser dependency graph.

The `skia` feature enables the production native preset. `parallel` controls the dedicated SDF raster pool; `scenes` enables non-profile application scenes. Resource bytes and masterdata are always supplied by the host application.

```sh
cargo test --workspace --all-features
cargo run --release --bin render-card -- profile.json output.jpeg
```

Build and test inside the repository development container so FreeType, Skia, Emscripten, and target versions match CI.

## Correctness gates

The release process covers:

- append-only ABI and scene-schema compatibility;
- shared-core source and golden-profile drift checks;
- TMP debug and glyph SDF parity;
- native/WASM state and final-pixel parity;
- complete-profile WebGL command coverage;
- atlas/cache budgets, worker singleflight, and context restoration;
- performance and bounded-memory soak tests;
- npm contents, public-language, and forbidden-resource audits.

Visibility and component-state mutations must not rebuild the timeline, layout, command stream, glyph atlas, or persistent glyph cache.

## Release artifacts

Tagged releases build the Skia-enabled `render-card` CLI on native Linux, Windows, and macOS runners, plus an isolated, statically linked Alpine musl build. The release contains `tar.xz` archives for Linux x86_64 GNU, Linux x86_64 musl, Linux AArch64 GNU, and both macOS architectures; Windows x86_64 is provided as a zip. `SHA256SUMS` covers every native archive and the complete browser WASM zip.

Manual workflow runs exercise the same native matrix, browser gates, package assembly, and GitHub Pages deployment without publishing npm packages or creating a GitHub Release.

## Licenses

The repository is AGPL-3.0-only. The npm browser package includes the limited browser linking exception in `crates/allium-renderer-wasm/LICENSE-EXCEPTION` and the required FreeType notices. Modified renderer builds and server or non-browser use remain subject to the full AGPL, including its network-use source requirement.
