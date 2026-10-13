import type { GlyphBatchRequest } from "../protocol.js";
import type { GlyphRasterIdentity } from "../cache/glyphPersistentCache.js";

export type GlyphRasterPlan = {
  region: string;
  family: string;
  fontSourceHash: string;
  schemaNamespace: string;
  fontEngineFingerprint: string;
  rasterContractId: string;
  contractId: string;
  backend: "edt" | "analytic";
  supersample: number;
  baseSize: number;
  spread: number;
  atlasWidth: number;
  atlasHeight: number;
  glyphs: Array<{ ch: string; glyphIndex: number; identity: GlyphRasterIdentity }>;
  missing: string[];
};

export type AtlasPixelRect = { x: number; y: number; width: number; height: number };

export type AtlasPlacement = {
  page: number;
  pageEpoch: number;
  pixelRect: AtlasPixelRect;
  u0: number;
  v0: number;
  u1: number;
  v1: number;
};

export type AtlasGlyphRecord = {
  key: string;
  glyphIndex: number;
  width: number;
  height: number;
  advance: number;
  xOffset: number;
  yOffset: number;
  planeBearingX: number;
  planeBearingY: number;
  planeWidth: number;
  planeHeight: number;
  drawable: boolean;
  pixels: Uint8Array;
};

export type AtlasGenerateRequest = GlyphBatchRequest & {
  glyphs: Array<{ key: string; char: string; glyphIndex: number }>;
};

export type AtlasStats = {
  pages: number;
  pinnedPages: number;
  glyphs: number;
  atlasBytes: number;
  evictions: number;
  pageAllocations: number;
  hardBudgetBytes: number;
};

export type AtlasResolveResult = {
  leases: number[];
  placements: Array<{ key: string; placement: AtlasPlacement }>;
  missingKeys: string[];
  generated: AtlasGlyphRecord[];
  stats: AtlasStats;
};

export type AtlasPageUpdate = {
  page: number;
  pageWidth: number;
  pageEpoch: number;
  revision: number;
  fullUpload: boolean;
  pixels: Uint8Array;
  dirtyRects: Array<AtlasPixelRect & { pixels: Uint8Array }>;
};
