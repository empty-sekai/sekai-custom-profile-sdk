import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { SemanticCommandPlan } from "../../src/gpu/semanticCommandPlanner.ts";
import { compileSemanticDrawBatches, SEMANTIC_FLOATS_PER_VERTEX } from "../../src/gpu/semanticCommandGeometry.ts";

const NORMAL_MAP = { namespace: "static", key: "ui/sekai_badge_normal" };
const LIT_BADGE = { kind: "lit_badge", normal_map: NORMAL_MAP };

function imageCommand(id, key, material, matrix = [1, 0, 0, 1, 0, 0]) {
  const payload = { kind: "image", resource: { namespace: "assets", key }, alpha_mask: null, uv: { x: 0, y: 0, width: 1, height: 1 }, tint: [1, 1, 1, 1] };
  if (material !== undefined) payload.material = material;
  return { id, layer_id: "collections", role: "collection", bounds: { x: -52, y: -52, width: 104, height: 104 }, matrix, payload };
}

function plan(commands, layerMatrix = [1, 0, 0, 1, 915, 406]) {
  return new SemanticCommandPlan({
    layerTableRevision: 1,
    layerTable: [{ layer_id: "collections", parent_id: null, slot: 0, subtree_start: 0, subtree_end: 1 }],
    layerSources: [{ layer_id: "collections", matrix: layerMatrix }],
    layerCommands: [{ layer_id: "collections", render_mask: true, transform: { dx: 0, dy: 0 }, command_start: 0, command_count: commands.length }],
    semanticCommands: commands,
  });
}

test("lit badge images batch apart from plain images and request their normal map", () => {
  const semantic = plan([
    imageCommand("plain", "collection/a"),
    imageCommand("plain-explicit", "collection/a", { kind: "plain" }),
    imageCommand("badge", "collection/a", LIT_BADGE),
    imageCommand("second-badge", "collection/b", LIT_BADGE),
  ]);
  const batches = compileSemanticDrawBatches(semantic.operations());
  assert.deepEqual(batches.map((batch) => batch.kind), ["image", "badge", "badge"]);
  assert.deepEqual(batches.map((batch) => batch.commandIds), [["plain", "plain-explicit"], ["badge"], ["second-badge"]]);
  assert.deepEqual(batches.map((batch) => batch.normalMapResource), [null, NORMAL_MAP, NORMAL_MAP]);
  assert.deepEqual(batches[1].resource, { namespace: "assets", key: "collection/a" });
  assert.deepEqual(semantic.resourceRequests(), [
    { namespace: "assets", key: "collection/a" },
    NORMAL_MAP,
    { namespace: "assets", key: "collection/b" },
  ]);
});

test("every vertex carries the canvas direction of its command's local x axis", () => {
  const radians = Math.PI / 6;
  const [cos, sin] = [Math.cos(radians), Math.sin(radians)];
  // Turned 30 degrees counter-clockwise on screen, then drawn at twice its size.
  const semantic = plan([imageCommand("badge", "collection/a", LIT_BADGE, [2, 0, 0, 2, 0, 0])], [cos, -sin, sin, cos, 915, 406]);
  const [batch] = compileSemanticDrawBatches(semantic.operations());
  assert.equal(batch.vertices.length, 6 * SEMANTIC_FLOATS_PER_VERTEX);
  for (let vertex = 0; vertex < 6; vertex += 1) {
    const base = vertex * SEMANTIC_FLOATS_PER_VERTEX;
    const axis = Array.from(batch.vertices.slice(base + 28, base + 30));
    assert.ok(Math.abs(axis[0] - 2 * cos) < 1e-6 && Math.abs(axis[1] + 2 * sin) < 1e-6, `vertex ${vertex}: ${axis}`);
  }
});

test("unknown image materials and badges without a normal map are rejected", () => {
  assert.throws(
    () => compileSemanticDrawBatches(plan([imageCommand("glow", "collection/a", { kind: "glow" })]).operations()),
    /unsupported image material glow: glow/,
  );
  assert.throws(
    () => compileSemanticDrawBatches(plan([imageCommand("bare", "collection/a", { kind: "lit_badge" })]).operations()),
    /missing command resource bare/,
  );
});

test("the badge program lights the image through a bilinear normal map in the shared frame", async () => {
  const source = await readFile(new URL("../../src/gpu/webglSemanticCommandExecutor.ts", import.meta.url), "utf8");
  const badgeShader = source.slice(source.indexOf("const BADGE_FRAGMENT_SHADER"), source.indexOf("const COMPOSITE_VERTEX_SHADER"));
  assert.match(source, /programs\.push\(createProgram\(gl, VERTEX_SHADER, BADGE_FRAGMENT_SHADER\)\);/);
  assert.match(source, /batch\.source\.kind === "badge" \? this\.badgeProgram : this\.textureProgram/);
  // The image keeps the nearest sampling of every image; the normal map is
  // filtered bilinearly on its own unit.
  assert.match(source, /this\.bindTexture\(program, "u_image", 2, batch\.source\.resource, gl\.NEAREST, batch\.source\.kind\);/);
  assert.match(source, /this\.bindTexture\(program, "u_normalMap", NORMAL_MAP_TEXTURE_UNIT, batch\.source\.normalMapResource, gl\.LINEAR, batch\.source\.kind\);/);
  const units = [...source.matchAll(/^const [A-Z_]+_TEXTURE_UNIT = (\d+);$/gm)].map((match) => Number(match[1]));
  assert.deepEqual(units, [5, 6, 7]);
  // Straight texels: no premultiplication on upload.
  assert.match(source, /gl\.pixelStorei\(gl\.UNPACK_PREMULTIPLY_ALPHA_WEBGL, 0\);/);
  // The per-vertex axis reaches the fragment stage turned to +y up.
  assert.match(source, /floatAttribute\(gl, 11, 2, stride, 28 \* 4\);/);
  assert.match(source, /layout\(location=11\) in vec2 a_axis;/);
  assert.match(source, /vec2 axis = vec2\(dot\(preview0\.xy, a_axis\), dot\(preview1\.xy, a_axis\)\);/);
  assert.match(badgeShader, /in vec2 v_tangent;/);
  assert.match(badgeShader, /uniform sampler2D u_normalMap;/);
  assert.match(badgeShader, /vec4 packedNormal = texture\(u_normalMap, v_uv\);/);
  // Premultiplied output with the element alpha, masked like any image.
  assert.match(badgeShader, /clamp\(v_fill\.a, 0\.0, 1\.0\)/);
  assert.match(badgeShader, /outColor = vec4\(color, alpha\);/);
  assert.match(badgeShader, /if \(u_hasAlphaMask == 1\) outColor \*= texture\(u_alphaMask, v_shapeUv\)\.a;/);
  assert.doesNotMatch(badgeShader, /u_maskMode/);
});
