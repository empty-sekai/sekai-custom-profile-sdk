import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const executor = await readFile(
  new URL("../../src/gpu/webglSemanticCommandExecutor.ts", import.meta.url),
  "utf8",
);
const shapeShader = executor.match(/const SHAPE_FRAGMENT_SHADER = `([\s\S]*?)`;/)?.[1] ?? "";

test("the shape shader source is present", () => {
  assert.match(shapeShader, /#version 300 es/);
});

test("a shape stroke occupies the stroke width inside the shape bounds", () => {
  // The native rasterizer draws the stroke between the shape outline and the
  // same shape inset by the stroke width; nothing is drawn outside the bounds.
  assert.match(shapeShader, /float shapeDistance\(float inset\)/);
  assert.match(shapeShader, /vec2 halfSize = v_shapeSize \* 0\.5 - vec2\(inset\);/);
  assert.match(shapeShader, /float innerDistance = shapeDistance\(v_params\.w\);/);
  assert.match(shapeShader, /float strokeCoverage = max\(outerCoverage - innerCoverage, 0\.0\);/);
  assert.doesNotMatch(shapeShader, /strokeHalfWidth|abs\(distance\)/);
});

test("shape coverage is a one-pixel box filter around the outline", () => {
  // A pixel whose centre lies half a pixel inside a straight edge is fully
  // covered, as it is for the native 2x2 sampler.
  assert.match(shapeShader, /return clamp\(0\.5 - distance \/ max\(length\(vec2\(dFdx\(distance\), dFdy\(distance\)\)\), 0\.0005\), 0\.0, 1\.0\);/);
  assert.doesNotMatch(shapeShader, /smoothstep|fwidth/);
});

test("fill and stroke are mixed as premultiplied colours", () => {
  assert.match(shapeShader, /vec4 fill = clamp\(v_fill, 0\.0, 1\.0\);/);
  assert.match(shapeShader, /vec4 stroke = clamp\(v_stroke, 0\.0, 1\.0\);/);
  assert.match(
    shapeShader,
    /outColor = vec4\(fill\.rgb \* fill\.a, fill\.a\) \* innerCoverage \+ vec4\(stroke\.rgb \* stroke\.a, stroke\.a\) \* strokeCoverage;/,
  );
  assert.doesNotMatch(shapeShader, /mix\(v_fill, v_stroke/);
});

test("derivatives are taken before the per-fragment clip discard", () => {
  const main = shapeShader.slice(shapeShader.indexOf("void main()"));
  assert.ok(main.indexOf("coverage(outerDistance)") >= 0);
  assert.ok(main.indexOf("coverage(outerDistance)") < main.indexOf("if (!insideClip()) discard;"));
});
