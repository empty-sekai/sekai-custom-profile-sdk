import assert from "node:assert/strict";
import test from "node:test";

import { buildPrebuiltSdfAtlas, isValidPrebuiltSdfAtlasManifest } from "../../dist/prebuiltSdfAtlas.js";
import { createOriginPrebuiltSdfAtlasPackage } from "../../dist/originPrebuiltSdfAtlasPackage.js";

const FONT_SHA256 = "ab".repeat(32);

async function sha256Hex(bytes) {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  return [...digest].map((value) => value.toString(16).padStart(2, "0")).join("");
}

/** One swizzled R8 page file, as `build-sdf-atlas` writes it. */
function pageBytes(size) {
  const bytes = new Uint8Array(64 + size * size);
  bytes.set(new TextEncoder().encode("ALLIUMSWZ8"), 0);
  const view = new DataView(bytes.buffer);
  view.setUint32(12, 1, true);
  view.setUint32(16, size, true);
  view.setUint32(20, size, true);
  view.setUint32(24, 8, true);
  view.setUint32(28, 8, true);
  bytes.fill(128, 64);
  return bytes;
}

async function atlasPackage(family, { size = 2048, file = "page-000.r8swz.br", pointSize = 75, spread = 6 } = {}) {
  const page = pageBytes(size);
  const manifest = {
    schema: "allium.sdf-atlas-manifest.v1",
    generator_contract: "outline-edt-v2:ss=2:fallback=analytic-v1",
    font_family: family,
    font_sha256: FONT_SHA256,
    point_size: pointSize,
    spread,
    pages: [{ file, width: size, height: size, file_sha256: await sha256Hex(page) }],
    glyphs: [{
      codepoint: 0x41,
      page: 0,
      rect: [1, 1, 4, 4],
      plane_bearing: [1, 50],
      plane_size: [40, 50],
      plane_advance_x: 45,
    }],
  };
  return { manifest, page };
}

function staticProvider(packages) {
  return {
    async manifest(family) {
      return packages.get(family)?.manifest ?? null;
    },
    async page(family, file) {
      const entry = packages.get(family);
      if (!entry || entry.manifest.pages[0].file !== file) throw new Error(`missing page ${family}/${file}`);
      return entry.page.slice().buffer;
    },
  };
}

function request(family) {
  return { region: "cn", family, fontSourceHash: FONT_SHA256, char: "A" };
}

/** Minimal in-memory IndexedDB covering the calls the atlas package makes. */
function installFakeIndexedDb() {
  const databases = new Map();
  class Request {
    constructor() {
      this.onsuccess = null;
      this.onerror = null;
      this.result = undefined;
    }
  }
  class Transaction {
    constructor(state) {
      this.state = state;
      this.pending = 0;
      this.finished = false;
      this.oncomplete = null;
      this.onerror = null;
      this.onabort = null;
      this.error = null;
      this.settle();
    }
    objectStore(name) {
      return new Store(this.state.get(name), this);
    }
    run(compute) {
      const result = new Request();
      this.pending += 1;
      queueMicrotask(() => {
        this.pending -= 1;
        if (this.finished) return;
        result.result = compute();
        result.onsuccess?.();
        this.settle();
      });
      return result;
    }
    settle() {
      setTimeout(() => {
        if (this.finished || this.pending > 0) return;
        this.finished = true;
        this.oncomplete?.();
      }, 0);
    }
    abort() {
      this.finished = true;
      setTimeout(() => this.onabort?.(), 0);
    }
  }
  class Store {
    constructor(store, transaction) {
      this.store = store;
      this.transaction = transaction;
    }
    put(value) {
      return this.transaction.run(() => {
        this.store.records.set(value[this.store.keyPath], structuredClone(value));
      });
    }
    get(key) {
      return this.transaction.run(() => structuredClone(this.store.records.get(key)));
    }
    delete(key) {
      return this.transaction.run(() => {
        this.store.records.delete(key);
      });
    }
    openKeyCursor() {
      const keys = [...this.store.records.keys()].sort();
      const cursor = new Request();
      let index = 0;
      const step = () => {
        this.transaction.pending += 1;
        queueMicrotask(() => {
          this.transaction.pending -= 1;
          cursor.result = index < keys.length
            ? { primaryKey: keys[index], continue: () => { index += 1; step(); } }
            : null;
          cursor.onsuccess?.();
          this.transaction.settle();
        });
      };
      step();
      return cursor;
    }
  }
  globalThis.indexedDB = {
    open(name) {
      const open = new Request();
      setTimeout(() => {
        const created = !databases.has(name);
        if (created) databases.set(name, new Map());
        const state = databases.get(name);
        open.result = {
          objectStoreNames: { contains: (store) => state.has(store) },
          createObjectStore(store, { keyPath }) {
            state.set(store, { keyPath, records: new Map() });
          },
          transaction: () => new Transaction(state),
          close() {},
          onversionchange: null,
        };
        if (created) open.onupgradeneeded?.();
        open.onsuccess?.();
      }, 0);
      return open;
    },
  };
  return () => {
    delete globalThis.indexedDB;
  };
}

test("atlases built with build-sdf-atlas page names and sizes are accepted", async () => {
  const built = await atlasPackage("Builder", { size: 1024, file: "page-000.r8swz" });
  assert.equal(isValidPrebuiltSdfAtlasManifest(built.manifest, "Builder"), true);
  const provider = staticProvider(new Map([["Builder", built]]));
  const atlas = await buildPrebuiltSdfAtlas(provider, [request("Builder")], new AbortController().signal);
  assert.ok(atlas, "the builder's package is served as a prebuilt atlas");
  assert.equal(atlas.width, 1024);
  assert.equal(atlas.height, 1024);
  const [update] = await atlas.pageUpdates(new Map());
  assert.equal(update.pageWidth, 1024);
  assert.equal(update.pixels.byteLength, 1024 * 1024);
  await atlas.release();
});

test("families sampled at different sizes are not merged into one atlas", async () => {
  const provider = staticProvider(new Map([
    ["Regular", await atlasPackage("Regular")],
    ["Large", await atlasPackage("Large", { pointSize: 90, spread: 7.2 })],
  ]));
  const atlas = await buildPrebuiltSdfAtlas(
    provider,
    [request("Regular"), request("Large")],
    new AbortController().signal,
  );
  assert.equal(atlas, null, "mixed sampling parameters fall back to dynamic generation");
});

test("installing and removing a package takes effect for the next atlas", async () => {
  const uninstall = installFakeIndexedDb();
  try {
    const source = staticProvider(new Map([["Font", await atlasPackage("Font")]]));
    const pkg = createOriginPrebuiltSdfAtlasPackage({ namespace: "test", source, databaseName: "install-remove" });
    const signal = new AbortController().signal;
    assert.equal(await buildPrebuiltSdfAtlas(pkg.provider, [request("Font")], signal), null);

    await pkg.install(["Font"]);
    const installed = await buildPrebuiltSdfAtlas(pkg.provider, [request("Font")], signal);
    assert.ok(installed, "an install after the first lookup is used");
    await installed.release();

    await pkg.remove(["Font"]);
    assert.equal(
      await buildPrebuiltSdfAtlas(pkg.provider, [request("Font")], signal),
      null,
      "a removed package is no longer served",
    );
    pkg.close();
  } finally {
    uninstall();
  }
});

test("removing an empty family list keeps the installed packages", async () => {
  const uninstall = installFakeIndexedDb();
  try {
    const source = staticProvider(new Map([["Font", await atlasPackage("Font")]]));
    const pkg = createOriginPrebuiltSdfAtlasPackage({ namespace: "test", source, databaseName: "remove-empty" });
    const progress = [];
    await pkg.install(["Font"], { onProgress: (value) => progress.push(value) });
    const pageLength = 64 + 2048 * 2048;
    assert.deepEqual(
      progress.map(({ storedBytes, totalBytes }) => ({ storedBytes, totalBytes })),
      [{ storedBytes: pageLength, totalBytes: pageLength }],
    );

    await pkg.remove([]);
    await pkg.remove(["  "]);
    assert.deepEqual((await pkg.status(["Font"])).installedFamilies, ["Font"]);

    await pkg.remove();
    assert.deepEqual((await pkg.status(["Font"])).installedFamilies, []);
    pkg.close();
  } finally {
    uninstall();
  }
});

test("install progress reports the whole package size up front", async () => {
  const uninstall = installFakeIndexedDb();
  try {
    const packages = new Map([
      ["One", await atlasPackage("One", { size: 8, file: "page-000.r8swz" })],
      ["Two", await atlasPackage("Two", { size: 16, file: "page-000.r8swz" })],
    ]);
    const pkg = createOriginPrebuiltSdfAtlasPackage({
      namespace: "test",
      source: staticProvider(packages),
      databaseName: "progress",
    });
    const progress = [];
    await pkg.install(["One", "Two"], { concurrency: 1, onProgress: (value) => progress.push(value) });
    const total = 64 + 8 * 8 + 64 + 16 * 16;
    assert.deepEqual(progress.map((value) => value.totalBytes), [total, total]);
    assert.equal(progress.at(-1).storedBytes, total);
    pkg.close();
  } finally {
    uninstall();
  }
});
