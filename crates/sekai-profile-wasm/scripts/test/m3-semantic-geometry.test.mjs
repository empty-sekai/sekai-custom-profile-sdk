import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { SemanticCommandPlan } from "../../src/gpu/semanticCommandPlanner.ts";
import {
  compileSemanticDrawBatches,
  composeMatrix,
  invertMatrix,
  SEMANTIC_COMMAND_SLOT_LOCATION,
  SEMANTIC_FLOATS_PER_VERTEX,
  SEMANTIC_LAYER_SLOT_LOCATION,
  SEMANTIC_VERTEX_ATTRIBUTES as LAYOUT,
  transformPoint,
} from "../../src/gpu/semanticCommandGeometry.ts";

const executorSource = await readFile(new URL("../../src/gpu/webglSemanticCommandExecutor.ts", import.meta.url), "utf8");

/** One attribute of a batch vertex, with negative zeros as zeros. */
function attribute(batch, vertex, name) {
  const { offset, size } = LAYOUT[name];
  const base = vertex * SEMANTIC_FLOATS_PER_VERTEX + offset;
  return Array.from(batch.vertices.slice(base, base + size), (value) => value + 0);
}

/** The canvas-to-local matrix a vertex carries. */
function inverseOf(batch, vertex = 0) {
  return [...attribute(batch, vertex, "inverse"), ...attribute(batch, vertex, "inverseOffset")];
}

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
  // Canvas pixels map back through the layer and the command matrix.
  assert.deepEqual(inverseOf(batches[0]), [0.5, 0, 0, 0.5, -52, -103]);
  assert.deepEqual(attribute(batches[0], 0, "bounds"), [0, 0, 10, 20]);
  assert.equal(batches[0].layerSlots[0], 0);
});

test("every vertex attribute has the shader location the layout names", () => {
  const vertexShader = executorSource.slice(executorSource.indexOf("const VERTEX_SHADER"), executorSource.indexOf("const FRAGMENT_COMMON"));
  const declared = [...vertexShader.matchAll(/^layout\(location=(\d+)\) in (\w+) a_(\w+);$/gm)]
    .map(([, location, type, name]) => [name, Number(location), type]);
  const expected = Object.entries(LAYOUT).map(([name, { location, size }]) => [name, location, `vec${size}`]);
  expected.push(["layerSlot", SEMANTIC_LAYER_SLOT_LOCATION, "uint"], ["commandSlot", SEMANTIC_COMMAND_SLOT_LOCATION, "uint"]);
  assert.deepEqual(declared.sort((a, b) => a[1] - b[1]), expected.sort((a, b) => a[1] - b[1]));
  // The float attributes tile the vertex without gaps or overlap.
  const spans = Object.values(LAYOUT).map(({ offset, size }) => [offset, offset + size]).sort((a, b) => a[0] - b[0]);
  spans.forEach(([start], index) => assert.equal(start, index === 0 ? 0 : spans[index - 1][1]));
  assert.equal(spans.at(-1)[1], SEMANTIC_FLOATS_PER_VERTEX);
  // WebGL2 guarantees sixteen attributes.
  assert.ok(Math.max(...declared.map(([, location]) => location)) < 16);
});

test("matrices compose and invert in single precision in the native order of operations", () => {
  const f = Math.fround;
  const layer = [0.88, 0, 0, 0.88, 1370, 406];
  const command = [Math.cos(0.5), Math.sin(0.5), -Math.sin(0.5), Math.cos(0.5), 12.25, -7.5];
  const device = composeMatrix(layer, command);
  const [p, c] = [layer.map(f), command.map(f)];
  assert.equal(device[0], f(f(p[0] * c[0]) + f(p[2] * c[1])));
  assert.equal(device[4], f(f(f(p[0] * c[4]) + f(p[2] * c[5])) + p[4]));
  assert.ok(device.every((value) => value === f(value)));
  const inverse = invertMatrix(device);
  const determinant = f(f(device[0] * device[3]) - f(device[1] * device[2]));
  assert.equal(inverse[0], f(device[3] / determinant));
  assert.equal(inverse[4], f(f(f(device[2] * device[5]) - f(device[3] * device[4])) / determinant));
  // A canvas point maps back to where it came from.
  const [x, y] = transformPoint(inverse, ...transformPoint(device, 3.25, -9.5));
  assert.ok(Math.abs(x - 3.25) < 1e-3 && Math.abs(y + 9.5) < 1e-3, `${x}, ${y}`);
  // The native compositor refuses a matrix without an inverse.
  assert.equal(invertMatrix([1e-4, 0, 0, 1e-4, 0, 0]), null);
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
  // The layer offset stays out of the static inverse.
  assert.deepEqual(inverseOf(batches[0]), [1, 0, 0, 1, 0, 0]);
  assert.deepEqual(attribute(batches[0], 0, "bounds"), [0, 0, 10, 10]);
  assert.deepEqual(attribute(batches[0], 6, "bounds"), [20, 0, 10, 10]);
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
  assert.deepEqual(attribute(batch, 0, "params"), [1, 26, 26, 1]);
  assert.deepEqual(attribute(batch, 0, "bounds"), [-110, -26, 220, 52]);
  assert.deepEqual(attribute(batch, 0, "fill"), [0.15, 0.15, 0.2, 0.85].map(Math.fround));
  assert.deepEqual(attribute(batch, 0, "stroke"), [1, 1, 1, 0.15].map(Math.fround));
  // No gradient: an empty line and the fill at both ends.
  assert.deepEqual(attribute(batch, 0, "gradient"), [0, 0, 0, 0]);
  assert.deepEqual(attribute(batch, 0, "gradientEndColor"), attribute(batch, 0, "fill"));
  // The six vertices share every value but their corner.
  assert.deepEqual([0, 1, 2, 3, 4, 5].map((vertex) => attribute(batch, vertex, "corner")), [[0, 0], [1, 0], [1, 1], [0, 0], [1, 1], [0, 1]]);
  for (let vertex = 1; vertex < 6; vertex += 1) {
    for (const name of Object.keys(LAYOUT).filter((name) => name !== "corner")) {
      assert.deepEqual(attribute(batch, vertex, name), attribute(batch, 0, name), `${name} of vertex ${vertex}`);
    }
  }
});

test("a gradient shape carries its line and end colours for the fragment stage", () => {
  const layer = "append";
  const plan = new SemanticCommandPlan({
    layerTableRevision: 1,
    layerTable: [{ layer_id: layer, parent_id: null, slot: 0, subtree_start: 0, subtree_end: 1 }],
    layerSources: [{ layer_id: layer, matrix: [1, 0, 0, 1, 0, 0] }],
    layerCommands: [{ layer_id: layer, render_mask: true, transform: { dx: 0, dy: 0 }, command_start: 0, command_count: 1 }],
    semanticCommands: [{
      id: "label",
      layer_id: layer,
      role: "label",
      bounds: { x: 0, y: 0, width: 100, height: 20 },
      matrix: [1, 0, 0, 1, 0, 0],
      payload: {
        kind: "shape",
        primitive: { rounded_rect: { radius: [8, 8] } },
        fill: [0.5, 0.5, 0.5, 1],
        gradient: { start: [0, 0.5], end: [1, 0.5], start_color: [1, 0, 0, 1], end_color: [0, 0, 1, 0.5] },
        stroke: [0, 0, 0, 0],
        stroke_width: 0,
      },
    }],
  });
  const [batch] = compileSemanticDrawBatches(plan.operations());
  assert.deepEqual(attribute(batch, 0, "fill"), [1, 0, 0, 1]);
  assert.deepEqual(attribute(batch, 0, "gradient"), [0, 0.5, 1, 0.5]);
  assert.deepEqual(attribute(batch, 0, "gradientEndColor"), [0, 0, 1, 0.5]);
});

test("empty bounds and matrices without an inverse contribute no vertices", () => {
  const layer = "layer";
  const shape = (id, bounds, matrix) => ({
    id, layer_id: layer, role: id, bounds, matrix,
    payload: { kind: "shape", primitive: "rect", fill: [1, 1, 1, 1], stroke: [0, 0, 0, 0], stroke_width: 0 },
  });
  const plan = new SemanticCommandPlan({
    layerTableRevision: 1,
    layerTable: [{ layer_id: layer, parent_id: null, slot: 0, subtree_start: 0, subtree_end: 1 }],
    layerSources: [{ layer_id: layer, matrix: [1, 0, 0, 1, 0, 0] }],
    layerCommands: [{ layer_id: layer, render_mask: true, transform: { dx: 0, dy: 0 }, command_start: 0, command_count: 3 }],
    semanticCommands: [
      shape("flat", { x: 0, y: 0, width: 10, height: 10 }, [1, 0, 2, 0, 0, 0]),
      shape("empty", { x: 0, y: 0, width: 0, height: 10 }, [1, 0, 0, 1, 0, 0]),
      shape("drawn", { x: 0, y: 0, width: 10, height: 10 }, [1, 0, 0, 1, 0, 0]),
    ],
  });
  const [batch] = compileSemanticDrawBatches(plan.operations());
  assert.deepEqual(batch.commandIds, ["flat", "empty", "drawn"]);
  assert.equal(batch.vertices.length, 6 * SEMANTIC_FLOATS_PER_VERTEX);
  assert.deepEqual(Array.from(batch.commandSlots), new Array(6).fill(2));
});

test("ellipse strokes test each sample against the ellipse inscribed in the inset bounds", () => {
  const shape = executorSource.slice(executorSource.indexOf("const SHAPE_FRAGMENT_SHADER"), executorSource.indexOf("const TEXTURE_FRAGMENT_SHADER"));
  // The stroke is the band between the shape and the shape inset by the
  // stroke width; both are tested at each sample in local pixels.
  assert.match(shape, /float left = v_bounds\.x \+ inset;/);
  assert.match(shape, /float right = v_bounds\.x \+ v_bounds\.z - inset;/);
  assert.match(shape, /float nx = \(point\.x - \(left \+ right\) \* 0\.5\) \/ rx;/);
  assert.match(shape, /return nx \* nx \+ ny \* ny <= 1\.0;/);
  assert.match(shape, /bool useStroke = v_params\.w > 0\.0 && !shapeContains\(point, v_params\.w\);/);
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
  // The bounds' top-left corner lands at (1028.56, 290.016) and maps back.
  const [x, y] = transformPoint(inverseOf(batch), 1028.56005859375, 290.0159912109375);
  // Single precision at canvas coordinates near 1000.
  assert.ok(Math.abs(x + 388) < 1e-3 && Math.abs(y + 131.8) < 1e-3, `${x}, ${y}`);
  assert.deepEqual(attribute(batch, 0, "bounds"), [-388, -131.8, 175, 60.8].map(Math.fround));
  assert.deepEqual(
    [...attribute(batch, 0, "clip01"), ...attribute(batch, 0, "clip23")],
    [944.52001953125, 251.55999755859375, 1795.47998046875, 251.55999755859375, 1795.47998046875, 657.6799926757812, 944.52001953125, 657.6799926757812],
  );
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

test("an asset mask takes its face colour from its fill, not from a gradient", () => {
  const layer = "shape";
  const plan = new SemanticCommandPlan({
    layerTableRevision: 1,
    layerTable: [{ layer_id: layer, parent_id: null, slot: 0, subtree_start: 0, subtree_end: 1 }],
    layerSources: [{ layer_id: layer, matrix: [1, 0, 0, 1, 0, 0] }],
    layerCommands: [{ layer_id: layer, render_mask: true, transform: { dx: 0, dy: 0 }, command_start: 0, command_count: 1 }],
    semanticCommands: [{
      id: "heart",
      layer_id: layer,
      role: "shape",
      bounds: { x: 0, y: 0, width: 64, height: 64 },
      matrix: [1, 0, 0, 1, 0, 0],
      payload: {
        kind: "shape",
        primitive: { asset_mask: { resource: { namespace: "assets", key: "custom_profile/shape/heart" } } },
        fill: [0.25, 0.5, 0.75, 0.8],
        gradient: { start: [0, 0.5], end: [1, 0.5], start_color: [1, 0, 0, 1], end_color: [0, 0, 1, 1] },
        stroke: [0, 0, 0, 1],
        stroke_width: 0.3,
      },
    }],
  });
  const [batch] = compileSemanticDrawBatches(plan.operations());
  assert.equal(batch.kind, "mask");
  const fill = [0.25, 0.5, 0.75, 0.8].map(Math.fround);
  assert.deepEqual(attribute(batch, 0, "fill"), fill);
  assert.deepEqual(attribute(batch, 0, "gradient"), [0, 0, 0, 0]);
  assert.deepEqual(attribute(batch, 0, "gradientEndColor"), fill);
});

test("a clip whose corners form an axis-aligned rectangle is reduced to its bounds", async () => {
  const { axisAlignedClipBounds, CLIP_AXIS_TOLERANCE, commandClipQuad, NO_CLIP } = await import("../../src/gpu/semanticCommandGeometry.ts");
  const f = Math.fround;
  assert.equal(CLIP_AXIS_TOLERANCE, f(0.0001));
  const identity = [1, 0, 0, 1, 0, 0];
  // Sides within the tolerance of an axis count as on it; the bounds take
  // the outermost corners, clockwise from the top-left one.
  const nearly = [[10, 20], [30, 20.00005], [30.00005, 50], [10, 50]];
  assert.deepEqual(commandClipQuad(nearly, identity, "clip"), [[10, 20], [f(30.00005), 20], [f(30.00005), 50], [10, 50]]);
  const anticlockwise = [[10, 50], [30, 50], [30, 20], [10, 20]];
  assert.deepEqual(commandClipQuad(anticlockwise, identity, "clip"), [[10, 20], [30, 20], [30, 50], [10, 50]]);
  // A turned clip keeps its corners, through the layer matrix in single
  // precision.
  const turned = [[0, 0], [10, 1], [9, 11], [-1, 10]];
  assert.equal(axisAlignedClipBounds(turned), null);
  assert.deepEqual(commandClipQuad(turned, [2, 0, 0, 2, 5, 5], "clip"), [[5, 5], [25, 7], [23, 27], [3, 25]]);
  assert.equal(commandClipQuad(null, identity, "clip"), NO_CLIP);
  assert.throws(() => commandClipQuad([[0, 0], [1, 0]], identity, "clip"), /invalid command clip clip/);
});

test("fragment stages keep [min, max) of a clip's bounds on both axes", async () => {
  const clipSource = await readFile(new URL("../../src/gpu/commandClipShader.ts", import.meta.url), "utf8");
  assert.match(clipSource, /float turn = cross2\(side, point - start\);/);
  assert.match(clipSource, /return turn > 0\.0 \|\| \(turn == 0\.0 && \(side\.y < 0\.0 \|\| \(side\.y == 0\.0 && side\.x > 0\.0\)\)\);/);
  assert.match(clipSource, /float winding = cross2\(p\[1\] - p\[0\], p\[3\] - p\[0\]\) < 0\.0 \? -1\.0 : 1\.0;/);
  assert.match(executorSource, /\$\{COMMAND_CLIP_GLSL\}/);
  // The same rule in plain arithmetic: a point on the top or left edge is
  // kept, one on the bottom or right edge is not.
  const keeps = (corners, [x, y]) => {
    const cross = (a, b) => a[0] * b[1] - a[1] * b[0];
    const winding = cross([corners[1][0] - corners[0][0], corners[1][1] - corners[0][1]], [corners[3][0] - corners[0][0], corners[3][1] - corners[0][1]]) < 0 ? -1 : 1;
    return corners.every((start, index) => {
      const end = corners[(index + 1) % 4];
      const side = [(end[0] - start[0]) * winding, (end[1] - start[1]) * winding];
      const turn = cross(side, [x - start[0], y - start[1]]);
      return turn > 0 || (turn === 0 && (side[1] < 0 || (side[1] === 0 && side[0] > 0)));
    });
  };
  const rect = [[10, 20], [14, 20], [14, 23], [10, 23]];
  for (const corners of [rect, [...rect].reverse()]) {
    assert.ok(keeps(corners, [10, 20]));
    assert.ok(keeps(corners, [13.5, 22.5]));
    assert.ok(!keeps(corners, [14, 21]));
    assert.ok(!keeps(corners, [12, 23]));
    assert.ok(!keeps(corners, [9.5, 21]));
  }
});
