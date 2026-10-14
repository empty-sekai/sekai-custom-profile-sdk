//! Text SDF atlas construction.
//!
//! An atlas holds one R8 distance field per glyph of one font family, packed
//! onto square pages with a shelf packer and stored 8×8-block swizzled, the
//! layout [`MappedSdfAtlas`] maps and the browser's prebuilt atlas provider
//! serves unchanged.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use ttf_parser::Face;

use crate::sdf::atlas::{
    MappedSdfAtlas, SdfAtlasGenerationFailure, SdfAtlasGenerationReport, SdfAtlasGlyphManifest,
    SdfAtlasManifest, SdfAtlasPageManifest, ATLAS_MANIFEST_SCHEMA, SWIZZLED_BLOCK_HEIGHT,
    SWIZZLED_BLOCK_WIDTH, SWIZZLED_PAGE_HEADER_BYTES, SWIZZLED_PAGE_MAGIC, SWIZZLED_PAGE_VERSION,
};
use crate::sdf::outline::{
    self, OfflineAtlasGlyphGenerator, OfflineGenerationMethod, OutlineSdfGlyph,
};

/// Page edge, in texels, of a text atlas built with default options.
pub const DEFAULT_TEXT_PAGE_SIZE: u32 = 2048;
/// Empty texels kept around each glyph on a page.
pub const DEFAULT_TEXT_GUTTER: u32 = 1;

/// How to build one text atlas.
#[derive(Clone, Debug)]
pub struct TextAtlasOptions {
    /// Font family the atlas serves. Its font file is found the way the
    /// renderer finds it, through the configured font directories.
    pub font_family: String,
    /// Directory the atlas is written to. It must be empty or absent.
    pub output: PathBuf,
    /// Glyph SDF generation method; EDT at 2× supersampling by default.
    pub method: OfflineGenerationMethod,
    /// Sampling point size, finite and positive; the renderer's sampling
    /// point size by default.
    pub point_size: f32,
    /// Distance field spread in texels, finite and positive; the renderer's
    /// spread by default.
    pub spread: f32,
    /// Page edge in texels: a non-zero multiple of 8; 2048 by default.
    pub page_size: u32,
    /// Empty texels around each glyph, at most a quarter of the page size; 1
    /// by default.
    pub gutter: u32,
    /// Codepoints to include; every codepoint the font maps when `None`.
    /// Requested codepoints the font does not map are listed as failures in
    /// the manifest.
    pub codepoints: Option<BTreeSet<u32>>,
}

impl TextAtlasOptions {
    /// Options that build the atlas the renderer samples by default: the
    /// distance-transform generator at 2× supersampling, at the renderer's
    /// sampling point size and spread.
    pub fn new(font_family: impl Into<String>, output: impl Into<PathBuf>) -> Self {
        Self {
            font_family: font_family.into(),
            output: output.into(),
            method: OfflineGenerationMethod::Edt { supersample: 2 },
            point_size: outline::sampling_point_size(),
            spread: outline::sampling_spread(),
            page_size: DEFAULT_TEXT_PAGE_SIZE,
            gutter: DEFAULT_TEXT_GUTTER,
            codepoints: None,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.page_size == 0
            || !self.page_size.is_multiple_of(SWIZZLED_BLOCK_WIDTH)
            || !self.page_size.is_multiple_of(SWIZZLED_BLOCK_HEIGHT)
        {
            return Err("page size must be a non-zero multiple of 8".into());
        }
        if self.gutter > self.page_size / 4 {
            return Err("gutter is too large for the page size".into());
        }
        if !self.point_size.is_finite() || self.point_size <= 0.0 {
            return Err("point size must be finite and positive".into());
        }
        if !self.spread.is_finite() || self.spread <= 0.0 {
            return Err("spread must be finite and positive".into());
        }
        if self.codepoints.as_ref().is_some_and(BTreeSet::is_empty) {
            return Err("the requested codepoint set is empty".into());
        }
        Ok(())
    }
}

/// Builds a text atlas and returns the path of its `manifest.json`.
///
/// The written atlas is reopened and checked against the manifest before
/// this returns.
pub fn build_text_atlas(options: &TextAtlasOptions) -> Result<PathBuf, String> {
    options.validate()?;
    super::ensure_empty_dir(&options.output)?;
    let font_path = outline::resolve_font_path(&options.font_family)
        .ok_or_else(|| format!("no font file found for family {}", options.font_family))?;
    let font_bytes = fs::read(&font_path)
        .map_err(|error| format!("read font {} failed: {error}", font_path.display()))?;
    let font_sha256 = hex::encode(Sha256::digest(&font_bytes));
    let cmap_codepoints = font_codepoints(&font_bytes)?;
    let requested = options
        .codepoints
        .clone()
        .unwrap_or_else(|| cmap_codepoints.clone());
    if requested.is_empty() {
        return Err(format!("font {} maps no codepoint", font_path.display()));
    }

    let size = options.page_size;
    let gutter = options.gutter;
    let mut pages = Vec::new();
    let mut page = ShelfPage::new(size)?;
    let mut glyphs = Vec::with_capacity(requested.len());
    let mut failures = Vec::new();
    let mut fallback_codepoints = Vec::new();
    let generator = OfflineAtlasGlyphGenerator::new_at_sampling(
        &options.font_family,
        options.point_size,
        options.spread,
    )?;

    for codepoint in requested.iter().copied() {
        if !cmap_codepoints.contains(&codepoint) {
            failures.push(SdfAtlasGenerationFailure {
                codepoint,
                reason: "codepoint is absent from the font cmap".into(),
            });
            continue;
        }
        let Some(ch) = char::from_u32(codepoint) else {
            failures.push(SdfAtlasGenerationFailure {
                codepoint,
                reason: "codepoint is not a Unicode scalar value".into(),
            });
            continue;
        };
        let (glyph, used_fallback) = match generator.generate(ch, options.method) {
            Ok(result) => result,
            Err(reason) => {
                failures.push(SdfAtlasGenerationFailure { codepoint, reason });
                continue;
            }
        };
        let glyph_width = u32::try_from(glyph.width())
            .map_err(|_| format!("glyph U+{codepoint:04X} width overflow"))?;
        let glyph_height = u32::try_from(glyph.height())
            .map_err(|_| format!("glyph U+{codepoint:04X} height overflow"))?;
        if !fits(size, glyph_width, glyph_height, gutter) {
            failures.push(SdfAtlasGenerationFailure {
                codepoint,
                reason: format!(
                    "glyph {glyph_width}x{glyph_height} plus gutter does not fit a {size}x{size} page"
                ),
            });
            continue;
        }
        let mut origin = page.place(size, glyph_width, glyph_height, gutter);
        if origin.is_none() {
            pages.push(write_page(
                &options.output,
                pages.len(),
                size,
                &page.pixels,
            )?);
            page = ShelfPage::new(size)?;
            origin = page.place(size, glyph_width, glyph_height, gutter);
        }
        let origin =
            origin.ok_or_else(|| format!("a fresh page rejected glyph U+{codepoint:04X}"))?;
        page.copy_glyph(size, origin, &glyph);
        if used_fallback {
            fallback_codepoints.push(codepoint);
        }
        glyphs.push(SdfAtlasGlyphManifest {
            codepoint,
            page: u16::try_from(pages.len())
                .map_err(|_| "atlas page count exceeds u16".to_string())?,
            rect: [origin[0], origin[1], glyph_width, glyph_height],
            plane_bearing: [glyph.plane_bearing_x(), glyph.plane_bearing_y()],
            plane_size: [glyph.plane_width(), glyph.plane_height()],
            plane_advance_x: glyph.plane_advance_x(),
        });
    }
    if glyphs.is_empty() {
        return Err(format!(
            "no glyph of family {} could be generated",
            options.font_family
        ));
    }
    pages.push(write_page(
        &options.output,
        pages.len(),
        size,
        &page.pixels,
    )?);

    let count = |value: usize, what: &str| {
        u32::try_from(value).map_err(|_| format!("{what} count exceeds u32"))
    };
    let manifest = SdfAtlasManifest {
        schema: ATLAS_MANIFEST_SCHEMA.into(),
        generator_contract: options.method.contract(),
        font_family: options.font_family.clone(),
        font_sha256,
        point_size: options.point_size,
        spread: options.spread,
        pages,
        generation: SdfAtlasGenerationReport {
            cmap_codepoint_count: count(cmap_codepoints.len(), "cmap codepoint")?,
            requested_codepoint_count: count(requested.len(), "requested codepoint")?,
            generated_glyph_count: count(glyphs.len(), "generated glyph")?,
            failed_glyph_count: count(failures.len(), "failed glyph")?,
            analytic_fallback_count: count(fallback_codepoints.len(), "fallback glyph")?,
            page_width: size,
            page_height: size,
            gutter,
            failures,
            analytic_fallback_codepoints: fallback_codepoints,
        },
        glyphs,
    };
    let manifest_path = options.output.join(super::MANIFEST_FILE);
    super::write_manifest(&manifest_path, &manifest)?;
    let reopened = MappedSdfAtlas::open(&manifest_path)
        .map_err(|error| format!("reopening the written atlas failed: {error}"))?;
    if reopened.manifest() != &manifest {
        return Err("the reopened atlas manifest differs from the written one".into());
    }
    Ok(manifest_path)
}

/// Every Unicode scalar value the font's Unicode cmap subtables map.
pub fn font_codepoints(font_bytes: &[u8]) -> Result<BTreeSet<u32>, String> {
    let face =
        Face::parse(font_bytes, 0).map_err(|error| format!("parse font failed: {error:?}"))?;
    let cmap = face
        .tables()
        .cmap
        .ok_or_else(|| "font has no cmap table".to_string())?;
    let mut codepoints = BTreeSet::new();
    for subtable in cmap.subtables {
        if subtable.is_unicode() {
            subtable.codepoints(|codepoint| {
                if char::from_u32(codepoint).is_some() {
                    codepoints.insert(codepoint);
                }
            });
        }
    }
    Ok(codepoints)
}

fn fits(size: u32, width: u32, height: u32, gutter: u32) -> bool {
    let padding = gutter.saturating_mul(2);
    width.saturating_add(padding) <= size && height.saturating_add(padding) <= size
}

#[derive(Debug)]
struct ShelfPage {
    pixels: Vec<u8>,
    cursor_x: u32,
    cursor_y: u32,
    shelf_height: u32,
}

impl ShelfPage {
    fn new(size: u32) -> Result<Self, String> {
        let len = usize::try_from(size)
            .ok()
            .and_then(|value| value.checked_mul(value))
            .ok_or_else(|| "atlas page size overflow".to_string())?;
        Ok(Self {
            pixels: vec![0; len],
            cursor_x: 0,
            cursor_y: 0,
            shelf_height: 0,
        })
    }

    fn place(
        &mut self,
        size: u32,
        glyph_width: u32,
        glyph_height: u32,
        gutter: u32,
    ) -> Option<[u32; 2]> {
        let packed_width = glyph_width.checked_add(gutter.checked_mul(2)?)?;
        let packed_height = glyph_height.checked_add(gutter.checked_mul(2)?)?;
        if packed_width > size || packed_height > size {
            return None;
        }
        if self.cursor_x.checked_add(packed_width)? > size {
            self.cursor_x = 0;
            self.cursor_y = self.cursor_y.checked_add(self.shelf_height)?;
            self.shelf_height = 0;
        }
        if self.cursor_y.checked_add(packed_height)? > size {
            return None;
        }
        let placed = [self.cursor_x + gutter, self.cursor_y + gutter];
        self.cursor_x += packed_width;
        self.shelf_height = self.shelf_height.max(packed_height);
        Some(placed)
    }

    fn copy_glyph(&mut self, size: u32, origin: [u32; 2], glyph: &OutlineSdfGlyph) {
        let width = glyph.width();
        let x = origin[0] as usize;
        let y = origin[1] as usize;
        let stride = size as usize;
        for row in 0..glyph.height() {
            let destination = (y + row) * stride + x;
            self.pixels[destination..destination + width]
                .copy_from_slice(&glyph.pixels()[row * width..(row + 1) * width]);
        }
    }
}

fn swizzle_page(linear: &[u8], size: u32) -> Vec<u8> {
    let mut swizzled = vec![0; linear.len()];
    let blocks_per_row = size / SWIZZLED_BLOCK_WIDTH;
    for y in 0..size {
        for x in 0..size {
            let block = (y / SWIZZLED_BLOCK_HEIGHT) * blocks_per_row + x / SWIZZLED_BLOCK_WIDTH;
            let in_block =
                (y % SWIZZLED_BLOCK_HEIGHT) * SWIZZLED_BLOCK_WIDTH + x % SWIZZLED_BLOCK_WIDTH;
            swizzled[(block * 64 + in_block) as usize] = linear[(y * size + x) as usize];
        }
    }
    swizzled
}

fn write_page(
    output: &Path,
    index: usize,
    size: u32,
    linear: &[u8],
) -> Result<SdfAtlasPageManifest, String> {
    let payload = swizzle_page(linear, size);
    let source_hash = Sha256::digest(linear);
    let mut bytes = vec![0; SWIZZLED_PAGE_HEADER_BYTES];
    bytes[..SWIZZLED_PAGE_MAGIC.len()].copy_from_slice(SWIZZLED_PAGE_MAGIC);
    bytes[12..16].copy_from_slice(&SWIZZLED_PAGE_VERSION.to_le_bytes());
    bytes[16..20].copy_from_slice(&size.to_le_bytes());
    bytes[20..24].copy_from_slice(&size.to_le_bytes());
    bytes[24..28].copy_from_slice(&SWIZZLED_BLOCK_WIDTH.to_le_bytes());
    bytes[28..32].copy_from_slice(&SWIZZLED_BLOCK_HEIGHT.to_le_bytes());
    bytes[32..64].copy_from_slice(&source_hash);
    bytes.extend_from_slice(&payload);
    let file = format!("page-{index:03}.r8swz");
    fs::write(output.join(&file), &bytes)
        .map_err(|error| format!("write atlas page {file} failed: {error}"))?;
    Ok(SdfAtlasPageManifest {
        file,
        width: size,
        height: size,
        file_sha256: hex::encode(Sha256::digest(&bytes)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swizzle_matches_scalar_block_addressing() {
        let size = 16;
        let linear = (0..size * size)
            .map(|value| value as u8)
            .collect::<Vec<_>>();
        let swizzled = swizzle_page(&linear, size);
        for y in 0..size {
            for x in 0..size {
                let block = (y / 8) * (size / 8) + x / 8;
                let offset = block * 64 + (y % 8) * 8 + x % 8;
                assert_eq!(swizzled[offset as usize], linear[(y * size + x) as usize]);
            }
        }
    }

    #[test]
    fn shelf_packer_preserves_gutter_and_starts_new_shelf() {
        let mut page = ShelfPage::new(16).expect("page");
        assert_eq!(page.place(16, 5, 4, 1), Some([1, 1]));
        assert_eq!(page.place(16, 5, 4, 1), Some([8, 1]));
        assert_eq!(page.place(16, 5, 4, 1), Some([1, 7]));
    }

    /// The browser's prebuilt atlas provider serves this output unchanged:
    /// it expects `page-NNN.r8swz` files whose hash covers the whole file.
    #[test]
    fn pages_are_named_and_hashed_as_browser_packages_expect() {
        let output = tempfile::tempdir().expect("output directory");
        let page = write_page(output.path(), 7, 16, &[0; 16 * 16]).expect("page");
        assert_eq!(page.file, "page-007.r8swz");
        assert_eq!((page.width, page.height), (16, 16));
        let bytes = fs::read(output.path().join(&page.file)).expect("page bytes");
        assert_eq!(bytes.len(), SWIZZLED_PAGE_HEADER_BYTES + 16 * 16);
        assert_eq!(&bytes[..SWIZZLED_PAGE_MAGIC.len()], SWIZZLED_PAGE_MAGIC);
        assert_eq!(page.file_sha256, hex::encode(Sha256::digest(&bytes)));
    }

    #[test]
    fn invalid_options_are_rejected_before_any_file_is_written() {
        let output = tempfile::tempdir().expect("output directory");
        let mut options = TextAtlasOptions::new("DejaVu Sans", output.path().join("atlas"));
        options.page_size = 12;
        assert!(build_text_atlas(&options).is_err());
        options.page_size = 64;
        options.codepoints = Some(BTreeSet::new());
        assert!(build_text_atlas(&options).is_err());
        assert!(!output.path().join("atlas").exists());
    }

    #[test]
    fn builds_a_mappable_atlas_for_requested_codepoints() {
        if outline::resolve_font_path("DejaVu Sans").is_none() {
            eprintln!("skipping: DejaVu Sans is not installed");
            return;
        }
        let output = tempfile::tempdir().expect("output directory");
        let mut options = TextAtlasOptions::new("DejaVu Sans", output.path().join("atlas"));
        options.page_size = 256;
        options.codepoints = Some([u32::from('A'), u32::from('b'), 0x10_FFFD].into());
        let manifest = build_text_atlas(&options).expect("build atlas");
        let atlas = MappedSdfAtlas::open(&manifest).expect("open atlas");
        assert_eq!(atlas.manifest().font_family, "DejaVu Sans");
        assert!(atlas.glyph(u32::from('A')).is_some());
        assert!(atlas.glyph(u32::from('b')).is_some());
        let report = &atlas.manifest().generation;
        assert_eq!(report.requested_codepoint_count, 3);
        assert_eq!(report.generated_glyph_count, 2);
        assert_eq!(report.failures[0].codepoint, 0x10_FFFD);
    }
}
