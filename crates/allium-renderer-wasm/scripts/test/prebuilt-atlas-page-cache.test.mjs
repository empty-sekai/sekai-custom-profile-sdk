import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const source = await readFile(new URL("../../src/prebuiltSdfAtlas.ts", import.meta.url), "utf8");
const originPackage = await readFile(new URL("../../src/originPrebuiltSdfAtlasPackage.ts", import.meta.url), "utf8");
const index = await readFile(new URL("../../src/index.ts", import.meta.url), "utf8");

test("decoded prebuilt atlas pages are reused per provider with a bounded cache", () => {
  assert.match(source, /const providerCaches = new WeakMap<PrebuiltSdfAtlasProvider, PrebuiltProviderCache>\(\)/);
  assert.match(source, /const MAX_DECODED_PAGES_PER_PROVIDER = 6/);
  assert.match(source, /cachedManifest\(provider, family, signal\)/);
  assert.match(source, /cachedDecodedPage\(provider, source\.family, descriptor, combined\.signal\)/);
  assert.match(source, /const key = `\$\{family\}\\0\$\{descriptor\.file\}\\0\$\{descriptor\.file_sha256\}`/);
  assert.match(source, /while \(cache\.size > MAX_DECODED_PAGES_PER_PROVIDER\)/);
});

test("origin atlas installation is explicit, bounded, and falls back when storage is unavailable", () => {
  assert.match(originPackage, /async manifest\(family, \{ signal \}\)/);
  assert.match(originPackage, /if \(!storage\.available\) return null/);
  assert.match(originPackage, /await assertStorageQuota\(totalBytes\)/);
  assert.match(originPackage, /if \(installOptions\.requestPersistence\) await requestOriginPersistence\(\)/);
  assert.match(originPackage, /await storage\.putManifests/);
  assert.match(originPackage, /await storage\.remove\(namespace, missing\)\.catch/);
  assert.match(originPackage, /const concurrency = boundedConcurrency\(installOptions\.concurrency \?\? 4\)/);
  assert.match(index, /createOriginPrebuiltSdfAtlasPackage/);
  assert.match(index, /PrebuiltSdfAtlasPackageStatus/);
});
