# @empty-sekai/renderer-wasm

[简体中文](README.md) | [English](README.en.md)

`@empty-sekai/renderer-wasm` is the WebGL2 browser runtime for Project SEKAI (PJSK) custom profile-card scenes.

TMP rich text and layout implement a compatibility model for the Unity TextMesh Pro data and behavior used by PJSK. It covers the tags, layout, material, and dynamic semantics currently modeled and verified by the runtime. Unmodeled game behavior and later game updates may differ from the client, so complete reproduction of the game's rendering logic or final pixels is not guaranteed.

Rust/WASM owns profile resolution, TMP rich text, layout, dynamic formulas, stable semantic IDs, glyph demand, FreeType metrics, glyph SDF generation, and atlas placement. TypeScript owns the worker boundary, asynchronous resource scheduling, cache I/O, and GPU resource orchestration. WebGL2 consumes the semantic command stream and compact state tables.

Version 0.2 provides a stateful scene API built from a Rust/WASM semantic runtime, dedicated worker, FreeType/SDF atlas, and WebGL2 renderer.

## Requirements

- WebGL2;
- ES modules and Web Workers;
- optional IndexedDB for persistent opaque glyph records;
- host-provided profile/card JSON, masterdata, a font provider or pre-registered fonts, and a `ResourceProvider`.

The host supplies fonts, player data, masterdata, and image assets. Host-provided font bytes with fixed source hashes, FreeType metrics, TMP parsing, and the SDF pipeline jointly define text layout and glyph pixels.

## Installation

```sh
npm install @empty-sekai/renderer-wasm
```

By default, the worker, Emscripten glue, and WASM files load from the `dist/` directory adjacent to the package entry. Supply `workerUrl`, `moduleUrl`, and `wasmUrl` when a bundler or deployment uses another layout.

## Minimal integration

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
  async provide(descriptor, { signal }) {
    const request = await resolveResourceRequest(descriptor);
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

// Always exports the native 1830×812 card size after drawing and copying the frame.
const png = await scene.exportPng();
```

## Custom-profile authoring documents

`BrowserAuthoringClient` owns a game-compatible editable document inside the dedicated worker. Importing a complete public Profile extracts only `userCustomProfileCards`; exports contain no worker handles, stable element IDs, selection state, or other browser metadata.

```ts
import { BrowserAuthoringClient, type AuthoringCommand } from "@empty-sekai/renderer-wasm";

const authoring = await BrowserAuthoringClient.create();
const document = profile
  ? await authoring.importProfile(profile)
  : await authoring.createBlank();

const command: AuthoringCommand = {
  kind: "set_transform",
  id: selectedElementId,
  position: [120, -40, 0],
};
const delta = await document.apply(command);

await document.undo();
await document.redo();
const gameDocument = await document.export();

await document.destroy();
authoring.destroy();
```

The authoring core retains at most 150 history transactions and validates the 150-element page limit, all 12 game arrays, finite numbers, layer order, and the `objectData` boundary. Create, duplicate, delete, transform, lock, visibility, parameter, and layer commands return incremental changes; callers apply those changes to their existing scene and UI instead of maintaining a second TypeScript command history.

## Complete scene flow

`createProfileScene()` performs the following work:

1. The worker sends the card, profile, locale, and masterdata session to WASM.
2. The shared semantic core collects scene-local localization demand, and the host provider returns an immutable text snapshot.
3. WASM resolves authored elements and components with that snapshot and emits the font-family demand actually used by the scene.
4. The main thread invokes the optional `FontProvider` through a bounded queue, hashes returned bytes, and fixes each family-to-hash mapping for the renderer lifetime. The host may instead call `registerFont()` before scene creation.
5. WASM emits complete glyph demand, a layout request, and stable resource descriptors.
6. The main thread deduplicates descriptors by stable ID and invokes the `ResourceProvider` through a bounded queue.
7. WASM uses registered fonts for FreeType measurement, TMP layout, glyph SDF generation, and atlas placement.
8. The core compiles the authored layer tree, semantic commands, control bindings, interaction regions, and initial dynamic state.
9. WebGL2 uploads atlas pages, decoded images, geometry, command state, and layer-mask buffers.
10. The host decides when to draw, advance the timeline, mutate layers, process interaction, or export a dump.

The glyph atlas is created from actual glyph demand. A missing resource, provider error, or image decode failure records a warning and uses a transparent placeholder while the remaining scene continues. Schema, ABI, memory-budget, and scene-contract violations fail explicitly.

## ResourceProvider

The renderer produces and consumes semantic descriptors. The host maps those descriptors to URLs, CDNs, object storage, files, manifests, authenticated requests, or another resource system.

```ts
export type ResourceDescriptor = {
  id: string;
  namespace: string;
  key: string;
  role: string;
  provenance: Record<string, unknown>;
  expectedSize?: { width: number; height: number };
};

export interface ResourceProvider {
  provide(
    descriptor: ResourceDescriptor,
    context: { signal: AbortSignal },
  ): Promise<{
    source: Blob | ArrayBuffer | Uint8Array | TexImageSource;
  } | null>;

  cacheIdentity?(descriptor: ResourceDescriptor): string | null;
}
```

`namespace`, `key`, and `role` are renderer semantics, not filesystem conventions. The host may interpret them in any way.

### Arbitrary asynchronous sources

```ts
const provider: ResourceProvider = {
  cacheIdentity(descriptor) {
    return `${assetRevision}:${descriptor.id}`;
  },

  async provide(descriptor, { signal }) {
    const memoryHit = memoryImages.get(descriptor.id);
    if (memoryHit) return { source: memoryHit };

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

A provider may combine local files, HTTP caching, Cache Storage, IndexedDB, user-selected files, a Service Worker, signed URLs, GraphQL, authenticated APIs, or an already decoded `ImageBitmap`. The provider retains full authority over path interpretation and invalidation.

### Concurrency, deduplication, and failures

- `resourceConcurrency` defaults to 8 and must be a positive integer.
- A batch is deduplicated by descriptor stable `id`.
- Concurrent scenes owned by the same renderer share in-flight singleflight.
- Each waiter responds independently to its own cancellation signal; the provider request is cancelled only after every waiter has left.
- Decoded-cache hits reuse session leases while provider concurrency remains available for actual loads.
- The provider receives an `AbortSignal`.
- A `null` result, thrown error, or decode failure becomes a warning and transparent placeholder.
- Exhausting the decoded-image hard budget with pinned leases produces an explicit resource error.

`cacheIdentity()` defines decoded session-cache identity. Include every catalog revision, region, user scope, or authentication domain that can change the bytes. The stable descriptor ID is used when the method is omitted. Persistent encoded-asset policy belongs entirely to the provider.

## LocalizationProvider

Renderer-owned UI text such as General titles, tabs, and difficulty names is requested through stable localization keys. Player names, signatures, and other user content remain unchanged from the profile. The host may resolve `region + locale + key` through arbitrary logic:

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

WASM first collects the deduplicated keys used by the current scene. The main thread invokes the provider through a bounded queue and returns one complete immutable `key → UTF-8 value` snapshot to WASM. A missing required key fails scene creation explicitly without guessing across regions. The runtime uses its masterdata/runtime localization source when no `LocalizationProvider` is supplied.

## Masterdata

`loadMasterData()` accepts an arbitrary asynchronous table loader supplied by the host. The host defines table naming, location, transport, authentication, and caching:

```ts
const masterData = await renderer.loadMasterData(
  "catalog-2026-07",
  async ({ table, region, revision }, { signal }) => {
    const request = await applicationMasterDataRouter.resolve({ table, region, revision });
    return applicationMasterDataLoader.load(request, { signal });
  },
  { concurrency: 4 },
);
```

Drive the masterdata session directly when the application needs per-table lifecycle control:

```ts
const masterData = await renderer.createMasterData("catalog-2026-07");

for (const table of masterData.requiredTables) {
  await masterData.putTable(table, await applicationMasterData.load(table));
}

await masterData.seal();
```

A session must be sealed before scene creation and should be destroyed when no longer used.

## Fonts

Font bytes are always host-provided. `family` is the opaque logical identity referenced by masterdata and text layers and is passed unchanged to the host's font-resolution policy.

The recommended integration supplies a `FontProvider` to `BrowserRenderer.create()`. WASM requests only families used by the current scene, and the main thread invokes the provider with bounded `fontConcurrency`, which defaults to 4:

```ts
const renderer = await BrowserRenderer.create({
  canvas,
  region: "cn",
  resourceProvider,
  fontProvider: {
    async provide({ region, family }, { signal }) {
      const bytes = await applicationFonts.resolve({ region, family, signal });
      return bytes ? { bytes } : null;
    },
  },
  fontConcurrency: 4,
});
```

The provider may use user-selected files, memory, IndexedDB, an application bundle, network requests, or any other implementation. Returned bytes are copied into an immutable snapshot for that resolution. Scene creation fails explicitly when a demanded family is unavailable.

Applications that want fully manual lifecycle control may register fonts before scene creation:

```ts
const bytes = await file.arrayBuffer();

await renderer.registerFont({ family: "FOT-RodinNTLGPro-DB", bytes });
await renderer.registerFont({ family: "Application Alias", bytes });
```

One file may be registered under multiple logical aliases. Glyph identity includes region, family, source hash, and character.

The first successful registration fixes a family to one source hash for the renderer lifetime. Re-registering identical bytes is idempotent; replacing the same family with different bytes returns `FONT_IDENTITY_CONFLICT`. Create a new renderer when switching font versions so font snapshots, glyph identities, atlases, and persistent-cache boundaries remain explicit.

## Optional prebuilt atlas packages (0.2.1)

Scenes still generate SDF glyphs from their actual demand by default and reuse the origin IndexedDB glyph cache. Prebuilt atlases are never downloaded automatically. A host can use a local/HTTP provider directly, or install a complete atlas package into origin IndexedDB from an explicit user action so multiple renderers, editors, and viewers share one installation.

```ts
import {
  BrowserRenderer,
  createHttpPrebuiltSdfAtlasProvider,
  createOriginPrebuiltSdfAtlasPackage,
} from "@empty-sekai/renderer-wasm";

const source = createHttpPrebuiltSdfAtlasProvider("/font-atlases");
const atlasPackage = createOriginPrebuiltSdfAtlasPackage({
  namespace: "cn-6.0.0-font-atlas-v1",
  source,
});

// Call only from an explicit user action. Manifests become visible after every page is stored.
await atlasPackage.install([
  "FZLanTingHei-DB-GBK",
  "FZZhengHei-EB-GBK",
  "FZShaoEr-M11-JF",
], {
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

`atlasPackage.provider` reads only fully installed families. Missing, in-progress, removed, or unavailable IndexedDB state returns a `null` manifest, so scene creation falls back to demand-driven glyph generation. Installation checks the browser-reported quota, verifies every page SHA-256, and removes incomplete target families after failure. `requestPersistence` defaults to false; the renderer never requests persistent storage without a user action. Atlas packages store no profile, text, user ID, layout, or scene dump.

When browser persistence is unnecessary, pass `createHttpPrebuiltSdfAtlasProvider()` directly to `BrowserRenderer.create()` and let the host's local files, Service Worker, or HTTP cache own the lifecycle.

## Scene state

```ts
await scene.advance(tick);
await scene.setLayerVisible(layerId, false);
await scene.setLayerMasks(layerTableRevision, [
  { layerId, visible: true },
]);
await scene.setTab(controlId, value);
await scene.setScrollOffset(controlId, offset);
await scene.scrollBy(controlId, delta);
scene.draw();
```

These mutations reuse the authored layer table, timeline, layout, glyph atlas, and persistent glyph cache while updating WASM scene state and compact GPU buffers. Layer visibility changes preserve the current dynamic playback position.

Public layers correspond to game-authored elements. Shapes, glyphs, masks, and other draw primitives remain commands owned by those layers.

## Dumps, layers, and interaction

```ts
const dump = await scene.dump();
```

The dump includes:

- stable layer, glyph, command, control, and region IDs;
- the layer table and parent tree;
- source content and resolved parameters;
- authored visibility, render masks, and dynamic state;
- bounds, quads, matrices, clips, and hit geometry;
- semantic commands and component controls;
- interaction regions and continuous numeric runs detected after TMP markup is removed.

Scroll components expose separate fixed viewport, offset-translated content, and proportional thumb regions. The host may handle wheel input over any of them and call `setScrollOffset()` from thumb dragging; the renderer owns clamping, state patches, and updated hit geometry, while pointer behavior remains host-owned.

The host implements product interaction using renderer-provided capabilities, resolved data, control bindings, and geometry for:

- honor, character, card, event, music, or story navigation;
- tab switching and scrolling;
- numeric-text copying;
- hover cards, selection, and editing;
- accessible DOM/SVG overlays.

Do not rebuild the layer-tree DOM on every animation tick. Keep stable layer rows and update only the canvas, overlay, and dynamic details that need to move.

## Cache model

| Layer | Lifetime | Owner | Contents |
| --- | --- | --- | --- |
| provider persistent cache | application-defined | host | encoded images or application records |
| decoded image cache | renderer session | TypeScript runtime | bounded decoded `TexImageSource` leases |
| glyph persistent cache | browser origin | renderer | opaque, version-validated glyph records |
| glyph session atlas | worker session | WASM | atlas pages, placement, leases, and revisions |
| GPU texture/buffer | WebGL context | WebGL2 runtime | atlases, images, geometry, and state buffers |

`sdf.persistence` defaults to `"origin"` and uses IndexedDB. Use `"memory-only"` for session-only glyph caching. IndexedDB records contain opaque glyph identities, version data, metrics, and R8 SDF payloads.

```ts
const scene = await renderer.createProfileScene({
  masterData,
  documentKey: "preview",
  card,
  sdf: { persistence: "memory-only" },
});
```

WebGL context restoration reuploads retained atlas, image, and buffer state while reusing completed TMP parsing, layout, and SDF generation results.

## Debugging and telemetry

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

Telemetry covers worker requests; font-provider requests, peak concurrency, bytes, failures, and elapsed time; localization-provider requests, resolved values, peak concurrency, failures, and elapsed time; resource-provider calls, peak concurrency, known encoded bytes, failures, cancellations, and cumulative resolution time, together with decoded session-cache entries, bytes, pins, hits, loads, and evictions. It also reports glyph generation, persistent-cache hits and misses, atlas pages, texture uploads, GPU buffers, frame timing, and context recovery. `summary` retains aggregates, `trace` retains raw samples within a fixed bound, and `off` exposes immediate runtime state.

Telemetry uses a privacy-safe schema containing runtime counts, timings, capacities, and recovery state. Unavailable or disjoint GPU timings use `null` to represent measurement state.

## Lifecycle and cancellation

```ts
const controller = new AbortController();

const pendingScene = renderer.createProfileScene({
  masterData,
  documentKey: "preview",
  card,
  signal: controller.signal,
});

controller.abort();

await pendingScene; // rejects if creation was still in progress

const scene = await renderer.createProfileScene({ masterData, documentKey: "preview", card });
await scene.destroy();
await masterData.destroy();
renderer.destroy();
```

The signal only cancels scene creation while it is in progress; an already-created scene is released through `destroy()`. Destroying a scene releases image leases, atlas leases, GPU textures, and core scene state. `renderer.destroy()` terminates the worker and aborts resource acquisition still in progress.

## Building from source

Use the repository container toolchain, which pins the FreeType, Emscripten, and Rust target environment.

```sh
npm run build
npm run typecheck
npm run test:gates
npm run verify:wasm:runtime
npm run measure:wasm:size
npm run audit:public
```

The FreeType build enables only the TrueType, CFF, SFNT, PS auxiliary/name, and smooth raster modules used by the runtime. CPU EDT is the production glyph SDF backend; the analytic backend is an explicit debug and parity option.

## License

AGPL-3.0-only with the browser linking exception in `LICENSE-EXCEPTION`. The exception applies to unmodified browser use of the package. Modified renderer builds, server use, and non-browser use remain subject to the full AGPL, including its network-use source requirement.
