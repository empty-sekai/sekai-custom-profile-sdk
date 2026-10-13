import assert from "node:assert/strict";
import test from "node:test";

import { packPreviewTransformsForTexture } from "../../src/gpu/previewTransformTextureLayout.ts";

test("preview transforms are packed by texture row for multiple layers", () => {
  const source = new Float32Array([
    1, 2, 3, 0, 4, 5, 6, 0,
    7, 8, 9, 0, 10, 11, 12, 0,
  ]);

  assert.deepEqual([...packPreviewTransformsForTexture(source, 2)], [
    1, 2, 3, 0,
    7, 8, 9, 0,
    4, 5, 6, 0,
    10, 11, 12, 0,
  ]);
});

test("preview transform packing rejects mismatched texture widths", () => {
  assert.throws(
    () => packPreviewTransformsForTexture(new Float32Array(8), 2),
    /invalid preview transform buffer/,
  );
});
