export const GLYPH_CACHE_SCHEMA = "allium.glyph-raster-cache.v1";

export type PersistentCacheMode = "memory-only" | "origin";

export type GlyphRasterKeyInput = {
  region: string;
  fontSha256: string;
  faceIndex: number;
  variationAxes: ReadonlyArray<readonly [string, number]>;
  glyphId: number;
  pointSize26d6: number;
  dpiX: number;
  dpiY: number;
  loadFlags: string;
  renderMode: string;
  spread26d6: number;
  sdfAlgorithm: string;
  supersample: number;
  threshold: number;
  downsampleVersion: string;
  fontEngineFingerprint: string;
  rasterContractId: string;
};

export type GlyphRasterIdentity = {
  opaqueKey: string;
  schemaNamespace: string;
  fontEngineFingerprint: string;
  rasterContractId: string;
};

export type PersistentGlyphMetrics = {
  advance: number;
  xOffset: number;
  yOffset: number;
  planeBearingX: number;
  planeBearingY: number;
  planeWidth: number;
  planeHeight: number;
  drawable: boolean;
  width: number;
  height: number;
  pixels: Uint8Array;
};

export type PersistentGlyphRecord = {
  opaqueKey: string;
  schemaNamespace: string;
  fontEngineFingerprint: string;
  rasterContractId: string;
  identity: GlyphRasterIdentity;
  format: "r8";
  advance: number;
  xOffset: number;
  yOffset: number;
  planeBearingX: number;
  planeBearingY: number;
  planeWidth: number;
  planeHeight: number;
  drawable: boolean;
  width: number;
  height: number;
  pixels: ArrayBuffer;
  payloadLength: number;
  payloadDigest: string;
  byteSize: number;
  lastAccessDay: number;
};

export type PersistentStoreStats = { entries: number; bytes: number };
export type PersistentSweepResult = { scanned: number; deleted: number };
export type PersistentWriteToken = { epoch: number; startedAtMs: number };

export interface GlyphRecordStore {
  getMany(keys: readonly string[]): Promise<PersistentGlyphRecord[]>;
  putMany(records: readonly PersistentGlyphRecord[], token?: PersistentWriteToken): Promise<boolean>;
  deleteMany(keys: readonly string[]): Promise<void>;
  stats(): Promise<PersistentStoreStats>;
  trimLruTo(maxBytes: number, protectedKeys?: ReadonlySet<string>): Promise<number>;
  sweepBefore(lastAccessDayExclusive: number, maxRecords: number): Promise<PersistentSweepResult>;
  clear(clearedAtMs?: number): Promise<void>;
}

export type GlyphPersistentCacheStats = {
  lookups: number;
  hits: number;
  misses: number;
  inserts: number;
  evictions: number;
  corruptions: number;
  skippedWrites: number;
  cancelledWrites: number;
  readErrors: number;
  writeErrors: number;
  degraded: boolean;
};

export type GlyphPersistentCacheOptions = {
  mode?: PersistentCacheMode;
  store?: GlyphRecordStore;
  softBytes?: number;
  hardBytes?: number;
  ttlDays?: number;
  now?: () => number;
};

const DAY_MS = 86_400_000;
const DEFAULT_SOFT_BYTES = 64 * 1024 * 1024;
const DEFAULT_HARD_BYTES = 96 * 1024 * 1024;

export async function createGlyphRasterIdentity(input: GlyphRasterKeyInput): Promise<GlyphRasterIdentity> {
  validateKeyInput(input);
  const canonical = [
    ["schema", GLYPH_CACHE_SCHEMA],
    ["region", input.region],
    ["font_sha256", input.fontSha256.toLowerCase()],
    ["face_index", input.faceIndex],
    ["variation_axes", [...input.variationAxes].sort(([a], [b]) => a.localeCompare(b))],
    ["glyph_id", input.glyphId],
    ["point_size_26d6", input.pointSize26d6],
    ["dpi_x", input.dpiX],
    ["dpi_y", input.dpiY],
    ["load_flags", input.loadFlags],
    ["render_mode", input.renderMode],
    ["spread_26d6", input.spread26d6],
    ["sdf_algorithm", input.sdfAlgorithm],
    ["supersample", input.supersample],
    ["threshold", input.threshold],
    ["downsample_version", input.downsampleVersion],
    ["font_engine", input.fontEngineFingerprint],
    ["raster_contract", input.rasterContractId],
  ];
  const opaqueKey = await sha256Hex(new TextEncoder().encode(JSON.stringify(canonical)));
  return {
    opaqueKey,
    schemaNamespace: GLYPH_CACHE_SCHEMA,
    fontEngineFingerprint: input.fontEngineFingerprint,
    rasterContractId: input.rasterContractId,
  };
}

export async function createPersistentGlyphRecord(
  identity: GlyphRasterIdentity,
  glyph: PersistentGlyphMetrics,
  nowMs = Date.now(),
): Promise<PersistentGlyphRecord> {
  const pixels = glyph.pixels.slice();
  const payloadDigest = await sha256Hex(pixels);
  const byteSize = estimateRecordBytes(pixels.byteLength);
  return {
    opaqueKey: identity.opaqueKey,
    schemaNamespace: identity.schemaNamespace,
    fontEngineFingerprint: identity.fontEngineFingerprint,
    rasterContractId: identity.rasterContractId,
    identity: { ...identity },
    format: "r8",
    advance: glyph.advance,
    xOffset: glyph.xOffset,
    yOffset: glyph.yOffset,
    planeBearingX: glyph.planeBearingX,
    planeBearingY: glyph.planeBearingY,
    planeWidth: glyph.planeWidth,
    planeHeight: glyph.planeHeight,
    drawable: glyph.drawable,
    width: glyph.width,
    height: glyph.height,
    pixels: pixels.buffer.slice(pixels.byteOffset, pixels.byteOffset + pixels.byteLength) as ArrayBuffer,
    payloadLength: pixels.byteLength,
    payloadDigest,
    byteSize,
    lastAccessDay: dayBucket(nowMs),
  };
}

export function persistentRecordPublicShape(record: PersistentGlyphRecord): Omit<PersistentGlyphRecord, "pixels"> {
  const { pixels: _, ...publicShape } = record;
  return publicShape;
}

export class GlyphPersistentCache {
  private readonly mode: PersistentCacheMode;
  private readonly store?: GlyphRecordStore;
  private readonly softBytes: number;
  private readonly hardBytes: number;
  private readonly ttlDays: number;
  private readonly now: () => number;
  private readonly counters: GlyphPersistentCacheStats = {
    lookups: 0,
    hits: 0,
    misses: 0,
    inserts: 0,
    evictions: 0,
    corruptions: 0,
    skippedWrites: 0,
    cancelledWrites: 0,
    readErrors: 0,
    writeErrors: 0,
    degraded: false,
  };
  private writeEpoch = 0;

  constructor(options: GlyphPersistentCacheOptions = {}) {
    this.mode = options.mode ?? "memory-only";
    this.store = options.store;
    this.softBytes = options.softBytes ?? DEFAULT_SOFT_BYTES;
    this.hardBytes = options.hardBytes ?? DEFAULT_HARD_BYTES;
    this.ttlDays = options.ttlDays ?? 30;
    this.now = options.now ?? Date.now;
    if (this.softBytes <= 0 || this.hardBytes < this.softBytes) {
      throw new Error("persistent glyph cache requires 0 < softBytes <= hardBytes");
    }
  }

  async getMany(identities: readonly GlyphRasterIdentity[]): Promise<Map<string, PersistentGlyphRecord>> {
    this.counters.lookups += identities.length;
    if (!this.available()) {
      this.counters.misses += identities.length;
      return new Map();
    }
    try {
      const expected = new Map(identities.map((identity) => [identity.opaqueKey, identity]));
      const records = await this.store!.getMany([...expected.keys()]);
      const valid = new Map<string, PersistentGlyphRecord>();
      const corruptKeys: string[] = [];
      for (const record of records) {
        const identity = expected.get(record.opaqueKey);
        if (!identity || !(await validateRecord(record, identity))) {
          corruptKeys.push(record.opaqueKey);
          this.counters.corruptions += 1;
          continue;
        }
        record.lastAccessDay = dayBucket(this.now());
        valid.set(record.opaqueKey, record);
      }
      if (corruptKeys.length > 0) await this.store!.deleteMany(corruptKeys);
      if (valid.size > 0) await this.store!.putMany([...valid.values()]);
      this.counters.hits += valid.size;
      this.counters.misses += identities.length - valid.size;
      return valid;
    } catch {
      this.counters.readErrors += 1;
      this.counters.degraded = true;
      this.counters.misses += identities.length;
      return new Map();
    }
  }

  beginWrite(): PersistentWriteToken {
    return { epoch: this.writeEpoch, startedAtMs: this.now() };
  }

  async putMany(
    records: readonly PersistentGlyphRecord[],
    writeToken: PersistentWriteToken = this.beginWrite(),
  ): Promise<void> {
    if (writeToken.epoch !== this.writeEpoch) {
      this.counters.cancelledWrites += records.length;
      return;
    }
    if (!this.available() || records.length === 0) return;
    try {
      const accepted: PersistentGlyphRecord[] = [];
      let pendingBytes = 0;
      for (const record of records) {
        if (record.byteSize > this.hardBytes) {
          this.counters.skippedWrites += 1;
          continue;
        }
        const before = await this.store!.stats();
        const pendingWithRecord = pendingBytes + record.byteSize;
        if (before.bytes + pendingWithRecord > this.softBytes) {
          const target = Math.max(0, this.softBytes - pendingWithRecord);
          this.counters.evictions += await this.store!.trimLruTo(target, new Set([record.opaqueKey]));
        }
        const afterTrim = await this.store!.stats();
        if (afterTrim.bytes + pendingWithRecord > this.hardBytes) {
          this.counters.skippedWrites += 1;
          continue;
        }
        accepted.push(record);
        pendingBytes = pendingWithRecord;
      }
      if (accepted.length > 0) {
        const acceptedByStore = await this.store!.putMany(accepted, writeToken);
        if (acceptedByStore) {
          this.counters.inserts += accepted.length;
        } else {
          this.counters.cancelledWrites += accepted.length;
        }
      }
    } catch {
      this.counters.writeErrors += 1;
      this.counters.degraded = true;
    }
  }

  async sweep(options: { maxRecords?: number } = {}): Promise<PersistentSweepResult> {
    if (!this.available()) return { scanned: 0, deleted: 0 };
    try {
      const cutoff = dayBucket(this.now()) - this.ttlDays;
      return await this.store!.sweepBefore(cutoff, options.maxRecords ?? 128);
    } catch {
      this.counters.writeErrors += 1;
      this.counters.degraded = true;
      return { scanned: 0, deleted: 0 };
    }
  }

  async trimPersistentCache(targetBytes = this.softBytes): Promise<number> {
    if (!this.available()) return 0;
    try {
      const evicted = await this.store!.trimLruTo(Math.max(0, targetBytes));
      this.counters.evictions += evicted;
      return evicted;
    } catch {
      this.counters.writeErrors += 1;
      this.counters.degraded = true;
      return 0;
    }
  }

  async clearPersistentCache(): Promise<void> {
    this.writeEpoch += 1;
    if (!this.store) return;
    try {
      await this.store.clear(this.now());
    } catch {
      this.counters.writeErrors += 1;
      this.counters.degraded = true;
    }
  }

  async getPersistentCacheStats(): Promise<PersistentStoreStats & GlyphPersistentCacheStats> {
    const stored = this.available() ? await this.store!.stats().catch(() => ({ entries: 0, bytes: 0 })) : { entries: 0, bytes: 0 };
    return { ...stored, ...this.stats() };
  }

  stats(): GlyphPersistentCacheStats {
    return { ...this.counters };
  }

  private available(): boolean {
    return this.mode === "origin" && this.store != null && !this.counters.degraded;
  }
}

export class MemoryGlyphRecordStore implements GlyphRecordStore {
  private readonly records = new Map<string, PersistentGlyphRecord>();
  private readonly failWrites: boolean;

  constructor(options: { failWrites?: boolean } = {}) {
    this.failWrites = options.failWrites ?? false;
  }

  async getMany(keys: readonly string[]): Promise<PersistentGlyphRecord[]> {
    return keys.flatMap((key) => {
      const record = this.records.get(key);
      return record ? [cloneRecord(record)] : [];
    });
  }

  private lastClearMs = Number.NEGATIVE_INFINITY;

  async putMany(records: readonly PersistentGlyphRecord[], token?: PersistentWriteToken): Promise<boolean> {
    if (this.failWrites) throw new Error("simulated quota/private-mode failure");
    if (token && token.startedAtMs <= this.lastClearMs) return false;
    for (const record of records) this.records.set(record.opaqueKey, cloneRecord(record));
    return true;
  }

  async deleteMany(keys: readonly string[]): Promise<void> {
    if (this.failWrites) throw new Error("simulated write failure");
    for (const key of keys) this.records.delete(key);
  }

  async stats(): Promise<PersistentStoreStats> {
    return {
      entries: this.records.size,
      bytes: [...this.records.values()].reduce((sum, record) => sum + record.byteSize, 0),
    };
  }

  async trimLruTo(maxBytes: number, protectedKeys: ReadonlySet<string> = new Set()): Promise<number> {
    if (this.failWrites) throw new Error("simulated write failure");
    let bytes = (await this.stats()).bytes;
    let evicted = 0;
    const candidates = [...this.records.values()]
      .filter((record) => !protectedKeys.has(record.opaqueKey))
      .sort((a, b) => a.lastAccessDay - b.lastAccessDay || a.opaqueKey.localeCompare(b.opaqueKey));
    for (const record of candidates) {
      if (bytes <= maxBytes) break;
      this.records.delete(record.opaqueKey);
      bytes -= record.byteSize;
      evicted += 1;
    }
    return evicted;
  }

  async sweepBefore(lastAccessDayExclusive: number, maxRecords: number): Promise<PersistentSweepResult> {
    if (this.failWrites) throw new Error("simulated write failure");
    let scanned = 0;
    let deleted = 0;
    for (const record of [...this.records.values()].sort((a, b) => a.lastAccessDay - b.lastAccessDay)) {
      if (scanned >= maxRecords) break;
      scanned += 1;
      if (record.lastAccessDay < lastAccessDayExclusive) {
        this.records.delete(record.opaqueKey);
        deleted += 1;
      }
    }
    return { scanned, deleted };
  }

  async clear(clearedAtMs = Date.now()): Promise<void> {
    if (this.failWrites) throw new Error("simulated write failure");
    this.lastClearMs = Math.max(this.lastClearMs, clearedAtMs);
    this.records.clear();
  }
}

function validateKeyInput(input: GlyphRasterKeyInput): void {
  if (!/^[0-9a-f]{64}$/i.test(input.fontSha256)) throw new Error("fontSha256 must be a full SHA-256 hex digest");
  if (!input.region || !input.fontEngineFingerprint || !input.rasterContractId) throw new Error("glyph raster key identity fields must be non-empty");
  for (const value of [input.faceIndex, input.glyphId, input.pointSize26d6, input.dpiX, input.dpiY, input.spread26d6, input.supersample, input.threshold]) {
    if (!Number.isSafeInteger(value) || value < 0) throw new Error("glyph raster numeric fields must be non-negative safe integers");
  }
}

async function validateRecord(record: PersistentGlyphRecord, identity: GlyphRasterIdentity): Promise<boolean> {
  if (
    record.opaqueKey !== identity.opaqueKey ||
    record.schemaNamespace !== identity.schemaNamespace ||
    record.fontEngineFingerprint !== identity.fontEngineFingerprint ||
    record.rasterContractId !== identity.rasterContractId ||
    record.format !== "r8" ||
    record.payloadLength !== record.pixels.byteLength ||
    record.byteSize !== estimateRecordBytes(record.payloadLength)
  ) return false;
  return (await sha256Hex(new Uint8Array(record.pixels))) === record.payloadDigest;
}

function estimateRecordBytes(payloadBytes: number): number {
  return payloadBytes + 256;
}

function dayBucket(nowMs: number): number {
  return Math.floor(nowMs / DAY_MS);
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const subtle = globalThis.crypto?.subtle;
  if (!subtle) throw new Error("Web Crypto SHA-256 is required for persistent glyph identity");
  const digest = await subtle.digest("SHA-256", bytes as BufferSource);
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function cloneRecord(record: PersistentGlyphRecord): PersistentGlyphRecord {
  return {
    ...record,
    identity: { ...record.identity },
    pixels: record.pixels.slice(0),
  };
}
