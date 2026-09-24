import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../../src/renderer.ts", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText;

const REQUIRED = ["cards", "customProfileTextColors"];
const OPTIONAL = [
  "customProfileCharacterIconResources",
  "customProfileMaterialResources",
  "customProfileUserInterfaceIconResources",
];

function browserRenderer() {
  class Resources {
    stats() { return {}; }
  }
  const dependencies = {
    "./gpu/browserSemanticResources.js": { BrowserSemanticResourceManager: Resources },
    "./resourceProvider.js": { profileResourceDescriptors: () => [] },
    "./localizationProvider.js": {},
    "./fontProvider.js": {},
    "./gpu/semanticCommandPlanner.js": {},
    "./gpu/semanticWebglSceneRenderer.js": {},
    "./fontSdfAtlas.js": { disposeWorkerAtlasSessions() {} },
    "./prebuiltSdfAtlas.js": {},
    "./worker-client.js": {},
    "./telemetry/rendererTelemetry.js": {},
  };
  const module = { exports: {} };
  const context = vm.createContext({
    module, exports: module.exports, console, performance, AbortController, DOMException,
    Map, Set, Error, ArrayBuffer, Uint8Array, TextEncoder, TextDecoder,
    require(name) {
      assert.ok(Object.hasOwn(dependencies, name), `unexpected dependency ${name}`);
      return dependencies[name];
    },
  });
  vm.runInContext(compiled, context);
  const calls = [];
  const session = {
    requiredTables: REQUIRED,
    optionalTables: OPTIONAL,
    async putTable(name) { calls.push(`put ${name}`); return {}; },
    async seal() { calls.push("seal"); return {}; },
    async destroy() { calls.push("destroy"); },
  };
  const worker = { async createMasterData() { return session; }, terminate() {} };
  const renderer = new module.exports.BrowserRenderer({}, worker, "cn", new EventTarget(), {});
  return { renderer, calls };
}

test("optional tables the loader does not supply are left out of the session", async () => {
  const { renderer, calls } = browserRenderer();
  const requests = [];
  await renderer.loadMasterData("revision", async ({ table, region, revision, optional }) => {
    requests.push([table, region, revision, optional]);
    if (table === OPTIONAL[0]) return null;
    if (table === OPTIONAL[1]) throw new Error("HTTP 404");
    return [];
  }, { concurrency: 1 });
  assert.deepEqual(requests, [
    ...REQUIRED.map((table) => [table, "cn", "revision", false]),
    ...OPTIONAL.map((table) => [table, "cn", "revision", true]),
  ]);
  assert.deepEqual(calls, [...REQUIRED.map((table) => `put ${table}`), `put ${OPTIONAL[2]}`, "seal"]);
});

test("a required table that fails to load still fails the load", async () => {
  const { renderer, calls } = browserRenderer();
  const failure = new Error("HTTP 503");
  await assert.rejects(
    renderer.loadMasterData("revision", async ({ table }) => {
      if (table === REQUIRED[1]) throw failure;
      return [];
    }, { concurrency: 1 }),
    (error) => error === failure,
  );
  assert.deepEqual(calls, [`put ${REQUIRED[0]}`, "destroy"]);
});

test("cancelling while an optional table loads rejects with the abort reason", async () => {
  const { renderer, calls } = browserRenderer();
  const controller = new AbortController();
  const reason = new Error("cancelled");
  await assert.rejects(
    renderer.loadMasterData("revision", async ({ optional }) => {
      if (!optional) return [];
      controller.abort(reason);
      throw reason;
    }, { concurrency: 1, signal: controller.signal }),
    (error) => error === reason,
  );
  assert.deepEqual(calls, [...REQUIRED.map((table) => `put ${table}`), "destroy"]);
});
