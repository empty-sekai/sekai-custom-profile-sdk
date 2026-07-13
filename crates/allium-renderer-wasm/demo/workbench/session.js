import { BrowserRenderer } from "../../dist/index.js";

const CARD_WIDTH = 1830;
const CARD_HEIGHT = 812;

export class WorkbenchSession extends EventTarget {
  constructor(canvas) {
    super();
    this.canvas = canvas;
    this.cardFile = null;
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

  setCardFile(file) {
    this.cardFile = file;
    this.emit("inputchange", { kind: "card", file });
  }

  setProfileFile(file) {
    this.profileFile = file;
    this.emit("inputchange", { kind: "profile", file });
  }

  async addFontFiles(files) {
    for (const file of files) {
      const bytes = await file.arrayBuffer();
      const stem = file.name.replace(/\.(?:ttf|otf)$/i, "");
      const families = [stem];
      const id = this.nextFontId++;
      this.fonts.push({ id, file, bytes, families });
      this.emit("fontchange", { kind: "added", id, file, families });
    }
    this.emit("inputchange", { kind: "fonts", fonts: this.fonts });
  }

  removeFont(id) {
    const index = this.fonts.findIndex((font) => font.id === id);
    if (index < 0) return;
    const [font] = this.fonts.splice(index, 1);
    this.emit("fontchange", { kind: "removed", id, file: font.file, families: font.families });
    this.emit("inputchange", { kind: "fonts", fonts: this.fonts });
  }

  updateFontFamilies(id, value) {
    const font = this.fonts.find((candidate) => candidate.id === id);
    if (!font) return;
    const families = [...new Set(String(value).split(",").map((family) => family.trim()).filter(Boolean))];
    if (families.length === 0) throw new Error("A font source needs at least one family name.");
    font.families = families;
    this.emit("fontchange", { kind: "families", id, file: font.file, families });
    this.emit("inputchange", { kind: "fonts", fonts: this.fonts });
  }

  async build(config) {
    if (!this.cardFile) throw new Error("Choose a card JSON file before building the scene.");
    if (this.fonts.length === 0) throw new Error("Register at least one font before building the scene.");
    await this.resetRuntime();
    this.config = normalizeConfig(config);
    this.setBusy(true, "Starting renderer", "Creating the WebGL2 context and worker");

    try {
      const cardInput = JSON.parse(await this.cardFile.text());
      const profile = this.profileFile ? JSON.parse(await this.profileFile.text()) : undefined;
      const normalizedPages = normalizePages(cardInput);
      this.renderer = await BrowserRenderer.create({
        canvas: this.canvas,
        region: this.config.region,
        workerUrl: new URL("../../dist/worker.js", import.meta.url),
        moduleUrl: new URL("../../dist/allium_renderer_wasm.js", import.meta.url),
        wasmUrl: new URL("../../dist/allium_renderer_wasm.wasm", import.meta.url),
        resolveMasterDataUrl: (table) => `${this.config.masterdataBase}/${encodeURIComponent(table)}.json`,
        resolveResourceUrl: (namespace, key) => namespace === "static"
          ? `https://cdn.emptysekai.com/renderer-static/v0.2/${normalizeAssetKey(key)}.png`
          : `${this.config.assetBase}/${normalizeAssetKey(key)}.png`,
        telemetry: { level: this.config.debugLevel, maxSamples: 240 },
      });

      this.setBusy(true, "Registering fonts", `${this.fonts.length} source file${this.fonts.length === 1 ? "" : "s"}`);
      for (const font of this.fonts) {
        for (const family of font.families) {
          await this.renderer.registerFont({ family, bytes: font.bytes });
        }
      }

      this.setBusy(true, "Loading masterdata", this.config.masterdataBase);
      this.masterData = await this.renderer.loadMasterData(this.config.revision);
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
    this.cardFile = null;
    this.profileFile = null;
    this.fonts = [];
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
  const wrapped = input && !Array.isArray(input) && Array.isArray(input.userCustomProfileCards)
    ? input.userCustomProfileCards
    : input;
  const entries = Array.isArray(wrapped) ? wrapped : [wrapped];
  if (entries.length === 0 || entries.some((entry) => !entry || typeof entry !== "object")) {
    throw new Error("Card JSON does not contain a profile card or page array.");
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

function normalizeAssetKey(key) {
  return String(key).replace(/^\/+/, "").replace(/\.png$/i, "");
}

export { CARD_WIDTH, CARD_HEIGHT };
