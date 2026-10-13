/** A glyph instance produced by the shared Rust layout engine. */
export type GlyphInstance = {
  layerId: string;
  plainTextIndex: number;
  char: string;
  drawable: boolean;
  glyphKey: string;
  atlasPage: number;
  z: number;
  quad: Array<[number, number, number, number, number]>;
  charPosition: [string, number, number, number, number, number];
  charOp: [string, number, number, number, number, number, number];
  charQuad: [string, Array<[number, number]>];
  deviceCharPosition: [string, number, number];
  deviceCharQuad: [string, Array<[number, number]>];
  deviceGlyphQuad: [string, Array<[number, number]>];
  layoutMetrics: {
    lineWidths: number[];
    rectWidths: number[];
    boxW: number;
    anchorBase: number;
    lineOffsets: number[];
  };
  fill: [number, number, number, number];
  outline: [number, number, number, number];
  outlineWidth: number;
  shaderFontSize: number;
  shaderFaceScale: number;
  shaderFaceBias: number;
  shaderUnderlayScale: number;
  shaderUnderlayBias: number;
  shaderVertexAlpha: number;
};
