import assert from "node:assert/strict";
import test from "node:test";

import { SemanticCommandPlan } from "../../src/gpu/semanticCommandPlanner.ts";
import { compileSemanticDrawBatches, SEMANTIC_FLOATS_PER_VERTEX } from "../../src/gpu/semanticCommandGeometry.ts";

test("semantic geometry preserves cross-pipeline order and authored transforms", () => {
  const layer = "layer";
  const common = { layer_id: layer, bounds: { x: 0, y: 0, width: 10, height: 20 }, matrix: [1, 0, 0, 1, 2, 3] };
  const plan = new SemanticCommandPlan({
    layerTableRevision: 1,
    layerTable: [{ layer_id: layer, parent_id: null, slot: 0, subtree_start: 0, subtree_end: 1 }],
    layerSources: [{ layer_id: layer, matrix: [2, 0, 0, 2, 100, 200] }],
    layerCommands: [{ layer_id: layer, render_mask: true, transform: { dx: 0, dy: 0 }, command_start: 0, command_count: 4 }],
    semanticCommands: [
      { ...common, id: "shape", role: "background", payload: { kind: "shape", primitive: "rect", fill: [1, 0, 0, 1], stroke: [0, 0, 0, 0], stroke_width: 0 } },
      { ...common, id: "text", role: "title", payload: { kind: "text", source: { kind: "authored", value: "A" } } },
      { ...common, id: "image", role: "image", payload: { kind: "image", resource: { namespace: "assets", key: "a" }, uv: { x: 0, y: 0, width: 1, height: 1 }, tint: [1, 1, 1, 1] } },
      { ...common, id: "ellipse", role: "dot", payload: { kind: "shape", primitive: "ellipse", fill: [0, 1, 0, 1], stroke: [0, 0, 0, 0], stroke_width: 0 } },
    ],
  });
  const batches = compileSemanticDrawBatches(plan.operations());
  assert.deepEqual(batches.map((batch) => batch.kind), ["shape", "text", "image", "shape"]);
  assert.deepEqual(batches.map((batch) => batch.commandIds), [["shape"], ["text"], ["image"], ["ellipse"]]);
  assert.deepEqual(Array.from(batches[0].vertices.slice(0, 2)), [104, 206]);
  assert.equal(batches[0].layerSlots[0], 0);
});

test("adjacent compatible shapes batch while mask and dynamic state stay out of static geometry", () => {
  const layer = "layer";
  const command = (id, x) => ({
    id,
    layer_id: layer,
    role: id,
    bounds: { x, y: 0, width: 10, height: 10 },
    matrix: [1, 0, 0, 1, 0, 0],
    payload: { kind: "shape", primitive: "rect", fill: [1, 1, 1, 1], stroke: [0, 0, 0, 0], stroke_width: 0 },
  });
  const plan = new SemanticCommandPlan({
    layerTableRevision: 1,
    layerTable: [{ layer_id: layer, parent_id: null, slot: 0, subtree_start: 0, subtree_end: 1 }],
    layerSources: [{ layer_id: layer, matrix: [1, 0, 0, 1, 0, 0] }],
    layerCommands: [{ layer_id: layer, render_mask: false, transform: { dx: 50, dy: 60 }, command_start: 0, command_count: 2 }],
    semanticCommands: [command("a", 0), command("b", 20)],
  });
  const batches = compileSemanticDrawBatches(plan.operations());
  assert.equal(batches.length, 1);
  assert.equal(batches[0].vertices.length, 12 * SEMANTIC_FLOATS_PER_VERTEX);
  assert.deepEqual(Array.from(batches[0].vertices.slice(0, 2)), [0, 0]);
  assert.deepEqual(batches[0].commandIds, ["a", "b"]);
});
