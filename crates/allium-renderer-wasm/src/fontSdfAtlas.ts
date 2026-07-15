import {
  GlyphPersistentCache,
  createPersistentGlyphRecord,
  type GlyphRasterIdentity,
  type PersistentGlyphRecord,
} from "./cache/glyphPersistentCache.js";
import { createOriginGlyphPersistentCache } from "./cache/indexedDbGlyphRecordStore.js";
import { RendererAtlas, RendererWorkerClient } from "./worker-client.js";
import type { RendererWorkerStats } from "./protocol.js";
import type {
  AtlasGenerateRequest,
  AtlasGlyphRecord,
  AtlasPageUpdate,
  AtlasStats,
  GlyphRasterPlan,
} from "./types/atlas.js";

export type GlyphRequest = {
  region: string;
  family: string;
  fontSourceHash: string;
  char: string;
};

export type FontSource = {
  region: string;
  family: string;
  sourceHash: string;
};

export type GlyphInfo = {
  key: string;
  resourceId: string;
  glyphIndex: number;
  page: number;
  pageEpoch: number;
  region: string;
  family: string;
  fontSourceHash: string;
  char: string;
  u0: number;
  v0: number;
  u1: number;
  v1: number;
  width: number;
  height: number;
  advance: number;
  xOffset: number;
  yOffset: number;
  planeBearingX: number;
  planeBearingY: number;
  planeWidth: number;
  planeHeight: number;
  drawable: boolean;
};

export type SdfBackend = "edt" | "analytic";
export type PersistentCacheSelection = "memory-only" | "origin";

export type SdfAtlasPerf = {
  backend: SdfBackend;
  fontBatches: [];
  totalGlyphMs: number;
  totalFaceLoadMs: number;
  totalGlyphCount: number;
  totalPixelCount: number;
  worker: RendererWorkerStats;
  cache: {
    hits: number;
    misses: number;
    generations: number;
    bytes: number;
    sessionHits: number;
    persistentHits: number;
    persistentMisses: number;
    persistentWritesQueued: number;
    pages: number;
    pinnedPages: number;
    pageEvictions: number;
  };
};

export type SdfAtlas = {
  width: number;
  height: number;
  depth: number;
  baseSize: number;
  spread: number;
  glyphs: Map<string, GlyphInfo>;
  missing: string[];
  backend: "freetype-wasm";
  sdfBackend: SdfBackend;
  contractId: string;
  fontEngineFingerprint: string;
  perf: SdfAtlasPerf;
  pageUpdates(revisions: ReadonlyMap<number, number>): Promise<AtlasPageUpdate[]>;
  release(): Promise<void>;
};

type ResolvedRequest = {
  request: GlyphRequest;
  glyphIndex: number;
  identity: GlyphRasterIdentity;
};

type CanonicalGlyphRasterPlan = Omit<GlyphRasterPlan, "region" | "family" | "fontSourceHash" | "glyphs" | "missing"> & {
  glyphs: Array<GlyphRasterPlan["glyphs"][number] & Pick<GlyphRasterPlan, "region" | "family" | "fontSourceHash">>;
};

type SessionEngine = {
  atlas: RendererAtlas;
  contractId: string;
  charToGlyphIndex: Map<string, number>;
  missingChars: Set<string>;
  metrics: Map<string, Omit<AtlasGlyphRecord, "pixels">>;
  placements: Map<string, { page: number; pageEpoch: number }>;
  latestStats: AtlasStats;
};

const engines = new WeakMap<RendererWorkerClient, Map<string, Promise<SessionEngine>>>();
const engineRegistry = new Map<RendererWorkerClient, Set<SessionEngine>>();
let originPersistentCache: GlyphPersistentCache | null = null;

export function glyphKey(region: string, fontSourceHash: string, family: string, char: string): string {
  return `${region}\u0000${fontSourceHash}\u0000${family}\u0000${char}`;
}

export async function buildSdfAtlas(
  fontSources: FontSource[],
  requests: GlyphRequest[],
  options: { worker: RendererWorkerClient; persistence?: PersistentCacheSelection; persistentCache?: GlyphPersistentCache },
  supersample?: number,
  backend?: SdfBackend,
): Promise<SdfAtlas> {
  validateFontSources(fontSources);
  const sdfBackend = backend ?? (supersample != null && supersample <= 0 ? "analytic" : "edt");
  const persistent = options.persistentCache ?? (options.persistence === "origin" ? getOriginPersistentCache() : new GlyphPersistentCache());
  const uniqueRequests = deduplicateRequests(requests);
  const plan = await planGlyphs(options.worker, fontSources, uniqueRequests, sdfBackend, supersample ?? 2);
  const engine = await getEngine(options.worker, plan.contractId);
  const planned = new Map(plan.glyphs.map((glyph) => [
    glyphKey(glyph.region, glyph.fontSourceHash, glyph.family, glyph.ch),
    glyph,
  ] as const));
  for (const request of uniqueRequests) {
    const glyph = planned.get(requestKey(request));
    if (glyph) engine.charToGlyphIndex.set(requestKey(request), glyph.glyphIndex);
    else engine.missingChars.add(requestKey(request));
  }
  const resolved = uniqueRequests.flatMap((request): ResolvedRequest[] => {
    const glyph = planned.get(requestKey(request));
    return glyph ? [{ request, glyphIndex: glyph.glyphIndex, identity: glyph.identity }] : [];
  });
  const unique = new Map(resolved.map((value) => [value.identity.opaqueKey, value] as const));
  const placementsBefore = new Map([...unique.keys()].flatMap((key) => {
    const placement = engine.placements.get(key);
    return placement ? [[key, placement] as const] : [];
  }));
  const identities = [...unique.values()].map((value) => value.identity);
  const persisted = await persistent.getMany(identities);
  const cached = [...persisted.values()].map(recordToAtlas);
  for (const record of cached) engine.metrics.set(record.key, withoutPixels(record));
  const missing = [...unique.values()].filter((value) => !persisted.has(value.identity.opaqueKey));
  const generate = generationGroups(fontSources, missing, plan.backend, plan.supersample);
  const workerBefore = await options.worker.stats();
  const result = await engine.atlas.resolve([...unique.keys()], cached, generate);
  engine.latestStats = result.stats;
  const placementByKey = new Map(result.placements.map((entry) => [entry.key, entry.placement] as const));
  for (const [key, placement] of placementByKey) {
    engine.placements.set(key, { page: placement.page, pageEpoch: placement.pageEpoch });
  }
  const sessionHitKeys = new Set([...unique.keys()].filter((key) => {
    const before = placementsBefore.get(key);
    const after = placementByKey.get(key);
    return before != null && after != null && before.page === after.page && before.pageEpoch === after.pageEpoch;
  }));
  const sessionHits = sessionHitKeys.size;
  const persistentHits = [...persisted.keys()].filter((key) => !sessionHitKeys.has(key)).length;
  for (const record of result.generated) engine.metrics.set(record.key, withoutPixels(record));
  if (result.generated.length > 0) {
    const writeEpoch = persistent.beginWrite();
    const records = await Promise.all(result.generated.map((record) => {
      const identity = unique.get(record.key)?.identity;
      if (!identity) throw new Error(`generated atlas resource has unknown identity ${record.key}`);
      return createPersistentGlyphRecord(identity, record);
    }));
    void persistent.putMany(records, writeEpoch).catch(() => undefined);
  }
  const glyphs = new Map<string, GlyphInfo>();
  for (const value of resolved) {
    const resourceId = value.identity.opaqueKey;
    const placement = placementByKey.get(resourceId);
    const metrics = engine.metrics.get(resourceId);
    if (!placement || !metrics) throw new Error(`WASM atlas omitted glyph resource ${resourceId}`);
    glyphs.set(requestKey(value.request), {
      key: requestKey(value.request),
      resourceId,
      glyphIndex: value.glyphIndex,
      page: placement.page,
      pageEpoch: placement.pageEpoch,
      region: value.request.region,
      family: value.request.family,
      fontSourceHash: value.request.fontSourceHash,
      char: value.request.char,
      u0: placement.u0,
      v0: placement.v0,
      u1: placement.u1,
      v1: placement.v1,
      width: metrics.width,
      height: metrics.height,
      advance: metrics.advance,
      xOffset: metrics.xOffset,
      yOffset: metrics.yOffset,
      planeBearingX: metrics.planeBearingX,
      planeBearingY: metrics.planeBearingY,
      planeWidth: metrics.planeWidth,
      planeHeight: metrics.planeHeight,
      drawable: metrics.drawable,
    });
  }
  const workerAfter = await options.worker.stats();
  let released = false;
  return {
    width: plan.atlasWidth,
    height: plan.atlasHeight,
    depth: result.stats.pages,
    baseSize: plan.baseSize,
    spread: plan.spread,
    glyphs,
    missing: uniqueRequests.filter((request) => engine.missingChars.has(requestKey(request))).map(requestKey),
    backend: "freetype-wasm",
    sdfBackend: plan.backend,
    contractId: plan.contractId,
    fontEngineFingerprint: plan.fontEngineFingerprint,
    perf: {
      backend: sdfBackend,
      fontBatches: [],
      totalGlyphMs: Math.max(0, workerAfter.wasmMs - workerBefore.wasmMs),
      totalFaceLoadMs: 0,
      totalGlyphCount: result.generated.length,
      totalPixelCount: result.generated.reduce((sum, record) => sum + record.pixels.byteLength, 0),
      worker: workerAfter,
      cache: {
        hits: sessionHits + persistentHits,
        misses: unique.size - sessionHits - persistentHits,
        generations: result.generated.length,
        bytes: result.stats.atlasBytes,
        sessionHits,
        persistentHits,
        persistentMisses: unique.size - sessionHits - persistentHits,
        persistentWritesQueued: result.generated.length,
        pages: result.stats.pages,
        pinnedPages: result.stats.pinnedPages,
        pageEvictions: result.stats.evictions,
      },
    },
    pageUpdates: (revisions) => engine.atlas.pages(revisions),
    release: async () => {
      if (released) return;
      released = true;
      await Promise.all(result.leases.map((lease) => engine.atlas.release(lease)));
    },
  };
}

export function getPersistentGlyphCache(): GlyphPersistentCache {
  return getOriginPersistentCache();
}

export function getSessionAtlasStats(): Array<{ contractId: string; stats: AtlasStats }> {
  return [...engineRegistry.values()].flatMap((sessions) => [...sessions]
    .map((engine) => ({ contractId: engine.contractId, stats: engine.latestStats })));
}

export function disposeWorkerAtlasSessions(worker: RendererWorkerClient): void {
  engineRegistry.delete(worker);
  engines.delete(worker);
}

async function getEngine(worker: RendererWorkerClient, contractId: string): Promise<SessionEngine> {
  let byContract = engines.get(worker);
  if (!byContract) {
    byContract = new Map();
    engines.set(worker, byContract);
  }
  let pending = byContract.get(contractId);
  if (!pending) {
    pending = worker.createAtlas().then((atlas) => {
      const engine: SessionEngine = {
        atlas,
        contractId,
        charToGlyphIndex: new Map(),
        missingChars: new Set(),
        metrics: new Map(),
        placements: new Map(),
        latestStats: atlas.initialStats,
      };
      let registered = engineRegistry.get(worker);
      if (!registered) {
        registered = new Set();
        engineRegistry.set(worker, registered);
      }
      registered.add(engine);
      return engine;
    });
    byContract.set(contractId, pending);
  }
  return pending;
}

async function planGlyphs(
  worker: RendererWorkerClient,
  fonts: readonly FontSource[],
  requests: readonly GlyphRequest[],
  backend: SdfBackend,
  supersample: number,
): Promise<CanonicalGlyphRasterPlan> {
  const groups = new Map<string, { font: FontSource; chars: string[]; seen: Set<string> }>();
  for (const font of fonts) groups.set(fontSourceKey(font), { font, chars: [], seen: new Set() });
  for (const request of requests) {
    const group = groups.get(fontRequestKey(request));
    if (!group || group.seen.has(request.char)) continue;
    group.seen.add(request.char);
    group.chars.push(request.char);
  }
  const activeGroups = [...groups.values()].filter((group) => group.chars.length > 0);
  if (activeGroups.length === 0 && fonts[0]) activeGroups.push({ font: fonts[0], chars: [], seen: new Set() });
  const plans = await Promise.all(activeGroups.map((group) => worker.planGlyphs({
    region: group.font.region,
    family: group.font.family,
    sourceHash: group.font.sourceHash,
    chars: group.chars,
    backend,
    supersample,
  })));
  const first = plans[0];
  if (!first) throw new Error("WASM glyph raster plan requires at least one glyph request");
  for (const plan of plans) {
    if (
      plan.contractId !== first.contractId
      || plan.baseSize !== first.baseSize
      || plan.spread !== first.spread
      || plan.atlasWidth !== first.atlasWidth
      || plan.atlasHeight !== first.atlasHeight
    ) throw new Error("WASM returned incompatible glyph raster plans for one atlas");
  }
  return {
    ...first,
    glyphs: plans.flatMap((plan) => plan.glyphs.map((glyph) => ({
      ...glyph,
      region: plan.region,
      family: plan.family,
      fontSourceHash: plan.fontSourceHash,
    }))),
  };
}

function generationGroups(
  fonts: readonly FontSource[],
  missing: readonly ResolvedRequest[],
  backend: SdfBackend,
  supersample: number,
): AtlasGenerateRequest[] {
  const groups = new Map<string, AtlasGenerateRequest>();
  for (const value of missing) {
    const font = fonts.find((candidate) => fontSourceKey(candidate) === fontRequestKey(value.request));
    if (!font) throw new Error(`font source missing for ${value.request.region}/${value.request.family}`);
    const key = fontSourceKey(font);
    let group = groups.get(key);
    if (!group) {
      group = { region: font.region, family: font.family, sourceHash: font.sourceHash, chars: [], backend, supersample, glyphs: [] };
      groups.set(key, group);
    }
    group.chars.push(value.request.char);
    group.glyphs.push({ key: value.identity.opaqueKey, char: value.request.char, glyphIndex: value.glyphIndex });
  }
  return [...groups.values()];
}

function recordToAtlas(record: PersistentGlyphRecord): AtlasGlyphRecord {
  return {
    key: record.opaqueKey,
    glyphIndex: 0,
    width: record.width,
    height: record.height,
    advance: record.advance,
    xOffset: record.xOffset,
    yOffset: record.yOffset,
    planeBearingX: record.planeBearingX,
    planeBearingY: record.planeBearingY,
    planeWidth: record.planeWidth,
    planeHeight: record.planeHeight,
    drawable: record.drawable,
    pixels: new Uint8Array(record.pixels),
  };
}

function withoutPixels(record: AtlasGlyphRecord): Omit<AtlasGlyphRecord, "pixels"> {
  const { pixels: _, ...metrics } = record;
  return metrics;
}

function getOriginPersistentCache(): GlyphPersistentCache {
  originPersistentCache ??= typeof indexedDB === "undefined" ? new GlyphPersistentCache() : createOriginGlyphPersistentCache();
  return originPersistentCache;
}

function validateFontSources(fonts: readonly FontSource[]): void {
  for (const font of fonts) {
    if (!font.region || !font.family) throw new Error("font region/family must be non-empty");
    if (!/^[0-9a-f]{64}$/i.test(font.sourceHash)) throw new Error(`font ${font.region}/${font.family} requires a full SHA-256 digest`);
  }
}

function deduplicateRequests(requests: readonly GlyphRequest[]): GlyphRequest[] {
  const result = new Map<string, GlyphRequest>();
  for (const request of requests) if (request.char !== "\n" && request.char !== "\r" && !result.has(requestKey(request))) result.set(requestKey(request), request);
  return [...result.values()];
}

function fontSourceKey(source: FontSource): string {
  return `${source.region}\u0000${source.sourceHash}\u0000${source.family}`;
}

function fontRequestKey(request: GlyphRequest): string {
  return `${request.region}\u0000${request.fontSourceHash}\u0000${request.family}`;
}

function requestKey(request: GlyphRequest): string {
  return glyphKey(request.region, request.fontSourceHash, request.family, request.char);
}
