import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const source = (path) => readFile(new URL(`../../${path}`, import.meta.url), "utf8");

test("the stateful WASM ABI owns atlas placement, leases, revisions, and page pixels", async () => {
  const [rust, linkDriver] = await Promise.all([source("src/lib.rs"), source("src/main.rs")]);
  for (const symbol of [
    "sdf_atlas_create_json",
    "sdf_atlas_resolve_json",
    "sdf_atlas_pages_since_json",
    "sdf_atlas_page_pixels_ptr",
    "sdf_atlas_page_pixels_len",
    "sdf_atlas_release",
    "sdf_atlas_destroy",
  ]) {
    assert.match(rust, new RegExp(`fn ${symbol}`), symbol);
    assert.match(linkDriver, new RegExp(`exports::${symbol}`), symbol);
  }
});

test("the worker protocol exposes atlas sessions without a TypeScript packer", async () => {
  const [protocol, worker, client, facade] = await Promise.all([
    source("src/protocol.ts"),
    source("src/worker.ts"),
    source("src/worker-client.ts"),
    source("src/fontSdfAtlas.ts"),
  ]);
  for (const kind of ["createAtlas", "resolveAtlas", "atlasPages", "releaseAtlas", "destroyAtlas"]) {
    assert.match(protocol, new RegExp(`kind: "${kind}"`), kind);
  }
  assert.match(worker, /sdf_atlas_resolve_json/);
  assert.match(client, /class RendererAtlas/);
  assert.doesNotMatch(facade, /SessionSdfAtlas|putGlyph|placeAndPin/);
});

test("atlas pages remain 2048 square R8 with bounded four-to-six page budgets", async () => {
  const atlas = await source("src/atlas.rs");
  assert.match(atlas, /page_width: 2048/);
  assert.match(atlas, /page_height: 2048/);
  assert.match(atlas, /soft_pages: 4/);
  assert.match(atlas, /hard_pages: 6/);
  assert.match(atlas, /MEMORY_BUDGET_EXCEEDED/);
});

test("atlas state avoids randomized collections that call browser random on growable WASM memory", async () => {
  const atlas = await source("src/atlas.rs");
  assert.doesNotMatch(atlas, /\bHashMap\b|\bHashSet\b/);
  assert.match(atlas, /BTreeMap/);
  assert.match(atlas, /BTreeSet/);
});
