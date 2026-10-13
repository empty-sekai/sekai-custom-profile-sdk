/** Text command input consumed by the shared Rust TMP layout engine. */
export type TextLayer = {
  id: string;
  parentId?: string | null;
  documentKey: string;
  z: number;
  text: string;
  authoredVisible: boolean;
  region: string;
  fontId: number;
  fontFamily: string;
  fontSourceHash: string;
  x: number;
  y: number;
  rotationDeg: number;
  scaleX: number;
  scaleY: number;
  skewX: number;
  fontSize: number;
  color: [number, number, number, number];
  outlineColor: [number, number, number, number];
  colorRgb: [number, number, number];
  colorId: number;
  outlineColorId: number;
  outlineWidth: number;
  lineSpacing: number;
  textType: number;
};
