import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const executor = await readFile(
  new URL("../../src/gpu/webglSemanticCommandExecutor.ts", import.meta.url),
  "utf8",
);
const shapeShader = executor.match(/const SHAPE_FRAGMENT_SHADER = `([\s\S]*?)`;/)?.[1] ?? "";
const vertexShader = executor.match(/const VERTEX_SHADER = `([\s\S]*?)`;/)?.[1] ?? "";
const main = shapeShader.slice(shapeShader.indexOf("void main()"));

test("the shape shader source is present", () => {
  assert.match(shapeShader, /#version 300 es/);
  assert.match(shapeShader, /\$\{FRAGMENT_COMMON\}/);
});

test("a shape stroke occupies the stroke width inside the shape bounds", () => {
  // The native rasterizer draws the stroke between the shape outline and the
  // same shape inset by the stroke width; nothing is drawn outside the bounds.
  assert.match(shapeShader, /bool shapeContains\(vec2 point, float inset\)/);
  assert.match(shapeShader, /float bottom = v_bounds\.y \+ v_bounds\.w - inset;/);
  assert.match(main, /if \(!shapeContains\(point, 0\.0\)\) continue;/);
  assert.match(main, /bool useStroke = v_params\.w > 0\.0 && !shapeContains\(point, v_params\.w\);/);
  assert.match(main, /vec4 color = useStroke \? v_stroke : shapeFill\(point\);/);
  assert.doesNotMatch(shapeShader, /strokeHalfWidth|abs\(distance\)/);
});

test("shape coverage counts the covered samples of a two-by-two grid in each pixel", () => {
  // Coverage is the share of four fixed samples inside the half-open shape,
  // as the native rasterizer counts it; a pixel whose centre lies outside the
  // shape still takes the samples that fall inside.
  const samples = [...main.matchAll(/samples\[(\d)\] = vec2\(([\d.]+), ([\d.]+)\);/g)].map(([, index, x, y]) => [Number(index), Number(x), Number(y)]);
  assert.deepEqual(samples, [[0, 0.25, 0.25], [1, 0.75, 0.25], [2, 0.25, 0.75], [3, 0.75, 0.75]]);
  assert.match(main, /vec2 point = toLocal\(pixel \+ samples\[index\]\);/);
  assert.match(main, /accumulated \+= vec4\(clamp\(color\.rgb, 0\.0, 1\.0\) \* alpha, alpha\) \* 0\.25;/);
  assert.match(main, /if \(!covered\) discard;/);
  assert.match(shapeShader, /point\.x < left \|\| point\.x >= right \|\| point\.y < top \|\| point\.y >= bottom\) return false;/);
  assert.doesNotMatch(shapeShader, /smoothstep|fwidth|dFdx|dFdy/);
});

test("shape quads reach pixels whose samples fall inside a shape edge", () => {
  // A pixel centre up to a quarter pixel diagonal outside the edge has a
  // covered sample, so the vertex stage grows every quad by half a pixel.
  assert.match(vertexShader, /const float QUAD_OUTSET = 0\.5;/);
  assert.match(vertexShader, /vec2 local = a_bounds\.xy \+ a_corner \* a_bounds\.zw \+ \(a_corner \* 2\.0 - 1\.0\) \* outset;/);
  // A left edge at x = 9.7: pixel 9 has its centre 0.2 px outside and its
  // right-hand samples 0.05 px inside, so it is half covered.
  const edge = Math.fround(9.7);
  const covered = [0.25, 0.75, 0.25, 0.75].filter((offset) => 9 + offset >= edge).length;
  assert.equal(covered, 2);
  assert.ok(edge - 0.5 <= 9 + 0.5, "the grown quad reaches the pixel centre");
});

test("fill and stroke are mixed as premultiplied colours", () => {
  assert.match(main, /float alpha = clamp\(color\.a, 0\.0, 1\.0\);/);
  assert.match(main, /outColor = accumulated;/);
  assert.doesNotMatch(shapeShader, /mix\(v_fill, v_stroke/);
});

test("the per-fragment clip tests the pixel centre before any sample", () => {
  assert.ok(main.indexOf("if (!insideClip(pixel + vec2(0.5))) discard;") >= 0);
  assert.ok(main.indexOf("if (!insideClip(pixel + vec2(0.5))) discard;") < main.indexOf("for (int index = 0;"));
});
