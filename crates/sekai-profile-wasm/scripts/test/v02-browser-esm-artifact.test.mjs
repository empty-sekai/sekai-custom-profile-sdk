import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const packageRoot = fileURLToPath(new URL("../../", import.meta.url));
const importPattern = /(?:from\s+|import\s*\()(["'])(\.{1,2}\/[^"']+)\1/g;

test("built browser ESM graph uses directly resolvable relative module URLs", async () => {
  const pending = ["dist/index.js", "dist/worker.js"];
  const visited = new Set();
  while (pending.length > 0) {
    const relativePath = pending.pop();
    if (!relativePath || visited.has(relativePath)) continue;
    visited.add(relativePath);
    const source = await readFile(path.join(packageRoot, relativePath), "utf8");
    for (const match of source.matchAll(importPattern)) {
      const specifier = match[2];
      assert.match(
        specifier,
        /\.(?:js|mjs|json|wasm)$/,
        `${relativePath} contains a browser-unresolvable extensionless import: ${specifier}`,
      );
      const resolved = path.normalize(path.join(path.dirname(relativePath), specifier));
      await access(path.join(packageRoot, resolved));
      if (/\.m?js$/.test(resolved)) pending.push(resolved);
    }
  }
  assert.ok(visited.size > 2, "the gate must traverse the emitted module graph");
});
