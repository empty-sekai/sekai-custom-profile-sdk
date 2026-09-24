import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
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
// White space is demanded too: an atlas that carries it supplies its advance.
assert.deepEqual(demand.requests.map((request) => request.char), ["A", "ß", " ", "1", "2", "□"]);

const masterData = callJsonInput("sdf_renderer_core_masterdata_create_json", {
  region: "en",
  revision: "smoke",
});
assert.ok(Number.isInteger(masterData.handle) && masterData.handle > 0);
const stats = callJson("sdf_renderer_core_masterdata_stats_json", ["number"], [masterData.handle]);
assert.equal(stats.region, "en");
assert.equal(module.ccall("sdf_renderer_core_masterdata_destroy", "number", ["number"], [masterData.handle]), 1);

const iconMasterData = callJsonInput("sdf_renderer_core_masterdata_create_json", {
  region: "jp",
  revision: "icons",
});
assert.deepEqual(iconMasterData.optional_tables, [
  "customProfileCharacterIconResources",
  "customProfileMaterialResources",
  "customProfileUserInterfaceIconResources",
  "omikujis",
]);
for (const [name, table] of [
  ["customProfileTextColors", [{ id: 1, colorCode: "#ff8000" }]],
  ["customProfileUserInterfaceIconResources", [{
    id: 1,
    customProfileResourceType: "user_interface_icon",
    resourceLoadVal: "custom_profile/user_interface_icon",
    fileName: "profile_icon_0001",
  }]],
]) {
  callJsonInput("sdf_renderer_core_masterdata_put_table_json", { name, table }, [iconMasterData.handle]);
}
callJson("sdf_renderer_core_masterdata_seal_json", ["number"], [iconMasterData.handle]);
const iconObject = {
  position: { x: 0, y: 0, z: 0 },
  scale: { x: 1, y: 1, z: 1 },
  rotation: { x: 0, y: 0, z: 0, w: 1 },
  layer: 1,
  lock: false,
  visible: true,
};
const iconPreparation = callJsonInput("sdf_renderer_core_profile_prepare_json", {
  documentKey: "icons",
  card: {
    userInterfaceIcons: [{ objectData: iconObject, id: 1, colorId: 1, alpha: 0.5 }],
    materials: [{ objectData: { ...iconObject, layer: 2 }, id: 1 }],
  },
}, [iconMasterData.handle]);
assert.deepEqual(
  iconPreparation.resources.map((request) => request.resource.key),
  ["custom_profile/user_interface_icon/profile_icon_0001"],
);
assert.equal(module.ccall("sdf_renderer_core_masterdata_destroy", "number", ["number"], [iconMasterData.handle]), 1);

const collectionMasterData = callJsonInput("sdf_renderer_core_masterdata_create_json", {
  region: "cn",
  revision: "collections",
});
callJsonInput("sdf_renderer_core_masterdata_put_table_json", {
  name: "customProfileCollectionResources",
  table: [
    { id: 1, customProfileResourceType: "collection", customProfileResourceCollectionType: "can_badge", resourceLoadVal: "custom_profile/collection/crash", fileName: "crash_fixture_canbadge" },
    { id: 2, customProfileResourceType: "collection", customProfileResourceCollectionType: "omikuji", resourceLoadVal: "lottery_game/new_year_2022", fileName: "Prefabs/Omikuji" },
  ],
}, [collectionMasterData.handle]);
callJsonInput("sdf_renderer_core_masterdata_put_table_json", {
  name: "omikujis",
  table: [{
    id: 7, unit: "idol", summary: "s",
    title1: "t1", description1: "d1", title2: "t2", description2: "d2", title3: "t3", description3: "d3",
    fortuneAssetbundleName: "lottery_game/new_year_2022_material", fortuneFilePath: "unsei_daikichi",
    omikujiCoverAssetbundleName: "lottery_game/new_year_2022_material", omikujiCoverFilePath: "omikuji_idol",
  }],
}, [collectionMasterData.handle]);
callJson("sdf_renderer_core_masterdata_seal_json", ["number"], [collectionMasterData.handle]);
const collectionPreparation = callJsonInput("sdf_renderer_core_profile_prepare_json", {
  documentKey: "collections",
  card: {
    collections: [
      { objectData: iconObject, id: 1, targetId: null },
      { objectData: { ...iconObject, layer: 2 }, id: 2, targetId: 7 },
    ],
  },
}, [collectionMasterData.handle]);
// A can badge requests its image and the static normal map; the omikuji its
// cover and fortune images, and its slip font as a uGUI font.
assert.deepEqual(
  collectionPreparation.resources.map((request) => `${request.resource.namespace}/${request.resource.key}`).sort(),
  [
    "assets/custom_profile/collection/crash/crash_fixture_canbadge",
    "assets/lottery_game/new_year_2022_material/bg_omikuji_idol",
    "assets/lottery_game/new_year_2022_material/unsei_daikichi",
    "static/ui/sekai_badge_normal",
  ],
);
// The slip font and the CN client's fallback faces.
assert.deepEqual(collectionPreparation.ugui_font_families, ["FOT-Omikuji", "Noto Sans CJK SC", "Roboto"]);
assert.deepEqual(collectionPreparation.font_families, ["FOT-Omikuji", "Noto Sans CJK SC", "Roboto"]);
const collectionScene = callJsonInput("sdf_renderer_core_profile_create_json", {
  documentKey: "collections",
  card: {
    collections: [
      { objectData: iconObject, id: 1, targetId: null },
      { objectData: { ...iconObject, layer: 2 }, id: 2, targetId: 7 },
    ],
  },
}, [collectionMasterData.handle]);
const uguiTexts = collectionScene.snapshot.semantic_commands.filter((command) => command.payload.kind === "ugui_text");
assert.deepEqual(uguiTexts.map((command) => command.role), [
  "omikuji-title", "omikuji-title", "omikuji-title", "omikuji-summary",
  "omikuji-description", "omikuji-description", "omikuji-description",
]);
assert.ok(uguiTexts.every((command) => command.payload.font.family === "FOT-Omikuji"));
assert.ok(uguiTexts.every((command) => JSON.stringify(command.payload.fallback_faces) === JSON.stringify([
  { family: "Roboto", face_index: 0 },
  { family: "Noto Sans CJK SC", face_index: 2 },
])));
assert.equal(module.ccall("sdf_renderer_core_scene_destroy", "number", ["number"], [collectionScene.handle]), 1);
assert.equal(module.ccall("sdf_renderer_core_masterdata_destroy", "number", ["number"], [collectionMasterData.handle]), 1);

// The wasm FreeType lays the synthetic fixture fonts out exactly as the
// native FreeType build does: each fixture records the native layout, with
// each coverage page as its SHA-256. The second one draws characters from
// fallback faces: a TrueType font and faces of a font collection.
const fixtureFile = async (name) => new Uint8Array(await readFile(new URL(`./test/fixtures/${name}`, import.meta.url)));
const uguiLayout = (fontBytes, request) => {
  const fontPointer = module._malloc(Math.max(fontBytes.byteLength, 1));
  try {
    module.HEAPU8.set(fontBytes, fontPointer);
    return callJsonInput("sdf_layout_ugui_text_json", request, [fontPointer, fontBytes.byteLength]);
  } finally {
    module._free(fontPointer);
  }
};
// The error of a layout that fails.
const uguiLayoutError = (fontBytes, request) => {
  const fontPointer = module._malloc(Math.max(fontBytes.byteLength, 1));
  const bytes = encoder.encode(JSON.stringify(request));
  const input = module._malloc(bytes.byteLength);
  try {
    module.HEAPU8.set(fontBytes, fontPointer);
    module.HEAPU8.set(bytes, input);
    const result = module.ccall(
      "sdf_layout_ugui_text_json",
      "number",
      ["number", "number", "number", "number"],
      [fontPointer, fontBytes.byteLength, input, bytes.byteLength],
    );
    try {
      return JSON.parse(readCString(result)).error;
    } finally {
      module.ccall("sdf_layout_freetype_free_string", null, ["number"], [result]);
    }
  } finally {
    module._free(input);
    module._free(fontPointer);
  }
};
let fixture;
let fixtureLayout;
let fixtureTexts = 0;
for (const name of ["ugui-fixture-layout.json", "ugui-fixture-fallback-layout.json"]) {
  fixture = JSON.parse(await readFile(new URL(`./test/fixtures/${name}`, import.meta.url), "utf8"));
  const files = await Promise.all(fixture.request.fonts.map((font) => fixtureFile(font.file)));
  const buffer = new Uint8Array(files.reduce((sum, file) => sum + file.byteLength, 0));
  const fonts = [];
  let offset = 0;
  for (const [index, file] of files.entries()) {
    buffer.set(file, offset);
    fonts.push({ family: fixture.request.fonts[index].family, offset, length: file.byteLength });
    offset += file.byteLength;
  }
  fixtureLayout = uguiLayout(buffer, { fonts, texts: fixture.request.texts });
  for (const page of fixtureLayout.pages) {
    const pixels = Buffer.from(page.pixelsBase64, "base64");
    assert.equal(pixels.byteLength, page.width * page.height);
    delete page.pixelsBase64;
    page.pixelsSha256 = createHash("sha256").update(pixels).digest("hex");
  }
  assert.deepEqual(fixtureLayout, fixture.layout, name);
  fixtureTexts += fixtureLayout.texts.length;
}
// A text whose font is not given fails the layout.
assert.match(
  uguiLayoutError(new Uint8Array(0), { fonts: [], texts: fixture.request.texts.slice(0, 1) }),
  /font UguiFixture is not registered/,
);
// So does a missing fallback font, and a face the collection does not have.
{
  const [primary, latin] = await Promise.all(["ugui-fixture.otf", "ugui-fixture-latin.ttf"].map(fixtureFile));
  const buffer = new Uint8Array(primary.byteLength + latin.byteLength);
  buffer.set(primary, 0);
  buffer.set(latin, primary.byteLength);
  const chain = fixture.request.texts.find((text) => text.id === "chain");
  const missingFallback = uguiLayoutError(buffer, {
    fonts: [
      { family: "UguiFixture", offset: 0, length: primary.byteLength },
      { family: "UguiFixture Latin", offset: primary.byteLength, length: latin.byteLength },
    ],
    texts: [chain],
  });
  assert.match(missingFallback, /chain: font UguiFixture CJK is not registered/);
  const cjk = await fixtureFile("ugui-fixture-cjk.ttc");
  const all = new Uint8Array(buffer.byteLength + cjk.byteLength);
  all.set(buffer, 0);
  all.set(cjk, buffer.byteLength);
  const beyond = structuredClone(chain);
  beyond.source.fallback_faces[1].face_index = 4;
  const missingFace = uguiLayoutError(all, {
    fonts: [
      { family: "UguiFixture", offset: 0, length: primary.byteLength },
      { family: "UguiFixture Latin", offset: primary.byteLength, length: latin.byteLength },
      { family: "UguiFixture CJK", offset: buffer.byteLength, length: cjk.byteLength },
    ],
    texts: [beyond],
  });
  assert.match(missingFace, /chain: glyphs of UguiFixture, UguiFixture Latin, UguiFixture CJK failed: open font face 4 of font 2/);
}

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
  uguiTextLayout: fixtureTexts,
  authoringLifecycle: "pass",
  atlasLifecycle: "pass",
}));
