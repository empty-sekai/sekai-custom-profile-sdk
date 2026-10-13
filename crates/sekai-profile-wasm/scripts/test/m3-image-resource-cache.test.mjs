import assert from "node:assert/strict";
import test from "node:test";

import { SessionImageResourceCache } from "../../src/cache/sessionImageResourceCache.ts";

function deferred() {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
}

test("concurrent image requests decode once and leases are reference counted", async () => {
  let loads = 0;
  const cache = new SessionImageResourceCache({ softBytes: 100, hardBytes: 120 });
  const loader = async () => ({ value: { id: ++loads }, bytes: 40 });
  const [a, b] = await Promise.all([cache.acquire("asset:a", loader), cache.acquire("asset:a", loader)]);
  assert.equal(loads, 1);
  assert.equal(a.value, b.value);
  assert.deepEqual(cache.stats(), { entries: 1, bytes: 40, pinned: 1, loads: 1, hits: 1, evictions: 0 });
  a.release();
  b.release();
  b.release();
  assert.equal(cache.stats().pinned, 0);
});

test("unpinned decoded images evict to soft budget without crossing hard budget", async () => {
  const cache = new SessionImageResourceCache({ softBytes: 60, hardBytes: 100 });
  const load = (id) => async () => ({ value: { id }, bytes: 40 });
  const a = await cache.acquire("a", load("a"));
  a.release();
  const b = await cache.acquire("b", load("b"));
  b.release();
  assert.deepEqual(cache.keys(), ["b"]);
  assert.equal(cache.stats().bytes, 40);
  await assert.rejects(() => cache.acquire("huge", async () => ({ value: {}, bytes: 101 })), /hard byte budget/);
  assert.equal(cache.stats().bytes, 40);
});

test("an entry above the soft budget stays alive until its first lease releases", async () => {
  const disposed = [];
  const value = { id: "large" };
  const cache = new SessionImageResourceCache({
    softBytes: 60,
    hardBytes: 100,
    dispose: (entry) => disposed.push(entry.id),
  });
  const lease = await cache.acquire("large", async () => ({ value, bytes: 80 }));
  assert.equal(lease.value, value);
  assert.deepEqual(disposed, []);
  assert.equal(cache.stats().pinned, 1);
  lease.release();
  assert.deepEqual(disposed, ["large"]);
});

test("failed image jobs are removed and can be retried", async () => {
  const cache = new SessionImageResourceCache({ softBytes: 100, hardBytes: 120 });
  let calls = 0;
  await assert.rejects(() => cache.acquire("a", async () => { calls += 1; throw new Error("decode"); }), /decode/);
  const lease = await cache.acquire("a", async () => ({ value: { calls: ++calls }, bytes: 20 }));
  assert.equal(lease.value.calls, 2);
  lease.release();
});

test("evicted decoded resources are disposed exactly once", async () => {
  const disposed = [];
  const cache = new SessionImageResourceCache({ softBytes: 20, hardBytes: 80, dispose: (value) => disposed.push(value.id) });
  const a = await cache.acquire("a", async () => ({ value: { id: "a" }, bytes: 40 }));
  a.release();
  assert.deepEqual(disposed, ["a"]);
  cache.clear();
  assert.deepEqual(disposed, ["a"]);
});

test("concurrent admissions cannot evict an image before its lease is delivered", async () => {
  const disposed = [];
  const cache = new SessionImageResourceCache({
    softBytes: 40,
    hardBytes: 40,
    dispose: (value) => disposed.push(value.id),
  });
  const [first, second] = await Promise.allSettled([
    cache.acquire("a", async () => ({ value: { id: "a" }, bytes: 40 })),
    cache.acquire("b", async () => ({ value: { id: "b" }, bytes: 40 })),
  ]);
  assert.equal(first.status, "fulfilled");
  assert.equal(second.status, "rejected");
  assert.match(second.reason.message, /hard byte budget is pinned/);
  assert.deepEqual(disposed, ["b"]);
  assert.equal(first.value.value.id, "a");
  assert.deepEqual(cache.stats(), { entries: 1, bytes: 40, pinned: 1, loads: 2, hits: 0, evictions: 0 });
  first.value.release();
  cache.clear();
  assert.deepEqual(disposed, ["b", "a"]);
});

test("soft trimming from one completed acquire preserves other waiting leases", async () => {
  const disposed = [];
  const cache = new SessionImageResourceCache({
    softBytes: 0,
    hardBytes: 80,
    dispose: (value) => disposed.push(value.id),
  });
  const [a, b] = await Promise.all([
    cache.acquire("a", async () => ({ value: { id: "a" }, bytes: 40 })),
    cache.acquire("b", async () => ({ value: { id: "b" }, bytes: 40 })),
  ]);
  assert.deepEqual(disposed, []);
  assert.equal(cache.stats().pinned, 2);
  a.release();
  assert.deepEqual(disposed, ["a"]);
  assert.equal(cache.stats().pinned, 1);
  b.release();
  assert.deepEqual(disposed, ["a", "b"]);
  assert.equal(cache.stats().bytes, 0);
});

test("clear preserves an admitted image while its first lease is still being delivered", async () => {
  const decoded = deferred();
  const disposed = [];
  const cache = new SessionImageResourceCache({
    softBytes: 40,
    hardBytes: 40,
    dispose: (value) => disposed.push(value.id),
  });
  const pending = cache.acquire("a", () => decoded.promise);
  await Promise.resolve();
  decoded.resolve({ value: { id: "a" }, bytes: 40 });
  // Admission runs in the next microtask, before the acquire continuation.
  await Promise.resolve();
  assert.equal(cache.stats().entries, 1);
  cache.clear();
  assert.deepEqual(disposed, []);
  const lease = await pending;
  assert.equal(cache.stats().pinned, 1);
  lease.release();
  cache.clear();
  assert.deepEqual(disposed, ["a"]);
});

test("cancellation after admission releases only the cancelled waiter's reservation", async () => {
  const decoded = deferred();
  const controller = new AbortController();
  const disposed = [];
  let loads = 0;
  const cache = new SessionImageResourceCache({
    softBytes: 0,
    hardBytes: 40,
    dispose: (value) => disposed.push(value.id),
  });
  const loader = () => { loads += 1; return decoded.promise; };
  const cancelled = cache.acquire("a", loader, controller.signal);
  const rejected = assert.rejects(cancelled, { name: "AbortError" });
  const surviving = cache.acquire("a", loader);
  await Promise.resolve();
  decoded.resolve({ value: { id: "a" }, bytes: 40 });
  await Promise.resolve();
  assert.equal(cache.stats().entries, 1);
  controller.abort();
  await rejected;
  const lease = await surviving;
  assert.equal(loads, 1);
  assert.equal(lease.value.id, "a");
  assert.equal(cache.stats().pinned, 1);
  assert.deepEqual(disposed, []);
  lease.release();
  lease.release();
  assert.deepEqual(disposed, ["a"]);
  assert.equal(cache.stats().bytes, 0);
});

test("cancelling all waiters after admission leaves no pinned image", async () => {
  const decoded = deferred();
  const controllers = [new AbortController(), new AbortController()];
  const disposed = [];
  const cache = new SessionImageResourceCache({
    softBytes: 0,
    hardBytes: 40,
    dispose: (value) => disposed.push(value.id),
  });
  const rejections = controllers.map((controller) => assert.rejects(
    cache.acquire("a", () => decoded.promise, controller.signal),
    { name: "AbortError" },
  ));
  await Promise.resolve();
  decoded.resolve({ value: { id: "a" }, bytes: 40 });
  await Promise.resolve();
  assert.equal(cache.stats().entries, 1);
  for (const controller of controllers) controller.abort();
  await Promise.all(rejections);
  assert.equal(cache.stats().pinned, 0);
  assert.equal(cache.stats().bytes, 0);
  cache.clear();
  assert.deepEqual(disposed, ["a"]);
});

test("cancellation before admission does not reserve an extra reference", async () => {
  const decoded = deferred();
  const controller = new AbortController();
  const disposed = [];
  const cache = new SessionImageResourceCache({
    softBytes: 0,
    hardBytes: 40,
    dispose: (value) => disposed.push(value.id),
  });
  const cancelled = assert.rejects(
    cache.acquire("a", () => decoded.promise, controller.signal),
    { name: "AbortError" },
  );
  const surviving = cache.acquire("a", () => decoded.promise);
  await Promise.resolve();
  controller.abort();
  await cancelled;
  assert.equal(cache.stats().entries, 0);
  decoded.resolve({ value: { id: "a" }, bytes: 40 });
  const lease = await surviving;
  assert.deepEqual(disposed, []);
  assert.equal(cache.stats().pinned, 1);
  lease.release();
  assert.deepEqual(disposed, ["a"]);
  assert.equal(cache.stats().bytes, 0);
});

for (const bytes of [-1, 0.5, NaN, Infinity, 101]) {
  test(`rejected admission disposes a decoded image once for byte size ${bytes}`, async () => {
    const disposed = [];
    const cache = new SessionImageResourceCache({
      softBytes: 100,
      hardBytes: 100,
      dispose: (value) => disposed.push(value.id),
    });
    const results = await Promise.allSettled([
      cache.acquire("a", async () => ({ value: { id: "rejected" }, bytes })),
      cache.acquire("a", async () => { throw new Error("duplicate decode"); }),
    ]);
    for (const result of results) {
      assert.equal(result.status, "rejected");
      assert.match(result.reason.message, bytes === 101 ? /hard byte budget/ : /invalid decoded image byte size/);
    }
    assert.deepEqual(disposed, ["rejected"]);
    assert.equal(cache.stats().bytes, 0);
    assert.equal(cache.stats().entries, 0);
    assert.equal(cache.stats().loads, 1);
    const retry = await cache.acquire("a", async () => ({ value: { id: "retry" }, bytes: 40 }));
    assert.equal(retry.value.id, "retry");
    retry.release();
    cache.clear();
    assert.deepEqual(disposed, ["rejected", "retry"]);
  });
}

test("an admission rejected by pinned images disposes its value and can be retried", async () => {
  const disposed = [];
  const cache = new SessionImageResourceCache({
    softBytes: 100,
    hardBytes: 100,
    dispose: (value) => disposed.push(value.id),
  });
  const a = await cache.acquire("a", async () => ({ value: { id: "a" }, bytes: 80 }));
  await assert.rejects(
    cache.acquire("b", async () => ({ value: { id: "b-rejected" }, bytes: 40 })),
    /hard byte budget is pinned/,
  );
  assert.deepEqual(disposed, ["b-rejected"]);
  assert.deepEqual(cache.keys(), ["a"]);
  assert.equal(cache.stats().pinned, 1);
  a.release();
  const b = await cache.acquire("b", async () => ({ value: { id: "b-retry" }, bytes: 40 }));
  assert.equal(b.value.id, "b-retry");
  assert.deepEqual(disposed, ["b-rejected", "a"]);
  assert.deepEqual(cache.stats(), { entries: 1, bytes: 40, pinned: 1, loads: 3, hits: 0, evictions: 1 });
  b.release();
  cache.clear();
  assert.deepEqual(disposed, ["b-rejected", "a", "b-retry"]);
});

test("a cancelled loader completing late disposes only its abandoned image", async () => {
  const decoded = deferred();
  const abandonedDisposed = deferred();
  const disposed = [];
  const controller = new AbortController();
  const cache = new SessionImageResourceCache({
    softBytes: 40,
    hardBytes: 40,
    dispose: (value) => {
      disposed.push(value.id);
      if (value.id === "abandoned") abandonedDisposed.resolve();
    },
  });
  const cancelled = assert.rejects(
    cache.acquire("a", () => decoded.promise, controller.signal),
    { name: "AbortError" },
  );
  await Promise.resolve();
  controller.abort();
  await cancelled;
  const retry = await cache.acquire("a", async () => ({ value: { id: "retry" }, bytes: 40 }));
  decoded.resolve({ value: { id: "abandoned" }, bytes: 40 });
  await abandonedDisposed.promise;
  assert.deepEqual(disposed, ["abandoned"]);
  assert.equal(retry.value.id, "retry");
  assert.deepEqual(cache.keys(), ["a"]);
  assert.equal(cache.stats().pinned, 1);
  retry.release();
  cache.clear();
  assert.deepEqual(disposed, ["abandoned", "retry"]);
});
