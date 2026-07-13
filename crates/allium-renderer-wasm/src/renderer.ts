import {
  BrowserSemanticResourceManager,
  defaultSemanticResourceUrl,
  type BrowserSemanticResourceSet,
} from "./gpu/browserSemanticResources.js";
import {
  semanticCommandPlanFromCoreSnapshot,
} from "./gpu/semanticCommandPlanner.js";
import { SemanticWebglSceneRenderer } from "./gpu/semanticWebglSceneRenderer.js";
import type { NumericTextRegion } from "./interaction/numericTextRegions.js";
import {
  buildSdfAtlas,
  disposeWorkerAtlasSessions,
  type FontSource,
  type GlyphRequest,
  type PersistentCacheSelection,
  type SdfAtlas,
  type SdfBackend,
} from "./fontSdfAtlas.js";
import {
  RendererMasterData,
  RendererScene,
  RendererWorkerClient,
  type RendererWorkerClientOptions,
} from "./worker-client.js";
import type {
  CoreSceneDelta,
  CoreSceneDump,
  StableId,
} from "./types/core.js";
import {
  RendererRuntimeTelemetry,
  type RendererAtlasSummary,
  type RendererTelemetryOptions,
  type RendererRuntimeSnapshot,
} from "./telemetry/rendererTelemetry.js";

const CARD_WIDTH = 1830;
const CARD_HEIGHT = 812;

export type BrowserRendererOptions = RendererWorkerClientOptions & {
  canvas: HTMLCanvasElement | OffscreenCanvas;
  region: string;
  resolveResourceUrl?: (namespace: string, key: string, region: string) => string;
  resolveMasterDataUrl?: (table: string, region: string, revision: string) => string;
  telemetry?: RendererTelemetryOptions;
};

export type ProfileSceneCreateOptions = {
  masterData: RendererMasterData;
  documentKey: string;
  card: unknown;
  profile?: unknown;
  locale?: string;
  frameMode?: "final" | "animate";
  sdf?: {
    backend?: SdfBackend;
    supersample?: number;
    persistence?: PersistentCacheSelection;
  };
};

/**
 * WebGL2-only browser renderer. Rust/WASM scene, text layout, and glyph work
 * stay in one worker; the main thread owns resource decoding and GPU state.
 */
export class BrowserRenderer {
  private readonly resources: BrowserSemanticResourceManager;
  private readonly fonts = new Map<string, FontSource>();
  private readonly scenes = new Set<BrowserScene>();
  private readonly canvas: HTMLCanvasElement | OffscreenCanvas;
  private readonly telemetryOptions: RendererTelemetryOptions;
  private contextLost = false;
  private restorePromise: Promise<void> | null = null;
  private destroyed = false;

  private constructor(
    private gl: WebGL2RenderingContext,
    private readonly worker: RendererWorkerClient,
    private readonly region: string,
    canvas: HTMLCanvasElement | OffscreenCanvas,
    resolveResourceUrl?: BrowserRendererOptions["resolveResourceUrl"],
    private readonly resolveMasterDataUrl: NonNullable<BrowserRendererOptions["resolveMasterDataUrl"]> = defaultMasterDataUrl,
    telemetryOptions: RendererTelemetryOptions = {},
  ) {
    this.canvas = canvas;
    this.telemetryOptions = telemetryOptions;
    this.resources = new BrowserSemanticResourceManager({
      environmentId: `cdn:${region}`,
      resolveUrl: (resource) => resolveResourceUrl?.(resource.namespace, resource.key, region)
        ?? defaultSemanticResourceUrl(resource, region),
    });
    if (isHtmlCanvas(canvas)) {
      canvas.addEventListener("webglcontextlost", this.handleContextLost);
      canvas.addEventListener("webglcontextrestored", this.handleContextRestored);
    }
  }

  static async create(options: BrowserRendererOptions): Promise<BrowserRenderer> {
    const gl = options.canvas.getContext("webgl2", {
      alpha: true,
      antialias: true,
      depth: false,
      premultipliedAlpha: true,
      preserveDrawingBuffer: false,
    }) as WebGL2RenderingContext | null;
    if (!gl) throw new BrowserRendererError("WEBGL2_UNAVAILABLE", "WebGL2 is required by renderer-wasm 0.2");
    options.canvas.width = CARD_WIDTH;
    options.canvas.height = CARD_HEIGHT;
    const worker = await RendererWorkerClient.create(options);
    return new BrowserRenderer(
      gl,
      worker,
      options.region,
      options.canvas,
      options.resolveResourceUrl,
      options.resolveMasterDataUrl,
      options.telemetry,
    );
  }

  async createProfileScene(options: ProfileSceneCreateOptions): Promise<BrowserScene> {
    this.assertAlive();
    let resources: BrowserSemanticResourceSet<ImageBitmap> | null = null;
    let core: RendererScene | null = null;
    let atlas: SdfAtlas | null = null;
    try {
      const preparation = await options.masterData.prepareProfile({
        documentKey: options.documentKey,
        card: options.card,
        profile: options.profile,
        locale: options.locale ?? this.region,
      });
      const glyphRequests = preparedGlyphRequests(preparation);
      atlas = await buildSdfAtlas(
        requiredFontSources(glyphRequests, this.fonts),
        glyphRequests,
        { worker: this.worker, persistence: options.sdf?.persistence ?? "origin" },
        options.sdf?.supersample,
        options.sdf?.backend,
      );
      const requestedResources = profileResourceRequests(preparation);
      resources = await this.resources.acquire(requestedResources);
      const compiled = await options.masterData.createProfileScene({
        documentKey: options.documentKey,
        card: options.card,
        profile: options.profile,
        locale: options.locale ?? this.region,
        frameMode: options.frameMode ?? "animate",
        resourceMetrics: requestedResources.map((resource) => {
          const source = resources?.sources.get(`${resource.namespace}\0${resource.key}`);
          return {
            namespace: resource.namespace,
            key: resource.key,
            width: source?.width ?? 0,
            height: source?.height ?? 0,
          };
        }),
      }, preparedLayoutRequest(preparation, atlas));
      core = compiled.scene;
      const plan = semanticCommandPlanFromCoreSnapshot(
        core.initial.snapshot as unknown as Parameters<typeof semanticCommandPlanFromCoreSnapshot>[0],
      );
      assertPreparedResources(plan.resourceRequests(), resources.sources);
      const renderer = new SemanticWebglSceneRenderer(this.gl);
      const bootstrap = await renderer.setScene({
        plan,
        atlas,
        layout: compiled.layout,
        imageSources: resources.sources,
      });
      const scene = new BrowserScene(core, renderer, resources, atlas, {
        telemetry: this.telemetryOptions,
        bootstrap,
        onDestroy: (destroyed) => this.scenes.delete(destroyed),
      });
      this.scenes.add(scene);
      return scene;
    } catch (error) {
      atlas?.release();
      resources?.release();
      await core?.destroy().catch(() => undefined);
      throw error;
    }
  }

  async createMasterData(revision: string): Promise<RendererMasterData> {
    this.assertAlive();
    return this.worker.createMasterData(this.region, revision);
  }

  async loadMasterData(
    revision = "latest",
    fetchTable: (url: string) => Promise<unknown> = fetchJson,
  ): Promise<RendererMasterData> {
    const session = await this.createMasterData(revision);
    try {
      let cursor = 0;
      const workers = Array.from({ length: Math.min(4, session.requiredTables.length) }, async () => {
        while (cursor < session.requiredTables.length) {
          const table = session.requiredTables[cursor++];
          const value = await fetchTable(this.resolveMasterDataUrl(table, this.region, revision));
          await session.putTable(table, value);
        }
      });
      await Promise.all(workers);
      await session.seal();
      return session;
    } catch (error) {
      await session.destroy().catch(() => undefined);
      throw error;
    }
  }

  async registerFont(font: { family: string; bytes: ArrayBuffer }): Promise<void> {
    this.assertAlive();
    const sourceHash = await sha256Hex(font.bytes);
    const source = { region: this.region, family: font.family, sourceHash };
    await this.worker.registerFont({ ...source, bytes: font.bytes });
    this.fonts.set(font.family, source);
  }

  async stats() {
    if (this.destroyed) throw new BrowserRendererError("RENDERER_DESTROYED", "Browser renderer is destroyed");
    return {
      worker: await this.worker.stats(),
      resources: this.resources.stats(),
      scenes: {
        active: this.scenes.size,
        contextLost: [...this.scenes].filter((scene) => scene.stats().state === "context-lost").length,
      },
    };
  }

  async restoreContext(): Promise<void> {
    if (this.destroyed) throw new BrowserRendererError("RENDERER_DESTROYED", "Browser renderer is destroyed");
    if (this.restorePromise) return this.restorePromise;
    this.restorePromise = (async () => {
      const started = monotonicNow();
      const gl = this.canvas.getContext("webgl2", {
        alpha: true,
        antialias: true,
        depth: false,
        premultipliedAlpha: true,
        preserveDrawingBuffer: false,
      }) as WebGL2RenderingContext | null;
      if (!gl) {
        const restoreMs = monotonicNow() - started;
        for (const scene of this.scenes) scene.notifyContextRestoreFailed(restoreMs);
        throw new BrowserRendererError("WEBGL2_RESTORE_FAILED", "WebGL2 context restoration failed");
      }
      this.gl = gl;
      await Promise.all([...this.scenes].map((scene) => scene.restoreContext(gl)));
      this.contextLost = false;
    })();
    try {
      await this.restorePromise;
    } finally {
      this.restorePromise = null;
    }
  }

  destroy(): void {
    if (this.destroyed) return;
    this.destroyed = true;
    if (isHtmlCanvas(this.canvas)) {
      this.canvas.removeEventListener("webglcontextlost", this.handleContextLost);
      this.canvas.removeEventListener("webglcontextrestored", this.handleContextRestored);
    }
    disposeWorkerAtlasSessions(this.worker);
    this.worker.terminate();
  }

  private assertAlive(): void {
    if (this.destroyed) throw new BrowserRendererError("RENDERER_DESTROYED", "Browser renderer is destroyed");
    if (this.contextLost) throw new BrowserRendererError("WEBGL_CONTEXT_LOST", "WebGL context is lost");
  }

  private readonly handleContextLost = (event: Event): void => {
    event.preventDefault();
    this.contextLost = true;
    for (const scene of this.scenes) scene.notifyContextLost();
  };

  private readonly handleContextRestored = (): void => {
    void this.restoreContext().catch(() => {
      // Scene recovery counters retain the failure. Call restoreContext() explicitly
      // when the host needs an actionable rejection.
    });
  };
}

function atlasLayoutInput(atlas: SdfAtlas) {
  return {
    baseSize: atlas.baseSize,
    spread: atlas.spread,
    glyphs: Array.from(atlas.glyphs.values()),
  };
}

function preparedGlyphRequests(preparation: Record<string, unknown>): GlyphRequest[] {
  const demand = preparation.glyph_demand;
  if (!demand || typeof demand !== "object" || Array.isArray(demand)) {
    throw new BrowserRendererError("INVALID_PROFILE_PREPARATION", "Profile preparation did not return glyph demand");
  }
  const requests = (demand as Record<string, unknown>).requests;
  if (!Array.isArray(requests)) {
    throw new BrowserRendererError("INVALID_PROFILE_PREPARATION", "Profile glyph demand is invalid");
  }
  return requests.map((entry) => {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
      throw new BrowserRendererError("INVALID_PROFILE_PREPARATION", "Profile glyph request is invalid");
    }
    const request = entry as Record<string, unknown>;
    if (
      typeof request.region !== "string"
      || typeof request.family !== "string"
      || typeof request.font_source_hash !== "string"
      || typeof request.char !== "string"
    ) {
      throw new BrowserRendererError("INVALID_PROFILE_PREPARATION", "Profile glyph identity is invalid");
    }
    return {
      region: request.region,
      family: request.family,
      fontSourceHash: request.font_source_hash,
      char: request.char,
    };
  });
}

function preparedLayoutRequest(
  preparation: Record<string, unknown>,
  atlas: SdfAtlas,
): Record<string, unknown> {
  const request = preparation.layout_request;
  if (!request || typeof request !== "object" || Array.isArray(request)) {
    throw new BrowserRendererError("INVALID_PROFILE_PREPARATION", "Profile preparation did not return an authored layout request");
  }
  return {
    ...(request as Record<string, unknown>),
    atlas: atlasLayoutInput(atlas),
  };
}

function requiredFontSources(
  requests: ReadonlyArray<{ family: string; fontSourceHash: string }>,
  fonts: ReadonlyMap<string, FontSource>,
): FontSource[] {
  const output = new Map<string, FontSource>();
  for (const request of requests) {
    const source = fonts.get(request.family);
    if (!source || source.sourceHash !== request.fontSourceHash) {
      throw new BrowserRendererError("FONT_NOT_REGISTERED", `Required font is not registered: ${request.family}`);
    }
    output.set(`${source.region}\0${source.sourceHash}\0${source.family}`, source);
  }
  return [...output.values()];
}

function defaultMasterDataUrl(table: string, region: string, revision: string): string {
  return `https://cdn.emptysekai.com/masterdata/${encodeURIComponent(region)}/${encodeURIComponent(revision)}/${encodeURIComponent(table)}.json`;
}

async function fetchJson(url: string): Promise<unknown> {
  const response = await fetch(url, { cache: "force-cache" });
  if (!response.ok) throw new BrowserRendererError("MASTERDATA_FETCH_FAILED", `Master-data fetch failed ${response.status}: ${url}`);
  return response.json();
}

function profileResourceRequests(preparation: Record<string, unknown>): Array<{ namespace: string; key: string }> {
  const resources = preparation.resources;
  if (!Array.isArray(resources)) throw new BrowserRendererError("INVALID_PROFILE_PREPARATION", "Profile preparation did not return a resource list");
  return resources.map((entry) => {
    if (!entry || typeof entry !== "object") throw new BrowserRendererError("INVALID_PROFILE_PREPARATION", "Profile resource entry is invalid");
    const resource = (entry as Record<string, unknown>).resource;
    if (!resource || typeof resource !== "object") throw new BrowserRendererError("INVALID_PROFILE_PREPARATION", "Profile resource key is missing");
    const { namespace, key } = resource as Record<string, unknown>;
    if (typeof namespace !== "string" || typeof key !== "string") throw new BrowserRendererError("INVALID_PROFILE_PREPARATION", "Profile resource identity is invalid");
    return { namespace, key };
  });
}

function assertPreparedResources(
  requested: Array<{ namespace: string; key: string }>,
  sources: Map<string, ImageBitmap>,
): void {
  for (const resource of requested) {
    if (!sources.has(`${resource.namespace}\0${resource.key}`)) {
      throw new BrowserRendererError("RESOURCE_PREPARATION_DRIFT", `Scene requested an unprepared resource: ${resource.namespace}/${resource.key}`);
    }
  }
}

export class BrowserScene {
  private destroyed = false;
  private readonly runtime: RendererRuntimeTelemetry;
  private readonly onDestroy: ((scene: BrowserScene) => void) | undefined;

  constructor(
    private readonly core: RendererScene,
    private readonly renderer: SemanticWebglSceneRenderer,
    private readonly resources: BrowserSemanticResourceSet<ImageBitmap>,
    private readonly atlas: SdfAtlas,
    options: {
      telemetry?: RendererTelemetryOptions;
      bootstrap: RendererRuntimeSnapshot["bootstrap"];
      onDestroy?: (scene: BrowserScene) => void;
    },
  ) {
    this.runtime = new RendererRuntimeTelemetry(options.telemetry ?? {}, atlasSummary(atlas), options.bootstrap);
    this.onDestroy = options.onDestroy;
  }

  draw() {
    this.assertAlive();
    this.assertGpuReady();
    const started = monotonicNow();
    const metrics = this.renderer.draw();
    this.runtime.recordDraw(metrics, monotonicNow() - started);
    return metrics;
  }

  async advance(tick: number) { this.assertGpuReady(); return this.apply(await this.core.advance(tick)); }
  async setLayerVisible(layerId: StableId, visible: boolean) { this.assertGpuReady(); return this.apply(await this.core.setLayerVisible(layerId, visible)); }
  async setLayerMasks(expectedRevision: number, overrides: Array<{ layerId: StableId; visible: boolean | null }>) {
    this.assertGpuReady();
    return this.apply(await this.core.setLayerMasks(expectedRevision, overrides));
  }
  async setTab(controlId: StableId, value: string) { this.assertGpuReady(); return this.apply(await this.core.setTab(controlId, value)); }
  async setScrollOffset(controlId: StableId, offset: number) { this.assertGpuReady(); return this.apply(await this.core.setScrollOffset(controlId, offset)); }
  async scrollBy(controlId: StableId, delta: number) { this.assertGpuReady(); return this.apply(await this.core.scrollBy(controlId, delta)); }

  async dump(): Promise<CoreSceneDump & { numeric_text_regions: NumericTextRegion[] }> {
    this.assertAlive();
    const dump = await this.core.dump();
    return { ...dump, numeric_text_regions: this.renderer.interactionRegions() };
  }

  async destroy(): Promise<void> {
    if (this.destroyed) return;
    this.destroyed = true;
    this.renderer.destroy();
    this.resources.release();
    this.atlas.release();
    await this.core.destroy();
    this.runtime.markDestroyed();
    this.onDestroy?.(this);
  }

  notifyContextLost(): void {
    if (this.destroyed) return;
    this.renderer.notifyContextLost();
    this.runtime.markContextLost();
  }

  notifyContextRestoreFailed(restoreMs: number): void {
    if (this.destroyed) return;
    this.runtime.markRestoreFailed(restoreMs);
  }

  async restoreContext(gl: WebGL2RenderingContext): Promise<void> {
    this.assertAlive();
    const started = monotonicNow();
    try {
      const uploads = await this.renderer.restoreContext(gl);
      this.runtime.markContextRestored(monotonicNow() - started, uploads);
    } catch (error) {
      this.runtime.markRestoreFailed(monotonicNow() - started);
      throw error;
    }
  }

  stats(): RendererRuntimeSnapshot {
    return this.runtime.snapshot();
  }

  private apply(delta: CoreSceneDelta) {
    this.assertAlive();
    const gpu = this.renderer.applyCoreDelta(delta);
    this.runtime.recordPatch(gpu);
    return { delta, gpu };
  }

  private assertAlive(): void {
    if (this.destroyed) throw new BrowserRendererError("SCENE_DESTROYED", "Browser scene is destroyed");
  }

  private assertGpuReady(): void {
    this.assertAlive();
    if (this.runtime.state() === "context-lost") {
      throw new BrowserRendererError("WEBGL_CONTEXT_LOST", "WebGL context is lost");
    }
  }
}

export class BrowserRendererError extends Error {
  constructor(readonly code: string, message: string) {
    super(message);
    this.name = "BrowserRendererError";
  }
}

function atlasSummary(atlas: SdfAtlas): RendererAtlasSummary {
  const cache = atlas.perf.cache;
  return {
    backend: atlas.sdfBackend,
    pages: atlas.depth,
    glyphs: atlas.glyphs.size,
    missingGlyphs: atlas.missing.length,
    generation: {
      glyphs: atlas.perf.totalGlyphCount,
      pixels: atlas.perf.totalPixelCount,
      glyphMs: atlas.perf.totalGlyphMs,
      faceLoadMs: atlas.perf.totalFaceLoadMs,
    },
    cache: cache ? {
      hits: cache.hits,
      misses: cache.misses,
      generations: cache.generations,
      bytes: cache.bytes,
      sessionHits: cache.sessionHits,
      persistentHits: cache.persistentHits,
      persistentMisses: cache.persistentMisses,
      persistentWritesQueued: cache.persistentWritesQueued,
      pinnedPages: cache.pinnedPages,
      pageEvictions: cache.pageEvictions,
    } : null,
  };
}

function monotonicNow(): number {
  return typeof performance === "undefined" ? Date.now() : performance.now();
}

async function sha256Hex(bytes: ArrayBuffer): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  return Array.from(digest, (value) => value.toString(16).padStart(2, "0")).join("");
}

function isHtmlCanvas(canvas: HTMLCanvasElement | OffscreenCanvas): canvas is HTMLCanvasElement {
  return typeof HTMLCanvasElement !== "undefined" && canvas instanceof HTMLCanvasElement;
}
