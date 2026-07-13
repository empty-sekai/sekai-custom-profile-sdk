import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const workflowUrl = new URL("../../../../.github/workflows/release.yml", import.meta.url);

async function workflowSource() {
  return readFile(workflowUrl, "utf8");
}

test("npm publishing accepts a tag that already matches package.json", async () => {
  const workflow = await workflowSource();

  assert.match(workflow, /CURRENT_VERSION=.*package\.json/);
  assert.match(
    workflow,
    /if \[\[ "\$\{CURRENT_VERSION\}" != "\$\{VERSION\}" \]\]; then\s+npm version --no-git-tag-version "\$VERSION"\s+fi/,
  );
});

test("wasm release job runs every browser release gate", async () => {
  const workflow = await workflowSource();

  for (const command of [
    "npm run test:gates",
    "npm run verify:wasm:runtime",
    "npm run audit:public",
    "npm run measure:wasm:size",
  ]) {
    assert.match(workflow, new RegExp(command.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  }

  assert.match(workflow, /name: WebGL2 GPU contract gate/);
  for (const gate of [
    "m3-browser-semantic-resources.test.mjs",
    "m3-semantic-geometry.test.mjs",
    "m3-semantic-text-glyph-bridge.test.mjs",
    "m4-shape-sdf-shader.test.mjs",
  ]) {
    assert.match(workflow, new RegExp(gate.replaceAll(".", "\\.")));
  }
});

test("release validates the complete Rust workspace feature surface", async () => {
  const workflow = await workflowSource();
  assert.match(workflow, /cargo test --workspace --all-features/);
});

test("GitHub Release zip retains the FreeType license under dist", async () => {
  const workflow = await workflowSource();

  assert.match(workflow, /cp -a artifacts\/wasm-dist\/. artifacts\/wasm-package\/dist\//);
  assert.match(workflow, /zip -r .*allium-renderer-wasm-\$\{VERSION\}\.zip.*dist/);
  assert.match(workflow, /dist\/third-party\/freetype\/FTL\.txt/);
  assert.doesNotMatch(workflow, /zip -j/);
});

test("native release matrix covers every supported platform with checksums", async () => {
  const workflow = await workflowSource();

  for (const target of [
    "x86_64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
  ]) {
    assert.match(workflow, new RegExp(target));
  }

  for (const archive of [
    "allium-renderer-linux-x86_64-gnu.tar.xz",
    "allium-renderer-linux-x86_64-musl.tar.xz",
    "allium-renderer-linux-aarch64-gnu.tar.xz",
    "allium-renderer-windows-x86_64.zip",
    "allium-renderer-macos-aarch64.tar.xz",
    "allium-renderer-macos-x86_64.tar.xz",
  ]) {
    assert.match(workflow, new RegExp(archive.replaceAll(".", "\\.")));
  }

  assert.match(workflow, /sha256sum \.\/\* > SHA256SUMS/);
  assert.match(workflow, /runs-on: ubuntu-24\.04-arm/);
  assert.match(workflow, /rust:1\.88-alpine3\.22/);
  assert.match(workflow, /PKG_CONFIG_ALL_STATIC=1/);
  assert.match(workflow, /target-feature=\+crt-static/);
  assert.match(workflow, /musl release binary is dynamically linked/);
});

test("manual dispatch never publishes npm or creates a GitHub Release", async () => {
  const workflow = await workflowSource();
  const tagGuard = /if: startsWith\(github\.ref, 'refs\/tags\/v'\)/g;

  assert.equal(workflow.match(tagGuard)?.length, 2);
  assert.match(workflow, /workflow_dispatch:/);
  assert.match(workflow, /name: Publish to npm\s+if: startsWith\(github\.ref, 'refs\/tags\/v'\)/);
  assert.match(workflow, /name: github release\s+needs:[\s\S]*?if: startsWith\(github\.ref, 'refs\/tags\/v'\)/);
});
