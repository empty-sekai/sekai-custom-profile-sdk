import assert from "node:assert/strict";
import test from "node:test";

import "./register-typescript.mjs";

const { compileSemanticTextGlyphBatches } = await import("../../src/gpu/semanticTextGlyphBridge.ts");
const { SDF_GLYPH_INSTANCE_ATTRIBUTES } = await import("../../src/gpu/webglSdfGlyphPipeline.ts");

test("semantic text glyphs retain command identity but consume authored layer slots", () => {
  const operation = (commandId, slot) => ({
    command: { id: commandId, layer_id: `layer-${slot}`, role: "text", payload: { kind: "text" } },
    layerId: `layer-${slot}`,
    layerSlot: slot,
    baseMatrix: [1, 0, 0, 1, 0, 0],
    visible: true,
    transform: { dx: 0, dy: 0 },
    commandSlot: slot,
    commandVisible: true,
    commandTransform: { dx: 0, dy: 0 },
  });
  const glyph = (commandId, x) => ({
    layerId: commandId,
    drawable: true,
    quad: [[x, 0, 0, 0], [x + 10, 0, 1, 0], [x + 10, 10, 1, 1], [x, 10, 0, 1]],
    fill: [1, 1, 1, 1], outline: [0, 0, 0, 0],
    shaderFaceScale: 1, shaderFaceBias: 0, shaderUnderlayScale: 1, shaderUnderlayBias: 0,
    shaderVertexAlpha: 1, atlasPage: 0,
  });
  const batches = compileSemanticTextGlyphBatches(
    [glyph("command-a", 0), glyph("command-b", 20)],
    [operation("command-a", 3), operation("command-b", 7)],
    { width: 16, height: 16 },
  );
  assert.deepEqual([...batches.keys()], ["command-a", "command-b"]);
  const layerSlot = SDF_GLYPH_INSTANCE_ATTRIBUTES.instanceMeta.offset + 1;
  assert.equal(batches.get("command-a")[layerSlot], 3);
  assert.equal(batches.get("command-b")[layerSlot], 7);
});
