import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { stripTypeScriptTypes } from "node:module";
import test from "node:test";
import vm from "node:vm";

const readSource = async (name) => stripTypeScriptTypes(
  await readFile(new URL(`../../src/${name}`, import.meta.url), "utf8"),
  { mode: "transform" },
);
const dataUrl = (source) => `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`;
const protocolSource = await readSource("protocol.ts");
const protocolUrl = dataUrl(protocolSource);
const { RENDERER_WORKER_PROTOCOL } = await import(protocolUrl);
const clientSource = (await readSource("worker-client.ts"))
  .replace(/from\s+["']\.\/protocol\.js["']/, `from ${JSON.stringify(protocolUrl)}`);
const { RendererWorkerClient } = await import(dataUrl(clientSource));

class FakeWorker {
  constructor(init = "ok") {
    this.init = init;
    this.terminations = 0;
    this.hold = false;
  }

  postMessage(request) {
    structuredClone(request);
    if (this.hold) return;
    const response = request.kind === "init"
      ? this.init === "failure"
        ? { id: request.id, ok: false, error: { code: "INIT_FAILED", message: "module initialization failed" } }
        : { id: request.id, ok: true, result: { kind: "init", protocol: this.init === "mismatch" ? "unsupported" : RENDERER_WORKER_PROTOCOL } }
      : { id: request.id, ok: true, result: { kind: "stats", stats: { initialized: true } } };
    queueMicrotask(() => this.onmessage?.({ data: response }));
  }

  terminate() { this.terminations += 1; }
}

function createClient(worker) {
  return RendererWorkerClient.create({
    workerUrl: "https://example.invalid/worker.js",
    moduleUrl: "https://example.invalid/module.js",
    wasmUrl: "https://example.invalid/module.wasm",
    workerFactory: () => worker,
  });
}

for (const [mode, code] of [["failure", "INIT_FAILED"], ["mismatch", "PROTOCOL_MISMATCH"]]) {
  test(`worker initialization ${mode} terminates the unreturned worker`, async () => {
    const worker = new FakeWorker(mode);
    await assert.rejects(createClient(worker), { code });
    assert.equal(worker.terminations, 1);
    assert.equal(worker.onmessage, null);
    assert.equal(worker.onerror, null);
  });
}

async function bounded(promise) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error("worker request remained pending")), 250);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

for (const [event, code] of [["onerror", "WORKER_CRASHED"], ["onmessageerror", "WORKER_MESSAGE_ERROR"]]) {
  test(`${event} rejects in-flight work and closes the client to new work`, async () => {
    const worker = new FakeWorker();
    const client = await createClient(worker);
    worker.hold = true;
    const first = assert.rejects(client.stats(), { code });
    const second = assert.rejects(client.stats(), { code });
    worker[event]({ message: "worker stopped" });
    await Promise.all([first, second]);
    await assert.rejects(bounded(client.stats()), { code: "WORKER_TERMINATED" });
    assert.equal(worker.terminations, 1);
    client.terminate();
    assert.equal(worker.terminations, 1);
  });
}

test("a synchronous clone failure removes only its pending request", async () => {
  const worker = new FakeWorker();
  const client = await createClient(worker);
  await assert.rejects(client.sceneRequest("createScene", { request: { callback() {} } }), { name: "DataCloneError" });
  assert.equal(client.pending.size, 0);
  assert.deepEqual(await client.stats(), { initialized: true });
  assert.equal(worker.terminations, 0);
  client.terminate();
});

const workerSource = (await readSource("worker.ts")).replace(
  /import\s*\{\s*RENDERER_WORKER_PROTOCOL\s*\}\s*from\s*["']\.\/protocol\.js["'];?/,
  `const RENDERER_WORKER_PROTOCOL = ${JSON.stringify(RENDERER_WORKER_PROTOCOL)};`,
);

function atlasWorker(options = {}) {
  const heap = new Uint8Array(65536);
  let allocation = 64;
  let resolves = 0;
  const pinned = new Set();
  const releases = [];
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
      if (name === "sdf_atlas_release") {
        releases.push(args[1]);
        if (options.failFirstRelease && releases.length === 1) throw new Error("release failed");
        return pinned.delete(args[1]) ? 1 : 0;
      }
      let value;
      if (name === "sdf_atlas_create_json") {
        value = { handle: 1, stats: {} };
      } else if (name === "sdf_atlas_resolve_json") {
        resolves += 1;
        if (resolves === 2 && options.failColdResolve) throw new Error("atlas budget exhausted");
        const lease = resolves === 1 ? 11 : 12;
        pinned.add(lease);
        value = { lease, placements: [], missingKeys: resolves === 1 || options.missingCold ? ["cold"] : [], stats: {} };
      } else if (name === "sdf_layout_freetype_build_glyph_json_edt") {
        if (options.failRaster) throw new Error("glyph rasterization failed");
        value = { glyphs: [{ ch: "A", width: 1, height: 1, pixels_base64: "AQ==" }] };
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
  return { send, pinned, releases };
}

async function prepareAtlas(fixture) {
  const created = await fixture.send("createAtlas", {});
  assert.equal(created.ok, true);
  const font = { region: "jp", family: "test", sourceHash: "0".repeat(64) };
  await fixture.send("registerFont", { ...font, bytes: new ArrayBuffer(1) });
  return {
    atlasId: created.result.atlasId,
    keys: ["warm", "cold"],
    cached: [],
    generate: [{ ...font, chars: ["A"], glyphs: [{ key: "cold", char: "A", glyphIndex: 1 }] }],
  };
}

test("successful atlas resolution transfers both leases to the caller", async () => {
  const fixture = atlasWorker();
  const payload = await prepareAtlas(fixture);
  const response = await fixture.send("resolveAtlas", payload);
  assert.equal(response.ok, true);
  assert.deepEqual(Array.from(response.result.result.leases), [11, 12]);
  assert.deepEqual([...fixture.pinned], [11, 12]);
  assert.deepEqual(fixture.releases, []);
  for (const lease of response.result.result.leases) {
    await fixture.send("releaseAtlas", { atlasId: payload.atlasId, lease });
  }
  assert.equal(fixture.pinned.size, 0);
});

for (const [name, options, released, message] of [
  ["raster failure", { failRaster: true }, [11], "glyph rasterization failed"],
  ["cold resolution failure", { failColdResolve: true }, [11], "atlas budget exhausted"],
  ["missing cold glyph", { missingCold: true }, [11, 12], "Atlas could not resolve 1 glyph resource(s)"],
]) {
  test(`atlas ${name} releases every acquired lease`, async () => {
    const fixture = atlasWorker(options);
    const payload = await prepareAtlas(fixture);
    const response = await fixture.send("resolveAtlas", payload);
    assert.equal(response.ok, false);
    assert.equal(response.error.message, message);
    assert.deepEqual(fixture.releases, released);
    assert.equal(fixture.pinned.size, 0);
  });
}

test("atlas rollback attempts later leases and preserves the resolution error", async () => {
  const fixture = atlasWorker({ missingCold: true, failFirstRelease: true });
  const payload = await prepareAtlas(fixture);
  const response = await fixture.send("resolveAtlas", payload);
  assert.equal(response.ok, false);
  assert.equal(response.error.code, "ATLAS_GLYPH_MISSING");
  assert.deepEqual(fixture.releases, [11, 12]);
  assert.deepEqual([...fixture.pinned], [11]);
});
