import type { SdfAtlas } from "../fontSdfAtlas.js";
import type { WasmLayoutBatch } from "../types/layout.js";
import { compileSemanticDrawBatches, semanticTextBatchKey, type SemanticDrawBatch } from "./semanticCommandGeometry.js";
import type { SemanticCommandPlan, SemanticCommandStatePatch, SemanticLayerPatch } from "./semanticCommandPlanner.js";
import { compileSemanticTextGlyphBatches } from "./semanticTextGlyphBridge.js";
import { WebglSemanticCommandExecutor, type SemanticGpuMetrics } from "./webglSemanticCommandExecutor.js";
import { applyNumericRegionRuntimeState, buildNumericTextRegions, type NumericTextRegion } from "../interaction/numericTextRegions.js";
import type { BrowserImageSource } from "./browserSemanticResources.js";
import { placeGeneralTextInstances } from "./generalTextRenderPlacement.js";
import type { GlyphInstance } from "../types/glyph.js";
import type { SemanticDrawOperation } from "./semanticCommandPlanner.js";

export type AuthoredTextHitGeometry = {
  bounds: { x: number; y: number; width: number; height: number };
  quad: [[number, number], [number, number], [number, number], [number, number]];
};

export class SemanticWebglSceneRenderer {
  private executor: WebglSemanticCommandExecutor;
  private plan: SemanticCommandPlan | null = null;
  private numericRegions: NumericTextRegion[] = [];
  private textHitGeometry = new Map<string, AuthoredTextHitGeometry>();
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
    this.textHitGeometry = buildAuthoredTextHitGeometry(textOperations, placedInstances);
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

  setLayerPreviewTransform(layerId: string, matrix: [number, number, number, number, number, number] | null) {
    this.assertContextReady();
    return this.executor.setLayerPreviewTransform(layerId, matrix);
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

  authoredTextHitGeometry(): Map<string, AuthoredTextHitGeometry> {
    const operations = this.plan?.operations() ?? [];
    const runtimeByLayer = new Map(operations.map((operation) => [operation.layerId, operation]));
    return new Map([...this.textHitGeometry].map(([layerId, geometry]) => {
      const operation = runtimeByLayer.get(layerId);
      const dx = (operation?.transform.dx ?? 0) + (operation?.commandTransform.dx ?? 0);
      const dy = (operation?.transform.dy ?? 0) + (operation?.commandTransform.dy ?? 0);
      const quad = geometry.quad.map(([x, y]) => [x + dx, y + dy]) as AuthoredTextHitGeometry["quad"];
      return [layerId, {
        bounds: { ...geometry.bounds, x: geometry.bounds.x + dx, y: geometry.bounds.y + dy },
        quad,
      }];
    }));
  }

  destroy(): void {
    this.executor.destroy();
    this.plan = null;
    this.numericRegions = [];
    this.textHitGeometry.clear();
    this.retainedScene = null;
    this.contextLost = false;
  }

  private assertContextReady(): void {
    if (this.contextLost) throw new Error("WebGL context is lost");
  }
}

export function buildAuthoredTextHitGeometry(
  operations: SemanticDrawOperation[],
  instances: GlyphInstance[],
): Map<string, AuthoredTextHitGeometry> {
  const output = new Map<string, AuthoredTextHitGeometry>();
  for (const operation of operations) {
    const quads = instances
      .filter((instance) => instance.layerId === operation.command.id)
      .map((instance) => instance.deviceCharQuad[1])
      .filter((quad) => quad.length >= 4);
    const points = quads.flat();
    if (points.length === 0) continue;
    const basis = quads.find((quad) => (
      Math.hypot(quad[1][0] - quad[0][0], quad[1][1] - quad[0][1]) > 1e-4
      && Math.hypot(quad[3][0] - quad[0][0], quad[3][1] - quad[0][1]) > 1e-4
    ));
    if (!basis) continue;
    const ux0 = basis[1][0] - basis[0][0];
    const uy0 = basis[1][1] - basis[0][1];
    const vx0 = basis[3][0] - basis[0][0];
    const vy0 = basis[3][1] - basis[0][1];
    const ul = Math.hypot(ux0, uy0);
    const vl = Math.hypot(vx0, vy0);
    const ux = ux0 / ul;
    const uy = uy0 / ul;
    const vx = vx0 / vl;
    const vy = vy0 / vl;
    const uValues = points.map(([x, y]) => x * ux + y * uy);
    const vValues = points.map(([x, y]) => x * vx + y * vy);
    const minU = Math.min(...uValues);
    const maxU = Math.max(...uValues);
    const minV = Math.min(...vValues);
    const maxV = Math.max(...vValues);
    const point = (u: number, v: number): [number, number] => [ux * u + vx * v, uy * u + vy * v];
    const quad: AuthoredTextHitGeometry["quad"] = [
      point(minU, minV),
      point(maxU, minV),
      point(maxU, maxV),
      point(minU, maxV),
    ];
    const xs = quad.map(([x]) => x);
    const ys = quad.map(([, y]) => y);
    const minX = Math.min(...xs);
    const maxX = Math.max(...xs);
    const minY = Math.min(...ys);
    const maxY = Math.max(...ys);
    output.set(operation.layerId, {
      bounds: { x: minX, y: minY, width: maxX - minX, height: maxY - minY },
      quad,
    });
  }
  return output;
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
