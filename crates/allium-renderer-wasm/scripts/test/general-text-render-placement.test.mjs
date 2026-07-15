import assert from "node:assert/strict";
import test from "node:test";

import { placeGeneralTextInstances } from "../../dist/gpu/generalTextRenderPlacement.js";

test("General placement translates completed glyph geometry and preserves TMP metrics", () => {
  const metrics = {
    lineWidths: [68],
    rectWidths: [68],
    boxW: 100,
    anchorBase: 4.2,
    lineOffsets: [0],
  };
  const instance = {
    layerId: "command",
    plainTextIndex: 0,
    char: "A",
    drawable: true,
    glyphKey: "font:A",
    atlasPage: 0,
    z: 0,
    quad: [[1, 2, 0, 0, 6], [3, 2, 1, 0, 6], [3, 4, 1, 1, 6], [1, 4, 0, 1, 6]],
    charPosition: ["A", 2, -4, 1, 0, 0],
    charOp: ["A", 1, 2, 1, 0, 0, 0],
    charQuad: ["A", [[1, 2], [3, 2], [3, 4], [1, 4]]],
    deviceCharPosition: ["A", 1, 2],
    deviceCharQuad: ["A", [[1, 2], [3, 2], [3, 4], [1, 4]]],
    deviceGlyphQuad: ["A", [[1, 2], [3, 2], [3, 4], [1, 4]]],
    layoutMetrics: metrics,
    fill: [1, 1, 1, 1], outline: [0, 0, 0, 0], outlineWidth: 0,
    shaderFontSize: 12, shaderFaceScale: 1, shaderFaceBias: 0,
    shaderUnderlayScale: 1, shaderUnderlayBias: 0, shaderVertexAlpha: 1,
  };
  const operation = {
    command: {
      id: "command",
      layer_id: "layer",
      role: "label",
      matrix: [1, 0, 0, 1, 5, 6],
      render_placement: { anchor_x: -30, baseline: 1.8 },
      payload: { kind: "text", alignment: 1 },
    },
    layerId: "layer",
    layerSlot: 0,
    baseMatrix: [2, 0, 0, 3, 10, 20],
    visible: true,
    transform: { dx: 0, dy: 0 },
    commandSlot: 0,
    commandVisible: true,
    commandTransform: { dx: 0, dy: 0 },
  };

  const [placed] = placeGeneralTextInstances([instance], [operation]);
  assert.deepEqual(placed.layoutMetrics, metrics);
  assert.notEqual(placed.layoutMetrics, metrics);
  assert.equal(placed.charOp[1], 21);
  assert.ok(Math.abs(placed.charOp[2] + 0.4) < 1e-6);
  assert.equal(placed.charPosition[1], 42);
  assert.ok(Math.abs(placed.charPosition[2] - 0.8) < 1e-6);
  assert.equal(placed.deviceGlyphQuad[1][0][0], 81);
  assert.ok(Math.abs(placed.deviceGlyphQuad[1][0][1] + 12.4) < 1e-6);
  assert.deepEqual(instance.deviceGlyphQuad[1][0], [1, 2]);
});
