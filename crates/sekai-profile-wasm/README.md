# @empty-sekai/sekai-custom-profile-sdk

English | [简体中文](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/crates/sekai-profile-wasm/README.zh-CN.md)

An unofficial browser SDK for Project SEKAI custom profiles. It renders custom-profile cards with WebGL2 from a profile API response, edits game-compatible custom-profile documents, exports PNG snapshots, and reports layers, geometry, and interaction regions for the host to build its own UI on.

Rich text and layout follow a compatibility model of the Unity TextMesh Pro data and behavior the game uses. The model covers the tags, layout, materials, and dynamic behavior the runtime implements and tests. Behavior outside the model, and later game updates, can differ from the game client, so exact reproduction of the client's rendering or pixels is not guaranteed.

A Rust/WebAssembly runtime in a dedicated worker resolves the profile, lays out text with FreeType metrics, and generates glyph SDFs into an atlas. The main thread calls the host's providers, decodes images, and drives WebGL2. The host supplies fonts, profile data, masterdata, and images; the package contains none of them. The [architecture overview](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/docs/architecture.md) describes the design, and the [repository README](https://github.com/empty-sekai/sekai-custom-profile-sdk#readme) covers the native renderer and CLI.

## Contents

- [Installation](#installation)
- [Quick start](#quick-start)
- [Core concepts](#core-concepts)
- [Authoring](#authoring)
- [Interaction and state](#interaction-and-state)
- [Caching and resource ownership](#caching-and-resource-ownership)
- [Errors and warnings](#errors-and-warnings)
- [Telemetry and debugging](#telemetry-and-debugging)
- [Browser support and building](#browser-support-and-building)
- [Exports](#exports)
- [License](#license)

## Installation

```sh
npm install @empty-sekai/sekai-custom-profile-sdk
```

The package is an ES module with TypeScript declarations. `BrowserRenderer.create()` and `BrowserAuthoringClient.create()` start a module worker from `dist/worker.js`, which loads `dist/sekai_profile_wasm.js` and `dist/sekai_profile_wasm.wasm`. These URLs resolve relative to the package's own modules. When a bundler or deployment moves the files, serve `dist/` as one directory (the worker imports sibling modules) and pass the locations:

| Option | Default | Purpose |
| --- | --- | --- |
| `workerUrl` | `dist/worker.js` | Module worker script. |
| `moduleUrl` | `dist/sekai_profile_wasm.js` | Emscripten loader the worker imports. |
| `wasmUrl` | `dist/sekai_profile_wasm.wasm` | WebAssembly binary. |
| `workerFactory` | `new Worker(url, { type: "module" })` | Creates the worker from `workerUrl`, for hosts that construct workers themselves. |

## Quick start

```ts
import {
  BrowserRenderer,
  type FontProvider,
  type ResourceProvider,
} from "@empty-sekai/sekai-custom-profile-sdk";

// loadApplicationFont, resolveResourceRequest and loadApplicationMasterData
// stand for the host's own font, asset and masterdata sources.
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
    const response = await fetch(request, { signal });
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
  ({ table, region, revision, optional }, { signal }) =>
    loadApplicationMasterData({ table, region, revision, optional, signal }),
);

const scene = await renderer.createProfileScene({
  masterData,
  documentKey: "profile-preview",
  profile, // the full profile API response
  frameMode: "animate",
});

scene.draw();
const png = await scene.exportPng(); // 1830 × 812 PNG Blob
```

`profile` is the full profile API response. The renderer orders the pages in `profile.userCustomProfileCards` by ascending `seq`, as the game shows them, and renders the first one or the one `pageIndex` selects. The rest of the response supplies the player data the card draws, such as deck cards, honors, and music results.

## Core concepts

### BrowserRenderer

A `BrowserRenderer` owns one WebGL2 context, one worker, and the provider queues. Create one per canvas and reuse it for every scene drawn on that canvas.

`BrowserRenderer.create()` sets the canvas to the card size, 1830 × 812, and rejects with `WEBGL2_UNAVAILABLE` when the canvas cannot provide a WebGL2 context.

| Option | Default | Description |
| --- | --- | --- |
| `canvas` | required | `HTMLCanvasElement` or `OffscreenCanvas`. |
| `region` | required | Game region: `cn`, `jp`, `tw`, `en`, or `kr`. It is the masterdata session region, the `region` of every font request, and the default scene locale. |
| `resourceProvider` | required | Image source; see [ResourceProvider](#resourceprovider). |
| `resourceConcurrency` | `8` | Maximum concurrent `ResourceProvider.provide()` calls. |
| `fontProvider` | none | Font source; see [FontProvider](#fontprovider). |
| `fontConcurrency` | `4` | Maximum concurrent `FontProvider.provide()` calls. |
| `localizationProvider` | none | Source of renderer-owned UI text; see [LocalizationProvider](#localizationprovider). |
| `localizationConcurrency` | `8` | Maximum concurrent `LocalizationProvider.provide()` calls. |
| `prebuiltSdfAtlasProvider` | none | Source of prebuilt glyph atlases; see [Prebuilt atlas packages](#prebuilt-atlas-packages). |
| `telemetry` | `{ level: "summary" }` | See [Telemetry and debugging](#telemetry-and-debugging). |
| `workerUrl`, `moduleUrl`, `wasmUrl`, `workerFactory` | package files | See [Installation](#installation). |

Concurrency values must be positive integers.

| Method | Description |
| --- | --- |
| `loadMasterData(revision, loadTable, options?)` | Creates a masterdata session, fills it through a table loader, and seals it. |
| `createMasterData(revision)` | Creates an empty masterdata session for table-by-table loading. |
| `registerFont({ family, bytes })` | Registers font bytes (`ArrayBuffer`) under a family name. |
| `createProfileScene(options)` | Resolves one profile page and returns a `BrowserScene`. |
| `stats()` | Worker, provider, cache, and scene counters. |
| `restoreContext()` | Recreates a lost WebGL2 context and reuploads every scene. |
| `destroy()` | Terminates the worker and aborts work in progress. |

### Masterdata

A masterdata session holds the game tables one region and revision need. `loadMasterData()` asks the host's loader for every table and seals the session. The host decides table naming, location, transport, authentication, and caching:

```ts
const masterData = await renderer.loadMasterData(
  "catalog-2026-07",
  async ({ table, region, revision, optional }, { signal }) => {
    const response = await fetch(`/masterdata/${region}/${revision}/${table}.json`, { signal });
    if (optional && response.status === 404) return null;
    if (!response.ok) throw new Error(`${table}: HTTP ${response.status}`);
    return response.json();
  },
  { concurrency: 4 },
);
```

The loader receives `{ table, region, revision, optional }` and an `AbortSignal`, and returns the parsed table. `options.concurrency` defaults to 4; `options.signal` aborts loading. A failed required table rejects the call and destroys the session.

`masterData.requiredTables` lists the tables every session needs. `masterData.optionalTables` lists the tables a region may lack: `customProfileCharacterIconResources`, `customProfileMaterialResources`, and `customProfileUserInterfaceIconResources`, which only some regions ship, and `omikujis`, which only omikuji collections read. For these the loader is called with `optional: true`; returning `null` or `undefined` means the region does not ship the table, and a failed optional load is skipped. Elements that draw from a missing table are left out, and the rest of the card renders unchanged.

For table-by-table control, fill a session directly:

```ts
const masterData = await renderer.createMasterData("catalog-2026-07");

for (const table of masterData.requiredTables) {
  await masterData.putTable(table, await applicationMasterData.load(table));
}
for (const table of masterData.optionalTables) {
  const value = await applicationMasterData.loadIfPresent(table);
  if (value != null) await masterData.putTable(table, value);
}

await masterData.seal();
```

A session must be sealed before it can create scenes. One session serves any number of scenes; call `masterData.destroy()` once its scenes are done.

### ResourceProvider

The renderer describes every image it needs with a semantic descriptor. The host maps descriptors to URLs, object storage, files, authenticated requests, or any other source.

```ts
type ResourceDescriptor = {
  id: string;
  namespace: string;
  key: string;
  role: string;
  provenance: Record<string, unknown>;
  expectedSize?: { width: number; height: number };
};

interface ResourceProvider {
  provide(
    descriptor: ResourceDescriptor,
    context: { signal: AbortSignal },
  ): Promise<{ source: Blob | ArrayBuffer | Uint8Array | TexImageSource } | null>;

  cacheIdentity?(descriptor: ResourceDescriptor): string | null;
}
```

- `id` is the stable descriptor identity. `namespace` and `key` name the resource; `role` and `provenance` say which part of the card asked for it. These are renderer semantics, not filesystem paths; the host interprets them.
- `source` may be encoded image bytes (`Blob`, `ArrayBuffer`, `Uint8Array`), which the renderer decodes with `createImageBitmap()`, or an already decoded `TexImageSource` such as an `ImageBitmap`.
- `cacheIdentity()` sets the key of the decoded-image session cache. Include every catalog revision, region, user scope, or authentication domain that can change the bytes. Without it, the descriptor `id` is the key.

A provider can combine memory, Cache Storage, IndexedDB, user-selected files, a Service Worker, signed URLs, or authenticated APIs, and keeps full control of path mapping and invalidation:

```ts
const provider: ResourceProvider = {
  cacheIdentity(descriptor) {
    return `${assetRevision}:${descriptor.id}`;
  },

  async provide(descriptor, { signal }) {
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

Scheduling and failures:

- Requests within one scene are deduplicated by descriptor `id`. Concurrent scenes of one renderer share requests in flight.
- Each waiting scene observes its own cancellation. The provider's signal aborts only after every waiter has left.
- Decoded-cache hits do not call the provider.
- A `null` result, a thrown error, or a decode failure becomes a warning and a transparent placeholder; the rest of the scene renders. See [Errors and warnings](#errors-and-warnings).

Two kinds of collection request more than their own image. A can-badge collection (`customProfileResourceCollectionType` `can_badge`) also requests its normal map, `namespace: "static"`, `key: "ui/sekai_badge_normal"`. An omikuji collection draws the fortune slip of the `omikujis` row its `targetId` names and requests the slip cover and the fortune image as two `assets` resources.

### FontProvider

Font bytes always come from the host. A `family` is the logical font name masterdata and text elements use; the renderer passes it unchanged to the host.

```ts
type FontRequest = { region: string; family: string };

interface FontProvider {
  provide(
    request: FontRequest,
    context: { signal: AbortSignal },
  ): Promise<{ bytes: ArrayBuffer | Uint8Array } | null>;
}
```

For each scene the worker reports the families the scene uses, and the renderer asks the provider for those not yet registered, at most `fontConcurrency` at a time. Returned bytes are copied, hashed with SHA-256, and registered. A `null` result, an empty file, or a thrown error rejects scene creation and aborts the other requests still in flight. Supply TrueType or OpenType files, including `.ttc` collections; the bundled FreeType build has no zlib or Brotli support, so it does not open WOFF or WOFF2 files.

Hosts that manage fonts themselves can register them before creating scenes instead:

```ts
const bytes = await file.arrayBuffer();

await renderer.registerFont({ family: "FOT-RodinNTLGPro-DB", bytes });
await renderer.registerFont({ family: "Application Alias", bytes });
```

One file can be registered under several family names. The first registration of a family fixes its source hash for the renderer's lifetime: registering the same bytes again does nothing, and different bytes reject with `FONT_IDENTITY_CONFLICT`. Create a new renderer to switch font versions. Glyph identity includes the region, family, source hash, and character, so cached glyphs never mix font versions.

Omikuji slip texts are uGUI text drawn from FreeType coverage of the font file, not from an SDF atlas. The slip font is `FOT-Omikuji` for 2022 slips and `FOT-UDMinchoPro-B` for 2023–2025 slips. A character the slip font has no glyph for comes from the first fallback face that has one: `Roboto`, then the Noto Sans CJK face of the masterdata region. The `FontProvider` is asked for all three families and returns whole files; the renderer opens the face listed below. These families are never looked up in a prebuilt atlas, and a missing one rejects scene creation like any other font the scene uses.

| Family | File | Face |
| --- | --- | --- |
| `FOT-Omikuji` | `FOT-Omikuji.otf` | 0 |
| `FOT-UDMinchoPro-B` | `FOT-UDMinchoPro-B.otf` | 0 |
| `Roboto` | `Roboto-Regular.ttf` | 0 |
| `Noto Sans CJK SC` (region `cn`, `en`, and any other) | `NotoSansCJK-Regular.ttc` | 2 |
| `Noto Sans CJK TC` (region `tw`) | `NotoSansCJK-Regular.ttc` | 3 |
| `Noto Sans CJK JP` (region `jp`) | `NotoSansCJK-Regular.ttc` | 0 |
| `Noto Sans CJK KR` (region `kr`) | `NotoSansCJK-Regular.ttc` | 1 |

The two fallback files are the stock Android 14 system fonts; [Fonts and assets](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/docs/fonts-and-assets.md) lists their SHA-256 digests.

### LocalizationProvider

Renderer-owned UI text, such as General panel titles, tab labels, and difficulty names, is requested by stable localization key. Player names, signatures, and other user content come from the profile unchanged.

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

The worker reports the keys the scene uses. The renderer resolves each unique key once and hands the worker one immutable `key → text` snapshot. A provider that returns `null` or throws rejects scene creation; the renderer does not fall back to another region's text. Without a `LocalizationProvider`, the runtime uses its built-in tables for the region.

### Scenes

`renderer.createProfileScene(options)` resolves one page into a `BrowserScene`.

| Option | Default | Description |
| --- | --- | --- |
| `masterData` | required | A sealed session created by the same renderer. |
| `documentKey` | required | Caller-chosen document key. Stable layer IDs derive from it and from each element's position in the card, so rebuilding a page with the same key gives the same IDs. |
| `profile` | required | Full profile API response containing `userCustomProfileCards`. |
| `pageIndex` | `0` | Zero-based page in ascending `seq` order. |
| `locale` | renderer `region` | Locale of renderer-owned UI text. |
| `frameMode` | `"animate"` | `"animate"` starts dynamic programs at tick 0; `"final"` settles them to their final state during creation. |
| `sdf.backend` | `"edt"` | Glyph SDF generator: `"edt"` (CPU Euclidean distance transform) or `"analytic"`, a debugging and comparison path. |
| `sdf.supersample` | `2` | Supersampling factor of the EDT generator. |
| `sdf.persistence` | `"origin"` | Glyph cache scope: `"origin"` (IndexedDB) or `"memory-only"`. |
| `signal` | none | Cancels creation while it runs. |

A profile without `userCustomProfileCards`, a page without `customProfileCard`, or a `pageIndex` outside the pages rejects with `INVALID_PROFILE_INPUT`.

Scene creation runs these steps:

1. With a `LocalizationProvider`, the worker reports the localization keys the page needs, and the renderer resolves them into one text snapshot.
2. With a `FontProvider` or a prebuilt atlas provider, the worker reports the font families the page uses. The renderer reads prebuilt atlas manifests for them and requests unregistered families from the `FontProvider`.
3. The worker resolves the page against masterdata and reports its glyph demand, layout request, and resource descriptors. Glyphs come from a prebuilt atlas when it covers every glyph of the scene; otherwise the worker generates the atlas from the registered fonts and reuses cached glyphs.
4. The renderer requests the images from the `ResourceProvider`.
5. The worker lays out text and compiles the layer tree, semantic commands, control bindings, interaction regions, and initial dynamic state. The renderer uploads atlas pages, images, geometry, and state buffers to WebGL2.

After that, the host decides when to draw, advance the timeline, change state, or read a dump.

| Member | Description |
| --- | --- |
| `draw()` | Draws the current state and returns the frame's GPU counters. Call it after every state change. |
| `exportPng()` | Draws, then encodes the canvas as a 1830 × 812 PNG `Blob`. Needs Canvas 2D in a document, or `OffscreenCanvas` elsewhere. |
| `advance(tick)` | Moves the dynamic timeline to the absolute `tick`, at 60 ticks per second. A tick before the current one replays the programs from the start. |
| `animated` | `true` when the scene has dynamic programs that need `advance()`. |
| `dynamicProgramCount` | Number of dynamic programs in the scene. |
| `dump()` | Layer table, geometry, and interaction data; see [Interaction and state](#interaction-and-state). |
| `stats()` | Runtime snapshot; see [Telemetry and debugging](#telemetry-and-debugging). |
| `destroy()` | Releases image leases, atlas leases, GPU resources, and the worker-side scene. |

Cancellation and teardown:

```ts
const controller = new AbortController();
const pending = renderer.createProfileScene({
  masterData,
  documentKey: "preview",
  profile,
  signal: controller.signal,
});
controller.abort(); // `pending` rejects if creation was still running

const scene = await renderer.createProfileScene({ masterData, documentKey: "preview", profile });
await scene.destroy();
await masterData.destroy();
renderer.destroy();
```

The signal cancels only a creation in progress; release a created scene with `destroy()`. `renderer.destroy()` terminates the worker and aborts every provider request still running.

## Authoring

`BrowserAuthoringClient` edits custom-profile documents in the game's format inside its own worker. A document holds the `userCustomProfileCards` pages; importing a full profile response keeps only that array, and exports carry no worker handles, element IDs, selection, or other editor state.

```ts
import {
  BrowserAuthoringClient,
  type AuthoringCommand,
} from "@empty-sekai/sekai-custom-profile-sdk";

const authoring = await BrowserAuthoringClient.create();
const doc = profile
  ? await authoring.importProfile(profile)
  : await authoring.createBlank();

const command: AuthoringCommand = {
  kind: "set_transform",
  id: selectedElementId,
  position: [120, -40, 0],
};
const delta = await doc.apply(command);

await doc.undo();
await doc.redo();
const exported = await doc.export(); // { userCustomProfileCards: [...] }

await doc.destroy();
authoring.destroy();
```

`BrowserAuthoringClient.create()` accepts the same `workerUrl`, `moduleUrl`, `wasmUrl`, and `workerFactory` options as the renderer. `createBlank()`, `importProfile(profile)`, and `restoreCheckpoint(checkpoint)` each return a `RendererAuthoringDocument`:

| Member | Description |
| --- | --- |
| `apply(command)` | Applies one `AuthoringCommand` as one history entry. |
| `undo()`, `redo()` | Steps through history; resolve to `null` when there is nothing to undo or redo. |
| `select(id)` | Selects an element, or clears the selection with `null`. |
| `elements()` | Lists every element with its `id`, `page`, `category`, `index`, and element object. |
| `beginGesture(id)`, `previewGesture(command)`, `commitGesture()`, `cancelGesture()` | Continuous edits of one element; see below. |
| `appendPage()`, `duplicatePage(page)`, `deletePage(page)`, `movePage(fromPage, page)` | Page operations. |
| `export()` | Returns the document as a `GameProfileDocument`. |
| `checkpoint()` | Returns an `AuthoringCheckpoint` that `restoreCheckpoint()` reopens with its history and selection. |
| `id`, `initialDocument`, `initialRevision` | Session handle, and the document and revision it opened with. |
| `destroy()` | Releases the worker-side session. |

`AuthoringCommand` kinds:

| `kind` | Fields | Effect |
| --- | --- | --- |
| `create` | `page`, `category`, `element` | Adds an element in the game's format, including `objectData`. |
| `duplicate` | `id` | Copies an element. |
| `delete` | `id` | Removes an element. |
| `set_transform` | `id`, `position?`, `scale?`, `rotation?` | Sets `[x, y, z]` position and scale or `[x, y, z, w]` rotation. |
| `set_lock` | `id`, `lock` | Locks or unlocks an element. |
| `set_visible` | `id`, `visible` | Shows or hides an element. |
| `set_parameters` | `id`, `values` | Sets element fields other than `objectData`. |
| `change_layer` | `id`, `layer` | Moves an element within its page's layer order. |

Every operation resolves to an `AuthoringDelta`: the new `revision`, the element `changes` (`inserted`, `updated`, `removed`), `pageChanges` (`inserted`, `removed`, `moved`), `canUndo`, `canRedo`, and the current selection. Apply these changes to the host's scene and UI; the history lives in the worker, so the host keeps no second command history.

A gesture covers a continuous edit such as a drag. `beginGesture(id)` starts it, `previewGesture()` applies `set_transform` or `set_parameters` commands for that element without recording history, `commitGesture()` records one history entry, and `cancelGesture()` restores the element. Other document commands are rejected while a gesture is active, and a checkpoint cannot be taken during one. To move a rendered layer during a gesture without rebuilding the scene, call `scene.setLayerPreviewTransform(layerId, matrix)` with an affine matrix `[a, b, c, d, e, f]` in canvas pixels (origin at the top left, y down), applied as `x' = a·x + c·y + e` and `y' = b·x + d·y + f`; `null` clears it. The call updates one GPU matrix and draws.

The authoring core validates what the game's editor allows: at most 150 elements per page and 10 pages, the 12 element arrays every page carries, finite transform values, layer order, and the `objectData` shape. History keeps the latest 150 entries. Page indices are positions in ascending `seq` order, the same order as the renderer's `pageIndex`, and page operations renumber `seq` from 1. To render an edited document, pass the exported `userCustomProfileCards` as part of the `profile` given to `createProfileScene()`.

## Interaction and state

Scene state changes run in the worker and update compact GPU buffers. They reuse the layer table, timeline, layout, and glyph atlas, and keep the current playback position.

```ts
await scene.advance(tick);
await scene.setLayerVisible(layerId, false);
await scene.setLayerMasks(dump.revisions.layer_table, [
  { layerId, visible: true },
  { layerId: otherLayerId, visible: null },
]);
await scene.setTab(controlId, value);
await scene.setScrollOffset(controlId, offset);
await scene.scrollBy(controlId, delta);
scene.draw();
```

| Method | Description |
| --- | --- |
| `setLayerVisible(layerId, visible)` | Sets the render mask of one layer. |
| `setLayerMasks(layerTableRevision, overrides)` | Sets several masks in one update; `visible: null` clears an override. The revision must equal `dump.revisions.layer_table`, or the update is rejected. |
| `setTab(controlId, value)` | Selects a tab of a tab control. |
| `setScrollOffset(controlId, offset)` | Sets a scroll offset; the renderer clamps it to the content range. |
| `scrollBy(controlId, delta)` | Scrolls by `delta`. |
| `setLayerPreviewTransform(layerId, matrix)` | Applies a GPU-only preview transform and draws. |

Each asynchronous method resolves to `{ delta, gpu }`: `delta` is the worker's `CoreSceneDelta` (revision, tick, dirty flags, layer and command patches) and `gpu` reports the bytes uploaded. They do not draw; call `scene.draw()` afterwards.

Public layers correspond to game-authored elements. Shapes, glyphs, masks, and other draw primitives are commands owned by those layers.

### Scene dumps

`await scene.dump()` returns a `CoreSceneDump` in card device coordinates (`coordinate_space: "card-device-v1"`, 1830 × 812). It contains:

- `layer_table` and `layer_tree`: stable layer IDs, slots, parents, and subtree ranges;
- `layers`: source content, resolved parameters, authored visibility, render masks, and dynamic state;
- `interaction_regions`: hit regions with `role`, `bounds`, `quad`, `matrix`, `hit_geometry`, `clip`, `control_bindings`, `resolved_data`, `capabilities`, and `render_mask`;
- `component_controls` and `command_states`;
- `revisions`, `tick`, and runtime counters;
- `numeric_text_regions`: runs of digits detected in text after TMP markup is removed, with the plain text and its range in `resolved_data`.

A region's `capabilities` lists what the host may do with it, such as `activate`, `scroll`, or `drag`, and `resolved_data` carries the resolved target, such as an honor, card, character, music, or story. Control bindings tie regions to component controls: `tab_option` (with the tab `value`), `scroll_viewport`, `scroll_content`, and `scroll_thumb`. A scroll component exposes a fixed viewport, content that moves with the offset, and a thumb that moves proportionally. The host handles wheel and drag input over these regions and calls `setScrollOffset()` or `scrollBy()`; the renderer clamps the offset, patches state, and updates the hit geometry.

Product interaction stays with the host: navigation to honors, characters, cards, events, music, or stories; tab and scroll input; copying numbers; hover cards, selection, and editing; and accessible DOM or SVG overlays. Keep overlay and layer-list DOM stable across animation ticks and update only what moves.

## Caching and resource ownership

| Layer | Lifetime | Owner | Contents |
| --- | --- | --- | --- |
| Provider storage | host-defined | host | Encoded images and other host records. |
| Decoded image cache | renderer | main thread | Decoded images held by scene leases. |
| Glyph persistent cache | browser origin | renderer | Version-checked SDF glyph records in IndexedDB. |
| Glyph session atlas | worker | WebAssembly runtime | Atlas pages, placements, leases, and revisions. |
| GPU textures and buffers | WebGL context | main thread | Atlas pages, images, geometry, and state buffers. |

The decoded image cache keeps released images up to 64 MiB and allows at most 96 MiB in total. When images pinned by live scenes fill the 96 MiB, scene creation rejects instead of evicting them. Persistent encoded-image policy belongs entirely to the `ResourceProvider`.

With `sdf.persistence: "origin"`, the default, glyph records are stored in the IndexedDB database `sekai-custom-profile-sdk-glyph-cache-v1` and shared by every renderer of the origin. Records hold opaque glyph identities, version data, metrics, and R8 SDF pixels; the least recently used records are dropped beyond 64 MiB. Without IndexedDB the cache stays in memory, and a storage error counts as a cache miss, so the glyphs are generated again. `"memory-only"` keeps glyphs for the session only:

```ts
const scene = await renderer.createProfileScene({
  masterData,
  documentKey: "preview",
  profile,
  sdf: { persistence: "memory-only" },
});
```

When the WebGL context is lost, the renderer marks every scene `context-lost` and state calls reject with `WEBGL_CONTEXT_LOST`. On `webglcontextrestored` it recreates the context and reuploads the retained atlas, image, and buffer state, reusing the completed text layout and SDF generation. Call `renderer.restoreContext()` to retry and receive the failure as a rejection.

### Prebuilt atlas packages

By default, scenes generate SDF glyphs on demand and reuse the glyph cache. A prebuilt atlas package supplies a family's glyphs in advance. Packages are never downloaded on their own: the host either serves them through a provider or installs them into IndexedDB from an explicit user action.

```ts
import {
  BrowserRenderer,
  createHttpPrebuiltSdfAtlasProvider,
  createOriginPrebuiltSdfAtlasPackage,
} from "@empty-sekai/sekai-custom-profile-sdk";

const source = createHttpPrebuiltSdfAtlasProvider("/font-atlases");
const atlasPackage = createOriginPrebuiltSdfAtlasPackage({
  namespace: "font-atlases-v1",
  source,
});

// Call from an explicit user action. A family becomes visible after all its pages are stored.
await atlasPackage.install(["FZLanTingHei-DB-GBK", "FZZhengHei-EB-GBK"], {
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

- `createHttpPrebuiltSdfAtlasProvider(baseUrl)` reads `<baseUrl>/<family>/manifest.json` and the page files next to it. A 404 manifest means the family has no package; other HTTP errors reject scene creation.
- `createOriginPrebuiltSdfAtlasPackage({ namespace, source })` installs packages from `source` into the IndexedDB database `sekai-custom-profile-sdk-prebuilt-atlas-v1` under `namespace`. Its `provider` returns only fully installed families and answers `null` for missing, partial, or unavailable storage, so those scenes generate glyphs on demand.
- `install(families, options)` skips installed families, checks the browser-reported quota, verifies each page's SHA-256, and removes the families of a failed install. Options: `concurrency` (default 4, at most 8), `onProgress`, `requestPersistence` (default `false`; calls `navigator.storage.persist()`), and `signal`. It resolves to a `PrebuiltSdfAtlasPackageStatus`.
- `status(families)`, `remove(families?)` (all families of the namespace without an argument), and `close()` complete the package API.

Without browser persistence, pass `createHttpPrebuiltSdfAtlasProvider()` directly as `prebuiltSdfAtlasProvider` and let local files, a Service Worker, or the HTTP cache own the lifecycle. Packages store glyph pages and manifests only, no profile, text, user ID, layout, or scene dump. A package does not replace font files: the `FontProvider` is still asked for the scene's unregistered families. Providers are asked for manifests on every scene, so an install or removal takes effect with the next scene.

Packages are the output of the `build-sdf-atlas` tool of the `sekai-profile` crate, served unchanged:

```sh
SEKAI_PROFILE_FONT_DIR=/path/to/fonts cargo run -p sekai-profile --release \
  --bin build-sdf-atlas -- --font-family FZLanTingHei-DB-GBK \
  --output font-atlases/FZLanTingHei-DB-GBK
```

The directory name and the manifest's `font_family` must equal the family name scenes request. Each page's `file_sha256` covers the decoded page, so a page may be stored pre-compressed as `page-NNN.r8swz.br`, with the manifest entry renamed to match, when the server sends it with `Content-Encoding: br`. Pages are at most 2048 × 2048. All families one scene uses must share `--point-size`, `--spread`, and `--page-size`; otherwise that scene generates its glyphs on demand. [Fonts and assets](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/docs/fonts-and-assets.md) describes how the tool finds font files.

## Errors and warnings

Failures that would make a scene wrong reject explicitly. `BrowserRendererError` carries a string `code`:

| Code | Cause |
| --- | --- |
| `WEBGL2_UNAVAILABLE` | The canvas cannot provide a WebGL2 context. |
| `WEBGL_CONTEXT_LOST` | A call while the WebGL context is lost. |
| `WEBGL2_RESTORE_FAILED` | `restoreContext()` could not create a new context. |
| `RENDERER_DESTROYED`, `SCENE_DESTROYED` | A call after `destroy()`. |
| `INVALID_PROFILE_INPUT` | The profile has no usable page, or `pageIndex` is out of range. |
| `MASTERDATA_LOADER_REQUIRED`, `INVALID_MASTERDATA_CONCURRENCY` | Invalid `loadMasterData()` arguments. |
| `INVALID_FONT_FAMILY`, `INVALID_FONT_BYTES` | An empty family name or empty font bytes. |
| `FONT_IDENTITY_CONFLICT` | A family registered again with different bytes. |
| `FONT_NOT_REGISTERED` | The scene needs a family that is not registered. |
| `PNG_EXPORT_UNAVAILABLE`, `PNG_EXPORT_FAILED` | No Canvas 2D or `OffscreenCanvas`, or PNG encoding returned no data. |

Errors raised inside the worker reject with a `RendererWorkerError`, whose `code` names the failure. `WASM_CALL_FAILED` means the runtime refused the input, for example an unsealed masterdata session, invalid masterdata, or an authoring command outside the limits; the message gives the reason. `WORKER_CRASHED` and `WORKER_TERMINATED` reject every pending call. An atlas package's `install()` rejects with `PrebuiltSdfAtlasStorageError` and the codes `PREBUILT_ATLAS_STORAGE_UNAVAILABLE`, `PREBUILT_ATLAS_INVALID_MANIFEST`, `PREBUILT_ATLAS_QUOTA`, and `PREBUILT_ATLAS_PAGE_HASH`.

Provider outcomes:

| Provider | `null`, error, or bad data | Abort |
| --- | --- | --- |
| `ResourceProvider` | Warning and transparent placeholder; the scene renders. | Rejects scene creation. |
| `FontProvider` | Rejects scene creation. | Rejects scene creation. |
| `LocalizationProvider` | Rejects scene creation. | Rejects scene creation. |
| Masterdata loader, required table | Rejects `loadMasterData()`. | Rejects `loadMasterData()`. |
| Masterdata loader, optional table | Table skipped; its elements are left out. | Rejects `loadMasterData()`. |

A resource warning is written to the console with the prefix `[sekai-custom-profile-sdk]`, the descriptor's `namespace`, `key`, and `role`, and the reason. It is also counted in `renderer.stats().resources.provider.failures`.

## Telemetry and debugging

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

`telemetry.level` is `"summary"` by default: each scene keeps p50, p95, and max of its frame timings over the latest `maxSamples` frames (1–240, default 240). `"trace"` also keeps the raw frame samples, and `"off"` records no frame samples while the runtime counters stay available.

`scene.stats()` returns a `schemaVersion: 1` snapshot: scene state (`ready`, `context-lost`, or `destroyed`), frame count, bootstrap uploads, atlas pages, glyph counts and generation time, glyph cache hits and misses, the last frame's GPU counters, state upload bytes, context-loss and restore counts, and the telemetry samples. GPU timings that are unavailable or disjoint are `null`, never zero.

`renderer.stats()` resolves to:

- `worker`: worker requests, failures, WebAssembly time, bridge bytes, and open sessions;
- `resources`: decoded-cache entries, bytes, pins, hits, loads, and evictions, plus `provider` calls, peak concurrency, known encoded bytes, failures, cancellations, and resolution time;
- `fonts`: registered families and `FontProvider` requests, bytes, failures, peak concurrency, and time;
- `localizations`: `LocalizationProvider` requests, resolved values, failures, peak concurrency, and time;
- `scenes`: active scenes and scenes waiting for context restoration.

Telemetry records counts, timings, capacities, and recovery state only. It holds no source text, stable IDs, resource URLs, font names, or player data.

## Browser support and building

The runtime needs:

- WebGL2;
- ES modules and module Web Workers;
- WebAssembly with exception handling (the runtime is compiled with `-fwasm-exceptions`);
- a secure context, because font and atlas page hashes use `crypto.subtle`;
- `createImageBitmap()` for encoded images;
- IndexedDB, optionally, for the glyph cache and prebuilt atlas packages.

The FreeType build in the runtime contains only the TrueType, CFF, SFNT, PSaux, PSNames, and smooth rasterizer modules. CPU EDT is the default glyph SDF generator; the analytic generator exists for debugging and comparison.

To build the package from a repository checkout:

```sh
cd crates/sekai-profile-wasm
npm ci
npm run build        # WebAssembly runtime (build.sh) and TypeScript
npm run typecheck
npm run test:gates
npm run demo:serve   # workbench at http://127.0.0.1:8088/demo/
```

`npm run build:wasm` runs `build.sh`, which builds the runtime in the Docker image defined by `Dockerfile` (Emscripten and a pinned Rust toolchain), or directly when Docker is absent and `emcc` and `cargo` are on `PATH`. [Development](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/docs/development.md) lists every script and CI check, and the [demo README](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/crates/sekai-profile-wasm/demo/README.md) describes the workbench.

## Exports

| Export | Kind | Purpose |
| --- | --- | --- |
| `BrowserRenderer`, `BrowserScene` | class | Rendering and scene state. |
| `BrowserRendererError` | class | Error with a `code`. |
| `RendererWorkerError` | class | Error with a `code`, raised by the worker. |
| `BrowserAuthoringClient`, `RendererAuthoringDocument` | class | Document editing. |
| `AUTHORING_CHECKPOINT_SCHEMA` | constant | Schema tag of `AuthoringCheckpoint`. |
| `createHttpPrebuiltSdfAtlasProvider`, `createOriginPrebuiltSdfAtlasPackage` | function | Prebuilt atlas sources. |
| `isValidPrebuiltSdfAtlasManifest` | function | Checks a manifest against a family name. |
| `PrebuiltSdfAtlasStorageError` | class | Atlas installation error with a `code`. |
| `FontProviderManager`, `LocalizationProviderManager` | class | The bounded provider queues the renderer uses. |
| `BrowserRendererOptions`, `ProfileSceneCreateOptions`, `MasterDataTableLoader`, `MasterDataTableRequest`, `RendererMasterData` | type | Renderer and masterdata API. |
| `ResourceProvider`, `ResourceDescriptor`, `ResourceContext`, `ResourceSource`, `ProvidedResource` | type | Resource provider contract. |
| `FontProvider`, `FontRequest`, `FontContext`, `ProvidedFont`, `FontProviderStats` | type | Font provider contract. |
| `LocalizationProvider`, `LocalizationRequest`, `LocalizationContext`, `LocalizationProviderStats` | type | Localization provider contract. |
| `PrebuiltSdfAtlasProvider`, `PrebuiltSdfAtlasManifest`, `PrebuiltSdfAtlasPage`, `PrebuiltSdfAtlasGlyph`, `OriginPrebuiltSdfAtlasPackage`, `PrebuiltSdfAtlasInstallOptions`, `PrebuiltSdfAtlasInstallProgress`, `PrebuiltSdfAtlasPackageStatus`, `PrebuiltSdfAtlasStorageErrorCode` | type | Prebuilt atlas API. |
| `CoreSceneDump`, `CoreSceneDelta`, `CoreInteractionRegion`, `CoreControlBinding`, `StableId` | type | Dump and delta shapes. |
| `AuthoringCommand`, `AuthoringDelta`, `AuthoringCategory`, `AuthoringChangeKind`, `AuthoringElementChange`, `AuthoringElementId`, `AuthoringSelection`, `AuthoringPageChange`, `AuthoringPageChangeKind`, `AuthoringCheckpoint`, `GameProfileDocument` | type | Authoring API. |
| `RendererFontContract`, `RendererWorkerStats` | type | Worker font-engine contract and counters. |

## License

AGPL-3.0-only ([LICENSE](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/crates/sekai-profile-wasm/LICENSE)) with the additional permission in [LICENSE-EXCEPTION](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/crates/sekai-profile-wasm/LICENSE-EXCEPTION). The permission lets a web page load an unmodified copy of the published package in an end user's browser and link it with its own front-end code without that code becoming subject to the AGPL. Modified builds of the package or the renderer, and use on a server or in any non-browser environment such as Node.js, fall under the full AGPL, including its network-use source requirement. [NOTICE](https://github.com/empty-sekai/sekai-custom-profile-sdk/blob/main/crates/sekai-profile-wasm/NOTICE) describes the bundled FreeType and its license.
