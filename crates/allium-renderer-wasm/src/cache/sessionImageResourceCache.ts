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

export class SessionImageResourceCache<T> {
  private readonly budget: { softBytes: number; hardBytes: number; dispose?: (value: T) => void };
  private readonly entries = new Map<string, CacheEntry<T>>();
  private readonly inflight = new Map<string, Promise<CacheEntry<T>>>();
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

  async acquire(key: string, loader: () => Promise<DecodedImageValue<T>>): Promise<ImageResourceLease<T>> {
    const cached = this.entries.get(key);
    if (cached) {
      this.hits += 1;
      cached.refs += 1;
      cached.touched = ++this.clock;
      return this.lease(cached);
    }
    const pending = this.inflight.get(key);
    if (pending) {
      this.hits += 1;
      const entry = await pending;
      entry.refs += 1;
      entry.touched = ++this.clock;
      return this.lease(entry);
    }
    this.loads += 1;
    const job = this.load(key, loader);
    this.inflight.set(key, job);
    try {
      return this.lease(await job);
    } finally {
      this.inflight.delete(key);
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

  private async load(key: string, loader: () => Promise<DecodedImageValue<T>>): Promise<CacheEntry<T>> {
    const decoded = await loader();
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
    const entry: CacheEntry<T> = { value: decoded.value, bytes: decoded.bytes, refs: 1, touched: ++this.clock };
    this.entries.set(key, entry);
    this.totalBytes += decoded.bytes;
    this.trimSoft();
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
