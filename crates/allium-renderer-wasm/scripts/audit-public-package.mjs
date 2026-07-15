import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { extname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const packageJson = JSON.parse(await readFile(resolve(packageRoot, "package.json"), "utf8"));
const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
const pack = JSON.parse(execFileSync(npmCommand, ["pack", "--dry-run", "--json"], {
  cwd: packageRoot,
  encoding: "utf8",
}));
const files = pack[0]?.files?.map((entry) => entry.path) ?? [];

const rootFiles = new Set([
  "LICENSE",
  "LICENSE-EXCEPTION",
  "NOTICE",
  "README.md",
  "README.en.md",
  "package.json",
]);
const sourceExtensions = new Set([".rs", ".ts"]);
const distExtensions = new Set([".js", ".ts", ".txt", ".wasm"]);
const failures = [];

for (const path of files) {
  if (rootFiles.has(path)) continue;
  if (path.startsWith("src/") && sourceExtensions.has(extname(path).toLowerCase())) continue;
  if (path.startsWith("dist/") && distExtensions.has(extname(path).toLowerCase())) continue;
  failures.push(`${path}: outside the declared public package roots`);
}

const wasmFiles = files.filter((path) => path.endsWith(".wasm"));
if (wasmFiles.length !== 1 || wasmFiles[0] !== "dist/allium_renderer_wasm.wasm") {
  failures.push(`expected one runtime WASM artifact, received ${JSON.stringify(wasmFiles)}`);
}

const exportKeys = Object.keys(packageJson.exports ?? {});
if (exportKeys.length !== 1 || exportKeys[0] !== ".") {
  failures.push(`expected one package export, received ${JSON.stringify(exportKeys)}`);
}
for (const path of [packageJson.main, packageJson.module, packageJson.types]) {
  const packedPath = String(path ?? "").replace(/^\.\//, "");
  if (!files.includes(packedPath)) failures.push(`${packedPath}: declared entry is absent from the package`);
}

if (failures.length > 0) {
  throw new Error(`Public package audit failed:\n${failures.sort().join("\n")}`);
}

console.log(JSON.stringify({ packedFiles: files.length, wasmFiles: wasmFiles.length, exports: exportKeys }));
