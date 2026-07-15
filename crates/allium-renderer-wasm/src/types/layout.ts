import type { GlyphInstance } from "./glyph.js";

export type CoreDynamicProgramDescriptor = {
  layerId: string;
  percent: number;
  lineAdvancesTmp: number[][];
  rotationDeg: number;
  scaleX: number;
};

export type WasmLayoutBatch = {
  version: number;
  source: string;
  instances: GlyphInstance[];
  dynamicPrograms: CoreDynamicProgramDescriptor[];
  error?: string;
};

export type WasmGlyphDemandBatch = {
  version: number;
  source: "wasm-tmp-glyph-demand";
  requests: Array<{
    region: string;
    family: string;
    font_source_hash: string;
    char: string;
  }>;
};
