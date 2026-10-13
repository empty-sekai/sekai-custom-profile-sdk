import assert from "node:assert/strict";
import test from "node:test";

import {
  GlyphPersistentCache,
  MemoryGlyphRecordStore,
  createGlyphRasterIdentity,
  createPersistentGlyphRecord,
  persistentRecordPublicShape,
} from "../../src/cache/glyphPersistentCache.ts";

const MiB = 1024 * 1024;
const DAY = 86_400_000;

function keyInput(overrides = {}) {
  return {
    region: "cn",
    fontSha256: "11".repeat(32),
    faceIndex: 0,
    variationAxes: [],
    glyphId: 42,
    pointSize26d6: 75 * 64,
    dpiX: 72,
    dpiY: 72,
    loadFlags: "NO_HINTING",
    renderMode: "normal-mask",
    spread26d6: 6 * 64,
    sdfAlgorithm: "cpu-edt",
    supersample: 2,
    threshold: 128,
    downsampleVersion: "box-v1",
    fontEngineFingerprint: "freetype-2.13.3:min-v1:truetype,sfnt,cff,smooth",
    rasterContractId: "allium-r8-edt-ss2-v1",
    ...overrides,
  };
}

async function record(input = {}, pixels = new Uint8Array([1, 2, 3, 4]), now = 10 * DAY) {
  const identity = await createGlyphRasterIdentity(keyInput(input));
  return createPersistentGlyphRecord(
    identity,
    {
      advance: 51,
      xOffset: -3,
      yOffset: -62,
      planeBearingX: 3,
      planeBearingY: 62,
      planeWidth: 48,
      planeHeight: 63,
      drawable: true,
      width: 2,
      height: 2,
      pixels,
    },
    now,
  );
}

test("opaque glyph identity changes for every semantic cache-key field", async () => {
  const base = await createGlyphRasterIdentity(keyInput());
  const variants = [
    { region: "jp" },
    { fontSha256: "22".repeat(32) },
    { faceIndex: 1 },
    { variationAxes: [["wght", 700]] },
    { glyphId: 43 },
    { pointSize26d6: 74 * 64 },
    { dpiX: 96 },
    { dpiY: 96 },
    { loadFlags: "NO_BITMAP|NO_HINTING" },
    { renderMode: "outline" },
    { spread26d6: 5 * 64 },
    { sdfAlgorithm: "analytic" },
    { supersample: 3 },
    { threshold: 127 },
    { downsampleVersion: "box-v2" },
    { fontEngineFingerprint: "freetype-next" },
    { rasterContractId: "raster-next" },
  ];
  for (const variant of variants) {
    const changed = await createGlyphRasterIdentity(keyInput(variant));
    assert.notEqual(changed.opaqueKey, base.opaqueKey, JSON.stringify(variant));
  }
  assert.match(base.opaqueKey, /^[0-9a-f]{64}$/);
});

test("persistent record is opaque and contains no source or profile metadata", async () => {
  const value = await record();
  const json = JSON.stringify(persistentRecordPublicShape(value));
  for (const forbidden of ["char", "scalar", "cluster", "family", "profile", "document", "userId"]) {
    assert.equal(json.includes(forbidden), false, forbidden);
  }
  assert.equal(value.payloadLength, 4);
  assert.match(value.payloadDigest, /^[0-9a-f]{64}$/);
});

test("corrupt payload and incompatible engine records are deleted and counted as misses", async () => {
  const store = new MemoryGlyphRecordStore();
  const cache = new GlyphPersistentCache({ mode: "origin", store, now: () => 10 * DAY });
  const good = await record();
  await store.putMany([{ ...good, payloadDigest: "00".repeat(32) }]);
  const corrupt = await cache.getMany([good.identity]);
  assert.equal(corrupt.size, 0);
  assert.equal((await store.stats()).entries, 0);

  const incompatible = await record({ fontEngineFingerprint: "freetype-old" });
  await store.putMany([incompatible]);
  const miss = await cache.getMany([good.identity]);
  assert.equal(miss.size, 0);
  assert.equal(cache.stats().corruptions, 1);
  assert.equal(cache.stats().misses, 2);
});

test("insert trims LRU to soft budget and never crosses hard budget", async () => {
  const store = new MemoryGlyphRecordStore();
  let now = 20 * DAY;
  const cache = new GlyphPersistentCache({
    mode: "origin",
    store,
    softBytes: 1 * MiB,
    hardBytes: 1.5 * MiB,
    now: () => now,
  });
  const first = await record({ glyphId: 1 }, new Uint8Array(700_000), now);
  await cache.putMany([first]);
  now += DAY;
  const second = await record({ glyphId: 2 }, new Uint8Array(700_000), now);
  await cache.putMany([second]);
  const stats = await store.stats();
  assert.ok(stats.bytes <= 1 * MiB, JSON.stringify(stats));
  assert.equal(stats.entries, 1);
  assert.equal(cache.stats().evictions, 1);

  const tooLarge = await record({ glyphId: 3 }, new Uint8Array(2 * MiB), now);
  await cache.putMany([tooLarge]);
  assert.equal(cache.stats().skippedWrites, 1);
  assert.ok((await store.stats()).bytes <= 1.5 * MiB);
});

test("one putMany batch accounts for accepted records before enforcing the hard budget", async () => {
  const store = new MemoryGlyphRecordStore();
  const cache = new GlyphPersistentCache({
    mode: "origin",
    store,
    softBytes: 1 * MiB,
    hardBytes: 1.5 * MiB,
  });
  const first = await record({ glyphId: 101 }, new Uint8Array(800_000));
  const second = await record({ glyphId: 102 }, new Uint8Array(800_000));

  await cache.putMany([first, second]);

  const stats = await store.stats();
  assert.ok(stats.bytes <= 1.5 * MiB, JSON.stringify(stats));
  assert.equal(stats.entries, 1);
  assert.equal(cache.stats().inserts, 1);
  assert.equal(cache.stats().skippedWrites, 1);
});

test("TTL sweep, stats and clear are bounded public operations", async () => {
  const store = new MemoryGlyphRecordStore();
  const now = 50 * DAY;
  const cache = new GlyphPersistentCache({ mode: "origin", store, ttlDays: 30, now: () => now });
  await store.putMany([
    await record({ glyphId: 1 }, new Uint8Array(4), 1 * DAY),
    await record({ glyphId: 2 }, new Uint8Array(4), 49 * DAY),
  ]);
  const swept = await cache.sweep({ maxRecords: 1 });
  assert.equal(swept.scanned, 1);
  assert.equal(swept.deleted, 1);
  assert.equal((await cache.getPersistentCacheStats()).entries, 1);
  await cache.clearPersistentCache();
  assert.equal((await cache.getPersistentCacheStats()).entries, 0);
});

test("quota/private-mode failures degrade to memory-only without rejecting rendering", async () => {
  const store = new MemoryGlyphRecordStore({ failWrites: true });
  const cache = new GlyphPersistentCache({ mode: "origin", store });
  await assert.doesNotReject(cache.putMany([await record()]));
  assert.equal(cache.stats().degraded, true);
  assert.equal(cache.stats().writeErrors, 1);
  assert.equal((await cache.getMany([(await record()).identity])).size, 0);
});

test("clear invalidates writes that were queued by an older render", async () => {
  const store = new MemoryGlyphRecordStore();
  const cache = new GlyphPersistentCache({ mode: "origin", store });
  const writeEpoch = cache.beginWrite();
  await cache.clearPersistentCache();
  await cache.putMany([await record()], writeEpoch);
  assert.equal((await cache.getPersistentCacheStats()).entries, 0);
  assert.equal(cache.stats().cancelledWrites, 1);
});
