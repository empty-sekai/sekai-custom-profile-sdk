# Scene Workbench

Scene Workbench is the reference browser application of `@empty-sekai/sekai-custom-profile-sdk`. It renders a custom-profile scene with `BrowserRenderer` and shows what the renderer reports: authored layers, geometry, component controls, interaction regions, scene dumps, and telemetry. It uses the public package API from `dist/` and nothing else.

Each release build of the repository also publishes the workbench to GitHub Pages: <https://empty-sekai.github.io/sekai-custom-profile-sdk/>.

## Features

- the full `BrowserRenderer` lifecycle: renderer, masterdata session, one scene per page, and teardown;
- page tabs for multi-page profiles; switching pages destroys the previous page's scene;
- a timeline with play, pause, single-tick and one-second steps, scrubbing over ticks 0–600, looping, and a jump to the last tick;
- a layer list with text and type filters, show or hide all, per-layer visibility, and subtree visibility through `setLayerMasks()`;
- layer details: source content, resolved parameters, bounds, quads, matrices, hit geometry, line-indent data, and semantic commands;
- tab and scroll components driven by the scene's control bindings;
- an SVG overlay of interaction regions with hover cards, activation, and copying of numeric text runs;
- an event log of the activation, copy, scroll, and control events a host application would handle;
- renderer and scene telemetry;
- a dump viewer with JSON-pointer navigation, search, copy, and JSON download;
- a WebGL context-loss test.

The workbench does not navigate anywhere. It writes activation events to its event log, where an embedding application would route them.

## Inputs

**Profile JSON.** One profile API response containing `userCustomProfileCards`. Each entry carries `customProfileCard`, `customProfileCardId`, and `seq`; pages are shown in ascending `seq` order. The same response supplies the player data the card draws.

**Fonts.** `.ttf` and `.otf` files. Each file is offered to the renderer under its file name without the extension. Three files are also offered under a second family name:

| File name | Also offered as |
| --- | --- |
| `FOT-RodinNTLGPro-DB` | `FZLanTingHei-DB-GBK` |
| `FOT-SkipProN-B` | `FZZhengHei-EB-GBK` |
| `FOT-PopHappinessStd-EB` | `FZShaoEr-M11-JF` |

The workbench passes these names to the renderer through its `FontProvider`, and the renderer asks only for the families the active scene uses.

The profile JSON and the font files are stored in IndexedDB for the page's origin, so they survive a reload. The reset button (↻) removes them.

## Providers and settings

The **Providers** panel sets the region (`cn`, `jp`, `tw`, `en`, `kr`), the masterdata revision (a git branch, tag or commit, `main` by default), and two base URLs. Their initial values are:

| Setting | Initial value |
| --- | --- |
| Masterdata base | `https://raw.githubusercontent.com/<repository>/<revision>/master` |
| Asset base | `https://storage.pjsk.moe/sekai-<region>-assets` |

The masterdata repository follows the region: `Team-Haruki/haruki-sekai-master` (`jp`), `Team-Haruki/haruki-sekai-sc-master` (`cn`), `Team-Haruki/haruki-sekai-tc-master` (`tw`), `Team-Haruki/haruki-sekai-kr-master` (`kr`) and `Team-Haruki/haruki-sekai-en-master` (`en`). Changing the region rewrites both bases, and changing the revision rewrites the masterdata base; either field can then be edited. The masterdata loader fetches `<masterdata base>/<table>.json` for every table and skips an optional table that fails to load. The demo `ResourceProvider` in `workbench/emptySekaiResourceProvider.js` maps `assets` descriptors to paths under the asset base and `static` descriptors to `https://profile-assets.emptysekai.dev/game-profile/cn/6.0.0/renderer-static`, keeping `static` images in Cache Storage. It shows one mapping strategy; other hosts write providers for their own resource systems. The resource origins must allow CORS requests from the workbench's origin.

The settings below the panel choose the glyph SDF generator (`edt` or `analytic`), the glyph cache (`origin` or `memory-only`), and the telemetry level (`summary`, `trace`, or `off`).

A missing or undecodable image becomes a transparent placeholder with a console warning, and the rest of the scene renders.

## Run locally

Build the package, then serve the package directory:

```sh
cd crates/sekai-profile-wasm
npm ci
npm run build
npm run demo:serve
```

`npm run demo:serve` serves the package directory at <http://127.0.0.1:8088/demo/>; `--host` and `--port` change the address, for example `npm run demo:serve -- --port 9000`. Any static server works as long as it serves the package directory rather than `demo/` alone, because the workbench loads `../../dist/`. `npm run build` needs Docker, or `emcc` and `cargo` on `PATH`; see the [package README](../README.md#browser-support-and-building).

Use a browser with WebGL2, module Web Workers, WebAssembly exception handling, Web Crypto, and IndexedDB.

## Keyboard

| Key | Action |
| --- | --- |
| `Space` | Play or pause the timeline. |
| `Left` / `Right` | Step one tick. |
| `Shift` + `Left` / `Right` | Step 60 ticks (one second). |
| `Ctrl` / `Cmd` + `S` | Download the current scene dump. |
| `Enter` / `Space` on a layer or region | Select or activate it. |
| `Shift` + click on a layer's visibility button | Apply the visibility to the layer's subtree. |

## Context loss

**Test context loss** calls `WEBGL_lose_context.loseContext()` and restores the context 900 ms later. The renderer rebuilds GPU textures and buffers from the retained scene state and atlas data, reusing completed text layout and SDF generation. The telemetry panel shows the loss and restore counts; GPU timers the browser cannot provide are reported as unavailable, not as zero.

## Integration code

`workbench/session.js` holds the renderer calls:

```js
const renderer = await BrowserRenderer.create({
  canvas,
  region,
  resourceProvider: createEmptySekaiResourceProvider({ region, assetBase }),
  resourceConcurrency: 8,
  fontProvider: {
    async provide({ family }) {
      const font = fonts.find((candidate) => candidate.families.includes(family));
      return font ? { bytes: font.bytes } : null;
    },
  },
  fontConcurrency: 3,
  telemetry: { level: debugLevel, maxSamples: 240 },
});

const masterData = await renderer.loadMasterData(
  revision,
  async ({ table, optional }, { signal }) => {
    const response = await fetch(`${masterdataBase}/${encodeURIComponent(table)}.json`, { signal });
    if (optional && !response.ok) return null;
    if (!response.ok) throw new Error(`Master-data fetch failed ${response.status}`);
    return response.json();
  },
);

const scene = await renderer.createProfileScene({
  masterData,
  documentKey,
  profile,
  pageIndex,
  frameMode: "animate",
  sdf: { backend: sdfBackend, persistence },
});

scene.draw();
```

The rest of the workbench (`workbench/inspector.js`, `workbench/interactions.js`, `workbench/telemetry.js`) reads scene dumps, control bindings, interaction regions, state deltas, and telemetry to build its UI.
