export type FontRequest = {
  region: string;
  family: string;
};

export type ProvidedFont = {
  bytes: ArrayBuffer | Uint8Array;
};

export type FontContext = {
  signal: AbortSignal;
};

/** Resolves a demanded font family through arbitrary caller-owned logic. */
export interface FontProvider {
  provide(
    request: FontRequest,
    context: FontContext,
  ): Promise<ProvidedFont | null>;
}

export type FontProviderManagerOptions = {
  provider: FontProvider;
  concurrency?: number;
};

export type FontProviderStats = {
  requested: number;
  unique: number;
  loaded: number;
  bytes: number;
  failures: number;
  active: number;
  peakActive: number;
  resolveMs: number;
};

export class FontProviderManager {
  private readonly provider: FontProvider;
  private readonly concurrency: number;
  private readonly counters: FontProviderStats = {
    requested: 0,
    unique: 0,
    loaded: 0,
    bytes: 0,
    failures: 0,
    active: 0,
    peakActive: 0,
    resolveMs: 0,
  };

  constructor(options: FontProviderManagerOptions) {
    if (!options.provider || typeof options.provider.provide !== "function") {
      throw new Error("font provider is required");
    }
    const concurrency = options.concurrency ?? 4;
    if (!Number.isInteger(concurrency) || concurrency <= 0) {
      throw new Error("font concurrency must be a positive integer");
    }
    this.provider = options.provider;
    this.concurrency = concurrency;
  }

  async resolve(
    requests: readonly FontRequest[],
    signal: AbortSignal = new AbortController().signal,
  ): Promise<Map<string, ArrayBuffer>> {
    const started = performance.now();
    const unique = [...new Map(requests.map((request) => [
      `${request.region}\0${request.family}`,
      request,
    ] as const)).values()];
    this.counters.requested += requests.length;
    this.counters.unique += unique.length;
    const values = new Map<string, ArrayBuffer>();
    let cursor = 0;
    const worker = async () => {
      while (cursor < unique.length) {
        if (signal.aborted) throw abortReason(signal);
        const request = unique[cursor++];
        this.counters.active += 1;
        this.counters.peakActive = Math.max(this.counters.peakActive, this.counters.active);
        let provided: ProvidedFont | null;
        try {
          provided = await this.provider.provide(request, { signal });
        } catch (error) {
          this.counters.failures += 1;
          throw error;
        } finally {
          this.counters.active -= 1;
        }
        const bytes = provided?.bytes;
        if (!(bytes instanceof ArrayBuffer) && !(bytes instanceof Uint8Array)) {
          this.counters.failures += 1;
          throw new Error(`font provider returned no bytes for ${request.region}:${request.family}`);
        }
        const snapshot = bytes instanceof ArrayBuffer
          ? bytes.slice(0)
          : Uint8Array.from(bytes).buffer;
        if (snapshot.byteLength === 0) {
          this.counters.failures += 1;
          throw new Error(`font provider returned an empty font for ${request.region}:${request.family}`);
        }
        this.counters.loaded += 1;
        this.counters.bytes += snapshot.byteLength;
        values.set(request.family, snapshot);
      }
    };
    try {
      await Promise.all(Array.from(
        { length: Math.min(this.concurrency, unique.length) },
        worker,
      ));
      return values;
    } finally {
      this.counters.resolveMs += performance.now() - started;
    }
  }

  stats(): FontProviderStats {
    return { ...this.counters };
  }
}

function abortReason(signal: AbortSignal): unknown {
  return signal.reason ?? new DOMException("The operation was aborted", "AbortError");
}
