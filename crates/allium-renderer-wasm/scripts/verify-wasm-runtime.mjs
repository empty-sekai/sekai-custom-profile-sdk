import assert from "node:assert/strict";
import createAlliumRenderer from "../dist/allium_renderer_wasm.js";

const module = await createAlliumRenderer({ noInitialRun: true });
assert.equal(typeof module._sdf_layout_freetype_plan_glyphs_json, "function");
const encoder = new TextEncoder();
const decoder = new TextDecoder();

const readCString = (pointer) => {
  let end = pointer;
  while (module.HEAPU8[end] !== 0) end += 1;
  return decoder.decode(module.HEAPU8.subarray(pointer, end));
};

const callJson = (name, types, values) => {
  const pointer = module.ccall(name, "number", types, values);
  try {
    const result = JSON.parse(readCString(pointer));
    assert.equal(result.error, undefined, `${name}: ${result.error}`);
    return result;
  } finally {
    module.ccall("sdf_layout_freetype_free_string", null, ["number"], [pointer]);
  }
};

const callJsonInput = (name, input, prefix = []) => {
  const bytes = encoder.encode(JSON.stringify(input));
  const pointer = module._malloc(bytes.byteLength);
  try {
    module.HEAPU8.set(bytes, pointer);
    return callJson(
      name,
      [...prefix.map(() => "number"), "number", "number"],
      [...prefix, pointer, bytes.byteLength],
    );
  } finally {
    module._free(pointer);
  }
};

const contract = callJson("sdf_layout_freetype_contract_json", [], []);
assert.deepEqual(contract.modules, ["truetype", "cff", "sfnt", "psaux", "psnames", "smooth"]);

const demand = callJsonInput("sdf_layout_freetype_glyph_demand_json", {
  layers: [{
    text: "<uppercase>aß</uppercase> 12",
    region: "en",
    fontFamily: "SyntheticSans",
    fontSourceHash: "a".repeat(64),
  }],
});
assert.deepEqual(demand.requests.map((request) => request.char), ["A", "S", "1", "2"]);

const masterData = callJsonInput("sdf_renderer_core_masterdata_create_json", {
  region: "en",
  revision: "smoke",
});
assert.ok(Number.isInteger(masterData.handle) && masterData.handle > 0);
const stats = callJson("sdf_renderer_core_masterdata_stats_json", ["number"], [masterData.handle]);
assert.equal(stats.region, "en");
assert.equal(module.ccall("sdf_renderer_core_masterdata_destroy", "number", ["number"], [masterData.handle]), 1);

const atlas = callJsonInput("sdf_atlas_create_json", {
  pageWidth: 2048,
  pageHeight: 2048,
  softPages: 4,
  hardPages: 6,
});
assert.ok(Number.isInteger(atlas.handle) && atlas.handle > 0);
const resolved = callJsonInput("sdf_atlas_resolve_json", {
  keys: ["synthetic-glyph"],
  records: [{ key: "synthetic-glyph", width: 2, height: 2, pixelsBase64: "AQIDBA==" }],
}, [atlas.handle]);
assert.equal(resolved.placements[0].placement.page, 0);
assert.equal(resolved.placements[0].placement.pixelRect.x, 1);
assert.equal(resolved.missingKeys.length, 0);
const pages = callJsonInput("sdf_atlas_pages_since_json", { revisions: [] }, [atlas.handle]);
assert.equal(pages[0].fullUpload, true);
assert.equal(module.ccall("sdf_atlas_page_pixels_len", "number", ["number", "number"], [atlas.handle, 0]), 2048 * 2048);
assert.equal(module.ccall("sdf_atlas_release", "number", ["number", "number"], [atlas.handle, resolved.lease]), 1);
assert.equal(module.ccall("sdf_atlas_destroy", "number", ["number"], [atlas.handle]), 1);

console.log(JSON.stringify({
  contract: contract.font_engine_fingerprint,
  glyphDemand: demand.requests.length,
  masterDataLifecycle: "pass",
  atlasLifecycle: "pass",
}));
