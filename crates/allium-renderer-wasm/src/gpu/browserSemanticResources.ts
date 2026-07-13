import { SessionImageResourceCache, type ImageResourceLease } from "../cache/sessionImageResourceCache.js";
import type { SemanticResourceKey } from "./semanticCommandPlanner.js";

type DecodedSource = TexImageSource & { width: number; height: number; close?: () => void };

export type BrowserSemanticResourceOptions<T extends DecodedSource, B = Blob> = {
  environmentId: string;
  resolveUrl: (resource: SemanticResourceKey) => string;
  fetchBlob?: (url: string, resource: SemanticResourceKey) => Promise<B>;
  decode?: (blob: B) => Promise<T>;
  softBytes?: number;
  hardBytes?: number;
};

export type BrowserSemanticResourceSet<T extends DecodedSource> = {
  sources: Map<string, T>;
  release(): void;
};

/** Browser HTTP cache persists encoded/versioned assets; this bounded session
 * cache retains decoded pixels. GPU textures remain context-local and can be
 * rebound from these decoded sources after context restoration. */
export class BrowserSemanticResourceManager<T extends DecodedSource = ImageBitmap, B = Blob> {
  private readonly environmentId: string;
  private readonly resolveUrl: (resource: SemanticResourceKey) => string;
  private readonly fetchBlob: (url: string, resource: SemanticResourceKey) => Promise<B>;
  private readonly decode: (blob: B) => Promise<T>;
  private readonly cache: SessionImageResourceCache<T>;

  constructor(options: BrowserSemanticResourceOptions<T, B>) {
    if (!options.environmentId) throw new Error("semantic resource environment id is required");
    this.environmentId = options.environmentId;
    this.resolveUrl = options.resolveUrl;
    this.fetchBlob = options.fetchBlob ?? (defaultFetchBlob as (url: string, resource: SemanticResourceKey) => Promise<B>);
    this.decode = options.decode ?? ((blob: B) => createImageBitmap(blob as unknown as Blob, {
      premultiplyAlpha: "none",
      colorSpaceConversion: "none",
    }) as unknown as Promise<T>);
    this.cache = new SessionImageResourceCache<T>({
      softBytes: options.softBytes ?? 64 * 1024 * 1024,
      hardBytes: options.hardBytes ?? 96 * 1024 * 1024,
      dispose: (value) => value.close?.(),
    });
  }

  async acquire(resources: SemanticResourceKey[]): Promise<BrowserSemanticResourceSet<T>> {
    const unique = new Map(resources.map((resource) => [resourceIdentity(resource), resource] as const));
    const leases: Array<{ identity: string; lease: ImageResourceLease<T> }> = [];
    try {
      await Promise.all([...unique].map(async ([identity, resource]) => {
        const persistentIdentity = `${this.environmentId}\0${identity}`;
        const lease = await this.cache.acquire(persistentIdentity, async () => {
          const blob = await this.fetchBlob(this.resolveUrl(resource), resource);
          const value = await this.decode(blob);
          return { value, bytes: Math.max(0, value.width * value.height * 4) };
        });
        leases.push({ identity, lease });
      }));
    } catch (error) {
      for (const { lease } of leases) lease.release();
      throw error;
    }
    const sources = new Map(leases.map(({ identity, lease }) => [identity, lease.value] as const));
    let released = false;
    return {
      sources,
      release: () => {
        if (released) return;
        released = true;
        for (const { lease } of leases) lease.release();
      },
    };
  }

  stats() {
    return this.cache.stats();
  }
}

export function resourceIdentity(resource: SemanticResourceKey): string {
  return `${resource.namespace}\0${resource.key}`;
}

export function defaultSemanticResourceUrl(resource: SemanticResourceKey, region: string): string {
  const encoded = resource.key.split("/").map(encodeURIComponent).join("/");
  if (resource.namespace === "static") {
    return `https://cdn.emptysekai.com/renderer-static/v0.2/${encoded}.png`;
  }
  if (resource.namespace === "assets") {
    return `https://cdn.emptysekai.com/assets/${encodeURIComponent(region)}/${encoded}.png`;
  }
  throw new Error(`unsupported semantic resource namespace ${resource.namespace}`);
}

const LEGACY_SEMANTIC_CACHE = "allium-renderer-semantic-assets-v1";
const IMMUTABLE_STATIC_CACHE = "allium-renderer-static-assets-v2";
let legacyCleanup: Promise<void> | null = null;

async function defaultFetchBlob(url: string, resource: SemanticResourceKey): Promise<Blob> {
  await cleanupLegacySemanticCache();
  const persistent = isImmutableRendererStatic(url, resource) && typeof caches !== "undefined"
    ? await caches.open(IMMUTABLE_STATIC_CACHE).catch(() => null)
    : null;
  const cached = persistent ? await persistent.match(url).catch(() => undefined) : undefined;
  if (cached?.ok) return cached.blob();
  // Versioned renderer-static may additionally live in CacheStorage. Game
  // assets remain under the browser HTTP cache so server freshness headers
  // and conditional revalidation stay authoritative.
  const response = await fetch(url, { cache: "default" });
  if (!response.ok) throw new Error(`semantic resource fetch failed ${response.status} ${url}`);
  if (persistent) await persistent.put(url, response.clone()).catch(() => undefined);
  return response.blob();
}

export async function clearPersistentSemanticResourceCache(): Promise<boolean> {
  if (typeof caches === "undefined") return false;
  const removed = await Promise.all([
    caches.delete(IMMUTABLE_STATIC_CACHE),
    caches.delete(LEGACY_SEMANTIC_CACHE),
  ]);
  return removed.some(Boolean);
}

function cleanupLegacySemanticCache(): Promise<void> {
  if (typeof caches === "undefined") return Promise.resolve();
  legacyCleanup ??= caches.delete(LEGACY_SEMANTIC_CACHE).then(() => undefined, () => undefined);
  return legacyCleanup;
}

function isImmutableRendererStatic(url: string, resource: SemanticResourceKey): boolean {
  if (resource.namespace !== "static") return false;
  try {
    const base = typeof location === "undefined" ? "https://renderer.invalid/" : location.href;
    return new URL(url, base).pathname.startsWith("/renderer-static/v0.2/");
  } catch {
    return false;
  }
}
