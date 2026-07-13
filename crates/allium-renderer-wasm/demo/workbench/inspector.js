const KIND_LABELS = {
  text: "Text",
  shape: "Shape",
  card_member: "Card member",
  stamp: "Stamp",
  other: "Decoration",
  bonds_honor: "Bonds honor",
  honor: "Honor",
  collection: "Collection",
  general: "General",
  stand_member: "Standing member",
  general_background: "General background",
  story_background: "Story background",
};

const MAX_GROUP_ROWS = 40;

export class SceneInspector {
  constructor(elements, actions) {
    this.elements = elements;
    this.actions = actions;
    this.dump = null;
    this.selectedLayerId = null;
    this.detail = "source";
    this.query = "";
    this.layerType = "all";
    this.groupLayerView = false;
    this.controlQuery = "";
    this.selectedRegionId = null;
    this.dumpQuery = "";
    this.dumpPath = "/";
    this.dumpRaw = false;
    this.bind();
  }

  bind() {
    this.elements.layerSearch.addEventListener("input", () => {
      this.query = this.elements.layerSearch.value.trim().toLowerCase();
      this.renderLayers();
    });
    this.elements.layerType.addEventListener("change", () => {
      this.layerType = this.elements.layerType.value;
      this.renderLayers();
    });
    this.elements.groupLayers.addEventListener("change", () => {
      this.groupLayerView = this.elements.groupLayers.checked;
      this.renderLayers();
    });
    this.elements.controlSearch.addEventListener("input", () => {
      this.controlQuery = this.elements.controlSearch.value.trim().toLowerCase();
      this.renderControls();
      this.renderInteractions();
    });
    this.elements.clearInteraction.addEventListener("click", () => this.selectRegion(null));
    this.elements.dumpSearch.addEventListener("input", () => {
      this.dumpQuery = this.elements.dumpSearch.value.trim();
      this.renderDump();
    });
    this.elements.dumpPath.addEventListener("keydown", (event) => {
      if (event.key !== "Enter") return;
      event.preventDefault();
      this.navigateDump(this.elements.dumpPath.value);
    });
    this.elements.dumpUp.addEventListener("click", () => this.navigateDump(parentJsonPath(this.dumpPath)));
    this.elements.dumpViewMode.addEventListener("click", () => {
      this.dumpRaw = !this.dumpRaw;
      this.elements.dumpViewMode.textContent = this.dumpRaw ? "Tree view" : "Raw JSON";
      this.elements.dumpViewMode.setAttribute("aria-pressed", String(this.dumpRaw));
      this.renderDump();
    });
    this.elements.showAll.addEventListener("click", () => this.applyAll(true));
    this.elements.hideAll.addEventListener("click", () => this.applyAll(false));
    for (const tab of this.elements.detailTabs) {
      tab.addEventListener("click", () => {
        this.detail = tab.dataset.detail;
        for (const candidate of this.elements.detailTabs) candidate.classList.toggle("active", candidate === tab);
        this.renderDetail();
      });
    }
  }

  setDump(dump) {
    this.dump = dump;
    if (!dump?.layers?.some((layer) => layer.layer_id === this.selectedLayerId)) {
      this.selectedLayerId = dump?.layers?.[0]?.layer_id ?? null;
    }
    const regions = [...(dump?.interaction_regions ?? []), ...(dump?.numeric_text_regions ?? [])];
    if (!regions.some((region) => region.id === this.selectedRegionId)) this.selectedRegionId = null;
    this.elements.clearInteraction.disabled = !this.selectedRegionId;
    this.elements.showAll.disabled = !dump?.layers?.length;
    this.elements.hideAll.disabled = !dump?.layers?.length;
    this.syncLayerTypes();
    if (dump && jsonAtPath(dump, this.dumpPath) === undefined) this.dumpPath = "/";
    this.renderLayers();
    this.renderDetail();
    this.renderControls();
    this.renderInteractions();
    this.renderDump();
  }

  selectLayer(layerId) {
    this.selectedLayerId = layerId;
    this.renderLayers();
    this.renderDetail();
    this.actions.onLayerSelected?.(layerId);
  }

  selectRegion(regionId) {
    this.selectedRegionId = regionId;
    this.elements.clearInteraction.disabled = !regionId;
    this.renderInteractions();
    this.actions.onInteractionSelected?.(regionId);
  }

  navigateDump(path) {
    const normalized = normalizeJsonPath(path);
    if (!this.dump || jsonAtPath(this.dump, normalized) === undefined) return;
    this.dumpPath = normalized;
    this.dumpQuery = "";
    this.elements.dumpSearch.value = "";
    this.renderDump();
  }

  syncLayerTypes() {
    const kinds = [...new Set((this.dump?.layers ?? []).map((layer) => layer.authored_kind).filter(Boolean))];
    const current = kinds.includes(this.layerType) ? this.layerType : "all";
    this.elements.layerType.replaceChildren(option("all", "All types"));
    for (const kind of kinds) this.elements.layerType.append(option(kind, KIND_LABELS[kind] ?? humanize(kind)));
    this.layerType = current;
    this.elements.layerType.value = current;
  }

  renderLayers() {
    const host = this.elements.layerTree;
    host.replaceChildren();
    if (!this.dump?.layers?.length) return host.append(empty("No scene data."));
    const parentById = new Map(this.dump.layer_tree ?? []);
    const layers = filterLayers(this.dump.layers, this.query, this.layerType);
    if (layers.length === 0) return host.append(empty("No authored layer matches this filter."));

    if (this.groupLayerView) {
      for (const [kind, grouped] of groupLayers(layers)) {
        host.append(collapsibleGroup(KIND_LABELS[kind] ?? humanize(kind), grouped.length, true, (body) => {
          for (const layer of grouped) body.append(this.layerRow(layer, parentById, true));
        }));
      }
      return;
    }
    for (const layer of layers) host.append(this.layerRow(layer, parentById, false));
  }

  layerRow(layer, parentById, grouped) {
      const row = document.createElement("div");
      row.className = `layer-row${layer.layer_id === this.selectedLayerId ? " selected" : ""}${layer.render_mask ? "" : " hidden-layer"}`;
      row.dataset.layerId = layer.layer_id;
      row.style.paddingLeft = `${9 + (grouped ? 0 : depthOf(layer.layer_id, parentById) * 12)}px`;
      row.tabIndex = 0;
      row.setAttribute("role", "treeitem");
      row.setAttribute("aria-selected", String(layer.layer_id === this.selectedLayerId));

      const eye = document.createElement("button");
      eye.className = "layer-eye";
      eye.type = "button";
      eye.textContent = layer.render_mask ? "◉" : "○";
      eye.title = "Toggle layer visibility. Shift-click applies to the authored subtree.";
      eye.setAttribute("aria-label", `${layer.render_mask ? "Hide" : "Show"} ${layerName(layer)}`);
      eye.addEventListener("click", (event) => {
        event.stopPropagation();
        const visible = !layer.render_mask;
        if (event.shiftKey) {
          const ids = subtreeLayerIds(layer.layer_id, this.dump.layer_table ?? []);
          this.actions.onLayerMasks?.(ids.map((layerId) => ({ layerId, visible })));
        } else {
          this.actions.onLayerVisible?.(layer.layer_id, visible);
        }
      });

      const label = document.createElement("div");
      label.className = "layer-label";
      const strong = document.createElement("strong");
      const kindClass = layer.kind === "text" || layer.kind === "shape" ? layer.kind : "image";
      strong.append(mark(kindClass), document.createTextNode(layerName(layer)));
      const small = document.createElement("small");
      small.textContent = `${layer.layer_id} · game layer ${layer.game_layer}`;
      label.append(strong, small);

      const index = document.createElement("span");
      index.className = "layer-index";
      index.textContent = `#${layer.authored_index}`;
      row.append(eye, label, index);
      row.addEventListener("click", () => this.selectLayer(layer.layer_id));
      row.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          this.selectLayer(layer.layer_id);
        }
      });
      return row;
  }

  renderDetail() {
    const host = this.elements.layerDetail;
    host.replaceChildren();
    const layer = this.dump?.layers?.find((candidate) => candidate.layer_id === this.selectedLayerId);
    if (!layer) return host.append(empty("Select a game-authored layer to inspect it."));

    if (this.detail === "source") {
      const source = document.createElement("div");
      source.className = "source-block";
      source.textContent = layer.source_content || "No source content.";
      host.append(source, propertyGrid({
        "Authored type": KIND_LABELS[layer.authored_kind] ?? layer.authored_kind,
        "Stable layer ID": layer.layer_id,
        "Parent ID": layer.parent_id ?? "root",
        "Authored visible": layer.authored_visible,
        "Render mask": layer.render_mask,
        "Mask override": serializeValue(layer.mask_override),
        "Dynamic status": layer.dynamic?.status ?? "static",
      }));
      return;
    }
    if (this.detail === "parameters") {
      const parameters = layer.resolved_parameters ?? {};
      return host.append(propertyGrid(Object.fromEntries(
        Object.entries(parameters).map(([key, value]) => [key, parameterValue(value)]),
      )));
    }
    if (this.detail === "geometry") {
      return host.append(propertyGrid({
        Bounds: formatRect(layer.bounds),
        Quad: formatPoints(layer.quad),
        Matrix: formatMatrix(layer.matrix),
        "Hit geometry": formatPoints(layer.hit_geometry),
        "Line indent": layer.line_indent ? JSON.stringify(layer.line_indent, null, 2) : "none",
        Glyphs: layer.glyphs?.length ?? 0,
      }));
    }
    const commands = layer.commands ?? [];
    if (commands.length === 0) return host.append(empty("This layer has no semantic commands."));
    for (const command of commands) host.append(commandCard(command));
  }

  renderControls() {
    const host = this.elements.controls;
    host.replaceChildren();
    const controls = filterControls(this.dump?.component_controls ?? [], this.controlQuery);
    if (controls.length === 0) return host.append(empty("No component controls in the active scene."));
    for (const [group, grouped] of groupControls(controls)) {
      host.append(collapsibleGroup(group, grouped.length, true, (body) => {
        for (const control of grouped.slice(0, MAX_GROUP_ROWS)) body.append(this.controlCard(control));
        appendBoundedNotice(body, grouped.length);
      }));
    }
  }

  controlCard(control) {
      const card = document.createElement("article");
      card.className = "control-card";
      const header = document.createElement("header");
      const title = document.createElement("strong");
      title.textContent = humanize(control.role ?? control.state?.kind ?? "Control");
      const code = document.createElement("code");
      code.textContent = control.id;
      header.append(title, code);
      card.append(header);
      if (control.state?.kind === "tabs") card.append(this.tabControl(control));
      else if (control.state?.kind === "scroll") card.append(this.scrollControl(control));
      else card.append(jsonBlock(control.state ?? control));
      return card;
  }

  tabControl(control) {
    const group = document.createElement("div");
    group.className = "segmented";
    for (const option of control.state.options ?? []) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = humanize(option);
      button.classList.toggle("active", option === control.state.active);
      button.addEventListener("click", () => this.actions.onTab?.(control.id, option));
      group.append(button);
    }
    return group;
  }

  scrollControl(control) {
    const group = document.createElement("div");
    group.className = "scroll-control";
    const previous = compactButton("−", () => this.actions.onScrollBy?.(control.id, -Number(control.state.step || 1)));
    const range = document.createElement("input");
    range.type = "range";
    range.min = control.state.min;
    range.max = control.state.max;
    range.step = control.state.step || 1;
    range.value = control.state.offset;
    range.setAttribute("aria-label", `${humanize(control.role)} offset`);
    range.addEventListener("change", () => this.actions.onScrollOffset?.(control.id, Number(range.value)));
    const next = compactButton("+", () => this.actions.onScrollBy?.(control.id, Number(control.state.step || 1)));
    group.append(previous, range, next);
    group.addEventListener("wheel", (event) => {
      event.preventDefault();
      this.actions.onScrollBy?.(control.id, Math.sign(event.deltaY) * Number(control.state.step || 1));
    }, { passive: false });
    return group;
  }

  renderInteractions() {
    const host = this.elements.interactions;
    host.replaceChildren();
    const regions = filterRegions([
      ...(this.dump?.interaction_regions ?? []),
      ...(this.dump?.numeric_text_regions ?? []),
    ], this.controlQuery);
    if (regions.length === 0) return host.append(empty("No visible interaction regions."));
    for (const [group, grouped] of groupRegions(regions)) {
      host.append(collapsibleGroup(group, grouped.length, Boolean(this.controlQuery), (body) => {
        for (const region of grouped.slice(0, MAX_GROUP_ROWS)) body.append(this.interactionCard(region));
        appendBoundedNotice(body, grouped.length);
      }));
    }
  }

  interactionCard(region) {
      const card = document.createElement("article");
      card.className = `interaction-card${region.id === this.selectedRegionId ? " selected" : ""}`;
      const header = document.createElement("header");
      const title = document.createElement("strong");
      title.textContent = humanize(region.role);
      const code = document.createElement("code");
      code.textContent = region.id;
      header.append(title, code);
      const capabilities = document.createElement("div");
      capabilities.className = "capabilities";
      for (const capability of region.capabilities ?? []) {
        const chip = document.createElement("span");
        chip.className = "capability";
        chip.textContent = capability;
        capabilities.append(chip);
      }
      const focus = document.createElement("button");
      focus.className = "button ghost small";
      focus.type = "button";
      focus.textContent = region.id === this.selectedRegionId ? "Focused on stage" : "Focus region";
      focus.addEventListener("click", () => this.selectRegion(region.id));
      if (region.role === "numeric_run") {
        const button = document.createElement("button");
        button.className = "button ghost small";
        button.type = "button";
        button.textContent = `Copy ${region.resolved_data?.text ?? "number"}`;
        button.addEventListener("click", () => this.actions.onInteraction?.(region, "copy"));
        const actions = document.createElement("div");
        actions.className = "interaction-actions";
        actions.append(focus, button);
        card.append(header, capabilities, actions);
      } else {
        const button = document.createElement("button");
        button.className = "button ghost small";
        button.type = "button";
        button.textContent = "Emit navigation event";
        button.addEventListener("click", () => this.actions.onInteraction?.(region, "activate"));
        const actions = document.createElement("div");
        actions.className = "interaction-actions";
        actions.append(focus, button);
        card.append(header, capabilities, actions);
      }
      return card;
  }

  renderDump() {
    const dump = this.dump;
    if (!dump) {
      this.elements.dumpPreview.textContent = "Build a scene to inspect its privacy-safe semantic dump.";
      this.elements.dumpSize.textContent = "No dump";
      this.elements.copyDump.disabled = true;
      this.elements.dumpUp.disabled = true;
      this.elements.dumpPath.value = "/";
      return;
    }
    const json = JSON.stringify(dump, null, 2);
    const host = this.elements.dumpPreview;
    host.replaceChildren();
    this.elements.dumpPath.value = this.dumpPath;
    this.elements.dumpUp.disabled = this.dumpPath === "/";
    if (this.dumpRaw) {
      const raw = document.createElement("pre");
      raw.className = "dump-raw";
      raw.textContent = json;
      host.append(raw);
    } else if (this.dumpQuery) {
      const results = searchJson(dump, this.dumpQuery, 100);
      if (results.length === 0) host.append(empty("No dump path or value matches this search."));
      for (const result of results) {
        const button = document.createElement("button");
        button.className = "dump-result";
        button.type = "button";
        button.append(codeText(result.path), document.createTextNode(result.preview));
        button.addEventListener("click", () => this.navigateDump(result.path));
        host.append(button);
      }
    } else {
      host.append(jsonTree(jsonAtPath(dump, this.dumpPath), this.dumpPath, (path) => this.navigateDump(path)));
    }
    this.elements.dumpSize.textContent = `${formatBytes(new TextEncoder().encode(json).byteLength)} · ${dump.layers?.length ?? 0} layers · ${countCommands(dump)} commands`;
    this.elements.copyDump.disabled = false;
  }

  async applyAll(visible) {
    const overrides = (this.dump?.layers ?? []).map((layer) => ({ layerId: layer.layer_id, visible }));
    await this.actions.onLayerMasks?.(overrides);
  }
}

export function filterLayers(layers, query = "", type = "all") {
  return layers.filter((layer) => (type === "all" || layer.authored_kind === type) && layerMatches(layer, query));
}

export function groupLayers(layers) {
  return groupBy(layers, (layer) => layer.authored_kind ?? "other");
}

export function filterControls(controls, query = "") {
  if (!query) return controls;
  return controls.filter((control) => searchable(control).includes(query.toLowerCase()));
}

export function groupControls(controls) {
  return groupBy(controls, (control) => control.state?.kind === "tabs" ? "Tabs" : control.state?.kind === "scroll" ? "Scroll" : "Other controls");
}

export function groupRegions(regions) {
  return groupBy(regions, (region) => {
    if (region.role === "numeric_run") return "Numeric text";
    if ((region.control_bindings ?? []).length || /tab|scroll/i.test(region.role ?? "")) return "Component controls";
    if ((region.capabilities ?? []).some((value) => /navigate|activate|open/i.test(value)) || /card|honor|story|music|character|event/i.test(region.role ?? "")) return "Navigation";
    return "Other regions";
  });
}

function filterRegions(regions, query) {
  if (!query) return regions;
  return regions.filter((region) => searchable(region).includes(query.toLowerCase()));
}

function layerMatches(layer, query) {
  if (!query) return true;
  return [layer.authored_kind, layer.kind, layer.layer_id, layer.source_content, layer.game_layer]
    .some((value) => String(value ?? "").toLowerCase().includes(query));
}

function searchable(value) {
  return JSON.stringify(value).toLowerCase();
}

function groupBy(values, keyOf) {
  const groups = new Map();
  for (const value of values) {
    const key = keyOf(value);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(value);
  }
  return groups;
}

function layerName(layer) {
  return `${KIND_LABELS[layer.authored_kind] ?? humanize(layer.authored_kind)} ${layer.authored_index}`;
}

function depthOf(id, parentById) {
  let depth = 0;
  let cursor = parentById.get(id);
  const seen = new Set();
  while (cursor && !seen.has(cursor) && depth < 8) {
    seen.add(cursor);
    depth += 1;
    cursor = parentById.get(cursor);
  }
  return depth;
}

function subtreeLayerIds(layerId, table) {
  const root = table.find((entry) => entry.layer_id === layerId);
  if (!root) return [layerId];
  return table
    .filter((entry) => entry.slot >= root.subtree_start && entry.slot < root.subtree_end)
    .map((entry) => entry.layer_id);
}

function mark(kind) {
  const value = document.createElement("span");
  value.className = `type-mark ${kind}`;
  return value;
}

function empty(text) {
  const paragraph = document.createElement("p");
  paragraph.className = "empty-copy padded";
  paragraph.textContent = text;
  return paragraph;
}

function propertyGrid(properties) {
  const grid = document.createElement("dl");
  grid.className = "property-grid";
  for (const [name, value] of Object.entries(properties)) {
    const key = document.createElement("dt");
    const content = document.createElement("dd");
    key.textContent = name;
    content.textContent = serializeValue(value);
    grid.append(key, content);
  }
  return grid;
}

function commandCard(command) {
  const card = document.createElement("article");
  card.className = "command-card";
  const header = document.createElement("header");
  const role = document.createElement("strong");
  const id = document.createElement("code");
  role.textContent = humanize(command.role ?? command.payload?.kind ?? "command");
  id.textContent = command.id ?? "unknown";
  header.append(role, id);
  card.append(header, propertyGrid({
    Bounds: formatRect(command.bounds),
    Matrix: formatMatrix(command.matrix),
    "Blend mode": command.blend_mode,
    Clip: command.clip ? formatPoints(command.clip) : "none",
    Payload: JSON.stringify(command.payload, null, 2),
    Metadata: JSON.stringify(command.metadata ?? {}, null, 2),
  }));
  return card;
}

function jsonBlock(value) {
  const block = document.createElement("div");
  block.className = "json-block";
  block.textContent = JSON.stringify(value, null, 2);
  return block;
}

function compactButton(label, action) {
  const button = document.createElement("button");
  button.className = "icon-button";
  button.type = "button";
  button.textContent = label;
  button.addEventListener("click", action);
  return button;
}

function option(value, label) {
  const entry = document.createElement("option");
  entry.value = value;
  entry.textContent = label;
  return entry;
}

function collapsibleGroup(label, count, open, renderBody) {
  const details = document.createElement("details");
  details.className = "inspector-group";
  details.open = open;
  const summary = document.createElement("summary");
  const title = document.createElement("span");
  title.textContent = label;
  const badge = document.createElement("span");
  badge.className = "count";
  badge.textContent = String(count);
  summary.append(title, badge);
  const body = document.createElement("div");
  body.className = "inspector-group-body";
  renderBody(body);
  details.append(summary, body);
  return details;
}

function appendBoundedNotice(host, count) {
  if (count <= MAX_GROUP_ROWS) return;
  host.append(empty(`Showing the first ${MAX_GROUP_ROWS} of ${count}. Refine the search to inspect the remainder.`));
}

function codeText(value) {
  const code = document.createElement("code");
  code.textContent = value;
  return code;
}

function jsonTree(value, path, navigate) {
  const root = document.createElement("div");
  root.className = "json-tree";
  if (!isContainer(value)) {
    root.append(jsonLeaf("value", value));
    return root;
  }
  const entries = Object.entries(value);
  for (const [key, child] of entries.slice(0, 100)) {
    const childPath = joinJsonPath(path, key);
    if (!isContainer(child)) {
      root.append(jsonLeaf(key, child));
      continue;
    }
    const row = document.createElement("details");
    row.className = "json-branch";
    const summary = document.createElement("summary");
    summary.append(codeText(key), document.createTextNode(Array.isArray(child) ? `Array · ${child.length}` : `Object · ${Object.keys(child).length}`));
    const open = document.createElement("button");
    open.type = "button";
    open.className = "button ghost small";
    open.textContent = "Open path";
    open.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      navigate(childPath);
    });
    summary.append(open);
    row.append(summary);
    row.addEventListener("toggle", () => {
      if (!row.open || row.children.length > 1) return;
      const preview = document.createElement("pre");
      preview.className = "json-branch-preview";
      preview.textContent = JSON.stringify(child, null, 2).slice(0, 4000);
      row.append(preview);
    });
    root.append(row);
  }
  if (entries.length > 100) root.append(empty(`Showing 100 of ${entries.length} entries. Open a child path or use search.`));
  return root;
}

function jsonLeaf(key, value) {
  const row = document.createElement("div");
  row.className = "json-leaf";
  row.append(codeText(key), document.createTextNode(serializeValue(value)));
  return row;
}

function isContainer(value) {
  return value !== null && typeof value === "object";
}

export function jsonAtPath(root, path) {
  const normalized = normalizeJsonPath(path);
  if (normalized === "/") return root;
  let cursor = root;
  for (const token of normalized.slice(1).split("/").map(unescapeJsonPointer)) {
    if (cursor === null || typeof cursor !== "object" || !(token in cursor)) return undefined;
    cursor = cursor[token];
  }
  return cursor;
}

export function searchJson(root, query, limit = 100) {
  const needle = String(query ?? "").trim().toLowerCase();
  if (!needle) return [];
  const results = [];
  const visit = (value, path) => {
    if (results.length >= limit) return;
    if (isContainer(value)) {
      for (const [key, child] of Object.entries(value)) {
        const childPath = joinJsonPath(path, key);
        if (key.toLowerCase().includes(needle) && results.length < limit) {
          results.push({ path: childPath, preview: compactPreview(child) });
        }
        visit(child, childPath);
        if (results.length >= limit) return;
      }
      return;
    }
    if (String(value).toLowerCase().includes(needle)) results.push({ path, preview: compactPreview(value) });
  };
  visit(root, "/");
  return results;
}

function compactPreview(value) {
  const serialized = serializeValue(value).replace(/\s+/g, " ");
  return serialized.length > 120 ? `${serialized.slice(0, 117)}…` : serialized;
}

function normalizeJsonPath(path) {
  const trimmed = String(path ?? "/").trim();
  if (!trimmed || trimmed === "/") return "/";
  return `/${trimmed.replace(/^\/+|\/+$/g, "")}`;
}

function parentJsonPath(path) {
  const normalized = normalizeJsonPath(path);
  if (normalized === "/") return "/";
  const parent = normalized.slice(0, normalized.lastIndexOf("/"));
  return parent || "/";
}

function joinJsonPath(path, key) {
  const token = String(key).replace(/~/g, "~0").replace(/\//g, "~1");
  return path === "/" ? `/${token}` : `${path}/${token}`;
}

function unescapeJsonPointer(token) {
  return token.replace(/~1/g, "/").replace(/~0/g, "~");
}

function parameterValue(value) {
  if (value && typeof value === "object" && "value" in value) return value.value;
  return value;
}

function formatRect(rect) {
  if (!rect) return "none";
  return `x ${number(rect.x)} · y ${number(rect.y)} · w ${number(rect.width)} · h ${number(rect.height)}`;
}

function formatMatrix(matrix) {
  return Array.isArray(matrix) ? `[${matrix.map(number).join(", ")}]` : "none";
}

function formatPoints(points) {
  return Array.isArray(points) ? points.map((point) => `(${number(point[0])}, ${number(point[1])})`).join(" ") : "none";
}

function number(value) {
  return Number.isFinite(Number(value)) ? Number(value).toFixed(2).replace(/\.00$/, "") : "--";
}

function serializeValue(value) {
  if (value === null || value === undefined) return "none";
  if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") return String(value);
  return JSON.stringify(value, null, 2);
}

function humanize(value) {
  return String(value ?? "unknown").replace(/[_-]+/g, " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function countCommands(dump) {
  return (dump.layers ?? []).reduce((sum, layer) => sum + (layer.commands?.length ?? 0), 0);
}

function formatBytes(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1024 ** 2).toFixed(1)} MiB`;
}
