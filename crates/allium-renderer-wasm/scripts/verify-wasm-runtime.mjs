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

const authoring = callJson("sdf_renderer_authoring_create_blank_json", [], []);
assert.ok(Number.isInteger(authoring.handle) && authoring.handle > 0);
assert.equal(authoring.document.userCustomProfileCards.length, 1);
const authored = callJsonInput("sdf_renderer_authoring_apply_json", {
  kind: "create",
  page: 0,
  category: "texts",
  element: {
    objectData: {
      position: { x: 0, y: 0, z: 0 },
      scale: { x: 1, y: 1, z: 1 },
      rotation: { x: 0, y: 0, z: 0, w: 1 },
      layer: 0,
      lock: false,
      visible: true,
    },
    text: "请多关照!",
    fontId: 1,
    type: 513,
    colorId: 1,
    size: 24,
    outlineColorId: 1,
    outlineSize: 0,
    lineSpacing: 0,
  },
}, [authoring.handle]);
assert.equal(authored.revision, 1);
assert.equal(authored.selectedId, authored.changes[0].id);
assert.equal(authored.selected.id, authored.changes[0].id);
assert.equal(authored.selected.page, 0);
assert.equal(authored.selected.category, "texts");
assert.equal(authored.selected.index, 0);
assert.equal(authored.selected.element.text, "请多关照!");
const gestureStarted = callJsonInput("sdf_renderer_authoring_begin_gesture_json", {
  id: authored.selectedId,
}, [authoring.handle]);
assert.equal(gestureStarted.revision, 1);
const gesturePreview = callJsonInput("sdf_renderer_authoring_preview_gesture_json", {
  kind: "set_parameters",
  id: authored.selectedId,
  values: { size: 48 },
}, [authoring.handle]);
assert.equal(gesturePreview.revision, 1);
assert.equal(gesturePreview.selected.element.size, 48);
const gestureCommitted = callJson("sdf_renderer_authoring_commit_gesture_json", ["number"], [authoring.handle]);
assert.equal(gestureCommitted.revision, 2);
const authoredExport = callJson("sdf_renderer_authoring_export_json", ["number"], [authoring.handle]);
assert.equal(authoredExport.userCustomProfileCards[0].customProfileCard.texts.length, 1);
assert.equal(authoredExport.userCustomProfileCards[0].customProfileCard.texts[0].size, 48);
const undone = callJson("sdf_renderer_authoring_undo_json", ["number"], [authoring.handle]);
assert.equal(undone.revision, 3);
assert.equal(undone.selected.element.size, 24);
assert.equal(module.ccall("sdf_renderer_authoring_destroy", "number", ["number"], [authoring.handle]), 1);

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
  authoringLifecycle: "pass",
  atlasLifecycle: "pass",
}));
