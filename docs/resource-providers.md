# Resource providers

The browser package contains renderer code only. It does not bundle fonts, player fixtures, masterdata, extracted strings, or game images.

## Default URLs

Unless overridden, the renderer loads:

- masterdata: `https://cdn.emptysekai.com/masterdata/{region}/{revision}/{table}.json`
- renderer static assets: `https://cdn.emptysekai.com/renderer-static/v0.2/{asset-key}.png`
- unpacked game assets: `https://cdn.emptysekai.com/assets/{region}/{asset-key}.png`

The shared core marks canonical renderer cuts as `static` and official unpacked game files as `assets`. The runtime does not probe alternate filenames, calculate asset fingerprints, or silently switch to a local asset tree.

Applications may replace URL construction with `resolveMasterDataUrl` and `resolveResourceUrl`. A provider changes transport and location only; it does not move TMP parsing, layout, glyph demand, or atlas construction into application code.

## Fonts

Fonts are always application-provided. Register the exact family requested by resolved masterdata before creating the scene. The renderer computes and retains the font identity required for glyph-cache validation; callers do not enumerate glyphs or upload atlas pages.

Font bytes cross the worker boundary once and remain there for the renderer lifetime. IndexedDB never stores the font file.

## Cache layers

All encoded CDN responses may use the browser HTTP cache. Only the finite, versioned `renderer-static/v0.2/` namespace is additionally stored in Cache Storage. Region game assets remain under normal HTTP cache and conditional revalidation, because their `assets/{region}/` URLs are not immutable revision keys. The runtime deletes the legacy unbounded semantic Cache Storage namespace. Decoded images are retained in a bounded session LRU. GPU textures are context-local and are recreated from decoded sources after context restoration.

Glyph persistence is separate. IndexedDB contains opaque glyph metrics and R8 SDF payloads keyed by renderer contract and font identity. It never contains profile content, source strings, layout, command buffers, or dynamic state.
