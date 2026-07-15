import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("shape SDF coverage is clipped by the source sprite alpha domain", async () => {
  const source = await readFile(
    new URL("../../src/gpu/webglSemanticCommandExecutor.ts", import.meta.url),
    "utf8",
  );
  assert.match(source, /float faceCoverage\s*=\s*[^;]+\* sampleColor\.a;/);
  assert.match(source, /float outerCoverage\s*=\s*[^;]+\* sampleColor\.a;/);
});
