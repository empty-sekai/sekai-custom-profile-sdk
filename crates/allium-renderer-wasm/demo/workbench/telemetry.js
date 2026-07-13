export class TelemetryPanel {
  constructor(host, fpsOutput) {
    this.host = host;
    this.fpsOutput = fpsOutput;
    this.samples = [];
    this.lastFrame = null;
    this.contextLosses = 0;
    this.restoreMs = null;
    this.lossStarted = null;
    this.lastStats = null;
  }

  recordFrame(now = performance.now()) {
    if (this.lastFrame !== null) {
      const duration = now - this.lastFrame;
      if (duration > 0 && duration < 1000) {
        this.samples.push(duration);
        if (this.samples.length > 240) this.samples.shift();
      }
    }
    this.lastFrame = now;
    const mean = average(this.samples.slice(-60));
    this.fpsOutput.textContent = mean ? `${(1000 / mean).toFixed(0)} fps` : "Idle";
  }

  contextLost() {
    this.contextLosses += 1;
    this.lossStarted = performance.now();
    this.render(this.lastStats);
  }

  contextRestored() {
    this.restoreMs = this.lossStarted === null ? null : performance.now() - this.lossStarted;
    this.lossStarted = null;
    this.render(this.lastStats);
  }

  render(stats) {
    this.lastStats = stats;
    this.host.replaceChildren();
    const worker = stats?.renderer?.worker ?? {};
    const resources = stats?.renderer?.resources ?? {};
    const scene = stats?.scene ?? {};
    const lastGpu = scene.lastGpu ?? {};
    const atlas = scene.atlas ?? {};
    const atlasCache = atlas.cache ?? {};
    const runtimeTelemetry = scene.telemetry ?? {};
    const lastTimings = runtimeTelemetry.last?.timingsMs ?? {};
    const recovery = scene.recovery ?? {};
    const core = stats?.dump ?? {};
    const recent = this.samples.slice(-120);
    const frameMean = average(recent);
    const groups = [
      {
        title: "Frame and GPU",
        metrics: [
          metric("Frame mean", frameMean, "ms", frameMean !== null && frameMean < 16.67 ? "good" : ""),
          metric("Frame p95", percentile(recent, .95), "ms"),
          metric("GPU timer", lastTimings.gpuTime, "ms"),
          metric("Submit CPU", lastTimings.gpuSubmitCpu, "ms"),
          metric("Draw calls", lastGpu.drawCalls),
          metric("Vertex bytes", lastGpu.vertexBytes, "B"),
        ],
      },
      {
        title: "Worker and WASM",
        metrics: [
          metric("Requests", worker.requests),
          metric("WASM time", worker.wasmMs, "ms"),
          metric("Bridge bytes", worker.bridgeBytes, "B"),
          metric("Failures", worker.failures, "", Number(worker.failures) === 0 ? "good" : "warn"),
          metric("Scenes", worker.scenes),
          metric("Fonts", worker.fonts),
        ],
      },
      {
        title: "Caches",
        metrics: [
          metric("Image entries", resources.entries),
          metric("Image bytes", resources.bytes, "B"),
          metric("Image hits", resources.hits),
          metric("Evictions", resources.evictions),
          metric("Atlas pages", atlas.pages),
          metric("Atlas glyphs", atlas.glyphs),
          metric("Session hits", atlasCache.sessionHits),
          metric("Persistent hits", atlasCache.persistentHits),
          metric("Persistent misses", atlasCache.persistentMisses),
          metric("Page evictions", atlasCache.pageEvictions),
        ],
      },
      {
        title: "Semantic core",
        metrics: [
          metric("Dynamic evals", core.dynamic_evaluations),
          metric("Dirty layers", core.dirty_layers),
          metric("Layout runs", core.layout_runs),
          metric("Command rebuilds", core.command_rebuilds, "", Number(core.command_rebuilds) === 0 ? "good" : "warn"),
          metric("Atlas generations", core.atlas_generations, "", Number(core.atlas_generations) === 0 ? "good" : "warn"),
          metric("Serialized", core.serialized_bytes, "B"),
        ],
      },
      {
        title: "Recovery",
        metrics: [
          metric("Context losses", valueOr(recovery.contextLosses, this.contextLosses), "", Number(valueOr(recovery.contextLosses, this.contextLosses)) === 0 ? "good" : "warn"),
          metric("Context restores", recovery.contextRestores),
          metric("Restore failures", recovery.restoreFailures, "", Number(recovery.restoreFailures) === 0 ? "good" : "warn"),
          metric("Restore time", valueOr(recovery.lastRestoreMs, this.restoreMs), "ms"),
          metric("Protocol", worker.protocol),
          metric("Runtime frames", scene.frames),
          metric("Trace samples", runtimeTelemetry.samples?.length, "/ 240"),
          metric("UI samples", this.samples.length, "/ 240"),
        ],
      },
    ];
    for (const group of groups) this.host.append(metricGroup(group));
  }
}

function metric(label, value, unit = "", tone = "") {
  return { label, value: normalize(value), unit, tone };
}

function metricGroup(group) {
  const section = document.createElement("section");
  section.className = "metric-group";
  const title = document.createElement("h3");
  title.textContent = group.title;
  const grid = document.createElement("div");
  grid.className = "metric-grid";
  for (const item of group.metrics) {
    const cell = document.createElement("div");
    cell.className = `metric ${item.tone}`.trim();
    const label = document.createElement("span");
    const value = document.createElement("strong");
    const unit = document.createElement("small");
    label.textContent = item.label;
    value.textContent = formatValue(item.value, item.unit);
    unit.textContent = item.value === null ? "unavailable" : item.unit && !unitIncluded(item.unit) ? item.unit : "";
    cell.append(label, value, unit);
    grid.append(cell);
  }
  section.append(title, grid);
  return section;
}

function normalize(value) {
  if (value === undefined || value === null || value === "") return null;
  return value;
}

function formatValue(value, unit) {
  if (value === null) return "—";
  if (typeof value === "string") return value;
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return "—";
  if (unit === "B") return formatBytes(numeric);
  if (unit === "ms") return numeric < 10 ? numeric.toFixed(2) : numeric.toFixed(1);
  return numeric.toLocaleString("en-US", { maximumFractionDigits: 1 });
}

function unitIncluded(unit) {
  return unit === "B";
}

function formatBytes(bytes) {
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1024 ** 2).toFixed(1)} MiB`;
}

function average(values) {
  return values.length ? values.reduce((sum, value) => sum + value, 0) / values.length : null;
}

function percentile(values, ratio) {
  if (!values.length) return null;
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * ratio))];
}

function valueOr(...values) {
  return values.find((value) => value !== undefined && value !== null) ?? null;
}
