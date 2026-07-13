import type { GlyphInstance } from "../types/glyph.js";
import { buildSdfGlyphInstanceVertices } from "./webglSdfGlyphPipeline.js";
import type { SemanticDrawOperation } from "./semanticCommandPlanner.js";

export function compileSemanticTextGlyphBatches(
  instances: GlyphInstance[],
  textOperations: SemanticDrawOperation[]
): Map<string, Float32Array> {
  const result = new Map<string, Float32Array>();
  for (const operation of textOperations) {
    if (operation.command.payload.kind !== "text") throw new Error(`non-text glyph bridge operation ${operation.command.id}`);
    const commandInstances = instances.filter((instance) => instance.layerId === operation.command.id);
    const clips = operation.command.clip == null
      ? new Map<string, [[number, number], [number, number], [number, number], [number, number]]>()
      : new Map([[operation.command.id, transformClip(operation.command.clip, operation.baseMatrix, operation.command.id)]]);
    result.set(
      operation.command.id,
      buildSdfGlyphInstanceVertices(
        commandInstances,
        new Map([[operation.command.id, operation.layerSlot]]),
        clips,
        new Map([[operation.command.id, operation.commandSlot]])
      )
    );
  }
  return result;
}

function transformClip(value: unknown, matrix: [number, number, number, number, number, number], commandId: string): [[number, number], [number, number], [number, number], [number, number]] {
  if (!Array.isArray(value) || value.length !== 4 || value.some((point) => !Array.isArray(point) || point.length !== 2)) throw new Error(`invalid text clip ${commandId}`);
  return value.map((point) => [
    matrix[0] * Number(point[0]) + matrix[2] * Number(point[1]) + matrix[4],
    matrix[1] * Number(point[0]) + matrix[3] * Number(point[1]) + matrix[5],
  ]) as [[number, number], [number, number], [number, number], [number, number]];
}
