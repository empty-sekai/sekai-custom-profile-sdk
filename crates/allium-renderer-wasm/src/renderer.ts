import {
  BrowserSemanticResourceManager,
  type BrowserImageSource,
  type BrowserSemanticResourceSet,
} from "./gpu/browserSemanticResources.js";
import {
  profileResourceDescriptors,
  type ResourceProvider,
} from "./resourceProvider.js";
import {
  LocalizationProviderManager,
  type LocalizationProvider,
  type LocalizationRequest,
} from "./localizationProvider.js";
import {
  FontProviderManager,
  type FontProvider,
  type FontRequest,
} from "./fontProvider.js";
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
  buildPrebuiltSdfAtlas,
  type PrebuiltSdfAtlasProvider,
} from "./prebuiltSdfAtlas.js";
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
  resourceProvider: ResourceProvider;
  resourceConcurrency?: number;
  localizationProvider?: LocalizationProvider;
  localizationConcurrency?: number;
  fontProvider?: FontProvider;
  fontConcurrency?: number;
  prebuiltSdfAtlasProvider?: PrebuiltSdfAtlasProvider;
  telemetry?: RendererTelemetryOptions;
};

export type MasterDataTableRequest = {
  table: string;
  region: string;
  revision: string;
};

export type MasterDataTableLoader = (
  request: MasterDataTableRequest,
  context: { signal: AbortSignal },
) => Promise<unknown>;

export type ProfileSceneCreateOptions = {
  masterData: RendererMasterData;
  documentKey: string;
  card: unknown;
  profile?: unknown;
  locale?: string;
  frameMode?: "final" | "animate";
  signal?: AbortSignal;
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
  private readonly localizations: LocalizationProviderManager | null;
  private readonly providedFonts: FontProviderManager | null;
  private readonly fonts = new Map<string, FontSource>();
  private readonly fontRegistrations = new Map<string, Promise<void>>();
  private readonly scenes = new Set<BrowserScene>();
  private readonly canvas: HTMLCanvasElement | OffscreenCanvas;
  private readonly telemetryOptions: RendererTelemetryOptions;
  private contextLost = false;
  private restorePromise: Promise<void> | null = null;
  private readonly lifetime = new AbortController();
  private destroyed = false;

  private constructor(
    private gl: WebGL2RenderingContext,
    private readonly worker: RendererWorkerClient,
    private readonly region: string,
    canvas: HTMLCanvasElement | OffscreenCanvas,
    resourceProvider: ResourceProvider,
    resourceConcurrency?: number,
    localizationProvider?: LocalizationProvider,
    localizationConcurrency?: number,
    fontProvider?: FontProvider,
    fontConcurrency?: number,
    private readonly prebuiltSdfAtlasProvider?: PrebuiltSdfAtlasProvider,
    telemetryOptions: RendererTelemetryOptions = {},
  ) {
    this.canvas = canvas;
    this.telemetryOptions = telemetryOptions;
    this.resources = new BrowserSemanticResourceManager({
      provider: resourceProvider,
      concurrency: resourceConcurrency,
    });
    this.localizations = localizationProvider
      ? new LocalizationProviderManager({
        provider: localizationProvider,
        concurrency: localizationConcurrency,
      })
      : null;
    this.providedFonts = fontProvider
      ? new FontProviderManager({
        provider: fontProvider,
        concurrency: fontConcurrency,
      })
      : null;
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
      options.resourceProvider,
      options.resourceConcurrency,
      options.localizationProvider,
      options.localizationConcurrency,
      options.fontProvider,
      options.fontConcurrency,
      options.prebuiltSdfAtlasProvider,
      options.telemetry,
    );
  }

  async createProfileScene(options: ProfileSceneCreateOptions): Promise<BrowserScene> {
    this.assertAlive();
    let resources: BrowserSemanticResourceSet | null = null;
    let core: RendererScene | null = null;
    let atlas: SdfAtlas | null = null;
    const abort = combinedAbortSignal(this.lifetime.signal, options.signal);
    try {
      const locale = options.locale ?? this.region;
      let localizedText: Record<string, string> | undefined;
      if (this.localizations) {
        const demandPreparation = await options.masterData.prepareProfile({
          documentKey: options.documentKey,
          card: options.card,
          profile: options.profile,
          locale,
          demandOnly: true,
        });
        localizedText = await this.localizations.resolve(
          preparedLocalizationDemands(demandPreparation),
          abort.signal,
        );
      }
      if (this.providedFonts) {
        const fontPreparation = await options.masterData.prepareProfile({
          documentKey: options.documentKey,
          card: options.card,
          profile: options.profile,
          locale,
          localizedText,
          fontDemandOnly: true,
        });
        const missing = preparedFontDemands(fontPreparation, this.region)
          .filter((request) => !this.fonts.has(request.family));
        const provided = await this.providedFonts.resolve(missing, abort.signal);
        await Promise.all([...provided].map(([family, bytes]) => this.registerFont({ family, bytes })));
      }
      const preparation = await options.masterData.prepareProfile({
        documentKey: options.documentKey,
        card: options.card,
        profile: options.profile,
        locale,
        localizedText,
      });
      const glyphRequests = preparedGlyphRequests(preparation);
      if (glyphRequests.length === 0) {
        atlas = null;
      } else {
        atlas = this.prebuiltSdfAtlasProvider
          ? await buildPrebuiltSdfAtlas(this.prebuiltSdfAtlasProvider, glyphRequests, abort.signal)
          : null;
        atlas ??= await buildSdfAtlas(
          requiredFontSources(glyphRequests, this.fonts),
          glyphRequests,
          { worker: this.worker, persistence: options.sdf?.persistence ?? "origin" },
          options.sdf?.supersample,
          options.sdf?.backend,
        );
      }
      const requestedResources = profileResourceDescriptors(preparation);
      let acquired: BrowserSemanticResourceSet;
      acquired = await this.resources.acquire(requestedResources, abort.signal);
      resources = acquired;
      const compiled = await options.masterData.createProfileScene({
        documentKey: options.documentKey,
        card: options.card,
        profile: options.profile,
        locale,
        localizedText,
        frameMode: options.frameMode ?? "animate",
        resourceMetrics: requestedResources.map((resource) => {
          const image = acquired.sources.get(resource.id);
          return {
            namespace: resource.namespace,
            key: resource.key,
            width: image?.width ?? 0,
            height: image?.height ?? 0,
            available: acquired.availability.get(resource.id) ?? false,
          };
        }),
      }, preparedLayoutRequest(preparation, atlas));
      core = compiled.scene;
      const plan = semanticCommandPlanFromCoreSnapshot(
        core.initial.snapshot as unknown as Parameters<typeof semanticCommandPlanFromCoreSnapshot>[0],
      );
      assertPreparedResources(plan.resourceRequests(), acquired.sources);
      const renderer = new SemanticWebglSceneRenderer(this.gl);
      const bootstrap = await renderer.setScene({
        plan,
        atlas,
        layout: compiled.layout,
        imageSources: acquired.sources,
      });
      const scene = new BrowserScene(core, renderer, acquired, atlas, {
        dynamicProgramCount: compiled.layout.dynamicPrograms.length,
        canvas: this.canvas,
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
    } finally {
      abort.dispose();
    }
  }

  async createMasterData(revision: string): Promise<RendererMasterData> {
    this.assertAlive();
    return this.worker.createMasterData(this.region, revision);
  }

  async loadMasterData(
    revision: string,
    loadTable: MasterDataTableLoader,
    options: { signal?: AbortSignal; concurrency?: number } = {},
  ): Promise<RendererMasterData> {
    if (typeof loadTable !== "function") throw new BrowserRendererError("MASTERDATA_LOADER_REQUIRED", "A master-data table loader is required");
    const session = await this.createMasterData(revision);
    const signal = options.signal ?? new AbortController().signal;
    const concurrency = options.concurrency ?? 4;
    if (!Number.isInteger(concurrency) || concurrency <= 0) {
      await session.destroy().catch(() => undefined);
      throw new BrowserRendererError("INVALID_MASTERDATA_CONCURRENCY", "Master-data concurrency must be a positive integer");
    }
    try {
      let cursor = 0;
      const workers = Array.from({ length: Math.min(concurrency, session.requiredTables.length) }, async () => {
        while (cursor < session.requiredTables.length) {
          if (signal.aborted) throw abortReason(signal);
          const table = session.requiredTables[cursor++];
          const value = await loadTable({ table, region: this.region, revision }, { signal });
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
    if (typeof font.family !== "string" || font.family.length === 0) {
      throw new BrowserRendererError("INVALID_FONT_FAMILY", "Font family must be a non-empty string");
    }
    if (!(font.bytes instanceof ArrayBuffer) || font.bytes.byteLength === 0) {
      throw new BrowserRendererError("INVALID_FONT_BYTES", `Font bytes are empty: ${font.family}`);
    }
    const sourceHash = await sha256Hex(font.bytes);
    const pending = this.fontRegistrations.get(font.family);
    if (pending) await pending;
    const registered = this.fonts.get(font.family);
    if (registered) {
      if (registered.sourceHash === sourceHash) return;
      throw new BrowserRendererError("FONT_IDENTITY_CONFLICT", `Font family is already registered with different bytes: ${font.family}`);
    }
    const source = { region: this.region, family: font.family, sourceHash };
    const registration = this.worker.registerFont({ ...source, bytes: font.bytes })
      .then(() => { this.fonts.set(font.family, source); });
    this.fontRegistrations.set(font.family, registration);
    try {
      await registration;
    } finally {
      this.fontRegistrations.delete(font.family);
    }
  }

  async stats() {
    if (this.destroyed) throw new BrowserRendererError("RENDERER_DESTROYED", "Browser renderer is destroyed");
    return {
      worker: await this.worker.stats(),
      resources: this.resources.stats(),
      fonts: {
        registered: this.fonts.size,
        provider: this.providedFonts?.stats() ?? null,
      },
      localizations: this.localizations?.stats() ?? null,
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
    this.lifetime.abort(new BrowserRendererError("RENDERER_DESTROYED", "Browser renderer is destroyed"));
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

function preparedFontDemands(
  preparation: Record<string, unknown>,
  region: string,
): FontRequest[] {
  const values = preparation.fontDemands;
  if (!Array.isArray(values)) {
    throw new BrowserRendererError("INVALID_PROFILE_PREPARATION", "Profile preparation did not return font demands");
  }
  return values.map((family, index) => {
    if (typeof family !== "string" || family.length === 0) {
      throw new BrowserRendererError("INVALID_PROFILE_PREPARATION", `Profile font demand is invalid at index ${index}`);
    }
    return { region, family };
  });
}

function preparedLayoutRequest(
  preparation: Record<string, unknown>,
  atlas: SdfAtlas | null,
): Record<string, unknown> {
  const request = preparation.layout_request;
  if (!request || typeof request !== "object" || Array.isArray(request)) {
    throw new BrowserRendererError("INVALID_PROFILE_PREPARATION", "Profile preparation did not return an authored layout request");
  }
  if (!atlas) {
    const layers = (request as { layers?: unknown }).layers;
    if (!Array.isArray(layers) || layers.length !== 0) {
      throw new BrowserRendererError("INVALID_PROFILE_PREPARATION", "Profile preparation returned text layers without glyph demand");
    }
    return {
      ...(request as Record<string, unknown>),
      atlas: { baseSize: 1, spread: 0, glyphs: [] },
    };
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

function assertPreparedResources(
  requested: Array<{ namespace: string; key: string }>,
  sources: Map<string, BrowserImageSource>,
): void {
  for (const resource of requested) {
    if (!sources.has(`${resource.namespace}\0${resource.key}`)) {
      throw new BrowserRendererError("RESOURCE_PREPARATION_DRIFT", `Scene requested an unprepared resource: ${resource.namespace}/${resource.key}`);
    }
  }
}

function preparedLocalizationDemands(preparation: Record<string, unknown>): LocalizationRequest[] {
  const raw = preparation.localizationDemands;
  if (!Array.isArray(raw)) return [];
  return raw.map((value, index) => {
    if (!value || typeof value !== "object" || Array.isArray(value)) {
      throw new BrowserRendererError("INVALID_LOCALIZATION_DEMAND", `Invalid localization demand at index ${index}`);
    }
    const { region, locale, key } = value as Record<string, unknown>;
    if (typeof region !== "string" || typeof locale !== "string" || typeof key !== "string") {
      throw new BrowserRendererError("INVALID_LOCALIZATION_DEMAND", `Invalid localization demand at index ${index}`);
    }
    return { region, locale, key };
  });
}

function combinedAbortSignal(lifetime: AbortSignal, request?: AbortSignal): { signal: AbortSignal; dispose(): void } {
  if (!request || lifetime.aborted || request.aborted) {
    return { signal: request?.aborted ? request : lifetime, dispose() {} };
  }
  const controller = new AbortController();
  const abortLifetime = () => controller.abort(lifetime.reason);
  const abortRequest = () => controller.abort(request.reason);
  lifetime.addEventListener("abort", abortLifetime, { once: true });
  request.addEventListener("abort", abortRequest, { once: true });
  return {
    signal: controller.signal,
    dispose() {
      lifetime.removeEventListener("abort", abortLifetime);
      request.removeEventListener("abort", abortRequest);
    },
  };
}

function abortReason(signal: AbortSignal): unknown {
  return signal.reason ?? new DOMException("The operation was aborted", "AbortError");
}

export class BrowserScene {
  private destroyed = false;
  private readonly runtime: RendererRuntimeTelemetry;
  private readonly onDestroy: ((scene: BrowserScene) => void) | undefined;

  constructor(
    private readonly core: RendererScene,
    private readonly renderer: SemanticWebglSceneRenderer,
    private readonly resources: BrowserSemanticResourceSet,
    private readonly atlas: SdfAtlas | null,
    options: {
      dynamicProgramCount: number;
      canvas: HTMLCanvasElement | OffscreenCanvas;
      telemetry?: RendererTelemetryOptions;
      bootstrap: RendererRuntimeSnapshot["bootstrap"];
      onDestroy?: (scene: BrowserScene) => void;
    },
  ) {
    this.dynamicProgramCount = options.dynamicProgramCount;
    this.animated = this.dynamicProgramCount > 0;
    this.canvas = options.canvas;
    this.runtime = new RendererRuntimeTelemetry(options.telemetry ?? {}, atlasSummary(atlas), options.bootstrap);
    this.onDestroy = options.onDestroy;
  }

  /** Whether this scene owns renderer-driven programs that require timeline advancement. */
  readonly animated: boolean;

  /** Number of renderer-compiled dynamic programs retained by this scene. */
  readonly dynamicProgramCount: number;

  private readonly canvas: HTMLCanvasElement | OffscreenCanvas;

  draw() {
    this.assertAlive();
    this.assertGpuReady();
    const started = monotonicNow();
    const metrics = this.renderer.draw();
    this.runtime.recordDraw(metrics, monotonicNow() - started);
    return metrics;
  }

  /** Draws the current frame and encodes an exact 1830×812 PNG snapshot. */
  async exportPng(): Promise<Blob> {
    this.assertAlive();
    this.draw();
    if (typeof document !== "undefined") {
      const snapshot = document.createElement("canvas");
      snapshot.width = CARD_WIDTH;
      snapshot.height = CARD_HEIGHT;
      const context = snapshot.getContext("2d");
      if (!context) throw new BrowserRendererError("PNG_EXPORT_UNAVAILABLE", "Canvas 2D is required for PNG export");
      context.drawImage(this.canvas, 0, 0, CARD_WIDTH, CARD_HEIGHT);
      return new Promise<Blob>((resolve, reject) => {
        snapshot.toBlob((blob: Blob | null) => blob
          ? resolve(blob)
          : reject(new BrowserRendererError("PNG_EXPORT_FAILED", "Canvas PNG encoding returned no data")), "image/png");
      });
    }
    if (typeof OffscreenCanvas === "undefined") {
      throw new BrowserRendererError("PNG_EXPORT_UNAVAILABLE", "OffscreenCanvas is required outside a document");
    }
    const snapshot = new OffscreenCanvas(CARD_WIDTH, CARD_HEIGHT);
    const context = snapshot.getContext("2d");
    if (!context) throw new BrowserRendererError("PNG_EXPORT_UNAVAILABLE", "Canvas 2D is required for PNG export");
    context.drawImage(this.canvas, 0, 0, CARD_WIDTH, CARD_HEIGHT);
    return snapshot.convertToBlob({ type: "image/png" });
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

  setLayerPreviewTransform(layerId: StableId, matrix: [number, number, number, number, number, number] | null) {
    this.assertGpuReady();
    const gpu = this.renderer.setLayerPreviewTransform(layerId, matrix);
    const draw = this.draw();
    return { gpu, draw };
  }

  async dump(): Promise<CoreSceneDump & { numeric_text_regions: NumericTextRegion[] }> {
    this.assertAlive();
    const dump = await this.core.dump();
    const textGeometry = this.renderer.authoredTextHitGeometry();
    const interactionRegions = dump.interaction_regions.map((region) => {
      const geometry = region.role === "primary" ? textGeometry.get(region.layer_id) : undefined;
      return geometry ? {
        ...region,
        hit_geometry: geometry.quad,
      } : region;
    });
    return {
      ...dump,
      interaction_regions: interactionRegions,
      numeric_text_regions: this.renderer.interactionRegions(),
    };
  }

  async destroy(): Promise<void> {
    if (this.destroyed) return;
    this.destroyed = true;
    this.renderer.destroy();
    this.resources.release();
    await this.atlas?.release();
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

function atlasSummary(atlas: SdfAtlas | null): RendererAtlasSummary {
  if (!atlas) {
    return {
      backend: "edt",
      pages: 0,
      glyphs: 0,
      missingGlyphs: 0,
      generation: { glyphs: 0, pixels: 0, glyphMs: 0, faceLoadMs: 0 },
      cache: null,
    };
  }
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
