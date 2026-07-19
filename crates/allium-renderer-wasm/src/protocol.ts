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
  GlyphRasterPlan,
  AtlasResolveResult,
  AtlasStats,
} from "./types/atlas.js";
import type { AuthoringCheckpoint, AuthoringCommand, AuthoringDelta, AuthoringSelection, GameProfileDocument } from "./types/authoring.js";

export const RENDERER_WORKER_PROTOCOL = "allium.renderer-worker/2" as const;

export type RendererWorkerInit = {
  moduleUrl: string;
  wasmUrl?: string;
};

export type RegisteredFont = {
  region: string;
  family: string;
  sourceHash: string;
  bytes: ArrayBuffer;
};

export type GlyphBatchRequest = {
  region: string;
  family: string;
  sourceHash: string;
  chars: string[];
  backend?: "edt" | "analytic";
  supersample?: number;
};

export type RendererFontContract = {
  font_engine_fingerprint: string;
  freetype_version: string;
  modules: string[];
  load_contract: string;
};

export type LayerMaskOverride = {
  layerId: StableId;
  visible: boolean | null;
};

export type RendererWorkerStats = {
  protocol: typeof RENDERER_WORKER_PROTOCOL;
  initialized: boolean;
  scenes: number;
  masterDataSessions: number;
  atlasSessions: number;
  authoringSessions: number;
  fonts: number;
  requests: number;
  failures: number;
  wasmMs: number;
  bridgeBytes: number;
};

export type RendererWorkerRequest =
  | { id: number; kind: "init"; payload: RendererWorkerInit }
  | { id: number; kind: "contract"; payload: Record<string, never> }
  | { id: number; kind: "registerFont"; payload: RegisteredFont }
  | { id: number; kind: "mapGlyphs"; payload: GlyphBatchRequest }
  | { id: number; kind: "planGlyphs"; payload: GlyphBatchRequest }
  | { id: number; kind: "buildGlyphs"; payload: GlyphBatchRequest }
  | { id: number; kind: "createAtlas"; payload: { pageWidth?: number; pageHeight?: number; softPages?: number; hardPages?: number } }
  | { id: number; kind: "resolveAtlas"; payload: { atlasId: string; keys: string[]; cached: AtlasGlyphRecord[]; generate: AtlasGenerateRequest[] } }
  | { id: number; kind: "atlasPages"; payload: { atlasId: string; revisions: Array<{ page: number; revision: number }> } }
  | { id: number; kind: "releaseAtlas"; payload: { atlasId: string; lease: number } }
  | { id: number; kind: "destroyAtlas"; payload: { atlasId: string } }
  | { id: number; kind: "createAuthoringBlank"; payload: Record<string, never> }
  | { id: number; kind: "importAuthoringProfile"; payload: { profile: unknown } }
  | { id: number; kind: "restoreAuthoringCheckpoint"; payload: { checkpoint: AuthoringCheckpoint } }
  | { id: number; kind: "applyAuthoring"; payload: { authoringId: string; command: AuthoringCommand } }
  | { id: number; kind: "selectAuthoring"; payload: { authoringId: string; id: number | null } }
  | { id: number; kind: "elementsAuthoring"; payload: { authoringId: string } }
  | { id: number; kind: "beginAuthoringGesture"; payload: { authoringId: string; id: number } }
  | { id: number; kind: "previewAuthoringGesture"; payload: { authoringId: string; command: AuthoringCommand } }
  | { id: number; kind: "commitAuthoringGesture"; payload: { authoringId: string } }
  | { id: number; kind: "cancelAuthoringGesture"; payload: { authoringId: string } }
  | { id: number; kind: "appendAuthoringPage"; payload: { authoringId: string } }
  | { id: number; kind: "duplicateAuthoringPage"; payload: { authoringId: string; page: number } }
  | { id: number; kind: "deleteAuthoringPage"; payload: { authoringId: string; page: number } }
  | { id: number; kind: "moveAuthoringPage"; payload: { authoringId: string; fromPage: number; page: number } }
  | { id: number; kind: "undoAuthoring"; payload: { authoringId: string } }
  | { id: number; kind: "redoAuthoring"; payload: { authoringId: string } }
  | { id: number; kind: "exportAuthoring"; payload: { authoringId: string } }
  | { id: number; kind: "checkpointAuthoring"; payload: { authoringId: string } }
  | { id: number; kind: "destroyAuthoring"; payload: { authoringId: string } }
  | { id: number; kind: "layoutText"; payload: { request: unknown } }
  | { id: number; kind: "glyphDemand"; payload: { request: unknown } }
  | { id: number; kind: "createMasterData"; payload: { region: string; revision: string } }
  | { id: number; kind: "putMasterDataTable"; payload: { masterDataId: string; name: string; table: unknown } }
  | { id: number; kind: "sealMasterData"; payload: { masterDataId: string } }
  | { id: number; kind: "prepareProfile"; payload: { masterDataId: string; request: unknown } }
  | { id: number; kind: "createProfileScene"; payload: { masterDataId: string; request: unknown; layoutRequest: unknown } }
  | { id: number; kind: "destroyMasterData"; payload: { masterDataId: string } }
  | { id: number; kind: "createScene"; payload: { request: unknown } }
  | { id: number; kind: "advance"; payload: { sceneId: string; tick: number } }
  | { id: number; kind: "setLayerMask"; payload: { sceneId: string; layerId: StableId; visible: boolean } }
  | { id: number; kind: "setLayerMasks"; payload: { sceneId: string; expectedLayerTableRevision: number; overrides: LayerMaskOverride[] } }
  | { id: number; kind: "setTab"; payload: { sceneId: string; controlId: StableId; value: string } }
  | { id: number; kind: "scroll"; payload: { sceneId: string; controlId: StableId; offset?: number; delta?: number } }
  | { id: number; kind: "dumpScene"; payload: { sceneId: string } }
  | { id: number; kind: "destroyScene"; payload: { sceneId: string } }
  | { id: number; kind: "stats"; payload: Record<string, never> };

export type RendererWorkerResult =
  | { kind: "init"; protocol: typeof RENDERER_WORKER_PROTOCOL }
  | { kind: "contract"; contract: RendererFontContract }
  | { kind: "registerFont"; registered: boolean }
  | { kind: "mapGlyphs"; batch: FreeTypeGlyphMapBatch }
  | { kind: "planGlyphs"; plan: GlyphRasterPlan }
  | { kind: "buildGlyphs"; batch: FreeTypeGlyphBatch }
  | { kind: "createAtlas"; atlasId: string; stats: AtlasStats }
  | { kind: "resolveAtlas"; result: AtlasResolveResult }
  | { kind: "atlasPages"; updates: AtlasPageUpdate[] }
  | { kind: "releaseAtlas"; released: boolean }
  | { kind: "destroyAtlas"; destroyed: boolean }
  | { kind: "createAuthoring"; authoringId: string; document: GameProfileDocument; revision: number }
  | { kind: "authoringDelta"; delta: AuthoringDelta | null }
  | { kind: "elementsAuthoring"; elements: AuthoringSelection[] }
  | { kind: "exportAuthoring"; document: GameProfileDocument }
  | { kind: "checkpointAuthoring"; checkpoint: AuthoringCheckpoint }
  | { kind: "destroyAuthoring"; destroyed: boolean }
  | { kind: "layoutText"; batch: WasmLayoutBatch }
  | { kind: "glyphDemand"; batch: WasmGlyphDemandBatch }
  | { kind: "createMasterData"; masterDataId: string; report: Record<string, unknown> }
  | { kind: "masterDataReport"; report: Record<string, unknown> }
  | { kind: "prepareProfile"; preparation: Record<string, unknown> }
  | { kind: "destroyMasterData"; destroyed: boolean }
  | { kind: "createProfileScene"; sceneId: string; response: CoreSceneCreateResponse; layout: WasmLayoutBatch }
  | { kind: "createScene"; sceneId: string; response: CoreSceneCreateResponse }
  | { kind: "delta"; delta: CoreSceneDelta }
  | { kind: "dumpScene"; dump: CoreSceneDump }
  | { kind: "destroyScene"; destroyed: boolean }
  | { kind: "stats"; stats: RendererWorkerStats };

export type RendererWorkerResponse =
  | { id: number; ok: true; result: RendererWorkerResult }
  | { id: number; ok: false; error: { code: string; message: string } };
