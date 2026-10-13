import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  RendererRuntimeTelemetry,
  assertTelemetryPrivacy,
} from "../../src/telemetry/rendererTelemetry.ts";

const GPU_METRICS = {
  drawCalls: 3,
  geometryBuilds: 1,
  vertexBytes: 4096,
  textureUploads: 2,
  textureBytes: 8192,
  stateUploadBytes: 0,
  maskUploadBytes: 0,
  glyphGeometryBuilds: 1,
  isolationBegins: 1,
  isolationComposites: 1,
  isolationTargetAllocations: 1,
  isolationTextureBytes: 5943840,
};

function tracker() {
  return new RendererRuntimeTelemetry({ level: "trace", maxSamples: 240 }, {
    backend: "edt",
    pages: 1,
    glyphs: 1,
    missingGlyphs: 0,
    generation: { glyphs: 1, pixels: 64, glyphMs: 4, faceLoadMs: 1 },
    cache: { hits: 1, misses: 0, generations: 0, bytes: 128, sessionHits: 1, persistentHits: 0, persistentMisses: 0, persistentWritesQueued: 0, pinnedPages: 1, pageEvictions: 0 },
  }, { textCommands: 2, glyphInstances: 5, atlasUploadBytes: 128, atlasUploadRects: 1 });
}

test("runtime recovery counters advance without changing atlas identity", () => {
  const runtime = tracker();
  runtime.recordPatch({ stateUploadBytes: 0, maskUploadBytes: 1, commandMaskUploadBytes: 0, commandStateUploadBytes: 0 });
  runtime.markContextLost();
  assert.equal(runtime.state(), "context-lost");
  runtime.markContextRestored(12, { atlasUploadBytes: 4096, atlasUploadRects: 1, textureUploads: 2, textureBytes: 8192 });
  const stats = runtime.snapshot();
  assert.equal(stats.recovery.contextLosses, 1);
  assert.equal(stats.recovery.contextRestores, 1);
  assert.equal(stats.gpuEpoch, 2);
  assert.equal(stats.atlas.glyphs, 1);
  assert.equal(stats.updates.maskUploadBytes, 1);
});

test("failed recovery remains context-lost and is counted", () => {
  const runtime = tracker();
  runtime.markContextLost();
  runtime.markRestoreFailed(7);
  const stats = runtime.snapshot();
  assert.equal(stats.state, "context-lost");
  assert.equal(stats.recovery.restoreFailures, 1);
  assert.equal(stats.recovery.lastRestoreMs, 7);
});

test("scene stats expose bounded privacy-safe trace and aggregate atlas performance", () => {
  const runtime = tracker();
  for (let index = 0; index < 300; index += 1) runtime.recordDraw(GPU_METRICS, 0.25);
  const stats = runtime.snapshot();
  assert.equal(stats.lastGpu.drawCalls, 3);
  assert.equal(stats.lastGpu.isolationTargetAllocations, 1);
  assert.equal(stats.atlas.glyphs, 1);
  assert.equal(stats.atlas.backend, "edt");
  assert.equal(stats.telemetry.samples.length, 240);
  assert.equal(stats.telemetry.recordedFrames, 300);
  assertTelemetryPrivacy(stats);
});

test("scene source restores retained GPU inputs without re-running worker layout", async () => {
  const source = await readFile(new URL("../../src/gpu/semanticWebglSceneRenderer.ts", import.meta.url), "utf8");
  assert.match(source, /restoreContext\(gl: WebGL2RenderingContext\)/);
  assert.match(source, /this\.retainedScene/);
  const restoreBody = source.slice(source.indexOf("restoreContext("), source.indexOf("interactionRegions("));
  assert.doesNotMatch(restoreBody, /layoutText\(/);
  assert.match(restoreBody, /setSdfAtlas\(retained\.atlas\)/);
  assert.match(restoreBody, /setTextGlyphBatch/);
});

test("HTML canvas lifecycle automatically prevents loss and restores every live scene", async () => {
  const source = await readFile(new URL("../../src/renderer.ts", import.meta.url), "utf8");
  assert.match(source, /addEventListener\(["']webglcontextlost["']/);
  assert.match(source, /event\.preventDefault\(\)/);
  assert.match(source, /addEventListener\(["']webglcontextrestored["']/);
  assert.match(source, /scene\.notifyContextLost\(\)/);
  assert.match(source, /scene\.restoreContext\(/);
  assert.match(source, /scene\.notifyContextRestoreFailed\(/);
  assert.match(source, /removeEventListener\(["']webglcontextlost["']/);
  assert.match(source, /removeEventListener\(["']webglcontextrestored["']/);
  const restoreStart = source.indexOf("async restoreContext(): Promise<void>");
  const restoreEnd = source.indexOf("destroy(): void", restoreStart);
  assert.doesNotMatch(source.slice(restoreStart, restoreEnd), /this\.assertAlive\(\)/);
});

test("renderer stats remain readable while the WebGL context is lost", async () => {
  const source = await readFile(new URL("../../src/renderer.ts", import.meta.url), "utf8");
  const statsStart = source.indexOf("async stats()");
  const statsEnd = source.indexOf("async restoreContext()", statsStart);
  const statsMethod = source.slice(statsStart, statsEnd);
  assert.match(statsMethod, /this\.destroyed/);
  assert.doesNotMatch(statsMethod, /this\.assertAlive\(\)/);
  assert.match(statsMethod, /contextLost/);
});

test("layer visibility stays on the patch-only path", async () => {
  const source = await readFile(new URL("../../src/renderer.ts", import.meta.url), "utf8");
  const start = source.indexOf("async setLayerVisible");
  const end = source.indexOf("async setLayerMasks", start);
  const method = source.slice(start, end);
  assert.match(method, /core\.setLayerVisible/);
  assert.doesNotMatch(method, /buildSdfAtlas|layoutText|restoreContext|setScene/);
});
