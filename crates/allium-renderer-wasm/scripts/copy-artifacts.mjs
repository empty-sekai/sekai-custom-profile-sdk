// 把 wasm 构建产物（dist/ 内的 .js + .wasm）保留在 tsc 输出旁。
// tsc 只编 src/*.ts → dist/*.js，不会动 build.sh 已放进 dist/ 的 wasm 产物；
// 本脚本仅校验产物存在，缺失则给出明确指引（CI 顺序保障：build:wasm 在前）。
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const dist = join(here, "..", "dist");
const required = ["allium_renderer_wasm.js", "allium_renderer_wasm.wasm"];

const missing = required.filter((f) => !existsSync(join(dist, f)));
if (missing.length > 0) {
  console.error(
    `dist/ 缺少 wasm 产物: ${missing.join(", ")}\n` +
      `先运行 \`npm run build:wasm\`（或 bash build.sh）生成。`,
  );
  process.exit(1);
}
console.log("wasm 产物就位:", required.join(", "));
