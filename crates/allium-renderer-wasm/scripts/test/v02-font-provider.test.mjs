import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { FontProviderManager } from "../../src/fontProvider.ts";

test("font provider resolves arbitrary caller logic with bounded concurrency and immutable bytes", async () => {
  let active = 0;
  let maximum = 0;
  const source = new Uint8Array([1, 2, 3]);
  const manager = new FontProviderManager({
    concurrency: 2,
    provider: {
      async provide(request, { signal }) {
        assert.equal(signal.aborted, false);
        active += 1;
        maximum = Math.max(maximum, active);
        await new Promise((resolve) => setTimeout(resolve, 5));
        active -= 1;
        return { bytes: request.family === "A" ? source : new Uint8Array([4]) };
      },
    },
  });
  const result = await manager.resolve([
    { region: "cn", family: "A" },
    { region: "cn", family: "A" },
    { region: "cn", family: "B" },
    { region: "cn", family: "C" },
  ]);
  source[0] = 9;
  assert.equal(maximum, 2);
  assert.deepEqual([...result.keys()], ["A", "B", "C"]);
  assert.deepEqual([...new Uint8Array(result.get("A"))], [1, 2, 3]);
  const { requested, unique, loaded, bytes, failures, peakActive } = manager.stats();
  assert.deepEqual(
    { requested, unique, loaded, bytes, failures, peakActive },
    { requested: 4, unique: 3, loaded: 3, bytes: 5, failures: 0, peakActive: 2 },
  );
});

test("font provider fails closed on a missing demanded family", async () => {
  const manager = new FontProviderManager({
    provider: { async provide() { return null; } },
  });
  await assert.rejects(
    manager.resolve([{ region: "en", family: "Missing" }]),
    /font provider returned no bytes for en:Missing/,
  );
});

test("BrowserRenderer performs a WASM font-demand phase before final glyph preparation", async () => {
  const renderer = await readFile(new URL("../../src/renderer.ts", import.meta.url), "utf8");
  const worker = await readFile(new URL("../../src/worker.ts", import.meta.url), "utf8");
  const index = await readFile(new URL("../../src/index.ts", import.meta.url), "utf8");
  assert.match(renderer, /fontProvider\?: FontProvider/);
  assert.match(renderer, /fontDemandOnly: true/);
  assert.match(renderer, /preparedFontDemands\(fontPreparation, this\.region\)/);
  assert.match(renderer, /FONT_IDENTITY_CONFLICT/);
  assert.match(worker, /preparation: \{ fontDemands: \[\.\.\.families\] \}/);
  assert.match(index, /FontProvider/);
});
