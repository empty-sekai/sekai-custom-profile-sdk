import type { SdfAtlas } from "../fontSdfAtlas.js";
import type { WasmLayoutBatch } from "../types/layout.js";
import { compileSemanticDrawBatches, semanticTextBatchKey, type SemanticDrawBatch } from "./semanticCommandGeometry.js";
import type { SemanticCommandPlan, SemanticCommandStatePatch, SemanticLayerPatch } from "./semanticCommandPlanner.js";
import { compileSemanticTextGlyphBatches } from "./semanticTextGlyphBridge.js";
import { WebglSemanticCommandExecutor, type SemanticGpuMetrics } from "./webglSemanticCommandExecutor.js";
import { applyNumericRegionRuntimeState, buildNumericTextRegions, type NumericTextRegion } from "../interaction/numericTextRegions.js";
import type { BrowserImageSource } from "./browserSemanticResources.js";
import { placeGeneralTextInstances } from "./generalTextRenderPlacement.js";

export class SemanticWebglSceneRenderer {
  private executor: WebglSemanticCommandExecutor;
  private plan: SemanticCommandPlan | null = null;
  private numericRegions: NumericTextRegion[] = [];
  private contextLost = false;
  private retainedScene: {
    plan: SemanticCommandPlan;
    atlas: SdfAtlas | null;
    batches: SemanticDrawBatch[];
    imageSources: Map<string, BrowserImageSource>;
    textGlyphBatches: Map<string, Float32Array>;
  } | null = null;

  constructor(gl: WebGL2RenderingContext) {
    this.executor = new WebglSemanticCommandExecutor(gl);
  }

  async setScene(input: {
    plan: SemanticCommandPlan;
    atlas: SdfAtlas | null;
    layout: WasmLayoutBatch;
    imageSources: Map<string, BrowserImageSource>;
  }): Promise<{
    textCommands: number;
    glyphInstances: number;
    atlasUploadBytes: number;
    atlasUploadRects: number;
  }> {
    const operations = input.plan.operations();
    this.plan = input.plan;
    const batches = compileSemanticDrawBatches(operations);
    this.executor.setScene(input.plan, batches, input.imageSources);
    const atlasUpload = input.atlas
      ? await this.executor.setSdfAtlas(input.atlas)
      : { bytes: 0, rects: 0 };
    const textOperations = operations.filter((operation) => operation.command.payload.kind === "text");
    const placedInstances = placeGeneralTextInstances(input.layout.instances, textOperations);
    this.numericRegions = textOperations.flatMap((operation) => buildNumericTextRegions(
      operation.command as Parameters<typeof buildNumericTextRegions>[0],
      placedInstances,
      operation.visible && operation.commandVisible,
    ));
    const glyphsByCommand = compileSemanticTextGlyphBatches(placedInstances, textOperations);
    const textGlyphBatches = new Map<string, Float32Array>();
    for (const batch of batches.filter((batch) => batch.kind === "text")) {
      const vertices = concatenateFloat32(batch.commandIds.map((commandId) => glyphsByCommand.get(commandId) ?? new Float32Array()));
      const key = semanticTextBatchKey(batch.commandIds);
      textGlyphBatches.set(key, vertices);
      this.executor.setTextGlyphBatch(key, vertices);
    }
    this.retainedScene = {
      plan: input.plan,
      atlas: input.atlas,
      batches,
      imageSources: input.imageSources,
      textGlyphBatches,
    };
    this.contextLost = false;
    return {
      textCommands: textOperations.length,
      glyphInstances: input.layout.instances.length,
      atlasUploadBytes: atlasUpload.bytes,
      atlasUploadRects: atlasUpload.rects,
    };
  }

  applyLayerPatches(patches: SemanticLayerPatch[]) {
    this.assertContextReady();
    return this.executor.applyLayerPatches(patches);
  }

  applyCommandPatches(patches: SemanticCommandStatePatch[]) {
    this.assertContextReady();
    return this.executor.applyCommandPatches(patches);
  }

  applyCoreDelta(delta: { patches: SemanticLayerPatch[]; command_patches: SemanticCommandStatePatch[] }) {
    this.assertContextReady();
    const layer = this.executor.applyLayerPatches(delta.patches);
    const command = this.executor.applyCommandPatches(delta.command_patches);
    return { ...layer, ...command };
  }

  draw(): SemanticGpuMetrics {
    this.assertContextReady();
    return this.executor.draw();
  }

  notifyContextLost(): void {
    this.contextLost = true;
  }

  async restoreContext(gl: WebGL2RenderingContext): Promise<{
    atlasUploadBytes: number;
    atlasUploadRects: number;
    textureUploads: number;
    textureBytes: number;
  }> {
    const retained = this.retainedScene;
    if (!retained) throw new Error("semantic GPU scene is not initialized");
    const previous = this.executor;
    const restored = new WebglSemanticCommandExecutor(gl);
    try {
      restored.setScene(retained.plan, retained.batches, retained.imageSources);
      const atlasUpload = retained.atlas
        ? await restored.setSdfAtlas(retained.atlas)
        : { bytes: 0, rects: 0 };
      for (const [key, vertices] of retained.textGlyphBatches) restored.setTextGlyphBatch(key, vertices);
      this.executor = restored;
      this.contextLost = false;
      previous.destroy();
      return {
        atlasUploadBytes: atlasUpload.bytes,
        atlasUploadRects: atlasUpload.rects,
        textureUploads: retained.imageSources.size,
        textureBytes: imageSourceBytes(retained.imageSources.values()),
      };
    } catch (error) {
      restored.destroy();
      this.contextLost = true;
      throw error;
    }
  }

  interactionRegions(): NumericTextRegion[] {
    const operations = new Map((this.plan?.operations() ?? []).map((operation) => [operation.command.id, operation]));
    return this.numericRegions.map((region) => {
      const operation = operations.get(region.id.split(":numeric:", 1)[0]);
      if (!operation) return region;
      return {
        ...applyNumericRegionRuntimeState(region, {
          layerTransform: operation.transform,
          commandTransform: operation.commandTransform,
        }),
        render_mask: operation.visible && operation.commandVisible,
      };
    });
  }

  destroy(): void {
    this.executor.destroy();
    this.plan = null;
    this.numericRegions = [];
    this.retainedScene = null;
    this.contextLost = false;
  }

  private assertContextReady(): void {
    if (this.contextLost) throw new Error("WebGL context is lost");
  }
}

function concatenateFloat32(values: Float32Array[]): Float32Array {
  const output = new Float32Array(values.reduce((sum, value) => sum + value.length, 0));
  let offset = 0;
  for (const value of values) {
    output.set(value, offset);
    offset += value.length;
  }
  return output;
}

function imageSourceBytes(values: Iterable<BrowserImageSource>): number {
  let bytes = 0;
  for (const source of values) bytes += Math.max(0, source.width * source.height * 4);
  return bytes;
}
