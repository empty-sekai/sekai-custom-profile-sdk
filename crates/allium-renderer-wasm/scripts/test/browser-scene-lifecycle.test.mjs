import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../../src/renderer.ts", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText;

function harness({ prepare, create, upload, failRelease } = {}) {
  const calls = [];
  const failure = new Error("release failed");
  const dispose = (name) => {
    calls.push(name);
    if (failRelease === name) throw failure;
  };
  class Resources {
    async acquire() {
      calls.push("acquire");
      return { sources: new Map(), availability: new Map(), release: () => dispose("images") };
    }
    stats() { return {}; }
  }
  class Gpu {
    async setScene() {
      calls.push("upload");
      await upload?.();
      return {};
    }
    destroy() { dispose("gpu"); }
  }
  class Telemetry {
    markDestroyed() { calls.push("destroyed"); }
    snapshot() { return {}; }
  }
  const dependencies = {
    "./gpu/browserSemanticResources.js": { BrowserSemanticResourceManager: Resources },
    "./resourceProvider.js": { profileResourceDescriptors: () => [] },
    "./localizationProvider.js": {},
    "./fontProvider.js": {},
    "./gpu/semanticCommandPlanner.js": {
      semanticCommandPlanFromCoreSnapshot: () => ({ resourceRequests: () => [] }),
    },
    "./gpu/semanticWebglSceneRenderer.js": { SemanticWebglSceneRenderer: Gpu },
    "./fontSdfAtlas.js": { disposeWorkerAtlasSessions() {} },
    "./prebuiltSdfAtlas.js": {},
    "./worker-client.js": {},
    "./telemetry/rendererTelemetry.js": { RendererRuntimeTelemetry: Telemetry },
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
  const { BrowserRenderer, BrowserScene } = module.exports;
  const core = { initial: { snapshot: {} }, destroy: async () => dispose("core") };
  const renderer = new BrowserRenderer({}, { terminate() {} }, "en", {}, {});
  const masterData = {
    async prepareProfile() {
      calls.push("prepare");
      await prepare?.();
      return { glyph_demand: { requests: [] }, layout_request: { layers: [] } };
    },
    async createProfileScene() {
      calls.push("create");
      await create?.();
      return { scene: core, layout: { dynamicPrograms: [] } };
    },
  };
  const options = { masterData, documentKey: "scene", card: {} };
  return { calls, failure, renderer, BrowserScene, options, core, Gpu, dispose };
}

function releases(calls) {
  return calls.filter((name) => ["gpu", "images", "atlas", "core"].includes(name));
}

test("failed GPU bootstrap releases its GPU, images and core without hiding the upload error", async () => {
  const error = new Error("upload failed");
  const h = harness({ upload: () => { throw error; }, failRelease: "images" });
  await assert.rejects(h.renderer.createProfileScene(h.options), (actual) => actual === error);
  assert.deepEqual(releases(h.calls), ["gpu", "images", "core"]);
  assert.equal(h.renderer.scenes.size, 0);
});

test("an already cancelled empty scene performs no preparation or allocation", async () => {
  const controller = new AbortController();
  controller.abort(new Error("cancelled"));
  const h = harness();
  await assert.rejects(
    h.renderer.createProfileScene({ ...h.options, signal: controller.signal }),
    (error) => error === controller.signal.reason,
  );
  assert.deepEqual(h.calls, []);
});

for (const phase of ["prepare", "create", "upload"]) {
  test(`cancellation during ${phase} cannot publish an empty scene`, async () => {
    const controller = new AbortController();
    const h = harness({ [phase]: () => controller.abort(new Error("cancelled")) });
    await assert.rejects(
      h.renderer.createProfileScene({ ...h.options, signal: controller.signal }),
      (error) => error === controller.signal.reason,
    );
    assert.equal(h.renderer.scenes.size, 0);
    const expected = phase === "prepare" ? ["images"]
      : phase === "create" ? ["images", "core"] : ["gpu", "images", "core"];
    assert.deepEqual(releases(h.calls), expected);
  });
}

test("renderer destruction during GPU bootstrap aborts creation and releases the unregistered scene", async () => {
  let h;
  h = harness({ upload: () => h.renderer.destroy() });
  await assert.rejects(h.renderer.createProfileScene(h.options), { code: "RENDERER_DESTROYED" });
  assert.deepEqual(releases(h.calls), ["gpu", "images", "core"]);
  assert.equal(h.renderer.scenes.size, 0);
});

for (const failRelease of ["gpu", "images", "atlas", "core", undefined]) {
  test(`scene destruction completes every cleanup step when ${failRelease ?? "no step"} fails`, async () => {
    const h = harness({ failRelease });
    const scene = new h.BrowserScene(
      h.core,
      new h.Gpu(),
      { release: () => h.dispose("images") },
      null,
      { dynamicProgramCount: 0, canvas: {}, bootstrap: {}, onDestroy: () => h.calls.push("unregistered") },
    );
    scene.atlas = { release: async () => h.dispose("atlas") };
    if (failRelease) {
      await assert.rejects(scene.destroy(), (error) => error === h.failure);
    } else {
      await scene.destroy();
    }
    assert.deepEqual(h.calls, ["gpu", "images", "atlas", "core", "destroyed", "unregistered"]);
    await scene.destroy();
    assert.equal(h.calls.length, 6);
  });
}
