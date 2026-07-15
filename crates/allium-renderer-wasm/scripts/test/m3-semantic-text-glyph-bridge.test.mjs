import assert from "node:assert/strict";
import test from "node:test";

import "./register-typescript.mjs";

const { compileSemanticTextGlyphBatches } = await import("../../src/gpu/semanticTextGlyphBridge.ts");

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
    [operation("command-a", 3), operation("command-b", 7)]
  );
  assert.deepEqual([...batches.keys()], ["command-a", "command-b"]);
  assert.equal(batches.get("command-a")[25], 3);
  assert.equal(batches.get("command-b")[25], 7);
});
