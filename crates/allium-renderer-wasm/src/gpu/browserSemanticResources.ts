import { SessionImageResourceCache, type ImageResourceLease } from "../cache/sessionImageResourceCache.js";
import type {
  ProvidedResource,
  ResourceDescriptor,
  ResourceProvider,
} from "../resourceProvider.js";

export type BrowserImageSource = {
  source: TexImageSource;
  width: number;
  height: number;
};

export type BrowserSemanticResourceOptions = {
  provider: ResourceProvider;
  concurrency?: number;
  decode?: (blob: Blob) => Promise<TexImageSource>;
  softBytes?: number;
  hardBytes?: number;
};

export type BrowserSemanticResourceSet = {
  sources: Map<string, BrowserImageSource>;
  availability: Map<string, boolean>;
  release(): void;
};

export type ResourceProviderRuntimeStats = {
  requested: number;
  loaded: number;
  failures: number;
  cancellations: number;
  encodedBytes: number;
  resolveMs: number;
  active: number;
  queued: number;
  limit: number;
  peak: number;
};

type LoadedBrowserResource = {
  image: BrowserImageSource;
  available: boolean;
};

/** The provider owns acquisition and persistent caching. This manager only
 * schedules provider calls and retains bounded decoded session resources. */
export class BrowserSemanticResourceManager {
  private readonly provider: ResourceProvider;
  private readonly decode: (blob: Blob) => Promise<TexImageSource>;
  private readonly scheduler: BoundedScheduler;
  private readonly cache: SessionImageResourceCache<LoadedBrowserResource>;
  private readonly providerCounters = {
    requested: 0,
    loaded: 0,
    failures: 0,
    cancellations: 0,
    encodedBytes: 0,
    resolveMs: 0,
  };

  constructor(options: BrowserSemanticResourceOptions) {
    if (!options.provider || typeof options.provider.provide !== "function") {
      throw new Error("semantic resource provider is required");
    }
    this.provider = options.provider;
    this.decode = options.decode ?? ((blob: Blob) => createImageBitmap(blob, {
      premultiplyAlpha: "none",
      colorSpaceConversion: "none",
    }) as Promise<TexImageSource>);
    this.scheduler = new BoundedScheduler(options.concurrency ?? 8);
    this.cache = new SessionImageResourceCache<LoadedBrowserResource>({
      softBytes: options.softBytes ?? 64 * 1024 * 1024,
      hardBytes: options.hardBytes ?? 96 * 1024 * 1024,
      dispose: (value) => closeImageSource(value.image.source),
    });
  }

  async acquire(
    resources: ResourceDescriptor[],
    signal: AbortSignal = new AbortController().signal,
  ): Promise<BrowserSemanticResourceSet> {
    const unique = new Map(resources.map((resource) => [resourceIdentity(resource), resource] as const));
    const leases: Array<{
      identity: string;
      lease: ImageResourceLease<LoadedBrowserResource>;
    }> = [];
    try {
      await Promise.all([...unique].map(async ([identity, resource]) => {
        const cacheIdentity = this.provider.cacheIdentity?.(resource) ?? identity;
        const lease = await this.cache.acquire(cacheIdentity, async (sharedSignal) => {
          const value = await this.load(resource, sharedSignal);
          return { value, bytes: decodedBytes(value.image) };
        }, signal);
        leases.push({ identity, lease });
      }));
    } catch (error) {
      for (const { lease } of leases) lease.release();
      throw error;
    }
    const sources = new Map(
      leases.map(({ identity, lease }) => [identity, lease.value.image] as const),
    );
    const availability = new Map(
      leases.map(({ identity, lease }) => [identity, lease.value.available] as const),
    );
    let released = false;
    return {
      sources,
      availability,
      release: () => {
        if (released) return;
        released = true;
        for (const { lease } of leases) lease.release();
      },
    };
  }

  stats() {
    return {
      ...this.cache.stats(),
      provider: {
        ...this.providerCounters,
        ...this.scheduler.stats(),
      } satisfies ResourceProviderRuntimeStats,
    };
  }

  private async load(
    resource: ResourceDescriptor,
    signal: AbortSignal,
  ): Promise<LoadedBrowserResource> {
    this.providerCounters.requested += 1;
    const started = performance.now();
    try {
      const provided = await this.scheduler.run(
        () => this.provider.provide(resource, { signal }),
        signal,
      );
      if (signal.aborted) throw abortReason(signal);
      if (!provided) throw new Error("provider returned no resource");
      const image = await this.resolveSource(provided);
      this.providerCounters.loaded += 1;
      this.providerCounters.encodedBytes += providedByteLength(provided);
      return { image, available: true };
    } catch (error) {
      if (signal.aborted || isAbortError(error)) {
        this.providerCounters.cancellations += 1;
        throw error;
      }
      this.providerCounters.failures += 1;
      warnMissingResource(resource, error);
      return {
        image: normalizeSource(await this.decode(transparentPngBlob())),
        available: false,
      };
    } finally {
      this.providerCounters.resolveMs += Math.max(0, performance.now() - started);
    }
  }

  private async resolveSource(provided: ProvidedResource): Promise<BrowserImageSource> {
    const source = provided.source;
    if (source instanceof Blob) return normalizeSource(await this.decode(source));
    if (source instanceof ArrayBuffer) return normalizeSource(await this.decode(new Blob([source])));
    if (ArrayBuffer.isView(source)) {
      const view = new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
      const bytes = new Uint8Array(view.byteLength);
      bytes.set(view);
      return normalizeSource(await this.decode(new Blob([bytes])));
    }
    const dimensions = sourceDimensions(source);
    if (!dimensions) throw new Error("direct image source has no finite dimensions");
    return { source, ...dimensions };
  }
}

export function resourceIdentity(resource: Pick<ResourceDescriptor, "id">): string {
  if (!resource.id) throw new Error("semantic resource stable id is required");
  return resource.id;
}

class BoundedScheduler {
  private readonly limit: number;
  private active = 0;
  private peak = 0;
  private readonly queue: Array<() => void> = [];

  constructor(limit: number) {
    if (!Number.isInteger(limit) || limit <= 0) throw new Error("semantic resource concurrency must be a positive integer");
    this.limit = limit;
  }

  run<T>(task: () => Promise<T>, signal: AbortSignal): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const start = () => {
        if (signal.aborted) {
          reject(abortReason(signal));
          this.drain();
          return;
        }
        this.active += 1;
        this.peak = Math.max(this.peak, this.active);
        void task().then(resolve, reject).finally(() => {
          this.active -= 1;
          this.drain();
        });
      };
      this.queue.push(start);
      this.drain();
    });
  }

  stats(): { active: number; queued: number; limit: number; peak: number } {
    return { active: this.active, queued: this.queue.length, limit: this.limit, peak: this.peak };
  }

  private drain(): void {
    while (this.active < this.limit) {
      const start = this.queue.shift();
      if (!start) return;
      start();
    }
  }
}

function sourceDimensions(source: TexImageSource): { width: number; height: number } | null {
  const value = source as unknown as Record<string, unknown>;
  const width = firstPositiveDimension(value.videoWidth, value.naturalWidth, value.displayWidth, value.width);
  const height = firstPositiveDimension(value.videoHeight, value.naturalHeight, value.displayHeight, value.height);
  return width != null && height != null
    ? { width, height }
    : null;
}

function decodedBytes(source: BrowserImageSource): number {
  return Math.max(0, Math.floor(source.width * source.height * 4));
}

function providedByteLength(provided: ProvidedResource): number {
  const source = provided.source;
  if (source instanceof Blob) return source.size;
  if (source instanceof ArrayBuffer) return source.byteLength;
  if (ArrayBuffer.isView(source)) return source.byteLength;
  return 0;
}

function normalizeSource<T extends TexImageSource>(source: T): BrowserImageSource & { source: T } {
  const dimensions = sourceDimensions(source);
  if (!dimensions) throw new Error("decoded image source has no finite dimensions");
  return { source, ...dimensions };
}

function firstPositiveDimension(...values: unknown[]): number | null {
  for (const value of values) {
    const dimension = numericDimension(value);
    if (Number.isFinite(dimension) && dimension > 0) return dimension;
  }
  return null;
}

function numericDimension(value: unknown): number {
  if (value && typeof value === "object") {
    const animated = value as { baseVal?: { value?: unknown }; animVal?: { value?: unknown } };
    const resolved = animated.animVal?.value ?? animated.baseVal?.value;
    if (resolved != null) return Number(resolved);
  }
  return Number(value);
}

function closeImageSource(source: TexImageSource): void {
  const close = (source as { close?: unknown }).close;
  if (typeof close === "function") close.call(source);
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError";
}

function abortReason(signal: AbortSignal): unknown {
  return signal.reason ?? new DOMException("The operation was aborted", "AbortError");
}

const TRANSPARENT_PNG_BASE64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGBgAAAABQABpfZFQAAAAABJRU5ErkJggg==";

function transparentPngBlob(): Blob {
  const bytes = Uint8Array.from(atob(TRANSPARENT_PNG_BASE64), (value) => value.charCodeAt(0));
  return new Blob([bytes], { type: "image/png" });
}

function warnMissingResource(resource: ResourceDescriptor, error: unknown): void {
  const reason = error instanceof Error ? error.message : String(error);
  console.warn(`[sekai-custom-profile-sdk] semantic resource unavailable ${resource.namespace}/${resource.key} role=${resource.role}: ${reason}; using transparent placeholder`);
}
