# demo — browser layer viewer

Vanilla TypeScript/JS layer viewer that drives `@allium/renderer-wasm` directly in the browser. Zero runtime
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

Then start the bundled server. It serves the **crate root** (not just
`demo/`) so that `../dist/` module imports resolve, and reverse-proxies
`/cdn/*` to the URL in `ALLIUM_CDN_BASE`, sidestepping CORS on upstream
mirrors that don't set `Access-Control-Allow-Origin`:

```sh
ALLIUM_CDN_BASE=https://your-cdn.example.com \
  python crates/allium-renderer-wasm/demo/serve.py 8088
# then open http://localhost:8088/demo/
```

The demo auto-detects the `/cdn/` proxy and populates the CDN-base field
with `http://localhost:8088/cdn`. If you'd rather hit a CORS-enabled CDN
directly, leave `ALLIUM_CDN_BASE` unset and type the full URL into the
field — it persists in `localStorage`.

`serve.py` is stdlib-only (no pip install required) and runs on
Python 3.8+.

## What you'll need

| Resource | Where from |
|----------|-----------|
| Card JSON | Your own `userCustomProfileCards` API response. The demo accepts the wrapper, a bare array, or a single card. |
| Fonts (`.ttf`/`.otf`) | The FOT fonts shipped with the game. The demo auto-aliases common filenames (e.g. `FOT-RodinNTLGPro-DB.ttf` → both `FOT-RodinNTLGPro-DB` and `FZLanTingHei-DB-GBK`). |
| masterdata | A mirror you control, exposing `/masterdata/{region}/latest/<table>.json`. |
| Asset images | The same mirror, exposing `/assets/{region}/<key>.png`. |

Nothing is bundled. No CDN is hardcoded. The demo is meant as a
reference for wiring `@allium/renderer-wasm` into a host page — you can
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
