const SVG_NS = "http://www.w3.org/2000/svg";

export class InteractionOverlay {
  constructor(svg, hoverCard, actions) {
    this.svg = svg;
    this.hoverCard = hoverCard;
    this.actions = actions;
    this.enabled = false;
    this.regions = [];
    this.selectedId = null;
  }

  setEnabled(enabled) {
    this.enabled = Boolean(enabled);
    this.updateMode();
  }

  render(dump) {
    this.regions = [
      ...(dump?.interaction_regions ?? []),
      ...(dump?.numeric_text_regions ?? []),
    ].sort((left, right) => regionArea(right) - regionArea(left));
    this.svg.replaceChildren();
    for (const region of this.regions) {
      const points = validPoints(region.hit_geometry) ?? boundsPoints(region.bounds);
      if (!points) continue;
      const polygon = document.createElementNS(SVG_NS, "polygon");
      polygon.setAttribute("points", points.map(([x, y]) => `${x},${y}`).join(" "));
      polygon.dataset.regionId = region.id;
      polygon.classList.toggle("numeric", region.role === "numeric_run");
      polygon.classList.toggle("hidden-region", region.render_mask === false);
      polygon.classList.toggle("selected", region.id === this.selectedId);
      polygon.setAttribute("tabindex", region.render_mask === false ? "-1" : "0");
      polygon.setAttribute("role", "button");
      polygon.setAttribute("aria-label", regionLabel(region));
      polygon.addEventListener("click", () => this.activate(region));
      polygon.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          this.activate(region);
        }
      });
      polygon.addEventListener("pointerenter", (event) => this.showHover(region, event));
      polygon.addEventListener("pointermove", (event) => this.positionHover(event));
      polygon.addEventListener("pointerleave", () => this.hideHover());
      polygon.addEventListener("wheel", (event) => {
        const binding = scrollBinding(region.control_bindings);
        if (!binding) return;
        event.preventDefault();
        this.actions.onScroll?.(binding.control_id, Math.sign(event.deltaY), region);
      }, { passive: false });
      this.svg.append(polygon);
    }
    this.updateMode();
  }

  select(regionId) {
    this.selectedId = regionId;
    for (const polygon of this.svg.querySelectorAll("polygon")) {
      polygon.classList.toggle("selected", polygon.dataset.regionId === regionId);
    }
    this.updateMode();
  }

  updateMode() {
    this.svg.hidden = this.regions.length === 0;
    this.svg.classList?.toggle("focus-only", !this.enabled);
    this.svg.classList?.toggle("visualized", this.enabled);
    this.svg.setAttribute?.("aria-label", this.enabled ? "All interaction regions" : "Selected interaction region");
  }

  async activate(region) {
    if (region.render_mask === false) return;
    this.select(region.id);
    if (region.role === "numeric_run") {
      const text = String(region.resolved_data?.text ?? "");
      if (text && navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(text);
        this.actions.onEvent?.({ type: "copy", region, value: text });
      } else {
        this.actions.onEvent?.({ type: "copy-request", region, value: text });
      }
      return;
    }
    const target = resolvedTarget(region);
    if (this.actions.onActivate) await this.actions.onActivate(region, target);
    else this.actions.onEvent?.({ type: "activate", region, value: target });
  }

  showHover(region, event) {
    const data = compactResolvedData(region.resolved_data);
    this.hoverCard.textContent = `${regionLabel(region)}${data ? ` · ${data}` : ""}`;
    this.hoverCard.hidden = false;
    this.positionHover(event);
    this.actions.onEvent?.({ type: "hover", region, value: resolvedTarget(region), quiet: true });
  }

  positionHover(event) {
    const shell = this.svg.parentElement.getBoundingClientRect();
    const left = Math.min(shell.width - 270, Math.max(8, event.clientX - shell.left + 12));
    const top = Math.min(shell.height - 45, Math.max(8, event.clientY - shell.top + 12));
    this.hoverCard.style.left = `${left}px`;
    this.hoverCard.style.top = `${top}px`;
  }

  hideHover() {
    this.hoverCard.hidden = true;
  }
}

export async function handleInteractionAction(region, action, emit) {
  if (action === "copy") {
    const text = String(region.resolved_data?.text ?? "");
    if (navigator.clipboard?.writeText) await navigator.clipboard.writeText(text);
    emit({ type: "copy", region, value: text });
    return;
  }
  emit({ type: "activate", region, value: resolvedTarget(region) });
}

function validPoints(points) {
  if (!Array.isArray(points) || points.length < 3) return null;
  const result = points.filter((point) => Array.isArray(point) && Number.isFinite(point[0]) && Number.isFinite(point[1]));
  return result.length >= 3 ? result : null;
}

function boundsPoints(bounds) {
  if (!bounds || !Number.isFinite(bounds.x) || !Number.isFinite(bounds.y)) return null;
  const right = bounds.x + bounds.width;
  const bottom = bounds.y + bounds.height;
  return [[bounds.x, bounds.y], [right, bounds.y], [right, bottom], [bounds.x, bottom]];
}

function regionLabel(region) {
  const role = String(region.role ?? "interaction").replace(/[_-]+/g, " ");
  return role.replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function compactResolvedData(data) {
  if (!data || typeof data !== "object") return "";
  const preferred = ["title", "name", "text", "card_id", "story_id", "honor_id", "character_id", "target"];
  for (const key of preferred) {
    if (key in data) return `${key}: ${unwrap(data[key])}`;
  }
  const entry = Object.entries(data)[0];
  return entry ? `${entry[0]}: ${unwrap(entry[1])}` : "";
}

function resolvedTarget(region) {
  return Object.fromEntries(Object.entries(region.resolved_data ?? {}).map(([key, value]) => [key, unwrap(value)]));
}

function unwrap(value) {
  return value && typeof value === "object" && "value" in value ? value.value : value;
}

export function scrollBinding(bindings) {
  return (bindings ?? []).find((binding) =>
    binding.kind === "scroll_viewport"
      || binding.kind === "scroll_content"
      || binding.kind === "scroll_thumb"
  );
}

function regionArea(region) {
  const width = Number(region?.bounds?.width);
  const height = Number(region?.bounds?.height);
  return Number.isFinite(width) && Number.isFinite(height) ? Math.max(0, width * height) : 0;
}
