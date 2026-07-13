import assert from "node:assert/strict";
import test from "node:test";

import "./register-typescript.mjs";

const { BrowserSemanticResourceManager, defaultSemanticResourceUrl } = await import("../../src/gpu/browserSemanticResources.ts");

test("default provider separates renderer static assets from unpacked game assets", () => {
  assert.equal(
    defaultSemanticResourceUrl({ namespace: "static", key: "chara_avatar/chara01_02" }, "cn"),
    "https://cdn.emptysekai.com/renderer-static/v0.2/chara_avatar/chara01_02.png",
  );
  assert.equal(
    defaultSemanticResourceUrl({ namespace: "assets", key: "character/member_small/example/card_normal" }, "cn"),
    "https://cdn.emptysekai.com/assets/cn/character/member_small/example/card_normal.png",
  );
});

test("semantic resources singleflight, deduplicate requests, and retain decoded session values", async () => {
  let fetches = 0;
  let decodes = 0;
  const manager = new BrowserSemanticResourceManager({
    environmentId: "cn:manifest-a",
    resolveUrl: (resource) => `https://assets/${resource.key}`,
    fetchBlob: async () => { fetches += 1; return { size: 8 }; },
    decode: async () => { decodes += 1; return { width: 2, height: 3, id: decodes }; },
    softBytes: 1024,
    hardBytes: 2048,
  });
  const resource = { namespace: "assets", key: "card/a" };
  const first = await manager.acquire([resource, resource]);
  assert.equal(first.sources.size, 1);
  assert.equal(fetches, 1);
  assert.equal(decodes, 1);
  first.release();
  const warm = await manager.acquire([resource]);
  assert.equal(fetches, 1);
  assert.equal(decodes, 1);
  warm.release();
});

test("environment id participates in decoded resource identity", async () => {
  let calls = 0;
  const options = (environmentId) => ({
    environmentId,
    resolveUrl: () => "/asset.png",
    fetchBlob: async () => ({ size: 4 }),
    decode: async () => ({ width: 1, height: 1, id: ++calls }),
    softBytes: 1024,
    hardBytes: 2048,
  });
  const resource = { namespace: "assets", key: "same" };
  const cn = new BrowserSemanticResourceManager(options("cn:a"));
  const jp = new BrowserSemanticResourceManager(options("jp:a"));
  const a = await cn.acquire([resource]);
  const b = await jp.acquire([resource]);
  assert.notEqual(a.sources.values().next().value.id, b.sources.values().next().value.id);
  a.release();
  b.release();
});

test("only immutable renderer-static assets enter CacheStorage", async () => {
  const originalCaches = globalThis.caches;
  const originalFetch = globalThis.fetch;
  const opened = [];
  const deleted = [];
  const matched = [];
  const persisted = [];
  const fetched = [];
  globalThis.caches = {
    async delete(name) { deleted.push(name); return true; },
    async open(name) {
      opened.push(name);
      return {
        async match(url) { matched.push(String(url)); return undefined; },
        async put(url) { persisted.push(String(url)); },
      };
    },
  };
  globalThis.fetch = async (url, init) => {
    fetched.push({ url: String(url), cache: init?.cache });
    return new Response(new Blob(["asset"]), { status: 200 });
  };
  try {
    const manager = new BrowserSemanticResourceManager({
      environmentId: "cdn:cn",
      resolveUrl: (resource) => defaultSemanticResourceUrl(resource, "cn"),
      decode: async () => ({ width: 1, height: 1 }),
      softBytes: 1024,
      hardBytes: 2048,
    });
    const game = await manager.acquire([{ namespace: "assets", key: "card/example" }]);
    game.release();
    const rendererStatic = await manager.acquire([{ namespace: "static", key: "honor/icon_degreeLv" }]);
    rendererStatic.release();

    assert.deepEqual(opened, ["allium-renderer-static-assets-v2"]);
    assert.deepEqual(deleted, ["allium-renderer-semantic-assets-v1"]);
    assert.equal(matched.length, 1);
    assert.match(matched[0], /\/renderer-static\/v0\.2\//);
    assert.equal(persisted.length, 1);
    assert.match(persisted[0], /\/renderer-static\/v0\.2\//);
    assert.ok(!persisted.some((url) => url.includes("/assets/cn/")));
    assert.deepEqual(fetched.map((entry) => entry.cache), ["default", "default"]);
  } finally {
    if (originalCaches === undefined) delete globalThis.caches;
    else globalThis.caches = originalCaches;
    globalThis.fetch = originalFetch;
  }
});
