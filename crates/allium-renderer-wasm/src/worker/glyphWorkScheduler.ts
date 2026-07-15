export type KeyedGlyphJob = { key: string };
export type KeyedGlyphResult = { key: string };

export type GlyphWorkSchedulerStats = {
  dispatches: number;
  dispatchedJobs: number;
  cacheHits: number;
  coalesced: number;
  failures: number;
  pending: number;
  cached: number;
};

type Deferred<T> = {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason?: unknown) => void;
};

export class GlyphWorkScheduler<Job extends KeyedGlyphJob, Result extends KeyedGlyphResult> {
  private readonly dispatch: (jobs: readonly Job[]) => Promise<readonly Result[]>;
  private readonly cache = new Map<string, Result>();
  private readonly pending = new Map<string, Deferred<Result>>();
  private readonly counters = {
    dispatches: 0,
    dispatchedJobs: 0,
    cacheHits: 0,
    coalesced: 0,
    failures: 0,
  };

  constructor(dispatch: (jobs: readonly Job[]) => Promise<readonly Result[]>) {
    this.dispatch = dispatch;
  }

  run(jobs: readonly Job[]): Promise<Result[]> {
    const ordered: Array<Promise<Result>> = [];
    const fresh: Job[] = [];
    const freshKeys = new Set<string>();
    for (const job of jobs) {
      const cached = this.cache.get(job.key);
      if (cached) {
        this.counters.cacheHits += 1;
        ordered.push(Promise.resolve(cached));
        continue;
      }
      const inFlight = this.pending.get(job.key);
      if (inFlight) {
        this.counters.coalesced += 1;
        ordered.push(inFlight.promise);
        continue;
      }
      const deferred = createDeferred<Result>();
      this.pending.set(job.key, deferred);
      ordered.push(deferred.promise);
      if (!freshKeys.has(job.key)) {
        freshKeys.add(job.key);
        fresh.push(job);
      }
    }
    if (fresh.length > 0) void this.dispatchFresh(fresh);
    return Promise.all(ordered);
  }

  get(key: string): Result | undefined {
    const value = this.cache.get(key);
    if (value) this.counters.cacheHits += 1;
    return value;
  }

  prime(results: readonly Result[]): void {
    for (const result of results) {
      if (!this.pending.has(result.key)) this.cache.set(result.key, result);
    }
  }

  delete(key: string): boolean {
    return this.cache.delete(key);
  }

  clear(): void {
    this.cache.clear();
  }

  stats(): GlyphWorkSchedulerStats {
    return {
      ...this.counters,
      pending: this.pending.size,
      cached: this.cache.size,
    };
  }

  private async dispatchFresh(jobs: readonly Job[]): Promise<void> {
    this.counters.dispatches += 1;
    this.counters.dispatchedJobs += jobs.length;
    try {
      const results = await this.dispatch(jobs);
      const byKey = new Map(results.map((result) => [result.key, result]));
      for (const job of jobs) {
        const deferred = this.pending.get(job.key);
        if (!deferred) continue;
        const result = byKey.get(job.key);
        this.pending.delete(job.key);
        if (!result) {
          deferred.reject(new Error(`glyph worker omitted result for ${job.key}`));
          continue;
        }
        this.cache.set(job.key, result);
        deferred.resolve(result);
      }
    } catch (error) {
      this.counters.failures += 1;
      for (const job of jobs) {
        const deferred = this.pending.get(job.key);
        this.pending.delete(job.key);
        deferred?.reject(error);
      }
    }
  }
}

function createDeferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}
