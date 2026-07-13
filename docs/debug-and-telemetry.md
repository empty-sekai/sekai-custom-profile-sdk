# Debugging and telemetry

The browser runtime provides two complementary inspection surfaces.

## Scene dump

`await scene.dump()` returns a complete point-in-time semantic description:

- schema and scene revisions;
- authored layer table and tree;
- source content and resolved parameters;
- layer bounds, quad, matrix, hit geometry, and clipping;
- semantic commands and command state;
- interaction regions and control bindings;
- component controls;
- render masks and transforms;
- TMP-stripped contiguous ASCII numeric regions.

The dump is intended for developer tools and caller-owned interaction policies. It does not prescribe navigation or editing behavior.

## Runtime statistics

`scene.stats()` reports bounded scene-local measurements, including frame timing, GPU draw/upload counts, atlas pages and glyphs, glyph-generation time, cache results, state-patch traffic, and context recovery. `await renderer.stats()` adds worker, decoded-resource cache, and live-scene totals.

Trace samples are held in a bounded ring. Telemetry may contain counts, byte sizes, durations, revisions, and backend identifiers. It must never contain:

- player identifiers or profile JSON;
- TMP/source strings or extracted labels;
- font bytes;
- image payloads or authenticated URLs;
- application navigation targets.

Context-loss and restoration counters make recovery observable. Restoration upload metrics are separated from steady-state generation so a GPU reset cannot be mistaken for atlas regeneration.
