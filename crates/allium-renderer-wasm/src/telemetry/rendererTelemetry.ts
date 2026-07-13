export type TelemetryLevel = "off" | "summary" | "trace";

const MAX_TRACE_SAMPLES = 240;

export type RendererTelemetrySample = {
  frameId: number;
  gpuEpoch: number;
  timingsMs: Record<string, number | null>;
  workload: Record<string, number>;
  recovery: { contextLosses: number; restoreMs: number | null; [key: string]: number | null };
  [key: string]: unknown;
};

export type TelemetryDistribution = {
  p50: number;
  p95: number;
  max: number;
  samples: number;
};

export type RendererTelemetrySnapshot = {
  schemaVersion: 1;
  level: Exclude<TelemetryLevel, "off">;
  last: RendererTelemetrySample;
  summary: Record<string, TelemetryDistribution>;
  recordedFrames: number;
  droppedFrames: number;
  samples?: RendererTelemetrySample[];
};

export class RendererTelemetry {
  private readonly level: TelemetryLevel;
  private readonly maxSamples: number;
  private readonly summaryEvery: number;
  private readonly samples: RendererTelemetrySample[] = [];
  private readonly timingValues = new Map<string, number[]>();
  private last: RendererTelemetrySample | null = null;
  private recordedFrames = 0;
  private droppedFrames = 0;
  private cachedSummary: Record<string, TelemetryDistribution> = {};

  constructor(options: { level?: TelemetryLevel; maxSamples?: number; summaryEvery?: number } = {}) {
    this.level = options.level ?? "summary";
    const requestedSamples = options.maxSamples ?? MAX_TRACE_SAMPLES;
    this.maxSamples = Number.isFinite(requestedSamples)
      ? Math.min(MAX_TRACE_SAMPLES, Math.max(1, Math.trunc(requestedSamples)))
      : MAX_TRACE_SAMPLES;
    this.summaryEvery = Math.max(1, options.summaryEvery ?? 30);
  }

  record(sample: RendererTelemetrySample): void {
    if (this.level === "off") return;
    assertTelemetryPrivacy(sample);
    const sanitized = structuredClone(sample);
    this.last = sanitized;
    this.recordedFrames += 1;
    for (const [name, value] of Object.entries(sample.timingsMs)) {
      if (value == null || !Number.isFinite(value)) continue;
      const values = this.timingValues.get(name) ?? [];
      values.push(value);
      if (values.length > this.maxSamples) values.splice(0, values.length - this.maxSamples);
      this.timingValues.set(name, values);
    }
    if (this.level === "trace") {
      this.samples.push(sanitized);
      if (this.samples.length > this.maxSamples) {
        this.droppedFrames += this.samples.length - this.maxSamples;
        this.samples.splice(0, this.samples.length - this.maxSamples);
      }
    }
    if (this.recordedFrames % this.summaryEvery === 0 || Object.keys(this.cachedSummary).length === 0) {
      this.cachedSummary = summarize(this.timingValues);
    }
  }

  snapshot(): RendererTelemetrySnapshot | null {
    if (this.level === "off" || !this.last) return null;
    const snapshot: RendererTelemetrySnapshot = {
      schemaVersion: 1,
      level: this.level,
      last: structuredClone(this.last),
      summary: structuredClone(summarize(this.timingValues)),
      recordedFrames: this.recordedFrames,
      droppedFrames: this.droppedFrames,
    };
    if (this.level === "trace") snapshot.samples = structuredClone(this.samples);
    assertTelemetryPrivacy(snapshot);
    return snapshot;
  }

  stats(): { level: TelemetryLevel; recordedFrames: number; retainedSamples: number; droppedFrames: number } {
    return {
      level: this.level,
      recordedFrames: this.recordedFrames,
      retainedSamples: this.samples.length,
      droppedFrames: this.droppedFrames,
    };
  }
}

export type RendererTelemetryOptions = {
  level?: TelemetryLevel;
  maxSamples?: number;
  summaryEvery?: number;
};

export type RendererAtlasSummary = {
  backend: "edt" | "analytic";
  pages: number;
  glyphs: number;
  missingGlyphs: number;
  generation: {
    glyphs: number;
    pixels: number;
    glyphMs: number;
    faceLoadMs: number;
  };
  cache: {
    hits: number;
    misses: number;
    generations: number;
    bytes: number;
    sessionHits: number;
    persistentHits: number;
    persistentMisses: number;
    persistentWritesQueued: number;
    pinnedPages: number;
    pageEvictions: number;
  } | null;
};

export type RendererGpuSummary = {
  drawCalls: number;
  geometryBuilds: number;
  vertexBytes: number;
  textureUploads: number;
  textureBytes: number;
  stateUploadBytes: number;
  maskUploadBytes: number;
  glyphGeometryBuilds: number;
};

export type RendererUploadSummary = {
  stateUploadBytes: number;
  maskUploadBytes: number;
  commandMaskUploadBytes: number;
  commandStateUploadBytes: number;
};

export type RendererRestoreUploadSummary = {
  atlasUploadBytes: number;
  atlasUploadRects: number;
  textureUploads: number;
  textureBytes: number;
};

export type RendererRuntimeSnapshot = {
  schemaVersion: 1;
  state: "ready" | "context-lost" | "destroyed";
  gpuEpoch: number;
  frames: number;
  bootstrap: {
    textCommands: number;
    glyphInstances: number;
    atlasUploadBytes: number;
    atlasUploadRects: number;
  };
  atlas: RendererAtlasSummary;
  lastGpu: RendererGpuSummary | null;
  updates: RendererUploadSummary;
  recovery: {
    contextLosses: number;
    contextRestores: number;
    restoreFailures: number;
    lastRestoreMs: number | null;
    lastRestoreUploads: RendererRestoreUploadSummary | null;
  };
  telemetry: RendererTelemetrySnapshot | null;
};

/** Tracks aggregate rendering health without retaining source text, stable ids,
 * resource URLs, font names, or player data. */
export class RendererRuntimeTelemetry {
  private readonly telemetry: RendererTelemetry;
  private readonly atlas: RendererAtlasSummary;
  private readonly bootstrap: RendererRuntimeSnapshot["bootstrap"];
  private runtimeState: RendererRuntimeSnapshot["state"] = "ready";
  private gpuEpoch = 1;
  private frames = 0;
  private lastGpu: RendererGpuSummary | null = null;
  private updates: RendererUploadSummary = {
    stateUploadBytes: 0,
    maskUploadBytes: 0,
    commandMaskUploadBytes: 0,
    commandStateUploadBytes: 0,
  };
  private contextLosses = 0;
  private contextRestores = 0;
  private restoreFailures = 0;
  private lastRestoreMs: number | null = null;
  private lastRestoreUploads: RendererRestoreUploadSummary | null = null;

  constructor(
    options: RendererTelemetryOptions,
    atlas: RendererAtlasSummary,
    bootstrap: RendererRuntimeSnapshot["bootstrap"],
  ) {
    this.telemetry = new RendererTelemetry(options);
    this.atlas = structuredClone(atlas);
    this.bootstrap = structuredClone(bootstrap);
    assertTelemetryPrivacy(this.atlas);
  }

  state(): RendererRuntimeSnapshot["state"] {
    return this.runtimeState;
  }

  recordDraw(metrics: RendererGpuSummary, submitCpuMs: number): void {
    this.lastGpu = structuredClone(metrics);
    this.frames += 1;
    this.telemetry.record({
      frameId: this.frames,
      gpuEpoch: this.gpuEpoch,
      timingsMs: {
        gpuSubmitCpu: finiteOrNull(submitCpuMs),
        gpuTime: null,
        total: finiteOrNull(submitCpuMs),
      },
      workload: {
        drawCalls: metrics.drawCalls,
        vertexBytes: metrics.vertexBytes,
        textureUploads: metrics.textureUploads,
        textureBytes: metrics.textureBytes,
        dirtyBytes: this.updates.stateUploadBytes
          + this.updates.maskUploadBytes
          + this.updates.commandMaskUploadBytes
          + this.updates.commandStateUploadBytes,
      },
      recovery: {
        contextLosses: this.contextLosses,
        contextRestores: this.contextRestores,
        restoreFailures: this.restoreFailures,
        restoreMs: this.lastRestoreMs,
      },
    });
  }

  recordPatch(patch: Partial<RendererUploadSummary>): void {
    for (const key of Object.keys(this.updates) as Array<keyof RendererUploadSummary>) {
      this.updates[key] += patch[key] ?? 0;
    }
  }

  markContextLost(): void {
    if (this.runtimeState === "destroyed" || this.runtimeState === "context-lost") return;
    this.runtimeState = "context-lost";
    this.contextLosses += 1;
  }

  markContextRestored(restoreMs: number, uploads: RendererRestoreUploadSummary): void {
    if (this.runtimeState === "destroyed") return;
    this.runtimeState = "ready";
    this.contextRestores += 1;
    this.gpuEpoch += 1;
    this.lastRestoreMs = finiteOrNull(restoreMs);
    this.lastRestoreUploads = structuredClone(uploads);
  }

  markRestoreFailed(restoreMs: number): void {
    if (this.runtimeState === "destroyed") return;
    this.runtimeState = "context-lost";
    this.restoreFailures += 1;
    this.lastRestoreMs = finiteOrNull(restoreMs);
  }

  markDestroyed(): void {
    this.runtimeState = "destroyed";
  }

  snapshot(): RendererRuntimeSnapshot {
    const snapshot: RendererRuntimeSnapshot = {
      schemaVersion: 1,
      state: this.runtimeState,
      gpuEpoch: this.gpuEpoch,
      frames: this.frames,
      bootstrap: structuredClone(this.bootstrap),
      atlas: structuredClone(this.atlas),
      lastGpu: structuredClone(this.lastGpu),
      updates: structuredClone(this.updates),
      recovery: {
        contextLosses: this.contextLosses,
        contextRestores: this.contextRestores,
        restoreFailures: this.restoreFailures,
        lastRestoreMs: this.lastRestoreMs,
        lastRestoreUploads: structuredClone(this.lastRestoreUploads),
      },
      telemetry: this.telemetry.snapshot(),
    };
    assertTelemetryPrivacy(snapshot);
    return snapshot;
  }
}

export function normalizeGpuTimerResult(nanoseconds: number | undefined, disjoint: boolean): number | null {
  if (disjoint || nanoseconds == null || !Number.isFinite(nanoseconds) || nanoseconds < 0) return null;
  return nanoseconds / 1_000_000;
}

const FORBIDDEN_FIELDS = new Set([
  "rawtext",
  "sourcecontent",
  "userid",
  "profileseq",
  "documentdigest",
  "fontpath",
  "fonturl",
  "character",
  "scalar",
  "cluster",
]);

export function assertTelemetryPrivacy(value: unknown): void {
  visit(value, "telemetry");
}

function visit(value: unknown, path: string): void {
  if (value == null || typeof value !== "object") return;
  if (Array.isArray(value)) {
    value.forEach((entry, index) => visit(entry, `${path}[${index}]`));
    return;
  }
  for (const [key, child] of Object.entries(value as Record<string, unknown>)) {
    const normalized = key.replace(/[_-]/g, "").toLowerCase();
    if (FORBIDDEN_FIELDS.has(normalized)) throw new Error(`forbidden telemetry field ${path}.${key}`);
    visit(child, `${path}.${key}`);
  }
}

function summarize(values: ReadonlyMap<string, readonly number[]>): Record<string, TelemetryDistribution> {
  return Object.fromEntries([...values.entries()].map(([name, source]) => {
    const sorted = [...source].sort((a, b) => a - b);
    const quantile = (q: number) => sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * q))] ?? 0;
    return [name, {
      p50: quantile(0.5),
      p95: quantile(0.95),
      max: quantile(1),
      samples: sorted.length,
    }];
  }));
}

function finiteOrNull(value: number): number | null {
  return Number.isFinite(value) && value >= 0 ? value : null;
}
