import type { GlyphInstance } from "../types/glyph.js";

type NumericRun = { text: string; plain_start: number; plain_end: number };
type NumericCommand = {
  id: string;
  layer_id: string;
  clip?: Array<[number, number]> | null;
  numeric_text_runs?: NumericRun[];
};

export type NumericTextRegion = {
  id: string;
  layer_id: string;
  role: "numeric_run";
  bounds: { x: number; y: number; width: number; height: number };
  quad: [[number, number], [number, number], [number, number], [number, number]];
  hit_geometry: [[number, number], [number, number], [number, number], [number, number]];
  clip: Array<[number, number]> | null;
  resolved_data: { text: string; plain_start: number; plain_end: number };
  capabilities: [];
  render_mask: boolean;
};

export type NumericRegionRuntimeState = {
  layerTransform: { dx: number; dy: number };
  commandTransform: { dx: number; dy: number };
};

export function buildNumericTextRegions(
  command: NumericCommand,
  instances: Array<Pick<GlyphInstance, "layerId" | "plainTextIndex" | "deviceCharQuad">>,
  renderMask: boolean,
  runtimeState?: NumericRegionRuntimeState,
): NumericTextRegion[] {
  return (command.numeric_text_runs ?? []).flatMap((run) => {
    const points = instances
      .filter((instance) => instance.layerId === command.id && instance.plainTextIndex >= run.plain_start && instance.plainTextIndex < run.plain_end)
      .flatMap((instance) => instance.deviceCharQuad[1]);
    if (points.length === 0) return [];
    const minX = Math.min(...points.map(([x]) => x));
    const minY = Math.min(...points.map(([, y]) => y));
    const maxX = Math.max(...points.map(([x]) => x));
    const maxY = Math.max(...points.map(([, y]) => y));
    const quad: NumericTextRegion["quad"] = [[minX, minY], [maxX, minY], [maxX, maxY], [minX, maxY]];
    const region: NumericTextRegion = {
      id: `${command.id}:numeric:${run.plain_start}:${run.plain_end}`,
      layer_id: command.layer_id,
      role: "numeric_run",
      bounds: { x: minX, y: minY, width: maxX - minX, height: maxY - minY },
      quad,
      hit_geometry: quad,
      clip: command.clip ?? null,
      resolved_data: { text: run.text, plain_start: run.plain_start, plain_end: run.plain_end },
      capabilities: [],
      render_mask: renderMask,
    };
    return [runtimeState ? applyNumericRegionRuntimeState(region, runtimeState) : region];
  });
}

export function applyNumericRegionRuntimeState(
  region: NumericTextRegion,
  state: NumericRegionRuntimeState,
): NumericTextRegion {
  const dx = state.layerTransform.dx + state.commandTransform.dx;
  const dy = state.layerTransform.dy + state.commandTransform.dy;
  const translate = ([x, y]: [number, number]): [number, number] => [x + dx, y + dy];
  const translateClip = ([x, y]: [number, number]): [number, number] => [
    x + state.layerTransform.dx,
    y + state.layerTransform.dy,
  ];
  const quad = region.quad.map(translate) as NumericTextRegion["quad"];
  return {
    ...region,
    bounds: { ...region.bounds, x: region.bounds.x + dx, y: region.bounds.y + dy },
    quad,
    hit_geometry: quad,
    clip: region.clip?.map(translateClip) ?? null,
  };
}
