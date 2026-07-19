import assert from "node:assert/strict";
import test from "node:test";
import { buildAuthoredTextHitGeometry } from "../../dist/gpu/semanticWebglSceneRenderer.js";

test("authored text hit geometry unions rotated character quads in their shared basis", () => {
  const operations = [{ command: { id: "command" }, layerId: "layer" }];
  const instances = [
    { layerId: "command", deviceCharQuad: ["char", [[10, 20], [10, 30], [4, 30], [4, 20]]] },
    { layerId: "command", deviceCharQuad: ["char", [[10, 30], [10, 42], [4, 42], [4, 30]]] },
  ];

  const geometry = buildAuthoredTextHitGeometry(operations, instances).get("layer");
  assert.ok(geometry);
  assert.deepEqual(geometry.quad, [[10, 20], [10, 42], [4, 42], [4, 20]]);
  assert.deepEqual(geometry.bounds, { x: 4, y: 20, width: 6, height: 22 });
});
