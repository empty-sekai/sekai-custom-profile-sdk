#!/usr/bin/env bash
# 构建 allium-renderer-wasm 浏览器制品（.js + .wasm），输出到 dist/。
#
# 用法：从仓库根或本目录运行均可：
#   crates/allium-renderer-wasm/build.sh
#
# 流程：docker build（上下文=仓库根）→ docker create 临时容器 → docker cp 取产物。
# 构建链版本固定在 Dockerfile（emsdk 4.0.10 / Rust 1.94），勿在此覆盖。
set -euo pipefail

# 定位仓库根（本脚本在 crates/allium-renderer-wasm/）
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DOCKERFILE="$SCRIPT_DIR/Dockerfile"
DIST_DIR="$SCRIPT_DIR/dist"
IMAGE="allium-renderer-wasm:latest"

# Disable proxy during Docker build so downloads reach the internet directly.
PROXY_ARGS=(
  --build-arg HTTP_PROXY=
  --build-arg HTTPS_PROXY=
  --build-arg http_proxy=
  --build-arg https_proxy=
  --build-arg NO_PROXY='*'
)

echo "==> docker build (context: $REPO_ROOT)"
docker build "${PROXY_ARGS[@]}" -f "$DOCKERFILE" -t "$IMAGE" "$REPO_ROOT"

echo "==> extracting artifacts to $DIST_DIR"
mkdir -p "$DIST_DIR"
CID="$(docker create "$IMAGE")"
trap 'docker rm -f "$CID" >/dev/null 2>&1 || true' EXIT
docker cp "$CID:/artifacts/allium_renderer_wasm.js"   "$DIST_DIR/"
docker cp "$CID:/artifacts/allium_renderer_wasm.wasm" "$DIST_DIR/"

echo "==> done:"
ls -la "$DIST_DIR"
