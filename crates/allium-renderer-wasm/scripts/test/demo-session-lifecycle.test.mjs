import assert from "node:assert/strict";
import test from "node:test";

if (typeof globalThis.CustomEvent === "undefined") {
  globalThis.CustomEvent = class CustomEvent extends Event {
    constructor(type, options = {}) {
      super(type);
      this.detail = options.detail;
    }
  };
}

const { WorkbenchSession } = await import("../../demo/workbench/session.js");

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
