import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("interaction dumps expose a discriminated control binding contract", async () => {
  const types = await readFile(new URL("../../src/types/core.ts", import.meta.url), "utf8");
  const index = await readFile(new URL("../../src/index.ts", import.meta.url), "utf8");
  assert.match(types, /export type CoreControlBinding\s*=/);
  assert.match(types, /kind:\s*"tab_option";\s*control_id:\s*StableId;\s*value:\s*string/);
  assert.match(types, /kind:\s*"scroll_content";\s*control_id:\s*StableId/);
  assert.match(types, /kind:\s*"scroll_thumb";\s*control_id:\s*StableId/);
  assert.match(types, /control_bindings:\s*CoreControlBinding\[\]/);
  assert.match(index, /CoreControlBinding/);
});
