import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { stripTypeScriptTypes } from "node:module";
import test from "node:test";
import vm from "node:vm";

import { SemanticCommandPlan } from "../../src/gpu/semanticCommandPlanner.ts";
import { compileSemanticDrawBatches, SEMANTIC_FLOATS_PER_VERTEX } from "../../src/gpu/semanticCommandGeometry.ts";
import { WebglSemanticCommandExecutor } from "../../dist/gpu/webglSemanticCommandExecutor.js";

const IDENTITY = [1, 0, 0, 1, 0, 0];
// A text node turned a quarter turn clockwise and pivoted at (10, -30): node
// +x runs down the layer, node +y to the right.
const TURNED = [0, 1, 1, 0, 10, 30];
const WHITE = [1, 1, 1, 1];
const GREY = [0.30980393, 0.30980393, 0.30980393, 1];

function uguiCommand(id, { color = WHITE, nodeMatrix = TURNED, blend = "src_over" } = {}) {
  return {
    id,
    layer_id: "slip",
    role: id,
    bounds: { x: 0, y: 0, width: 10, height: 10 },
    matrix: IDENTITY,
    blend_mode: blend,
    clip: null,
    payload: { kind: "ugui_text", text: "AO", font: { family: "UguiFixture" }, color, node_matrix: nodeMatrix },
  };
}

function imageCommand(id) {
  return {
    id,
    layer_id: "slip",
    role: id,
    bounds: { x: -740, y: -245, width: 1480, height: 490 },
    matrix: IDENTITY,
    payload: { kind: "image", resource: { namespace: "assets", key: id }, uv: { x: 0, y: 0, width: 1, height: 1 }, tint: [1, 1, 1, 1] },
  };
}

function plan(commands, layerMatrix = [1, 0, 0, 1, 915, 406]) {
  return new SemanticCommandPlan({
    layerTableRevision: 1,
    layerTable: [{ layer_id: "slip", parent_id: null, slot: 0, subtree_start: 0, subtree_end: 1 }],
    layerSources: [{ layer_id: "slip", matrix: layerMatrix }],
    layerCommands: [{ layer_id: "slip", render_mask: true, transform: { dx: 0, dy: 0 }, command_start: 0, command_count: commands.length }],
    semanticCommands: commands,
  });
}

const CELL = [[-1, 21], [23, 21], [23, -2], [-1, -2]];

function quad(glyph, dx = 0) {
  return { corners: CELL.map(([x, y]) => [x + dx, y]), glyph };
}

// Two glyphs on page 0 and one on page 1.
function layout() {
  const glyph = (page, x, width, rows) => ({ page, x, y: 3, width, rows, bitmapLeft: 0, bitmapTop: rows, advance26d6: 64 * width });
  return {
    version: 1,
    texts: [
      {
        id: "title",
        preferredWidth: 40,
        cellPadding: 1,
        backdrop: { rect: { x: -6, y: -210.2, width: 50, height: 74 }, color: [0, 0.8, 0.73333335, 1] },
        quads: [quad(0), quad(1, 24)],
      },
      { id: "summary", preferredWidth: 20, cellPadding: 1, backdrop: null, quads: [quad(1), quad(2, 24), quad(0, 48)] },
      { id: "description", preferredWidth: 20, cellPadding: 1, backdrop: null, quads: [quad(0)] },
    ],
    glyphs: [glyph(0, 0, 22, 21), glyph(0, 22, 20, 19), glyph(1, 0, 22, 21)],
    pages: [
      { width: 1024, height: 24, pixels: new Uint8Array(1024 * 24) },
      { width: 1024, height: 24, pixels: new Uint8Array(1024 * 24) },
    ],
  };
}

function vertex(batch, index) {
  return Array.from(batch.vertices.slice(index * SEMANTIC_FLOATS_PER_VERTEX, (index + 1) * SEMANTIC_FLOATS_PER_VERTEX));
}

test("a uGUI text draws its backdrop as a shape, then its glyph runs page by page", () => {
  const commands = [imageCommand("cover"), uguiCommand("title"), uguiCommand("summary", { color: GREY }), uguiCommand("description", { color: GREY })];
  const batches = compileSemanticDrawBatches(plan(commands).operations(), layout());
  assert.deepEqual(batches.map((batch) => [batch.kind, batch.glyphPage, batch.commandIds]), [
    ["image", null, ["cover"]],
    ["shape", null, ["title"]],
    // The title's glyphs and the summary's first glyph share page 0.
    ["ugui_glyph", 0, ["title", "summary"]],
    // The summary's second glyph sits on page 1: its run breaks the batch.
    ["ugui_glyph", 1, ["summary"]],
    // Its third glyph and the description are back on page 0.
    ["ugui_glyph", 0, ["summary", "description"]],
  ]);
  assert.deepEqual(batches.map((batch) => batch.vertices.length / SEMANTIC_FLOATS_PER_VERTEX), [6, 6, 18, 6, 12]);
  // The backdrop is the fitted rectangle in layer space, filled solid.
  const backdrop = vertex(batches[1], 0);
  assert.deepEqual(backdrop.slice(0, 2), [915 - 6, Math.fround(406 - 210.2)]);
  assert.deepEqual(backdrop.slice(6, 10), [0, Math.fround(0.8), Math.fround(0.73333335), 1]);
  assert.deepEqual(backdrop.slice(14, 18), [0, 0, 0, 0]);
  assert.deepEqual(backdrop.slice(26, 28), [50, 74]);
});

test("glyph vertices map the padded cell onto the turned quad through the node matrix", () => {
  const batches = compileSemanticDrawBatches(
    plan([uguiCommand("title")]).operations(),
    { ...layout(), texts: [{ ...layout().texts[0], backdrop: null }] },
  );
  // Without a backdrop the text is its glyphs alone.
  assert.equal(batches.length, 1);
  const [glyphs] = batches;
  assert.equal(glyphs.kind, "ugui_glyph");
  // Unit corners (0,0) (1,0) (1,1) (0,0) (1,1) (0,1) are quad corners
  // 0 1 2 0 2 3; the node matrix sends node (x, y) to layer (y + 10, x + 30).
  const expected = [[0, 0, 0], [1, 1, 0], [2, 1, 1], [0, 0, 0], [2, 1, 1], [3, 0, 1]];
  for (const [index, [corner, unitX, unitY]] of expected.entries()) {
    const [nodeX, nodeY] = CELL[corner];
    const values = vertex(glyphs, index);
    assert.deepEqual(values.slice(0, 2), [915 + nodeY + 10, 406 + nodeX + 30], `vertex ${index}`);
    // Cell texels: 22 + 2 x 21 + 2.
    assert.deepEqual(values.slice(2, 4), [unitX * 24, unitY * 23], `vertex ${index}`);
    assert.deepEqual(values.slice(4, 6), [unitX, unitY]);
    // Vertex colour, the bitmap's page rectangle and the padding.
    assert.deepEqual(values.slice(6, 10), WHITE);
    assert.deepEqual(values.slice(10, 14), [0, 3, 22, 21]);
    assert.deepEqual(values.slice(14, 18), [1, 0, 0, 0]);
    assert.deepEqual(values.slice(26, 28), [24, 23]);
    // No clip: the clip quad covers everything.
    assert.deepEqual(values.slice(18, 20), [-1e9, -1e9]);
  }
  assert.deepEqual(Array.from(glyphs.layerSlots), new Array(12).fill(0));
  assert.deepEqual(Array.from(glyphs.commandSlots), new Array(12).fill(0));
  // The second glyph's rectangle.
  assert.deepEqual(vertex(glyphs, 6).slice(10, 14), [22, 3, 20, 19]);
});

test("a uGUI text needs its layout, and glyph batches never merge across blend modes", () => {
  assert.throws(
    () => compileSemanticDrawBatches(plan([uguiCommand("title")]).operations()),
    /missing uGUI text layout title/,
  );
  const batches = compileSemanticDrawBatches(
    plan([uguiCommand("description"), uguiCommand("summary", { blend: "add" })]).operations(),
    { ...layout(), texts: layout().texts.slice(1).reverse() },
  );
  assert.deepEqual(batches.map((batch) => [batch.kind, batch.blendMode, batch.glyphPage]), [
    ["ugui_glyph", "src_over", 0],
    ["ugui_glyph", "add", 0],
    ["ugui_glyph", "add", 1],
    ["ugui_glyph", "add", 0],
  ]);
  // The commands request no resources of their own.
  assert.deepEqual(plan([uguiCommand("title")]).resourceRequests(), []);
});

// The worker, evaluated as the classic script it ships as, over a module
// stand-in that answers the scene and uGUI layout exports.
const readSource = async (name) => stripTypeScriptTypes(
  await readFile(new URL(`../../src/${name}`, import.meta.url), "utf8"),
  { mode: "transform" },
);
const { RENDERER_WORKER_PROTOCOL } = await import(
  `data:text/javascript;base64,${Buffer.from(await readSource("protocol.ts")).toString("base64")}`
);
const workerSource = (await readSource("worker.ts")).replace(
  /import\s*\{\s*RENDERER_WORKER_PROTOCOL\s*\}\s*from\s*["']\.\/protocol\.js["'];?/,
  `const RENDERER_WORKER_PROTOCOL = ${JSON.stringify(RENDERER_WORKER_PROTOCOL)};`,
);

const FALLBACK_FACES = [
  { family: "Roboto", face_index: 0 },
  { family: "Noto Sans CJK SC", face_index: 2 },
];
const SCENE_COMMANDS = [
  { id: "cover", payload: { kind: "image" } },
  { id: "a", payload: { kind: "ugui_text", text: "AO", font: { family: "FOT-Omikuji" }, fallback_faces: FALLBACK_FACES } },
  { id: "b", payload: { kind: "ugui_text", text: "O", font: { family: "FOT-UDMinchoPro-B" }, fallback_faces: FALLBACK_FACES } },
  { id: "c", payload: { kind: "ugui_text", text: "A", font: { family: "FOT-Omikuji" }, fallback_faces: [] } },
];
// A file for every family of the scene's face chains.
const FONT_FILES = [
  ["FOT-Omikuji", [1, 2, 3]],
  ["FOT-UDMinchoPro-B", [4, 5]],
  ["Roboto", [6]],
  ["Noto Sans CJK SC", [7, 8]],
];

function uguiWorker({ commands = SCENE_COMMANDS, layout } = {}) {
  const heap = new Uint8Array(1 << 16);
  let allocation = 64;
  const layoutCalls = [];
  const destroyed = [];
  const text = (pointer, length) => new TextDecoder().decode(heap.subarray(pointer, pointer + length));
  const module = {
    HEAPU8: heap,
    _malloc(bytes) {
      const pointer = allocation;
      allocation = (allocation + bytes + 3) & ~3;
      return pointer;
    },
    _free() {},
    ccall(name, _result, _types, args) {
      if (name === "sdf_layout_freetype_free_string") return;
      if (name === "sdf_renderer_core_scene_destroy") {
        destroyed.push(args[0]);
        return 1;
      }
      let value;
      if (name === "sdf_renderer_core_masterdata_create_json") {
        value = { handle: 3 };
      } else if (name === "sdf_layout_freetype_build_layout_json") {
        value = { dynamicPrograms: [] };
      } else if (name === "sdf_renderer_core_profile_create_json") {
        value = { handle: 5, snapshot: { schema_major: 1, scene_id: "collections", semantic_commands: commands } };
      } else if (name === "sdf_layout_ugui_text_json") {
        const [fontPointer, fontLength, inputPointer, inputLength] = args;
        const call = {
          fonts: Array.from(heap.subarray(fontPointer, fontPointer + fontLength)),
          input: JSON.parse(text(inputPointer, inputLength)),
        };
        layoutCalls.push(call);
        value = layout ? layout(call) : { version: 1, texts: [], glyphs: [], pages: [] };
      } else {
        throw new Error(`unexpected WASM call ${name}`);
      }
      const bytes = new TextEncoder().encode(`${JSON.stringify(value)}\0`);
      const pointer = this._malloc(bytes.length);
      heap.set(bytes, pointer);
      return pointer;
    },
  };
  const pending = new Map();
  let sequence = 0;
  const context = vm.createContext({
    console, performance, TextEncoder, TextDecoder, Uint8Array, ArrayBuffer, Error, atob, btoa,
    fixtureModule: module,
    postMessage(response, transfers) {
      const resolve = pending.get(response.id);
      pending.delete(response.id);
      resolve({ response, transfers });
    },
  });
  vm.runInContext(`${workerSource}\nmoduleInstance = fixtureModule; modulePromise = Promise.resolve(fixtureModule);`, context);
  const send = (kind, payload) => new Promise((resolve) => {
    const id = ++sequence;
    pending.set(id, resolve);
    context.onmessage({ data: { id, kind, payload } });
  }).then(({ response, transfers }) => {
    // The worker's objects belong to its own realm: compare its responses as
    // plain data, keeping the coverage pages' bytes.
    const pages = response.result?.uguiText?.pages ?? [];
    const plain = JSON.parse(JSON.stringify(response));
    if (plain.result?.uguiText) {
      plain.result.uguiText.pages = Array.from(pages, (page) => ({ width: page.width, height: page.height, pixels: page.pixels }));
    }
    return { ...plain, transfers };
  });
  const registerFont = (family, bytes) => send("registerFont", {
    region: "cn",
    family,
    sourceHash: "0".repeat(64),
    bytes: Uint8Array.from(bytes).buffer,
  });
  const createScene = async () => {
    const created = await send("createMasterData", { region: "cn", revision: "r1" });
    assert.equal(created.ok, true);
    return send("createProfileScene", {
      masterDataId: created.result.masterDataId,
      layoutRequest: { layers: [] },
      request: { documentKey: "collections" },
    });
  };
  return { send, registerFont, createScene, layoutCalls, destroyed };
}

test("the worker lays out every uGUI command with each family's registered font once", async () => {
  const pixels = [0, 128, 255, 7, 9, 11];
  const worker = uguiWorker({
    layout: ({ input }) => ({
      version: 1,
      texts: input.texts.map(({ id }) => ({ id, preferredWidth: 1, cellPadding: 1, backdrop: null, quads: [] })),
      glyphs: [],
      pages: [{ width: 3, height: 2, pixelsBase64: Buffer.from(pixels).toString("base64") }],
    }),
  });
  for (const [family, bytes] of FONT_FILES) await worker.registerFont(family, bytes);
  const response = await worker.createScene();
  assert.equal(response.ok, true, JSON.stringify(response.error));
  assert.equal(worker.layoutCalls.length, 1);
  const [{ fonts, input }] = worker.layoutCalls;
  // Each family's file once, in the order the face chains first name them;
  // the face indices stay in the texts' sources.
  assert.deepEqual(fonts, [1, 2, 3, 6, 7, 8, 4, 5]);
  assert.deepEqual(input.fonts, [
    { family: "FOT-Omikuji", offset: 0, length: 3 },
    { family: "Roboto", offset: 3, length: 1 },
    { family: "Noto Sans CJK SC", offset: 4, length: 2 },
    { family: "FOT-UDMinchoPro-B", offset: 6, length: 2 },
  ]);
  // Every uGUI command in command order, its payload as the source.
  assert.deepEqual(input.texts, SCENE_COMMANDS.slice(1).map(({ id, payload }) => ({ id, source: payload })));
  // The coverage pages reach the main thread decoded and transferred.
  const { uguiText } = response.result;
  assert.deepEqual(uguiText.texts.map(({ id }) => id), ["a", "b", "c"]);
  assert.deepEqual(Array.from(uguiText.pages[0].pixels), pixels);
  assert.deepEqual([uguiText.pages[0].width, uguiText.pages[0].height], [3, 2]);
  assert.equal(response.transfers.length, 1);
  assert.equal(response.transfers[0], uguiText.pages[0].pixels.buffer);
  assert.equal(response.result.sceneId, "collections:5");
  assert.deepEqual(worker.destroyed, []);
});

test("a scene without uGUI text needs no font files and no layout call", async () => {
  const worker = uguiWorker({ commands: SCENE_COMMANDS.slice(0, 1) });
  const response = await worker.createScene();
  assert.equal(response.ok, true, JSON.stringify(response.error));
  assert.deepEqual(response.result.uguiText, { version: 1, texts: [], glyphs: [], pages: [] });
  assert.equal(worker.layoutCalls.length, 0);
});

test("a uGUI font without a registered file fails the scene and releases it", async () => {
  const worker = uguiWorker();
  await worker.registerFont("FOT-Omikuji", [1, 2, 3]);
  // A prebuilt atlas registration carries no font file.
  await worker.send("registerPrebuiltFont", { region: "cn", family: "FOT-UDMinchoPro-B", sourceHash: "1".repeat(64) });
  await worker.registerFont("Roboto", [6]);
  await worker.registerFont("Noto Sans CJK SC", [7, 8]);
  const response = await worker.createScene();
  assert.equal(response.ok, false);
  assert.deepEqual(response.error, {
    code: "FONT_NOT_REGISTERED",
    message: "Required font is not registered: FOT-UDMinchoPro-B",
  });
  assert.equal(worker.layoutCalls.length, 0);
  assert.deepEqual(worker.destroyed, [5]);
});

test("a fallback family without a registered file fails the scene like the text's own font", async () => {
  const worker = uguiWorker();
  await worker.registerFont("FOT-Omikuji", [1, 2, 3]);
  await worker.registerFont("FOT-UDMinchoPro-B", [4, 5]);
  await worker.registerFont("Roboto", [6]);
  const response = await worker.createScene();
  assert.equal(response.ok, false);
  assert.deepEqual(response.error, {
    code: "FONT_NOT_REGISTERED",
    message: "Required font is not registered: Noto Sans CJK SC",
  });
  assert.equal(worker.layoutCalls.length, 0);
  assert.deepEqual(worker.destroyed, [5]);
});

const WRONG_SIZE_PAGE = { width: 4, height: 2, pixelsBase64: Buffer.from([1, 2, 3, 4, 5, 6]).toString("base64") };
for (const [name, layout, message] of [
  ["a layout error", () => ({ error: "a: font FOT-Omikuji is not registered" }), "a: font FOT-Omikuji is not registered"],
  ["an unsupported layout", () => ({ version: 2, texts: [], glyphs: [], pages: [] }), "uGUI text layout has an unsupported shape"],
  ["a page of the wrong size", () => ({ version: 1, texts: [], glyphs: [], pages: [WRONG_SIZE_PAGE] }), "uGUI glyph page 0 holds 6 bytes for 4x2"],
]) {
  test(`${name} fails the scene and releases it`, async () => {
    const worker = uguiWorker({ layout });
    for (const [family, bytes] of FONT_FILES) await worker.registerFont(family, bytes);
    const response = await worker.createScene();
    assert.equal(response.ok, false);
    assert.deepEqual(response.error, { code: "UGUI_TEXT_LAYOUT_FAILED", message });
    assert.deepEqual(worker.destroyed, [5]);
  });
}

test("a uGUI command without an id or a font family fails the scene", async () => {
  const font = { family: "FOT-Omikuji" };
  for (const [commands, message] of [
    [[{ payload: { kind: "ugui_text", font, fallback_faces: [] } }], "uGUI text command has no id"],
    [[{ id: "x", payload: { kind: "ugui_text", fallback_faces: [] } }], "uGUI text x names no font family"],
    [[{ id: "x", payload: { kind: "ugui_text", font } }], "uGUI text x has no fallback faces"],
    [[{ id: "x", payload: { kind: "ugui_text", font, fallback_faces: [{ face_index: 2 }] } }], "uGUI text x fallback face 0 names no font family"],
  ]) {
    const worker = uguiWorker({ commands });
    const response = await worker.createScene();
    assert.deepEqual(response.error, { code: "UGUI_TEXT_LAYOUT_FAILED", message });
    assert.deepEqual(worker.destroyed, [5]);
  }
});

// A WebGL2 context stand-in that records calls and tracks live objects.
function recordingContext() {
  const calls = [];
  const live = new Set();
  let next = 0;
  const create = (kind) => () => {
    const object = { kind, id: next += 1 };
    live.add(object);
    return object;
  };
  const release = (object) => { live.delete(object); };
  const methods = {
    createShader: create("shader"),
    createProgram: create("program"),
    createTexture: create("texture"),
    createVertexArray: create("vertex array"),
    createBuffer: create("buffer"),
    createFramebuffer: create("framebuffer"),
    deleteShader: release,
    deleteProgram: release,
    deleteTexture: release,
    deleteVertexArray: release,
    deleteBuffer: release,
    deleteFramebuffer: release,
    getShaderParameter: () => true,
    getProgramParameter: () => true,
    getParameter: (name) => name === "VIEWPORT" ? new Int32Array([0, 0, 1830, 812]) : null,
    isContextLost: () => false,
    checkFramebufferStatus: () => "FRAMEBUFFER_COMPLETE",
  };
  const gl = new Proxy(methods, {
    get(target, property) {
      if (property in target) {
        const method = target[property];
        return (...args) => {
          calls.push([property, ...args]);
          return method(...args);
        };
      }
      if (typeof property === "string" && /^[A-Z0-9_]+$/.test(property)) return property;
      return (...args) => {
        calls.push([property, ...args]);
        return property === "getUniformLocation" ? { uniform: args[1] } : undefined;
      };
    },
  });
  return { gl, calls, live };
}

test("the executor uploads R8 coverage pages and draws glyph batches with the glyph program", async () => {
  const source = await readFile(new URL("../../src/gpu/webglSemanticCommandExecutor.ts", import.meta.url), "utf8");
  // The fifth program is the glyph program, sharing the semantic vertex stage.
  assert.match(source, /\[this\.shapeProgram, this\.textureProgram, this\.compositeProgram, this\.badgeProgram, this\.uguiGlyphProgram\] = programs;/);
  const { gl, calls, live } = recordingContext();
  const executor = new WebglSemanticCommandExecutor(gl);
  const glyphProgram = [...live].filter((object) => object.kind === "program")[4];
  const semantic = plan([uguiCommand("title"), uguiCommand("summary", { color: GREY })]);
  const uguiText = layout();
  const batches = compileSemanticDrawBatches(semantic.operations(), uguiText);
  executor.setScene(semantic, batches, new Map());
  calls.length = 0;
  assert.deepEqual(executor.setUguiGlyphPages(uguiText.pages), { bytes: 2 * 1024 * 24 });
  const uploads = calls.filter(([name]) => name === "texImage2D");
  assert.deepEqual(uploads.map((call) => call.slice(1, 9)), [
    ["TEXTURE_2D", 0, "R8", 1024, 24, 0, "RED", "UNSIGNED_BYTE"],
    ["TEXTURE_2D", 0, "R8", 1024, 24, 0, "RED", "UNSIGNED_BYTE"],
  ]);
  assert.equal(uploads[0][9], uguiText.pages[0].pixels);
  assert.ok(calls.some(([name, parameter, value]) => name === "pixelStorei" && parameter === "UNPACK_ALIGNMENT" && value === 1));
  assert.equal(calls.filter(([name]) => name === "createTexture").length, 2);

  calls.length = 0;
  const metrics = executor.draw();
  // The title's backdrop and three glyph batches.
  assert.equal(batches.length, 4);
  assert.equal(metrics.drawCalls, batches.length);
  const glyphDraws = [];
  let program = null;
  let unitTwo = null;
  let activeUnit = null;
  for (const [name, ...args] of calls) {
    if (name === "useProgram") program = args[0];
    if (name === "activeTexture") activeUnit = args[0];
    if (name === "bindTexture" && activeUnit === "TEXTURE2") unitTwo = args[1];
    if (name === "drawArrays" && program === glyphProgram) glyphDraws.push([args[2], unitTwo?.id]);
  }
  assert.equal(glyphDraws.length, batches.filter((batch) => batch.kind === "ugui_glyph").length);
  assert.deepEqual(glyphDraws.map(([vertices]) => vertices), batches.filter((batch) => batch.kind === "ugui_glyph").map((batch) => batch.layerSlots.length));
  // Pages 0, 1 and 0: each batch samples its own page.
  const pageIds = glyphDraws.map(([, id]) => id);
  assert.equal(pageIds.length, 3);
  assert.notEqual(pageIds[1], pageIds[0]);
  assert.equal(pageIds[2], pageIds[0]);
  assert.ok(calls.some(([name, location, unit]) => name === "uniform1i" && location?.uniform === "u_glyphs" && unit === 2));
  // Premultiplied source-over.
  assert.ok(calls.some(([name, ...args]) => name === "blendFuncSeparate" && args.join() === "ONE,ONE_MINUS_SRC_ALPHA,ONE,ONE_MINUS_SRC_ALPHA"));

  executor.destroy();
  assert.deepEqual([...live].map((object) => object.kind), []);
});

test("a glyph batch without its uploaded page fails the draw", () => {
  const { gl } = recordingContext();
  const executor = new WebglSemanticCommandExecutor(gl);
  const semantic = plan([uguiCommand("title")]);
  executor.setScene(semantic, compileSemanticDrawBatches(semantic.operations(), layout()), new Map());
  assert.throws(() => executor.draw(), /uGUI glyph page 0 is not uploaded/);
  executor.destroy();
});
