import assert from "node:assert/strict";
import test from "node:test";

import { buildNumericTextRegions } from "../../src/interaction/numericTextRegions.ts";

test("numeric runs become stable regions from plain-indexed glyph quads", () => {
  const command = {
    id: "text-1",
    layer_id: "layer-1",
    clip: null,
    numeric_text_runs: [{ text: "1234", plain_start: 0, plain_end: 4 }],
  };
  const instances = [0, 1, 2, 3].map((plainTextIndex) => ({
    layerId: "text-1",
    plainTextIndex,
    deviceCharQuad: [String(plainTextIndex + 1), [[plainTextIndex * 10, 2], [plainTextIndex * 10 + 8, 2], [plainTextIndex * 10 + 8, 12], [plainTextIndex * 10, 12]]],
  }));
  const [region] = buildNumericTextRegions(command, instances, true);
  assert.equal(region.id, "text-1:numeric:0:4");
  assert.equal(region.role, "numeric_run");
  assert.deepEqual(region.bounds, { x: 0, y: 2, width: 38, height: 10 });
  assert.deepEqual(region.resolved_data, { text: "1234", plain_start: 0, plain_end: 4 });
  assert.equal(region.render_mask, true);
  assert.deepEqual(region.capabilities, []);
});

test("hidden commands keep dump data but cannot be hit", () => {
  const [region] = buildNumericTextRegions({
    id: "text-2", layer_id: "layer-2", clip: null,
    numeric_text_runs: [{ text: "05", plain_start: 0, plain_end: 2 }],
  }, [{ layerId: "text-2", plainTextIndex: 0, deviceCharQuad: ["0", [[0, 0], [1, 0], [1, 1], [0, 1]]] }], false);
  assert.equal(region.render_mask, false);
});

test("numeric regions follow the latest layer and command translations", () => {
  const [region] = buildNumericTextRegions({
    id: "text-moving", layer_id: "layer-moving",
    clip: [[0, 0], [20, 0], [20, 10], [0, 10]],
    numeric_text_runs: [{ text: "42", plain_start: 0, plain_end: 2 }],
  }, [{
    layerId: "text-moving",
    plainTextIndex: 0,
    deviceCharQuad: ["4", [[1, 2], [9, 2], [9, 12], [1, 12]]],
  }], true, {
    layerTransform: { dx: 12, dy: -4 },
    commandTransform: { dx: 3, dy: 7 },
  });

  assert.deepEqual(region.bounds, { x: 16, y: 5, width: 8, height: 10 });
  assert.deepEqual(region.quad, [[16, 5], [24, 5], [24, 15], [16, 15]]);
  assert.deepEqual(region.hit_geometry, region.quad);
  assert.deepEqual(region.clip, [[12, -4], [32, -4], [32, 6], [12, 6]]);
});
