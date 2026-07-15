export type DecodedImageValue<T> = { value: T; bytes: number };

export type ImageResourceLease<T> = {
  value: T;
  release(): void;
};

type CacheEntry<T> = {
  value: T;
  bytes: number;
  refs: number;
  touched: number;
};

type InflightEntry<T> = {
  promise: Promise<CacheEntry<T>>;
  controller: AbortController;
  waiters: number;
  settled: boolean;
};

export class SessionImageResourceCache<T> {
  private readonly budget: { softBytes: number; hardBytes: number; dispose?: (value: T) => void };
  private readonly entries = new Map<string, CacheEntry<T>>();
  private readonly inflight = new Map<string, InflightEntry<T>>();
  private clock = 0;
  private totalBytes = 0;
  private loads = 0;
  private hits = 0;
  private evictions = 0;

  constructor(budget: { softBytes: number; hardBytes: number; dispose?: (value: T) => void }) {
    if (!Number.isFinite(budget.softBytes) || !Number.isFinite(budget.hardBytes)
      || budget.softBytes < 0 || budget.hardBytes <= 0 || budget.softBytes > budget.hardBytes) {
      throw new Error("invalid image cache byte budget");
    }
    this.budget = budget;
  }

  async acquire(
    key: string,
    loader: (signal: AbortSignal) => Promise<DecodedImageValue<T>>,
    signal: AbortSignal = new AbortController().signal,
  ): Promise<ImageResourceLease<T>> {
    if (signal.aborted) throw abortReason(signal);
    const cached = this.entries.get(key);
    if (cached) {
      this.hits += 1;
      cached.refs += 1;
      cached.touched = ++this.clock;
      return this.lease(cached);
    }
    let pending = this.inflight.get(key);
    if (pending) {
      this.hits += 1;
    } else {
      this.loads += 1;
      const controller = new AbortController();
      const created: InflightEntry<T> = {
        controller,
        waiters: 0,
        settled: false,
        promise: undefined as unknown as Promise<CacheEntry<T>>,
      };
      created.promise = Promise.resolve().then(() => this.load(key, () => loader(controller.signal), created));
      pending = created;
      this.inflight.set(key, pending);
      void pending.promise.finally(() => {
        pending!.settled = true;
        if (this.inflight.get(key) === pending) this.inflight.delete(key);
      }).catch(() => undefined);
    }
    pending.waiters += 1;
    try {
      const entry = await awaitWithAbort(pending.promise, signal);
      entry.refs += 1;
      entry.touched = ++this.clock;
      return this.lease(entry);
    } finally {
      pending.waiters = Math.max(0, pending.waiters - 1);
      if (pending.waiters === 0 && !pending.settled) {
        if (this.inflight.get(key) === pending) this.inflight.delete(key);
        pending.controller.abort(new DOMException("No resource waiters remain", "AbortError"));
      }
      this.trimSoft();
    }
  }

  stats(): { entries: number; bytes: number; pinned: number; loads: number; hits: number; evictions: number } {
    return {
      entries: this.entries.size,
      bytes: this.totalBytes,
      pinned: [...this.entries.values()].filter((entry) => entry.refs > 0).length,
      loads: this.loads,
      hits: this.hits,
      evictions: this.evictions,
    };
  }

  keys(): string[] {
    return [...this.entries.entries()]
      .sort(([, left], [, right]) => left.touched - right.touched)
      .map(([key]) => key);
  }

  clear(): void {
    for (const [key, entry] of this.entries) {
      if (entry.refs === 0) this.remove(key, entry);
    }
  }

  private async load(
    key: string,
    loader: () => Promise<DecodedImageValue<T>>,
    owner: InflightEntry<T>,
  ): Promise<CacheEntry<T>> {
    const decoded = await loader();
    if (this.inflight.get(key) !== owner) {
      this.budget.dispose?.(decoded.value);
      throw abortReason(owner.controller.signal);
    }
    if (!Number.isFinite(decoded.bytes) || !Number.isInteger(decoded.bytes) || decoded.bytes < 0) {
      throw new Error(`invalid decoded image byte size ${decoded.bytes}`);
    }
    if (decoded.bytes > this.budget.hardBytes) {
      throw new Error(`decoded image exceeds hard byte budget: ${decoded.bytes} > ${this.budget.hardBytes}`);
    }
    this.evictUntil(this.budget.hardBytes - decoded.bytes);
    if (this.totalBytes + decoded.bytes > this.budget.hardBytes) {
      throw new Error(`decoded image cache hard byte budget is pinned`);
    }
    const entry: CacheEntry<T> = { value: decoded.value, bytes: decoded.bytes, refs: 0, touched: ++this.clock };
    this.entries.set(key, entry);
    this.totalBytes += decoded.bytes;
    return entry;
  }

  private lease(entry: CacheEntry<T>): ImageResourceLease<T> {
    let released = false;
    return {
      value: entry.value,
      release: () => {
        if (released) return;
        released = true;
        entry.refs = Math.max(0, entry.refs - 1);
        entry.touched = ++this.clock;
        this.trimSoft();
      },
    };
  }

  private trimSoft(): void {
    this.evictUntil(this.budget.softBytes);
  }

  private evictUntil(targetBytes: number): void {
    if (this.totalBytes <= targetBytes) return;
    const candidates = [...this.entries.entries()]
      .filter(([, entry]) => entry.refs === 0)
      .sort(([, left], [, right]) => left.touched - right.touched);
    for (const [key, entry] of candidates) {
      if (this.totalBytes <= targetBytes) break;
      this.remove(key, entry);
    }
  }

  private remove(key: string, entry: CacheEntry<T>): void {
    if (!this.entries.delete(key)) return;
    this.totalBytes -= entry.bytes;
    this.evictions += 1;
    this.budget.dispose?.(entry.value);
  }
}

function awaitWithAbort<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  if (signal.aborted) return Promise.reject(abortReason(signal));
  return new Promise<T>((resolve, reject) => {
    const abort = () => reject(abortReason(signal));
    signal.addEventListener("abort", abort, { once: true });
    void promise.then(resolve, reject).finally(() => signal.removeEventListener("abort", abort));
  });
}

function abortReason(signal: AbortSignal): unknown {
  return signal.reason ?? new DOMException("The operation was aborted", "AbortError");
}
