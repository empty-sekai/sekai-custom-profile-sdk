# Development

This page covers building and testing the repository: prerequisites, the
Rust and browser package commands, benchmarks and examples, the CI checks,
the commit convention and how releases are made.

See also: [architecture](architecture.md),
[native renderer](native-renderer.md) and [command line](cli.md).

## Prerequisites

- A stable Rust toolchain with `rustfmt` and `clippy`.
- FreeType, found through pkg-config. Hosts without pkg-config can enable
  the `bundled-freetype` feature of `sekai-profile` to build the bundled
  FreeType sources instead.
- CMake and a C compiler for the `jpeg-turbo` feature, which builds the
  bundled libjpeg-turbo. The CLI enables it by default.
- libx264 for the `animation-export` feature.
- Node.js 24, the version the CI uses, for the browser package.
- Docker for the WASM build, or Emscripten (`emcc` on the `PATH`) and a
  Rust toolchain with the `wasm32-unknown-emscripten` target.

On Debian or Ubuntu, the packages the CI installs are
`libfreetype6-dev libx264-dev pkg-config fonts-dejavu-core`.

## Rust

The CI runs these commands:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy -p sekai-profile --all-targets --no-default-features -- -D warnings
cargo test --workspace --all-features
cargo test -p sekai-profile --no-default-features
cargo build -p sekai-profile-cli
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --exclude sekai-profile-cli
```

Tests that load fonts, generate SDF glyphs or lay out text look up their
font families through the usual [font directories](fonts-and-assets.md#font-files)
and skip themselves when a family is missing. The CI points `FONT_DIR` at
the DejaVu fonts so those tests run:

```sh
FONT_DIR=/usr/share/fonts/truetype/dejavu cargo test --workspace --all-features
```

Tests and benchmarks use synthetic inputs; no game data is needed.

## Browser package

The npm package lives in `crates/sekai-profile-wasm`.
`crates/sekai-profile-wasm/build.sh` builds the WASM runtime and the minimal
FreeType it links. With Docker available, it builds the image defined by
`crates/sekai-profile-wasm/Dockerfile` (Emscripten 4.0.10 and Rust 1.94.0)
and copies `sekai_profile_wasm.js`, `sekai_profile_wasm.wasm` and the
FreeType license into `dist/`. Without Docker and with `emcc` on the `PATH`,
it builds in the current environment.

```sh
cd crates/sekai-profile-wasm
npm ci
npm run build
npm run typecheck
npm run test:gates
```

| Script | Effect |
| --- | --- |
| `build:wasm` | Runs `build.sh` |
| `build:ts` | Compiles the TypeScript wrapper into `dist/` and checks that the WASM artifacts are present |
| `build` | `build:wasm`, then `build:ts` |
| `typecheck` | Type-checks the TypeScript sources |
| `test:gates` | Runs the Node tests in `scripts/test/` |
| `audit:public` | Checks that the packed npm package contains only its declared files and one export |
| `verify:wasm:runtime` | Loads the built WASM module in Node and exercises its exported functions |
| `measure:wasm:size` | Prints the raw, gzip and brotli size of the WASM file |
| `demo:serve` | Serves the package root for the demo at `http://127.0.0.1:8088/demo/`; `--host` and `--port` change the address |

`test:gates` covers the demo server, which loads the built WASM artifacts,
so build the package before running it.

## Benchmarks

```sh
FONT_DIR=/usr/share/fonts/truetype/dejavu \
  cargo bench -p sekai-profile --features jpeg-turbo --bench render
```

The render and encode benchmarks build every input themselves from DejaVu
Sans, so `FONT_DIR` (or `SEKAI_PROFILE_FONT_DIR`) must name a directory
holding `DejaVuSans.ttf`; without it, the benchmarks that need the font are
skipped. Without `jpeg-turbo` the JPEG benchmarks are left out. The SIMD and
AVX-512 benchmarks run only on a CPU that supports them.

## Examples

`crates/sekai-profile/examples/` holds:

| Example | Purpose |
| --- | --- |
| `ordered_pages` | Renders each page of a card list on the scalar executor and prints the SHA-256 of its pixels |
| `animation_export` | Runs cards through the animation export and prints one line per format; needs `animation-export` |
| `plain_canvas` | Draws plain labels and graphics with the plain CPU canvas |
| `compare_sdf` | Compares the analytic and distance-transform SDF generators; needs `dev` |

The comment at the top of `ordered_pages`, `animation_export` and
`compare_sdf` shows their arguments; `plain_canvas` takes a font file and an
output PNG path.

## Continuous integration

`.github/workflows/ci.yml` runs on pushes and pull requests to `main`:

- `test`: formatting, clippy with and without optional features, the
  workspace tests and the rustdoc build listed above;
- `test (no default features)`: the `sekai-profile` tests without default
  features and a build of the CLI, with only FreeType and pkg-config
  installed;
- `browser gates`: the WASM build, `typecheck`, `build:ts`, `test:gates`,
  `verify:wasm:runtime`, `audit:public` and the WebGL2 shader and geometry
  contract tests.

`.github/workflows/commitlint.yml` checks commit messages.

## Commits

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/),
with a header of at most 100 characters. Release notes are generated from
the commits by [git-cliff](https://git-cliff.org/) with `cliff.toml`:
`feat` commits appear under Features, `fix` under Bug Fixes, `perf` under
Performance, and `refactor` commits with a `BREAKING CHANGE` footer under
Breaking Changes. Other types are left out of the release notes.

## Releases

Pushing a tag `v<version>` runs `.github/workflows/release.yml`:

1. the same checks as the CI `test` and `test (no default features)` jobs;
2. `sekai-profile` archives for Linux x86_64 and aarch64, Windows x86_64 and
   macOS aarch64;
3. the WASM build, the browser gates and the npm publication of
   `@empty-sekai/sekai-custom-profile-sdk`. The tag must match the `version`
   in `crates/sekai-profile-wasm/package.json`; a tag containing `-` is
   published under the `next` dist-tag;
4. the demo, deployed to GitHub Pages;
5. a GitHub Release with the release notes, the CLI archives, the browser
   package as `sekai-custom-profile-sdk-browser-<version>.zip` and
   `SHA256SUMS`. A tag containing `-` makes a prerelease.

Running the workflow manually builds and deploys the demo without publishing
to npm or creating a release.
