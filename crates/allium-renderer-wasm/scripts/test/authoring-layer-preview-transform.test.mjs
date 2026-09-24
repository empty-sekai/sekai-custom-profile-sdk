import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const executor = await readFile(new URL("../../src/gpu/webglSemanticCommandExecutor.ts", import.meta.url), "utf8");
const glyphs = await readFile(new URL("../../src/gpu/webglSdfGlyphPipeline.ts", import.meta.url), "utf8");
const renderer = await readFile(new URL("../../src/renderer.ts", import.meta.url), "utf8");

test("authored gesture previews update a retained GPU matrix instead of rebuilding geometry", () => {
  assert.match(renderer, /setLayerPreviewTransform\(/);
  assert.match(executor, /texSubImage2D\([\s\S]*previewTransforms\.subarray/);
  assert.match(executor, /uniform sampler2D u_previewTransform/);
  // The vertex stage undoes the retained preview matrix when it maps canvas
  // pixels back into each draw.
  assert.match(executor, /float previewDeterminant = preview0\.x \* preview1\.y - preview1\.x \* preview0\.y;/);
  assert.match(executor, /vec4 previewInverse = vec4\(preview1\.y, -preview1\.x, -preview0\.y, preview0\.x\) \/ previewDeterminant;/);
  assert.match(executor, /vec2 undone = previewInverseOffset - totalOffset;/);
  assert.match(glyphs, /uniform sampler2D u_previewTransform/);
  assert.match(glyphs, /vec2 undone = previewInverseOffset - totalState;/);
  assert.match(glyphs, /return vec2\(dot\(preview0\.xy, point\) \+ preview0\.z, dot\(preview1\.xy, point\) \+ preview1\.z\);/);
  assert.doesNotMatch(executor.match(/setLayerPreviewTransform\([\s\S]*?\n  }/)?.[0] ?? "", /bufferData|setScene|compileSemanticDrawBatches/);
});

test("image alpha masks cannot replace the authored preview transform texture", () => {
  assert.match(executor, /const PREVIEW_TRANSFORM_TEXTURE_UNIT = 5/);
  assert.match(executor, /const ALPHA_MASK_TEXTURE_UNIT = 6/);
  assert.match(executor, /u_previewTransform"\), PREVIEW_TRANSFORM_TEXTURE_UNIT/);
  assert.match(executor, /u_alphaMask"\), ALPHA_MASK_TEXTURE_UNIT/);
  assert.notEqual(
    executor.match(/const PREVIEW_TRANSFORM_TEXTURE_UNIT = (\d+)/)?.[1],
    executor.match(/const ALPHA_MASK_TEXTURE_UNIT = (\d+)/)?.[1],
  );
});
