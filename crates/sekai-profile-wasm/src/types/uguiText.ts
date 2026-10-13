/** Glyph quads and coverage pages of a scene's uGUI text commands, laid out by
 * the shared core layout in the renderer worker. */
export type UguiTextLayout = {
  version: 1;
  texts: UguiTextMesh[];
  glyphs: UguiGlyphPlacement[];
  pages: UguiGlyphPage[];
};

export type UguiTextMesh = {
  /** Id of the `ugui_text` command. */
  id: string;
  /** Preferred width of the text in canvas units. */
  preferredWidth: number;
  /** Empty texels around the bitmap in every glyph cell. */
  cellPadding: number;
  /** Solid rectangle drawn under the glyphs, in layer space. */
  backdrop: {
    rect: { x: number; y: number; width: number; height: number };
    color: [number, number, number, number];
  } | null;
  /** Quads that show a glyph, in string order. */
  quads: Array<{
    /** Node-space corners of the glyph cell: top-left, top-right,
     * bottom-right, bottom-left. */
    corners: [[number, number], [number, number], [number, number], [number, number]];
    /** Index into `UguiTextLayout.glyphs`. */
    glyph: number;
  }>;
};

export type UguiGlyphPlacement = {
  page: number;
  /** Top-left texel of the bitmap in its page. */
  x: number;
  y: number;
  width: number;
  rows: number;
  bitmapLeft: number;
  bitmapTop: number;
  /** Unhinted advance, 26.6 fixed point. */
  advance26d6: number;
};

export type UguiGlyphPage = {
  width: number;
  height: number;
  /** `height` rows of `width` 8-bit coverage values, top row first. */
  pixels: Uint8Array;
};
