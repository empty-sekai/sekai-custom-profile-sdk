# @empty-sekai/renderer-wasm

`@empty-sekai/renderer-wasm` is the WebGL2 browser runtime for Allium profile-card scenes. Rust/WASM owns profile resolution, TMP rich-text parsing, layout, dynamic formulas, stable semantic IDs, glyph demand, FreeType metrics, SDF generation, and atlas placement. TypeScript performs persistent-cache I/O and GPU resource orchestration; WebGL2 draws the resulting semantic command stream.

Version 0.2 is a breaking replacement for the 0.1 CPU image API. It does not ship a Skia/CPU renderer, image-layer worker, or compatibility shim.

## Runtime requirements

- WebGL2
- Web Workers and ES modules
- IndexedDB for optional persistent glyph records
- Cache Storage for finite versioned renderer-static assets when available
- application-provided profile/card JSON and font files

The package never uses `CanvasRenderingContext2D.fillText`, `measureText`, or browser font fallback as rendering truth.

## Resource boundary

No fonts, player profiles, masterdata snapshots, or game images are bundled.

Here, **host application** means package-external code such as a profile site or the demo. It provides:

- profile and card JSON;
- each required font as an `ArrayBuffer`;
- optional masterdata and asset URL resolvers.

The default resolvers read masterdata, versioned renderer cuts under `renderer-static/v0.2/`, and region-specific unpacked game assets from `https://cdn.emptysekai.com`. Resource namespaces come from the shared core; hosts do not infer them from filenames. Fonts are always supplied by the host.

The host does **not** parse TMP, enumerate glyphs, construct an atlas, calculate layout, or provide dynamic-program metadata. Those operations remain inside the renderer worker and WASM runtime.

The package-internal TypeScript runtime is an I/O orchestrator, not another text engine. It consumes structured glyph demand produced by WASM, checks session and IndexedDB caches, and returns cached records plus missing requests to the worker. WASM performs FreeType rasterization, SDF generation, atlas packing, UV assignment, revisions, and eviction; TypeScript uploads the resulting dirty atlas pages to WebGL2.

## Basic usage

```ts
import { BrowserRenderer } from "@empty-sekai/renderer-wasm";

const renderer = await BrowserRenderer.create({
  canvas: document.querySelector("canvas")!,
  region: "en",
});

await renderer.registerFont({
  family: "Profile Font 1",
  bytes: await fetch("/fonts/profile-font-1.ttf").then((response) => response.arrayBuffer()),
});

const masterData = await renderer.loadMasterData("latest");
const scene = await renderer.createProfileScene({
  masterData,
  documentKey: "profile-preview",
  card,
  profile,
});

scene.draw();
```

Destroy scenes and the renderer when their owner is disposed:

```ts
await scene.destroy();
await masterData.destroy();
renderer.destroy();
```

## State and interaction

Renderer state mutations update compact GPU state buffers. They do not recreate the timeline, semantic command stream, layout, glyph atlas, or persistent glyph cache.

```ts
await scene.advance(tick);
await scene.setLayerVisible(layerId, false);
await scene.setLayerMasks(layerTableRevision, overrides);
await scene.setTab(controlId, value);
await scene.scrollBy(controlId, delta);
scene.draw();
```

Public layers correspond to game-authored card elements. Internal draw primitives are commands, not extra public layers.

The renderer exposes interaction regions, control bindings, stable IDs, resolved data, geometry, source content, and numeric text regions. Hover, navigation, copying, selection, editing, scrolling policy, and DOM/SVG overlays remain application behavior.

```ts
const dump = await scene.dump();
const numericRegions = dump.numeric_text_regions;
```

## Caching

- The in-memory session atlas is bounded, leased, and shared across scenes owned by the same worker.
- IndexedDB stores opaque, validated glyph records only. It never stores source text, profiles, font files, layout, commands, timelines, or masks.
- All encoded CDN assets may use the browser HTTP cache. Only immutable `renderer-static/v0.2/` cuts additionally enter Cache Storage; mutable region game assets revalidate through HTTP caching.
- Decoded images and GPU textures are bounded session resources.
- A WebGL context restore reuploads retained atlas/image/buffer state without rerunning TMP parsing, layout, or SDF generation.

Set `sdf.persistence` to `"memory-only"` for session-only glyph caching. The default is `"origin"`, which uses IndexedDB and safely falls back to memory when persistent storage is unavailable.

## Debugging and telemetry

`scene.dump()` returns the complete semantic scene description, including the authored layer tree, source content, resolved parameters, bounds, quads, matrices, hit geometry, commands, masks, and component controls.

`scene.stats()` and `renderer.stats()` expose bounded, privacy-safe runtime metrics for worker activity, glyph generation, cache hits, atlas pages, texture uploads, GPU buffers, frame timings, and context recovery. Telemetry never includes player content, TMP source strings, font bytes, or asset payloads.

## Custom providers

Override URL construction without moving parsing or rendering policy into the host:

```ts
const renderer = await BrowserRenderer.create({
  canvas,
  region: "en",
  resolveMasterDataUrl: (table, region, revision) =>
    `/renderer-data/${region}/${revision}/${table}.json`,
  resolveResourceUrl: (namespace, key, region) =>
    `/renderer-assets/${region}/${namespace}/${key}.png`,
});
```

## Building from source

The supported build runs in the repository development container. The toolchain is pinned by `Dockerfile` and `build.sh`.

```sh
npm run build
npm run typecheck
npm run test:gates
npm run verify:wasm:runtime
npm run measure:wasm:size
```

The FreeType build enables only the TrueType, CFF, SFNT, PS auxiliary/name, and smooth raster modules required by the runtime. CPU EDT is the production SDF backend; the analytic backend is an explicit debug option.

## License

AGPL-3.0-only with the browser linking exception in `LICENSE-EXCEPTION`. The exception applies to unmodified browser use of this package; modified renderer builds and server or non-browser use remain subject to the full AGPL, including the network-use source requirement.
