import assert from "node:assert/strict";
import test from "node:test";

import { InteractionOverlay, scrollBinding } from "../../demo/workbench/interactions.js";

test("interaction overlays start quiet until the caller enables all regions", () => {
  const overlay = new InteractionOverlay({}, {}, {});
  assert.equal(overlay.enabled, false);
  assert.equal(overlay.selectedId, null);
});

test("wheel routing selects only an authored scroll binding", () => {
  const tab = { kind: "tab", control_id: "tabs" };
  const scroll = { kind: "scroll", control_id: "stories", step: 2 };

  assert.equal(scrollBinding([tab]), undefined);
  assert.equal(scrollBinding([tab, scroll]), scroll);
});
