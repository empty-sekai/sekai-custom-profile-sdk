# Browser Renderer v0.2 Implementation Plan

**Goal:** Prepare the shared renderer v0.2 core, FreeType WASM/WebGL2 browser runtime, and a polished English Scene Workbench demo as one release candidate in the public repository. Do not tag or publish 0.2.0 until the workbench and every release gate pass.

**Architecture:** Forward-port the validated append-only shared core and browser-next modules into the existing public workspace. Replace the browser CPU/Skia artifact and API completely; native CPU rendering remains a separate adapter over the same shared core. The browser worker owns WASM/font/glyph/scene work; the main thread owns WebGL2, interaction overlays, and the workbench UI.

**Tech Stack:** Rust, FreeType 2.13.2, Emscripten, TypeScript, Web Workers, WebGL2, IndexedDB, vanilla HTML/CSS, Node test runner, Docker.

---

## Milestone 1: Public shared core and drift gates

**Files:**

- Create: `crates/allium-renderer-core/Cargo.toml`
- Create: `crates/allium-renderer-core/src/lib.rs`
- Create: `crates/allium-renderer-core/src/locale.rs`
- Create: `crates/allium-renderer-core/src/profile_scene.rs`
- Create: `crates/allium-renderer-core/src/profile_transform.rs`
- Create: `crates/allium-renderer-core/src/sdf_geometry.rs`
- Create: `crates/allium-renderer-core/src/tmp_text.rs`
- Create: `crates/allium-renderer-core/src/profile_layout/{mod,cards,extras,header,music,stats}.rs`
- Create: `crates/allium-renderer-core/src/profile_source/{mod,card,elements}.rs`
- Create: `crates/allium-renderer-core/locales/{cn,jp,tw,en,kr}.json`
- Modify: `Cargo.toml`
- Modify: `crates/allium-renderer/Cargo.toml`
- Create: `tools/verify-source-parity.mjs`
- Modify: internal source-parity CI outside this public repository

- [x] Port schema 1.11 and its 41 core tests without private fixtures or internal references. This includes TMP-stripped numeric runs, render-observable dynamic layer IDs, proved timeline descriptors, and discoverable tab/scroll control bindings.
- [ ] Add `allium-renderer-core` as a workspace member and dependency of native and WASM adapters.
- [ ] Keep the normalized private-to-public source-diff gate in internal CI. Public GitHub CI must remain self-contained and must not depend on `/shipping` or another private checkout.
- [ ] Run in the renderer dev container:

```bash
cargo test -p allium-renderer-core
```

Expected: 34 self-contained public tests pass. Internal CI separately reports zero unclassified semantic differences.

## Milestone 2: Minimal FreeType WASM and semantic browser API

**Files:**

- Create: `crates/allium-renderer-wasm/freetype-min/ftmodule.h`
- Create: `crates/allium-renderer-wasm/src/edt.rs`
- Create: `crates/allium-renderer-wasm/src/geometry.rs`
- Create: `crates/allium-renderer-wasm/src/layout.rs`
- Create: `crates/allium-renderer-wasm/src/scene.rs`
- Modify: `crates/allium-renderer-wasm/src/lib.rs`
- Modify: `crates/allium-renderer-wasm/src/protocol.ts`
- Modify: `crates/allium-renderer-wasm/src/renderer.ts`
- Modify: `crates/allium-renderer-wasm/src/worker.ts`
- Modify: `crates/allium-renderer-wasm/src/worker-client.ts`
- Modify: `crates/allium-renderer-wasm/build.sh`
- Modify: `crates/allium-renderer-wasm/package.json`

- [ ] Add failing ABI tests for scene creation, dump/snapshot round trips, mask patches, dynamic ticks, component controls, glyph payloads, and major-version rejection.
- [ ] Port the measured FreeType module set: `truetype`, `cff`, `sfnt`, `psaux`, `psnames`, and `smooth`.
- [ ] Expose stable layer/glyph IDs, layer tree, sources, resolved parameters, geometry, commands, interaction regions, component controls, scene deltas, and telemetry.
- [ ] Publish the semantic core plus minimal FreeType artifact as the only browser entrypoint. Delete the Skia CPU artifact, `render`, `renderAllLayers`, CPU WebP layer protocol, legacy worker/host code, and their dependencies and tests.
- [ ] Move scene creation, ticks, masks, component controls, dumps, and destruction into the worker protocol. The main thread must never call the semantic WASM scene directly.
- [ ] Build and measure in the Emscripten dev container:

```bash
npm ci
npm run build
npm run build:wasm
npm run verify:wasm:minimal
npm run measure:wasm:size
```

Expected: minimal/full font profiles match; the only browser artifact remains within the recorded 844,088 raw / 371,260 gzip / 285,030 brotli baseline plus a documented schema delta. No Skia symbols or CPU image renderer dependencies remain in the packed browser distribution.

## Milestone 3: WebGL2 runtime, worker, and caches

**Files:**

- Create: `crates/allium-renderer-wasm/src/cache/glyph-persistent-cache.ts`
- Create: `crates/allium-renderer-wasm/src/cache/indexed-db-glyph-store.ts`
- Create: `crates/allium-renderer-wasm/src/cache/session-atlas.ts`
- Create: `crates/allium-renderer-wasm/src/cache/session-image-cache.ts`
- Create: `crates/allium-renderer-wasm/src/gpu/semantic-command-planner.ts`
- Create: `crates/allium-renderer-wasm/src/gpu/semantic-command-geometry.ts`
- Create: `crates/allium-renderer-wasm/src/gpu/webgl-atlas-texture.ts`
- Create: `crates/allium-renderer-wasm/src/gpu/webgl-glyph-pipeline.ts`
- Create: `crates/allium-renderer-wasm/src/gpu/webgl-semantic-executor.ts`
- Create: `crates/allium-renderer-wasm/src/gpu/webgl-scene-renderer.ts`
- Create: `crates/allium-renderer-wasm/src/interaction/numeric-text-regions.ts`
- Create: `crates/allium-renderer-wasm/src/telemetry/renderer-telemetry.ts`

- [ ] Port the browser-next tests first and confirm they fail against the old package.
- [ ] Port atlas leases, dirty rectangles, bounded eviction, IndexedDB validation, request singleflight, decoded-resource leases, and context restoration.
- [ ] Port semantic command planning and WebGL2 execution for all twelve authored element types. Shape SDF coverage must multiply the source alpha domain.
- [ ] Port numeric-run geometry and keep product behavior outside the renderer.
- [ ] Run:

```bash
npm run typecheck
npm run test:gates
npm run verify:m3:full-profile-gpu
```

Expected: all contract gates pass; the complete profile executes with authored order, no unknown commands, and zero steady-state command/atlas rebuilds.

## Milestone 4: English Scene Workbench

**Files:**

- Replace: `crates/allium-renderer-wasm/demo/index.html`
- Replace: `crates/allium-renderer-wasm/demo/demo.css`
- Replace: `crates/allium-renderer-wasm/demo/demo.js`
- Delete: `crates/allium-renderer-wasm/demo/static-manifest.js`
- Modify: `crates/allium-renderer-wasm/demo/README.md`
- Create: `crates/allium-renderer-wasm/demo/workbench/session.js`
- Create: `crates/allium-renderer-wasm/demo/workbench/inspector.js`
- Create: `crates/allium-renderer-wasm/demo/workbench/interactions.js`
- Create: `crates/allium-renderer-wasm/demo/workbench/telemetry.js`

- [ ] Set default providers to `https://cdn.emptysekai.com/masterdata/cn/latest` and `https://cdn.emptysekai.com/assets/cn`.
- [ ] Require user-provided card/profile JSON and font files; keep both in session memory only and never upload or persist their source bytes.
- [ ] Render directly into WebGL2. Remove CPU WebP layer stacking, static/dynamic dual URLs, and the local static-key manifest.
- [ ] Implement the three-pane workbench, live stage, timeline, authored layer tree, single/subtree masks, source/parameter/geometry/command inspectors, component controls, numeric copy example, interaction overlays, cache/worker/GPU telemetry, context restore, and dump export.
- [ ] Add an English-only gate that rejects CJK UI strings and a forbidden-resource gate that rejects bundled font, profile, masterdata, and game-image files.
- [ ] Verify responsive desktop/tablet layouts with browser screenshots and keyboard navigation.

## Milestone 5: Native/browser parity and public documentation

**Files:**

- Modify: `README.md`
- Replace: `crates/allium-renderer-wasm/README.md`
- Modify: `crates/allium-renderer-wasm/package.json`
- Modify: `crates/allium-renderer-wasm/LICENSE-EXCEPTION`
- Create: `crates/allium-renderer-wasm/NOTICE`
- Create: `docs/browser-runtime.md`
- Create: `docs/scene-schema.md`
- Create: `docs/resource-providers.md`
- Create: `docs/debug-and-telemetry.md`
- Create: `docs/migrating-to-0.2.md`

- [ ] Document every public type, lifecycle, cache budget, error, privacy rule, CDN default, and caller-owned interaction boundary in English.
- [ ] Document removed 0.1 CPU APIs and their semantic/WebGL2 replacements. Do not provide runtime compatibility shims.
- [ ] Run ABI/schema, TMP debug, glyph SDF, native/WASM state, complete-profile GPU, final pixel, cache soak, context restore, and performance gates.
- [ ] Correct the exception package name to `@empty-sekai/renderer-wasm`, include the FreeType license/notice in the packed artifact, and run license and dependency audits. Confirm no second unintended static libwebp and no non-redistributable resource.
- [ ] Audit the actual `npm pack` file list for English public content, private paths, player data, fonts, game images, and extracted game strings.
- [ ] Run the complete container verification matrix:

```bash
cargo test --workspace --all-features
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
npm --prefix crates/allium-renderer-wasm ci
npm --prefix crates/allium-renderer-wasm run build
npm --prefix crates/allium-renderer-wasm run test:gates
```

Expected: every command passes with no public-language or forbidden-resource violations.

## Milestone 6: Public review and aggregation

- [ ] Rebase the feature branch on `upstream/main` and rerun the complete verification matrix.
- [ ] Push `feat/browser-renderer-v0.2` to the review-bot fork.
- [ ] Open an English pull request to `empty-sekai/allium-renderer:main` with architecture, compatibility, performance, privacy, license, and demo evidence.
- [ ] After merge, update the aggregate repository's `allium-renderer-oss` gitlink together with the already-pushed internal repository pointers.
- [ ] Run the merged package and Scene Workbench as the 0.2.0 release candidate. Fix candidate defects in 0.2.0 itself; do not consume a patch version before the first public release.
- [ ] Do not create a `v0.2.0` tag until the explicit release decision; a tag triggers public release and npm publication.
