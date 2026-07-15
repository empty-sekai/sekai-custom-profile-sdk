import { readFile } from "node:fs/promises";
import { brotliCompressSync, gzipSync } from "node:zlib";

const bytes = await readFile(new URL("../dist/allium_renderer_wasm.wasm", import.meta.url));
console.log(JSON.stringify({
  raw: bytes.byteLength,
  gzip: gzipSync(bytes, { level: 9 }).byteLength,
  brotli: brotliCompressSync(bytes).byteLength,
}, null, 2));
