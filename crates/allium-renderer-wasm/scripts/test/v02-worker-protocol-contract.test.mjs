import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const protocolUrl = new URL("../../src/protocol.ts", import.meta.url);
const rendererUrl = new URL("../../src/renderer.ts", import.meta.url);

test("worker protocol version 2 owns glyph, layout, scene, and interaction requests", async () => {
  const source = await readFile(protocolUrl, "utf8");
  assert.match(source, /RENDERER_WORKER_PROTOCOL\s*=\s*"allium\.renderer-worker\/2"/);
  for (const kind of [
    "contract",
    "registerFont",
    "mapGlyphs",
    "buildGlyphs",
    "layoutText",
    "createScene",
    "advance",
    "setLayerMask",
    "setLayerMasks",
    "setTab",
    "scroll",
    "dumpScene",
    "destroyScene",
    "createAuthoringBlank",
    "importAuthoringProfile",
    "restoreAuthoringCheckpoint",
    "applyAuthoring",
    "selectAuthoring",
    "beginAuthoringGesture",
    "previewAuthoringGesture",
    "commitAuthoringGesture",
    "cancelAuthoringGesture",
    "appendAuthoringPage",
    "duplicateAuthoringPage",
    "deleteAuthoringPage",
    "moveAuthoringPage",
    "undoAuthoring",
    "redoAuthoring",
    "exportAuthoring",
    "checkpointAuthoring",
    "destroyAuthoring",
    "stats",
  ]) {
    assert.match(source, new RegExp(`kind: "${kind}"`), kind);
  }
});

test("worker telemetry exposes bounded counters instead of profile content", async () => {
  const source = await readFile(protocolUrl, "utf8");
  for (const counter of ["scenes", "authoringSessions", "fonts", "requests", "failures", "wasmMs", "bridgeBytes"]) {
    assert.match(source, new RegExp(`${counter}: number`), counter);
  }
  for (const forbidden of ["rawText", "userId", "fontPath", "profileSeq", "documentDigest"]) {
    assert.doesNotMatch(source, new RegExp(`${forbidden}:`), forbidden);
  }
});

test("PNG export snapshots the freshly drawn canonical canvas without retaining WebGL frames", async () => {
  const source = await readFile(rendererUrl, "utf8");
  assert.match(source, /async exportPng\(\): Promise<Blob>/);
  assert.match(source, /this\.draw\(\)/);
  assert.match(source, /snapshot\.width = CARD_WIDTH/);
  assert.match(source, /snapshot\.height = CARD_HEIGHT/);
  assert.match(source, /snapshot\.toBlob/);
  assert.match(source, /snapshot\.convertToBlob\(\{ type: "image\/png" \}\)/);
  assert.doesNotMatch(source, /preserveDrawingBuffer:\s*true/);
});

test("BrowserScene animation capability comes from compiled dynamic programs", async () => {
  const source = await readFile(rendererUrl, "utf8");
  assert.match(source, /dynamicProgramCount: compiled\.layout\.dynamicPrograms\.length/);
  assert.match(source, /this\.animated = this\.dynamicProgramCount > 0/);
  assert.match(source, /readonly animated: boolean/);
  assert.match(source, /readonly dynamicProgramCount: number/);
  assert.doesNotMatch(source, /dump\(\)[\s\S]*line_indent[\s\S]*animated/);
});
