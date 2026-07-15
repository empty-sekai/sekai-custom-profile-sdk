import type { SemanticDrawOperation, SemanticResourceKey } from "./semanticCommandPlanner.js";

export type SemanticDrawBatchKind = "shape" | "image" | "mask" | "text" | "composite";
export type SemanticBlendMode = "src_over" | "src_in" | "dst_in" | "multiply" | "screen" | "add";
export type SemanticCompositeOperation = "marker" | "begin_isolation" | "end_isolation";

export type SemanticDrawBatch = {
  kind: SemanticDrawBatchKind;
  commandIds: string[];
  operations: SemanticDrawOperation[];
  vertices: Float32Array;
  layerSlots: Uint32Array;
  commandSlots: Uint32Array;
  resource: SemanticResourceKey | null;
  maskResource: SemanticResourceKey | null;
  blendMode: SemanticBlendMode;
  compositeOperation: SemanticCompositeOperation | null;
};

export const SEMANTIC_FLOATS_PER_VERTEX = 28;
export function semanticTextBatchKey(commandIds: readonly string[]): string {
  return `semantic-text-batch\0${commandIds.join("\0")}`;
}
const UNIT_TRIANGLES = [
  [0, 0], [1, 0], [1, 1],
  [0, 0], [1, 1], [0, 1],
] as const;

/** Compile immutable geometry only. Render mask and dynamic translation remain
 * in the dense layer-state textures, so toggles/ticks never rebuild vertices. */
export function compileSemanticDrawBatches(operations: SemanticDrawOperation[]): SemanticDrawBatch[] {
  const groups: Array<{ key: string; kind: SemanticDrawBatchKind; resource: SemanticResourceKey | null; maskResource: SemanticResourceKey | null; blendMode: SemanticBlendMode; compositeOperation: SemanticCompositeOperation | null; operations: SemanticDrawOperation[] }> = [];
  for (const operation of operations) {
    const descriptor = batchDescriptor(operation);
    const previous = groups.at(-1);
    if (previous?.key === descriptor.key) previous.operations.push(operation);
    else groups.push({ ...descriptor, operations: [operation] });
  }
  return groups.map((group) => compileGroup(group.kind, group.resource, group.maskResource, group.blendMode, group.compositeOperation, group.operations));
}

function batchDescriptor(operation: SemanticDrawOperation): {
  key: string;
  kind: SemanticDrawBatchKind;
  resource: SemanticResourceKey | null;
  maskResource: SemanticResourceKey | null;
  blendMode: SemanticBlendMode;
  compositeOperation: SemanticCompositeOperation | null;
} {
  const payload = operation.command.payload;
  const blendMode = commandBlendMode(operation.command.blend_mode);
  if (payload.kind === "image") {
    const resource = requireResource(payload.resource, operation.command.id);
    const maskResource = optionalResource(payload.alpha_mask);
    const maskKey = maskResource ? `\0${maskResource.namespace}\0${maskResource.key}` : "";
    return { key: `image\0${blendMode}\0${resource.namespace}\0${resource.key}${maskKey}`, kind: "image", resource, maskResource, blendMode, compositeOperation: null };
  }
  if (payload.kind === "shape") {
    const maskResource = assetMaskResource(payload.primitive);
    if (maskResource) return { key: `mask\0${blendMode}\0${maskResource.namespace}\0${maskResource.key}`, kind: "mask", resource: maskResource, maskResource: null, blendMode, compositeOperation: null };
    return { key: `shape\0${blendMode}`, kind: "shape", resource: null, maskResource: null, blendMode, compositeOperation: null };
  }
  if (payload.kind === "text") return { key: `text\0${blendMode}`, kind: "text", resource: null, maskResource: null, blendMode, compositeOperation: null };
  if (payload.kind === "composite") {
    const compositeOperation = requireCompositeOperation(payload.operation, operation.command.id);
    return { key: `composite\0${operation.command.id}`, kind: "composite", resource: null, maskResource: null, blendMode, compositeOperation };
  }
  throw new Error(`unsupported semantic command payload ${(payload as { kind?: unknown }).kind}`);
}

function compileGroup(
  kind: SemanticDrawBatchKind,
  resource: SemanticResourceKey | null,
  maskResource: SemanticResourceKey | null,
  blendMode: SemanticBlendMode,
  compositeOperation: SemanticCompositeOperation | null,
  operations: SemanticDrawOperation[]
): SemanticDrawBatch {
  if (kind === "text" || kind === "composite") {
    return { kind, resource, maskResource, blendMode, compositeOperation, operations: [...operations], commandIds: operations.map((op) => op.command.id), vertices: new Float32Array(), layerSlots: new Uint32Array(), commandSlots: new Uint32Array() };
  }
  const vertices = new Float32Array(operations.length * 6 * SEMANTIC_FLOATS_PER_VERTEX);
  const layerSlots = new Uint32Array(operations.length * 6);
  const commandSlots = new Uint32Array(operations.length * 6);
  let vertexOffset = 0;
  for (const operation of operations) {
    const bounds = requireRect(operation.command.bounds, operation.command.id);
    const commandMatrix = requireMatrix(operation.command.matrix, operation.command.id);
    const clip = commandClip(operation.command.clip, operation.baseMatrix, operation.command.id);
    const payload = operation.command.payload;
    const uv = payload.kind === "image" ? optionalRect(payload.uv) : { x: 0, y: 0, width: 1, height: 1 };
    const fill = payload.kind === "image" ? optionalColor(payload.tint, [1, 1, 1, 1]) : optionalColor(payload.fill, [1, 1, 1, 1]);
    const stroke = payload.kind === "shape" ? optionalColor(payload.stroke, [0, 0, 0, 0]) : [0, 0, 0, 0];
    const [primitive, radiusX, radiusY] = payload.kind === "shape"
      ? shapeParams(payload.primitive, bounds)
      : payload.kind === "image" ? imageClipParams(payload.clip, bounds) : [0, 0, 0];
    const strokeWidth = payload.kind === "shape" && typeof payload.stroke_width === "number"
      ? payload.stroke_width
      : 0;
    for (const [unitX, unitY] of UNIT_TRIANGLES) {
      const vertexFill = payload.kind === "shape" ? gradientColor(payload.gradient, unitX, unitY, fill) : fill;
      const localX = bounds.x + bounds.width * unitX;
      const localY = bounds.y + bounds.height * unitY;
      const commandPoint = transformPoint(commandMatrix, localX, localY);
      const [x, y] = transformPoint(operation.baseMatrix, commandPoint[0], commandPoint[1]);
      const base = vertexOffset * SEMANTIC_FLOATS_PER_VERTEX;
      vertices.set([
        x, y,
        uv.x + uv.width * unitX, uv.y + uv.height * unitY,
        unitX, unitY,
        ...vertexFill,
        ...stroke,
        primitive, radiusX, radiusY, strokeWidth,
        ...clip[0], ...clip[1], ...clip[2], ...clip[3],
        bounds.width, bounds.height,
      ], base);
      layerSlots[vertexOffset] = operation.layerSlot;
      commandSlots[vertexOffset] = operation.commandSlot;
      vertexOffset += 1;
    }
  }
  return {
    kind,
    resource,
    maskResource,
    blendMode,
    compositeOperation,
    operations: [...operations],
    commandIds: operations.map((operation) => operation.command.id),
    vertices,
    layerSlots,
    commandSlots,
  };
}

function commandBlendMode(value: unknown): SemanticBlendMode {
  if (value == null) return "src_over";
  if (value === "src_over" || value === "src_in" || value === "dst_in" || value === "multiply" || value === "screen" || value === "add") return value;
  throw new Error(`unsupported semantic blend mode ${String(value)}`);
}

function requireCompositeOperation(value: unknown, commandId: string): SemanticCompositeOperation {
  if (value == null || value === "marker") return "marker";
  if (value === "begin_isolation" || value === "end_isolation") return value;
  throw new Error(`unsupported composite operation ${commandId}: ${String(value)}`);
}

function commandClip(
  value: unknown,
  baseMatrix: [number, number, number, number, number, number],
  commandId: string
): [[number, number], [number, number], [number, number], [number, number]] {
  if (value == null) return [[-1e9, -1e9], [1e9, -1e9], [1e9, 1e9], [-1e9, 1e9]];
  if (!Array.isArray(value) || value.length !== 4 || value.some((point) => !Array.isArray(point) || point.length !== 2 || point.some((entry) => typeof entry !== "number" || !Number.isFinite(entry)))) {
    throw new Error(`invalid command clip ${commandId}`);
  }
  return value.map((point) => transformPoint(baseMatrix, point[0], point[1])) as [[number, number], [number, number], [number, number], [number, number]];
}

function gradientColor(value: unknown, x: number, y: number, fallback: number[]): number[] {
  if (!value || typeof value !== "object" || Array.isArray(value)) return fallback;
  const gradient = value as Record<string, unknown>;
  const start = optionalPoint(gradient.start, [0, 0.5]);
  const end = optionalPoint(gradient.end, [1, 0.5]);
  const startColor = optionalColor(gradient.start_color, fallback);
  const endColor = optionalColor(gradient.end_color, fallback);
  const dx = end[0] - start[0];
  const dy = end[1] - start[1];
  const lengthSq = dx * dx + dy * dy;
  const t = lengthSq > 1e-9 ? Math.max(0, Math.min(1, ((x - start[0]) * dx + (y - start[1]) * dy) / lengthSq)) : 0;
  return startColor.map((component, index) => component + (endColor[index] - component) * t);
}

function optionalPoint(value: unknown, fallback: [number, number]): [number, number] {
  return Array.isArray(value) && value.length === 2 && value.every((entry) => typeof entry === "number" && Number.isFinite(entry))
    ? [value[0], value[1]]
    : fallback;
}

function transformPoint(matrix: [number, number, number, number, number, number], x: number, y: number): [number, number] {
  return [matrix[0] * x + matrix[2] * y + matrix[4], matrix[1] * x + matrix[3] * y + matrix[5]];
}

function requireMatrix(value: unknown, commandId: string): [number, number, number, number, number, number] {
  if (!Array.isArray(value) || value.length !== 6 || value.some((entry) => typeof entry !== "number" || !Number.isFinite(entry))) {
    throw new Error(`invalid command matrix ${commandId}`);
  }
  return value as [number, number, number, number, number, number];
}

function requireRect(value: unknown, commandId: string): { x: number; y: number; width: number; height: number } {
  const rect = optionalRect(value);
  if (![rect.x, rect.y, rect.width, rect.height].every(Number.isFinite) || rect.width < 0 || rect.height < 0) {
    throw new Error(`invalid command bounds ${commandId}`);
  }
  return rect;
}

function optionalRect(value: unknown): { x: number; y: number; width: number; height: number } {
  if (!value || typeof value !== "object" || Array.isArray(value)) return { x: 0, y: 0, width: 1, height: 1 };
  const rect = value as Record<string, unknown>;
  return {
    x: Number(rect.x ?? 0),
    y: Number(rect.y ?? 0),
    width: Number(rect.width ?? 1),
    height: Number(rect.height ?? 1),
  };
}

function optionalColor(value: unknown, fallback: number[]): number[] {
  return Array.isArray(value) && value.length === 4 && value.every((entry) => typeof entry === "number" && Number.isFinite(entry))
    ? value
    : fallback;
}

function shapeParams(value: unknown, _bounds: { width: number; height: number }): [number, number, number] {
  if (value === "ellipse") return [2, 0, 0];
  if (value === "rect") return [0, 0, 0];
  if (!value || typeof value !== "object" || Array.isArray(value)) return [0, 0, 0];
  const rounded = (value as Record<string, unknown>).rounded_rect;
  if (rounded && typeof rounded === "object" && !Array.isArray(rounded)) {
    const radius = (rounded as Record<string, unknown>).radius;
    if (Array.isArray(radius) && radius.length === 2) {
      return [1, Number(radius[0]), Number(radius[1])];
    }
  }
  return [0, 0, 0];
}

function imageClipParams(value: unknown, _bounds: { width: number; height: number }): [number, number, number] {
  if (value === "ellipse") return [2, 0, 0];
  if (!value || typeof value !== "object" || Array.isArray(value)) return [0, 0, 0];
  const rounded = (value as Record<string, unknown>).rounded_rect;
  if (rounded && typeof rounded === "object" && !Array.isArray(rounded)) {
    const radius = (rounded as Record<string, unknown>).radius;
    if (Array.isArray(radius) && radius.length === 2) {
      return [1, Number(radius[0]), Number(radius[1])];
    }
  }
  return [0, 0, 0];
}

function assetMaskResource(value: unknown): SemanticResourceKey | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const mask = (value as Record<string, unknown>).asset_mask;
  if (!mask || typeof mask !== "object" || Array.isArray(mask)) return null;
  return requireResource((mask as Record<string, unknown>).resource, "asset-mask");
}

function requireResource(value: unknown, commandId: string): SemanticResourceKey {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`missing command resource ${commandId}`);
  const resource = value as Record<string, unknown>;
  if (typeof resource.namespace !== "string" || typeof resource.key !== "string") throw new Error(`invalid command resource ${commandId}`);
  return { namespace: resource.namespace, key: resource.key };
}

function optionalResource(value: unknown): SemanticResourceKey | null {
  if (value == null) return null;
  return requireResource(value, "image-alpha-mask");
}
