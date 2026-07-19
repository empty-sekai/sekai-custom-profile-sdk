import {
  GLYPH_CACHE_SCHEMA,
  GlyphPersistentCache,
  type GlyphRecordStore,
  type PersistentGlyphRecord,
  type PersistentStoreStats,
  type PersistentSweepResult,
  type PersistentWriteToken,
} from "./glyphPersistentCache.js";

const DATABASE_NAME = "sekai-custom-profile-sdk-glyph-cache-v1";
const DATABASE_VERSION = 1;
const GLYPH_STORE = "glyphs";
const META_STORE = "meta";
const TOTALS_KEY = "totals";

type TotalsRecord = { key: typeof TOTALS_KEY; entries: number; bytes: number; lastClearMs: number };

export class IndexedDbGlyphRecordStore implements GlyphRecordStore {
  private databasePromise: Promise<IDBDatabase> | null = null;

  constructor(private readonly databaseName = DATABASE_NAME) {}

  async getMany(keys: readonly string[]): Promise<PersistentGlyphRecord[]> {
    if (keys.length === 0) return [];
    const database = await this.database();
    const transaction = database.transaction(GLYPH_STORE, "readonly");
    const store = transaction.objectStore(GLYPH_STORE);
    const records = await Promise.all(keys.map((key) => request(store.get(key))));
    await transactionDone(transaction);
    return records.filter((record): record is PersistentGlyphRecord => record != null);
  }

  async putMany(records: readonly PersistentGlyphRecord[], token?: PersistentWriteToken): Promise<boolean> {
    if (records.length === 0) return true;
    const database = await this.database();
    const transaction = database.transaction([GLYPH_STORE, META_STORE], "readwrite", { durability: "relaxed" });
    const glyphs = transaction.objectStore(GLYPH_STORE);
    const meta = transaction.objectStore(META_STORE);
    const totals = await readTotals(meta);
    if (token && token.startedAtMs <= totals.lastClearMs) {
      transaction.abort();
      try { await transactionDone(transaction); } catch { /* intentional abort */ }
      return false;
    }
    for (const record of records) {
      const existing = await request<PersistentGlyphRecord | undefined>(glyphs.get(record.opaqueKey));
      if (existing) {
        totals.bytes -= existing.byteSize;
      } else {
        totals.entries += 1;
      }
      totals.bytes += record.byteSize;
      glyphs.put(record);
    }
    meta.put(totals);
    await transactionDone(transaction);
    return true;
  }

  async deleteMany(keys: readonly string[]): Promise<void> {
    if (keys.length === 0) return;
    const database = await this.database();
    const transaction = database.transaction([GLYPH_STORE, META_STORE], "readwrite", { durability: "relaxed" });
    const glyphs = transaction.objectStore(GLYPH_STORE);
    const meta = transaction.objectStore(META_STORE);
    const totals = await readTotals(meta);
    for (const key of keys) {
      const existing = await request<PersistentGlyphRecord | undefined>(glyphs.get(key));
      if (!existing) continue;
      glyphs.delete(key);
      totals.entries -= 1;
      totals.bytes -= existing.byteSize;
    }
    meta.put(normalizeTotals(totals));
    await transactionDone(transaction);
  }

  async stats(): Promise<PersistentStoreStats> {
    const database = await this.database();
    const transaction = database.transaction(META_STORE, "readonly");
    const totals = await readTotals(transaction.objectStore(META_STORE));
    await transactionDone(transaction);
    return { entries: totals.entries, bytes: totals.bytes };
  }

  async trimLruTo(maxBytes: number, protectedKeys: ReadonlySet<string> = new Set()): Promise<number> {
    const database = await this.database();
    const transaction = database.transaction([GLYPH_STORE, META_STORE], "readwrite", { durability: "relaxed" });
    const glyphs = transaction.objectStore(GLYPH_STORE);
    const meta = transaction.objectStore(META_STORE);
    const totals = await readTotals(meta);
    let evicted = 0;
    if (totals.bytes > maxBytes) {
      const index = glyphs.index("lastAccessDay");
      await iterateCursor(index.openCursor(), (cursor) => {
        if (totals.bytes <= maxBytes) return false;
        const record = cursor.value as PersistentGlyphRecord;
        if (!protectedKeys.has(record.opaqueKey)) {
          cursor.delete();
          totals.entries -= 1;
          totals.bytes -= record.byteSize;
          evicted += 1;
        }
        return true;
      });
    }
    meta.put(normalizeTotals(totals));
    await transactionDone(transaction);
    return evicted;
  }

  async sweepBefore(lastAccessDayExclusive: number, maxRecords: number): Promise<PersistentSweepResult> {
    const database = await this.database();
    const transaction = database.transaction([GLYPH_STORE, META_STORE], "readwrite", { durability: "relaxed" });
    const glyphs = transaction.objectStore(GLYPH_STORE);
    const meta = transaction.objectStore(META_STORE);
    const totals = await readTotals(meta);
    let scanned = 0;
    let deleted = 0;
    await iterateCursor(glyphs.index("lastAccessDay").openCursor(), (cursor) => {
      if (scanned >= maxRecords) return false;
      scanned += 1;
      const record = cursor.value as PersistentGlyphRecord;
      if (record.lastAccessDay >= lastAccessDayExclusive) return false;
      cursor.delete();
      totals.entries -= 1;
      totals.bytes -= record.byteSize;
      deleted += 1;
      return true;
    });
    meta.put(normalizeTotals(totals));
    await transactionDone(transaction);
    return { scanned, deleted };
  }

  async clear(clearedAtMs = Date.now()): Promise<void> {
    const database = await this.database();
    const transaction = database.transaction([GLYPH_STORE, META_STORE], "readwrite");
    transaction.objectStore(GLYPH_STORE).clear();
    const meta = transaction.objectStore(META_STORE);
    const totals = await readTotals(meta);
    meta.put({ key: TOTALS_KEY, entries: 0, bytes: 0, lastClearMs: Math.max(totals.lastClearMs, clearedAtMs) } satisfies TotalsRecord);
    await transactionDone(transaction);
  }

  close(): void {
    void this.databasePromise?.then((database) => database.close());
    this.databasePromise = null;
  }

  private database(): Promise<IDBDatabase> {
    if (typeof indexedDB === "undefined") return Promise.reject(new Error("IndexedDB is unavailable"));
    this.databasePromise ??= new Promise((resolve, reject) => {
      const open = indexedDB.open(this.databaseName, DATABASE_VERSION);
      open.onupgradeneeded = () => {
        const database = open.result;
        const glyphs = database.createObjectStore(GLYPH_STORE, { keyPath: "opaqueKey" });
        glyphs.createIndex("lastAccessDay", "lastAccessDay");
        database.createObjectStore(META_STORE, { keyPath: "key" });
      };
      open.onerror = () => reject(open.error ?? new Error("open IndexedDB glyph cache failed"));
      open.onblocked = () => reject(new Error("IndexedDB glyph cache upgrade was blocked"));
      open.onsuccess = () => {
        open.result.onversionchange = () => open.result.close();
        resolve(open.result);
      };
    });
    return this.databasePromise;
  }
}

export function createOriginGlyphPersistentCache(options: {
  databaseName?: string;
  softBytes?: number;
  hardBytes?: number;
  ttlDays?: number;
} = {}): GlyphPersistentCache {
  return new GlyphPersistentCache({
    mode: "origin",
    store: new IndexedDbGlyphRecordStore(options.databaseName),
    softBytes: options.softBytes,
    hardBytes: options.hardBytes,
    ttlDays: options.ttlDays,
  });
}

export const GLYPH_PERSISTENT_CACHE_INFO = Object.freeze({
  databaseName: DATABASE_NAME,
  databaseVersion: DATABASE_VERSION,
  schemaNamespace: GLYPH_CACHE_SCHEMA,
});

function readTotals(store: IDBObjectStore): Promise<TotalsRecord> {
  return request<TotalsRecord | undefined>(store.get(TOTALS_KEY)).then(
    (totals) => totals
      ? { ...totals, lastClearMs: Number.isFinite(totals.lastClearMs) ? totals.lastClearMs : Number.NEGATIVE_INFINITY }
      : { key: TOTALS_KEY, entries: 0, bytes: 0, lastClearMs: Number.NEGATIVE_INFINITY },
  );
}

function normalizeTotals(totals: TotalsRecord): TotalsRecord {
  totals.entries = Math.max(0, totals.entries);
  totals.bytes = Math.max(0, totals.bytes);
  return totals;
}

function request<T>(value: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    value.onsuccess = () => resolve(value.result);
    value.onerror = () => reject(value.error ?? new Error("IndexedDB request failed"));
  });
}

function transactionDone(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => reject(transaction.error ?? new Error("IndexedDB transaction failed"));
    transaction.onabort = () => reject(transaction.error ?? new Error("IndexedDB transaction aborted"));
  });
}

function iterateCursor(
  cursorRequest: IDBRequest<IDBCursorWithValue | null>,
  visit: (cursor: IDBCursorWithValue) => boolean,
): Promise<void> {
  return new Promise((resolve, reject) => {
    cursorRequest.onerror = () => reject(cursorRequest.error ?? new Error("IndexedDB cursor failed"));
    cursorRequest.onsuccess = () => {
      const cursor = cursorRequest.result;
      if (!cursor || !visit(cursor)) {
        resolve();
        return;
      }
      cursor.continue();
    };
  });
}
