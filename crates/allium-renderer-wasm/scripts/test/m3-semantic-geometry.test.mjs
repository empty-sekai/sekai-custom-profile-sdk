import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
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

test("rounded rectangles retain local pixel radii and full two-axis bounds", () => {
  const layer = "player-level";
  const plan = new SemanticCommandPlan({
    layerTableRevision: 1,
    layerTable: [{ layer_id: layer, parent_id: null, slot: 0, subtree_start: 0, subtree_end: 1 }],
    layerSources: [{ layer_id: layer, matrix: [0.72, 0, 0, 0.72, 100, 200] }],
    layerCommands: [{ layer_id: layer, render_mask: true, transform: { dx: 0, dy: 0 }, command_start: 0, command_count: 1 }],
    semanticCommands: [{
      id: "pill",
      layer_id: layer,
      role: "background",
      bounds: { x: -110, y: -26, width: 220, height: 52 },
      matrix: [1, 0, 0, 1, 0, 0],
      payload: {
        kind: "shape",
        primitive: { rounded_rect: { radius: [26, 26] } },
        fill: [0.15, 0.15, 0.2, 0.85],
        stroke: [1, 1, 1, 0.15],
        stroke_width: 1,
      },
    }],
  });
  const [batch] = compileSemanticDrawBatches(plan.operations());
  const vertex = Array.from(batch.vertices.slice(0, SEMANTIC_FLOATS_PER_VERTEX));
  assert.deepEqual(vertex.slice(14, 18), [1, 26, 26, 1]);
  assert.deepEqual(vertex.slice(26, 28), [220, 52]);
});

test("ellipse strokes compare pixel widths against pixel-space distance", async () => {
  const executor = await readFile(new URL("../../src/gpu/webglSemanticCommandExecutor.ts", import.meta.url), "utf8");
  assert.match(executor, /float localRadius = min\(v_shapeSize\.x, v_shapeSize\.y\) \* 0\.5;/);
  assert.match(executor, /\(length\(\(v_shapeUv - 0\.5\) \* 2\.0\) - 1\.0\) \* localRadius/);

  const radius = 88;
  const strokeHalfWidth = 2;
  const distanceAtCenter = (0 - 1) * radius;
  const distanceAtBoundary = (1 - 1) * radius;
  assert.ok(Math.abs(distanceAtCenter) > strokeHalfWidth);
  assert.ok(Math.abs(distanceAtBoundary) < strokeHalfWidth);
});

test("authored component scaling transforms geometry and viewport clips with one layer matrix", () => {
  const layer = "compact-character-rank";
  const plan = new SemanticCommandPlan({
    layerTableRevision: 1,
    layerTable: [{ layer_id: layer, parent_id: null, slot: 0, subtree_start: 0, subtree_end: 1 }],
    layerSources: [{ layer_id: layer, matrix: [0.88, 0, 0, 0.88, 1370, 406] }],
    layerCommands: [{ layer_id: layer, render_mask: true, transform: { dx: 0, dy: -273 }, command_start: 0, command_count: 1 }],
    semanticCommands: [{
      id: "character-21-background",
      layer_id: layer,
      role: "character-21-background",
      bounds: { x: -388, y: -131.8, width: 175, height: 60.8 },
      matrix: [1, 0, 0, 1, 0, 0],
      clip: [[-483.5, -175.5], [483.5, -175.5], [483.5, 286], [-483.5, 286]],
      payload: { kind: "shape", primitive: { rounded_rect: { radius: [30.4, 30.4] } }, fill: [0, 1, 1, 1], stroke: [0, 0, 0, 0], stroke_width: 0 },
    }],
  });
  const [batch] = compileSemanticDrawBatches(plan.operations());
  const vertex = Array.from(batch.vertices.slice(0, SEMANTIC_FLOATS_PER_VERTEX));
  assert.deepEqual(vertex.slice(0, 2), [1028.56005859375, 290.0159912109375]);
  assert.deepEqual(vertex.slice(18, 26), [944.52001953125, 251.55999755859375, 1795.47998046875, 251.55999755859375, 1795.47998046875, 657.6799926757812, 944.52001953125, 657.6799926757812]);
});

test("group isolation markers and DstIn mask remain ordered, distinct GPU batches", () => {
  const layer = "honors";
  const common = { layer_id: layer, bounds: { x: 0, y: 0, width: 180, height: 80 }, matrix: [1, 0, 0, 1, 0, 0] };
  const image = (id, key, blend_mode = "src_over") => ({
    ...common,
    id,
    role: id,
    blend_mode,
    payload: { kind: "image", resource: { namespace: "assets", key }, alpha_mask: null, uv: { x: 0, y: 0, width: 1, height: 1 }, tint: [1, 1, 1, 1] },
  });
  const commands = [
    { ...common, id: "begin", role: "bonds_group_begin", payload: { kind: "composite", operation: "begin_isolation", opacity: 1, clip: null } },
    image("background-a", "honor/background-a"),
    image("background-b", "honor/background-b"),
    image("mask", "honor/mask", "dst_in"),
    { ...common, id: "end", role: "bonds_group_end", payload: { kind: "composite", operation: "end_isolation", opacity: 1, clip: null } },
    image("frame", "honor/frame"),
  ];
  const plan = new SemanticCommandPlan({
    layerTableRevision: 1,
    layerTable: [{ layer_id: layer, parent_id: null, slot: 0, subtree_start: 0, subtree_end: 1 }],
    layerSources: [{ layer_id: layer, matrix: [1, 0, 0, 1, 0, 0] }],
    layerCommands: [{ layer_id: layer, render_mask: true, transform: { dx: 0, dy: 0 }, command_start: 0, command_count: commands.length }],
    semanticCommands: commands,
  });
  const batches = compileSemanticDrawBatches(plan.operations());
  assert.deepEqual(batches.map((batch) => batch.compositeOperation), ["begin_isolation", null, null, null, "end_isolation", null]);
  assert.deepEqual(batches.map((batch) => batch.commandIds), [["begin"], ["background-a"], ["background-b"], ["mask"], ["end"], ["frame"]]);
  assert.equal(batches[3].blendMode, "dst_in");
  assert.equal(batches[2].blendMode, "src_over");
  assert.equal(batches[3].maskResource, null);
  assert.equal(commands.slice(1, 4).every((command) => command.payload.alpha_mask == null), true);
});

test("WebGL executor reuses full-card isolation targets and applies group-level DstIn", async () => {
  const source = await readFile(new URL("../../src/gpu/webglSemanticCommandExecutor.ts", import.meta.url), "utf8");
  assert.match(source, /operation === "begin_isolation"/);
  assert.match(source, /operation === "end_isolation"/);
  assert.match(source, /this\.isolationTargets\[depth\]/);
  assert.match(source, /gl\.RGBA8, this\.canvasWidth, this\.canvasHeight/);
  assert.match(source, /mode === "dst_in"\) gl\.blendFuncSeparate\(gl\.ZERO, gl\.SRC_ALPHA, gl\.ZERO, gl\.SRC_ALPHA\)/);
  assert.match(source, /isolationTargetAllocations/);
  assert.doesNotMatch(source.slice(source.indexOf("draw(): SemanticGpuMetrics"), source.indexOf("destroy(): void")), /createFramebuffer\(/);
});
