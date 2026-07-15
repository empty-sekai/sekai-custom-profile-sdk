# Allium Renderer

[简体中文](README.md) | [English](README.en.md)

Allium Renderer resolves application-provided player profiles, card data, masterdata, fonts, and image assets into renderable custom profile-card scenes.

The repository contains two adapters for different environments:

- a native CPU/Skia renderer for servers, CLI tools, and offline jobs;
- a browser WebGL2 renderer backed by Rust/WASM, FreeType, and glyph SDF atlases.

Both adapters share a backend-independent Rust semantic core. The browser package uses a dedicated Rust/WASM and WebGL2 execution path, while the native adapter uses Rust and Skia.

## Responsibilities

Rust/WASM owns:

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
| `allium-renderer-core` | Backend-independent scene schema, profile resolution, dynamics, stable IDs, masks, controls, and interaction geometry |
| `allium-renderer` | Native CPU/Skia adapter and reusable native renderer components |
| `allium-renderer-host` | Native host utilities and JSON masterdata provider |
| `allium-renderer-cli` | `render-card` CLI and long-running NDJSON service mode |
| `allium-renderer-wasm` | Minimal FreeType WASM, stateful worker protocol, WebGL2 runtime, caches, and Scene Workbench |

## Browser quick start

The 0.2 browser API starts with `BrowserRenderer`. A required `ResourceProvider` interprets renderer-provided semantic descriptors with arbitrary asynchronous host logic; the package does not prescribe resource locations, request protocols, or storage forms.

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

See the [browser package README](crates/allium-renderer-wasm/README.en.md) for the complete integration flow, provider contract, caching model, and interaction API.

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

The native adapter serves still-image, CLI, and application-specific scene workloads. Resource bytes and masterdata are always host-provided. Enable the `skia` feature for the native production preset.

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

The project uses a pinned container toolchain for FreeType, Skia, and Emscripten targets.

```sh
cargo test --workspace --all-features

cd crates/allium-renderer-wasm
npm run build
npm run typecheck
npm run test:gates
npm run verify:wasm:runtime
npm run measure:wasm:size
```

Release gates cover ABI/schema compatibility, shared-core drift, TMP debug parity, glyph SDF parity, complete-profile command coverage, atlas and cache budgets, worker singleflight, context restoration, bounded telemetry, npm contents, and the native/browser release matrix.

## License

The repository is AGPL-3.0-only. The browser npm package also includes the limited browser linking exception in `crates/allium-renderer-wasm/LICENSE-EXCEPTION`. Modified renderer builds, server use, and non-browser use remain subject to the full AGPL, including its network-use source requirement.
