import { glyphKey, type GlyphRequest, type SdfAtlas } from "./fontSdfAtlas.js";
import type { AtlasPageUpdate } from "./types/atlas.js";

export type PrebuiltSdfAtlasPage = {
  file: string;
  width: number;
  height: number;
  file_sha256: string;
};

export type PrebuiltSdfAtlasGlyph = {
  codepoint: number;
  page: number;
  rect: [number, number, number, number];
  plane_bearing: [number, number];
  plane_size: [number, number];
  plane_advance_x: number;
};

export type PrebuiltSdfAtlasManifest = {
  schema: "allium.sdf-atlas-manifest.v1";
  generator_contract: string;
  font_family: string;
  font_sha256: string;
  point_size: number;
  spread: number;
  pages: PrebuiltSdfAtlasPage[];
  glyphs: PrebuiltSdfAtlasGlyph[];
};

export interface PrebuiltSdfAtlasProvider {
  manifest(family: string, context: { signal: AbortSignal }): Promise<PrebuiltSdfAtlasManifest | null>;
  page(family: string, file: string, context: { signal: AbortSignal }): Promise<ArrayBuffer>;
}

type PrebuiltProviderCache = {
  manifests: Map<string, PrebuiltSdfAtlasManifest | null>;
  decodedPages: Map<string, Uint8Array>;
};

const MAX_DECODED_PAGES_PER_PROVIDER = 6;
const providerCaches = new WeakMap<PrebuiltSdfAtlasProvider, PrebuiltProviderCache>();

export function createHttpPrebuiltSdfAtlasProvider(baseUrl: string): PrebuiltSdfAtlasProvider {
  const root = baseUrl.replace(/\/+$/, "");
  const familyPath = (family: string) => encodeURIComponent(family);
  return {
    async manifest(family, { signal }) {
      const response = await fetch(`${root}/${familyPath(family)}/manifest.json`, { signal, cache: "force-cache" });
      if (response.status === 404) return null;
      if (!response.ok) throw new Error(`prebuilt atlas manifest ${family}: HTTP ${response.status}`);
      return response.json() as Promise<PrebuiltSdfAtlasManifest>;
    },
    async page(family, file, { signal }) {
      if (!/^page-\d{3}\.r8swz\.br$/.test(file)) throw new Error(`invalid prebuilt atlas page path ${file}`);
      const response = await fetch(`${root}/${familyPath(family)}/${file}`, { signal, cache: "force-cache" });
      if (!response.ok) throw new Error(`prebuilt atlas page ${family}/${file}: HTTP ${response.status}`);
      return response.arrayBuffer();
    },
  };
}

export async function buildPrebuiltSdfAtlas(
  provider: PrebuiltSdfAtlasProvider,
  requests: GlyphRequest[],
  signal: AbortSignal,
  maxPages = 6,
): Promise<SdfAtlas | null> {
  const families = [...new Set(requests.map((request) => request.family))];
  const manifests = new Map<string, PrebuiltSdfAtlasManifest>();
  for (const family of families) {
    const manifest = await cachedManifest(provider, family, signal);
    if (!manifest || !isValidPrebuiltSdfAtlasManifest(manifest, family)) return null;
    manifests.set(family, manifest);
  }
  const glyphByFamily = new Map([...manifests].map(([family, manifest]) => [
    family,
    new Map(manifest.glyphs.map((glyph) => [glyph.codepoint, glyph] as const)),
  ] as const));
  const resolved = requests.map((request) => {
    const codepoint = singleCodepoint(request.char);
    const glyph = codepoint == null ? undefined : glyphByFamily.get(request.family)?.get(codepoint);
    return glyph ? { request, glyph } : null;
  });
  if (resolved.some((entry) => entry == null)) return null;

  const sourcePages = [...new Map(resolved.map((entry) => {
    const value = entry!;
    const key = `${value.request.family}\0${value.glyph.page}`;
    return [key, { family: value.request.family, sourcePage: value.glyph.page }] as const;
  })).values()];
  if (sourcePages.length > maxPages) return null;
  const compactPage = new Map(sourcePages.map((page, index) => [`${page.family}\0${page.sourcePage}`, index] as const));
  const pagePromises = new Map<number, Promise<Uint8Array>>();
  const controller = new AbortController();
  const combined = combineSignals(signal, controller.signal);
  let released = false;
  const pagePixels = (page: number): Promise<Uint8Array> => {
    const cached = pagePromises.get(page);
    if (cached) return cached;
    const source = sourcePages[page];
    const manifest = manifests.get(source.family)!;
    const descriptor = manifest.pages[source.sourcePage];
    const promise = cachedDecodedPage(provider, source.family, descriptor, combined.signal);
    pagePromises.set(page, promise);
    return promise;
  };
  const glyphs = new Map(resolved.map((entry) => {
    const { request, glyph } = entry!;
    const manifest = manifests.get(request.family)!;
    const descriptor = manifest.pages[glyph.page];
    const page = compactPage.get(`${request.family}\0${glyph.page}`)!;
    const [x, y, width, height] = glyph.rect;
    const key = glyphKey(request.region, request.fontSourceHash, request.family, request.char);
    return [key, {
      key,
      resourceId: `prebuilt:${manifest.font_sha256}:${glyph.codepoint}`,
      glyphIndex: glyph.codepoint,
      page,
      pageEpoch: 1,
      region: request.region,
      family: request.family,
      fontSourceHash: request.fontSourceHash,
      char: request.char,
      u0: x / descriptor.width,
      v0: y / descriptor.height,
      u1: (x + width) / descriptor.width,
      v1: (y + height) / descriptor.height,
      width,
      height,
      advance: glyph.plane_advance_x,
      xOffset: glyph.plane_bearing[0],
      yOffset: glyph.plane_bearing[1],
      planeBearingX: glyph.plane_bearing[0],
      planeBearingY: glyph.plane_bearing[1],
      planeWidth: glyph.plane_size[0],
      planeHeight: glyph.plane_size[1],
      drawable: width > 0 && height > 0,
    }] as const;
  }));
  const first = manifests.values().next().value as PrebuiltSdfAtlasManifest;
  const contractId = `prebuilt:${[...manifests.values()].map((manifest) => manifest.font_sha256).join(":")}:${sourcePages.map((page) => `${page.family}:${page.sourcePage}`).join(",")}`;
  return {
    width: 2048,
    height: 2048,
    depth: sourcePages.length,
    baseSize: first.point_size,
    spread: first.spread,
    glyphs,
    missing: [],
    backend: "freetype-wasm",
    sdfBackend: "edt",
    contractId,
    fontEngineFingerprint: first.generator_contract,
    perf: {
      backend: "edt",
      fontBatches: [],
      totalGlyphMs: 0,
      totalFaceLoadMs: 0,
      totalGlyphCount: 0,
      totalPixelCount: 0,
      worker: emptyWorkerStats(),
      cache: {
        hits: glyphs.size, misses: 0, generations: 0, bytes: sourcePages.length * 2048 * 2048,
        sessionHits: glyphs.size, persistentHits: 0, persistentMisses: 0,
        persistentWritesQueued: 0, pages: sourcePages.length, pinnedPages: sourcePages.length, pageEvictions: 0,
      },
    },
    async pageUpdates(revisions) {
      const updates: AtlasPageUpdate[] = [];
      for (let page = 0; page < sourcePages.length; page += 1) {
        if ((revisions.get(page) ?? 0) >= 1) continue;
        updates.push({
          page, pageWidth: 2048, pageEpoch: 1, revision: 1, fullUpload: true,
          pixels: await pagePixels(page), dirtyRects: [],
        });
      }
      return updates;
    },
    async release() {
      if (released) return;
      released = true;
      controller.abort();
      combined.dispose();
      pagePromises.clear();
    },
  };
}

function providerCache(provider: PrebuiltSdfAtlasProvider): PrebuiltProviderCache {
  let cache = providerCaches.get(provider);
  if (!cache) {
    cache = { manifests: new Map(), decodedPages: new Map() };
    providerCaches.set(provider, cache);
  }
  return cache;
}

async function cachedManifest(
  provider: PrebuiltSdfAtlasProvider,
  family: string,
  signal: AbortSignal,
): Promise<PrebuiltSdfAtlasManifest | null> {
  if (signal.aborted) throw abortReason(signal);
  const cache = providerCache(provider).manifests;
  if (cache.has(family)) return cache.get(family) ?? null;
  const manifest = await provider.manifest(family, { signal });
  if (!signal.aborted) cache.set(family, manifest);
  return manifest;
}

async function cachedDecodedPage(
  provider: PrebuiltSdfAtlasProvider,
  family: string,
  descriptor: PrebuiltSdfAtlasPage,
  signal: AbortSignal,
): Promise<Uint8Array> {
  if (signal.aborted) throw abortReason(signal);
  const cache = providerCache(provider).decodedPages;
  const key = `${family}\0${descriptor.file}\0${descriptor.file_sha256}`;
  const cached = cache.get(key);
  if (cached) {
    cache.delete(key);
    cache.set(key, cached);
    return cached;
  }
  const buffer = await provider.page(family, descriptor.file, { signal });
  const bytes = new Uint8Array(buffer);
  await verifySha256(bytes, descriptor.file_sha256);
  if (signal.aborted) throw abortReason(signal);
  const decoded = unswizzleR8(bytes, descriptor.width, descriptor.height);
  cache.set(key, decoded);
  while (cache.size > MAX_DECODED_PAGES_PER_PROVIDER) {
    const oldest = cache.keys().next().value as string | undefined;
    if (oldest == null) break;
    cache.delete(oldest);
  }
  return decoded;
}

function abortReason(signal: AbortSignal): unknown {
  return signal.reason ?? new DOMException("The operation was aborted", "AbortError");
}

export function isValidPrebuiltSdfAtlasManifest(manifest: PrebuiltSdfAtlasManifest, family: string): boolean {
  return manifest.schema === "allium.sdf-atlas-manifest.v1"
    && manifest.font_family === family
    && manifest.font_sha256.length === 64
    && Number.isFinite(manifest.point_size) && manifest.point_size > 0
    && Number.isFinite(manifest.spread) && manifest.spread > 0
    && manifest.pages.length > 0
    && manifest.pages.every((page) => page.width === 2048 && page.height === 2048 && /^page-\d{3}\.r8swz\.br$/.test(page.file));
}

function singleCodepoint(value: string): number | null {
  const values = [...value];
  return values.length === 1 ? values[0].codePointAt(0) ?? null : null;
}

function unswizzleR8(bytes: Uint8Array, width: number, height: number): Uint8Array {
  const header = 64;
  const payloadLength = width * height;
  if (bytes.byteLength !== header + payloadLength) throw new Error(`invalid R8SWZ page length ${bytes.byteLength}`);
  if (new TextDecoder().decode(bytes.subarray(0, 10)) !== "ALLIUMSWZ8") throw new Error("invalid R8SWZ page magic");
  const output = new Uint8Array(payloadLength);
  const blocksPerRow = width / 8;
  for (let y = 0; y < height; y += 1) {
    const blockY = Math.floor(y / 8);
    const inY = y % 8;
    for (let x = 0; x < width; x += 1) {
      const block = blockY * blocksPerRow + Math.floor(x / 8);
      output[y * width + x] = bytes[header + block * 64 + inY * 8 + (x % 8)];
    }
  }
  return output;
}

async function verifySha256(bytes: Uint8Array, expected: string): Promise<void> {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes.slice().buffer));
  const actual = [...digest].map((value) => value.toString(16).padStart(2, "0")).join("");
  if (actual !== expected.toLowerCase()) throw new Error(`prebuilt atlas page hash mismatch: expected ${expected}, got ${actual}`);
}

function combineSignals(first: AbortSignal, second: AbortSignal): { signal: AbortSignal; dispose(): void } {
  const controller = new AbortController();
  const abort = (event: Event) => controller.abort((event.target as AbortSignal).reason);
  first.addEventListener("abort", abort, { once: true });
  second.addEventListener("abort", abort, { once: true });
  if (first.aborted) controller.abort(first.reason);
  else if (second.aborted) controller.abort(second.reason);
  return {
    signal: controller.signal,
    dispose() {
      first.removeEventListener("abort", abort);
      second.removeEventListener("abort", abort);
    },
  };
}

function emptyWorkerStats() {
  return {
    protocol: "allium.renderer-worker/2" as const,
    initialized: true, scenes: 0, masterDataSessions: 0, atlasSessions: 0,
    authoringSessions: 0, fonts: 0, requests: 0, failures: 0, wasmMs: 0, bridgeBytes: 0,
  };
}
