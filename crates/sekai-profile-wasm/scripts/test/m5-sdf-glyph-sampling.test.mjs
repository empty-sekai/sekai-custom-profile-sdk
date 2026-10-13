import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import "./register-typescript.mjs";

const {
  buildSdfGlyphInstanceVertices,
  glyphTexelMap,
  SDF_GLYPH_FLOATS_PER_INSTANCE,
  SDF_GLYPH_INSTANCE_ATTRIBUTES: LAYOUT,
} = await import("../../src/gpu/webglSdfGlyphPipeline.ts");

const source = await readFile(new URL("../../src/gpu/webglSdfGlyphPipeline.ts", import.meta.url), "utf8");
const vertexShader = source.match(/const VERTEX_SHADER = `([\s\S]*?)`;/)?.[1] ?? "";
const fragmentShader = source.match(/const FRAGMENT_SHADER = `([\s\S]*?)`;/)?.[1] ?? "";

/** `[a, b, c, d, e, f]` applied to a point, in double precision. */
function apply(matrix, [x, y]) {
  const [a, b, c, d, e, f] = matrix;
  return [a * x + c * y + e, b * x + d * y + f];
}

function assertNear(actual, expected, tolerance, message) {
  actual.forEach((value, index) => assert.ok(Math.abs(value - expected[index]) <= tolerance, `${message}: ${actual} vs ${expected}`));
}

test("every glyph instance attribute has the shader location the layout names", () => {
  const declared = [...vertexShader.matchAll(/^layout\(location=(\d+)\) in (\w+) a_(\w+);$/gm)]
    .map(([, location, type, name]) => [name, Number(location), type]);
  const expected = Object.entries(LAYOUT).map(([name, { location, size }]) => [name, location, `vec${size}`]);
  assert.deepEqual(declared.sort((a, b) => a[1] - b[1]), expected.sort((a, b) => a[1] - b[1]));
  const spans = Object.values(LAYOUT).map(({ offset, size }) => [offset, offset + size]).sort((a, b) => a[0] - b[0]);
  spans.forEach(([start], index) => assert.equal(start, index === 0 ? 0 : spans[index - 1][1]));
  assert.equal(spans.at(-1)[1], SDF_GLYPH_FLOATS_PER_INSTANCE);
});

test("the glyph texel matrix maps the quad onto its atlas rectangle edge to edge", () => {
  // Texel centres lie at whole coordinates, so the rectangle [10, 50) spans
  // texel coordinates 9.5 to 49.5.
  const quad = [[100.25, 50.5], [120.25, 50.5], [120.25, 80.5], [100.25, 80.5]];
  const matrix = glyphTexelMap(quad, [10, 20, 40, 60]);
  assert.ok(matrix.every((value) => value === Math.fround(value)), "single precision");
  assertNear(apply(matrix, quad[0]), [9.5, 19.5], 1e-4, "top-left");
  assertNear(apply(matrix, quad[2]), [49.5, 79.5], 1e-4, "bottom-right");
  // A turned quad keeps its corners on the rectangle's corners.
  const angle = Math.PI / 6;
  const turn = ([x, y]) => [Math.fround(900 + x * Math.cos(angle) - y * Math.sin(angle)), Math.fround(400 + x * Math.sin(angle) + y * Math.cos(angle))];
  const turned = [[0, 0], [20, 0], [20, 30], [0, 30]].map(turn);
  const turnedMatrix = glyphTexelMap(turned, [10, 20, 40, 60]);
  assertNear(apply(turnedMatrix, turned[1]), [49.5, 19.5], 1e-3, "top-right");
  assertNear(apply(turnedMatrix, turned[3]), [9.5, 79.5], 1e-3, "bottom-left");
  // A quad without area draws nothing.
  assert.equal(glyphTexelMap([[0, 0], [10, 0], [10, 0], [0, 0]], [0, 0, 4, 4]), null);
});

test("glyph instances carry their quad, texel matrix, rectangle and clip", () => {
  const instance = {
    layerId: "text",
    drawable: true,
    atlasPage: 2,
    quad: [[10, 20, 0.25, 0.5, 3], [30, 20, 0.75, 0.5, 3], [30, 60, 0.75, 1, 3], [10, 60, 0.25, 1, 3]],
    fill: [1, 0.5, 0.25, 1], outline: [0, 0, 0, 1],
    shaderFaceScale: 10, shaderFaceBias: 4.5, shaderUnderlayScale: 10, shaderUnderlayBias: 4.5,
    shaderVertexAlpha: 1,
  };
  const clip = [[0, 0], [100, 0], [100, 50], [0, 50]];
  const vertices = buildSdfGlyphInstanceVertices(
    [instance, { ...instance, drawable: false }],
    new Map([["text", 3]]),
    { width: 64, height: 32 },
    new Map([["text", clip]]),
    new Map([["text", 5]]),
  );
  assert.equal(vertices.length, SDF_GLYPH_FLOATS_PER_INSTANCE);
  const read = (name) => Array.from(vertices.slice(LAYOUT[name].offset, LAYOUT[name].offset + LAYOUT[name].size));
  assert.deepEqual([...read("quad01"), ...read("quad23")], [10, 20, 30, 20, 30, 60, 10, 60]);
  assert.deepEqual(read("rect"), [16, 16, 32, 16]);
  assert.deepEqual([...read("toTexel"), ...read("toTexelOffset")], glyphTexelMap(instance.quad.map(([x, y]) => [x, y]), [16, 16, 32, 16]).map(Math.fround));
  assert.deepEqual(read("instanceMeta"), [1, 3, 2, 5]);
  assert.deepEqual([...read("clip01"), ...read("clip23")], clip.flat());
});

test("the glyph fragment stage samples at the fragment position through texel fetches", () => {
  const inputs = [...fragmentShader.matchAll(/^(flat )?in \w+ \w+;$/gm)];
  assert.ok(inputs.length >= 12);
  assert.ok(inputs.every(([, flat]) => flat), "every input is per glyph");
  assert.match(fragmentShader, /vec2 window = floor\(gl_FragCoord\.xy\);/);
  assert.match(fragmentShader, /if \(!insideClip\(centre\) \|\| !quadCovers\(centre\)\) discard;/);
  assert.match(fragmentShader, /return texelFetch\(u_atlas, ivec3\(texel, v_atlasPage\), 0\)\.r;/);
  assert.doesNotMatch(fragmentShader, /texture\(u_atlas|v_uv|smoothstep|fwidth|dFdx/);
  // A pixel whose centre lies on the quad's right edge belongs to the pixel
  // span of the quad beyond it, as in the native scanline rule.
  assert.match(fragmentShader, /return crossings >= 2 && point\.x >= low && point\.x < high;/);
  assert.match(fragmentShader, /if \(point\.y < bottom \|\| point\.y >= top \|\| top == bottom\) continue;/);
});
