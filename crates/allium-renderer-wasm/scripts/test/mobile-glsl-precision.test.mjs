import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const shaderSources = await Promise.all([
  "../../src/gpu/webglSemanticCommandExecutor.ts",
  "../../src/gpu/webglSdfGlyphPipeline.ts",
].map((path) => readFile(new URL(path, import.meta.url), "utf8")));

test("GLSL vector arrays declare explicit precision for strict mobile drivers", () => {
  for (const source of shaderSources) {
    assert.doesNotMatch(source, /^\s+(?:vec[234]|mat[234])\s+\w+\s*\[/m);
    assert.doesNotMatch(source, /\b(?:vec[234]|mat[234])\s*\[\s*\d+\s*\]\s*\(/);
  }
  assert.equal(
    shaderSources.join("\n").match(/^\s+highp vec2\s+\w+\s*\[/gm)?.length,
    7,
  );
});
