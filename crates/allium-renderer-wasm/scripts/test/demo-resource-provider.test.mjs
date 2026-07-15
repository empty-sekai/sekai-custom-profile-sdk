import assert from "node:assert/strict";
import { once } from "node:events";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { createDemoServer } from "../serve-demo.mjs";

const {
  createEmptySekaiResourceProvider,
  emptySekaiResourceUrl,
} = await import("../../demo/workbench/emptySekaiResourceProvider.js");

const config = {
  region: "cn",
  assetBase: "https://cdn.emptysekai.com/assets/cn",
  staticBase: "https://cdn.emptysekai.com/renderer-static/v0.2",
};

test("demo provider maps EmptySekai resources without exposing rules from the package", () => {
  const cases = [
    [{ namespace: "static", key: "chara_avatar/chara01_02" }, "https://cdn.emptysekai.com/renderer-static/v0.2/chara_avatar/chara01_02.png"],
    [{ namespace: "assets", key: "character/member_small/example/card_normal" }, "https://cdn.emptysekai.com/assets/cn/character/member_small/example/card_normal.png"],
    [{ namespace: "assets", key: "bonds_honor/chr_sd_07_01" }, "https://cdn.emptysekai.com/assets/cn/bonds_honor/character/chr_sd_07_01/chr_sd_07_01.png"],
    [{ namespace: "assets", key: "bonds_honor/word/example_01" }, "https://cdn.emptysekai.com/assets/cn/bonds_honor/word/example_01/example_01.png"],
    [{ namespace: "assets", key: "ugc/editor_image/2026-07/example.png" }, "https://cdn.emptysekai.com/ugc/editor_image/2026-07/example.png"],
    [{ namespace: "assets", key: "uploads/avatar/2026-07/example.png" }, "https://cdn.emptysekai.com/uploads/avatar/2026-07/example.png"],
    [{ namespace: "assets", key: "presets/stamp/example.png" }, "https://cdn.emptysekai.com/presets/stamp/example.png"],
  ];
  for (const [resource, expected] of cases) {
    assert.equal(emptySekaiResourceUrl(resource, config), expected);
  }
});

test("demo provider performs asynchronous fetch with the runtime AbortSignal", async () => {
  const originalFetch = globalThis.fetch;
  const controller = new AbortController();
  let request = null;
  globalThis.fetch = async (url, init) => {
    request = { url: String(url), signal: init.signal, cache: init.cache };
    return new Response(new Blob(["asset"]), { status: 200 });
  };
  try {
    const provider = createEmptySekaiResourceProvider(config);
    const result = await provider.provide({
      id: "assets\0card/example",
      namespace: "assets",
      key: "card/example",
      role: "image",
      provenance: {},
    }, { signal: controller.signal });
    assert.ok(result.source instanceof Blob);
    assert.deepEqual(request, {
      url: "https://cdn.emptysekai.com/assets/cn/card/example.png",
      signal: controller.signal,
      cache: "default",
    });
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("demo entrypoints identify the current local build", async () => {
  const [html, demo, session, telemetry] = await Promise.all([
    readFile(new URL("../../demo/index.html", import.meta.url), "utf8"),
    readFile(new URL("../../demo/demo.js", import.meta.url), "utf8"),
    readFile(new URL("../../demo/workbench/session.js", import.meta.url), "utf8"),
    readFile(new URL("../../demo/workbench/telemetry.js", import.meta.url), "utf8"),
  ]);
  assert.match(html, /demo\.js\?v=20260715-13/);
  assert.match(demo, /session\.js\?v=20260715-13/);
  assert.match(session, /dist\/index\.js\?v=20260715-13/);
  assert.match(session, /dist\/worker\.js\?v=20260715-13/);
  assert.match(session, /dist\/allium_renderer_wasm\.js\?v=20260715-13/);
  assert.match(session, /dist\/allium_renderer_wasm\.wasm\?v=20260715-13/);
  assert.match(session, /fontProvider:\s*\{/);
  assert.doesNotMatch(session, /renderer\.registerFont\(/);
  for (const field of ["requested", "loaded", "failures", "cancellations", "encodedBytes", "resolveMs", "peak"]) {
    assert.match(telemetry, new RegExp(`resourceProvider\\.${field}`));
  }
});

test("demo server disables caching for every ESM and WASM graph node", async () => {
  const root = fileURLToPath(new URL("../..", import.meta.url));
  const server = createDemoServer(root);
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const { port } = server.address();
  try {
    for (const path of [
      "/demo/",
      "/demo/workbench/session.js",
      "/dist/index.js",
      "/dist/gpu/generalTextRenderPlacement.js",
      "/dist/worker.js",
      "/dist/allium_renderer_wasm.js",
      "/dist/allium_renderer_wasm.wasm",
    ]) {
      const response = await fetch(`http://127.0.0.1:${port}${path}`);
      assert.equal(response.status, 200, path);
      assert.equal(response.headers.get("cache-control"), "no-store, max-age=0", path);
      await response.arrayBuffer();
    }
  } finally {
    server.close();
    await once(server, "close");
  }
});
