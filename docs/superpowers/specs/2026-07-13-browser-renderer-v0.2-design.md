# Browser Renderer v0.2 and Scene Workbench Design

**Date:** 2026-07-13
**Status:** Approved
**Scope:** `allium-renderer-core`, browser WASM runtime, WebGL2 backend, public package, and demo

## 1. Outcome

Version 0.2 replaces the browser package's CPU-image composition path with the shared semantic renderer architecture already validated in the browser-next laboratory. This is an intentional breaking replacement: the browser package no longer ships or exposes a CPU/Skia rendering backend. The same Rust core drives native and browser scene resolution, TMP parsing, layout, dynamic formulas, stable IDs, command schemas, and interaction geometry.

The browser runtime uses FreeType WASM for metrics and glyph SDF generation. WebGL2 consumes reusable glyph atlases, semantic command buffers, layer state, and render masks. Visibility changes and interaction state never rebuild the timeline, layout, command stream, glyph atlas, or persistent cache.

The public demo becomes an English-only **Scene Workbench**. It is a complete developer surface, not a product-specific viewer.

## 2. Selected Approach

The existing `@empty-sekai/renderer-wasm` package is replaced in place for 0.2:

- add the shared `allium-renderer-core` crate to the public workspace;
- expose the stateful semantic scene API through the browser WASM boundary;
- use the FreeType minimal build and WebGL2 renderer from the validated browser-next implementation;
- delete the old CPU image API, Skia WASM artifact, image-layer worker protocol, and duplicated browser algorithms;
- ship one semantic artifact containing the shared core and minimal FreeType runtime.

Rejected alternatives:

- extending CPU Skia WASM layer images would preserve the wrong rendering and caching model;
- publishing a second browser package or a legacy subpath would preserve two runtimes, two dependency graphs, and two demos without serving an established user base;
- embedding fonts, profiles, masterdata, or game assets would violate the public resource-provider boundary.

## 3. Public Runtime Boundaries

### Shared Rust core

The core owns:

- all twelve authored custom-profile element types;
- stable scene, layer, glyph, command, component, and interaction IDs;
- authored game-layer ordering and layer tree;
- source content and resolved parameters;
- bounds, quads, matrices, clipping, and hit geometry;
- TMP parsing and layout contracts;
- TMP-stripped contiguous ASCII numeric runs;
- deterministic 60 Hz dynamic programs and state;
- render-mask and component-control deltas;
- command and telemetry schemas.

### Browser WASM

WASM owns:

- FreeType font metrics;
- glyph SDF generation and atlas payloads;
- scene creation, dumps, snapshots, and deltas;
- layout and command generation;
- dynamic evaluation and component state updates;
- canonical cache identities using region, full font SHA-256, family, raster contract, and engine fingerprint.

### WebGL2

WebGL2 owns:

- instanced glyph rendering;
- semantic image and shape commands;
- reusable atlas pages and decoded resource textures;
- layer-state textures and bounded dirty uploads;
- render masks, transforms, clipping, and context restoration;
- GPU timing when supported.

Renderer APIs expose state and geometry only. Navigation, hover behavior, copying, selection, editing, scrolling, and overlays remain caller policy.

## 4. Resource Contract

The package bundles no game masterdata, player profile, game asset, or font.

The Scene Workbench defaults to:

- `https://cdn.emptysekai.com/masterdata/cn/latest` for CN masterdata;
- `https://cdn.emptysekai.com/assets/cn` for CN game assets.

Users provide profile/card JSON and font files. Font files are never uploaded, persisted as source bytes, or fetched from the Allium CDN. Local font registration may map one file to the aliases declared by the selected region's masterdata.

CDN paths are derived from canonical renderer asset keys by appending `.png`. The demo displays every resolved URL and reports missing resources without inventing browser fallback rendering.

## 5. Cache and Worker Model

A dedicated worker owns WASM, semantic scenes, font bytes, glyph generation, timeline evaluation, mask/component mutations, dumps, and request singleflight. The main thread owns WebGL and UI state and receives immutable snapshots, deltas, and atlas payloads through a versioned protocol.

The session atlas is a bounded 2048² R8 texture-array cache with pin/lease semantics, dirty rectangles, page revisions, soft/hard budgets, eviction, and context restore. Decoded image resources use a separate bounded LRU.

IndexedDB persists only opaque glyph records: canonical digest, metrics, R8 payload, dimensions, and payload digest. It does not persist source text, profiles, fonts, packed atlas placement, layout, commands, timelines, masks, decoded textures, or GPU buffers. Corrupt, incompatible, quota-blocked, and private-mode records degrade safely to memory-only behavior.

## 6. Scene Workbench

The demo uses an IDE-like three-pane layout:

- a resource/session rail for profile, fonts, region, backend, and cache actions;
- a large live 1830×812 scene stage with page navigation, overlays, and timeline controls;
- an inspector with layer tree, command/source/parameter tabs, component controls, and telemetry.

It demonstrates every v0.2 feature:

- all authored element types in imported real profiles;
- pure-front-end dynamic formulas with play, pause, scrub, and deterministic tick input;
- authored-layer and subtree masks without timeline/cache reset;
- source content, resolved parameters, bounds, quads, matrices, clips, and hit geometry;
- numeric-run overlays and a caller-owned copy action example;
- hover, selection, and navigation-event examples for titles, cards, stories, and other interactive regions;
- component tabs, music counters, character/challenge rank switching, and scrollable component regions;
- atlas pages, worker jobs, persistent-cache statistics, decoded-resource cache, GPU buffers, frame timings, and context restoration;
- summary/trace debug modes, privacy validation, scene dump export, and parity status.

The visual language uses a restrained dark workbench, a high-contrast card stage, cyan state accents, amber diagnostics, compact monospace values, and subtle motion. Diagnostics remain readable without competing with the rendered card.

## 7. Debug and Telemetry

Public summary telemetry is bounded and privacy-safe. Trace mode retains at most 240 sanitized samples. Timing fields are nullable when unavailable; unsupported or disjoint GPU timers never report fake zeroes.

The debug dump includes the complete authored layer tree, sources, resolved parameters, semantic commands, interaction regions, component controls, revisions, cache counters, and dynamic state. Hidden layers remain inspectable but are not hittable.

## 8. Compatibility and Publication

The native CPU renderer and browser runtime consume the same core, but the native renderer is not compiled into or exposed by the browser package. Source-diff gates remain until duplicate native/browser TMP, layout, SDF parameter, and command logic is deleted.

The public repository remains AGPL-3.0-only. The npm browser exception remains limited to the existing `@empty-sekai/renderer-wasm` package terms. The npm distribution includes the required FreeType license and notice. All public code, comments, UI strings, API documentation, tests, commit messages, and pull-request text are English. No internal KB links, private infrastructure details, credentials, or player fixtures enter the repository.

Renderer locale bundles contain renderer-owned interface labels only. Extracted game strings remain caller-provided masterdata and never enter the package.

The 0.2 migration guide explicitly lists removed 0.1 CPU APIs and maps them to the semantic scene, WebGL2 stage, dump, mask, and interaction APIs. Compatibility is documentation-only; no shim or legacy artifact is shipped.

## 9. Verification

Release gates cover:

- core ABI/schema round trips and major-version rejection;
- all twelve element types and authored ordering;
- native/WASM scene and dynamic-state parity;
- TMP debug and glyph SDF parity;
- final WebGL pixel parity;
- render-mask and component-control invariants;
- atlas, decoded-resource, IndexedDB, worker, and context-restore behavior;
- multi-region and multi-font cache isolation;
- numeric interaction geometry;
- debug privacy and bounded telemetry;
- complete-profile GPU execution;
- cold/warm performance and memory budgets;
- an English-content and forbidden-resource audit for the public repository.

The demo is accepted only when a user can provide profile JSON and fonts, render through the default CDN configuration, inspect and control the complete scene, exercise all interaction examples, and export a full debug dump without using Canvas text APIs or browser font fallback as rendering truth.
