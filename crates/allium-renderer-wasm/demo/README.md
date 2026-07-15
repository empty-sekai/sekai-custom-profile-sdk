# Allium Scene Workbench

Scene Workbench is the browser reference application for `@empty-sekai/renderer-wasm` 0.2. It draws the semantic scene directly through WebGL2 and demonstrates how a profile site can build navigation, selection, copying, component controls, diagnostics, and accessibility overlays around the renderer.

The workbench accepts one profile JSON response and the referenced font files. Card pages are read from `userCustomProfileCards`, while application adapters provide masterdata and image resources.

## What it demonstrates

- the complete WebGL2-only `BrowserRenderer` lifecycle;
- a dedicated worker that owns WASM, FreeType, glyph SDF generation, TMP parsing, layout, scene state, and dynamic evaluation;
- an origin-persistent opaque glyph cache and a memory-only mode;
- deterministic play, pause, stepping, scrubbing, looping, and final-state inspection;
- multi-page profile navigation without retaining unbounded scene resources;
- stable game-authored layer IDs, ordering, filtering, per-layer visibility, and authored-subtree masks;
- source content, resolved parameters, bounds, quads, matrices, hit geometry, line-indent diagnostics, glyphs, and semantic commands;
- component tabs and scrolling driven by scene control bindings;
- caller-owned overlays for cards, honors, stories, music, characters, and other semantic regions;
- TMP-stripped continuous numeric regions with a copy example;
- bounded worker, cache, semantic-core, frame, GPU, and context-recovery diagnostics;
- searchable, path-addressable, privacy-safe scene dumps;
- stable inspector DOM while the timeline advances, so layer selection remains usable during playback.

The renderer reports semantic state and geometry. The workbench implements UI policy and emits navigation events for the embedding application to connect with its own routes.

## Inputs and persistence

The single profile input must contain `userCustomProfileCards`. Each entry supplies `customProfileCard`, `customProfileCardId`, and `seq`; the same response may supply player fields, deck cards, honors, music results, story favorites, and character ranks.

Profile JSON and selected font files are stored in IndexedDB for this local origin so they survive a refresh. All input persistence remains local to the selected browser origin, and resetting the session removes the stored inputs.

Font source files use their packaged names. The workbench maps logical production aliases to the selected files, then exposes that map through its application-owned `FontProvider`. WASM requests only families used by the active scene. An alias is another runtime family identity for the same bytes, not another file that the user must find.

## Demo resource provider

The workbench supplies an application-owned `ResourceProvider` that demonstrates one CDN and path mapping strategy. Other consumers provide adapters that match their own resource systems.

Its initial configuration uses:

| Resource | Initial base |
| --- | --- |
| CN masterdata | `https://cdn.emptysekai.com/masterdata/cn/latest` |
| CN game assets | `https://cdn.emptysekai.com/assets/cn` |
| versioned renderer cuts | `https://cdn.emptysekai.com/renderer-static/v0.2` |

Changing the region updates the editable masterdata and asset bases. The demo provider owns all mapping rules, asynchronous `fetch`, `AbortSignal` propagation, browser HTTP caching, and Cache Storage use for versioned renderer cuts. None of those rules are exported by the renderer package.

Missing or undecodable images are reported as warnings and become transparent placeholders, allowing the remaining scene to continue. The selected resource origin supplies the required CORS permission.

## Run locally

Build the package in the repository development container:

```sh
cd crates/allium-renderer-wasm
npm ci
npm run build
```

Serve the crate root, not only the `demo` directory, because the workbench loads package artifacts from `dist/`:

```sh
python -m http.server 8088
```

Open `http://127.0.0.1:8088/demo/` in a browser with WebGL2, Web Workers, ES modules, Web Crypto, and IndexedDB support.

## Keyboard controls

| Key | Action |
| --- | --- |
| `Space` | Play or pause the dynamic timeline |
| `Left` / `Right` | Step one deterministic tick |
| `Shift` + `Left` / `Right` | Step one second at 60 Hz |
| `Ctrl`/`Cmd` + `S` | Export the current scene dump |
| `Enter` / `Space` on a layer or overlay | Select or activate it |
| Shift-click a layer eye | Apply the visibility value to its authored subtree |

## Context recovery

**Test context loss** uses `WEBGL_lose_context` when available. The renderer reconstructs GPU textures and buffers from retained semantic resources and atlas data while reusing completed TMP parsing, layout, and SDF generation results. GPU timing and recovery diagnostics use explicit availability states.

## Integration boundary

The workbench calls the high-level browser API and provides an application-owned resource provider:

```js
const renderer = await BrowserRenderer.create({
  canvas,
  region: "cn",
  resourceProvider: createApplicationResourceProvider(config),
  resourceConcurrency: 8,
  fontProvider: {
    async provide({ family }) {
      const selected = selectedFonts.find((font) => font.families.includes(family));
      return selected ? { bytes: selected.bytes } : null;
    },
  },
  fontConcurrency: 3,
  telemetry: { level: "summary", maxSamples: 240 },
});

const masterData = await renderer.loadMasterData(
  "latest",
  ({ table }, { signal }) => loadDemoMasterData(table, { signal }),
);
const scene = await renderer.createProfileScene({
  masterData,
  documentKey,
  card,
  profile,
  frameMode: "animate",
  sdf: { backend: "edt", persistence: "origin" },
});

scene.draw();
```

The Rust/WASM runtime performs TMP parsing, glyph enumeration, SDF generation, atlas placement, line-indentation formulas, and semantic command lowering. The workbench consumes scene dumps, controls, interaction geometry, state deltas, and telemetry to implement caller-owned UI behavior.
