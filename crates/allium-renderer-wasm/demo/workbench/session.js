import { BrowserRenderer } from "../../dist/index.js?v=20260715-13";
import { createEmptySekaiResourceProvider } from "./emptySekaiResourceProvider.js?v=20260715-13";
import { BrowserInputStore } from "./inputPersistence.js?v=20260715-13";

const CARD_WIDTH = 1830;
const CARD_HEIGHT = 812;
const FONT_FAMILY_ALIASES = new Map([
  ["FOT-RodinNTLGPro-DB", ["FOT-RodinNTLGPro-DB", "FZLanTingHei-DB-GBK"]],
  ["FOT-SkipProN-B", ["FOT-SkipProN-B", "FZZhengHei-EB-GBK"]],
  ["FOT-PopHappinessStd-EB", ["FOT-PopHappinessStd-EB", "FZShaoEr-M11-JF"]],
]);

export class WorkbenchSession extends EventTarget {
  constructor(canvas, inputStore = new BrowserInputStore()) {
    super();
    this.canvas = canvas;
    this.inputStore = inputStore;
    this.profileFile = null;
    this.fonts = [];
    this.nextFontId = 1;
    this.pages = [];
    this.activePage = -1;
    this.renderer = null;
    this.masterData = null;
    this.dump = null;
    this.config = null;
    this.busy = false;
  }

  get activeScene() {
    return this.pages[this.activePage]?.scene ?? null;
  }

  get activePageInfo() {
    return this.pages[this.activePage] ?? null;
  }

  async setProfileFile(file) {
    this.profileFile = file;
    this.emit("inputchange", { kind: "profile", file });
    await this.inputStore.saveProfile(file).catch(() => undefined);
  }

  async restoreInputs() {
    const restored = await this.inputStore.restore().catch(() => ({ profileFile: null, fonts: [] }));
    this.profileFile = restored.profileFile ?? null;
    this.fonts = (restored.fonts ?? []).map(({ file, bytes }) => ({
      id: this.nextFontId++,
      file,
      bytes,
      families: defaultFontFamilies(file.name),
    }));
    this.emit("inputchange", { kind: "restored", profileFile: this.profileFile, fonts: this.fonts });
    return { profileFile: this.profileFile, fonts: this.fonts };
  }

  async addFontFiles(files) {
    for (const file of files) {
      const bytes = await file.arrayBuffer();
      const stem = file.name.replace(/\.(?:ttf|otf)$/i, "");
      const families = defaultFontFamilies(file.name);
      const id = this.nextFontId++;
      this.fonts.push({ id, file, bytes, families });
      this.emit("fontchange", { kind: "added", id, file, families });
    }
    await this.inputStore.saveFonts(this.fonts).catch(() => undefined);
    this.emit("inputchange", { kind: "fonts", fonts: this.fonts });
  }

  removeFont(id) {
    const index = this.fonts.findIndex((font) => font.id === id);
    if (index < 0) return;
    const [font] = this.fonts.splice(index, 1);
    this.emit("fontchange", { kind: "removed", id, file: font.file, families: font.families });
    void this.inputStore.saveFonts(this.fonts).catch(() => undefined);
    this.emit("inputchange", { kind: "fonts", fonts: this.fonts });
  }

  updateFontFamilies(id, value) {
    const font = this.fonts.find((candidate) => candidate.id === id);
    if (!font) return;
    const families = [...new Set(String(value).split(",").map((family) => family.trim()).filter(Boolean))];
    if (families.length === 0) throw new Error("A font source needs at least one family name.");
    font.families = families;
    void this.inputStore.saveFonts(this.fonts).catch(() => undefined);
    this.emit("fontchange", { kind: "families", id, file: font.file, families });
    this.emit("inputchange", { kind: "fonts", fonts: this.fonts });
  }

  async build(config) {
    if (!this.profileFile) throw new Error("Choose a profile JSON file before building the scene.");
    if (this.fonts.length === 0) throw new Error("Register at least one font before building the scene.");
    await this.resetRuntime();
    this.config = normalizeConfig(config);
    this.setBusy(true, "Starting renderer", "Creating the WebGL2 context and worker");

    try {
      const profile = JSON.parse(await this.profileFile.text());
      const normalizedPages = normalizePages(profile);
      this.renderer = await BrowserRenderer.create({
        canvas: this.canvas,
        region: this.config.region,
        workerUrl: new URL("../../dist/worker.js?v=20260715-13", import.meta.url),
        moduleUrl: new URL("../../dist/allium_renderer_wasm.js?v=20260715-13", import.meta.url),
        wasmUrl: new URL("../../dist/allium_renderer_wasm.wasm?v=20260715-13", import.meta.url),
        resourceProvider: createEmptySekaiResourceProvider({
          region: this.config.region,
          assetBase: this.config.assetBase,
        }),
        resourceConcurrency: 8,
        fontProvider: {
          provide: async ({ family }) => {
            const font = this.fonts.find((candidate) => candidate.families.includes(family));
            return font ? { bytes: font.bytes } : null;
          },
        },
        fontConcurrency: 3,
        telemetry: { level: this.config.debugLevel, maxSamples: 240 },
      });

      this.setBusy(true, "Loading masterdata", this.config.masterdataBase);
      this.masterData = await this.renderer.loadMasterData(
        this.config.revision,
        async ({ table }, { signal }) => {
          const url = `${this.config.masterdataBase}/${encodeURIComponent(table)}.json`;
          const response = await fetch(url, { signal, cache: "default" });
          if (!response.ok) throw new Error(`Master-data fetch failed ${response.status}: ${url}`);
          return response.json();
        },
      );
      this.pages = normalizedPages.map((page) => ({ ...page, profile, scene: null, dump: null }));
      await this.switchPage(0);
      this.emit("ready", { pages: this.pages.length, dump: this.dump });
    } catch (error) {
      await this.resetRuntime();
      this.emit("error", { error });
      throw error;
    } finally {
      this.setBusy(false);
    }
  }

  async switchPage(index) {
    if (index < 0 || index >= this.pages.length) return;
    const previous = this.activePageInfo;
    const page = this.pages[index];
    this.setBusy(true, "Resolving scene", `Page ${index + 1} of ${this.pages.length}`);
    try {
      if (previous && previous !== page && previous.scene) {
        await previous.scene.destroy();
        previous.scene = null;
      }
      if (!page.scene) {
        page.scene = await this.renderer.createProfileScene({
          masterData: this.masterData,
          documentKey: page.documentKey,
          card: page.card,
          profile: page.profile,
          frameMode: "animate",
          sdf: {
            backend: this.config.sdfBackend,
            persistence: this.config.persistence,
          },
        });
      }
      this.activePage = index;
      await this.refreshDump();
      page.scene.draw();
      this.emit("pagechange", { index, page, dump: this.dump });
    } finally {
      this.setBusy(false);
    }
  }

  draw() {
    return this.activeScene?.draw();
  }

  async advance(tick) {
    if (!this.activeScene) return null;
    const result = await this.activeScene.advance(Math.max(0, Math.floor(tick)));
    this.activeScene.draw();
    await this.refreshDump(false);
    this.emit("scenechange", { reason: "tick", result, dump: this.dump });
    return result;
  }

  async setLayerVisible(layerId, visible) {
    if (!this.activeScene) return;
    const result = await this.activeScene.setLayerVisible(layerId, visible);
    this.activeScene.draw();
    await this.refreshDump(false);
    this.emit("scenechange", { reason: "mask", result, dump: this.dump });
  }

  async setLayerMasks(overrides) {
    if (!this.activeScene || !this.dump) return;
    const revision = Number(this.dump.revisions?.layer_table ?? 0);
    const result = await this.activeScene.setLayerMasks(revision, overrides);
    this.activeScene.draw();
    await this.refreshDump(false);
    this.emit("scenechange", { reason: "masks", result, dump: this.dump });
  }

  async setTab(controlId, value) {
    if (!this.activeScene) return;
    const result = await this.activeScene.setTab(controlId, value);
    this.activeScene.draw();
    await this.refreshDump(false);
    this.emit("scenechange", { reason: "tab", result, dump: this.dump });
  }

  async setScrollOffset(controlId, offset) {
    if (!this.activeScene) return;
    const result = await this.activeScene.setScrollOffset(controlId, Number(offset));
    this.activeScene.draw();
    await this.refreshDump(false);
    this.emit("scenechange", { reason: "scroll", result, dump: this.dump });
  }

  async scrollBy(controlId, delta) {
    if (!this.activeScene) return;
    const result = await this.activeScene.scrollBy(controlId, Number(delta));
    this.activeScene.draw();
    await this.refreshDump(false);
    this.emit("scenechange", { reason: "scroll", result, dump: this.dump });
  }

  async refreshDump(emit = true) {
    if (!this.activeScene) return null;
    this.dump = await this.activeScene.dump();
    this.activePageInfo.dump = this.dump;
    if (emit) this.emit("dump", { dump: this.dump });
    return this.dump;
  }

  async stats() {
    const renderer = this.renderer && typeof this.renderer.stats === "function"
      ? await this.renderer.stats()
      : { unavailable: true };
    const scene = this.activeScene && typeof this.activeScene.stats === "function"
      ? await this.activeScene.stats()
      : null;
    return { renderer, scene, dump: this.dump?.telemetry ?? null };
  }

  triggerContextLoss() {
    const gl = this.canvas.getContext("webgl2");
    const extension = gl?.getExtension("WEBGL_lose_context");
    if (!extension) return false;
    extension.loseContext();
    window.setTimeout(() => extension.restoreContext(), 900);
    return true;
  }

  async reset() {
    await this.resetRuntime();
    this.profileFile = null;
    this.fonts = [];
    await this.inputStore.clear().catch(() => undefined);
    this.emit("reset", {});
  }

  async resetRuntime() {
    for (const page of this.pages) await page.scene?.destroy().catch(() => undefined);
    this.pages = [];
    this.activePage = -1;
    this.dump = null;
    await this.masterData?.destroy?.().catch(() => undefined);
    this.masterData = null;
    this.renderer?.destroy?.();
    this.renderer = null;
  }

  setBusy(busy, label = "", detail = "") {
    this.busy = busy;
    this.emit("busy", { busy, label, detail });
  }

  emit(type, detail) {
    this.dispatchEvent(new CustomEvent(type, { detail }));
  }
}

export function normalizePages(input) {
  if (!input || Array.isArray(input) || !Array.isArray(input.userCustomProfileCards)) {
    throw new Error("Profile JSON must contain userCustomProfileCards.");
  }
  const entries = input.userCustomProfileCards;
  if (entries.length === 0 || entries.some((entry) => !entry || typeof entry !== "object")) {
    throw new Error("Profile JSON does not contain any valid custom-profile pages.");
  }
  return entries
    .map((entry, index) => {
      const card = entry.customProfileCard ?? entry;
      const sequence = Number.isFinite(entry.seq) ? Number(entry.seq) : index + 1;
      const cardId = entry.customProfileCardId ?? card.id ?? null;
      return {
        sequence,
        cardId,
        card,
        documentKey: `workbench:page:${sequence}:card:${cardId ?? "unassigned"}`,
      };
    })
    .sort((left, right) => left.sequence - right.sequence);
}

export function defaultFontFamilies(fileName) {
  const stem = String(fileName).replace(/\.(?:ttf|otf)$/i, "");
  return [...(FONT_FAMILY_ALIASES.get(stem) ?? [stem])];
}

function normalizeConfig(config) {
  return {
    region: String(config.region || "cn").toLowerCase(),
    revision: String(config.revision || "latest"),
    masterdataBase: trimSlash(config.masterdataBase),
    assetBase: trimSlash(config.assetBase),
    sdfBackend: config.sdfBackend === "analytic" ? "analytic" : "edt",
    persistence: config.persistence === "memory-only" ? "memory-only" : "origin",
    debugLevel: ["off", "trace"].includes(config.debugLevel) ? config.debugLevel : "summary",
  };
}

function trimSlash(value) {
  const result = String(value || "").trim().replace(/\/+$/, "");
  if (!/^https?:\/\//i.test(result)) throw new Error(`Provider URL must use HTTP or HTTPS: ${result || "empty value"}`);
  return result;
}

export { CARD_WIDTH, CARD_HEIGHT };
