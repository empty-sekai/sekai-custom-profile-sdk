# Fonts, atlases and assets

This page describes the inputs the renderers read besides the profile
itself: the atlas directory, font files and how they are found, the
masterdata tables, collection types, and how honor asset keys are computed.

See also: [native renderer](native-renderer.md), [command line](cli.md) and
[architecture](architecture.md).

## Atlas directory

The native renderer samples text and shapes from SDF atlases.
`CustomProfileRenderer::with_atlas_dir` installs a directory laid out as
follows. `sekai_profile::prepare::prepare_atlas_dir` (and so
`sekai-profile prepare`) writes `text/` and `shape/`; `tiers/` is optional,
and `fallback/` is filled while rendering.

```text
atlases/text/<family>/manifest.json    one text atlas per font family
atlases/tiers/<name>/manifest.json     optional atlases of the same faces at larger point sizes
atlases/shape/manifest.json            the shape atlas
atlases/fallback/<font SHA-256>/       glyphs of the fallback face, generated while rendering
```

- A text atlas records the SHA-256 of the font file it was built from, and
  the font installed under the same family must match it, because glyphs
  larger than the atlas size are generated from that font while rendering.
- A tier atlas must extend a base text atlas of the same face at a smaller
  point size. With tiers installed, a glyph larger than every tier samples
  the largest tier that holds it instead of being generated.
- When `Source Han Sans SC` is installed, characters the card fonts lack
  are generated from it and cached under `fallback/`, one directory per font
  file.
- In a family's directory name, path separators, `:` and control characters
  become `_`.

The `text/` directory can also be served to the browser renderer as
prebuilt atlases: `createHttpPrebuiltSdfAtlasProvider(baseUrl)` reads
`<baseUrl>/<family>/manifest.json` and the page files next to it.

### Building atlases

`prepare_atlas_dir` builds a text atlas for every family of
`customProfileTextFonts` and a shape atlas from
`customProfileShapeResources`, reading each shape sprite from a callback.
An existing text atlas is kept when it was built from the installed font and
already holds the requested codepoints; otherwise it is rebuilt to cover
both. Every atlas is written to a temporary directory first and moved into
place when complete.

`CustomProfileRenderer::text_codepoints` gives the codepoints a card needs
per font family, and `PrepareOptions::codepoints` builds only those glyphs.
`PrepareOptions::force` rebuilds atlases that already cover the request, and
`PrepareOptions::skip_shapes` leaves the shape atlas alone. The returned
`PrepareReport` lists, per family, whether the atlas was `Built` or `Kept`,
and the families without a font file.

The `sekai-profile` crate also has these executables, run for example with
`cargo run --release -p sekai-profile --bin build-sdf-atlas -- <arguments>`.
Each prints its usage when run without arguments.

| Executable | Builds |
| --- | --- |
| `build-sdf-atlas` | One text atlas from one font family, with a chosen point size, spread, method and codepoint set |
| `build-shape-sdf-atlas` | The shape atlas from `customProfileShapeResources` and a directory of sprites |
| `build-render-object-store` | A store of pre-decoded textures, installed with `--render-objects` or `with_render_object_store` |

## Font files

The native renderer resolves a font family to a file name and looks for it,
in order, in:

1. the directories given to `sekai_profile::set_font_dirs` (`--font-dir` on
   the command line);
2. the directory named by `SEKAI_PROFILE_FONT_DIR`;
3. the directory named by `FONT_DIR`;
4. `assets/fonts`.

The first file found is used. The browser renderer instead requests each
family from the application's `FontProvider`, or takes fonts registered with
`registerFont`.

| Font family (CN server name) | File |
| --- | --- |
| `FOT-RodinNTLGPro-DB` (`FZLanTingHei-DB-GBK`) | `FOT-RodinNTLGPro-DB.ttf` or `.otf` |
| `FOT-SkipProN-B` (`FZZhengHei-EB-GBK`) | `FOT-SkipProN-B.otf` |
| `FOT-PopHappinessStd-EB` (`FZShaoEr-M11-JF`) | `FOT-PopHappinessStd-EB.otf` |
| `FOT-YurukaStd-UB` | `FOT-YurukaStd-UB.otf` |
| `Source Han Sans SC` (fallback face) | `SourceHanSansSC-Medium.otf` or `SourceHanSansSC-Regular.otf` |

With region `cn`, `JsonMasterDataProvider` resolves the masterdata font
names to the names in parentheses, which use the same files. The caller
supplies every font file.

### Omikuji fonts

The texts of omikuji fortune slips use the font asset `FOT-Omikuji` (2022
slips) or `FOT-UDMinchoPro-B` (2023 to 2025 slips). A character that font
has no glyph for comes from the first fallback face that has one: `Roboto`,
then the Noto Sans CJK face of the region the masterdata tables belong to.
The glyph keeps that face's bitmap offsets, size and advance, while padding,
tracking and line metrics stay the slip font's. An omikuji page needs its
slip font and both fallback fonts, and fails to render without any of
them.

| Family | File | Face |
| --- | --- | --- |
| `FOT-Omikuji` (2022 slips) | `FOT-Omikuji.otf` | 0 |
| `FOT-UDMinchoPro-B` (2023 to 2025 slips) | `FOT-UDMinchoPro-B.otf` | 0 |
| `Roboto` | `Roboto-Regular.ttf` | 0 |
| `Noto Sans CJK SC` (region `cn`, `en` and any other) | `NotoSansCJK-Regular.ttc` | 2 |
| `Noto Sans CJK TC` (region `tw`) | `NotoSansCJK-Regular.ttc` | 3 |
| `Noto Sans CJK JP` (region `jp`) | `NotoSansCJK-Regular.ttc` | 0 |
| `Noto Sans CJK KR` (region `kr`) | `NotoSansCJK-Regular.ttc` | 1 |

The native renderer finds each file in the font directories and opens the
face listed; the browser renderer requests each family's file from its
`FontProvider`. The two fallback files are the stock Android 14 system
fonts:

- `Roboto-Regular.ttf`, SHA-256
  `9ca9debb09459bf4e3e7f826f5cd0f35f253902b85684921fce2ba3f28dd0f50`;
- `NotoSansCJK-Regular.ttc`, SHA-256
  `39fb47c543da50618ab99e8b9e5529e54566bdbef41719308165975f627d5c93`.

## Masterdata tables

The native renderer reads the tables in
`sekai_profile::masterdata::REQUIRED_TABLES`:

`cards`, `stamps`, `honors`, `honorGroups`, `bondsHonors`,
`bondsHonorWords`, `gameCharacterUnits`, `customProfileTextColors`,
`customProfileTextFonts`, `customProfileShapeResources`,
`customProfileEtcResources`, `customProfileCollectionResources`,
`customProfileGeneralBackgroundResources`,
`customProfileMemberStandingPictureResources`,
`customProfilePlayerInfoResources`, `customProfileStoryBackgroundResources`,
`eventStories` and `unitStoryEpisodeGroups`.

The four tables in `sekai_profile::masterdata::OPTIONAL_TABLES` may be
absent: `customProfileCharacterIconResources`,
`customProfileMaterialResources` and
`customProfileUserInterfaceIconResources` are shipped by some regions only,
and `omikujis` is read only by omikuji collections. Without them the
`characterIcons`, `materials` and `userInterfaceIcons` elements and omikuji
collections are not drawn, and no missing-table warning is reported.

The browser renderer's `loadMasterData` requests the tables listed in
`RendererMasterData.requiredTables` and `optionalTables`, and marks each
request with `optional`.

## Collections

A collection element draws according to the
`customProfileResourceCollectionType` of its
`customProfileCollectionResources` row:

- `can_badge` rows draw their image through a lit badge material, which also
  needs the static asset `ui/sekai_badge_normal` (a normal map, listed in the
  CLI's static asset manifest);
- `omikuji` rows name a fortune-slip prefab, and the element's `targetId`
  picks the `omikujis` row it shows. Both renderers draw the slip's cover and
  fortune images and its vertical title, summary and description texts. The
  texts are rendered from FreeType coverage rather than SDF glyphs, with the
  layout and FreeType glyph settings of `sekai-profile-core`. The two images
  are requested as ordinary assets. A missing `targetId`, a `targetId` that
  names no row, and a slip prefab the core does not lay out draw nothing and
  request nothing;
- every other type, and a missing one, draws as a plain image.

## Asset keys

An asset key is the path of an image without its extension, such as
`honor/frame_degree_m_1`. The native `AssetStore` and the command line's
`--assets-dir` key each PNG by its path relative to the directory; the
browser `ResourceProvider` receives each key in a resource descriptor with
its `namespace`.

Honor asset keys are computed by `sekai-profile-core` and shared by the
native and browser renderers:

- `effective_honor_background_asset_bundle_name` picks the bundle of a
  standard honor's degree layer: an explicit background bundle when the
  honor names one; otherwise `honor_bg_event_cheerteam` for a `limitevent`
  honor whose bundle starts with `honor_top_`, and the honor's own bundle
  for every other honor.
- `bonds_honor_view_flags` decodes `bondsHonorViewType` as composable flags:
  one value may contain both `reverse` and `unit_virtual_singer`.
- `bonds_honor_asset_plan` lists the background, character, mask, frame,
  word and star assets of a bonds honor.

For URL layouts that differ from the asset key, see
[`--asset-url-layout`](cli.md#masterdata-and-assets).
