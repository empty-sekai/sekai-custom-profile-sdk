const DEFAULT_STATIC_BASE = "https://profile-assets.emptysekai.dev/game-profile/cn/6.0.0/renderer-static";
const ASSET_REGIONS = new Set(["jp", "cn", "tw", "kr", "en"]);
const STATIC_CACHE = "sekai-custom-profile-sdk-demo-static-v0.3";

/** Game asset base for `region`: its asset tree on storage.pjsk.moe, without a trailing slash. */
export function defaultAssetBase(region) {
  const key = String(region || "cn").toLowerCase();
  return `https://storage.pjsk.moe/sekai-${ASSET_REGIONS.has(key) ? key : "cn"}-assets`;
}

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
  return `${normalized.assetBase}/${encodePath(gameAssetPath(rawKey))}`;
}

function gameAssetPath(key) {
  const normalized = stripPng(key);
  const character = normalized.match(/^bonds_honor\/chr_sd_(.+)$/);
  if (character) {
    const name = `chr_sd_${character[1]}`;
    return `bonds_honor/character/${name}/${name}.png`;
  }
  const word = normalized.match(/^bonds_honor\/word\/(.+)$/);
  if (word) return `bonds_honor/word/${word[1]}/${word[1]}.png`;
  return `${normalized}.png`;
}

function normalizeConfig(config) {
  const region = String(config.region || "cn").toLowerCase();
  return {
    region,
    assetBase: trimSlash(config.assetBase ?? defaultAssetBase(region)),
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
