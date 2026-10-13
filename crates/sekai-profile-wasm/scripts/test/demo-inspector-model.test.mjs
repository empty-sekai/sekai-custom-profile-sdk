import assert from "node:assert/strict";
import test from "node:test";

import {
  filterControls,
  filterLayers,
  groupControls,
  groupLayers,
  groupRegions,
  jsonAtPath,
  searchJson,
} from "../../demo/workbench/inspector.js";

test("layer filters preserve authored order while optional groups use authored kinds", () => {
  const layers = [
    { layer_id: "shape-1", authored_index: 0, authored_kind: "shape", source_content: "Panel" },
    { layer_id: "text-1", authored_index: 1, authored_kind: "text", source_content: "Score 123" },
    { layer_id: "shape-2", authored_index: 2, authored_kind: "shape", source_content: "Rule" },
  ];

  assert.deepEqual(filterLayers(layers, "score", "all").map((layer) => layer.layer_id), ["text-1"]);
  assert.deepEqual(filterLayers(layers, "", "shape").map((layer) => layer.layer_id), ["shape-1", "shape-2"]);
  assert.deepEqual([...groupLayers(layers).entries()].map(([kind, values]) => [kind, values.map((layer) => layer.layer_id)]), [
    ["shape", ["shape-1", "shape-2"]],
    ["text", ["text-1"]],
  ]);
});

test("controls and regions are searchable and grouped by meaningful behavior", () => {
  const controls = [
    { id: "music-tabs", role: "song completion", state: { kind: "tabs" } },
    { id: "story-scroll", role: "favorite stories", state: { kind: "scroll" } },
  ];
  const regions = [
    { id: "card-1", role: "card", capabilities: ["navigate"] },
    { id: "number-1", role: "numeric_run", resolved_data: { text: "123" } },
    { id: "tab-1", role: "tab", control_bindings: [{ kind: "tab_option" }] },
  ];

  assert.deepEqual(filterControls(controls, "story").map((control) => control.id), ["story-scroll"]);
  assert.deepEqual([...groupControls(controls).keys()], ["Tabs", "Scroll"]);
  assert.deepEqual([...groupRegions(regions).entries()].map(([group, values]) => [group, values.map((region) => region.id)]), [
    ["Navigation", ["card-1"]],
    ["Numeric text", ["number-1"]],
    ["Component controls", ["tab-1"]],
  ]);
});

test("dump navigation resolves JSON pointer paths and search reports bounded paths", () => {
  const dump = {
    schema_major: 1,
    layers: [
      { layer_id: "layer-a", source_content: "hello" },
      { layer_id: "layer-b", source_content: "score 123" },
    ],
  };

  assert.deepEqual(jsonAtPath(dump, "/layers/1"), dump.layers[1]);
  assert.equal(jsonAtPath(dump, "/layers/99"), undefined);
  assert.deepEqual(searchJson(dump, "score", 10).map((result) => result.path), ["/layers/1/source_content"]);
  assert.equal(searchJson(dump, "layer", 1).length, 1);
});
