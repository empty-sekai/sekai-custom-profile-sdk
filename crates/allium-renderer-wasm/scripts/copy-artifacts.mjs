import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const dist = join(here, "..", "dist");
const required = [
  "allium_renderer_wasm.js",
  "allium_renderer_wasm.wasm",
  "third-party/freetype/FTL.txt",
];
const missing = required.filter((file) => !existsSync(join(dist, file)));

if (missing.length > 0) {
  console.error(
    `Missing generated browser artifacts: ${missing.join(", ")}\n` +
      "Run `npm run build:wasm` before compiling the TypeScript package.",
  );
  process.exit(1);
}

console.log("Browser artifacts are ready:", required.join(", "));
