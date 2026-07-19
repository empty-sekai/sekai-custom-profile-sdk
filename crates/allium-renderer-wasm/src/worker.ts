/// <reference lib="webworker" />

import type { EmscriptenModule, EmscriptenModuleFactory } from "./emscripten.js";
import {
  RENDERER_WORKER_PROTOCOL,
  type GlyphBatchRequest,
  type RendererFontContract,
  type RendererWorkerRequest,
  type RendererWorkerResponse,
  type RendererWorkerResult,
  type RendererWorkerStats,
} from "./protocol.js";
import type { CoreSceneCreateResponse } from "./types/core.js";
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
import type { AuthoringCheckpoint, AuthoringDelta, AuthoringSelection, GameProfileDocument } from "./types/authoring.js";

const scope = globalThis as unknown as DedicatedWorkerGlobalScope;
const ATLAS_TRANSIENT_HARD_BYTES = 48 * 1024 * 1024;
const scenes = new Map<string, number>();
const masterDataSessions = new Map<string, number>();
const atlasSessions = new Map<string, number>();
const authoringSessions = new Map<string, number>();
const fonts = new Map<string, ArrayBuffer>();
const fontSources = new Map<string, { region: string; family: string; sourceHash: string }>();
let modulePromise: Promise<EmscriptenModule> | null = null;
let moduleInstance: EmscriptenModule | null = null;
const counters: RendererWorkerStats = {
  protocol: RENDERER_WORKER_PROTOCOL,
  initialized: false,
  scenes: 0,
  masterDataSessions: 0,
  atlasSessions: 0,
  authoringSessions: 0,
  fonts: 0,
  requests: 0,
  failures: 0,
  wasmMs: 0,
  bridgeBytes: 0,
};

scope.onmessage = (event: MessageEvent<RendererWorkerRequest>) => {
  void dispatch(event.data).catch((error: unknown) => {
    counters.failures += 1;
    post({
      id: event.data.id,
      ok: false,
      error: {
        code: error instanceof WorkerError ? error.code : "RENDERER_WORKER_FAILED",
        message: error instanceof Error ? error.message : String(error),
      },
    });
  });
};

async function dispatch(request: RendererWorkerRequest): Promise<void> {
  counters.requests += 1;
  const started = performance.now();
  let result: RendererWorkerResult;
  let transfers: Transferable[] = [];
  switch (request.kind) {
    case "init":
      await initialize(request.payload.moduleUrl, request.payload.wasmUrl);
      result = { kind: "init", protocol: RENDERER_WORKER_PROTOCOL };
      break;
    case "contract": {
      const contract = callJson<RendererFontContract>(await loadedModule(), "sdf_layout_freetype_contract_json", [], []);
      result = { kind: "contract", contract };
      break;
    }
    case "registerFont": {
      assertHash(request.payload.sourceHash);
      fonts.set(fontKey(request.payload), request.payload.bytes);
      fontSources.set(request.payload.family, {
        region: request.payload.region,
        family: request.payload.family,
        sourceHash: request.payload.sourceHash,
      });
      counters.fonts = fonts.size;
      counters.bridgeBytes += request.payload.bytes.byteLength;
      result = { kind: "registerFont", registered: true };
      break;
    }
    case "mapGlyphs": {
      const batch = await buildGlyphBatch(request.payload, true) as FreeTypeGlyphMapBatch;
      result = { kind: "mapGlyphs", batch };
      break;
    }
    case "planGlyphs": {
      result = { kind: "planGlyphs", plan: await planGlyphBatch(request.payload) };
      break;
    }
    case "buildGlyphs": {
      const batch = await buildGlyphBatch(request.payload, false) as FreeTypeGlyphBatch;
      transfers = materializeGlyphPixels(batch);
      result = { kind: "buildGlyphs", batch };
      break;
    }
    case "createAtlas": {
      const response = callJsonWithInput<{ handle: number; stats: AtlasStats }>(
        await loadedModule(),
        "sdf_atlas_create_json",
        request.payload,
      );
      if (!Number.isInteger(response.handle) || response.handle <= 0) throw new WorkerError("ATLAS_CREATE_FAILED", "WASM returned an invalid atlas handle");
      const atlasId = `atlas:${response.handle}`;
      atlasSessions.set(atlasId, response.handle);
      counters.atlasSessions = atlasSessions.size;
      result = { kind: "createAtlas", atlasId, stats: response.stats };
      break;
    }
    case "resolveAtlas": {
      const mod = await loadedModule();
      const handle = atlasHandle(request.payload.atlasId);
      const warm = resolveAtlasRecords(mod, handle, request.payload.keys, request.payload.cached);
      const missing = new Set(warm.missingKeys);
      const generated: AtlasGlyphRecord[] = [];
      for (const group of request.payload.generate) {
        const glyphs = group.glyphs.filter((entry) => missing.has(entry.key));
        if (glyphs.length > 0) generated.push(...await generateAtlasRecords({ ...group, glyphs, chars: glyphs.map((entry) => entry.char) }));
      }
      const cold = generated.length > 0 ? resolveAtlasRecords(mod, handle, [...missing], generated) : null;
      const missingKeys = cold?.missingKeys ?? warm.missingKeys;
      if (missingKeys.length > 0) throw new WorkerError("ATLAS_GLYPH_MISSING", `Atlas could not resolve ${missingKeys.length} glyph resource(s)`);
      transfers = generated.map((record) => record.pixels.buffer);
      counters.bridgeBytes += generated.reduce((sum, record) => sum + record.pixels.byteLength, 0);
      result = { kind: "resolveAtlas", result: {
        leases: [warm.lease, cold?.lease].filter((lease): lease is number => lease != null),
        placements: [...warm.placements, ...(cold?.placements ?? [])],
        missingKeys,
        generated,
        stats: cold?.stats ?? warm.stats,
      } };
      break;
    }
    case "atlasPages": {
      const mod = await loadedModule();
      const handle = atlasHandle(request.payload.atlasId);
      const metadata = callJsonWithInput<Array<Omit<AtlasPageUpdate, "pixels" | "dirtyRects"> & { dirtyRects: Array<{ x: number; y: number; width: number; height: number }> }>>(
        mod,
        "sdf_atlas_pages_since_json",
        { revisions: request.payload.revisions },
        [handle],
      );
      const updates = metadata.map((update) => materializeAtlasPage(mod, handle, update));
      transfers = updates.flatMap((update) => update.fullUpload
        ? [update.pixels.buffer]
        : update.dirtyRects.map((rect) => rect.pixels.buffer));
      counters.bridgeBytes += updates.reduce((sum, update) => sum + (update.fullUpload
        ? update.pixels.byteLength
        : update.dirtyRects.reduce((rectSum, rect) => rectSum + rect.pixels.byteLength, 0)), 0);
      result = { kind: "atlasPages", updates };
      break;
    }
    case "releaseAtlas": {
      const status = (await loadedModule()).ccall(
        "sdf_atlas_release",
        "number",
        ["number", "number"],
        [atlasHandle(request.payload.atlasId), request.payload.lease],
      );
      result = { kind: "releaseAtlas", released: status === 1 };
      break;
    }
    case "destroyAtlas": {
      const handle = atlasHandle(request.payload.atlasId);
      const status = (await loadedModule()).ccall("sdf_atlas_destroy", "number", ["number"], [handle]);
      atlasSessions.delete(request.payload.atlasId);
      counters.atlasSessions = atlasSessions.size;
      result = { kind: "destroyAtlas", destroyed: status === 1 };
      break;
    }
    case "createAuthoringBlank": {
      const response = callJson<{ handle: number; revision: number; document: GameProfileDocument }>(
        await loadedModule(),
        "sdf_renderer_authoring_create_blank_json",
        [],
        [],
      );
      const authoringId = registerAuthoring(response.handle);
      result = { kind: "createAuthoring", authoringId, revision: response.revision, document: response.document };
      break;
    }
    case "importAuthoringProfile": {
      const response = callJsonWithInput<{ handle: number; revision: number; document: GameProfileDocument }>(
        await loadedModule(),
        "sdf_renderer_authoring_import_profile_json",
        request.payload.profile,
      );
      const authoringId = registerAuthoring(response.handle);
      result = { kind: "createAuthoring", authoringId, revision: response.revision, document: response.document };
      break;
    }
    case "restoreAuthoringCheckpoint": {
      const response = callJsonWithInput<{ handle: number; revision: number; document: GameProfileDocument }>(
        await loadedModule(),
        "sdf_renderer_authoring_restore_checkpoint_json",
        request.payload.checkpoint,
      );
      const authoringId = registerAuthoring(response.handle);
      result = { kind: "createAuthoring", authoringId, revision: response.revision, document: response.document };
      break;
    }
    case "applyAuthoring":
      result = { kind: "authoringDelta", delta: callJsonWithInput<AuthoringDelta>(
        await loadedModule(),
        "sdf_renderer_authoring_apply_json",
        request.payload.command,
        [authoringHandle(request.payload.authoringId)],
      ) };
      break;
    case "selectAuthoring":
      result = { kind: "authoringDelta", delta: callJsonWithInput<AuthoringDelta>(
        await loadedModule(),
        "sdf_renderer_authoring_select_json",
        { id: request.payload.id },
        [authoringHandle(request.payload.authoringId)],
      ) };
      break;
    case "elementsAuthoring":
      result = { kind: "elementsAuthoring", elements: callJson<AuthoringSelection[]>(
        await loadedModule(),
        "sdf_renderer_authoring_elements_json",
        ["number"],
        [authoringHandle(request.payload.authoringId)],
      ) };
      break;
    case "beginAuthoringGesture":
      result = { kind: "authoringDelta", delta: callJsonWithInput<AuthoringDelta>(
        await loadedModule(),
        "sdf_renderer_authoring_begin_gesture_json",
        { id: request.payload.id },
        [authoringHandle(request.payload.authoringId)],
      ) };
      break;
    case "previewAuthoringGesture":
      result = { kind: "authoringDelta", delta: callJsonWithInput<AuthoringDelta>(
        await loadedModule(),
        "sdf_renderer_authoring_preview_gesture_json",
        request.payload.command,
        [authoringHandle(request.payload.authoringId)],
      ) };
      break;
    case "commitAuthoringGesture":
      result = { kind: "authoringDelta", delta: callJson<AuthoringDelta>(
        await loadedModule(),
        "sdf_renderer_authoring_commit_gesture_json",
        ["number"],
        [authoringHandle(request.payload.authoringId)],
      ) };
      break;
    case "cancelAuthoringGesture":
      result = { kind: "authoringDelta", delta: callJson<AuthoringDelta>(
        await loadedModule(),
        "sdf_renderer_authoring_cancel_gesture_json",
        ["number"],
        [authoringHandle(request.payload.authoringId)],
      ) };
      break;
    case "appendAuthoringPage":
      result = { kind: "authoringDelta", delta: callJson<AuthoringDelta>(
        await loadedModule(),
        "sdf_renderer_authoring_append_page_json",
        ["number"],
        [authoringHandle(request.payload.authoringId)],
      ) };
      break;
    case "duplicateAuthoringPage":
      result = { kind: "authoringDelta", delta: callJsonWithInput<AuthoringDelta>(
        await loadedModule(),
        "sdf_renderer_authoring_duplicate_page_json",
        { page: request.payload.page },
        [authoringHandle(request.payload.authoringId)],
      ) };
      break;
    case "deleteAuthoringPage":
      result = { kind: "authoringDelta", delta: callJsonWithInput<AuthoringDelta>(
        await loadedModule(),
        "sdf_renderer_authoring_delete_page_json",
        { page: request.payload.page },
        [authoringHandle(request.payload.authoringId)],
      ) };
      break;
    case "moveAuthoringPage":
      result = { kind: "authoringDelta", delta: callJsonWithInput<AuthoringDelta>(
        await loadedModule(),
        "sdf_renderer_authoring_move_page_json",
        { fromPage: request.payload.fromPage, page: request.payload.page },
        [authoringHandle(request.payload.authoringId)],
      ) };
      break;
    case "undoAuthoring":
      result = { kind: "authoringDelta", delta: callJson<AuthoringDelta | null>(
        await loadedModule(),
        "sdf_renderer_authoring_undo_json",
        ["number"],
        [authoringHandle(request.payload.authoringId)],
      ) };
      break;
    case "redoAuthoring":
      result = { kind: "authoringDelta", delta: callJson<AuthoringDelta | null>(
        await loadedModule(),
        "sdf_renderer_authoring_redo_json",
        ["number"],
        [authoringHandle(request.payload.authoringId)],
      ) };
      break;
    case "exportAuthoring":
      result = { kind: "exportAuthoring", document: callJson<GameProfileDocument>(
        await loadedModule(),
        "sdf_renderer_authoring_export_json",
        ["number"],
        [authoringHandle(request.payload.authoringId)],
      ) };
      break;
    case "checkpointAuthoring":
      result = { kind: "checkpointAuthoring", checkpoint: callJson<AuthoringCheckpoint>(
        await loadedModule(),
        "sdf_renderer_authoring_checkpoint_json",
        ["number"],
        [authoringHandle(request.payload.authoringId)],
      ) };
      break;
    case "destroyAuthoring": {
      const handle = authoringHandle(request.payload.authoringId);
      const status = (await loadedModule()).ccall("sdf_renderer_authoring_destroy", "number", ["number"], [handle]);
      authoringSessions.delete(request.payload.authoringId);
      counters.authoringSessions = authoringSessions.size;
      result = { kind: "destroyAuthoring", destroyed: status === 1 };
      break;
    }
    case "layoutText": {
      const batch = callJsonWithInput<WasmLayoutBatch>(
        await loadedModule(),
        "sdf_layout_freetype_build_layout_json",
        request.payload.request,
      );
      result = { kind: "layoutText", batch };
      break;
    }
    case "glyphDemand": {
      const batch = callJsonWithInput<WasmGlyphDemandBatch>(
        await loadedModule(),
        "sdf_layout_freetype_glyph_demand_json",
        request.payload.request,
      );
      result = { kind: "glyphDemand", batch };
      break;
    }
    case "createMasterData": {
      const report = callJsonWithInput<Record<string, unknown>>(
        await loadedModule(),
        "sdf_renderer_core_masterdata_create_json",
        request.payload,
      );
      const handle = Number(report.handle);
      if (!Number.isInteger(handle) || handle <= 0) throw new WorkerError("MASTERDATA_CREATE_FAILED", "Renderer returned an invalid master-data handle");
      const masterDataId = `${request.payload.region}:${request.payload.revision}:${handle}`;
      masterDataSessions.set(masterDataId, handle);
      counters.masterDataSessions = masterDataSessions.size;
      result = { kind: "createMasterData", masterDataId, report };
      break;
    }
    case "putMasterDataTable":
      result = { kind: "masterDataReport", report: callJsonWithInput(
        await loadedModule(),
        "sdf_renderer_core_masterdata_put_table_json",
        { name: request.payload.name, table: request.payload.table },
        [masterDataHandle(request.payload.masterDataId)],
      ) };
      break;
    case "sealMasterData":
      result = { kind: "masterDataReport", report: callJson(
        await loadedModule(),
        "sdf_renderer_core_masterdata_seal_json",
        ["number"],
        [masterDataHandle(request.payload.masterDataId)],
      ) };
      break;
    case "prepareProfile": {
      const mod = await loadedModule();
      const preparation = callJsonWithInput<Record<string, unknown>>(
        mod,
        "sdf_renderer_core_profile_prepare_json",
        request.payload.request,
        [masterDataHandle(request.payload.masterDataId)],
      );
      const profileRequest = preparationRecord(request.payload.request, "request");
      if (profileRequest.fontDemandOnly === true) {
        const families = preparation.font_families;
        if (!Array.isArray(families) || families.some((family) => typeof family !== "string" || family.length === 0)) {
          throw new WorkerError("INVALID_PROFILE_PREPARATION", "Profile preparation did not return valid font families");
        }
        result = {
          kind: "prepareProfile",
          preparation: { fontDemands: [...families] },
        };
        break;
      }
      const glyphLayers = completeGlyphLayers(preparation);
      const glyphDemand = callJsonWithInput<WasmGlyphDemandBatch>(
        mod,
        "sdf_layout_freetype_glyph_demand_json",
        { layers: glyphLayers },
      );
      result = {
        kind: "prepareProfile",
        preparation: {
          ...preparation,
          glyph_demand: glyphDemand,
          layout_request: { layers: completeLayoutLayers(preparation) },
        },
      };
      break;
    }
    case "createProfileScene": {
      const layout = callJsonWithInput<WasmLayoutBatch>(
        await loadedModule(),
        "sdf_layout_freetype_build_layout_json",
        request.payload.layoutRequest,
      );
      const response = callJsonWithInput<CoreSceneCreateResponse>(
        await loadedModule(),
        "sdf_renderer_core_profile_create_json",
        profileCompileRequest(request.payload.request, layout.dynamicPrograms),
        [masterDataHandle(request.payload.masterDataId)],
      );
      assertSchema(response.snapshot.schema_major);
      const sceneId = `${response.snapshot.scene_id}:${response.handle}`;
      scenes.set(sceneId, response.handle);
      counters.scenes = scenes.size;
      result = { kind: "createProfileScene", sceneId, response, layout };
      break;
    }
    case "destroyMasterData": {
      const mod = await loadedModule();
      const handle = masterDataHandle(request.payload.masterDataId);
      const status = mod.ccall("sdf_renderer_core_masterdata_destroy", "number", ["number"], [handle]);
      masterDataSessions.delete(request.payload.masterDataId);
      counters.masterDataSessions = masterDataSessions.size;
      result = { kind: "destroyMasterData", destroyed: status === 1 };
      break;
    }
    case "createScene": {
      const mod = await loadedModule();
      const response = callJsonWithInput<CoreSceneCreateResponse>(
        mod,
        "sdf_renderer_core_profile_scene_create_json",
        request.payload.request,
      );
      assertSchema(response.snapshot.schema_major);
      const sceneId = `${response.snapshot.scene_id}:${response.handle}`;
      scenes.set(sceneId, response.handle);
      counters.scenes = scenes.size;
      result = { kind: "createScene", sceneId, response };
      break;
    }
    case "advance":
      result = { kind: "delta", delta: callSceneNoInput("sdf_renderer_core_scene_advance_json", request.payload.sceneId, [tick(request.payload.tick)]) };
      break;
    case "setLayerMask":
      result = { kind: "delta", delta: callSceneWithInput("sdf_renderer_core_scene_set_mask_json", request.payload.sceneId, {
        layer_id: request.payload.layerId,
        visible: request.payload.visible,
      }) };
      break;
    case "setLayerMasks":
      result = { kind: "delta", delta: callSceneWithInput("sdf_renderer_core_scene_set_masks_json", request.payload.sceneId, {
        expected_layer_table_revision: request.payload.expectedLayerTableRevision,
        overrides: request.payload.overrides.map((entry) => ({
          layer_id: entry.layerId,
          mask_override: entry.visible == null ? "inherit_authored" : entry.visible ? "force_visible" : "force_hidden",
        })),
      }) };
      break;
    case "setTab":
      result = { kind: "delta", delta: callSceneWithInput("sdf_renderer_core_scene_set_tab_json", request.payload.sceneId, {
        control_id: request.payload.controlId,
        value: request.payload.value,
      }) };
      break;
    case "scroll":
      result = { kind: "delta", delta: callSceneWithInput("sdf_renderer_core_scene_scroll_json", request.payload.sceneId, {
        control_id: request.payload.controlId,
        offset: request.payload.offset,
        delta: request.payload.delta,
      }) };
      break;
    case "dumpScene":
      result = { kind: "dumpScene", dump: callSceneNoInput("sdf_renderer_core_scene_dump_json", request.payload.sceneId) };
      break;
    case "destroyScene": {
      const mod = await loadedModule();
      const handle = sceneHandle(request.payload.sceneId);
      const status = mod.ccall("sdf_renderer_core_scene_destroy", "number", ["number"], [handle]);
      scenes.delete(request.payload.sceneId);
      counters.scenes = scenes.size;
      result = { kind: "destroyScene", destroyed: status === 1 };
      break;
    }
    case "stats":
      result = { kind: "stats", stats: { ...counters } };
      break;
    default:
      throw new WorkerError("RENDERER_WORKER_PROTOCOL", "Unsupported renderer worker request");
  }
  counters.wasmMs += performance.now() - started;
  post({ id: request.id, ok: true, result }, transfers);
}

function profileCompileRequest(request: unknown, dynamicPrograms: unknown[]): Record<string, unknown> {
  if (!request || typeof request !== "object" || Array.isArray(request)) {
    throw new WorkerError("INVALID_PROFILE_REQUEST", "Profile compile request must be an object");
  }
  const source = request as Record<string, unknown>;
  if ("lineIndent" in source || "dynamicPrograms" in source) {
    throw new WorkerError("INVALID_PROFILE_REQUEST", "Dynamic programs are owned by the renderer worker");
  }
  return { ...source, dynamicPrograms };
}

function completeGlyphLayers(preparation: Record<string, unknown>): Array<{
  text: string;
  region: string;
  fontFamily: string;
  fontSourceHash: string;
}> {
  if (!Array.isArray(preparation.glyph_layers)) {
    throw new WorkerError("INVALID_PROFILE_PREPARATION", "Profile preparation did not return complete glyph inputs");
  }
  return preparation.glyph_layers.map((entry) => {
    const value = preparationRecord(entry, "glyph input");
    const family = preparationString(value.font_family, "font_family");
    const source = registeredFontSource(family);
    return {
      text: preparationString(value.text, "text"),
      region: source.region,
      fontFamily: source.family,
      fontSourceHash: source.sourceHash,
    };
  });
}

function completeLayoutLayers(preparation: Record<string, unknown>): Array<Record<string, unknown>> {
  if (!Array.isArray(preparation.layout_layers)) {
    throw new WorkerError("INVALID_PROFILE_PREPARATION", "Profile preparation did not return complete layout inputs");
  }
  return preparation.layout_layers.map((entry) => {
    const value = preparationRecord(entry, "layout input");
    const family = preparationString(value.fontFamily, "fontFamily");
    const source = registeredFontSource(family);
    return {
      ...value,
      region: source.region,
      fontSourceHash: source.sourceHash,
    };
  });
}

function registeredFontSource(family: string): { region: string; family: string; sourceHash: string } {
  const source = fontSources.get(family);
  if (!source) throw new WorkerError("FONT_NOT_REGISTERED", `Required font is not registered: ${family}`);
  return source;
}

function preparationRecord(value: unknown, field: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new WorkerError("INVALID_PROFILE_PREPARATION", `Profile ${field} is invalid`);
  }
  return value as Record<string, unknown>;
}

function preparationString(value: unknown, field: string): string {
  if (typeof value !== "string") throw new WorkerError("INVALID_PROFILE_PREPARATION", `Profile ${field} is invalid`);
  return value;
}

async function initialize(moduleUrl: string, wasmUrl?: string): Promise<EmscriptenModule> {
  modulePromise ??= import(moduleUrl).then(async (namespace: Record<string, unknown>) => {
    const factory = (namespace.default ?? namespace.Module) as EmscriptenModuleFactory | undefined;
    if (typeof factory !== "function") throw new WorkerError("WASM_FACTORY_MISSING", "The renderer module does not export an Emscripten factory");
    const mod = await factory({
      locateFile: (path) => path.endsWith(".wasm") && wasmUrl ? wasmUrl : new URL(path, moduleUrl).toString(),
      printErr: (...values) => console.warn("[allium-renderer]", ...values),
    });
    moduleInstance = mod;
    counters.initialized = true;
    return mod;
  });
  return modulePromise;
}

async function loadedModule(): Promise<EmscriptenModule> {
  if (!modulePromise) throw new WorkerError("WORKER_NOT_INITIALIZED", "Initialize the renderer worker before using it");
  return modulePromise;
}

async function buildGlyphBatch(request: GlyphBatchRequest, mapOnly: boolean): Promise<FreeTypeGlyphBatch | FreeTypeGlyphMapBatch> {
  const mod = await loadedModule();
  const bytes = fonts.get(fontKey(request));
  if (!bytes) throw new WorkerError("FONT_NOT_REGISTERED", `Font is not registered: ${request.region}/${request.family}`);
  const font = new Uint8Array(bytes);
  const codepoints = request.chars.map((value) => value.codePointAt(0)).filter((value): value is number => value != null);
  const fontPointer = mod._malloc(font.byteLength);
  const codePointer = mod._malloc(codepoints.length * 4);
  let resultPointer = 0;
  try {
    mod.HEAPU8.set(font, fontPointer);
    mod.HEAPU32.set(codepoints, codePointer >>> 2);
    const edt = !mapOnly && (request.backend ?? "edt") === "edt";
    const name = mapOnly
      ? "sdf_layout_freetype_map_glyphs_json"
      : edt ? "sdf_layout_freetype_build_glyph_json_edt" : "sdf_layout_freetype_build_glyph_json";
    const argumentTypes: Array<"number" | "string"> = ["number", "number", "number", "number", "string", "string", "string"];
    const arguments_: Array<number | string> = [
      fontPointer, font.byteLength, codePointer, codepoints.length,
      request.region, request.family, request.sourceHash,
    ];
    if (edt) {
      argumentTypes.push("number");
      arguments_.push(Math.max(1, Math.floor(request.supersample ?? 2)));
    }
    resultPointer = mod.ccall(name, "number", argumentTypes, arguments_) as number;
    const result = JSON.parse(readCString(mod, resultPointer)) as (FreeTypeGlyphBatch | FreeTypeGlyphMapBatch) & { error?: string };
    if (result.error) throw new WorkerError("GLYPH_BUILD_FAILED", result.error);
    return result;
  } finally {
    if (resultPointer) mod.ccall("sdf_layout_freetype_free_string", null, ["number"], [resultPointer]);
    mod._free(codePointer);
    mod._free(fontPointer);
  }
}

async function planGlyphBatch(request: GlyphBatchRequest): Promise<GlyphRasterPlan> {
  const mod = await loadedModule();
  const bytes = fonts.get(fontKey(request));
  if (!bytes) throw new WorkerError("FONT_NOT_REGISTERED", `Font is not registered: ${request.region}/${request.family}`);
  const font = new Uint8Array(bytes);
  const codepoints = request.chars.map((value) => value.codePointAt(0)).filter((value): value is number => value != null);
  const fontPointer = mod._malloc(font.byteLength);
  const codePointer = mod._malloc(codepoints.length * 4);
  let resultPointer = 0;
  try {
    mod.HEAPU8.set(font, fontPointer);
    mod.HEAPU32.set(codepoints, codePointer >>> 2);
    resultPointer = mod.ccall(
      "sdf_layout_freetype_plan_glyphs_json",
      "number",
      ["number", "number", "number", "number", "string", "string", "string", "string", "number"],
      [
        fontPointer,
        font.byteLength,
        codePointer,
        codepoints.length,
        request.region,
        request.family,
        request.sourceHash,
        request.backend ?? "edt",
        Math.max(0, Math.floor(request.supersample ?? 2)),
      ],
    ) as number;
    const plan = JSON.parse(readCString(mod, resultPointer)) as GlyphRasterPlan & { error?: string };
    if (plan.error) throw new WorkerError("GLYPH_PLAN_FAILED", plan.error);
    return plan;
  } finally {
    if (resultPointer) mod.ccall("sdf_layout_freetype_free_string", null, ["number"], [resultPointer]);
    mod._free(codePointer);
    mod._free(fontPointer);
  }
}

function materializeGlyphPixels(batch: FreeTypeGlyphBatch): Transferable[] {
  const transfers: Transferable[] = [];
  for (const glyph of batch.glyphs) {
    if (!glyph.pixels_base64) continue;
    const raw = atob(glyph.pixels_base64);
    const bytes = new Uint8Array(raw.length);
    for (let index = 0; index < raw.length; index += 1) bytes[index] = raw.charCodeAt(index);
    glyph.pixels = bytes;
    glyph.pixels_base64 = undefined;
    counters.bridgeBytes += bytes.byteLength;
    transfers.push(bytes.buffer);
  }
  return transfers;
}

type WasmAtlasResolve = {
  lease: number | null;
  placements: AtlasResolveResult["placements"];
  missingKeys: string[];
  stats: AtlasStats;
};

function resolveAtlasRecords(mod: EmscriptenModule, handle: number, keys: string[], records: AtlasGlyphRecord[]): WasmAtlasResolve {
  const bytes = records.reduce((sum, record) => sum + record.pixels.byteLength, 0);
  if (bytes > ATLAS_TRANSIENT_HARD_BYTES) throw new WorkerError(
    "MEMORY_BUDGET_EXCEEDED",
    `Atlas raster bridge requires ${bytes} bytes, above the ${ATLAS_TRANSIENT_HARD_BYTES}-byte transient budget`,
  );
  return callJsonWithInput<WasmAtlasResolve>(mod, "sdf_atlas_resolve_json", {
    keys,
    records: records.map((record) => ({
      key: record.key,
      width: record.width,
      height: record.height,
      pixelsBase64: bytesToBase64(record.pixels),
    })),
  }, [handle]);
}

async function generateAtlasRecords(request: AtlasGenerateRequest): Promise<AtlasGlyphRecord[]> {
  const batch = await buildGlyphBatch(request, false) as FreeTypeGlyphBatch;
  const byCharacter = new Map(batch.glyphs.map((glyph) => [glyph.ch, glyph] as const));
  return request.glyphs.map((entry) => {
    const glyph = byCharacter.get(entry.char);
    if (!glyph) throw new WorkerError("GLYPH_BUILD_FAILED", `FreeType omitted an atlas glyph for U+${entry.char.codePointAt(0)?.toString(16).toUpperCase()}`);
    return {
      key: entry.key,
      glyphIndex: entry.glyphIndex,
      width: glyph.width,
      height: glyph.height,
      advance: glyph.advance,
      xOffset: glyph.x_offset,
      yOffset: glyph.y_offset,
      planeBearingX: glyph.plane_bearing_x,
      planeBearingY: glyph.plane_bearing_y,
      planeWidth: glyph.plane_width,
      planeHeight: glyph.plane_height,
      drawable: glyph.drawable,
      pixels: glyphPixels(glyph),
    };
  });
}

function glyphPixels(glyph: FreeTypeGlyphBatch["glyphs"][number]): Uint8Array {
  if (glyph.pixels instanceof Uint8Array) return glyph.pixels;
  if (Array.isArray(glyph.pixels)) return new Uint8Array(glyph.pixels);
  if (!glyph.pixels_base64) return new Uint8Array(glyph.width * glyph.height);
  return base64ToBytes(glyph.pixels_base64);
}

function materializeAtlasPage(
  mod: EmscriptenModule,
  handle: number,
  metadata: Omit<AtlasPageUpdate, "pixels" | "dirtyRects"> & { dirtyRects: Array<{ x: number; y: number; width: number; height: number }> },
): AtlasPageUpdate {
  const pointer = mod.ccall("sdf_atlas_page_pixels_ptr", "number", ["number", "number"], [handle, metadata.page]) as number;
  const length = mod.ccall("sdf_atlas_page_pixels_len", "number", ["number", "number"], [handle, metadata.page]) as number;
  if (!pointer || length <= 0) throw new WorkerError("ATLAS_PAGE_FAILED", `WASM returned an invalid atlas page ${metadata.page}`);
  const page = mod.HEAPU8.subarray(pointer, pointer + length);
  const pageWidth = metadata.pageWidth;
  if (!Number.isInteger(pageWidth) || pageWidth <= 0 || length % pageWidth !== 0) throw new WorkerError("ATLAS_PAGE_FAILED", `Atlas page ${metadata.page} has invalid R8 dimensions`);
  if (metadata.fullUpload) return { ...metadata, pixels: new Uint8Array(page), dirtyRects: [] };
  return {
    ...metadata,
    pixels: new Uint8Array(),
    dirtyRects: metadata.dirtyRects.map((rect) => ({ ...rect, pixels: copyAtlasRect(page, pageWidth, rect) })),
  };
}

function copyAtlasRect(page: Uint8Array, pageWidth: number, rect: { x: number; y: number; width: number; height: number }): Uint8Array {
  const pixels = new Uint8Array(rect.width * rect.height);
  for (let row = 0; row < rect.height; row += 1) {
    pixels.set(page.subarray((rect.y + row) * pageWidth + rect.x, (rect.y + row) * pageWidth + rect.x + rect.width), row * rect.width);
  }
  return pixels;
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += chunk) binary += String.fromCharCode(...bytes.subarray(offset, offset + chunk));
  return btoa(binary);
}

function base64ToBytes(value: string): Uint8Array {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

function callSceneWithInput<T>(name: string, sceneId: string, input: unknown): T {
  const mod = requireLoadedModule();
  return callJsonWithInput<T>(mod, name, input, [sceneHandle(sceneId)]);
}

function callSceneNoInput<T>(name: string, sceneId: string, suffix: number[] = []): T {
  const mod = requireLoadedModule();
  return callJson<T>(mod, name, ["number", ...suffix.map(() => "number" as const)], [sceneHandle(sceneId), ...suffix]);
}

function callJsonWithInput<T>(mod: EmscriptenModule, name: string, input: unknown, prefix: number[] = []): T {
  const bytes = new TextEncoder().encode(JSON.stringify(input));
  const pointer = mod._malloc(bytes.byteLength);
  try {
    mod.HEAPU8.set(bytes, pointer);
    return callJson<T>(mod, name, [...prefix.map(() => "number" as const), "number", "number"], [...prefix, pointer, bytes.byteLength]);
  } finally {
    mod._free(pointer);
  }
}

function callJson<T>(mod: EmscriptenModule, name: string, argumentTypes: Array<"number" | "string">, arguments_: Array<number | string>): T {
  const pointer = mod.ccall(name, "number", argumentTypes, arguments_) as number;
  try {
    const value = JSON.parse(readCString(mod, pointer)) as T & { error?: string };
    if (value.error) throw new WorkerError("WASM_CALL_FAILED", value.error);
    return value;
  } finally {
    mod.ccall("sdf_layout_freetype_free_string", null, ["number"], [pointer]);
  }
}

function readCString(mod: EmscriptenModule, pointer: number): string {
  let end = pointer;
  while (mod.HEAPU8[end] !== 0) end += 1;
  return new TextDecoder().decode(new Uint8Array(mod.HEAPU8.subarray(pointer, end)));
}

function requireLoadedModule(): EmscriptenModule {
  if (!modulePromise) throw new WorkerError("WORKER_NOT_INITIALIZED", "Initialize the renderer worker before using it");
  if (!moduleInstance) throw new WorkerError("WORKER_BUSY", "The renderer module is still initializing");
  return moduleInstance;
}

function sceneHandle(sceneId: string): number {
  const handle = scenes.get(sceneId);
  if (handle == null) throw new WorkerError("SCENE_NOT_FOUND", `Unknown scene: ${sceneId}`);
  return handle;
}

function masterDataHandle(masterDataId: string): number {
  const handle = masterDataSessions.get(masterDataId);
  if (handle == null) throw new WorkerError("MASTERDATA_NOT_FOUND", `Unknown master-data session: ${masterDataId}`);
  return handle;
}

function atlasHandle(atlasId: string): number {
  const handle = atlasSessions.get(atlasId);
  if (handle == null) throw new WorkerError("ATLAS_NOT_FOUND", `Unknown atlas session: ${atlasId}`);
  return handle;
}

function registerAuthoring(handle: number): string {
  if (!Number.isInteger(handle) || handle <= 0) {
    throw new WorkerError("AUTHORING_CREATE_FAILED", "WASM returned an invalid authoring handle");
  }
  const authoringId = `authoring:${handle}`;
  authoringSessions.set(authoringId, handle);
  counters.authoringSessions = authoringSessions.size;
  return authoringId;
}

function authoringHandle(authoringId: string): number {
  const handle = authoringSessions.get(authoringId);
  if (handle == null) throw new WorkerError("AUTHORING_NOT_FOUND", `Unknown authoring session: ${authoringId}`);
  return handle;
}

function fontKey(value: { region: string; family: string; sourceHash: string }): string {
  return `${value.region}\u0000${value.sourceHash}\u0000${value.family}`;
}

function assertHash(value: string): void {
  if (!/^[0-9a-f]{64}$/i.test(value)) throw new WorkerError("INVALID_FONT_HASH", "Font sourceHash must be a full SHA-256 digest");
}

function assertSchema(major: number): void {
  if (major !== 1) throw new WorkerError("SCHEMA_MAJOR_UNSUPPORTED", `Unsupported renderer schema major: ${major}`);
}

function tick(value: number): number {
  if (!Number.isFinite(value) || value < 0) throw new WorkerError("INVALID_TICK", "Tick must be a non-negative finite number");
  return Math.floor(value);
}

function post(message: RendererWorkerResponse, transfers: Transferable[] = []): void {
  scope.postMessage(message, transfers);
}

class WorkerError extends Error {
  constructor(readonly code: string, message: string) {
    super(message);
    this.name = "WorkerError";
  }
}
