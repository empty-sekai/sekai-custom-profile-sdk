import assert from "node:assert/strict";
import test from "node:test";

import { buildAuthoredTextHitGeometry } from "../../dist/gpu/semanticWebglSceneRenderer.js";

const operation = (id, layerId, kind) => ({ command: { id, payload: { kind } }, layerId });
const glyph = (commandId, x) => ({
  layerId: commandId,
  deviceCharQuad: ["char", [[x, 0], [x + 10, 0], [x + 10, 20], [x, 20]]],
});

test("glyph quads replace the hit geometry of authored text layers only", () => {
  const operations = [
    operation("authored", "text-layer", "text"),
    operation("background", "panel-layer", "shape"),
    operation("title", "panel-layer", "text"),
    operation("caption", "panel-layer", "text"),
    operation("frame", "honor-layer", "image"),
    operation("progress", "honor-layer", "text"),
  ];
  const instances = [glyph("authored", 0), glyph("title", 100), glyph("caption", 200), glyph("progress", 300)];

  const geometry = buildAuthoredTextHitGeometry(operations, instances);
  // A layer that also draws shapes, images, or several text runs keeps the
  // core's layer hit geometry instead of shrinking to one of its text runs.
  assert.deepEqual([...geometry.keys()], ["text-layer"]);
  assert.deepEqual(geometry.get("text-layer").bounds, { x: 0, y: 0, width: 10, height: 20 });
});
