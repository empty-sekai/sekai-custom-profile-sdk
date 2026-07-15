import { WorkbenchSession } from "./workbench/session.js?v=20260715-13";
import { SceneInspector } from "./workbench/inspector.js?v=20260715-13";
import { InteractionOverlay, handleInteractionAction } from "./workbench/interactions.js?v=20260715-13";
import { TelemetryPanel } from "./workbench/telemetry.js?v=20260715-13";

const byId = (id) => document.getElementById(id);
const elements = {
  canvas: byId("scene-canvas"),
  stageShell: byId("stage-shell"),
  stageViewport: byId("stage-viewport"),
  stageEmpty: byId("stage-empty"),
  stageBusy: byId("stage-busy"),
  stageBusyLabel: byId("stage-busy-label"),
  stageBusyDetail: byId("stage-busy-detail"),
  contextBanner: byId("context-banner"),
  contextMessage: byId("context-message"),
  runtimeDot: byId("runtime-dot"),
  runtimeStatus: byId("runtime-status"),
  schemaPill: byId("schema-pill"),
  profileFile: byId("profile-file"),
  fontFiles: byId("font-files"),
  profileFileName: byId("profile-file-name"),
  fontList: byId("font-list"),
  fontCount: byId("font-count"),
  region: byId("region"),
  revision: byId("revision"),
  masterdataBase: byId("masterdata-base"),
  assetBase: byId("asset-base"),
  providerPreview: byId("provider-preview"),
  sdfBackend: byId("sdf-backend"),
  persistence: byId("persistence"),
  debugLevel: byId("debug-level"),
  createScene: byId("create-scene"),
  resetSession: byId("reset-session"),
  sessionProgress: byId("session-progress"),
  sessionMessage: byId("session-message"),
  pageTabs: byId("page-tabs"),
  previousPage: byId("previous-page"),
  nextPage: byId("next-page"),
  timelinePlay: byId("timeline-play"),
  timelineStepBack: byId("timeline-step-back"),
  timelineStepForward: byId("timeline-step-forward"),
  timelineRange: byId("timeline-range"),
  timelineOutput: byId("timeline-output"),
  timelineLoop: byId("timeline-loop"),
  timelineFinal: byId("timeline-final"),
  toggleOverlays: byId("toggle-overlays"),
  exportDump: byId("export-dump"),
  copyDump: byId("copy-dump"),
  loseContext: byId("lose-context"),
  fitStage: byId("fit-stage"),
  eventLog: byId("event-log"),
  clearEvents: byId("clear-events"),
  toastRegion: byId("toast-region"),
};

const session = new WorkbenchSession(elements.canvas);
Object.defineProperty(globalThis, "__ALLIUM_RENDERER_WORKBENCH__", {
  configurable: true,
  value: Object.freeze({
    dump: () => session.dump,
    stats: () => session.stats(),
    draw: () => session.draw(),
    page: (index) => session.switchPage(Number(index)),
    tab: (controlId, value) => session.setTab(String(controlId), String(value)),
    scroll: (controlId, offset) => session.setScrollOffset(String(controlId), Number(offset)),
    scrollBy: (controlId, delta) => session.scrollBy(String(controlId), Number(delta)),
  }),
});
const telemetry = new TelemetryPanel(byId("telemetry-dashboard"), byId("stage-fps"));
const overlay = new InteractionOverlay(byId("interaction-overlay"), byId("hover-card"), {
  onEvent: emitInteractionEvent,
  onActivate: (region, target) => guarded(() => activateRegion(region, target)),
  onScroll: (controlId, direction, region) => guarded(async () => {
    const delta = direction * scrollStep(controlId);
    await session.scrollBy(controlId, delta);
    pushEvent("scroll", region.role, { controlId, delta });
  }),
});
const inspector = new SceneInspector({
  layerTree: byId("layer-tree"),
  layerSearch: byId("layer-search"),
  layerType: byId("layer-type"),
  groupLayers: byId("group-layers"),
  showAll: byId("show-all-layers"),
  hideAll: byId("hide-all-layers"),
  detailTabs: document.querySelectorAll(".detail-tab"),
  layerDetail: byId("layer-detail"),
  controls: byId("component-controls"),
  interactions: byId("interaction-list"),
  controlSearch: byId("control-search"),
  clearInteraction: byId("clear-interaction-focus"),
  dumpPreview: byId("dump-preview"),
  dumpSize: byId("dump-size"),
  dumpSearch: byId("dump-search"),
  dumpPath: byId("dump-path"),
  dumpUp: byId("dump-up"),
  dumpViewMode: byId("dump-view-mode"),
  copyDump: elements.copyDump,
}, {
  onLayerVisible: (layerId, visible) => guarded(() => session.setLayerVisible(layerId, visible)),
  onLayerMasks: (overrides) => guarded(() => session.setLayerMasks(overrides)),
  onLayerSelected: selectLayerRegions,
  onTab: (controlId, value) => guarded(async () => {
    await session.setTab(controlId, value);
    pushEvent("control", "tab", { controlId, value });
  }),
  onScrollOffset: (controlId, offset) => guarded(() => session.setScrollOffset(controlId, offset)),
  onScrollBy: (controlId, delta) => guarded(() => session.scrollBy(controlId, delta)),
  onInteraction: (region, action) => guarded(() => handleInteractionAction(region, action, emitInteractionEvent)),
  onInteractionSelected: (regionId) => overlay.select(regionId),
});

let playing = false;
let animationEpoch = 0;
let animationStartTick = 0;
let advancePending = false;
let statsTimer = null;

bindInputs();
bindNavigation();
bindTimeline();
bindInspectorTabs();
bindContextRecovery();
bindKeyboard();
updateProviderPreview();
updateInputState();
telemetry.render(null);

session.addEventListener("inputchange", () => {
  renderFonts();
  updateInputState();
});
session.addEventListener("busy", ({ detail }) => setBusy(detail.busy, detail.label, detail.detail));
session.addEventListener("ready", ({ detail }) => {
  elements.stageEmpty.hidden = true;
  setRuntime("ready", `${detail.pages} page${detail.pages === 1 ? "" : "s"} ready`);
  enableSceneControls(true);
  renderPages();
  setSessionMessage("Scene created. All source inputs remain in session memory.", "success");
  startStatsPolling();
});
session.addEventListener("pagechange", ({ detail }) => {
  renderPages();
  applyDump(detail.dump);
  setTick(detail.dump?.tick ?? 0, false);
});
session.addEventListener("scenechange", ({ detail }) => {
  if (detail.reason === "tick") inspector.updateRuntimeDump(detail.dump);
  applyDump(detail.dump, detail.reason !== "tick");
});
session.addEventListener("dump", ({ detail }) => applyDump(detail.dump));
session.addEventListener("error", ({ detail }) => {
  stopPlayback();
  elements.stageEmpty.hidden = false;
  enableSceneControls(false);
  inspector.setDump(null);
  overlay.render(null);
  setRuntime("error", "Scene failed");
  toast(errorMessage(detail.error), "error");
});
session.addEventListener("reset", resetUi);

guarded(async () => {
  const restored = await session.restoreInputs();
  if (restored.profileFile) {
    elements.profileFileName.textContent = `${restored.profileFile.name} · restored locally`;
  }
  renderFonts();
  updateInputState();
});

function bindInputs() {
  elements.profileFile.addEventListener("change", () => guarded(async () => {
    const file = elements.profileFile.files?.[0];
    if (file) {
      await session.setProfileFile(file);
      elements.profileFileName.textContent = `${file.name} · ${formatBytes(file.size)}`;
    }
  }));
  elements.fontFiles.addEventListener("change", () => guarded(async () => {
    await session.addFontFiles([...elements.fontFiles.files]);
    elements.fontFiles.value = "";
  }));
  setupDrop(byId("profile-drop"), async (file) => {
    await session.setProfileFile(file);
    elements.profileFileName.textContent = `${file.name} · ${formatBytes(file.size)}`;
  }, (file) => file.name.toLowerCase().endsWith(".json"));
  setupDrop(elements.stageViewport, async (file) => {
    if (file.name.toLowerCase().endsWith(".json")) {
      await session.setProfileFile(file);
      elements.profileFileName.textContent = `${file.name} · ${formatBytes(file.size)}`;
    } else {
      await session.addFontFiles([file]);
    }
  }, (file) => /\.(?:json|ttf|otf)$/i.test(file.name));

  byId("provider-toggle").addEventListener("click", (event) => {
    const button = event.currentTarget;
    const expanded = button.getAttribute("aria-expanded") === "true";
    button.setAttribute("aria-expanded", String(!expanded));
    byId("provider-fields").hidden = expanded;
  });
  elements.region.addEventListener("change", () => {
    const region = elements.region.value;
    elements.masterdataBase.value = `https://cdn.emptysekai.com/masterdata/${region}/${elements.revision.value || "latest"}`;
    elements.assetBase.value = `https://cdn.emptysekai.com/assets/${region}`;
    updateProviderPreview();
  });
  elements.revision.addEventListener("change", () => {
    const region = elements.region.value;
    elements.masterdataBase.value = `https://cdn.emptysekai.com/masterdata/${region}/${elements.revision.value || "latest"}`;
    updateProviderPreview();
  });
  elements.masterdataBase.addEventListener("input", updateProviderPreview);
  elements.assetBase.addEventListener("input", updateProviderPreview);

  elements.createScene.addEventListener("click", () => guarded(async () => {
    stopPlayback();
    setRuntime("busy", "Building scene");
    await session.build(readConfig());
  }));
  elements.resetSession.addEventListener("click", () => guarded(() => session.reset()));
}

function bindNavigation() {
  elements.previousPage.addEventListener("click", () => guarded(() => session.switchPage(session.activePage - 1)));
  elements.nextPage.addEventListener("click", () => guarded(() => session.switchPage(session.activePage + 1)));
  elements.fitStage.addEventListener("click", () => {
    elements.stageShell.scrollIntoView({ block: "center", inline: "center", behavior: "smooth" });
    toast("Stage fitted to the available viewport.");
  });
  elements.toggleOverlays.addEventListener("click", () => {
    const enabled = elements.toggleOverlays.getAttribute("aria-pressed") !== "true";
    elements.toggleOverlays.setAttribute("aria-pressed", String(enabled));
    elements.toggleOverlays.textContent = enabled ? "Overlays" : "Overlays off";
    overlay.setEnabled(enabled);
  });
  elements.exportDump.addEventListener("click", exportDump);
  elements.copyDump.addEventListener("click", () => guarded(async () => {
    await navigator.clipboard.writeText(JSON.stringify(session.dump, null, 2));
    toast("Scene dump copied.");
  }));
  elements.clearEvents.addEventListener("click", () => {
    elements.eventLog.replaceChildren(muted("Interaction events will appear here."));
  });
}

function bindTimeline() {
  elements.timelinePlay.addEventListener("click", () => playing ? stopPlayback() : startPlayback());
  elements.timelineStepBack.addEventListener("click", () => moveTick(-1));
  elements.timelineStepForward.addEventListener("click", () => moveTick(1));
  elements.timelineRange.addEventListener("input", () => {
    stopPlayback();
    setTick(Number(elements.timelineRange.value));
  });
  elements.timelineFinal.addEventListener("click", () => {
    stopPlayback();
    setTick(Number(elements.timelineRange.max));
  });
}

function bindInspectorTabs() {
  for (const tab of document.querySelectorAll(".inspector-tab")) {
    tab.addEventListener("click", () => {
      for (const candidate of document.querySelectorAll(".inspector-tab")) {
        const active = candidate === tab;
        candidate.classList.toggle("active", active);
        candidate.setAttribute("aria-selected", String(active));
      }
      for (const panel of document.querySelectorAll(".inspector-panel")) {
        panel.classList.toggle("active", panel.id === `panel-${tab.dataset.panel}`);
      }
      if (tab.dataset.panel === "telemetry") updateStats();
    });
  }
}

function bindContextRecovery() {
  elements.canvas.addEventListener("webglcontextlost", (event) => {
    event.preventDefault();
    telemetry.contextLost();
    elements.contextBanner.hidden = false;
    elements.contextMessage.textContent = "WebGL context lost. Waiting for resource and atlas restoration.";
    setRuntime("error", "Context lost");
  });
  elements.canvas.addEventListener("webglcontextrestored", () => guarded(async () => {
    elements.contextMessage.textContent = "Context restored. Rebinding GPU resources.";
    if (typeof session.renderer?.restoreContext === "function") await session.renderer.restoreContext();
    session.draw();
    telemetry.contextRestored();
    window.setTimeout(() => { elements.contextBanner.hidden = true; }, 1400);
    setRuntime("ready", "Context restored");
    pushEvent("runtime", "context-restored", {});
  }));
  elements.loseContext.addEventListener("click", () => {
    if (!session.triggerContextLoss()) toast("WEBGL_lose_context is unavailable in this browser.", "warning");
  });
}

function bindKeyboard() {
  window.addEventListener("keydown", (event) => {
    if (event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement) return;
    if (event.code === "Space" && session.activeScene) {
      event.preventDefault();
      playing ? stopPlayback() : startPlayback();
    } else if (event.key === "ArrowLeft" && session.activeScene) {
      event.preventDefault();
      moveTick(event.shiftKey ? -60 : -1);
    } else if (event.key === "ArrowRight" && session.activeScene) {
      event.preventDefault();
      moveTick(event.shiftKey ? 60 : 1);
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s" && session.dump) {
      event.preventDefault();
      exportDump();
    }
  });
}

function startPlayback() {
  if (!session.activeScene) return;
  playing = true;
  animationEpoch = performance.now();
  animationStartTick = Number(elements.timelineRange.value);
  elements.timelinePlay.textContent = "❚❚";
  elements.timelinePlay.setAttribute("aria-label", "Pause timeline");
  requestAnimationFrame(animationFrame);
}

function stopPlayback() {
  playing = false;
  elements.timelinePlay.textContent = "▶";
  elements.timelinePlay.setAttribute("aria-label", "Play timeline");
}

function animationFrame(now) {
  if (!playing) return;
  telemetry.recordFrame(now);
  const maximum = Number(elements.timelineRange.max);
  let tick = animationStartTick + Math.floor((now - animationEpoch) * .06);
  if (tick > maximum) {
    if (elements.timelineLoop.checked) {
      tick %= maximum + 1;
      animationEpoch = now;
      animationStartTick = tick;
    } else {
      setTick(maximum);
      stopPlayback();
      return;
    }
  }
  if (!advancePending && tick !== Number(elements.timelineRange.value)) {
    advancePending = true;
    setTick(tick, true).finally(() => { advancePending = false; });
  }
  requestAnimationFrame(animationFrame);
}

function moveTick(delta) {
  stopPlayback();
  const current = Number(elements.timelineRange.value);
  setTick(Math.max(0, Math.min(Number(elements.timelineRange.max), current + delta)));
}

async function setTick(tick, advance = true) {
  const value = Math.max(0, Math.floor(Number(tick) || 0));
  elements.timelineRange.value = String(value);
  elements.timelineOutput.value = `${value} / ${elements.timelineRange.max}`;
  elements.timelineOutput.textContent = `${value} / ${elements.timelineRange.max}`;
  if (advance && session.activeScene) await session.advance(value);
}

function applyDump(dump, refreshInspector = true) {
  if (!dump) return;
  if (refreshInspector) inspector.setDump(dump);
  overlay.render(dump);
  elements.schemaPill.textContent = `schema ${dump.schema_major}.${dump.schema_minor}`;
  elements.timelineRange.value = String(dump.tick ?? 0);
  elements.timelineOutput.textContent = `${dump.tick ?? 0} / ${elements.timelineRange.max}`;
  updateStats();
}

function renderPages() {
  elements.pageTabs.replaceChildren();
  for (const [index, page] of session.pages.entries()) {
    const button = document.createElement("button");
    button.className = `page-tab${index === session.activePage ? " active" : ""}`;
    button.type = "button";
    button.role = "tab";
    button.setAttribute("aria-selected", String(index === session.activePage));
    button.textContent = String(page.sequence);
    button.title = `Page ${page.sequence}${page.cardId == null ? "" : ` · card ${page.cardId}`}`;
    button.addEventListener("click", () => guarded(() => session.switchPage(index)));
    elements.pageTabs.append(button);
  }
  elements.previousPage.disabled = session.activePage <= 0;
  elements.nextPage.disabled = session.activePage < 0 || session.activePage >= session.pages.length - 1;
}

function renderFonts() {
  elements.fontList.replaceChildren();
  elements.fontCount.textContent = String(session.fonts.length);
  if (session.fonts.length === 0) return elements.fontList.append(empty("Add the font files referenced by the profile."));
  for (const font of session.fonts) {
    const item = document.createElement("article");
    item.className = "font-entry";
    const name = document.createElement("strong");
    name.className = "font-family-input";
    name.textContent = font.file.name;
    const hash = document.createElement("small");
    hash.textContent = `${formatBytes(font.file.size)} · logical family aliases registered automatically`;
    const remove = document.createElement("button");
    remove.type = "button";
    remove.textContent = "×";
    remove.title = `Remove ${font.file.name}`;
    remove.setAttribute("aria-label", `Remove ${font.file.name}`);
    remove.addEventListener("click", () => session.removeFont(font.id));
    item.append(name, hash, remove);
    elements.fontList.append(item);
  }
}

function updateInputState() {
  elements.createScene.disabled = !session.profileFile || session.fonts.length === 0 || session.busy;
  if (!session.profileFile) setSessionMessage("Choose a profile JSON and at least one font file.");
  else if (session.fonts.length === 0) setSessionMessage("Add the font files used by the profile.");
  else setSessionMessage("Inputs ready. Build the semantic scene when providers are configured.");
}

function updateProviderPreview() {
  const masterdata = elements.masterdataBase.value.replace(/\/+$/, "");
  const assets = elements.assetBase.value.replace(/\/+$/, "");
  elements.providerPreview.textContent = `Tables: ${masterdata}/{table}.json\nAssets: ${assets}/{canonical-key}.png`;
}

function readConfig() {
  return {
    region: elements.region.value,
    revision: elements.revision.value,
    masterdataBase: elements.masterdataBase.value,
    assetBase: elements.assetBase.value,
    sdfBackend: elements.sdfBackend.value,
    persistence: elements.persistence.value,
    debugLevel: elements.debugLevel.value,
  };
}

function setBusy(busy, label = "", detail = "") {
  elements.stageBusy.hidden = !busy;
  elements.sessionProgress.hidden = !busy;
  elements.stageBusyLabel.textContent = label || "Preparing scene";
  elements.stageBusyDetail.textContent = detail || "Working in the renderer worker";
  elements.createScene.disabled = busy || !session.profileFile || session.fonts.length === 0;
  if (busy) {
    setRuntime("busy", label || "Working");
    setSessionMessage(`${label}${detail ? ` · ${detail}` : ""}`);
  }
}

function setRuntime(kind, message) {
  elements.runtimeDot.className = `status-dot ${kind}`;
  elements.runtimeStatus.textContent = message;
}

function enableSceneControls(enabled) {
  for (const control of [elements.timelinePlay, elements.timelineStepBack, elements.timelineStepForward, elements.timelineRange, elements.timelineFinal, elements.exportDump, elements.loseContext]) {
    control.disabled = !enabled;
  }
}

async function updateStats() {
  if (!session.renderer) return telemetry.render(null);
  try { telemetry.render(await session.stats()); } catch { telemetry.render(null); }
}

function startStatsPolling() {
  clearInterval(statsTimer);
  statsTimer = window.setInterval(updateStats, 1000);
  updateStats();
}

function selectLayerRegions(layerId) {
  const region = [
    ...(session.dump?.interaction_regions ?? []),
    ...(session.dump?.numeric_text_regions ?? []),
  ].find((candidate) => candidate.layer_id === layerId);
  inspector.selectRegion(region?.id ?? null);
}

async function emitInteractionEvent(event) {
  if (!event.quiet) {
    pushEvent(event.type, event.region?.role ?? "region", event.value);
    if (event.type === "copy") toast(`Copied ${event.value || "numeric text"}.`);
    if (event.type === "activate") toast("Navigation event emitted. The renderer did not navigate.");
  }
  inspector.selectLayer(event.region?.layer_id);
  inspector.selectRegion(event.region?.id ?? null);
}

async function activateRegion(region, target) {
  if (target.action === "set_tab" && target.control_id && target.value) {
    await session.setTab(String(target.control_id), String(target.value));
    pushEvent("control", region.role, {
      controlId: String(target.control_id),
      value: String(target.value),
    });
    inspector.selectLayer(region.layer_id);
    inspector.selectRegion(region.id);
    return;
  }
  await emitInteractionEvent({ type: "activate", region, value: target });
}

function scrollStep(controlId) {
  const control = (session.dump?.component_controls ?? [])
    .find((candidate) => candidate.id === controlId);
  const step = Number(control?.state?.step);
  return Number.isFinite(step) && step > 0 ? step : 1;
}

function pushEvent(type, role, value) {
  if (elements.eventLog.querySelector(".muted")) elements.eventLog.replaceChildren();
  const entry = document.createElement("span");
  entry.className = "event-entry";
  const time = document.createElement("span");
  time.className = "event-time";
  time.textContent = new Date().toLocaleTimeString("en-GB", { hour12: false });
  entry.append(time, document.createTextNode(`${type}:${role} ${compactJson(value)}`));
  elements.eventLog.prepend(entry);
  while (elements.eventLog.children.length > 6) elements.eventLog.lastElementChild.remove();
}

function exportDump() {
  if (!session.dump) return;
  const blob = new Blob([JSON.stringify(session.dump, null, 2)], { type: "application/json" });
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = `allium-scene-${session.dump.scene_id}-tick-${session.dump.tick}.json`;
  link.click();
  URL.revokeObjectURL(link.href);
  pushEvent("debug", "dump-export", { sceneId: session.dump.scene_id, tick: session.dump.tick });
}

async function resetUi() {
  stopPlayback();
  clearInterval(statsTimer);
  elements.profileFile.value = "";
  elements.fontFiles.value = "";
  elements.profileFileName.textContent = "Drop or choose a profile response";
  elements.stageEmpty.hidden = false;
  elements.contextBanner.hidden = true;
  elements.pageTabs.replaceChildren();
  inspector.setDump(null);
  overlay.render(null);
  telemetry.render(null);
  renderFonts();
  enableSceneControls(false);
  setRuntime("", "No active scene");
  elements.schemaPill.textContent = "schema --";
  setTick(0, false);
  updateInputState();
}

function setupDrop(host, accept, predicate) {
  host.addEventListener("dragover", (event) => {
    event.preventDefault();
    host.classList.add("dragging");
  });
  host.addEventListener("dragleave", () => host.classList.remove("dragging"));
  host.addEventListener("drop", (event) => {
    event.preventDefault();
    host.classList.remove("dragging");
    const file = [...event.dataTransfer.files].find(predicate);
    if (file) guarded(() => accept(file));
    else toast("The dropped file is not supported here.", "warning");
  });
}

async function guarded(action) {
  try { return await action(); }
  catch (error) {
    const message = errorMessage(error);
    setSessionMessage(message, "error");
    toast(message, "error");
    console.error(error);
    return undefined;
  }
}

function toast(message, kind = "") {
  const item = document.createElement("div");
  item.className = `toast ${kind}`.trim();
  item.textContent = message;
  elements.toastRegion.append(item);
  window.setTimeout(() => item.remove(), 4200);
}

function setSessionMessage(message, kind = "") {
  elements.sessionMessage.textContent = message;
  elements.sessionMessage.className = `session-message ${kind}`.trim();
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error ?? "Unknown renderer error");
}

function compactJson(value) {
  if (value === undefined || value === null) return "";
  const json = JSON.stringify(value);
  return json.length > 90 ? `${json.slice(0, 87)}…` : json;
}

function formatBytes(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1024 ** 2).toFixed(1)} MiB`;
}

function empty(text) {
  const paragraph = document.createElement("p");
  paragraph.className = "empty-copy";
  paragraph.textContent = text;
  return paragraph;
}

function muted(text) {
  const span = document.createElement("span");
  span.className = "muted";
  span.textContent = text;
  return span;
}
