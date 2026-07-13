# Semantic scene schema

The browser and native adapters consume the same append-only semantic scene contract. Version 0.2 uses schema 1.11.

## Coordinate system

Profile-card geometry uses the `card-device-v1` coordinate space. The full card is 1830 by 812 units. Bounds, quads, matrices, hit geometry, clip polygons, glyph planes, and semantic commands use this space unless a field explicitly declares another one.

## Authored layers and commands

A public layer represents one game-authored profile-card element. Its stable ID, parent, source content, resolved parameters, geometry, interaction data, and visibility remain available even when its render mask is false.

One authored layer may lower to several semantic draw commands. Commands preserve authored order and carry their own stable identity and state slots, but they do not become public layers.

The layer table is in depth-first authored order. Each entry contains a `slot`, `subtree_start`, and `subtree_end`, allowing one bounded state-buffer range to control a complete authored subtree.

## Revisions and deltas

A snapshot contains immutable semantic commands plus initial state. Mutations return a revision-checked delta with explicit dirty categories and compact layer/command patches.

Mask, transform, timeline, tab, and scroll updates never imply a layout, command, or atlas rebuild unless the corresponding dirty flag is explicitly true. Unknown IDs, stale layer-table revisions, invalid command spans, and unsupported commands fail closed.

## Interaction regions

Interaction regions expose:

- stable region and authored-layer IDs;
- semantic role and capabilities;
- bounds, quad, affine matrix, hit polygon, and optional clip polygon;
- resolved application data;
- tab, scroll, and other control bindings;
- current render-mask state.

The renderer supplies geometry and state only. The host decides whether a region navigates, copies, selects, edits, scrolls, or displays an overlay.

Text is generally exposed as source/dump geometry rather than an automatic selection layer. The first specialized text interaction is TMP-stripped contiguous ASCII numeric runs. Each numeric region retains the owning command/layer identity and derived glyph geometry; hidden commands remain inspectable but cannot be hit.

## Dumps

`scene.dump()` is a complete, debug-oriented snapshot containing:

- schema, scene, tick, and revision metadata;
- the authored layer table/tree and full layer records;
- semantic commands and current command states;
- interaction regions and component controls;
- source content and resolved parameters;
- masks, transforms, geometry, and numeric text regions;
- aggregate core telemetry.

The dump format is append-only within schema major 1. Consumers must ignore unknown fields and reject unsupported major versions.
