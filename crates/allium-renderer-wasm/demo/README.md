# Allium Scene Workbench

Scene Workbench is the complete browser reference surface for `@empty-sekai/renderer-wasm` 0.2. It renders the semantic scene directly through WebGL2 and exposes the state that a profile site can use to build navigation, selection, copying, component controls, and debugging.

The workbench is deliberately not a profile-data fixture or an asset bundle. You supply a card JSON document, optional profile JSON, and font files. Masterdata and game assets use public CDN providers by default.

## What it demonstrates

- the WebGL2-only `BrowserRenderer` lifecycle;
- a dedicated worker that owns WASM, FreeType, glyph SDF generation, TMP parsing, layout, scene state, and dynamic evaluation;
- an origin-persistent opaque glyph cache, with a memory-only mode;
- deterministic timeline play, pause, single-tick stepping, scrubbing, looping, and final-state inspection;
- multi-page profile navigation;
- stable authored layer IDs, authored ordering, type/search filters, optional inspector grouping, per-layer visibility, and shift-click subtree visibility;
- source content, resolved parameters, bounds, quads, matrices, hit geometry, line-indent diagnostics, glyphs, and semantic commands;
- component tabs and scroll controls supplied by the scene contract;
- quiet-by-default interaction overlays with selected-region focus for cards, honors, stories, and other semantic regions;
- TMP-stripped contiguous numeric-run regions with a caller-owned copy example;
- bounded worker, cache, semantic-core, frame, GPU, and context-recovery diagnostics;
- searchable, path-addressable, collapsible privacy-safe scene dump inspection, raw preview, copy action, and JSON export.

The renderer reports regions and state. The workbench implements hover, activation, copy, scrolling, and navigation-event policy. It intentionally does not navigate to a product route.

## Resource contract

The defaults are:

| Resource | Default base |
| --- | --- |
| CN masterdata | `https://cdn.emptysekai.com/masterdata/cn/latest` |
| CN game assets | `https://cdn.emptysekai.com/assets/cn` |

The selected region changes both defaults. A masterdata provider serves `{base}/{table}.json`; an asset provider serves `{base}/{canonical-key}.png`. Both providers must allow browser CORS requests.

No font, player profile, card document, masterdata table, or game image is bundled in the demo. Card, profile, and font source bytes remain in the current renderer session. None of these source inputs is written to `localStorage`, `sessionStorage`, Cache Storage, or IndexedDB.

When origin persistence is selected, IndexedDB may contain only opaque glyph records keyed by renderer and full font identities: metrics, the R8 SDF payload, dimensions, and payload digest. It does not contain source font bytes, source text, profile JSON, card JSON, layout, commands, atlas placement, or decoded game textures.

## Input shapes

The card input accepts:

1. one bare custom-profile card object;
2. an array of page entries;
3. an API wrapper containing `userCustomProfileCards`.

For page entries, the workbench reads `customProfileCard`, `customProfileCardId`, and `seq`. The optional profile document supplies component data such as player fields, deck cards, honors, music results, story favorites, and character ranks.

Font family identity defaults to each file name without its extension. Edit the family field after upload to match the selected region's masterdata. The field accepts comma-separated aliases when the caller intentionally maps the same source bytes to more than one family. The demo contains no hardcoded regional font-family mapping.

## Run locally

Build the package in the renderer development container first:

```sh
cd crates/allium-renderer-wasm
npm ci
npm run build
```

Serve the crate root, rather than only the `demo` directory, because the workbench imports `../dist/index.js` and worker artifacts:

```sh
python -m http.server 8088
```

Open `http://localhost:8088/demo/` in a browser with WebGL2, Web Workers, ES modules, and Web Crypto support. The renderer uses Web Crypto internally for canonical font identity; callers do not calculate or supply a fingerprint.

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

The **Test context loss** action uses `WEBGL_lose_context` when the browser exposes it. The UI reports loss and restoration without inventing timing values. The renderer is responsible for reconstructing GPU textures and buffers from retained semantic resources and atlas data; unsupported context-recovery diagnostics degrade to an explicit unavailable state.

## Architecture boundary

The demo calls only the high-level browser API:

```js
const renderer = await BrowserRenderer.create({ canvas, region: "cn" });
await renderer.registerFont({ family, bytes });
const masterData = await renderer.loadMasterData("latest");
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

The demo does not parse TMP, collect glyphs, generate SDF pixels, place glyphs in an atlas, calculate line indentation, lower semantic commands, or use Canvas text APIs. WASM owns those responsibilities, including atlas placement and canonical font fingerprinting. The workbench consumes scene dumps, interaction geometry, control state, deltas, and telemetry to provide caller-owned UI behavior.
