import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

if (typeof globalThis.CustomEvent === "undefined") {
  globalThis.CustomEvent = class CustomEvent extends Event {
    constructor(type, options = {}) {
      super(type);
      this.detail = options.detail;
    }
  };
}

const {
  WorkbenchSession,
  defaultFontFamilies,
  normalizePages,
} = await import("../../demo/workbench/session.js");

test("profile input is the only accepted source of card pages", () => {
  const profile = {
    userCustomProfileCards: [
      { seq: 2, customProfileCardId: 20, customProfileCard: { texts: [] } },
      { seq: 1, customProfileCardId: 10, customProfileCard: { shapes: [] } },
    ],
  };

  assert.deepEqual(normalizePages(profile).map((page) => page.sequence), [1, 2]);
  assert.throws(() => normalizePages({ texts: [] }), /Profile JSON must contain userCustomProfileCards/);
});

test("production font files automatically register their logical aliases", () => {
  assert.deepEqual(defaultFontFamilies("FOT-RodinNTLGPro-DB.ttf"), [
    "FOT-RodinNTLGPro-DB",
    "FZLanTingHei-DB-GBK",
  ]);
  assert.deepEqual(defaultFontFamilies("FOT-SkipProN-B.otf"), [
    "FOT-SkipProN-B",
    "FZZhengHei-EB-GBK",
  ]);
  assert.deepEqual(defaultFontFamilies("FOT-PopHappinessStd-EB.otf"), [
    "FOT-PopHappinessStd-EB",
    "FZShaoEr-M11-JF",
  ]);
});

test("profile and font inputs restore from the local input store", async () => {
  const profileFile = {
    name: "profile.json",
    size: 20,
    async text() { return '{"userCustomProfileCards":[]}'; },
  };
  const fontFile = {
    name: "FOT-PopHappinessStd-EB.otf",
    size: 3,
    async arrayBuffer() { return new Uint8Array([1, 2, 3]).buffer; },
  };
  const store = {
    async restore() {
      return {
        profileFile,
        fonts: [{ file: fontFile, bytes: await fontFile.arrayBuffer() }],
      };
    },
    async saveProfile() {},
    async saveFonts() {},
    async clear() {},
  };
  const session = new WorkbenchSession({}, store);

  await session.restoreInputs();

  assert.equal(session.profileFile, profileFile);
  assert.equal(session.fonts.length, 1);
  assert.deepEqual(session.fonts[0].families, [
    "FOT-PopHappinessStd-EB",
    "FZShaoEr-M11-JF",
  ]);
});

test("page switching releases the inactive scene before allocating the next one", async () => {
  const calls = [];
  const oldScene = {
    async destroy() { calls.push("destroy-old"); },
  };
  const nextScene = {
    async dump() { return { layers: [] }; },
    draw() { calls.push("draw-next"); },
  };
  const session = new WorkbenchSession({});
  session.config = { sdfBackend: "edt", persistence: "origin" };
  session.masterData = {};
  session.renderer = {
    async createProfileScene() {
      calls.push("create-next");
      return nextScene;
    },
  };
  session.pages = [
    { documentKey: "old", card: {}, profile: undefined, scene: oldScene, dump: null },
    { documentKey: "next", card: {}, profile: undefined, scene: null, dump: null },
  ];
  session.activePage = 0;

  await session.switchPage(1);

  assert.deepEqual(calls, ["destroy-old", "create-next", "draw-next"]);
  assert.equal(session.pages[0].scene, null);
  assert.equal(session.activePage, 1);
});

test("animation ticks update inspector data without rebuilding the layer tree", async () => {
  const [demo, inspector] = await Promise.all([
    readFile(new URL("../../demo/demo.js", import.meta.url), "utf8"),
    readFile(new URL("../../demo/workbench/inspector.js", import.meta.url), "utf8"),
  ]);
  assert.match(demo, /reason === "tick"[\s\S]*inspector\.updateRuntimeDump\(detail\.dump\)/);
  const runtimeUpdate = inspector.match(/updateRuntimeDump\(dump\) \{[\s\S]*?\n  \}/)?.[0] ?? "";
  assert.match(runtimeUpdate, /this\.dump = dump/);
  assert.doesNotMatch(runtimeUpdate, /renderLayers|renderDetail|renderControls|renderInteractions|renderDump/);
});

test("workbench exposes readback and control hooks for quantitative runtime verification", async () => {
  const source = await readFile(new URL("../../demo/demo.js", import.meta.url), "utf8");
  assert.match(source, /__ALLIUM_RENDERER_WORKBENCH__/);
  assert.match(source, /dump: \(\) => session\.dump/);
  assert.match(source, /draw: \(\) => session\.draw/);
  assert.match(source, /tab: \(controlId, value\) => session\.setTab/);
  assert.match(source, /scroll: \(controlId, offset\) => session\.setScrollOffset/);
  assert.match(source, /scrollBy: \(controlId, delta\) => session\.scrollBy/);
});
