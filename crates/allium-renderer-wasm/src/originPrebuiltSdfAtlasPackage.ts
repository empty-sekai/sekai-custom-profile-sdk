import {
  isValidPrebuiltSdfAtlasManifest,
  type PrebuiltSdfAtlasManifest,
  type PrebuiltSdfAtlasProvider,
} from "./prebuiltSdfAtlas.js";

const DATABASE_NAME = "sekai-custom-profile-sdk-prebuilt-atlas-v1";
const DATABASE_VERSION = 1;
const ENTRY_STORE = "entries";

export type PrebuiltSdfAtlasInstallProgress = {
  family: string;
  completedPages: number;
  totalPages: number;
  storedBytes: number;
  totalBytes: number;
};

export type PrebuiltSdfAtlasPackageStatus = {
  namespace: string;
  storageAvailable: boolean;
  installedFamilies: string[];
  installedPages: number;
  storedBytes: number;
};

export type PrebuiltSdfAtlasInstallOptions = {
  concurrency?: number;
  onProgress?: (progress: PrebuiltSdfAtlasInstallProgress) => void;
  requestPersistence?: boolean;
  signal?: AbortSignal;
};

export type OriginPrebuiltSdfAtlasPackage = {
  readonly namespace: string;
  readonly provider: PrebuiltSdfAtlasProvider;
  install(families: readonly string[], options?: PrebuiltSdfAtlasInstallOptions): Promise<PrebuiltSdfAtlasPackageStatus>;
  remove(families?: readonly string[]): Promise<void>;
  status(families: readonly string[]): Promise<PrebuiltSdfAtlasPackageStatus>;
  close(): void;
};

export type PrebuiltSdfAtlasStorageErrorCode =
  | "PREBUILT_ATLAS_STORAGE_UNAVAILABLE"
  | "PREBUILT_ATLAS_INVALID_MANIFEST"
  | "PREBUILT_ATLAS_QUOTA"
  | "PREBUILT_ATLAS_PAGE_HASH";

export class PrebuiltSdfAtlasStorageError extends Error {
  constructor(readonly code: PrebuiltSdfAtlasStorageErrorCode, message: string) {
    super(message);
    this.name = "PrebuiltSdfAtlasStorageError";
  }
}

type ManifestRecord = {
  key: string;
  kind: "manifest";
  namespace: string;
  family: string;
  manifest: PrebuiltSdfAtlasManifest;
  installedPages: number;
  storedBytes: number;
  installedAtMs: number;
};

type PageRecord = {
  key: string;
  kind: "page";
  namespace: string;
  family: string;
  file: string;
  sha256: string;
  bytes: ArrayBuffer;
};

type AtlasRecord = ManifestRecord | PageRecord;

export function createOriginPrebuiltSdfAtlasPackage(options: {
  namespace: string;
  source: PrebuiltSdfAtlasProvider;
  databaseName?: string;
}): OriginPrebuiltSdfAtlasPackage {
  if (!options.namespace.trim()) throw new TypeError("prebuilt atlas namespace must be non-empty");
  const storage = new OriginPrebuiltAtlasStorage(options.databaseName ?? DATABASE_NAME);
  const manifestCache = new Map<string, PrebuiltSdfAtlasManifest>();
  const namespace = options.namespace.trim();

  const installedManifest = async (family: string): Promise<PrebuiltSdfAtlasManifest | null> => {
    const cached = manifestCache.get(family);
    if (cached) return cached;
    const record = await storage.getManifest(namespace, family);
    if (!record || !isValidPrebuiltSdfAtlasManifest(record.manifest, family)) return null;
    manifestCache.set(family, record.manifest);
    return record.manifest;
  };

  const provider: PrebuiltSdfAtlasProvider = {
    async manifest(family, { signal }) {
      if (signal.aborted) throw abortReason(signal);
      if (!storage.available) return null;
      try {
        return await installedManifest(family);
      } catch {
        return null;
      }
    },
    async page(family, file, { signal }) {
      if (signal.aborted) throw abortReason(signal);
      const manifest = await installedManifest(family);
      const descriptor = manifest?.pages.find((page) => page.file === file);
      if (!descriptor) throw new Error(`prebuilt atlas page is not installed: ${family}/${file}`);
      const record = await storage.getPage(namespace, family, file, descriptor.file_sha256);
      if (!record) {
        manifestCache.delete(family);
        await storage.deleteManifest(namespace, family).catch(() => undefined);
        throw new Error(`prebuilt atlas page is missing: ${family}/${file}`);
      }
      return record.bytes.slice(0);
    },
  };

  return {
    namespace,
    provider,
    async status(families) {
      if (!storage.available) return emptyStatus(namespace, false);
      const records = await Promise.all(uniqueFamilies(families).map((family) => storage.getManifest(namespace, family)));
      return records.reduce<PrebuiltSdfAtlasPackageStatus>((status, record) => {
        if (!record || !isValidPrebuiltSdfAtlasManifest(record.manifest, record.family)) return status;
        status.installedFamilies.push(record.family);
        status.installedPages += record.installedPages;
        status.storedBytes += record.storedBytes;
        return status;
      }, emptyStatus(namespace, true));
    },
    async install(families, installOptions = {}) {
      if (!storage.available) {
        throw new PrebuiltSdfAtlasStorageError(
          "PREBUILT_ATLAS_STORAGE_UNAVAILABLE",
          "IndexedDB is unavailable for the prebuilt atlas package",
        );
      }
      const signal = installOptions.signal ?? new AbortController().signal;
      const requested = uniqueFamilies(families);
      const current = await this.status(requested);
      const installed = new Set(current.installedFamilies);
      const missing = requested.filter((family) => !installed.has(family));
      if (missing.length === 0) return current;

      const manifests = new Map<string, PrebuiltSdfAtlasManifest>();
      for (const family of missing) {
        if (signal.aborted) throw abortReason(signal);
        const manifest = await options.source.manifest(family, { signal });
        if (!manifest || !isValidPrebuiltSdfAtlasManifest(manifest, family)) {
          throw new PrebuiltSdfAtlasStorageError(
            "PREBUILT_ATLAS_INVALID_MANIFEST",
            `prebuilt atlas manifest is invalid: ${family}`,
          );
        }
        manifests.set(family, manifest);
      }

      const totalPages = [...manifests.values()].reduce((sum, manifest) => sum + manifest.pages.length, 0);
      const totalBytes = [...manifests.values()].flatMap((manifest) => manifest.pages)
        .reduce((sum, page) => sum + 64 + page.width * page.height, 0);
      await assertStorageQuota(totalBytes);
      if (installOptions.requestPersistence) await requestOriginPersistence();

      await storage.remove(namespace, missing);
      manifestCache.clear();
      const tasks = [...manifests].flatMap(([family, manifest]) => manifest.pages.map((page) => ({ family, page })));
      const concurrency = boundedConcurrency(installOptions.concurrency ?? 4);
      let cursor = 0;
      let completedPages = 0;
      let storedBytes = 0;
      const familyBytes = new Map<string, number>();
      try {
        await Promise.all(Array.from({ length: Math.min(concurrency, tasks.length) }, async () => {
          while (cursor < tasks.length) {
            const task = tasks[cursor++];
            if (signal.aborted) throw abortReason(signal);
            const buffer = await options.source.page(task.family, task.page.file, { signal });
            await assertPageHash(buffer, task.page.file_sha256, task.family, task.page.file);
            await storage.putPage({
              key: pageKey(namespace, task.family, task.page.file, task.page.file_sha256),
              kind: "page",
              namespace,
              family: task.family,
              file: task.page.file,
              sha256: task.page.file_sha256,
              bytes: buffer,
            });
            completedPages += 1;
            storedBytes += buffer.byteLength;
            familyBytes.set(task.family, (familyBytes.get(task.family) ?? 0) + buffer.byteLength);
            installOptions.onProgress?.({
              family: task.family,
              completedPages,
              totalPages,
              storedBytes,
              totalBytes,
            });
          }
        }));
        await storage.putManifests([...manifests].map(([family, manifest]) => ({
          key: manifestKey(namespace, family),
          kind: "manifest",
          namespace,
          family,
          manifest,
          installedPages: manifest.pages.length,
          storedBytes: familyBytes.get(family) ?? 0,
          installedAtMs: Date.now(),
        })));
      } catch (error) {
        await storage.remove(namespace, missing).catch(() => undefined);
        throw error;
      }
      return this.status(requested);
    },
    async remove(families) {
      manifestCache.clear();
      if (!storage.available) return;
      await storage.remove(namespace, families == null ? undefined : uniqueFamilies(families));
    },
    close() {
      manifestCache.clear();
      storage.close();
    },
  };
}

class OriginPrebuiltAtlasStorage {
  private databasePromise: Promise<IDBDatabase> | null = null;

  constructor(private readonly databaseName: string) {}

  get available(): boolean { return typeof indexedDB !== "undefined"; }

  async getManifest(namespace: string, family: string): Promise<ManifestRecord | null> {
    const record = await this.get(manifestKey(namespace, family));
    return record?.kind === "manifest" ? record : null;
  }

  async getPage(namespace: string, family: string, file: string, sha256: string): Promise<PageRecord | null> {
    const record = await this.get(pageKey(namespace, family, file, sha256));
    return record?.kind === "page" ? record : null;
  }

  async putPage(record: PageRecord): Promise<void> {
    const database = await this.database();
    const transaction = database.transaction(ENTRY_STORE, "readwrite", { durability: "relaxed" });
    transaction.objectStore(ENTRY_STORE).put(record);
    await transactionDone(transaction);
  }

  async putManifests(records: readonly ManifestRecord[]): Promise<void> {
    const database = await this.database();
    const transaction = database.transaction(ENTRY_STORE, "readwrite");
    const store = transaction.objectStore(ENTRY_STORE);
    for (const record of records) store.put(record);
    await transactionDone(transaction);
  }

  async deleteManifest(namespace: string, family: string): Promise<void> {
    const database = await this.database();
    const transaction = database.transaction(ENTRY_STORE, "readwrite");
    transaction.objectStore(ENTRY_STORE).delete(manifestKey(namespace, family));
    await transactionDone(transaction);
  }

  async remove(namespace: string, families?: readonly string[]): Promise<void> {
    const database = await this.database();
    const transaction = database.transaction(ENTRY_STORE, "readwrite");
    const store = transaction.objectStore(ENTRY_STORE);
    const prefixes = (families?.length ? families : [""]).map((family) => familyPrefix(namespace, family));
    await deletePrefixes(store, prefixes);
    await transactionDone(transaction);
  }

  close(): void {
    void this.databasePromise?.then((database) => database.close());
    this.databasePromise = null;
  }

  private async get(key: string): Promise<AtlasRecord | null> {
    const database = await this.database();
    const transaction = database.transaction(ENTRY_STORE, "readonly");
    const record = await request<AtlasRecord | undefined>(transaction.objectStore(ENTRY_STORE).get(key));
    await transactionDone(transaction);
    return record ?? null;
  }

  private database(): Promise<IDBDatabase> {
    if (!this.available) return Promise.reject(new Error("IndexedDB is unavailable"));
    this.databasePromise ??= new Promise((resolve, reject) => {
      const open = indexedDB.open(this.databaseName, DATABASE_VERSION);
      open.onupgradeneeded = () => {
        if (!open.result.objectStoreNames.contains(ENTRY_STORE)) {
          open.result.createObjectStore(ENTRY_STORE, { keyPath: "key" });
        }
      };
      open.onerror = () => reject(open.error ?? new Error("open IndexedDB prebuilt atlas storage failed"));
      open.onblocked = () => reject(new Error("IndexedDB prebuilt atlas storage upgrade was blocked"));
      open.onsuccess = () => {
        open.result.onversionchange = () => open.result.close();
        resolve(open.result);
      };
    });
    return this.databasePromise;
  }
}

function emptyStatus(namespace: string, storageAvailable: boolean): PrebuiltSdfAtlasPackageStatus {
  return { namespace, storageAvailable, installedFamilies: [], installedPages: 0, storedBytes: 0 };
}

function uniqueFamilies(families: readonly string[]): string[] {
  return [...new Set(families.map((family) => family.trim()).filter(Boolean))];
}

function familyPrefix(namespace: string, family: string): string {
  return family ? `${namespace}\0${family}\0` : `${namespace}\0`;
}

function manifestKey(namespace: string, family: string): string {
  return `${familyPrefix(namespace, family)}manifest`;
}

function pageKey(namespace: string, family: string, file: string, sha256: string): string {
  return `${familyPrefix(namespace, family)}page\0${file}\0${sha256}`;
}

function boundedConcurrency(value: number): number {
  if (!Number.isInteger(value) || value <= 0) throw new TypeError("prebuilt atlas install concurrency must be positive");
  return Math.min(8, value);
}

async function assertStorageQuota(requiredBytes: number): Promise<void> {
  if (typeof navigator === "undefined" || !navigator.storage?.estimate) return;
  const estimate = await navigator.storage.estimate();
  if (estimate.quota == null || estimate.usage == null) return;
  const available = Math.max(0, estimate.quota - estimate.usage);
  if (requiredBytes > available) {
    throw new PrebuiltSdfAtlasStorageError(
      "PREBUILT_ATLAS_QUOTA",
      `prebuilt atlas requires ${requiredBytes} bytes but only ${available} bytes are available`,
    );
  }
}

async function requestOriginPersistence(): Promise<void> {
  if (typeof navigator === "undefined" || !navigator.storage?.persist) return;
  await navigator.storage.persist();
}

async function assertPageHash(buffer: ArrayBuffer, expected: string, family: string, file: string): Promise<void> {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", buffer.slice(0)));
  const actual = [...digest].map((value) => value.toString(16).padStart(2, "0")).join("");
  if (actual !== expected.toLowerCase()) {
    throw new PrebuiltSdfAtlasStorageError(
      "PREBUILT_ATLAS_PAGE_HASH",
      `prebuilt atlas page hash mismatch: ${family}/${file}`,
    );
  }
}

function abortReason(signal: AbortSignal): unknown {
  return signal.reason ?? new DOMException("The operation was aborted", "AbortError");
}

function request<T>(value: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    value.onsuccess = () => resolve(value.result);
    value.onerror = () => reject(value.error ?? new Error("IndexedDB prebuilt atlas request failed"));
  });
}

function transactionDone(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onerror = () => reject(transaction.error ?? new Error("IndexedDB prebuilt atlas transaction failed"));
    transaction.onabort = () => reject(transaction.error ?? new Error("IndexedDB prebuilt atlas transaction aborted"));
  });
}

function deletePrefixes(store: IDBObjectStore, prefixes: readonly string[]): Promise<void> {
  return new Promise((resolve, reject) => {
    const cursor = store.openKeyCursor();
    cursor.onerror = () => reject(cursor.error ?? new Error("IndexedDB prebuilt atlas cursor failed"));
    cursor.onsuccess = () => {
      const value = cursor.result;
      if (!value) {
        resolve();
        return;
      }
      const key = String(value.primaryKey);
      if (prefixes.some((prefix) => key.startsWith(prefix))) store.delete(value.primaryKey);
      value.continue();
    };
  });
}
