import assert from "node:assert/strict";
import { once } from "node:events";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { createDemoServer } from "../serve-demo.mjs";

const {
  createEmptySekaiResourceProvider,
  defaultAssetBase,
  emptySekaiResourceUrl,
} = await import("../../demo/workbench/emptySekaiResourceProvider.js");

const config = {
  region: "cn",
  assetBase: "https://assets.example.test/cn",
  staticBase: "https://static.example.test/renderer-static",
};

test("demo provider maps semantic descriptors to the configured bases", () => {
  const cases = [
    [{ namespace: "static", key: "chara_avatar/chara01_02" }, "https://static.example.test/renderer-static/chara_avatar/chara01_02.png"],
    [{ namespace: "assets", key: "character/member_small/example/card_normal" }, "https://assets.example.test/cn/character/member_small/example/card_normal.png"],
    [{ namespace: "assets", key: "bonds_honor/chr_sd_07_01" }, "https://assets.example.test/cn/bonds_honor/character/chr_sd_07_01/chr_sd_07_01.png"],
    [{ namespace: "assets", key: "bonds_honor/word/example_01" }, "https://assets.example.test/cn/bonds_honor/word/example_01/example_01.png"],
  ];
  for (const [resource, expected] of cases) {
    assert.equal(emptySekaiResourceUrl(resource, config), expected);
  }
});

test("demo provider defaults to the region's asset tree and the shared static images", () => {
  for (const region of ["jp", "cn", "tw", "kr", "en"]) {
    assert.equal(defaultAssetBase(region), `https://storage.pjsk.moe/sekai-${region}-assets`);
    assert.equal(
      emptySekaiResourceUrl({ namespace: "assets", key: "honor/honor_0001/degree_main" }, { region }),
      `https://storage.pjsk.moe/sekai-${region}-assets/honor/honor_0001/degree_main.png`,
    );
  }
  assert.equal(defaultAssetBase("XX"), "https://storage.pjsk.moe/sekai-cn-assets");
  assert.equal(
    emptySekaiResourceUrl({ namespace: "static", key: "chara_avatar/chara01_02" }, { region: "jp" }),
    "https://profile-assets.emptysekai.dev/game-profile/cn/6.0.0/renderer-static/chara_avatar/chara01_02.png",
  );
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
      url: "https://assets.example.test/cn/card/example.png",
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
  assert.match(session, /dist\/sekai_profile_wasm\.js\?v=20260715-13/);
  assert.match(session, /dist\/sekai_profile_wasm\.wasm\?v=20260715-13/);
  assert.match(session, /fontProvider:\s*\{/);
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
      "/dist/sekai_profile_wasm.js",
      "/dist/sekai_profile_wasm.wasm",
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
