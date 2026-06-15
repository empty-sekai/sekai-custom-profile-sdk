# demo — browser layer viewer

Vanilla TypeScript/JS layer viewer that drives `@empty-sekai/renderer-wasm` directly in the browser. Zero runtime
dependencies beyond the package itself; no build step beyond `npm run
build:ts` of the parent crate.

## What it shows

- The wasm renderer running in a Web Worker, with the main thread free.
- Per-layer rendering via `renderAllLayers`: each element gets its own
  cropped WebP, positioned absolutely over the canvas frame at its
  original `z` index. Toggle layers on/off without re-rendering.
- A property inspector for each layer (font, color, text, etc.) with
  type-coloured chips, color swatches, and monospace text values.
- Page navigator (arrows + dots + touch swipe) for multi-page cards.
- Drag-and-drop a card JSON onto the canvas.

## Run locally

The demo loads the wasm package's `dist/` files via relative paths, so
you need the package built first:

```sh
npm install && npm run build   # in crates/allium-renderer-wasm/
```

Then serve the **crate root** (not just `demo/`) so that `../dist/`
module imports resolve, with any static file server:

```sh
python -m http.server 8088   # in crates/allium-renderer-wasm/
# then open http://localhost:8088/demo/
```

Fill in the three resource URLs in the demo (they persist in
`localStorage`). Each is a plain prefix the demo appends to directly —
no region or path segments are inserted, so any layout works as long as
the files sit directly under the URL you give. The host must send CORS
headers.

## What you'll need

| Resource | Where from |
|----------|-----------|
| Card JSON | Your own `userCustomProfileCards` API response. The demo accepts the wrapper, a bare array, or a single card. |
| Fonts (`.ttf`/`.otf`) | The FOT fonts shipped with the game. The demo auto-aliases common filenames (e.g. `FOT-RodinNTLGPro-DB.ttf` → both `FOT-RodinNTLGPro-DB` and `FZLanTingHei-DB-GBK`). |
| masterdata URL | A host you control. The demo fetches `<masterdata-url>/<table>.json`. |
| Dynamic asset URL | Card art, stamps, thumbnails (change per game version). The demo fetches `<dynamic-url>/<key>.png`. |
| Static asset URL | Frames, icons, badges, masks (ship with the engine). The demo fetches `<static-url>/<key>.png`. May point at the same host as dynamic. |

Which keys are static vs dynamic is decided by the key's first path
segment (`card/`, `honor/`, `general/`, `sprite/`, `ui/`,
`chara_avatar/`, `mysekai/` are static; everything else is dynamic).

Nothing is bundled. No URL is hardcoded. The demo is meant as a
reference for wiring `@empty-sekai/renderer-wasm` into a host page — you can
also import the package directly and feed it bytes from wherever you
like.

## Layout

```
┌─────────────────────────────┬─────────────────┐
│                             │ Input section   │
│       <canvas frame>        ├─────────────────┤
│   (layered <img> stack)     │ Layer list      │
│                             │ (z, type, eye,  │
│  ◂ page nav ▸  ● ○ ○ dots   │  size, props)   │
└─────────────────────────────┴─────────────────┘
```

The canvas frame keeps the source aspect ratio (1830:812 by default).
Each layer image is positioned with percentage coords relative to that
frame, so resizing the window scales everything proportionally.
