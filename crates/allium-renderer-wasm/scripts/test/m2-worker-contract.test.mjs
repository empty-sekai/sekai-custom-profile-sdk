import assert from "node:assert/strict";
import test from "node:test";

import { GlyphWorkScheduler } from "../../src/worker/glyphWorkScheduler.ts";

test("concurrent requests share one glyph generation", async () => {
  let calls = 0;
  const scheduler = new GlyphWorkScheduler(async (jobs) => {
    calls += 1;
    await new Promise((resolve) => setTimeout(resolve, 5));
    return jobs.map((job) => ({ key: job.key, value: `${job.key}:generated` }));
  });
  const jobs = [{ key: "a" }, { key: "b" }];
  const [left, right] = await Promise.all([scheduler.run(jobs), scheduler.run(jobs)]);
  assert.equal(calls, 1);
  assert.deepEqual(left, right);
  assert.equal(scheduler.stats().coalesced, 2);
});

test("a failed in-flight job is removed and can be retried", async () => {
  let calls = 0;
  const scheduler = new GlyphWorkScheduler(async (jobs) => {
    calls += 1;
    if (calls === 1) throw new Error("first generation failed");
    return jobs.map((job) => ({ key: job.key, value: "ok" }));
  });
  await assert.rejects(scheduler.run([{ key: "a" }]), /first generation failed/);
  assert.deepEqual(await scheduler.run([{ key: "a" }]), [{ key: "a", value: "ok" }]);
  assert.equal(calls, 2);
  assert.equal(scheduler.stats().pending, 0);
});

test("cached values skip worker dispatch and preserve request order", async () => {
  let dispatched = [];
  const scheduler = new GlyphWorkScheduler(async (jobs) => {
    dispatched.push(jobs.map((job) => job.key));
    return jobs.map((job) => ({ key: job.key, value: job.key.toUpperCase() }));
  });
  await scheduler.run([{ key: "a" }, { key: "b" }]);
  const result = await scheduler.run([{ key: "b" }, { key: "c" }, { key: "a" }]);
  assert.deepEqual(dispatched, [["a", "b"], ["c"]]);
  assert.deepEqual(result.map((entry) => entry.value), ["B", "C", "A"]);
  assert.equal(scheduler.stats().cacheHits, 2);
});

test("persistent-cache results can prime the session without worker dispatch", async () => {
  let calls = 0;
  const scheduler = new GlyphWorkScheduler(async () => {
    calls += 1;
    return [];
  });
  scheduler.prime([{ key: "persisted", value: "IDB" }]);
  assert.deepEqual(await scheduler.run([{ key: "persisted" }]), [{ key: "persisted", value: "IDB" }]);
  assert.equal(calls, 0);
});
