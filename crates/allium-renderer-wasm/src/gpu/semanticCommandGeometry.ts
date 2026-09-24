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

/** One float attribute of a semantic vertex: its shader location, its
 * component count and its offset in floats. */
export type SemanticVertexAttribute = { location: number; size: number; offset: number };

/** Float attributes of a semantic vertex. Every value is shared by the six
 * vertices of a quad except `corner`: the fragment stage maps each device
 * pixel into the draw's local space itself and decides there what it shows.
 *
 * - `inverse`, `inverseOffset`: the canvas-to-local matrix of the draw, `[a,
 *   b, c, d]` and `[e, f]` (`x' = a x + c y + e`), before the dynamic offsets
 *   and the preview transform the vertex stage applies.
 * - `bounds`: the drawn rectangle in local space (x, y, width, height), and
 *   `corner` the unit corner of the vertex.
 * - `uvRect`: the image region an image or mask samples (x, y, width,
 *   height in `0..=1`).
 * - `fill`, `stroke`: the fill (a shape's gradient start colour, an image's
 *   tint, an asset mask's face colour) and the stroke colour, straight RGBA.
 * - `params`: the primitive (0 rectangle, 1 rounded rectangle, 2 ellipse),
 *   its corner radii and the stroke width.
 * - `gradient`, `gradientEndColor`: a shape's gradient line (start x, y, end
 *   x, y in normalised bounds coordinates) and end colour; a zero line keeps
 *   the fill.
 * - `clip01`, `clip23`: the clip quad on the canvas before the dynamic offset
 *   and the preview transform (see `commandClipQuad`).
 *
 * A `ugui_glyph` vertex maps the canvas onto its glyph cell, in texels: its
 * bounds are the cell, its `uvRect` the bitmap's rectangle on the coverage
 * page (x, y, width, rows), its fill the vertex colour and its first param
 * the cell padding. */
export const SEMANTIC_VERTEX_ATTRIBUTES = {
  inverse: { location: 0, size: 4, offset: 0 },
  inverseOffset: { location: 1, size: 2, offset: 4 },
  bounds: { location: 2, size: 4, offset: 6 },
  corner: { location: 3, size: 2, offset: 10 },
  uvRect: { location: 4, size: 4, offset: 12 },
  fill: { location: 5, size: 4, offset: 16 },
  stroke: { location: 6, size: 4, offset: 20 },
  params: { location: 7, size: 4, offset: 24 },
  gradient: { location: 8, size: 4, offset: 28 },
  gradientEndColor: { location: 9, size: 4, offset: 32 },
  clip01: { location: 10, size: 4, offset: 36 },
  clip23: { location: 11, size: 4, offset: 40 },
} as const satisfies Record<string, SemanticVertexAttribute>;
export const SEMANTIC_FLOATS_PER_VERTEX = 44;
/** Locations of the integer attributes, each in a buffer of its own: the
 * layer's and the command's state slot. */
export const SEMANTIC_LAYER_SLOT_LOCATION = 12;
export const SEMANTIC_COMMAND_SLOT_LOCATION = 13;

export function semanticTextBatchKey(commandIds: readonly string[]): string {
  return `semantic-text-batch\0${commandIds.join("\0")}`;
}
const UNIT_TRIANGLES = [
  [0, 0], [1, 0], [1, 1],
  [0, 0], [1, 1], [0, 1],
] as const;

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

/** Maps the canvas onto each glyph cell the way the native compositor does:
 * the cell's corners (top-left, top-right, bottom-right, bottom-left) are
 * placed on the canvas by the layer, command and text-node matrices, and the
 * inverse of the matrix spanned by the first, second and fourth corner sends
 * a pixel to its cell texel position. A cell whose corners span no area
 * draws nothing. */
function compileGlyphGroup(
  blendMode: SemanticBlendMode,
  page: number,
  runs: GlyphRun[],
  uguiText: UguiTextLayout,
): SemanticDrawBatch {
  const quads: QuadValues[] = [];
  for (const { operation, mesh, quads: glyphQuads } of runs) {
    const commandId = operation.command.id;
    const payload = operation.command.payload;
    const commandMatrix = requireMatrix(operation.command.matrix, commandId);
    const nodeMatrix = requireMatrix(payload.node_matrix, commandId);
    const clip = commandClipQuad(operation.command.clip, operation.baseMatrix, commandId);
    const color = requireColor(payload.color, commandId);
    const device = composeMatrix(composeMatrix(operation.baseMatrix, commandMatrix), nodeMatrix);
    const padding = mesh.cellPadding;
    for (const quad of glyphQuads) {
      const glyph = uguiText.glyphs[quad.glyph];
      const cellWidth = glyph.width + 2 * padding;
      const cellHeight = glyph.rows + 2 * padding;
      const [origin, right, , down] = quad.corners.map(([x, y]) => transformPoint(device, x, y));
      const toCell = invertMatrix([
        f32(f32(right[0] - origin[0]) / cellWidth),
        f32(f32(right[1] - origin[1]) / cellWidth),
        f32(f32(down[0] - origin[0]) / cellHeight),
        f32(f32(down[1] - origin[1]) / cellHeight),
        origin[0],
        origin[1],
      ]);
      if (!toCell) continue;
      quads.push({
        operation,
        inverse: toCell,
        bounds: [0, 0, cellWidth, cellHeight],
        uvRect: [glyph.x, glyph.y, glyph.width, glyph.rows],
        fill: color,
        stroke: [0, 0, 0, 0],
        params: [padding, 0, 0, 0],
        gradient: [0, 0, 0, 0],
        gradientEndColor: color,
        clip,
      });
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
    ...packQuads(quads),
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
  const quads: QuadValues[] = [];
  for (const operation of operations) {
    const bounds = requireRect(operation.command.bounds, operation.command.id);
    const commandMatrix = requireMatrix(operation.command.matrix, operation.command.id);
    const clip = commandClipQuad(operation.command.clip, operation.baseMatrix, operation.command.id);
    const inverse = invertMatrix(composeMatrix(operation.baseMatrix, commandMatrix));
    // Empty bounds cover no pixel, and a matrix without an inverse maps no
    // pixel back into the command.
    if (!inverse || bounds.width <= 0 || bounds.height <= 0) continue;
    const payload = operation.command.payload;
    if (payload.kind === "image") {
      const uv = optionalRect(payload.uv);
      const tint = optionalColor(payload.tint, [1, 1, 1, 1]);
      quads.push({
        operation,
        inverse,
        bounds: [bounds.x, bounds.y, bounds.width, bounds.height],
        uvRect: [uv.x, uv.y, uv.width, uv.height],
        fill: tint,
        stroke: [0, 0, 0, 0],
        params: [...imageClipParams(payload.clip), 0],
        gradient: [0, 0, 0, 0],
        gradientEndColor: tint,
        clip,
      });
      continue;
    }
    const fill = optionalColor(payload.fill, [1, 1, 1, 1]);
    // An asset mask's face is its fill: the native compositor draws it with
    // the shape distance-field material, whose face colour is solid, so a
    // gradient on the command does not reach it.
    const gradient = shapeGradient(kind === "mask" ? null : payload.gradient, fill);
    quads.push({
      operation,
      inverse,
      bounds: [bounds.x, bounds.y, bounds.width, bounds.height],
      uvRect: [0, 0, 1, 1],
      fill: gradient.startColor,
      stroke: optionalColor(payload.stroke, [0, 0, 0, 0]),
      params: [...shapeParams(payload.primitive), typeof payload.stroke_width === "number" ? payload.stroke_width : 0],
      gradient: gradient.line,
      gradientEndColor: gradient.endColor,
      clip,
    });
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
    ...packQuads(quads),
  };
}

type Quad4 = [number, number, number, number];

/** The values one quad's six vertices share. */
type QuadValues = {
  operation: SemanticDrawOperation;
  inverse: Matrix2d;
  bounds: Quad4;
  uvRect: Quad4;
  fill: number[];
  stroke: number[];
  params: number[];
  gradient: Quad4;
  gradientEndColor: number[];
  clip: [[number, number], [number, number], [number, number], [number, number]];
};

/** Writes two triangles per quad in `SEMANTIC_VERTEX_ATTRIBUTES` order. */
function packQuads(quads: readonly QuadValues[]): Pick<SemanticDrawBatch, "vertices" | "layerSlots" | "commandSlots"> {
  const vertices = new Float32Array(quads.length * 6 * SEMANTIC_FLOATS_PER_VERTEX);
  const layerSlots = new Uint32Array(quads.length * 6);
  const commandSlots = new Uint32Array(quads.length * 6);
  const layout = SEMANTIC_VERTEX_ATTRIBUTES;
  let vertex = 0;
  for (const quad of quads) {
    for (const corner of UNIT_TRIANGLES) {
      const base = vertex * SEMANTIC_FLOATS_PER_VERTEX;
      vertices.set(quad.inverse.slice(0, 4), base + layout.inverse.offset);
      vertices.set(quad.inverse.slice(4, 6), base + layout.inverseOffset.offset);
      vertices.set(quad.bounds, base + layout.bounds.offset);
      vertices.set(corner, base + layout.corner.offset);
      vertices.set(quad.uvRect, base + layout.uvRect.offset);
      vertices.set(quad.fill, base + layout.fill.offset);
      vertices.set(quad.stroke, base + layout.stroke.offset);
      vertices.set(quad.params, base + layout.params.offset);
      vertices.set(quad.gradient, base + layout.gradient.offset);
      vertices.set(quad.gradientEndColor, base + layout.gradientEndColor.offset);
      vertices.set([...quad.clip[0], ...quad.clip[1]], base + layout.clip01.offset);
      vertices.set([...quad.clip[2], ...quad.clip[3]], base + layout.clip23.offset);
      layerSlots[vertex] = quad.operation.layerSlot;
      commandSlots[vertex] = quad.operation.commandSlot;
      vertex += 1;
    }
  }
  return { vertices, layerSlots, commandSlots };
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

export type ClipQuad = [[number, number], [number, number], [number, number], [number, number]];

/** The clip of a command that has none: it holds the whole canvas. */
export const NO_CLIP: ClipQuad = [[-1e9, -1e9], [1e9, -1e9], [1e9, 1e9], [-1e9, 1e9]];

/** `pixel_sampling::CLIP_AXIS_TOLERANCE` of the shared core. */
export const CLIP_AXIS_TOLERANCE = Math.fround(0.0001);

/** A command's clip quad on the canvas, before the dynamic offset and the
 * preview transform: its corners through the layer's matrix. Corners that
 * form an axis-aligned rectangle become the corners of their bounds, as the
 * native compositor reduces such a clip to its bounds (see
 * `axisAlignedClipBounds`); the fragment stages keep `[min, max)` of those
 * on both axes. Any other quad, which the native compositor does not draw,
 * is kept as it is. */
export function commandClipQuad(value: unknown, baseMatrix: readonly number[], commandId: string): ClipQuad {
  if (value == null) return NO_CLIP;
  if (!Array.isArray(value) || value.length !== 4 || value.some((point) => !Array.isArray(point) || point.length !== 2 || point.some((entry) => typeof entry !== "number" || !Number.isFinite(entry)))) {
    throw new Error(`invalid command clip ${commandId}`);
  }
  const corners = value.map((point) => transformPoint(baseMatrix, point[0], point[1])) as ClipQuad;
  const bounds = axisAlignedClipBounds(corners);
  if (!bounds) return corners;
  const [minX, minY, maxX, maxY] = bounds;
  return [[minX, minY], [maxX, minY], [maxX, maxY], [minX, maxY]];
}

/** `pixel_sampling::axis_aligned_clip_bounds` of the shared core: the bounds
 * `[minX, minY, maxX, maxY]` of corners that, in order around the clip, form
 * an axis-aligned rectangle within `CLIP_AXIS_TOLERANCE`, or null. */
export function axisAlignedClipBounds(corners: ClipQuad): [number, number, number, number] | null {
  if (!corners.flat().every(Number.isFinite)) return null;
  const horizontal = (left: [number, number], right: [number, number]) =>
    Math.abs(f32(left[1] - right[1])) <= CLIP_AXIS_TOLERANCE && Math.abs(f32(left[0] - right[0])) > CLIP_AXIS_TOLERANCE;
  const vertical = (top: [number, number], bottom: [number, number]) =>
    Math.abs(f32(top[0] - bottom[0])) <= CLIP_AXIS_TOLERANCE && Math.abs(f32(top[1] - bottom[1])) > CLIP_AXIS_TOLERANCE;
  const [c0, c1, c2, c3] = corners;
  const rectangle = (horizontal(c0, c1) && vertical(c1, c2) && horizontal(c2, c3) && vertical(c3, c0))
    || (vertical(c0, c1) && horizontal(c1, c2) && vertical(c2, c3) && horizontal(c3, c0));
  if (!rectangle) return null;
  const xs = corners.map((corner) => corner[0]);
  const ys = corners.map((corner) => corner[1]);
  return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
}

/** A shape's gradient line and end colours; without a gradient the line is
 * empty and both colours are the fill. */
function shapeGradient(value: unknown, fill: number[]): { line: Quad4; startColor: number[]; endColor: number[] } {
  if (!value || typeof value !== "object" || Array.isArray(value)) return { line: [0, 0, 0, 0], startColor: fill, endColor: fill };
  const gradient = value as Record<string, unknown>;
  const start = optionalPoint(gradient.start, [0, 0.5]);
  const end = optionalPoint(gradient.end, [1, 0.5]);
  return {
    line: [start[0], start[1], end[0], end[1]],
    startColor: optionalColor(gradient.start_color, fill),
    endColor: optionalColor(gradient.end_color, fill),
  };
}

function optionalPoint(value: unknown, fallback: [number, number]): [number, number] {
  return Array.isArray(value) && value.length === 2 && value.every((entry) => typeof entry === "number" && Number.isFinite(entry))
    ? [value[0], value[1]]
    : fallback;
}

type Matrix2d = [number, number, number, number, number, number];

const f32 = Math.fround;
/** `f32::EPSILON`. */
const F32_EPSILON = 2 ** -23;

// The three functions below evaluate in single precision in the native
// compositor's order of operations, so a device pixel maps to the same local
// position on both backends. A single-precision sum, difference, product or
// quotient computed in double precision and then rounded is the correctly
// rounded single-precision result.

/** `parent` after `child`, as the native compositor composes matrices. */
export function composeMatrix(parent: readonly number[], child: readonly number[]): Matrix2d {
  const p = parent.map(f32);
  const c = child.map(f32);
  return [
    f32(f32(p[0] * c[0]) + f32(p[2] * c[1])),
    f32(f32(p[1] * c[0]) + f32(p[3] * c[1])),
    f32(f32(p[0] * c[2]) + f32(p[2] * c[3])),
    f32(f32(p[1] * c[2]) + f32(p[3] * c[3])),
    f32(f32(f32(p[0] * c[4]) + f32(p[2] * c[5])) + p[4]),
    f32(f32(f32(p[1] * c[4]) + f32(p[3] * c[5])) + p[5]),
  ];
}

/** The inverse, or null for a matrix the native compositor does not invert. */
export function invertMatrix(matrix: readonly number[]): Matrix2d | null {
  const [a, b, c, d, e, f] = matrix.map(f32);
  const determinant = f32(f32(a * d) - f32(b * c));
  if (!Number.isFinite(determinant) || Math.abs(determinant) <= F32_EPSILON) return null;
  return [
    f32(d / determinant),
    f32(-b / determinant),
    f32(-c / determinant),
    f32(a / determinant),
    f32(f32(f32(c * f) - f32(d * e)) / determinant),
    f32(f32(f32(b * e) - f32(a * f)) / determinant),
  ];
}

/** A point through the matrix with fused multiply-adds. Each fused result
 * is rounded to double and then to single precision, which differs from a
 * single rounding only when the double lies exactly halfway between two
 * single-precision values. */
export function transformPoint(matrix: readonly number[], x: number, y: number): [number, number] {
  const m = matrix.map(f32);
  const [px, py] = [f32(x), f32(y)];
  return [
    f32(m[0] * px + f32(m[2] * py + m[4])),
    f32(m[1] * px + f32(m[3] * py + m[5])),
  ];
}

function requireMatrix(value: unknown, commandId: string): Matrix2d {
  if (!Array.isArray(value) || value.length !== 6 || value.some((entry) => typeof entry !== "number" || !Number.isFinite(entry))) {
    throw new Error(`invalid command matrix ${commandId}`);
  }
  return value as Matrix2d;
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

function shapeParams(value: unknown): [number, number, number] {
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

function imageClipParams(value: unknown): [number, number, number] {
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
