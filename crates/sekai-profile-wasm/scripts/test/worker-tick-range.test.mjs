import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { stripTypeScriptTypes } from "node:module";
import test from "node:test";
import vm from "node:vm";

const readSource = async (name) => stripTypeScriptTypes(
  await readFile(new URL(`../../src/${name}`, import.meta.url), "utf8"),
  { mode: "transform" },
);
const workerSource = (await readSource("worker.ts")).replace(
  /import\s*\{\s*RENDERER_WORKER_PROTOCOL\s*\}\s*from\s*["']\.\/protocol\.js["'];?/,
  "const RENDERER_WORKER_PROTOCOL = \"test\";",
);

function sceneWorker() {
  const heap = new Uint8Array(65536);
  let allocation = 64;
  const advances = [];
  const module = {
    HEAPU8: heap,
    HEAPU32: new Uint32Array(heap.buffer),
    _malloc(bytes) {
      const pointer = allocation;
      allocation = (allocation + bytes + 3) & ~3;
      return pointer;
    },
    _free() {},
    ccall(name, _result, _types, args) {
      if (name === "sdf_layout_freetype_free_string") return;
      let value;
      if (name === "sdf_renderer_core_profile_scene_create_json") {
        value = { handle: 7, snapshot: { schema_major: 1, scene_id: "scene" } };
      } else if (name === "sdf_renderer_core_scene_advance_json") {
        // The export takes the tick as a 32-bit WASM integer.
        advances.push(args[1] >>> 0);
        value = { patches: [], command_patches: [] };
      } else {
        throw new Error(`unexpected WASM call ${name}`);
      }
      const bytes = new TextEncoder().encode(`${JSON.stringify(value)}\0`);
      const pointer = this._malloc(bytes.length);
      heap.set(bytes, pointer);
      return pointer;
    },
  };
  const pending = new Map();
  let sequence = 0;
  const context = vm.createContext({
    console, performance, TextEncoder, TextDecoder, Uint8Array, ArrayBuffer, Error, atob, btoa,
    fixtureModule: module,
    postMessage(response) {
      const resolve = pending.get(response.id);
      pending.delete(response.id);
      resolve(response);
    },
  });
  vm.runInContext(`${workerSource}\nmoduleInstance = fixtureModule; modulePromise = Promise.resolve(fixtureModule);`, context);
  const send = (kind, payload) => new Promise((resolve) => {
    const id = ++sequence;
    pending.set(id, resolve);
    context.onmessage({ data: { id, kind, payload } });
  });
  return { send, advances };
}

test("ticks beyond the 32-bit timeline are rejected instead of wrapping", async () => {
  const fixture = sceneWorker();
  const created = await fixture.send("createScene", { request: {} });
  assert.equal(created.ok, true);
  const sceneId = created.result.sceneId;

  const last = await fixture.send("advance", { sceneId, tick: 0xFFFF_FFFF });
  assert.equal(last.ok, true);
  assert.deepEqual(fixture.advances, [0xFFFF_FFFF]);

  for (const tick of [2 ** 32, 2 ** 32 + 5, Number.MAX_SAFE_INTEGER]) {
    const response = await fixture.send("advance", { sceneId, tick });
    assert.equal(response.ok, false, `tick ${tick}`);
    assert.equal(response.error.code, "INVALID_TICK");
  }
  assert.deepEqual(fixture.advances, [0xFFFF_FFFF]);
});
