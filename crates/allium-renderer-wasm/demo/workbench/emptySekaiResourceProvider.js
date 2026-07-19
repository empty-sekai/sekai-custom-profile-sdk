const DEFAULT_STATIC_BASE = "https://cdn.emptysekai.com/renderer-static/v0.2";
const STATIC_CACHE = "sekai-custom-profile-sdk-demo-static-v0.3";
const ROOT_ASSET_PREFIXES = [
  "ugc/editor_image/",
  "ugc/avatar/",
  "uploads/editor_image/",
  "uploads/avatar/",
  "presets/",
];

export function createEmptySekaiResourceProvider(config) {
  const normalized = normalizeConfig(config);
  return {
    cacheIdentity(resource) {
      return emptySekaiResourceUrl(resource, normalized);
    },
    async provide(resource, context) {
      const url = emptySekaiResourceUrl(resource, normalized);
      const persistent = resource.namespace === "static" && typeof caches !== "undefined"
        ? await caches.open(STATIC_CACHE).catch(() => null)
        : null;
      const cached = persistent ? await persistent.match(url).catch(() => undefined) : undefined;
      if (cached?.ok) return { source: await cached.blob() };
      const response = await fetch(url, { cache: "default", signal: context.signal });
      if (!response.ok) throw new Error(`resource fetch failed ${response.status}`);
      if (persistent) await persistent.put(url, response.clone()).catch(() => undefined);
      return { source: await response.blob() };
    },
  };
}

export function emptySekaiResourceUrl(resource, config) {
  const normalized = normalizeConfig(config);
  const rawKey = String(resource.key).replace(/^\/+/, "");
  if (resource.namespace === "static") {
    return `${normalized.staticBase}/${encodePath(stripPng(rawKey))}.png`;
  }
  if (resource.namespace !== "assets") {
    throw new Error(`unsupported EmptySekai demo resource namespace ${resource.namespace}`);
  }
  const objectPath = gameAssetObjectPath(rawKey, normalized.region);
  const standardPrefix = `assets/${normalized.region}/`;
  if (objectPath.startsWith(standardPrefix)) {
    return `${normalized.assetBase}/${encodePath(objectPath.slice(standardPrefix.length))}`;
  }
  return `${new URL(normalized.assetBase).origin}/${encodePath(objectPath)}`;
}

function gameAssetObjectPath(key, region) {
  if (ROOT_ASSET_PREFIXES.some((prefix) => key.startsWith(prefix))) return key;
  const normalized = stripPng(key);
  const character = normalized.match(/^bonds_honor\/chr_sd_(.+)$/);
  if (character) {
    const name = `chr_sd_${character[1]}`;
    return `assets/${region}/bonds_honor/character/${name}/${name}.png`;
  }
  const word = normalized.match(/^bonds_honor\/word\/(.+)$/);
  if (word) return `assets/${region}/bonds_honor/word/${word[1]}/${word[1]}.png`;
  return `assets/${region}/${normalized}.png`;
}

function normalizeConfig(config) {
  return {
    region: String(config.region || "cn").toLowerCase(),
    assetBase: trimSlash(config.assetBase),
    staticBase: trimSlash(config.staticBase || DEFAULT_STATIC_BASE),
  };
}

function stripPng(key) {
  return key.replace(/\.png$/i, "");
}

function trimSlash(value) {
  const result = String(value || "").trim().replace(/\/+$/, "");
  if (!/^https?:\/\//i.test(result)) throw new Error(`EmptySekai demo provider requires an HTTP(S) base: ${result || "empty value"}`);
  return result;
}

function encodePath(path) {
  return path.split("/").map(encodeURIComponent).join("/");
}
