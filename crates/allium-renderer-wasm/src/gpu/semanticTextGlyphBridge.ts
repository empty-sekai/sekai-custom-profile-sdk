import type { GlyphInstance } from "../types/glyph.js";
import { commandClipQuad, type ClipQuad } from "./semanticCommandGeometry.js";
import { buildSdfGlyphInstanceVertices, type SdfAtlasPageSize } from "./webglSdfGlyphPipeline.js";
import type { SemanticDrawOperation } from "./semanticCommandPlanner.js";

export function compileSemanticTextGlyphBatches(
  instances: GlyphInstance[],
  textOperations: SemanticDrawOperation[],
  pageSize: SdfAtlasPageSize,
): Map<string, Float32Array> {
  const result = new Map<string, Float32Array>();
  for (const operation of textOperations) {
    if (operation.command.payload.kind !== "text") throw new Error(`non-text glyph bridge operation ${operation.command.id}`);
    const commandInstances = instances.filter((instance) => instance.layerId === operation.command.id);
    const clips = operation.command.clip == null
      ? new Map<string, ClipQuad>()
      : new Map([[operation.command.id, commandClipQuad(operation.command.clip, operation.baseMatrix, operation.command.id)]]);
    result.set(
      operation.command.id,
      buildSdfGlyphInstanceVertices(
        commandInstances,
        new Map([[operation.command.id, operation.layerSlot]]),
        pageSize,
        clips,
        new Map([[operation.command.id, operation.commandSlot]])
      )
    );
  }
  return result;
}
