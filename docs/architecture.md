# Architecture

This page describes how the SDK divides work between the Rust core, the
browser runtime and the host application, how a browser scene is built, and
which component owns each cache.

See also: [native renderer](native-renderer.md), [command line](cli.md),
[fonts, atlases and assets](fonts-and-assets.md),
[development](development.md), and the
[browser package README](../crates/sekai-profile-wasm/README.md).

## Crates

| Crate | Responsibility |
| --- | --- |
| `sekai-profile-core` | Backend-independent scene schema, profile and masterdata resolution, authoring documents, dynamics, stable IDs, masks, controls and interaction geometry |
| `sekai-profile` | Native CPU renderer, JSON masterdata provider, asset store, SDF atlas building and the plain canvas |
| `sekai-profile-cli` | The `sekai-profile` executable: `render`, `prepare` and `serve` |
| `sekai-profile-wasm` | Minimal FreeType WASM runtime, worker protocol, WebGL2 renderer, caches and the browser demo, published as the npm package `@empty-sekai/sekai-custom-profile-sdk` |

Both renderers resolve a card through `sekai-profile-core`. The native
renderer draws the resolved scene with its own CPU rasterizer; the browser
renderer draws it with WebGL2. Rules that decide which asset a card element
reads, such as the honor asset keys, live in the core so both renderers
request the same keys.

## Responsibilities

Rust and WASM own:

- game-structure-compatible authoring documents, transactions, gestures,
  undo/redo, checkpoints and exports;
- profile-card and masterdata resolution;
- TextMesh Pro rich-text parsing and layout;
- dynamic formulas and mutable scene state;
- FreeType metrics, glyph SDF generation and atlas placement;
- stable layer, command, control and interaction IDs;
- the layer tree, source content, resolved parameters, bounds, quads,
  matrices and hit geometry.

In the browser, the main thread owns:

- calling the application's asynchronous font, localization and image
  resource providers;
- deduplicating, scheduling and retaining decoded image resources;
- uploading atlases, command buffers and compact state tables to WebGL2;
- restoring GPU resources after a WebGL context loss.

The host application owns:

- profile data, fonts, masterdata and image assets;
- where resources come from: the network, local files, IndexedDB, signed
  requests, authenticated APIs or any other source;
- hover, click, selection, editing, copying, navigation and DOM or SVG
  overlays.

Text geometry and glyph pixels follow from the font bytes the host provides,
FreeType metrics, the TMP layout and the SDF pipeline. Each font is
identified by the SHA-256 of its bytes: the browser renderer rejects
different bytes registered under a family it already holds, and a native
text atlas installs only when the font file found for its family is the one
it was built from.

## Creating a browser scene

`BrowserRenderer.createProfileScene` builds one page of a profile in this
order:

1. WASM in the worker reads the profile, the card and the masterdata and
   reports the text the page localizes; TypeScript resolves it through the
   `LocalizationProvider` when one is configured.
2. WASM reports the font families the page uses. Prebuilt atlases from the
   `PrebuiltSdfAtlasProvider` are registered, and families not registered
   yet are requested from the `FontProvider`. Fonts registered with
   `registerFont` before the scene is created are used as they are.
3. WASM prepares the page with the localized text and returns its glyph
   requests and image resource descriptors.
4. The glyph atlas is built: glyphs come from the prebuilt atlases, or WASM
   generates their SDFs with FreeType and places them in the session atlas.
   A page whose text is only white space needs no atlas.
5. The `ResourceProvider` returns a `Blob`, `ArrayBuffer`, `Uint8Array` or a
   directly uploadable `TexImageSource` for each resource descriptor, with
   bounded concurrency.
6. The worker lays the text out as TextMesh Pro does and the core produces
   the authored layer tree, commands, controls, interaction regions and
   dynamic state.
7. WebGL2 uploads atlases, images, geometry and state buffers and draws the
   scene.
8. `advance()`, layer masks, tabs and scrolling reuse the existing timeline,
   layout and atlas and update only the affected state.
9. `dump()` and `stats()` expose the semantic data and bounded telemetry.

A missing or undecodable image produces a warning and a transparent
placeholder, and the rest of the scene is still built. Schema, ABI and
memory-budget violations fail with an error.

## Layers, state and interaction

Public layers correspond to the card elements the game authors. Shapes,
glyphs, masks and other draw primitives are commands owned by those layers.

```ts
await scene.advance(tick);
await scene.setLayerVisible(layerId, false);
await scene.setLayerMasks(layerTableRevision, overrides);
await scene.setTab(controlId, value);
await scene.scrollBy(controlId, delta);
scene.draw();

const dump = await scene.dump();
```

Changing layer visibility reuses the active timeline, layout cache, glyph
atlas and image cache. The renderer exposes interaction regions, control
bindings, resolved data and geometry; the application decides what a click
does, for example opening a character, event, card or story page.

## Native rendering

`CustomProfileRenderer` takes a card and optional player data, resolves the
scene through the core, orders the elements by layer and lowers each element
to ordered draw commands. Text and shapes are rasterized as signed distance
fields sampled from prebuilt atlases; images are decoded from the asset store
or read from a pre-decoded render-object store and composited by the same
CPU compositor. A missing image is drawn transparent. See
[native renderer](native-renderer.md) for the API.

## Caches and resource ownership

Browser:

- Font bytes and logical families, registered directly or supplied by a
  `FontProvider`, are the only source for font resolution and the glyph
  cache identity.
- The glyph session atlas is bounded and reused across the scenes owned by
  the same worker.
- Optional IndexedDB persistence stores opaque, version-checked glyph
  records only.
- Persistent caching of encoded images belongs to the `ResourceProvider`,
  which may use HTTP caching, Cache Storage, IndexedDB or application
  storage.
- The renderer keeps only a bounded session cache of decoded images and the
  GPU textures of the current context.
- Restoring a lost context reuploads the retained atlas, image and buffer
  data and reuses the completed TMP layout and SDF results.

Native:

- `AssetStore` keeps encoded PNG bytes in an LRU cache with a byte budget
  and decodes them on demand; static assets can be pinned outside the budget.
- Text and shape atlases and render-object stores are memory-mapped from
  disk and shared by every render of the renderer.
- Glyphs from the fallback face are generated while rendering and cached on
  disk under the atlas directory.

## Debugging and telemetry

`scene.dump()` returns the authored layer tree, source content, resolved
parameters, bounds, quads, matrices, hit geometry, commands, masks, controls
and interaction regions.

`scene.stats()` and `renderer.stats()` report worker, glyph, cache, atlas,
texture, buffer, frame-timing and context-recovery metrics. The `telemetry`
option of `BrowserRenderer.create` sets the level: `off`, `summary` (the
default) or `trace`. At most 240 samples are retained. Telemetry records
counts and timings; a sample that contains text content, user IDs, or font
paths or URLs is rejected.

Each native `RenderedPage` carries the `RenderTelemetry` of its render in
its `telemetry` field. The command line logs through `tracing` to stderr;
`RUST_LOG` sets the filter and defaults to `info`.
