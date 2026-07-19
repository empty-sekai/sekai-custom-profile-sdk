import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const workflowUrl = new URL("../../../../.github/workflows/release.yml", import.meta.url);

async function workflowSource() {
  return readFile(workflowUrl, "utf8");
}

test("npm publishing is version-strict, idempotent, and uses trusted publishing", async () => {
  const workflow = await workflowSource();

  assert.match(workflow, /CURRENT_VERSION=.*package\.json/);
  assert.match(
    workflow,
    /if \[\[ "\$\{CURRENT_VERSION\}" != "\$\{VERSION\}" \]\]; then[\s\S]*?exit 1\s+fi/,
  );
  assert.match(workflow, /PACKAGE_NAME=.*package\.json/);
  assert.match(workflow, /PUBLISHED_VERSION=.*npm view/);
  assert.match(workflow, /is already published; skipping npm publish/);
  assert.match(workflow, /id-token: write\s+# npm Trusted Publishing \(OIDC\)/);
  assert.doesNotMatch(workflow, /npm version --no-git-tag-version/);
  assert.doesNotMatch(workflow, /NODE_AUTH_TOKEN/);
});

test("wasm release job runs every browser release gate", async () => {
  const workflow = await workflowSource();

  assert.match(workflow, /node-version: '24'/);
  assert.match(workflow, /npm ci --no-audit --no-fund/);
  assert.match(workflow, /os: macos-15\s/);

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

test("Pages deploys every ESM module graph below an immutable build path", async () => {
  const workflow = await workflowSource();
  assert.match(workflow, /SITE_ROOT: public\/builds\/\$\{\{ github\.sha \}\}/);
  assert.match(workflow, /path: public\/builds\/\$\{\{ github\.sha \}\}\/dist/);
  assert.match(workflow, /cp -r crates\/allium-renderer-wasm\/demo "\$\{SITE_ROOT\}\/demo"/);
  assert.match(workflow, /url=\.\/builds\/\$\{GITHUB_SHA\}\/demo\//);
});

test("release validates the complete Rust workspace feature surface", async () => {
  const workflow = await workflowSource();
  assert.match(workflow, /cargo test --workspace --all-features/);
});

test("GitHub Release zip retains the FreeType license under dist", async () => {
  const workflow = await workflowSource();

  assert.match(workflow, /cp -a artifacts\/wasm-dist\/. artifacts\/wasm-package\/dist\//);
  assert.match(workflow, /zip -r .*sekai-custom-profile-sdk-browser-\$\{VERSION\}\.zip.*dist/);
  assert.match(workflow, /dist\/third-party\/freetype\/FTL\.txt/);
});

test("native release matrix covers every supported platform with checksums", async () => {
  const workflow = await workflowSource();

  for (const target of [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
  ]) {
    assert.match(workflow, new RegExp(target));
  }

  for (const archive of [
    "sekai-custom-profile-sdk-linux-x86_64-gnu.tar.xz",
    "sekai-custom-profile-sdk-linux-aarch64-gnu.tar.xz",
    "sekai-custom-profile-sdk-windows-x86_64.zip",
    "sekai-custom-profile-sdk-macos-aarch64.tar.xz",
  ]) {
    assert.match(workflow, new RegExp(archive.replaceAll(".", "\\.")));
  }

  assert.match(workflow, /sha256sum \.\/\* > SHA256SUMS/);
  assert.match(workflow, /runs-on: ubuntu-24\.04-arm/);
});

test("manual dispatch never publishes npm or creates a GitHub Release", async () => {
  const workflow = await workflowSource();
  const tagGuard = /if: startsWith\(github\.ref, 'refs\/tags\/v'\)/g;

  assert.equal(workflow.match(tagGuard)?.length, 2);
  assert.match(workflow, /workflow_dispatch:/);
  assert.match(workflow, /name: Publish to npm\s+if: startsWith\(github\.ref, 'refs\/tags\/v'\)/);
  assert.match(workflow, /name: github release\s+needs:[\s\S]*?if: startsWith\(github\.ref, 'refs\/tags\/v'\)/);
});
