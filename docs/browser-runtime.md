# Browser runtime architecture

The 0.2 browser renderer has one semantic pipeline and one production drawing backend.

1. The host supplies card/profile JSON and font bytes.
2. The Rust core resolves masterdata, authored layers, resources, locale-dependent strings, stable IDs, controls, and complete text-layout inputs with full affine matrices.
3. The WASM text engine parses TMP and emits canonical glyph demand.
4. FreeType maps and measures glyphs; the production CPU EDT backend generates R8 SDF records on demand.
5. WASM places glyphs into bounded atlas pages, lays out every text command once, and returns glyph instances plus dynamic-program descriptors.
6. The shared core compiles the stateful scene from those descriptors; TypeScript only performs cache and resource I/O and uploads WASM outputs.
7. WebGL2 consumes semantic commands, glyph instances, state buffers, textures, masks, and transforms.

In this document, **host** means package-external application code such as a profile site or the demo. It does not mean the package-internal TypeScript runtime. The host cannot provide a prebuilt atlas or precomputed line-indent program through the public API. This prevents a second TMP/layout implementation from developing outside WASM.

The internal TypeScript runtime may consume the structured glyph demand returned by WASM, query session and IndexedDB caches, and send cache hits plus missing glyph requests back to the worker. It never parses TMP or infers demand from source text. FreeType rasterization, SDF record generation, atlas packing, UV assignment, page revisions, and eviction remain WASM-owned.

## Ownership

| Concern | Owner |
| --- | --- |
| Profile/masterdata resolution | Rust core |
| TMP parsing and visible glyph demand | WASM |
| FreeType measurement and SDF records | WASM |
| Dynamic formulas and timeline state | Rust core/WASM worker |
| Atlas packing, UVs, revisions, and dirty rectangles | WASM worker |
| Glyph-cache lookup, IndexedDB record I/O, and GPU uploads | Package-internal TypeScript runtime |
| Decoded image lifetime | TypeScript runtime |
| Drawing and compact state updates | WebGL2 |
| Navigation, hover, copy, selection, editing, overlays | Package-external host application |

The dedicated worker owns WASM, font bytes, scenes, glyph generation, timeline evaluation, dumps, and request singleflight. The main thread owns the WebGL context and application UI.

TypeScript never reconstructs text layers from semantic commands. In particular, it does not decompose authored matrices into rotation/scale/skew. Rust emits the complete layout record and full 2D affine matrix, so reflections and shear survive unchanged; the worker only joins registered font identity before invoking WASM.

## Update invariants

Layer masks, tab changes, scrolling, and timeline ticks update state without rebuilding unrelated work. In particular, a visibility change must not reset or recreate:

- timeline state;
- layout results;
- semantic commands;
- glyph demand;
- session atlas pages;
- persistent glyph records;
- decoded image cache entries.

Public layers represent authored profile-card elements. A renderer may split a layer into multiple GPU commands, but those commands never become extra public layers.

## Context loss

An HTML canvas listener prevents the default permanent-loss behavior and marks all live scenes unavailable. After restoration, the renderer recreates context-local programs, buffers, and textures from retained semantic commands, atlas pixels, and decoded resources. It does not rerun profile resolution, TMP parsing, layout, or glyph SDF generation.
