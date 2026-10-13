import assert from "node:assert/strict";
import test from "node:test";

import { SemanticCommandPlan, semanticCommandPlanFromCoreDump } from "../../src/gpu/semanticCommandPlanner.ts";

const layerA = "00000000000000a1";
const layerB = "00000000000000b1";

function fixture() {
  return {
    layerTableRevision: 7,
    layerTable: [
      { layer_id: layerA, parent_id: null, slot: 0, subtree_start: 0, subtree_end: 1 },
      { layer_id: layerB, parent_id: null, slot: 1, subtree_start: 1, subtree_end: 2 },
    ],
    layerSources: [
      { layer_id: layerA, matrix: [1, 0, 0, 1, 10, 20] },
      { layer_id: layerB, matrix: [0, 1, -1, 0, 30, 40] },
    ],
    layerCommands: [
      { layer_id: layerA, render_mask: true, transform: { dx: 0, dy: 0 }, command_start: 0, command_count: 2 },
      { layer_id: layerB, render_mask: true, transform: { dx: 0, dy: 0 }, command_start: 2, command_count: 2 },
    ],
    semanticCommands: [
      { id: "c1", layer_id: layerA, role: "background", payload: { kind: "shape", primitive: "rect" } },
      { id: "c2", layer_id: layerA, role: "title", payload: { kind: "text", source: { kind: "localized", key: "title", locale: "en", value: "Title" } } },
      { id: "c3", layer_id: layerB, role: "image", payload: { kind: "image", resource: { namespace: "assets", key: "card/a" } } },
      { id: "c4", layer_id: layerB, role: "mask", payload: { kind: "shape", primitive: { asset_mask: { resource: { namespace: "assets", key: "card/a" } } } } },
    ],
  };
}

test("planner preserves authored layer spans and command order", () => {
  const plan = new SemanticCommandPlan(fixture());
  assert.deepEqual(plan.operations().map((op) => [op.command.id, op.layerId, op.layerSlot]), [
    ["c1", layerA, 0],
    ["c2", layerA, 0],
    ["c3", layerB, 1],
    ["c4", layerB, 1],
  ]);
  assert.deepEqual(plan.operations()[2].baseMatrix, [0, 1, -1, 0, 30, 40]);
  assert.deepEqual(plan.resourceRequests(), [{ namespace: "assets", key: "card/a" }]);
});

test("mask and transform deltas update state without rebuilding commands or resources", () => {
  const plan = new SemanticCommandPlan(fixture());
  const commandRevision = plan.commandRevision;
  const resourceRevision = plan.resourceRevision;
  plan.applyLayerPatches([{ layer_id: layerB, render_mask: false, transform: { dx: 12, dy: -4 } }]);
  const image = plan.operations().find((op) => op.command.id === "c3");
  assert.equal(image.visible, false);
  assert.deepEqual(image.transform, { dx: 12, dy: -4 });
  assert.equal(plan.commandRevision, commandRevision);
  assert.equal(plan.resourceRevision, resourceRevision);
});

test("invalid command spans and unknown delta layers fail closed", () => {
  const broken = fixture();
  broken.layerCommands[0].command_count = 3;
  assert.throws(() => new SemanticCommandPlan(broken), /command span ownership mismatch/);
  const plan = new SemanticCommandPlan(fixture());
  assert.throws(
    () => plan.applyLayerPatches([{ layer_id: "missing", render_mask: false, transform: null }]),
    /unknown layer patch/
  );
  const missingMatrix = fixture();
  missingMatrix.layerSources.pop();
  assert.throws(() => new SemanticCommandPlan(missingMatrix), /missing authored layer matrix/);
});

test("native scene dumps flatten through authored layer-table order", () => {
  const source = fixture();
  const plan = semanticCommandPlanFromCoreDump({
    revisions: { layer_table: 7 },
    layer_table: source.layerTable,
    layers: [
      { layer_id: layerB, matrix: source.layerSources[1].matrix, render_mask: false, commands: source.semanticCommands.slice(2) },
      { layer_id: layerA, matrix: source.layerSources[0].matrix, render_mask: true, commands: source.semanticCommands.slice(0, 2) },
    ],
    command_states: source.semanticCommands.map((command, slot) => ({ command_id: command.id, slot, render_mask: true, transform: { dx: 0, dy: 0 } })),
  });
  assert.deepEqual(plan.operations().map((operation) => operation.command.id), ["c1", "c2", "c3", "c4"]);
  assert.equal(plan.operations()[2].visible, false);
  assert.deepEqual(plan.operations()[2].baseMatrix, source.layerSources[1].matrix);
});
