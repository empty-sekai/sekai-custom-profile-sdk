import assert from "node:assert/strict";
import test from "node:test";

import { SessionImageResourceCache } from "../../src/cache/sessionImageResourceCache.ts";

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
