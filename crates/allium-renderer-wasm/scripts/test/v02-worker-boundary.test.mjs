import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
import test from "node:test";

const source = (path) => readFile(new URL(`../../${path}`, import.meta.url), "utf8");

test("the public renderer no longer exposes the CPU image API", async () => {
  const renderer = await source("src/renderer.ts");
  assert.doesNotMatch(renderer, /\bcwrap\b/);
  assert.doesNotMatch(renderer, /\bImageFormat\b/);
  assert.doesNotMatch(renderer, /\brenderAllLayers\b/);
  assert.match(renderer, /class BrowserRenderer/);
  assert.match(renderer, /async createProfileScene\(/);
  assert.doesNotMatch(renderer, /async createScene\(/);
});

test("one worker owns scene, layout, and glyph WASM calls", async () => {
  const protocol = await source("src/protocol.ts");
  const atlas = await source("src/fontSdfAtlas.ts");
  const scene = await source("src/gpu/semanticWebglSceneRenderer.ts");

  assert.match(protocol, /kind: "contract"/);
  assert.match(protocol, /kind: "layoutText"/);
  assert.match(protocol, /kind: "glyphDemand"/);
  assert.match(atlas, /RendererWorkerClient/);
  assert.doesNotMatch(atlas, /sharedSdfGlyphWorker|mapGlyphsWithFreeType|getSdfFreeTypeContract/);
  assert.doesNotMatch(scene, /RendererWorkerClient/);
  assert.doesNotMatch(scene, /buildStrictLayoutBatchWithFreeTypeWasm/);
  assert.doesNotMatch(scene, /semanticTextOperationsToLayers/);
  assert.doesNotMatch(scene, /layoutText\(/);
});

test("WASM owns the canonical glyph raster plan and TypeScript only schedules cache I/O", async () => {
  const build = await source("build.sh");
  const protocol = await source("src/protocol.ts");
  const client = await source("src/worker-client.ts");
  const worker = await source("src/worker.ts");
  const atlas = await source("src/fontSdfAtlas.ts");

  assert.match(build, /_sdf_layout_freetype_plan_glyphs_json/);
  assert.match(protocol, /kind: "planGlyphs"/);
  assert.match(client, /async planGlyphs\(/);
  assert.match(worker, /sdf_layout_freetype_plan_glyphs_json/);
  assert.match(atlas, /worker\.planGlyphs\(/);
  assert.doesNotMatch(atlas, /createGlyphRasterIdentity|makeRasterContract/);
  assert.doesNotMatch(atlas, /const (?:BASE_SIZE|SPREAD|ATLAS_WIDTH|ATLAS_HEIGHT)\b/);
  assert.doesNotMatch(atlas, /worker\.contract\(\)|worker\.mapGlyphs\(/);
});

test("font bytes cross the worker boundary once and atlas registries are disposed", async () => {
  const atlas = await source("src/fontSdfAtlas.ts");
  const renderer = await source("src/renderer.ts");
  const buildAtlas = atlas.match(/export async function buildSdfAtlas\([\s\S]*?\n\}/)?.[0] ?? "";

  assert.doesNotMatch(buildAtlas, /registerFont/);
  assert.doesNotMatch(atlas, /const engineRegistry = new Set/);
  assert.match(atlas, /disposeWorkerAtlasSessions/);
  assert.match(renderer, /disposeWorkerAtlasSessions\(this\.worker\)[\s\S]*this\.worker\.terminate\(\)/);
});

test("WASM owns TMP glyph demand and callers do not provide an atlas", async () => {
  const build = await source("build.sh");
  const packageJson = JSON.parse(await source("package.json"));
  const publicEntry = await source("src/index.ts");
  const profileRuntime = await source("src/masterdata_runtime.rs");
  const renderer = await source("src/renderer.ts");
  const createOptions = renderer.match(/export type ProfileSceneCreateOptions = \{([\s\S]*?)\n\};/)?.[1] ?? "";
  assert.match(build, /_sdf_layout_freetype_glyph_demand_json/);
  assert.doesNotMatch(createOptions, /atlas:\s*SdfAtlas/);
  assert.doesNotMatch(createOptions, /lineIndent/);
  assert.match(renderer, /buildSdfAtlas/);
  assert.doesNotMatch(renderer, /bootstrapLayout\.dynamicPrograms\.map/);
  assert.doesNotMatch(renderer, /bootstrapLayout\.dynamicPrograms/);
  assert.doesNotMatch(renderer, /advances_tmp:\s*program\.advancesTmp/);
  assert.match(profileRuntime, /dynamic_programs:\s*Vec<DynamicProgramInput>/);
  assert.match(profileRuntime, /LineIndentSource\s*\{/);
  assert.match(renderer, /frameMode: options\.frameMode \?\? "animate"/);
  assert.match(renderer, /preparedLayoutRequest\(preparation, atlas\)/);
  assert.doesNotMatch(
    renderer.match(/function preparedLayoutRequest\([\s\S]*?\n\}/)?.[0] ?? "",
    /frameMode|tick/,
  );
  assert.match(profileRuntime, /frame_mode:\s*Option<String>/);
  assert.match(profileRuntime, /create_compiled_profile\([\s\S]*static_final/);
  assert.match(renderer, /registerFont\(font:\s*\{\s*family:\s*string;\s*bytes:\s*ArrayBuffer\s*\}/);
  assert.match(renderer, /crypto\.subtle\.digest\("SHA-256"/);
  assert.doesNotMatch(publicEntry, /buildSdfAtlas/);
  assert.doesNotMatch(publicEntry, /RendererWorkerClient,/);
  assert.deepEqual(Object.keys(packageJson.exports), ["."]);
});

test("profile preparation owns complete glyph demand and resolves one atlas", async () => {
  const renderer = await source("src/renderer.ts");
  const worker = await source("src/worker.ts");
  const method = renderer.match(/async createProfileScene\([\s\S]*?\n  }\n\n  async createMasterData/)?.[0] ?? "";

  assert.doesNotMatch(renderer, /preparedAuthoredTextLayers/);
  assert.doesNotMatch(method, /semanticTextOperationsToLayers/);
  assert.equal((method.match(/buildSdfAtlas\(/g) ?? []).length, 1);
  assert.equal((method.match(/buildAtlasForLayers\(/g) ?? []).length, 0);
  assert.match(worker, /case "prepareProfile"[\s\S]+sdf_layout_freetype_glyph_demand_json/);
  assert.match(worker, /glyph_layers/);
  assert.match(worker, /layout_layers/);
  assert.match(worker, /kind: "createProfileScene"/);
  assert.match(worker, /response, layout \}/);
  assert.doesNotMatch(worker, /function authoredLayoutLayers/);
});

test("TypeScript never reconstructs semantic text layout", async () => {
  const renderer = await source("src/renderer.ts");
  const scene = await source("src/gpu/semanticWebglSceneRenderer.ts");
  const worker = await source("src/worker.ts");

  await assert.rejects(access(new URL("../../src/gpu/semanticTextAdapter.ts", import.meta.url)));
  assert.doesNotMatch(renderer, /SemanticTextEnvironment|profileTextEnvironment/);
  assert.doesNotMatch(scene, /SemanticTextEnvironment|semanticTextOperationsToLayers|layoutText\(/);
  assert.doesNotMatch(worker, /payload\.kind\s*!==\s*"text"|decomposeMatrix|outline_size/);
});

test("public scene types do not expose raw worker or core handles", async () => {
  const renderer = await source("src/renderer.ts");
  const entry = await source("src/index.ts");
  const protocol = await source("src/protocol.ts");
  const worker = await source("src/worker.ts");
  const tsconfig = JSON.parse(await source("tsconfig.json"));
  assert.match(renderer, /private readonly worker: RendererWorkerClient/);
  assert.match(renderer, /private readonly core: RendererScene/);
  assert.doesNotMatch(renderer, /readonly plan: SemanticCommandPlan/);
  assert.doesNotMatch(entry, /RendererScene/);
  assert.doesNotMatch(protocol, /createProfileScene[^\n]+dynamicPrograms/);
  assert.match(worker, /case "createProfileScene"[\s\S]+sdf_layout_freetype_build_layout_json[\s\S]+profileCompileRequest/);
  assert.equal(tsconfig.compilerOptions.stripInternal, true);
});

test("direct main-thread WASM adapters are absent", async () => {
  for (const path of [
    "src/wasm/rendererCoreScene.ts",
    "src/wasm/sdfFreeTypeLayout.ts",
    "src/wasm/sdfFreeTypeWasm.ts",
    "src/worker/sdfGlyph.worker.ts",
    "src/worker/sdfGlyphWorkerClient.ts",
  ]) {
    await assert.rejects(access(new URL(`../../${path}`, import.meta.url)));
  }
});

test("WASM string decoding snapshots growable Emscripten memory", async () => {
  const worker = await source("src/worker.ts");
  assert.match(
    worker,
    /TextDecoder\(\)\.decode\(new Uint8Array\(mod\.HEAPU8\.subarray\(pointer, end\)\)\)/,
  );
  assert.doesNotMatch(
    worker,
    /TextDecoder\(\)\.decode\(mod\.HEAPU8\.subarray\(pointer, end\)\)/,
  );
});

test("the container build resolves Cargo home instead of a machine-specific registry", async () => {
  const build = await source("build.sh");
  assert.match(build, /CARGO_HOME/);
  assert.doesNotMatch(build, /find \/usr\/local\/cargo\/registry\/src/);
  assert.doesNotMatch(build, /cargo fetch --locked/);
  assert.match(build, /freetype-sys = "=0\.23\.0"/);
  assert.match(build, /command -v emcc/);
  assert.doesNotMatch(build, /\/opt\/emsdk\/upstream\/emscripten\/system/);
});

test("the Emscripten link driver retains every stateful v0.2 ABI family", async () => {
  const main = await source("src/main.rs");
  assert.match(main, /sdf_layout_freetype_glyph_demand_json/);
  assert.match(main, /sdf_renderer_core_masterdata_create_json/);
  assert.match(main, /sdf_renderer_core_profile_create_json/);
  assert.match(main, /sdf_renderer_core_masterdata_destroy/);
});
