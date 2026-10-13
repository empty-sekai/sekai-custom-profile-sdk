import assert from "node:assert/strict";
import test from "node:test";

import { buildSdfAtlas } from "../../dist/fontSdfAtlas.js";

test("atlas telemetry reports a reused session glyph instead of a persistent hit", async () => {
  const resources = new Set();
  let persisted = null;
  const atlas = {
    initialStats: stats(),
    async resolve(keys, cached, generate) {
      const generated = [];
      for (const record of cached) resources.add(record.key);
      for (const group of generate) for (const glyph of group.glyphs) {
        if (resources.has(glyph.key)) continue;
        resources.add(glyph.key);
        generated.push(glyphRecord(glyph.key, glyph.glyphIndex));
      }
      return {
        generated,
        placements: keys.map((key) => ({ key, placement: { page: 0, pageEpoch: 1, u0: 0, v0: 0, u1: 1 / 2048, v1: 1 / 2048 } })),
        leases: [resources.size],
        stats: stats(),
      };
    },
    async pages() { return []; },
    async release() {},
  };
  const worker = {
    async registerFont() {},
    async createAtlas() { return atlas; },
    async planGlyphs(request) {
      return {
        region: request.region,
        family: request.family,
        fontSourceHash: request.sourceHash,
        schemaNamespace: "allium.glyph-raster-cache.v1",
        fontEngineFingerprint: "freetype-test",
        rasterContractId: "allium-r8-edt-ss2-spread6-threshold128-box-v1",
        contractId: "freetype-test:allium-r8-edt-ss2-spread6-threshold128-box-v1",
        backend: "edt",
        supersample: 2,
        baseSize: 75,
        spread: 6,
        atlasWidth: 2048,
        atlasHeight: 2048,
        glyphs: request.chars.map((ch) => ({
          ch,
          glyphIndex: 7,
          identity: {
            opaqueKey: "77".repeat(32),
            schemaNamespace: "allium.glyph-raster-cache.v1",
            fontEngineFingerprint: "freetype-test",
            rasterContractId: "allium-r8-edt-ss2-spread6-threshold128-box-v1",
          },
        })),
        missing: [],
      };
    },
    async stats() { return { requests: 0, failures: 0, wasmMs: 0, transferBytes: 0 }; },
  };
  const persistentCache = {
    async getMany(identities) {
      return persisted ? new Map([[persisted.opaqueKey, persisted]]) : new Map();
    },
    beginWrite() { return { epoch: 0, startedAtMs: 0 }; },
    async putMany(records) { persisted = records[0] ?? null; },
  };
  const sourceHash = "11".repeat(32);
  const fontSources = [{ region: "cn", family: "Font 1", sourceHash, bytes: new ArrayBuffer(1) }];
  const requests = [{ region: "cn", family: "Font 1", fontSourceHash: sourceHash, char: "4" }];

  const cold = await buildSdfAtlas(fontSources, requests, { worker, persistentCache });
  assert.deepEqual(pick(cold.perf.cache), { sessionHits: 0, persistentHits: 0, persistentMisses: 1, generations: 1 });
  const warm = await buildSdfAtlas(fontSources, requests, { worker, persistentCache });

  assert.deepEqual(pick(warm.perf.cache), { sessionHits: 1, persistentHits: 0, persistentMisses: 0, generations: 0 });
});

function pick(cache) {
  return {
    sessionHits: cache.sessionHits,
    persistentHits: cache.persistentHits,
    persistentMisses: cache.persistentMisses,
    generations: cache.generations,
  };
}

function stats() {
  return { pages: 1, atlasBytes: 2048 * 2048, pinnedPages: 1, evictions: 0 };
}

function glyphRecord(key, glyphIndex) {
  return {
    key, glyphIndex, width: 1, height: 1, advance: 1,
    xOffset: 0, yOffset: 0, planeBearingX: 0, planeBearingY: 0,
    planeWidth: 1, planeHeight: 1, drawable: true,
    pixels: new Uint8Array([255]),
  };
}
