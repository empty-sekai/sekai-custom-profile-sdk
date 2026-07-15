import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { createServer } from "node:http";
import { dirname, extname, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const PACKAGE_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const MIME_TYPES = new Map([
  [".css", "text/css; charset=utf-8"],
  [".html", "text/html; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".json", "application/json; charset=utf-8"],
  [".png", "image/png"],
  [".svg", "image/svg+xml"],
  [".wasm", "application/wasm"],
]);

export function createDemoServer(root = PACKAGE_ROOT) {
  const normalizedRoot = resolve(root);
  const rootPrefix = `${normalizedRoot}${sep}`;
  return createServer(async (request, response) => {
    try {
      const url = new URL(request.url ?? "/", "http://127.0.0.1");
      let pathname = decodeURIComponent(url.pathname);
      if (pathname.endsWith("/")) pathname += "index.html";
      const filePath = resolve(normalizedRoot, `.${pathname}`);
      if (filePath !== normalizedRoot && !filePath.startsWith(rootPrefix)) {
        respond(response, 403, "Forbidden");
        return;
      }
      const metadata = await stat(filePath);
      if (!metadata.isFile()) {
        respond(response, 404, "Not found");
        return;
      }
      response.writeHead(200, {
        "Cache-Control": "no-store, max-age=0",
        "Content-Length": metadata.size,
        "Content-Type": MIME_TYPES.get(extname(filePath)) ?? "application/octet-stream",
      });
      if (request.method === "HEAD") {
        response.end();
        return;
      }
      createReadStream(filePath).pipe(response);
    } catch (error) {
      respond(response, error?.code === "ENOENT" ? 404 : 500, error?.code === "ENOENT" ? "Not found" : "Internal server error");
    }
  });
}

function respond(response, status, message) {
  response.writeHead(status, {
    "Cache-Control": "no-store, max-age=0",
    "Content-Type": "text/plain; charset=utf-8",
  });
  response.end(message);
}

function option(name, fallback) {
  const index = process.argv.indexOf(name);
  return index >= 0 && process.argv[index + 1] ? process.argv[index + 1] : fallback;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const host = option("--host", "127.0.0.1");
  const port = Number(option("--port", "8088"));
  if (!Number.isInteger(port) || port < 0 || port > 65535) {
    throw new Error(`invalid --port: ${port}`);
  }
  createDemoServer().listen(port, host, () => {
    process.stdout.write(`Allium renderer demo: http://${host}:${port}/demo/\n`);
  });
}
