import type { GlyphInstance } from "../types/glyph.js";
import type { SemanticDrawOperation } from "./semanticCommandPlanner.js";

const TEXT_SCALE = 2;

type RenderPlacement = {
  anchor_x: number;
  baseline: number | null;
};

/**
 * Applies General's measured draw anchors after the Rust TMP layout has
 * finished. The returned instances retain the exact layout metrics and glyph
 * relationships; only final local/device draw coordinates are translated.
 */
export function placeGeneralTextInstances(
  instances: readonly GlyphInstance[],
  operations: readonly SemanticDrawOperation[],
): GlyphInstance[] {
  const operationByCommand = new Map(
    operations
      .filter((operation) => operation.command.payload.kind === "text")
      .map((operation) => [operation.command.id, operation] as const),
  );
  const translationByCommand = new Map<string, Translation>();

  return instances.map((instance) => {
    const operation = operationByCommand.get(instance.layerId);
    if (!operation) return cloneInstance(instance);
    const placement = renderPlacement(operation.command.render_placement);
    if (!placement) return cloneInstance(instance);
    let translation = translationByCommand.get(instance.layerId);
    if (!translation) {
      translation = placementTranslation(instance, operation, placement);
      translationByCommand.set(instance.layerId, translation);
    }
    return translateInstance(instance, translation);
  });
}

type Translation = {
  localX: number;
  localY: number;
  deviceX: number;
  deviceY: number;
};

function placementTranslation(
  instance: GlyphInstance,
  operation: SemanticDrawOperation,
  placement: RenderPlacement,
): Translation {
  const alignment = numericAlignment(operation.command.payload.alignment);
  const autoAnchor = alignment === 2
    ? 0
    : alignment === 4
      ? instance.layoutMetrics.boxW / 2
      : -instance.layoutMetrics.boxW / 2;
  const localX = placement.anchor_x - autoAnchor;
  const localY = placement.baseline == null
    ? 0
    : placement.baseline - instance.layoutMetrics.anchorBase;
  const commandMatrix = matrix(operation.command.matrix);
  const combined = multiply(operation.baseMatrix, commandMatrix);
  const scaledX = localX * TEXT_SCALE;
  const scaledY = localY * TEXT_SCALE;
  return {
    localX,
    localY,
    deviceX: combined[0] * scaledX + combined[2] * scaledY,
    deviceY: combined[1] * scaledX + combined[3] * scaledY,
  };
}

function translateInstance(instance: GlyphInstance, value: Translation): GlyphInstance {
  const localDeviceX = value.localX * TEXT_SCALE;
  const localDeviceY = value.localY * TEXT_SCALE;
  const output = cloneInstance(instance);
  output.charOp[1] += value.localX;
  output.charOp[2] += value.localY;
  output.charPosition[1] += localDeviceX;
  output.charPosition[2] -= localDeviceY;
  output.charQuad[1] = shiftPoints(output.charQuad[1], localDeviceX, -localDeviceY);
  output.quad = output.quad.map(([x, y, u, v, spread]) => [
    x + value.deviceX,
    y + value.deviceY,
    u,
    v,
    spread,
  ]);
  output.deviceCharPosition[1] += value.deviceX;
  output.deviceCharPosition[2] += value.deviceY;
  output.deviceCharQuad[1] = shiftPoints(
    output.deviceCharQuad[1],
    value.deviceX,
    value.deviceY,
  );
  output.deviceGlyphQuad[1] = shiftPoints(
    output.deviceGlyphQuad[1],
    value.deviceX,
    value.deviceY,
  );
  return output;
}

function cloneInstance(instance: GlyphInstance): GlyphInstance {
  return {
    ...instance,
    quad: instance.quad.map((value) => [...value]),
    charPosition: [...instance.charPosition],
    charOp: [...instance.charOp],
    charQuad: [instance.charQuad[0], instance.charQuad[1].map((value) => [...value])],
    deviceCharPosition: [...instance.deviceCharPosition],
    deviceCharQuad: [
      instance.deviceCharQuad[0],
      instance.deviceCharQuad[1].map((value) => [...value]),
    ],
    deviceGlyphQuad: [
      instance.deviceGlyphQuad[0],
      instance.deviceGlyphQuad[1].map((value) => [...value]),
    ],
    layoutMetrics: {
      ...instance.layoutMetrics,
      lineWidths: [...instance.layoutMetrics.lineWidths],
      rectWidths: [...instance.layoutMetrics.rectWidths],
      lineOffsets: [...instance.layoutMetrics.lineOffsets],
    },
    fill: [...instance.fill],
    outline: [...instance.outline],
  };
}

function shiftPoints(
  points: Array<[number, number]>,
  dx: number,
  dy: number,
): Array<[number, number]> {
  return points.map(([x, y]) => [x + dx, y + dy]);
}

function renderPlacement(value: unknown): RenderPlacement | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (!Number.isFinite(record.anchor_x)) return null;
  if (record.baseline != null && !Number.isFinite(record.baseline)) return null;
  return {
    anchor_x: Number(record.anchor_x),
    baseline: record.baseline == null ? null : Number(record.baseline),
  };
}

function numericAlignment(value: unknown): number {
  return Number.isInteger(value) ? Number(value) & 0x07 : 1;
}

function matrix(value: unknown): [number, number, number, number, number, number] {
  if (!Array.isArray(value) || value.length !== 6 || value.some((part) => !Number.isFinite(part))) {
    return [1, 0, 0, 1, 0, 0];
  }
  return value.map(Number) as [number, number, number, number, number, number];
}

function multiply(
  parent: [number, number, number, number, number, number],
  child: [number, number, number, number, number, number],
): [number, number, number, number, number, number] {
  return [
    parent[0] * child[0] + parent[2] * child[1],
    parent[1] * child[0] + parent[3] * child[1],
    parent[0] * child[2] + parent[2] * child[3],
    parent[1] * child[2] + parent[3] * child[3],
    parent[0] * child[4] + parent[2] * child[5] + parent[4],
    parent[1] * child[4] + parent[3] * child[5] + parent[5],
  ];
}
