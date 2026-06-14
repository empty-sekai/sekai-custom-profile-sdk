#!/usr/bin/env python3
"""
Static + CDN-proxy server for the allium-renderer-wasm demo.

The browser demo at `demo/index.html` imports the wasm package from
`../dist/`, so the static root must be the crate directory (one level
above `demo/`). The CDN you mirror masterdata / assets from typically
does not return CORS headers, so this script also exposes a
same-origin reverse proxy at `/cdn/*` -> `$ALLIUM_CDN_BASE/*` with
permissive CORS, letting the browser fetch them without configuring
the upstream CDN.

Usage:
    ALLIUM_CDN_BASE=https://your-cdn.example.com python serve.py [port]

Default port: 8088. Default CDN base: empty (proxy disabled).

Stdlib only — no third-party dependencies.
"""
from __future__ import annotations

import os
import sys
import urllib.request
import urllib.error
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

# Static root = crate directory (one level above `demo/`).
CRATE_DIR = Path(__file__).resolve().parent.parent
CDN_BASE = os.environ.get("ALLIUM_CDN_BASE", "").rstrip("/")
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8088


class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        # Serve from the crate root, not demo/. `directory=` is the
        # documented way to set this on the SimpleHTTPRequestHandler.
        super().__init__(*args, directory=str(CRATE_DIR), **kwargs)

    def end_headers(self):
        # Allow ES modules + wasm to be fetched cross-origin during
        # development. Same-origin requests get the header too — harmless.
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def do_GET(self):
        if self.path.startswith("/cdn/"):
            return self._proxy_cdn()
        return super().do_GET()

    def do_HEAD(self):
        if self.path.startswith("/cdn/"):
            return self._proxy_cdn(method="HEAD")
        return super().do_HEAD()

    def _proxy_cdn(self, method: str = "GET"):
        if not CDN_BASE:
            self.send_error(503, "ALLIUM_CDN_BASE not set; proxy disabled")
            return
        # Strip the `/cdn` prefix; keep the rest of the path verbatim.
        upstream_path = self.path[len("/cdn") :] or "/"
        url = f"{CDN_BASE}{upstream_path}"
        req = urllib.request.Request(url, method=method)
        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                self.send_response(resp.status)
                # Forward content-type + length; CORS headers come from
                # end_headers() above.
                for h in ("Content-Type", "Content-Length", "ETag", "Last-Modified"):
                    v = resp.headers.get(h)
                    if v:
                        self.send_header(h, v)
                self.end_headers()
                if method == "GET":
                    while True:
                        chunk = resp.read(64 * 1024)
                        if not chunk:
                            break
                        self.wfile.write(chunk)
        except urllib.error.HTTPError as e:
            self.send_error(e.code, e.reason)
        except Exception as e:  # noqa: BLE001 — surface upstream failures
            self.send_error(502, f"upstream error: {e}")


def main() -> None:
    print(f"==> serving {CRATE_DIR} on http://localhost:{PORT}")
    if CDN_BASE:
        print(f"==> proxy:  /cdn/* -> {CDN_BASE}/*")
    else:
        print("==> proxy disabled (set ALLIUM_CDN_BASE to enable /cdn/*)")
    print(f"==> open:   http://localhost:{PORT}/demo/")
    with ThreadingHTTPServer(("127.0.0.1", PORT), Handler) as httpd:
        try:
            httpd.serve_forever()
        except KeyboardInterrupt:
            print("\n==> shutting down")


if __name__ == "__main__":
    main()
