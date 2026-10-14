# Command line

This page describes the `sekai-profile` executable: its `render`, `prepare`
and `serve` subcommands, where it reads masterdata and assets from, and its
flags and defaults.

The command line is built on the [native renderer](native-renderer.md). It
draws text and shapes from SDF atlases; see
[fonts, atlases and assets](fonts-and-assets.md) for the atlas directory and
the font files it expects.

## Installing

GitHub Releases provide archives of the `sekai-profile` executable for
Linux x86_64 and aarch64 (glibc), Windows x86_64 and macOS aarch64. To build
it from source, install the [prerequisites](development.md#prerequisites)
and run:

```sh
cargo build --release -p sekai-profile-cli
```

JPEG output uses the `jpeg-turbo` feature, which the CLI enables by default.
A build with `--no-default-features` writes PNG only.

## Quick start

```sh
sekai-profile prepare --masterdata ./masterdata --font-dir ./fonts \
  --assets-dir ./assets --atlas-dir ./atlases

sekai-profile render --profile ./profile.json --masterdata ./masterdata \
  --assets-dir ./assets --font-dir ./fonts --atlas-dir ./atlases -o card.png
```

## `render`

Renders one page to a PNG or JPEG file.

`--profile` takes a Profile API response. It supplies the player data and,
without `--card`, the pages: `render` draws the first page of
`userCustomProfileCards` in the order the game shows them. `--card` takes a
card JSON instead, either a `customProfileCard` object or a
`userCustomProfileCards` array; one of `--profile` and `--card` is required.

Before rendering, the glyphs the page needs are added to the text atlases
in the atlas directory, and the shape atlas is built when it is missing.
Existing atlases are kept and extended as needed, so later renders reuse
them. `--no-prepare` renders with the atlases as they are. Assets the page
draws that are missing are reported and drawn transparent.

| Flag | Default | Effect |
| --- | --- | --- |
| `--profile <FILE>` | | Profile API response (JSON) |
| `--card <FILE>` | | Card JSON: a `customProfileCard` object or a `userCustomProfileCards` array |
| `--page <N>` | `1` | Page to render, counted from 1 in the order the game shows them |
| `--seq <SEQ>` | | Render the page with this `seq` instead; conflicts with `--page` |
| `--atlas-dir <DIR>` | `atlases` | Atlas directory; also read from `SEKAI_PROFILE_ATLAS_DIR` |
| `--no-prepare` | | Render with the atlases as they are |
| `--render-objects <MANIFEST>` | | Pre-decoded render-object store, by its manifest |
| `-o`, `--output <FILE>` | | Output file (required) |
| `--format <png\|jpeg>` | from the extension | Output format; `.png`, `.jpg` and `.jpeg` are recognised |
| `--transparent` | | Keep the background transparent (PNG only) |
| `--quality <1-100>` | `90` | JPEG quality; ignored for PNG |
| `--executor <auto\|simd\|scalar>` | `auto` | SDF executor; `auto` uses AVX-512 when the CPU supports it, `simd` fails on a CPU without AVX-512 F, BW and VBMI plus FMA |

The masterdata and asset flags below also apply.

## `prepare`

Builds complete text atlases for every font family the masterdata names and
a shape atlas for every shape, which suits preparing once before starting a
long-running `serve`.

| Flag | Default | Effect |
| --- | --- | --- |
| `--atlas-dir <DIR>` | `atlases` | Atlas directory to fill; also read from `SEKAI_PROFILE_ATLAS_DIR` |
| `--for <FILE>` | | Cover only the text of these Profile API responses instead of every character the fonts map; repeatable |
| `--force` | | Rebuild atlases that already cover the request |

The shape atlas is built from the shape sprites among the assets, so it is
skipped when neither `--assets-dir` nor `--assets-url` is given. When it
finishes, `prepare` prints how many text atlases it built and kept, whether
it built the shape atlas, and the font families without a font file.

## `serve`

Answers render requests read as NDJSON from stdin, keeping fonts,
masterdata, atlases and the asset cache across requests. It uses the atlas
directory as `prepare` left it (`--atlas-dir`, default `atlases`, also read
from `SEKAI_PROFILE_ATLAS_DIR`) and takes the masterdata and asset flags
below.

Each input line is one JSON request. Requests are handled one at a time in
the order they arrive, and each gets one JSON response line on stdout. Logs
go to stderr.

```text
{"id": 1, "method": "render", "params": {"profile": <Profile API response>, "output": "card.jpg"}}
{"id": 2, "method": "reload_masterdata", "params": {"dir": "./masterdata"}}
{"id": 3, "method": "ping"}
{"id": 4, "method": "shutdown"}
```

`render` parameters:

| Parameter | Default | Meaning |
| --- | --- | --- |
| `profile` | | Profile API response object |
| `card` | the profile's `userCustomProfileCards` | A `customProfileCard` object or a `userCustomProfileCards` array |
| `page` | `1` | Page number counted from 1 in the order the game shows them |
| `seq` | | Render the page with this `seq` instead of `page` |
| `format` | `"jpeg"` | `"jpeg"` (or `"jpg"`) or `"png"` |
| `transparent` | `false` | Keep the background transparent, PNG only |
| `quality` | `90` | JPEG quality, 1 to 100 |
| `output` | | Path to write the image to |
| `inline` | `false` | Return the image base64-encoded in `data` |

At least one of `output` and `inline` is required. An absent or `null`
parameter takes its default; a parameter of the wrong type fails the
request.

Responses carry `path` when `output` is given and `data` when `inline` is
set; `missing_assets` lists the assets drawn transparent and `warnings` the
card references the masterdata does not define:

```text
{"id": 1, "ok": true, "result": {"path": "card.jpg", "bytes": 12345, "missing_assets": [], "warnings": []}}
{"id": 1, "ok": false, "error": "..."}
```

`reload_masterdata` replaces the masterdata with the tables in `params.dir`,
read for the region given at startup, and returns the required tables that
are missing. `ping` answers `"pong"`; `shutdown` answers `"bye"` and exits. The end of input is the same as
`shutdown`. A line that is not UTF-8 or JSON gets an error response, and the
session continues with the next line.

## Masterdata and assets

These flags apply to every subcommand.

| Flag | Default | Effect |
| --- | --- | --- |
| `--masterdata <DIR>` | | Directory of masterdata tables, read as `<DIR>/<table>.json` |
| `--masterdata-url <URL>` | | URL prefix of masterdata tables, fetched as `<URL>/<table>.json` |
| `--region <REGION>` | `cn` | Game server region: `cn`, `jp`, `tw`, `kr` or `en` (also `sc`, `tc` and `world`) |
| `--font-dir <DIR>` | | Directory of font files; also read from `SEKAI_PROFILE_FONT_DIR` |
| `--assets-dir <DIR>` | | Directory of PNG assets; a file's asset key is its relative path without `.png` |
| `--assets-url <URL>` | | URL prefix that assets missing from `--assets-dir` are fetched from |
| `--static-url <URL>` | | URL prefix of the renderer's static assets (frames, icons, masks); requires `--assets-url`, which serves them when this is not given |
| `--asset-url-layout <flat\|game-assets>` | `flat` | Path layout under the asset URLs |

Exactly one of `--masterdata` and `--masterdata-url` is required. Each URL is
a plain prefix: only `/<table>.json` or the asset path is appended.

Fetching masterdata from a URL requests every table the renderer reads. A
table the server answers with a 4xx status (other than 408 and 429) is
treated as absent: a required table is logged as missing and the elements
that use it are not drawn, and an optional table is skipped silently. Any
other failure, or a table that is not UTF-8 or JSON, stops the command.

Assets are fetched only when they are missing locally, eight at a time, with
a 30-second timeout and up to four attempts per request. A response that is
not a PNG counts as a failure. A key listed in the CLI's embedded manifest
of static assets is fetched from `--static-url`; every other key from
`--assets-url`.

With `--asset-url-layout flat`, a key is fetched from `<URL>/<key>.png`.
`game-assets` follows the directories of the game's asset bundles, as
`sekai_profile::asset_keys::key_to_s3_path` maps them: bonds honor character
art `bonds_honor/chr_sd_<name>` is fetched from
`bonds_honor/character/chr_sd_<name>/chr_sd_<name>.png`, bonds honor words
`bonds_honor/word/<name>` from `bonds_honor/word/<name>/<name>.png`, keys
under `ugc/editor_image/`, `ugc/avatar/`, `uploads/editor_image/`,
`uploads/avatar/` and `presets/` are used as complete paths, and every other
key is fetched from `<key>.png`. The layout changes only the URL; the asset
keys the renderer uses stay the same.

## Logging

Logs go to stderr through `tracing`. `RUST_LOG` sets the filter; the default
is `info`.
