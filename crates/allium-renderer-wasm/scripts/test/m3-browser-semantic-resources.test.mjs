import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import "./register-typescript.mjs";

const { BrowserSemanticResourceManager, resourceIdentity } = await import("../../src/gpu/browserSemanticResources.ts");
const { profileResourceDescriptors } = await import("../../src/resourceProvider.ts");

function descriptor(key, overrides = {}) {
  return {
    id: `assets\0${key}`,
    namespace: "assets",
    key,
    role: `image:${key}`,
    provenance: {},
    expectedSize: { width: 8, height: 6 },
    ...overrides,
  };
}

function directSource(id, width = 2, height = 3) {
  return { id, width, height };
}

function within(promise, milliseconds = 100) {
  return Promise.race([
    promise,
    new Promise((_, reject) => setTimeout(() => reject(new Error("timed out waiting for cancellation")), milliseconds)),
  ]);
}

test("resource identity is caller-independent and uses the stable descriptor id", () => {
  const resource = descriptor("card/a", { id: "stable:card:a" });
  assert.equal(resourceIdentity(resource), "stable:card:a");
});

test("provider may return a direct TexImageSource without URL or fetch semantics", async () => {
  const source = directSource("direct");
  const manager = new BrowserSemanticResourceManager({
    provider: { async provide() { return { source }; } },
    decode: async () => assert.fail("direct sources must not be decoded"),
    softBytes: 1024,
    hardBytes: 2048,
  });
  const result = await manager.acquire([descriptor("card/direct")]);
  assert.deepEqual(result.sources.get("assets\0card/direct"), { source, width: 2, height: 3 });
  result.release();
});

test("provider may return encoded bytes which the runtime decodes", async () => {
  let decoded = null;
  const manager = new BrowserSemanticResourceManager({
    provider: { async provide() { return { source: new Uint8Array([1, 2, 3]) }; } },
    decode: async (blob) => {
      decoded = new Uint8Array(await blob.arrayBuffer());
      return directSource("decoded", 4, 5);
    },
    softBytes: 1024,
    hardBytes: 2048,
  });
  const result = await manager.acquire([descriptor("card/bytes")]);
  assert.deepEqual([...decoded], [1, 2, 3]);
  assert.equal(result.sources.get("assets\0card/bytes").source.id, "decoded");
  const stats = manager.stats().provider;
  assert.deepEqual(
    {
      requested: stats.requested,
      loaded: stats.loaded,
      failures: stats.failures,
      cancellations: stats.cancellations,
      encodedBytes: stats.encodedBytes,
    },
    { requested: 1, loaded: 1, failures: 0, cancellations: 0, encodedBytes: 3 },
  );
  assert.ok(stats.resolveMs >= 0);
  result.release();
});

test("stable ids deduplicate a request batch and retain decoded session values", async () => {
  let calls = 0;
  const manager = new BrowserSemanticResourceManager({
    provider: { async provide() { calls += 1; return { source: directSource(calls) }; } },
    softBytes: 1024,
    hardBytes: 2048,
  });
  const resource = descriptor("card/a");
  const first = await manager.acquire([resource, { ...resource }]);
  assert.equal(first.sources.size, 1);
  assert.equal(calls, 1);
  first.release();
  const warm = await manager.acquire([resource]);
  assert.equal(calls, 1);
  warm.release();
  assert.equal(manager.stats().provider.requested, 1);
});

test("concurrent acquire calls share one in-flight provider request", async () => {
  let calls = 0;
  let release;
  const pending = new Promise((resolve) => { release = resolve; });
  const manager = new BrowserSemanticResourceManager({
    provider: {
      async provide() {
        calls += 1;
        await pending;
        return { source: directSource("shared") };
      },
    },
    softBytes: 1024,
    hardBytes: 2048,
  });
  const resource = descriptor("card/shared");
  const first = manager.acquire([resource]);
  const second = manager.acquire([resource]);
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(calls, 1);
  release();
  const [a, b] = await Promise.all([first, second]);
  a.release();
  b.release();
});

test("aborting the first waiter does not cancel a shared provider request", async () => {
  let calls = 0;
  let release;
  const pending = new Promise((resolve) => { release = resolve; });
  const manager = new BrowserSemanticResourceManager({
    provider: {
      async provide(_resource, { signal }) {
        calls += 1;
        await Promise.race([
          pending,
          new Promise((_, reject) => signal.addEventListener("abort", () => reject(signal.reason), { once: true })),
        ]);
        return { source: directSource("shared") };
      },
    },
    softBytes: 1024,
    hardBytes: 2048,
  });
  const firstController = new AbortController();
  const secondController = new AbortController();
  const resource = descriptor("card/shared-abort-first");
  const first = manager.acquire([resource], firstController.signal);
  const second = manager.acquire([resource], secondController.signal);
  await new Promise((resolve) => setTimeout(resolve, 0));
  firstController.abort(new DOMException("first cancelled", "AbortError"));
  await assert.rejects(within(first), { name: "AbortError" });
  release();
  const retained = await second;
  assert.equal(calls, 1);
  retained.release();
});

test("aborting a later waiter rejects only that waiter", async () => {
  let calls = 0;
  let release;
  const pending = new Promise((resolve) => { release = resolve; });
  const manager = new BrowserSemanticResourceManager({
    provider: {
      async provide() {
        calls += 1;
        await pending;
        return { source: directSource("shared") };
      },
    },
    softBytes: 1024,
    hardBytes: 2048,
  });
  const firstController = new AbortController();
  const secondController = new AbortController();
  const resource = descriptor("card/shared-abort-second");
  const first = manager.acquire([resource], firstController.signal);
  const second = manager.acquire([resource], secondController.signal);
  await new Promise((resolve) => setTimeout(resolve, 0));
  secondController.abort(new DOMException("second cancelled", "AbortError"));
  await assert.rejects(within(second), { name: "AbortError" });
  release();
  const retained = await first;
  assert.equal(calls, 1);
  retained.release();
});

test("a new acquire starts a fresh load after the last waiter cancels", async () => {
  let calls = 0;
  let releaseFirst;
  const firstPending = new Promise((resolve) => { releaseFirst = resolve; });
  const manager = new BrowserSemanticResourceManager({
    provider: {
      async provide() {
        calls += 1;
        if (calls === 1) {
          await firstPending;
          return { source: directSource("stale") };
        }
        return { source: directSource("fresh") };
      },
    },
    softBytes: 1024,
    hardBytes: 2048,
  });
  const controller = new AbortController();
  const resource = descriptor("card/retry-after-cancel");
  const cancelled = manager.acquire([resource], controller.signal);
  await new Promise((resolve) => setTimeout(resolve, 0));
  controller.abort(new DOMException("cancelled", "AbortError"));
  await assert.rejects(within(cancelled), { name: "AbortError" });

  const retry = manager.acquire([resource]);
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(calls, 2);
  releaseFirst();
  const result = await retry;
  assert.equal(result.sources.get(resource.id).source.id, "fresh");
  result.release();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(
    {
      requested: manager.stats().provider.requested,
      loaded: manager.stats().provider.loaded,
      cancellations: manager.stats().provider.cancellations,
    },
    { requested: 2, loaded: 1, cancellations: 1 },
  );
});

test("direct image sources retain intrinsic video, frame, and SVG dimensions", async () => {
  const video = { width: 0, height: 0, videoWidth: 640, videoHeight: 360 };
  const frame = { displayWidth: 320, displayHeight: 180 };
  const svg = {
    width: { baseVal: { value: 48 } },
    height: { baseVal: { value: 24 } },
  };
  const manager = new BrowserSemanticResourceManager({
    provider: {
      async provide(resource) {
        if (resource.key.endsWith("video")) return { source: video };
        if (resource.key.endsWith("frame")) return { source: frame };
        return { source: svg };
      },
    },
    softBytes: 4 * 1024 * 1024,
    hardBytes: 8 * 1024 * 1024,
  });
  const result = await manager.acquire([
    descriptor("card/video"),
    descriptor("card/frame"),
    descriptor("card/svg"),
  ]);
  assert.deepEqual(result.sources.get("assets\0card/video"), {
    source: video,
    width: 640,
    height: 360,
  });
  assert.deepEqual(result.sources.get("assets\0card/frame"), {
    source: frame,
    width: 320,
    height: 180,
  });
  assert.deepEqual(result.sources.get("assets\0card/svg"), {
    source: svg,
    width: 48,
    height: 24,
  });
  result.release();
});

test("provider calls obey configured bounded concurrency", async () => {
  let active = 0;
  let peak = 0;
  const manager = new BrowserSemanticResourceManager({
    provider: {
      async provide(resource) {
        active += 1;
        peak = Math.max(peak, active);
        await new Promise((resolve) => setTimeout(resolve, 5));
        active -= 1;
        return { source: directSource(resource.key) };
      },
    },
    concurrency: 2,
    softBytes: 4096,
    hardBytes: 8192,
  });
  const resources = Array.from({ length: 7 }, (_, index) => descriptor(`card/${index}`));
  const result = await manager.acquire(resources);
  assert.equal(peak, 2);
  assert.equal(manager.stats().provider.peak, 2);
  result.release();
});

test("provider null and errors warn and resolve transparent placeholders independently", async () => {
  const originalWarn = console.warn;
  const warnings = [];
  let fallbackDecodes = 0;
  console.warn = (...values) => warnings.push(values.join(" "));
  try {
    const manager = new BrowserSemanticResourceManager({
      provider: {
        async provide(resource) {
          if (resource.key.endsWith("null")) return null;
          if (resource.key.endsWith("throw")) throw new Error("unavailable");
          return { source: directSource("ok") };
        },
      },
      decode: async (blob) => {
        assert.equal(blob.type, "image/png");
        assert.ok(blob.size > 0);
        fallbackDecodes += 1;
        return directSource(`fallback-${fallbackDecodes}`, 1, 1);
      },
      softBytes: 1024,
      hardBytes: 2048,
    });
    const result = await manager.acquire([
      descriptor("card/null"),
      descriptor("card/throw"),
      descriptor("card/ok"),
    ]);
    assert.equal(result.sources.size, 3);
    assert.equal(result.availability.get("assets\0card/null"), false);
    assert.equal(result.availability.get("assets\0card/throw"), false);
    assert.equal(result.availability.get("assets\0card/ok"), true);
    assert.equal(fallbackDecodes, 2);
    assert.equal(warnings.length, 2);
    assert.match(warnings[0], /card\/(null|throw)/);
    assert.match(warnings[1], /card\/(null|throw)/);
    const stats = manager.stats().provider;
    assert.deepEqual(
      {
        requested: stats.requested,
        loaded: stats.loaded,
        failures: stats.failures,
        cancellations: stats.cancellations,
      },
      { requested: 3, loaded: 1, failures: 2, cancellations: 0 },
    );
    result.release();
  } finally {
    console.warn = originalWarn;
  }
});

test("provider receives a shared AbortSignal controlled by active waiters", async () => {
  const controller = new AbortController();
  let received = null;
  const manager = new BrowserSemanticResourceManager({
    provider: {
      async provide(_resource, context) {
        received = context.signal;
        return { source: directSource("signal") };
      },
    },
    softBytes: 1024,
    hardBytes: 2048,
  });
  const result = await manager.acquire([descriptor("card/signal")], controller.signal);
  assert.ok(received instanceof AbortSignal);
  assert.notEqual(received, controller.signal);
  result.release();
});

test("public package exports provider types but no CDN or object-path helpers", async () => {
  const index = await readFile(new URL("../../src/index.ts", import.meta.url), "utf8");
  assert.match(index, /ResourceProvider/);
  assert.doesNotMatch(index, /defaultSemanticResourceUrl|gameAssetObjectPath|clearPersistentSemanticResourceCache/);
});

test("profile preparation maps lookup roles and fallback metrics into semantic descriptors", () => {
  assert.deepEqual(profileResourceDescriptors({
    resources: [{
      lookup_key: "honor:26:main",
      resource: { namespace: "static", key: "honor/honor_0026/rank_main" },
      fallback: { width: 380, height: 80 },
      provenance: { table: "honors", id: 26 },
    }],
  }), [{
    id: "static\0honor/honor_0026/rank_main",
    namespace: "static",
    key: "honor/honor_0026/rank_main",
    role: "honor:26:main",
    provenance: { table: "honors", id: 26 },
    expectedSize: { width: 380, height: 80 },
  }]);
});

test("renderer requires ResourceProvider and contains no resource URL resolver", async () => {
  const renderer = await readFile(new URL("../../src/renderer.ts", import.meta.url), "utf8");
  assert.match(renderer, /resourceProvider: ResourceProvider/);
  assert.match(renderer, /resourceConcurrency\?: number/);
  assert.doesNotMatch(renderer, /resolveResourceUrl|defaultSemanticResourceUrl|defaultMasterDataUrl|cdn\.emptysekai\.com/);
  assert.doesNotMatch(renderer, /resolveMasterDataUrl/);
  assert.match(renderer, /loadTable: MasterDataTableLoader/);
  assert.match(renderer, /const abort = combinedAbortSignal[\s\S]+finally \{\s+abort\.dispose\(\);\s+\}/);
  assert.match(renderer, /removeEventListener\("abort", abortLifetime\)/);
  assert.match(renderer, /removeEventListener\("abort", abortRequest\)/);
});
