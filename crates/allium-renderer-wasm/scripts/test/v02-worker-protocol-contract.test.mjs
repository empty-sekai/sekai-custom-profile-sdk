import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const protocolUrl = new URL("../../src/protocol.ts", import.meta.url);

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
    "stats",
  ]) {
    assert.match(source, new RegExp(`kind: "${kind}"`), kind);
  }
});

test("worker telemetry exposes bounded counters instead of profile content", async () => {
  const source = await readFile(protocolUrl, "utf8");
  for (const counter of ["scenes", "fonts", "requests", "failures", "wasmMs", "bridgeBytes"]) {
    assert.match(source, new RegExp(`${counter}: number`), counter);
  }
  for (const forbidden of ["rawText", "userId", "fontPath", "profileSeq", "documentDigest"]) {
    assert.doesNotMatch(source, new RegExp(`${forbidden}:`), forbidden);
  }
});
