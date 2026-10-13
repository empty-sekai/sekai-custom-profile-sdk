import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../../src/renderer.ts", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText;

function loadRenderer() {
  class Resources {
    async acquire() { return { sources: new Map(), availability: new Map(), release() {} }; }
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
    Map, Set, Error, Promise,
    require(name) {
      assert.ok(Object.hasOwn(dependencies, name), `unexpected dependency ${name}`);
      return dependencies[name];
    },
  });
  vm.runInContext(compiled, context);
  return module.exports;
}

// An OffscreenCanvas is an EventTarget, not an HTMLCanvasElement; WebGL fires
// its context events at it all the same.
class FakeOffscreenCanvas extends EventTarget {
  constructor() {
    super();
    this.width = 1830;
    this.height = 812;
    this.contexts = 0;
  }

  getContext() {
    this.contexts += 1;
    return {};
  }
}

function createRenderer(canvas) {
  const { BrowserRenderer } = loadRenderer();
  const worker = { terminated: 0, terminate() { this.terminated += 1; } };
  return new BrowserRenderer({}, worker, "en", canvas, { provide: async () => null });
}

test("an OffscreenCanvas context loss is claimed for restoration and blocks new scenes", async () => {
  const canvas = new FakeOffscreenCanvas();
  const renderer = createRenderer(canvas);
  const lost = new Event("webglcontextlost", { cancelable: true });
  canvas.dispatchEvent(lost);
  assert.equal(lost.defaultPrevented, true);
  await assert.rejects(
    renderer.createProfileScene({ masterData: {}, documentKey: "scene", card: {} }),
    { code: "WEBGL_CONTEXT_LOST" },
  );
  renderer.destroy();
});

test("an OffscreenCanvas context restoration re-acquires the context", async () => {
  const canvas = new FakeOffscreenCanvas();
  const renderer = createRenderer(canvas);
  canvas.dispatchEvent(new Event("webglcontextlost", { cancelable: true }));
  canvas.dispatchEvent(new Event("webglcontextrestored"));
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(canvas.contexts, 1);
  const reached = new Error("scene creation reached master data");
  await assert.rejects(
    renderer.createProfileScene({
      masterData: { prepareProfile: async () => { throw reached; } },
      documentKey: "scene",
      card: {},
    }),
    reached,
  );
  renderer.destroy();
});

test("destroying the renderer detaches its context listeners", () => {
  const canvas = new FakeOffscreenCanvas();
  const renderer = createRenderer(canvas);
  renderer.destroy();
  const lost = new Event("webglcontextlost", { cancelable: true });
  canvas.dispatchEvent(lost);
  assert.equal(lost.defaultPrevented, false);
});
