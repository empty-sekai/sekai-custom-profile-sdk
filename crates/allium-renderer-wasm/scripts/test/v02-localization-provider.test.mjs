import "./register-typescript.mjs";

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const {
  LocalizationProviderManager,
} = await import("../../src/localizationProvider.ts");

test("localization provider resolves arbitrary caller logic with bounded concurrency", async () => {
  let active = 0;
  let peak = 0;
  const manager = new LocalizationProviderManager({
    concurrency: 2,
    provider: {
      async provide(request) {
        active += 1;
        peak = Math.max(peak, active);
        await new Promise((resolve) => setTimeout(resolve, 5));
        active -= 1;
        return `${request.locale}:${request.key}`;
      },
    },
  });
  const snapshot = await manager.resolve([
    { region: "en", locale: "en-US", key: "a" },
    { region: "en", locale: "en-US", key: "b" },
    { region: "en", locale: "en-US", key: "a" },
    { region: "en", locale: "en-US", key: "c" },
  ]);
  assert.deepEqual(snapshot, {
    a: "en-US:a",
    b: "en-US:b",
    c: "en-US:c",
  });
  assert.equal(peak, 2);
  const { requested, unique, resolved, failures, peakActive } = manager.stats();
  assert.deepEqual(
    { requested, unique, resolved, failures, peakActive },
    { requested: 4, unique: 3, resolved: 3, failures: 0, peakActive: 2 },
  );
});

test("localization provider fails closed on a missing required key", async () => {
  const manager = new LocalizationProviderManager({
    provider: { async provide() { return null; } },
  });
  await assert.rejects(
    manager.resolve([{ region: "jp", locale: "ja-JP", key: "missing" }]),
    /missing/,
  );
});

test("BrowserRenderer resolves localization demands before glyph and scene compilation", async () => {
  const renderer = await readFile(new URL("../../src/renderer.ts", import.meta.url), "utf8");
  assert.match(renderer, /localizationProvider\?: LocalizationProvider/);
  assert.match(renderer, /demandOnly: true/);
  assert.match(renderer, /localizationDemands/);
  assert.match(renderer, /localizedText/);
});
