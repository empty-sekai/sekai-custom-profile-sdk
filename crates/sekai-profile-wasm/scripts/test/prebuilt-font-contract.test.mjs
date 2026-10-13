import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const renderer = await readFile(new URL("../../src/renderer.ts", import.meta.url), "utf8");
const worker = await readFile(new URL("../../src/worker.ts", import.meta.url), "utf8");
const protocol = await readFile(new URL("../../src/protocol.ts", import.meta.url), "utf8");
const prebuiltAtlas = await readFile(new URL("../../src/prebuiltSdfAtlas.ts", import.meta.url), "utf8");
const atlasTexture = await readFile(new URL("../../src/gpu/webglSdfAtlasTexture.ts", import.meta.url), "utf8");

test("prebuilt manifests register font identity before profile preparation", () => {
  assert.match(renderer, /resolvePrebuiltFontContracts/);
  assert.match(renderer, /this\.worker\.registerPrebuiltFont\(contract\)/);
  assert.match(renderer, /if \(this\.providedFonts \|\| this\.prebuiltSdfAtlasProvider\)/);
  assert.match(protocol, /kind: "registerPrebuiltFont"/);
  assert.match(worker, /case "registerPrebuiltFont"/);
  assert.match(worker, /fontSources\.set\(request\.payload\.family/);
  assert.doesNotMatch(prebuiltAtlas, /sourcePages\.length > maxPages/);
  assert.match(atlasTexture, /gl\.texStorage3D\([^\n]+atlas\.depth\)/);
  assert.doesNotMatch(atlasTexture, /private readonly maxPages/);
});
