export type FreeTypeGlyph = {
  key: string;
  region: string;
  family: string;
  font_source_hash: string;
  ch: string;
  glyph_index: number;
  width: number;
  height: number;
  bearing_x: number;
  bearing_y: number;
  x_offset: number;
  y_offset: number;
  advance: number;
  plane_bearing_x: number;
  plane_bearing_y: number;
  plane_width: number;
  plane_height: number;
  drawable: boolean;
  pixels?: number[] | Uint8Array;
  pixels_base64?: string;
};

export type FreeTypeGlyphMapBatch = {
  region: string;
  family: string;
  font_source_hash: string;
  glyphs: Array<{ ch: string; glyph_index: number }>;
  missing: string[];
  error?: string;
};

export type FreeTypeGlyphBatchPerf = {
  total_ms: number;
  face_load_ms: number;
  glyph_total_ms: number;
  glyph_count: number;
  per_glyph_avg_ms: number;
  total_pixel_count: number;
  avg_pixels_per_glyph: number;
};

export type FreeTypeGlyphBatch = {
  region: string;
  family: string;
  font_source_hash: string;
  base_size: number;
  spread: number;
  glyphs: FreeTypeGlyph[];
  missing: string[];
  perf?: FreeTypeGlyphBatchPerf;
  error?: string;
};

export function decodeFreeTypePixels(glyph: FreeTypeGlyph): Uint8Array {
  if (glyph.pixels instanceof Uint8Array) return glyph.pixels;
  if (Array.isArray(glyph.pixels)) return new Uint8Array(glyph.pixels);
  if (!glyph.pixels_base64) return new Uint8Array(glyph.width * glyph.height);
  const raw = atob(glyph.pixels_base64);
  const pixels = new Uint8Array(raw.length);
  for (let index = 0; index < raw.length; index += 1) pixels[index] = raw.charCodeAt(index);
  return pixels;
}
