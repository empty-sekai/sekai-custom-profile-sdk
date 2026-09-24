import assert from "node:assert/strict";
import test from "node:test";

import {
  GlyphPersistentCache,
  MemoryGlyphRecordStore,
  createGlyphRasterIdentity,
  createPersistentGlyphRecord,
} from "../../src/cache/glyphPersistentCache.ts";

const DAY = 86_400_000;

async function identity(glyphId = 42) {
  return createGlyphRasterIdentity({
    region: "cn",
    fontSha256: "11".repeat(32),
    faceIndex: 0,
    variationAxes: [],
    glyphId,
    pointSize26d6: 75 * 64,
    dpiX: 72,
    dpiY: 72,
    loadFlags: "NO_HINTING",
    renderMode: "normal-mask",
    spread26d6: 6 * 64,
    sdfAlgorithm: "edt",
    supersample: 2,
    threshold: 128,
    downsampleVersion: "box-v1",
    fontEngineFingerprint: "freetype-test",
    rasterContractId: "raster-test",
  });
}

async function record(id, now) {
  return createPersistentGlyphRecord(id, {
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
    pixels: new Uint8Array([1, 2, 3, 4]),
  }, now);
}

/** Counts writes and can hold a lookup between its read and its bookkeeping. */
class ObservedStore extends MemoryGlyphRecordStore {
  constructor() {
    super();
    this.writes = 0;
    this.gate = null;
  }
  async getMany(keys) {
    const records = await super.getMany(keys);
    if (this.gate) await this.gate;
    return records;
  }
  async putMany(records, token) {
    this.writes += 1;
    return super.putMany(records, token);
  }
  async touch(keys, lastAccessDay, token) {
    this.writes += 1;
    return super.touch(keys, lastAccessDay, token);
  }
}

test("a lookup racing a clear does not bring the cleared glyphs back", async () => {
  let now = 10 * DAY;
  const store = new ObservedStore();
  const cache = new GlyphPersistentCache({ mode: "origin", store, now: () => now });
  const id = await identity();
  await cache.putMany([await record(id, now)]);
  now += 2 * DAY;

  let release;
  store.gate = new Promise((resolve) => { release = resolve; });
  const lookup = cache.getMany([id]);
  await new Promise((resolve) => setTimeout(resolve, 0));
  await cache.clearPersistentCache();
  release();
  const found = await lookup;

  assert.equal(found.size, 1, "the lookup still answers from what it read");
  assert.equal((await store.stats()).entries, 0, "the cleared record stays cleared");
});

test("repeat hits on the same day do not rewrite the record", async () => {
  let now = 10 * DAY;
  const store = new ObservedStore();
  const cache = new GlyphPersistentCache({ mode: "origin", store, now: () => now });
  const id = await identity();
  await cache.putMany([await record(id, now)]);
  const writesAfterInsert = store.writes;

  assert.equal((await cache.getMany([id])).size, 1);
  assert.equal((await cache.getMany([id])).size, 1);
  assert.equal(store.writes, writesAfterInsert, "same-day hits leave the store untouched");

  now += DAY;
  assert.equal((await cache.getMany([id])).size, 1);
  assert.equal((await cache.getMany([id])).size, 1);
  assert.equal(store.writes, writesAfterInsert + 1, "the first hit of a new day refreshes the access day once");
  const [stored] = await store.getMany([id.opaqueKey]);
  assert.equal(stored.lastAccessDay, 11);
});
