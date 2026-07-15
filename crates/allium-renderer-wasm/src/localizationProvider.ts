export type LocalizationRequest = {
  region: string;
  locale: string;
  key: string;
};

export type LocalizationContext = {
  signal: AbortSignal;
};

/** Resolves renderer-owned UI text through arbitrary caller-owned logic. */
export interface LocalizationProvider {
  provide(
    request: LocalizationRequest,
    context: LocalizationContext,
  ): Promise<string | null>;
}

export type LocalizationProviderManagerOptions = {
  provider: LocalizationProvider;
  concurrency?: number;
};

export type LocalizationProviderStats = {
  requested: number;
  unique: number;
  resolved: number;
  failures: number;
  active: number;
  peakActive: number;
  resolveMs: number;
};

export class LocalizationProviderManager {
  private readonly provider: LocalizationProvider;
  private readonly concurrency: number;
  private readonly counters: LocalizationProviderStats = {
    requested: 0,
    unique: 0,
    resolved: 0,
    failures: 0,
    active: 0,
    peakActive: 0,
    resolveMs: 0,
  };

  constructor(options: LocalizationProviderManagerOptions) {
    if (!options.provider || typeof options.provider.provide !== "function") {
      throw new Error("localization provider is required");
    }
    const concurrency = options.concurrency ?? 8;
    if (!Number.isInteger(concurrency) || concurrency <= 0) {
      throw new Error("localization concurrency must be a positive integer");
    }
    this.provider = options.provider;
    this.concurrency = concurrency;
  }

  async resolve(
    requests: readonly LocalizationRequest[],
    signal: AbortSignal = new AbortController().signal,
  ): Promise<Record<string, string>> {
    const started = performance.now();
    const unique = [...new Map(requests.map((request) => [
      `${request.region}\0${request.locale}\0${request.key}`,
      request,
    ] as const)).values()];
    this.counters.requested += requests.length;
    this.counters.unique += unique.length;
    const values = new Map<string, string>();
    let cursor = 0;
    const worker = async () => {
      while (cursor < unique.length) {
        if (signal.aborted) throw abortReason(signal);
        const request = unique[cursor++];
        this.counters.active += 1;
        this.counters.peakActive = Math.max(this.counters.peakActive, this.counters.active);
        let value: string | null;
        try {
          value = await this.provider.provide(request, { signal });
        } catch (error) {
          this.counters.failures += 1;
          throw error;
        } finally {
          this.counters.active -= 1;
        }
        if (typeof value !== "string") {
          this.counters.failures += 1;
          throw new Error(`localization provider returned no text for ${request.locale}:${request.key}`);
        }
        this.counters.resolved += 1;
        values.set(request.key, value);
      }
    };
    try {
      await Promise.all(Array.from(
        { length: Math.min(this.concurrency, unique.length) },
        worker,
      ));
      return Object.fromEntries(values);
    } finally {
      this.counters.resolveMs += performance.now() - started;
    }
  }

  stats(): LocalizationProviderStats {
    return { ...this.counters };
  }
}

function abortReason(signal: AbortSignal): unknown {
  return signal.reason ?? new DOMException("The operation was aborted", "AbortError");
}
