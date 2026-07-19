import {
  type GlyphBatchRequest,
  type LayerMaskOverride,
  type RegisteredFont,
  type RendererFontContract,
  type RendererWorkerRequest,
  type RendererWorkerResponse,
  type RendererWorkerResult,
  type RendererWorkerStats,
} from "./protocol.js";
import type {
  CoreSceneCreateResponse,
  CoreSceneDelta,
  CoreSceneDump,
  StableId,
} from "./types/core.js";
import type { FreeTypeGlyphBatch, FreeTypeGlyphMapBatch } from "./types/freeType.js";
import type { WasmGlyphDemandBatch, WasmLayoutBatch } from "./types/layout.js";
import type {
  AtlasGenerateRequest,
  AtlasGlyphRecord,
  AtlasPageUpdate,
  AtlasResolveResult,
  AtlasStats,
  GlyphRasterPlan,
} from "./types/atlas.js";
import type { AuthoringCheckpoint, AuthoringCommand, AuthoringDelta, AuthoringSelection, GameProfileDocument } from "./types/authoring.js";

export type RendererWorkerClientOptions = {
  workerUrl?: string | URL;
  moduleUrl?: string | URL;
  wasmUrl?: string | URL;
  workerFactory?: (url: string | URL) => Worker;
};

type Pending = {
  resolve(value: RendererWorkerResult): void;
  reject(reason: unknown): void;
};

export class RendererWorkerClient {
  private readonly worker: Worker;
  private readonly pending = new Map<number, Pending>();
  private sequence = 0;
  private closed = false;

  private constructor(worker: Worker) {
    this.worker = worker;
    worker.onmessage = (event: MessageEvent<RendererWorkerResponse>) => this.receive(event.data);
    worker.onerror = (event) => this.failAll(new RendererWorkerError("WORKER_CRASHED", event.message));
  }

  static async create(options: RendererWorkerClientOptions = {}): Promise<RendererWorkerClient> {
    const workerUrl = options.workerUrl ?? new URL("./worker.js", import.meta.url);
    const moduleUrl = options.moduleUrl ?? new URL("./allium_renderer_wasm.js", import.meta.url);
    const wasmUrl = options.wasmUrl ?? new URL("./allium_renderer_wasm.wasm", import.meta.url);
    const worker = options.workerFactory?.(workerUrl) ?? new Worker(workerUrl, { type: "module", name: "sekai-custom-profile-sdk" });
    const client = new RendererWorkerClient(worker);
    const result = await client.request("init", {
      moduleUrl: moduleUrl.toString(),
      wasmUrl: wasmUrl.toString(),
    });
    if (result.kind !== "init") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid init response");
    return client;
  }

  async registerFont(font: RegisteredFont): Promise<void> {
    const bytes = font.bytes.slice(0);
    const result = await this.request("registerFont", { ...font, bytes }, [bytes]);
    if (result.kind !== "registerFont" || !result.registered) throw new RendererWorkerError("FONT_REGISTRATION_FAILED", "Renderer worker rejected the font");
  }

  async contract(): Promise<RendererFontContract> {
    const result = await this.request("contract", {});
    if (result.kind !== "contract") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid contract response");
    return result.contract;
  }

  async mapGlyphs(request: GlyphBatchRequest): Promise<FreeTypeGlyphMapBatch> {
    const result = await this.request("mapGlyphs", request);
    if (result.kind !== "mapGlyphs") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid glyph-map response");
    return result.batch;
  }

  async planGlyphs(request: GlyphBatchRequest): Promise<GlyphRasterPlan> {
    const result = await this.request("planGlyphs", request);
    if (result.kind !== "planGlyphs") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid glyph-plan response");
    return result.plan;
  }

  async buildGlyphs(request: GlyphBatchRequest): Promise<FreeTypeGlyphBatch> {
    const result = await this.request("buildGlyphs", request);
    if (result.kind !== "buildGlyphs") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid glyph response");
    return result.batch;
  }

  async layoutText(request: unknown): Promise<WasmLayoutBatch> {
    const result = await this.request("layoutText", { request });
    if (result.kind !== "layoutText") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid text-layout response");
    return result.batch;
  }

  async glyphDemand(request: unknown): Promise<WasmGlyphDemandBatch> {
    const result = await this.request("glyphDemand", { request });
    if (result.kind !== "glyphDemand") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid glyph-demand response");
    return result.batch;
  }

  async createAtlas(): Promise<RendererAtlas> {
    const result = await this.request("createAtlas", {});
    if (result.kind !== "createAtlas") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid atlas response");
    return new RendererAtlas(this, result.atlasId, result.stats);
  }

  async createAuthoringDocument(profile?: unknown): Promise<RendererAuthoringDocument> {
    const result = profile === undefined
      ? await this.request("createAuthoringBlank", {})
      : await this.request("importAuthoringProfile", { profile });
    if (result.kind !== "createAuthoring") {
      throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid authoring response");
    }
    return new RendererAuthoringDocument(this, result.authoringId, result.document, result.revision);
  }

  async restoreAuthoringDocument(checkpoint: AuthoringCheckpoint): Promise<RendererAuthoringDocument> {
    const result = await this.request("restoreAuthoringCheckpoint", { checkpoint });
    if (result.kind !== "createAuthoring") {
      throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid restored authoring response");
    }
    return new RendererAuthoringDocument(this, result.authoringId, result.document, result.revision);
  }

  async createMasterData(region: string, revision: string): Promise<RendererMasterData> {
    const result = await this.request("createMasterData", { region, revision });
    if (result.kind !== "createMasterData") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid master-data response");
    const requiredTables = Array.isArray(result.report.required_tables)
      ? result.report.required_tables.filter((value): value is string => typeof value === "string")
      : [];
    return new RendererMasterData(this, result.masterDataId, region, revision, requiredTables);
  }

  async stats(): Promise<RendererWorkerStats> {
    const result = await this.request("stats", {});
    if (result.kind !== "stats") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid stats response");
    return result.stats;
  }

  terminate(): void {
    if (this.closed) return;
    this.closed = true;
    this.worker.terminate();
    this.failAll(new RendererWorkerError("WORKER_TERMINATED", "Renderer worker was terminated"));
  }

  async sceneRequest(
    kind: RendererWorkerRequest["kind"],
    payload: unknown,
  ): Promise<RendererWorkerResult> {
    return this.request(kind, payload);
  }

  private request(
    kind: RendererWorkerRequest["kind"],
    payload: unknown,
    transfers: Transferable[] = [],
  ): Promise<RendererWorkerResult> {
    if (this.closed) return Promise.reject(new RendererWorkerError("WORKER_TERMINATED", "Renderer worker is closed"));
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.worker.postMessage({ id, kind, payload } as RendererWorkerRequest, transfers);
    });
  }

  private receive(response: RendererWorkerResponse): void {
    const pending = this.pending.get(response.id);
    if (!pending) return;
    this.pending.delete(response.id);
    if (response.ok) pending.resolve(response.result);
    else pending.reject(new RendererWorkerError(response.error.code, response.error.message));
  }

  private failAll(error: Error): void {
    for (const pending of this.pending.values()) pending.reject(error);
    this.pending.clear();
  }
}

export class RendererAuthoringDocument {
  private destroyed = false;

  constructor(
    private readonly client: RendererWorkerClient,
    readonly id: string,
    readonly initialDocument: GameProfileDocument,
    readonly initialRevision: number,
  ) {}

  async apply(command: AuthoringCommand): Promise<AuthoringDelta> {
    this.assertAlive();
    const result = await this.client.sceneRequest("applyAuthoring", { authoringId: this.id, command });
    if (result.kind !== "authoringDelta" || result.delta == null) {
      throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid authoring delta");
    }
    return result.delta;
  }

  async select(id: number | null): Promise<AuthoringDelta> {
    return this.delta("selectAuthoring", { authoringId: this.id, id });
  }

  async elements(): Promise<AuthoringSelection[]> {
    this.assertAlive();
    const result = await this.client.sceneRequest("elementsAuthoring", { authoringId: this.id });
    if (result.kind !== "elementsAuthoring") {
      throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned invalid authoring elements");
    }
    return result.elements;
  }

  async beginGesture(id: number): Promise<AuthoringDelta> {
    return this.delta("beginAuthoringGesture", { authoringId: this.id, id });
  }

  async previewGesture(command: Extract<AuthoringCommand, { kind: "set_transform" | "set_parameters" }>): Promise<AuthoringDelta> {
    return this.delta("previewAuthoringGesture", { authoringId: this.id, command });
  }

  async commitGesture(): Promise<AuthoringDelta> {
    return this.delta("commitAuthoringGesture", { authoringId: this.id });
  }

  async cancelGesture(): Promise<AuthoringDelta> {
    return this.delta("cancelAuthoringGesture", { authoringId: this.id });
  }

  async appendPage(): Promise<AuthoringDelta> {
    return this.delta("appendAuthoringPage", { authoringId: this.id });
  }

  async duplicatePage(page: number): Promise<AuthoringDelta> {
    return this.delta("duplicateAuthoringPage", { authoringId: this.id, page });
  }

  async deletePage(page: number): Promise<AuthoringDelta> {
    return this.delta("deleteAuthoringPage", { authoringId: this.id, page });
  }

  async movePage(fromPage: number, page: number): Promise<AuthoringDelta> {
    return this.delta("moveAuthoringPage", { authoringId: this.id, fromPage, page });
  }

  async undo(): Promise<AuthoringDelta | null> {
    return this.history("undoAuthoring");
  }

  async redo(): Promise<AuthoringDelta | null> {
    return this.history("redoAuthoring");
  }

  async export(): Promise<GameProfileDocument> {
    this.assertAlive();
    const result = await this.client.sceneRequest("exportAuthoring", { authoringId: this.id });
    if (result.kind !== "exportAuthoring") {
      throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid authoring export");
    }
    return result.document;
  }

  async checkpoint(): Promise<AuthoringCheckpoint> {
    this.assertAlive();
    const result = await this.client.sceneRequest("checkpointAuthoring", { authoringId: this.id });
    if (result.kind !== "checkpointAuthoring") {
      throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid authoring checkpoint");
    }
    return result.checkpoint;
  }

  async destroy(): Promise<void> {
    if (this.destroyed) return;
    const result = await this.client.sceneRequest("destroyAuthoring", { authoringId: this.id });
    if (result.kind !== "destroyAuthoring" || !result.destroyed) {
      throw new RendererWorkerError("AUTHORING_DESTROY_FAILED", "Renderer worker did not destroy the authoring session");
    }
    this.destroyed = true;
  }

  private async history(kind: "undoAuthoring" | "redoAuthoring"): Promise<AuthoringDelta | null> {
    this.assertAlive();
    const result = await this.client.sceneRequest(kind, { authoringId: this.id });
    if (result.kind !== "authoringDelta") {
      throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid authoring history response");
    }
    return result.delta;
  }

  private async delta(
    kind: "selectAuthoring" | "beginAuthoringGesture" | "previewAuthoringGesture" | "commitAuthoringGesture" | "cancelAuthoringGesture" | "appendAuthoringPage" | "duplicateAuthoringPage" | "deleteAuthoringPage" | "moveAuthoringPage",
    payload: unknown,
  ): Promise<AuthoringDelta> {
    this.assertAlive();
    const result = await this.client.sceneRequest(kind, payload);
    if (result.kind !== "authoringDelta" || result.delta == null) {
      throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid authoring delta");
    }
    return result.delta;
  }

  private assertAlive(): void {
    if (this.destroyed) throw new RendererWorkerError("AUTHORING_DESTROYED", "Renderer authoring session is destroyed");
  }
}

export class RendererAtlas {
  private destroyed = false;

  constructor(
    private readonly client: RendererWorkerClient,
    readonly id: string,
    readonly initialStats: AtlasStats,
  ) {}

  async resolve(keys: string[], cached: AtlasGlyphRecord[], generate: AtlasGenerateRequest[]): Promise<AtlasResolveResult> {
    this.assertAlive();
    const result = await this.client.sceneRequest("resolveAtlas", { atlasId: this.id, keys, cached, generate });
    if (result.kind !== "resolveAtlas") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid atlas resolution");
    return result.result;
  }

  async pages(revisions: ReadonlyMap<number, number>): Promise<AtlasPageUpdate[]> {
    this.assertAlive();
    const result = await this.client.sceneRequest("atlasPages", {
      atlasId: this.id,
      revisions: [...revisions].map(([page, revision]) => ({ page, revision })),
    });
    if (result.kind !== "atlasPages") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned invalid atlas pages");
    return result.updates;
  }

  async release(lease: number): Promise<void> {
    if (this.destroyed) return;
    const result = await this.client.sceneRequest("releaseAtlas", { atlasId: this.id, lease });
    if (result.kind !== "releaseAtlas") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid atlas lease response");
  }

  async destroy(): Promise<void> {
    if (this.destroyed) return;
    const result = await this.client.sceneRequest("destroyAtlas", { atlasId: this.id });
    if (result.kind !== "destroyAtlas" || !result.destroyed) throw new RendererWorkerError("ATLAS_DESTROY_FAILED", "Renderer worker did not destroy the atlas session");
    this.destroyed = true;
  }

  private assertAlive(): void {
    if (this.destroyed) throw new RendererWorkerError("ATLAS_DESTROYED", "Renderer atlas session is destroyed");
  }
}

export class RendererMasterData {
  private destroyed = false;

  constructor(
    private readonly client: RendererWorkerClient,
    readonly id: string,
    readonly region: string,
    readonly revision: string,
    readonly requiredTables: readonly string[],
  ) {}

  async putTable(name: string, table: unknown): Promise<Record<string, unknown>> {
    return this.report("putMasterDataTable", { masterDataId: this.id, name, table });
  }

  async seal(): Promise<Record<string, unknown>> {
    return this.report("sealMasterData", { masterDataId: this.id });
  }

  async prepareProfile(request: unknown): Promise<Record<string, unknown>> {
    this.assertAlive();
    const result = await this.client.sceneRequest("prepareProfile", { masterDataId: this.id, request });
    if (result.kind !== "prepareProfile") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid profile preparation");
    return result.preparation;
  }

  /** @internal Profile compilation stays behind BrowserRenderer. */
  async createProfileScene(request: unknown, layoutRequest: unknown): Promise<{
    scene: RendererScene;
    layout: WasmLayoutBatch;
  }> {
    this.assertAlive();
    const result = await this.client.sceneRequest("createProfileScene", { masterDataId: this.id, request, layoutRequest });
    if (result.kind !== "createProfileScene") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid profile scene");
    return {
      scene: new RendererScene(this.client, result.sceneId, result.response),
      layout: result.layout,
    };
  }

  async destroy(): Promise<void> {
    if (this.destroyed) return;
    const result = await this.client.sceneRequest("destroyMasterData", { masterDataId: this.id });
    if (result.kind !== "destroyMasterData" || !result.destroyed) throw new RendererWorkerError("MASTERDATA_DESTROY_FAILED", "Renderer worker did not destroy the master-data session");
    this.destroyed = true;
  }

  private async report(kind: "putMasterDataTable" | "sealMasterData", payload: unknown): Promise<Record<string, unknown>> {
    this.assertAlive();
    const result = await this.client.sceneRequest(kind, payload);
    if (result.kind !== "masterDataReport") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid master-data report");
    return result.report;
  }

  private assertAlive(): void {
    if (this.destroyed) throw new RendererWorkerError("MASTERDATA_DESTROYED", "Renderer master-data session is destroyed");
  }
}

export class RendererScene {
  private destroyed = false;

  constructor(
    private readonly client: RendererWorkerClient,
    readonly id: string,
    readonly initial: CoreSceneCreateResponse,
  ) {}

  async advance(tick: number): Promise<CoreSceneDelta> {
    return this.delta("advance", { sceneId: this.id, tick });
  }

  async setLayerVisible(layerId: StableId, visible: boolean): Promise<CoreSceneDelta> {
    return this.delta("setLayerMask", { sceneId: this.id, layerId, visible });
  }

  async setLayerMasks(expectedLayerTableRevision: number, overrides: LayerMaskOverride[]): Promise<CoreSceneDelta> {
    return this.delta("setLayerMasks", { sceneId: this.id, expectedLayerTableRevision, overrides });
  }

  async setTab(controlId: StableId, value: string): Promise<CoreSceneDelta> {
    return this.delta("setTab", { sceneId: this.id, controlId, value });
  }

  async setScrollOffset(controlId: StableId, offset: number): Promise<CoreSceneDelta> {
    return this.delta("scroll", { sceneId: this.id, controlId, offset });
  }

  async scrollBy(controlId: StableId, delta: number): Promise<CoreSceneDelta> {
    return this.delta("scroll", { sceneId: this.id, controlId, delta });
  }

  async dump(): Promise<CoreSceneDump> {
    this.assertAlive();
    const result = await this.client.sceneRequest("dumpScene", { sceneId: this.id });
    if (result.kind !== "dumpScene") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid dump response");
    return result.dump;
  }

  async destroy(): Promise<void> {
    if (this.destroyed) return;
    const result = await this.client.sceneRequest("destroyScene", { sceneId: this.id });
    if (result.kind !== "destroyScene" || !result.destroyed) throw new RendererWorkerError("SCENE_DESTROY_FAILED", "Renderer worker did not destroy the scene");
    this.destroyed = true;
  }

  private async delta(
    kind: "advance" | "setLayerMask" | "setLayerMasks" | "setTab" | "scroll",
    payload: unknown,
  ): Promise<CoreSceneDelta> {
    this.assertAlive();
    const result = await this.client.sceneRequest(kind, payload);
    if (result.kind !== "delta") throw new RendererWorkerError("PROTOCOL_MISMATCH", "Renderer worker returned an invalid scene delta");
    return result.delta;
  }

  private assertAlive(): void {
    if (this.destroyed) throw new RendererWorkerError("SCENE_DESTROYED", "Renderer scene is destroyed");
  }
}

export class RendererWorkerError extends Error {
  constructor(readonly code: string, message: string) {
    super(message);
    this.name = "RendererWorkerError";
  }
}
