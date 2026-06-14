// allium-renderer-wasm browser layer viewer demo.
//
// Loads the wasm renderer in a Web Worker, fetches masterdata + assets
// from a CDN base the user supplies, renders all layers of a custom
// profile card, and presents them as an interactive stack with
// per-layer visibility toggles, page navigation, and a property
// inspector.
//
// Layout, type chips, and visibility model follow the original viewer
// design (vanilla TS reimplementation — no React, no build step required
// for this file beyond the npm package's own tsc build of dist/).
//
// Resources are not bundled. Card JSON, fonts, masterdata, and assets
// all come from the user side. The CDN base is left blank by default
// — fill it in once and the demo will persist it in localStorage. If
// the CDN does not return CORS headers, use the bundled `serve.py`
// (same directory) which exposes a same-origin `/cdn/` proxy.

import { AlliumWorkerClient } from "../dist/worker-client.js";

// ── Constants ──

const DEFAULT_WIDTH = 1830;
const DEFAULT_HEIGHT = 812;

// Tables consumed by allium-renderer-host's JsonMasterDataProvider.
// If a table is missing the renderer falls back to safe defaults
// (most often: skip the element).
const REQUIRED_TABLES = [
  "cards",
  "stamps",
  "honors",
  "honorGroups",
  "bondsHonors",
  "bondsHonorWords",
  "gameCharacterUnits",
  "customProfileTextColors",
  "customProfileTextFonts",
  "customProfileShapeResources",
  "customProfileEtcResources",
  "customProfileCollectionResources",
  "customProfileGeneralBackgroundResources",
  "customProfileMemberStandingPictureResources",
  "customProfilePlayerInfoResources",
  "customProfileStoryBackgroundResources",
  "eventStories",
  "unitStoryEpisodeGroups",
];

// File-name → font family aliases for the FOT fonts shipped with the
// game. masterdata's customProfileTextFonts table references both the
// FOT names and the FZ fallbacks; supplying one TTF as both means the
// user only needs to drop one file per typeface.
const FONT_ALIASES = {
  "FOT-RodinNTLGPro-DB.ttf":     ["FOT-RodinNTLGPro-DB", "FZLanTingHei-DB-GBK"],
  "FOT-RodinNTLGPro-DB.otf":     ["FOT-RodinNTLGPro-DB", "FZLanTingHei-DB-GBK"],
  "FOT-SkipProN-B.otf":          ["FOT-SkipProN-B", "FZZhengHei-EB-GBK"],
  "FOT-PopHappinessStd-EB.otf":  ["FOT-PopHappinessStd-EB", "FZShaoEr-M11-JF"],
  "FOT-YurukaStd-UB.otf":        ["FOT-YurukaStd-UB", "FOT-Yuruka Std UB"],
};

const TYPE_LABELS = {
  text: "文本",
  shape: "形状",
  card_member: "卡面",
  stamp: "印章",
  other: "装饰",
  bonds_honor: "羁绊称号",
  honor: "称号",
  collection: "收藏品",
  general: "通用贴图",
  stand_member: "立绘",
  general_background: "通用背景",
  story_background: "剧情背景",
};

const TYPE_COLORS = {
  card_member: "var(--type-card)",
  stand_member: "var(--type-stand)",
  stamp: "var(--type-stamp)",
  honor: "var(--type-honor)",
  bonds_honor: "var(--type-honor)",
  collection: "var(--type-honor)",
  text: "var(--type-text)",
  shape: "var(--type-shape)",
  general_background: "var(--type-bg)",
  story_background: "var(--type-bg)",
};

// ── DOM refs ──

const $ = (id) => document.getElementById(id);
const els = {
  canvasWrap:   $("canvas-wrap"),
  canvasFrame:  $("canvas-frame"),
  hud:          $("hud"),
  hudText:      $("hud-text"),
  hudProgress:  $("hud-progress"),
  hudProgressBar: $("hud-progress-bar"),
  empty:        $("empty"),
  pagePrev:     $("page-prev"),
  pageNext:     $("page-next"),
  pageDots:     $("page-dots"),
  pageTabs:     $("page-tabs"),
  cardFile:     $("card-file"),
  region:       $("region"),
  fontFiles:    $("font-files"),
  cdnBase:      $("cdn-base"),
  profileFile:  $("profile-file"),
  renderBtn:    $("render-btn"),
  status:       $("status"),
  layerCount:   $("layer-count"),
  showAll:      $("show-all"),
  hideAll:      $("hide-all"),
  resetVis:     $("reset-vis"),
  layerList:    $("layer-list"),
};

// ── State ──

const state = {
  /** AlliumWorkerClient | null — lazily spawned on first render. */
  client: null,
  /** Per-page layer arrays. Each layer: { z, type, original_visible,
   *  data, x, y, width, height, properties?, userVisible, blobUrl } */
  pages: [],
  /** Active page index. */
  activePage: 0,
  /** Output canvas size (driven by the wasm output, may vary by card). */
  canvasW: DEFAULT_WIDTH,
  canvasH: DEFAULT_HEIGHT,
};

// ── Helpers ──

function setStatus(text, kind) {
  els.status.textContent = text;
  els.status.className = "status" + (kind ? " " + kind : "");
}

function setHud(visible, text, progress) {
  if (!visible) {
    els.hud.hidden = true;
    return;
  }
  els.hud.hidden = false;
  els.hudText.textContent = text || "";
  if (progress == null) {
    els.hudProgress.hidden = true;
  } else {
    els.hudProgress.hidden = false;
    els.hudProgressBar.style.width = Math.round(Math.min(1, Math.max(0, progress)) * 100) + "%";
  }
}

function escHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

// ── Card normalization ──
//
// The renderer takes one CustomProfileCard at a time, but the
// `userCustomProfileCards` API returns up to N cards per user (each
// with its own `seq`, sorted ascending = display order). This helper
// returns the full list as `[{ seq, cardId, cardJson }]` so the demo
function normalizeCardJsonPages(text) {
  let v = JSON.parse(text);
  if (v && !Array.isArray(v) && Array.isArray(v.userCustomProfileCards)) {
    v = v.userCustomProfileCards;
  }
  // Tolerate three shapes: { userCustomProfileCards: [...] } |
  // [{ customProfileCard, ... }, ...] | a single CustomProfileCard.
  const entries = Array.isArray(v) ? v : [v];
  if (entries.length === 0) throw new Error("名片数组为空");

  const pages = entries.map((entry, i) => {
    const seq = typeof entry?.seq === "number" ? entry.seq : i + 1;
    const cardId = entry?.customProfileCardId ?? null;
    const cardObj = entry?.customProfileCard ?? entry;
    return { seq, cardId, cardJson: JSON.stringify(cardObj) };
  });

  // Sort by seq ascending so page 1 is first regardless of input order.
  pages.sort((a, b) => a.seq - b.seq);
  return pages;
}

// ── CDN fetching ──

async function fetchMasterdata(region, cdnBase, onProgress) {
  const base = `${cdnBase}/masterdata/${region}/latest`;
  const out = {};
  let done = 0;
  let failed = 0;

  await Promise.all(REQUIRED_TABLES.map(async (table) => {
    try {
      const res = await fetch(`${base}/${table}.json`);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      out[table] = await res.text();
    } catch {
      failed++;
    } finally {
      done++;
      onProgress(done / REQUIRED_TABLES.length, `masterdata: ${done}/${REQUIRED_TABLES.length}`);
    }
  }));

  if (failed > 0) {
    console.warn(`masterdata: ${failed} 表拉取失败，渲染可能不完整`);
  }
  return out;
}

async function fetchAssets(client, cardJsons, masterData, region, cdnBase, onProgress) {
  // Union asset keys across all pages (collectAssetKeys per page, then
  // dedupe). Cards typically share many keys — fetching once is enough.
  const keySet = new Set();
  for (const cardJson of cardJsons) {
    const keys = await client.collectAssetKeys(cardJson, masterData);
    for (const k of keys) keySet.add(k);
  }
  const keys = Array.from(keySet);
  const assets = [];
  if (keys.length === 0) return assets;

  const base = `${cdnBase}/assets/${region}`;
  const CONCURRENCY = 6;
  const queue = keys.slice();
  let done = 0;
  let failed = 0;

  async function pull() {
    while (queue.length > 0) {
      const key = queue.shift();
      try {
        const res = await fetch(`${base}/${key}.png`);
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const buf = await res.arrayBuffer();
        assets.push({ key, bytes: new Uint8Array(buf) });
      } catch {
        failed++;
      } finally {
        done++;
        onProgress(done / keys.length, `素材: ${done}/${keys.length}`);
      }
    }
  }

  await Promise.all(
    Array.from({ length: Math.min(CONCURRENCY, keys.length) }, () => pull())
  );

  if (failed > 0) {
    console.warn(`素材: ${failed} 个缺失`);
  }
  return assets;
}

// ── Font collection ──

async function readFontEntries(files) {
  const entries = [];
  for (const file of files) {
    const families = FONT_ALIASES[file.name]
      || [file.name.replace(/\.(ttf|otf|TTF|OTF)$/, "")];
    const bytes = new Uint8Array(await file.arrayBuffer());
    for (const family of families) {
      // slice() so the worker can transfer each buffer independently
      entries.push({ family, bytes: bytes.slice() });
    }
  }
  return entries;
}

// ── Worker client ──

async function ensureClient() {
  if (state.client) return state.client;
  setHud(true, "加载 wasm Worker…");
  state.client = await AlliumWorkerClient.spawn({
    workerUrl: new URL("../dist/worker.js", import.meta.url),
    moduleUrl: new URL("../dist/allium_renderer_wasm.js", import.meta.url).href,
    wasmUrl:   new URL("../dist/allium_renderer_wasm.wasm", import.meta.url).href,
  });
  return state.client;
}

// ── Render orchestration ──
//
// The card JSON may describe N pages (an array) or a single card. We
// render each page independently and keep them in state.pages; the
// page navigator switches between them.

async function doRender() {
  els.renderBtn.disabled = true;
  els.empty.hidden = true;
  clearPages();

  const t0 = performance.now();

  try {
    const rawCard = await readCardText();
    const pageInputs = normalizeCardJsonPages(rawCard);
    console.log(`[demo] 名片页数: ${pageInputs.length}`, pageInputs.map(p => ({ seq: p.seq, cardId: p.cardId, bytes: p.cardJson.length })));
    const profileJson = await readProfileText();

    const region = els.region.value;
    const cdnBase = els.cdnBase.value.trim().replace(/\/+$/, "");
    if (!cdnBase) {
      setStatus("请填写 CDN base", "error");
      els.empty.hidden = false;
      setHud(false);
      return;
    }
    persistInputs(region, cdnBase, rawCard, profileJson);

    setStatus("加载中…");
    const client = await ensureClient();

    setHud(true, "拉取 masterdata…", 0);
    const masterData = await fetchMasterdata(region, cdnBase, (p, t) => {
      setHud(true, t, p * 0.3);
    });

    let fonts = [];
    if (els.fontFiles.files.length > 0) {
      setHud(true, "读取字体…", 0.3);
      fonts = await readFontEntries(els.fontFiles.files);
    }

    setHud(true, "收集素材列表…", 0.32);
    const cardJsons = pageInputs.map((p) => p.cardJson);
    const assets = await fetchAssets(client, cardJsons, masterData, region, cdnBase, (p, t) => {
      setHud(true, t, 0.35 + p * 0.4);
    });

    // Render every page. The wasm ABI is one card at a time; we loop
    // here. renderAllLayers *transfers* font/asset ArrayBuffers to the
    // worker (postMessage ownership handover). After the first page
    // those buffers are detached, so we must slice() fresh copies for
    // every page. masterData (plain JSON strings) is cloned, not
    // transferred — no copy needed.
    const pages = [];
    for (let i = 0; i < pageInputs.length; i++) {
      const { cardJson, seq, cardId } = pageInputs[i];
      const phaseLabel = pageInputs.length > 1
        ? `渲染分层… (${i + 1}/${pageInputs.length})`
        : "渲染分层…";
      setHud(true, phaseLabel, 0.78 + (i / pageInputs.length) * 0.2);

      // Fresh copies for each page — the worker transfers them.
      const fontsCopy = fonts.map((f) => ({ family: f.family, bytes: f.bytes.slice() }));
      const assetsCopy = assets.map((a) => ({ key: a.key, bytes: a.bytes.slice() }));

      const result = await client.renderAllLayers({
        cardJson,
        profileJson,
        quality: 80,
        includeProperties: true,
        masterData,
        fonts: fontsCopy,
        assets: assetsCopy,
      });

      const layers = result.map((l, idx) => ({
        ...l,
        index: idx,
        userVisible: l.original_visible,
        blobUrl: (l.data && l.data.byteLength > 0)
          ? URL.createObjectURL(new Blob([l.data], { type: "image/webp" }))
          : null,
      }));

      pages.push({
        seq,
        cardId,
        layers,
        width: DEFAULT_WIDTH,
        height: DEFAULT_HEIGHT,
      });
    }

    state.pages = pages;
    state.activePage = 0;
    state.canvasW = DEFAULT_WIDTH;
    state.canvasH = DEFAULT_HEIGHT;

    renderActivePage();
    buildLayerList();
    updatePageNav();
    setHud(false);

    const ms = performance.now() - t0;
    const totalLayers = pages.reduce((n, p) => n + p.layers.length, 0);
    const summary = pages.length > 1
      ? `完成 · ${(ms / 1000).toFixed(1)}s · ${pages.length} 页 · ${totalLayers} 层`
      : `完成 · ${(ms / 1000).toFixed(1)}s · ${totalLayers} 层`;
    setStatus(summary, "ok");
    els.showAll.disabled = els.hideAll.disabled = els.resetVis.disabled = false;
  } catch (err) {
    console.error(err);
    setStatus(`渲染失败：${err.message || err}`, "error");
    setHud(false);
    els.empty.hidden = false;
  } finally {
    els.renderBtn.disabled = false;
  }
}

// ── Page management ──
//
// Even when a single card is rendered, the page model still applies
// (just one page). This keeps the navigator code uniform with future
// multi-page support.

function clearPages() {
  for (const page of state.pages) {
    for (const l of page.layers) {
      if (l.blobUrl) URL.revokeObjectURL(l.blobUrl);
    }
  }
  state.pages = [];
  state.activePage = 0;
  els.canvasFrame.querySelectorAll("img.layer").forEach((el) => el.remove());
  els.layerList.innerHTML = '<li class="layer-empty">渲染后将列出所有图层。</li>';
  els.layerCount.textContent = "0";
  els.showAll.disabled = els.hideAll.disabled = els.resetVis.disabled = true;
}

function renderActivePage() {
  // Wipe existing layer <img>s but keep HUD/empty.
  els.canvasFrame.querySelectorAll("img.layer").forEach((el) => el.remove());
  const page = state.pages[state.activePage];
  if (!page) return;

  els.canvasFrame.style.aspectRatio = `${state.canvasW} / ${state.canvasH}`;

  for (const layer of page.layers) {
    if (!layer.blobUrl) continue;
    const img = document.createElement("img");
    img.className = "layer" + (layer.userVisible ? "" : " hidden");
    img.src = layer.blobUrl;
    img.alt = TYPE_LABELS[layer.type] || layer.type;
    img.dataset.layerIndex = String(layer.index);
    img.style.zIndex = String(layer.z);
    img.style.left = (layer.x / state.canvasW * 100).toFixed(4) + "%";
    img.style.top = (layer.y / state.canvasH * 100).toFixed(4) + "%";
    img.style.width = (layer.width / state.canvasW * 100).toFixed(4) + "%";
    img.style.height = (layer.height / state.canvasH * 100).toFixed(4) + "%";
    els.canvasFrame.appendChild(img);
  }
}

function updatePageNav() {
  const n = state.pages.length;
  if (n <= 1) {
    els.pagePrev.hidden = true;
    els.pageNext.hidden = true;
    els.pageDots.hidden = true;
    els.pageTabs.hidden = true;
    return;
  }
  els.pagePrev.hidden = state.activePage === 0;
  els.pageNext.hidden = state.activePage === n - 1;
  els.pageDots.hidden = false;
  els.pageDots.innerHTML = "";
  for (let i = 0; i < n; i++) {
    const dot = document.createElement("button");
    dot.className = "page-dot" + (i === state.activePage ? " active" : "");
    dot.setAttribute("aria-label", `第 ${i + 1} 页`);
    dot.addEventListener("click", () => setPage(i));
    els.pageDots.appendChild(dot);
  }

  // Panel page tabs
  els.pageTabs.hidden = false;
  els.pageTabs.innerHTML = "";
  for (let i = 0; i < n; i++) {
    const tab = document.createElement("button");
    tab.className = "page-tab" + (i === state.activePage ? " active" : "");
    tab.textContent = `第 ${i + 1} 页`;
    tab.addEventListener("click", () => setPage(i));
    els.pageTabs.appendChild(tab);
  }

}

function setPage(i) {
  if (i < 0 || i >= state.pages.length || i === state.activePage) return;
  state.activePage = i;
  renderActivePage();
  buildLayerList();
  updatePageNav();
}

// ── Layer list ──

function buildLayerList() {
  els.layerList.innerHTML = "";
  const page = state.pages[state.activePage];
  if (!page) return;

  if (page.layers.length === 0) {
    const li = document.createElement("li");
    li.className = "layer-empty";
    li.textContent = "此页无图层。";
    els.layerList.appendChild(li);
    return;
  }

  let totalVisible = 0;
  let userVisible = 0;
  for (const l of page.layers) {
    if (l.original_visible) totalVisible++;
    if (l.userVisible && l.original_visible) userVisible++;
  }
  els.layerCount.textContent = `${userVisible}/${totalVisible}`;

  for (const layer of page.layers) {
    const li = document.createElement("li");
    li.className = "layer-row";
    li.dataset.layerIndex = String(layer.index);

    const head = document.createElement("div");
    head.className = "layer-head";

    const bar = document.createElement("span");
    bar.className = "layer-type-bar";
    bar.style.background = TYPE_COLORS[layer.type] || "transparent";
    head.appendChild(bar);

    const eye = document.createElement("button");
    eye.className = "layer-eye " + (layer.userVisible ? "on" : "off");
    eye.title = layer.userVisible ? "隐藏此层" : "显示此层";
    eye.textContent = layer.userVisible ? "●" : "○";
    eye.disabled = !layer.original_visible;
    eye.addEventListener("click", (e) => {
      e.stopPropagation();
      toggleLayer(layer.index);
    });
    head.appendChild(eye);

    const name = document.createElement("div");
    name.className = "layer-name";
    const typeLabel = TYPE_LABELS[layer.type] || layer.type;
    const sizeStr = layer.width > 0 ? `${layer.width}×${layer.height}` : "—";
    const visMark = layer.original_visible ? "" : " · 不可见";
    name.innerHTML =
      `<span class="layer-z">z${layer.z}</span>` +
      `<span class="layer-type">${escHtml(typeLabel)}</span>` +
      `<span class="layer-detail">${sizeStr}${visMark}</span>`;
    head.appendChild(name);

    const hasProps = layer.properties && Object.keys(layer.properties).length > 0;
    if (hasProps) {
      const arrow = document.createElement("span");
      arrow.className = "layer-toggle-arrow";
      arrow.textContent = "▸";
      head.appendChild(arrow);
      head.addEventListener("click", () => {
        li.classList.toggle("expanded");
      });
    } else {
      head.style.cursor = "default";
      head.appendChild(document.createElement("span"));
    }

    li.appendChild(head);

    if (hasProps) {
      const props = document.createElement("div");
      props.className = "layer-props";
      const grid = document.createElement("div");
      grid.className = "layer-props-grid";
      for (const [key, value] of Object.entries(layer.properties)) {
        if (value == null || value === "") continue;
        const strVal = typeof value === "string" ? value : String(value);
        const isColor = typeof value === "string" && /^#[0-9a-fA-F]{3,8}$/.test(value);
        const isText = key === "text" || key === "label" || key === "content";

        const k = document.createElement("span");
        k.className = "k";
        k.textContent = key;

        const v = document.createElement("span");
        if (isColor) {
          v.className = "v";
          v.innerHTML =
            `<span class="swatch" style="background:${escHtml(strVal)}"></span>${escHtml(strVal)}`;
        } else if (isText) {
          v.className = "v mono";
          v.textContent = strVal;
        } else {
          v.className = "v";
          v.textContent = strVal;
        }

        grid.appendChild(k);
        grid.appendChild(v);
      }
      props.appendChild(grid);
      li.appendChild(props);
    }

    els.layerList.appendChild(li);
  }
}

function toggleLayer(layerIndex) {
  const page = state.pages[state.activePage];
  if (!page) return;
  const layer = page.layers[layerIndex];
  if (!layer || !layer.original_visible) return;
  layer.userVisible = !layer.userVisible;

  const img = els.canvasFrame.querySelector(`img.layer[data-layer-index="${layerIndex}"]`);
  if (img) img.classList.toggle("hidden", !layer.userVisible);

  const row = els.layerList.querySelector(`.layer-row[data-layer-index="${layerIndex}"]`);
  if (row) {
    const eye = row.querySelector(".layer-eye");
    eye.className = "layer-eye " + (layer.userVisible ? "on" : "off");
    eye.textContent = layer.userVisible ? "●" : "○";
    eye.title = layer.userVisible ? "隐藏此层" : "显示此层";
  }
  updateCount();
}

function updateCount() {
  const page = state.pages[state.activePage];
  if (!page) return;
  let totalVisible = 0;
  let userVisible = 0;
  for (const l of page.layers) {
    if (l.original_visible) totalVisible++;
    if (l.userVisible && l.original_visible) userVisible++;
  }
  els.layerCount.textContent = `${userVisible}/${totalVisible}`;
}

function bulkVisibility(setter) {
  const page = state.pages[state.activePage];
  if (!page) return;
  for (const layer of page.layers) {
    if (!layer.original_visible) continue;
    layer.userVisible = setter(layer);
  }
  // Re-sync DOM in one pass.
  for (const layer of page.layers) {
    const img = els.canvasFrame.querySelector(`img.layer[data-layer-index="${layer.index}"]`);
    if (img) img.classList.toggle("hidden", !layer.userVisible);
    const row = els.layerList.querySelector(`.layer-row[data-layer-index="${layer.index}"]`);
    if (row) {
      const eye = row.querySelector(".layer-eye");
      eye.className = "layer-eye " + (layer.userVisible ? "on" : "off");
      eye.textContent = layer.userVisible ? "●" : "○";
    }
  }
  updateCount();
}

// ── Drag-and-drop ──

function bindDropTarget() {
  const frame = els.canvasFrame;
  frame.addEventListener("dragover", (e) => {
    e.preventDefault();
    frame.classList.add("drag-over");
  });
  frame.addEventListener("dragleave", () => frame.classList.remove("drag-over"));
  frame.addEventListener("drop", (e) => {
    e.preventDefault();
    frame.classList.remove("drag-over");
    const file = e.dataTransfer?.files?.[0];
    if (!file || !file.name.toLowerCase().endsWith(".json")) return;
    const dt = new DataTransfer();
    dt.items.add(file);
    els.cardFile.files = dt.files;
    setStatus(`已载入 ${file.name}`);
    els.renderBtn.disabled = false;
  });
}

// ── Page swipe (touch only — buttons handle mouse navigation) ──

function bindPageSwipe() {
  const SWIPE_MIN = 30;
  const SWIPE_RATIO = 1.5;
  let start = null;

  els.canvasFrame.addEventListener("pointerdown", (e) => {
    if (!e.isPrimary || e.pointerType === "mouse") return;
    start = { x: e.clientX, y: e.clientY, aborted: false };
  });
  els.canvasFrame.addEventListener("pointermove", (e) => {
    if (!start || start.aborted) return;
    const absDy = Math.abs(e.clientY - start.y);
    const absDx = Math.abs(e.clientX - start.x);
    if (absDy > absDx && absDy > 10) start.aborted = true;
  });
  els.canvasFrame.addEventListener("pointerup", (e) => {
    const s = start; start = null;
    if (!s || s.aborted) return;
    const dx = e.clientX - s.x;
    const dy = e.clientY - s.y;
    if (Math.abs(dx) >= SWIPE_MIN && Math.abs(dx) > Math.abs(dy) * SWIPE_RATIO) {
      if (dx < 0) setPage(state.activePage + 1);
      else setPage(state.activePage - 1);
    }
  });
}

// ── localStorage persistence ──

function lstoreKey(name) { return `allium-demo-${name}`; }

function lstoreGet(name) {
  try { return localStorage.getItem(lstoreKey(name)); } catch { return null; }
}

function lstoreSet(name, value) {
  try { localStorage.setItem(lstoreKey(name), value); } catch {}
}

// Restore saved inputs so the user doesn't have to reselect everything
// on each refresh. Fonts and files can't be persisted (binary, large);
// only text values are cached. Card JSON is stored as plain text — the
// file input can't be repopulated, but the render button is enabled
// when cached card text exists.
function restoreInputs() {
  const region = lstoreGet("region");
  if (region) els.region.value = region;

  const cdnBase = lstoreGet("cdn-base");
  if (cdnBase) {
    els.cdnBase.value = cdnBase;
  } else {
    // Default to same-origin proxy. The fetch probe is best-effort —
    // if the proxy is down we'll get a 404 and that's fine too.
    els.cdnBase.value = `${location.origin}/cdn`;
    fetch("/cdn/", { method: "HEAD" })
      .catch(() => {
        // Proxy unreachable — clear the guess so the user knows to fill it in.
        // But keep it if localStorage has since been populated.
        if (!lstoreGet("cdn-base")) els.cdnBase.value = "";
      });
  }

  // Cached card JSON lets the user render without re-selecting a file.
  if (lstoreGet("card-json")) els.renderBtn.disabled = false;
}

// Called from doRender on success — cache the inputs that worked.
function persistInputs(region, cdnBase, rawCard, profileJson) {
  lstoreSet("region", region);
  lstoreSet("cdn-base", cdnBase);
  lstoreSet("card-json", rawCard);
  if (profileJson) lstoreSet("profile-json", profileJson);
}

// Called from doRender — get card text from file or localStorage cache.
async function readCardText() {
  if (els.cardFile.files[0]) return els.cardFile.files[0].text();
  const cached = lstoreGet("card-json");
  if (cached) return cached;
  throw new Error("请选择名片 JSON");
}

async function readProfileText() {
  if (els.profileFile.files[0]) return els.profileFile.files[0].text();
  return lstoreGet("profile-json") || undefined;
}

// ── Event wiring ──

els.cardFile.addEventListener("change", () => {
  els.renderBtn.disabled = els.cardFile.files.length === 0;
});
els.region.addEventListener("change", () => {
  lstoreSet("region", els.region.value);
});
els.cdnBase.addEventListener("change", () => {
  lstoreSet("cdn-base", els.cdnBase.value.trim());
});
els.renderBtn.addEventListener("click", doRender);
els.pagePrev.addEventListener("click", () => setPage(state.activePage - 1));
els.pageNext.addEventListener("click", () => setPage(state.activePage + 1));
els.showAll.addEventListener("click", () => bulkVisibility(() => true));
els.hideAll.addEventListener("click", () => bulkVisibility(() => false));
els.resetVis.addEventListener("click", () => bulkVisibility((l) => l.original_visible));

bindDropTarget();
bindPageSwipe();
restoreInputs();
setStatus("未加载");
