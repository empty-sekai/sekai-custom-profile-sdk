import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const source = (path) => readFile(new URL(`../../${path}`, import.meta.url), "utf8");

test("textless profile scenes skip glyph atlas creation and upload", async () => {
  const renderer = await source("src/renderer.ts");
  const scene = await source("src/gpu/semanticWebglSceneRenderer.ts");

  assert.match(renderer, /if \(glyphRequests\.length === 0\) \{[\s\S]*?atlas = null;[\s\S]*?\} else \{/);
  assert.match(renderer, /atlas \?\?= await buildSdfAtlas\(/);
  assert.match(renderer, /atlas\?\.release\(\)/);
  assert.match(scene, /input\.atlas\s*\? await this\.executor\.setSdfAtlas\(input\.atlas\)/);
});

test("every semantic draw starts from an opaque white canvas", async () => {
  const executor = await source("src/gpu/webglSemanticCommandExecutor.ts");

  assert.match(executor, /gl\.clearColor\(1(?:\.0)?, 1(?:\.0)?, 1(?:\.0)?, 1(?:\.0)?\)/);
  assert.match(executor, /gl\.clear\(gl\.COLOR_BUFFER_BIT\)/);
});
