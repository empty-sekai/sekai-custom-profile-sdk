import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const source = await readFile(
  new URL("../../src/gpu/webglSemanticCommandExecutor.ts", import.meta.url),
  "utf8",
);
const textureShader = source.match(/const TEXTURE_FRAGMENT_SHADER = `([\s\S]*?)`;/)?.[1] ?? "";
const imageSampling = source.match(/const IMAGE_SAMPLING = `([\s\S]*?)`;/)?.[1] ?? "";

test("shape SDF coverage is clipped by the source sprite alpha domain", () => {
  assert.match(textureShader, /float faceCoverage\s*=\s*[^;]+\* sampleColor\.a;/);
  assert.match(textureShader, /float outerCoverage\s*=\s*[^;]+\* sampleColor\.a;/);
});

test("a shape mask reads the texel under the pixel centre of the drawn bounds", () => {
  // One sample per pixel at its centre, mapped into the shape through the
  // draw's inverse and drawn only inside the half-open bounds; the texel is
  // the nearest one below the sample, as the native nearest filter picks it.
  assert.match(textureShader, /vec2 centre = canvasPixel\(\) \+ vec2\(0\.5\);/);
  assert.match(textureShader, /vec2 local = toLocal\(centre\);/);
  assert.match(textureShader, /if \(u < 0\.0 \|\| u >= 1\.0 \|\| v < 0\.0 \|\| v >= 1\.0\) discard;/);
  assert.match(textureShader, /vec4 sampleColor = texelFetch\(u_image, nearestTexel\(imageUv, textureSize\(u_image, 0\)\), 0\);/);
  assert.match(imageSampling, /vec2 texel = floor\(coordinate \* vec2\(size\)\);/);
  assert.match(imageSampling, /return ivec2\(clamp\(texel, vec2\(0\.0\), vec2\(size - 1\)\)\);/);
  assert.doesNotMatch(textureShader, /texture\(|\bin vec2 v_uv\b/);
  // The sample reaches the mask in mask mode only.
  assert.match(source, /gl\.uniform1i\(gl\.getUniformLocation\(program, "u_maskMode"\), batch\.source\.kind === "mask" \? 1 : 0\);/);
});
