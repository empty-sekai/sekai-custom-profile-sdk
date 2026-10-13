import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";

import { InteractionOverlay, scrollBinding } from "../../demo/workbench/interactions.js";

test("interaction overlays start visually quiet while hit routing remains available", () => {
  const overlay = new InteractionOverlay({}, {}, {});
  assert.equal(overlay.enabled, false);
  assert.equal(overlay.selectedId, null);
});

test("quiet interaction mode never tints the selected card region", () => {
  const css = fs.readFileSync(new URL("../../demo/demo.css", import.meta.url), "utf8");
  assert.match(css, /\.interaction-overlay\.focus-only polygon\s*\{\s*opacity:\s*0;\s*\}/);
  assert.doesNotMatch(css, /focus-only polygon:not\(\.selected\)/);
  assert.match(css, /\.interaction-overlay \.selected, \.interaction-overlay polygon:hover\s*\{\s*fill:\s*transparent;/);
});

test("wheel routing selects only an authored scroll binding", () => {
  const tab = { kind: "tab", control_id: "tabs" };
  const content = { kind: "scroll_content", control_id: "stories" };
  const viewport = { kind: "scroll_viewport", control_id: "stories" };
  const thumb = { kind: "scroll_thumb", control_id: "stories" };

  assert.equal(scrollBinding([tab]), undefined);
  assert.equal(scrollBinding([tab, content]), content);
  assert.equal(scrollBinding([tab, viewport]), viewport);
  assert.equal(scrollBinding([tab, viewport, content]), viewport);
  assert.equal(scrollBinding([tab, thumb]), thumb);
});
