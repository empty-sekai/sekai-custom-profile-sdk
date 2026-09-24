import type { SemanticDrawOperation, SemanticResourceKey } from "./semanticCommandPlanner.js";
import type { UguiTextLayout, UguiTextMesh } from "../types/uguiText.js";

/** `badge` is an image drawn with the lit badge material; `ugui_glyph` holds
 * the glyph quads of uGUI text commands, drawn from one coverage page. */
export type SemanticDrawBatchKind = "shape" | "image" | "badge" | "mask" | "text" | "ugui_glyph" | "composite";
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
  normalMapResource: SemanticResourceKey | null;
  blendMode: SemanticBlendMode;
  compositeOperation: SemanticCompositeOperation | null;
  /** Coverage page of a `ugui_glyph` batch; null for every other kind. */
  glyphPage: number | null;
};

/** Per vertex: position, UV, shape UV, fill, stroke, params, clip quad,
 * shape size, and the canvas direction of the command's local +x axis.
 *
 * A `ugui_glyph` vertex carries the texel position in its glyph cell as UV,
 * the vertex colour as fill, the bitmap's page rectangle (x, y, width, rows)
 * as stroke, the cell padding as the first param, and the cell size as shape
 * size. */
export const SEMANTIC_FLOATS_PER_VERTEX = 30;
export function semanticTextBatchKey(commandIds: readonly string[]): string {
  return `semantic-text-batch\0${commandIds.join("\0")}`;
}
const UNIT_TRIANGLES = [
  [0, 0], [1, 0], [1, 1],
  [0, 0], [1, 1], [0, 1],
] as const;
/** Quad corner at a unit position: `UNIT_CORNER[y][x]`. */
const UNIT_CORNER = [[0, 1], [3, 2]] as const;

/** Glyph quads of one uGUI text command that sit on one coverage page. */
type GlyphRun = {
  operation: SemanticDrawOperation;
  mesh: UguiTextMesh;
  page: number;
  quads: UguiTextMesh["quads"];
};

type DrawItem = { operation: SemanticDrawOperation; run: GlyphRun | null };

/** Compile immutable geometry only. Render mask and dynamic translation remain
 * in the dense layer-state textures, so toggles/ticks never rebuild vertices.
 * `uguiText` holds the layout of every `ugui_text` command. */
export function compileSemanticDrawBatches(
  operations: SemanticDrawOperation[],
  uguiText: UguiTextLayout | null = null,
): SemanticDrawBatch[] {
  const meshes = new Map((uguiText?.texts ?? []).map((mesh) => [mesh.id, mesh] as const));
  const groups: Array<{ key: string; kind: SemanticDrawBatchKind; resource: SemanticResourceKey | null; maskResource: SemanticResourceKey | null; normalMapResource: SemanticResourceKey | null; blendMode: SemanticBlendMode; compositeOperation: SemanticCompositeOperation | null; glyphPage: number | null; items: DrawItem[] }> = [];
  for (const operation of operations) {
    for (const item of drawItems(operation, meshes, uguiText)) {
      const descriptor = item.run
        ? glyphBatchDescriptor(item.operation, item.run.page)
        : { ...batchDescriptor(item.operation), glyphPage: null };
      const previous = groups.at(-1);
      if (previous?.key === descriptor.key) previous.items.push(item);
      else groups.push({ ...descriptor, items: [item] });
    }
  }
  return groups.map((group) => group.kind === "ugui_glyph"
    ? compileGlyphGroup(group.blendMode, group.glyphPage ?? 0, group.items.map((item) => item.run!), uguiText!)
    : compileGroup(group.kind, group.resource, group.maskResource, group.normalMapResource, group.blendMode, group.compositeOperation, group.items.map((item) => item.operation)));
}

/** A uGUI text command draws its backdrop as a solid rectangle, the way
 * shapes are drawn, then its glyph quads in string order, one run per
 * stretch of quads on the same coverage page. Every other command draws
 * itself. */
function drawItems(
  operation: SemanticDrawOperation,
  meshes: ReadonlyMap<string, UguiTextMesh>,
  uguiText: UguiTextLayout | null,
): DrawItem[] {
  if (operation.command.payload.kind !== "ugui_text") return [{ operation, run: null }];
  const mesh = meshes.get(operation.command.id);
  if (!mesh || !uguiText) throw new Error(`missing uGUI text layout ${operation.command.id}`);
  const items: DrawItem[] = [];
  if (mesh.backdrop) {
    items.push({
      operation: {
        ...operation,
        command: {
          ...operation.command,
          bounds: { ...mesh.backdrop.rect },
          payload: { kind: "shape", primitive: "rect", fill: [...mesh.backdrop.color], stroke: [0, 0, 0, 0], stroke_width: 0 },
        },
      },
      run: null,
    });
  }
  let run: GlyphRun | null = null;
  for (const quad of mesh.quads) {
    const glyph = uguiText.glyphs[quad.glyph];
    if (!glyph) throw new Error(`uGUI text ${operation.command.id} names missing glyph ${quad.glyph}`);
    if (!run || run.page !== glyph.page) {
      run = { operation, mesh, page: glyph.page, quads: [] };
      items.push({ operation, run });
    }
    run.quads.push(quad);
  }
  return items;
}

function glyphBatchDescriptor(operation: SemanticDrawOperation, page: number) {
  const blendMode = commandBlendMode(operation.command.blend_mode);
  return {
    key: `ugui_glyph\0${blendMode}\0${page}`,
    kind: "ugui_glyph" as const,
    resource: null,
    maskResource: null,
    normalMapResource: null,
    blendMode,
    compositeOperation: null,
    glyphPage: page,
  };
}

/** Maps each glyph cell onto its quad: corner k of the quad is corner k of
 * the cell (top-left, top-right, bottom-right, bottom-left), placed on the
 * canvas by the layer, command and text-node matrices. */
function compileGlyphGroup(
  blendMode: SemanticBlendMode,
  page: number,
  runs: GlyphRun[],
  uguiText: UguiTextLayout,
): SemanticDrawBatch {
  const quadCount = runs.reduce((sum, run) => sum + run.quads.length, 0);
  const vertices = new Float32Array(quadCount * 6 * SEMANTIC_FLOATS_PER_VERTEX);
  const layerSlots = new Uint32Array(quadCount * 6);
  const commandSlots = new Uint32Array(quadCount * 6);
  let vertexOffset = 0;
  for (const { operation, mesh, quads } of runs) {
    const commandId = operation.command.id;
    const payload = operation.command.payload;
    const commandMatrix = requireMatrix(operation.command.matrix, commandId);
    const nodeMatrix = requireMatrix(payload.node_matrix, commandId);
    const clip = commandClip(operation.command.clip, operation.baseMatrix, commandId);
    const color = requireColor(payload.color, commandId);
    const axis = linearAxis(operation.baseMatrix, commandMatrix);
    const padding = mesh.cellPadding;
    for (const quad of quads) {
      const glyph = uguiText.glyphs[quad.glyph];
      const cellWidth = glyph.width + 2 * padding;
      const cellHeight = glyph.rows + 2 * padding;
      for (const [unitX, unitY] of UNIT_TRIANGLES) {
        const [nodeX, nodeY] = quad.corners[UNIT_CORNER[unitY][unitX]];
        const commandPoint = transformPoint(nodeMatrix, nodeX, nodeY);
        const layerPoint = transformPoint(commandMatrix, commandPoint[0], commandPoint[1]);
        const [x, y] = transformPoint(operation.baseMatrix, layerPoint[0], layerPoint[1]);
        vertices.set([
          x, y,
          unitX * cellWidth, unitY * cellHeight,
          unitX, unitY,
          ...color,
          glyph.x, glyph.y, glyph.width, glyph.rows,
          padding, 0, 0, 0,
          ...clip[0], ...clip[1], ...clip[2], ...clip[3],
          cellWidth, cellHeight,
          ...axis,
        ], vertexOffset * SEMANTIC_FLOATS_PER_VERTEX);
        layerSlots[vertexOffset] = operation.layerSlot;
        commandSlots[vertexOffset] = operation.commandSlot;
        vertexOffset += 1;
      }
    }
  }
  const batchOperations = runs.map((run) => run.operation);
  return {
    kind: "ugui_glyph",
    resource: null,
    maskResource: null,
    normalMapResource: null,
    blendMode,
    compositeOperation: null,
    glyphPage: page,
    operations: batchOperations,
    commandIds: [...new Set(batchOperations.map((operation) => operation.command.id))],
    vertices,
    layerSlots,
    commandSlots,
  };
}

function batchDescriptor(operation: SemanticDrawOperation): {
  key: string;
  kind: SemanticDrawBatchKind;
  resource: SemanticResourceKey | null;
  maskResource: SemanticResourceKey | null;
  normalMapResource: SemanticResourceKey | null;
  blendMode: SemanticBlendMode;
  compositeOperation: SemanticCompositeOperation | null;
} {
  const payload = operation.command.payload;
  const blendMode = commandBlendMode(operation.command.blend_mode);
  if (payload.kind === "image") {
    const resource = requireResource(payload.resource, operation.command.id);
    const maskResource = optionalResource(payload.alpha_mask);
    const maskKey = maskResource ? `\0${maskResource.namespace}\0${maskResource.key}` : "";
    const normalMapResource = imageNormalMap(payload.material, operation.command.id);
    if (normalMapResource) {
      const normalMapKey = `\0${normalMapResource.namespace}\0${normalMapResource.key}`;
      return { key: `badge\0${blendMode}\0${resource.namespace}\0${resource.key}${maskKey}${normalMapKey}`, kind: "badge", resource, maskResource, normalMapResource, blendMode, compositeOperation: null };
    }
    return { key: `image\0${blendMode}\0${resource.namespace}\0${resource.key}${maskKey}`, kind: "image", resource, maskResource, normalMapResource: null, blendMode, compositeOperation: null };
  }
  if (payload.kind === "shape") {
    const maskResource = assetMaskResource(payload.primitive);
    if (maskResource) return { key: `mask\0${blendMode}\0${maskResource.namespace}\0${maskResource.key}`, kind: "mask", resource: maskResource, maskResource: null, normalMapResource: null, blendMode, compositeOperation: null };
    return { key: `shape\0${blendMode}`, kind: "shape", resource: null, maskResource: null, normalMapResource: null, blendMode, compositeOperation: null };
  }
  if (payload.kind === "text") return { key: `text\0${blendMode}`, kind: "text", resource: null, maskResource: null, normalMapResource: null, blendMode, compositeOperation: null };
  if (payload.kind === "composite") {
    const compositeOperation = requireCompositeOperation(payload.operation, operation.command.id);
    return { key: `composite\0${operation.command.id}`, kind: "composite", resource: null, maskResource: null, normalMapResource: null, blendMode, compositeOperation };
  }
  throw new Error(`unsupported semantic command payload ${(payload as { kind?: unknown }).kind}`);
}

function compileGroup(
  kind: SemanticDrawBatchKind,
  resource: SemanticResourceKey | null,
  maskResource: SemanticResourceKey | null,
  normalMapResource: SemanticResourceKey | null,
  blendMode: SemanticBlendMode,
  compositeOperation: SemanticCompositeOperation | null,
  operations: SemanticDrawOperation[]
): SemanticDrawBatch {
  if (kind === "text" || kind === "composite") {
    return { kind, resource, maskResource, normalMapResource, blendMode, compositeOperation, glyphPage: null, operations: [...operations], commandIds: operations.map((op) => op.command.id), vertices: new Float32Array(), layerSlots: new Uint32Array(), commandSlots: new Uint32Array() };
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
    const axis = linearAxis(operation.baseMatrix, commandMatrix);
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
        ...axis,
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
    normalMapResource,
    blendMode,
    compositeOperation,
    glyphPage: null,
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

/** Canvas direction of the local +x axis under `base` after `command`. */
function linearAxis(
  base: [number, number, number, number, number, number],
  command: [number, number, number, number, number, number],
): [number, number] {
  return [base[0] * command[0] + base[2] * command[1], base[1] * command[0] + base[3] * command[1]];
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

function requireColor(value: unknown, commandId: string): number[] {
  if (!Array.isArray(value) || value.length !== 4 || value.some((entry) => typeof entry !== "number" || !Number.isFinite(entry))) {
    throw new Error(`invalid command colour ${commandId}`);
  }
  return value;
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

/** Normal map of a lit-badge image material; null for a plain image, whose
 * JSON omits the material. */
function imageNormalMap(value: unknown, commandId: string): SemanticResourceKey | null {
  if (value == null) return null;
  if (typeof value !== "object" || Array.isArray(value)) throw new Error(`invalid image material ${commandId}`);
  const material = value as Record<string, unknown>;
  if (material.kind === "plain") return null;
  if (material.kind !== "lit_badge") throw new Error(`unsupported image material ${commandId}: ${String(material.kind)}`);
  return requireResource(material.normal_map, commandId);
}

function optionalResource(value: unknown): SemanticResourceKey | null {
  if (value == null) return null;
  return requireResource(value, "image-alpha-mask");
}
