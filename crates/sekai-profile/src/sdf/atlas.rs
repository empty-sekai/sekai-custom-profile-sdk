//! Versioned, mmap-backed SDF atlas contract used by the SIMD executor.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use memmap2::{Mmap, MmapOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const ATLAS_MANIFEST_SCHEMA: &str = "allium.sdf-atlas-manifest.v1";
pub const SWIZZLED_PAGE_MAGIC: &[u8; 10] = b"ALLIUMSWZ8";
pub const SWIZZLED_PAGE_VERSION: u32 = 1;
pub const SWIZZLED_PAGE_HEADER_BYTES: usize = 64;
pub const SWIZZLED_BLOCK_WIDTH: u32 = 8;
pub const SWIZZLED_BLOCK_HEIGHT: u32 = 8;
/// Sparse global fallback chosen after a selected profile font misses a
/// codepoint. Glyphs are generated from this font on demand and persisted.
pub const PROFILE_TEXT_FALLBACK_FONT_FAMILY: &str = "Source Han Sans SC";
pub const PROFILE_TEXT_FALLBACK_FONT_FAMILIES: &[&str] = &[PROFILE_TEXT_FALLBACK_FONT_FAMILY];
const SDF_PREWARM_PAGE_STRIDE: usize = 4096;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct SdfAtlasPrewarmReport {
    pub atlas_count: u64,
    pub page_count: u64,
    pub mapped_bytes: u64,
    pub page_touch_count: u64,
    pub checksum: u64,
}

pub(crate) fn prewarm_mapped_page(bytes: &[u8], report: &mut SdfAtlasPrewarmReport) {
    report.page_count = report.page_count.saturating_add(1);
    report.mapped_bytes = report
        .mapped_bytes
        .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
    for value in bytes.iter().step_by(SDF_PREWARM_PAGE_STRIDE) {
        report.checksum = report.checksum.rotate_left(1) ^ u64::from(*value);
        report.page_touch_count = report.page_touch_count.saturating_add(1);
    }
    if bytes.len() > 1 && !(bytes.len() - 1).is_multiple_of(SDF_PREWARM_PAGE_STRIDE) {
        report.checksum = report.checksum.rotate_left(1) ^ u64::from(bytes[bytes.len() - 1]);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SdfAtlasManifest {
    pub schema: String,
    pub generator_contract: String,
    pub font_family: String,
    pub font_sha256: String,
    pub point_size: f32,
    pub spread: f32,
    pub pages: Vec<SdfAtlasPageManifest>,
    pub glyphs: Vec<SdfAtlasGlyphManifest>,
    pub generation: SdfAtlasGenerationReport,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SdfAtlasGenerationReport {
    pub cmap_codepoint_count: u32,
    pub requested_codepoint_count: u32,
    pub generated_glyph_count: u32,
    pub failed_glyph_count: u32,
    pub analytic_fallback_count: u32,
    pub page_width: u32,
    pub page_height: u32,
    pub gutter: u32,
    pub failures: Vec<SdfAtlasGenerationFailure>,
    pub analytic_fallback_codepoints: Vec<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SdfAtlasGenerationFailure {
    pub codepoint: u32,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SdfAtlasPageManifest {
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub file_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SdfAtlasGlyphManifest {
    pub codepoint: u32,
    pub page: u16,
    /// Linear texel coordinates `[x, y, width, height]` before 8x8 swizzling.
    pub rect: [u32; 4],
    pub plane_bearing: [f32; 2],
    pub plane_size: [f32; 2],
    pub plane_advance_x: f32,
}

#[derive(Debug, Error)]
pub enum SdfAtlasError {
    #[error("atlas manifest I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("atlas manifest JSON failed for {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid atlas contract: {0}")]
    Invalid(String),
    #[error("atlas page hash mismatch for {path}: expected {expected}, got {actual}")]
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
}

pub struct MappedSdfAtlasPage {
    width: u32,
    height: u32,
    mapping: Mmap,
}

impl MappedSdfAtlasPage {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn mapped_bytes(&self) -> usize {
        self.mapping.len()
    }

    pub fn swizzled_payload(&self) -> &[u8] {
        &self.mapping[SWIZZLED_PAGE_HEADER_BYTES..]
    }

    /// Scalar oracle for the physical 8x8 layout. The SIMD executor consumes
    /// `swizzled_payload()` directly and performs the equivalent address math.
    pub fn texel(&self, x: u32, y: u32) -> Option<u8> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let blocks_per_row = self.width / SWIZZLED_BLOCK_WIDTH;
        let block = (y / SWIZZLED_BLOCK_HEIGHT) * blocks_per_row + x / SWIZZLED_BLOCK_WIDTH;
        let in_block =
            (y % SWIZZLED_BLOCK_HEIGHT) * SWIZZLED_BLOCK_WIDTH + x % SWIZZLED_BLOCK_WIDTH;
        self.swizzled_payload()
            .get((block * 64 + in_block) as usize)
            .copied()
    }
}

pub struct MappedSdfAtlas {
    manifest: SdfAtlasManifest,
    manifest_sha256: String,
    pages: Vec<MappedSdfAtlasPage>,
    glyph_by_codepoint: BTreeMap<u32, usize>,
}

/// Immutable process-level atlas registry. A command stores the returned
/// `atlas_set` id, so page numbers from different font atlases never alias.
#[derive(Clone, Default)]
pub struct MappedSdfAtlasSet {
    atlases: Vec<Arc<MappedSdfAtlas>>,
    by_font_family: BTreeMap<String, Vec<u16>>,
}

impl MappedSdfAtlasSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, atlas: Arc<MappedSdfAtlas>) -> Result<u16, SdfAtlasError> {
        let family = atlas.manifest().font_family.clone();
        if let Some(existing_ids) = self.by_font_family.get(&family) {
            for existing_id in existing_ids.iter().copied() {
                let existing = self
                    .atlases
                    .get(usize::from(existing_id))
                    .ok_or_else(|| SdfAtlasError::Invalid("corrupt atlas registry".into()))?;
                if existing.manifest() == atlas.manifest() {
                    return Ok(existing_id);
                }
                if existing.manifest().font_sha256 != atlas.manifest().font_sha256 {
                    return Err(SdfAtlasError::Invalid(format!(
                        "font family {family} has conflicting font identities across sampling tiers"
                    )));
                }
                if existing.manifest().point_size.to_bits() == atlas.manifest().point_size.to_bits()
                {
                    return Err(SdfAtlasError::Invalid(format!(
                        "font family {family} point size {} has conflicting atlas identities",
                        atlas.manifest().point_size
                    )));
                }
            }
        }
        let id = u16::try_from(self.atlases.len())
            .map_err(|_| SdfAtlasError::Invalid("too many atlas sets".into()))?;
        self.atlases.push(atlas);
        let atlases = &self.atlases;
        let ids = self.by_font_family.entry(family).or_default();
        ids.push(id);
        ids.sort_by(|left, right| {
            atlases[usize::from(*left)]
                .manifest()
                .point_size
                .total_cmp(&atlases[usize::from(*right)].manifest().point_size)
        });
        Ok(id)
    }

    pub fn replace_or_insert(&mut self, atlas: Arc<MappedSdfAtlas>) -> Result<u16, SdfAtlasError> {
        let family = atlas.manifest().font_family.clone();
        if let Some(existing_ids) = self.by_font_family.get(&family) {
            for existing_id in existing_ids.iter().copied() {
                let slot = self
                    .atlases
                    .get_mut(usize::from(existing_id))
                    .ok_or_else(|| SdfAtlasError::Invalid("corrupt atlas registry".into()))?;
                if slot.manifest().point_size.to_bits() == atlas.manifest().point_size.to_bits() {
                    *slot = atlas;
                    return Ok(existing_id);
                }
            }
        }
        self.insert(atlas)
    }

    pub fn atlas(&self, atlas_set: u16) -> Option<&MappedSdfAtlas> {
        self.atlases.get(usize::from(atlas_set)).map(Arc::as_ref)
    }

    pub fn atlas_for_font_family(&self, family: &str) -> Option<(u16, &MappedSdfAtlas)> {
        let id = *self.by_font_family.get(family)?.first()?;
        Some((id, self.atlas(id)?))
    }

    pub fn atlas_for_font_family_at_least(
        &self,
        family: &str,
        minimum_point_size: f32,
    ) -> Option<(u16, &MappedSdfAtlas)> {
        self.by_font_family
            .get(family)?
            .iter()
            .copied()
            .filter_map(|id| Some((id, self.atlas(id)?)))
            .find(|(_, atlas)| atlas.manifest().point_size >= minimum_point_size)
    }

    /// Select the smallest sampling tier at or above `minimum_point_size`
    /// which actually contains `codepoint`. Sparse high-resolution tiers are
    /// intentionally allowed to omit most of a font's cmap.
    pub fn glyph_for_font_family_at_least(
        &self,
        family: &str,
        codepoint: u32,
        minimum_point_size: f32,
    ) -> Option<(u16, &MappedSdfAtlas, &SdfAtlasGlyphManifest)> {
        self.by_font_family
            .get(family)?
            .iter()
            .copied()
            .filter_map(|id| Some((id, self.atlas(id)?)))
            .filter(|(_, atlas)| atlas.manifest().point_size >= minimum_point_size)
            .find_map(|(id, atlas)| atlas.glyph(codepoint).map(|glyph| (id, atlas, glyph)))
    }

    /// Best available higher-resolution tier for an oversized glyph when no
    /// installed sparse tier reaches the requested sampling size. This keeps
    /// the complete 3x tier as a deterministic no-generation fallback.
    pub fn highest_glyph_for_font_family_above(
        &self,
        family: &str,
        codepoint: u32,
        minimum_exclusive_point_size: f32,
    ) -> Option<(u16, &MappedSdfAtlas, &SdfAtlasGlyphManifest)> {
        self.by_font_family
            .get(family)?
            .iter()
            .rev()
            .copied()
            .filter_map(|id| Some((id, self.atlas(id)?)))
            .filter(|(_, atlas)| atlas.manifest().point_size > minimum_exclusive_point_size)
            .find_map(|(id, atlas)| atlas.glyph(codepoint).map(|glyph| (id, atlas, glyph)))
    }

    pub fn profile_glyph_for_font_family(
        &self,
        primary_family: &str,
        codepoint: u32,
    ) -> Option<(u16, &MappedSdfAtlas, &SdfAtlasGlyphManifest)> {
        std::iter::once(primary_family)
            .chain(PROFILE_TEXT_FALLBACK_FONT_FAMILIES.iter().copied())
            .filter_map(|family| self.atlas_for_font_family(family))
            .find_map(|(atlas_set, atlas)| {
                atlas
                    .glyph(codepoint)
                    .map(|glyph| (atlas_set, atlas, glyph))
            })
    }

    pub fn len(&self) -> usize {
        self.atlases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.atlases.is_empty()
    }

    pub fn mapped_bytes(&self) -> u64 {
        self.atlases
            .iter()
            .flat_map(|atlas| atlas.pages())
            .map(|page| page.mapped_bytes() as u64)
            .fold(0u64, u64::saturating_add)
    }

    pub fn prewarm_pages(&self) -> SdfAtlasPrewarmReport {
        let mut report = SdfAtlasPrewarmReport {
            atlas_count: self.atlases.len() as u64,
            ..SdfAtlasPrewarmReport::default()
        };
        for atlas in &self.atlases {
            for page in atlas.pages() {
                prewarm_mapped_page(&page.mapping, &mut report);
            }
        }
        std::hint::black_box(report)
    }

    /// Touch only the smallest sampling tier for each font family. Higher
    /// resolution tiers remain mmap-lazy so their full file size never turns
    /// into startup RSS merely because they are installed.
    pub fn prewarm_primary_pages(&self) -> SdfAtlasPrewarmReport {
        let primary_ids = self
            .by_font_family
            .values()
            .filter_map(|ids| ids.first().copied())
            .collect::<Vec<_>>();
        let mut report = SdfAtlasPrewarmReport {
            atlas_count: primary_ids.len() as u64,
            ..SdfAtlasPrewarmReport::default()
        };
        for id in primary_ids {
            if let Some(atlas) = self.atlas(id) {
                for page in atlas.pages() {
                    prewarm_mapped_page(&page.mapping, &mut report);
                }
            }
        }
        std::hint::black_box(report)
    }

    pub fn iter(&self) -> impl Iterator<Item = &MappedSdfAtlas> {
        self.atlases.iter().map(Arc::as_ref)
    }
}

impl MappedSdfAtlas {
    pub fn open(manifest_path: impl AsRef<Path>) -> Result<Self, SdfAtlasError> {
        Self::open_with_page_hash_verification(manifest_path, true)
    }

    /// Open an atlas embedded in a content-addressed, read-only image that was
    /// already fully verified by the offline builder. Manifest structure,
    /// page paths, lengths and headers are still validated, but page payloads
    /// are not read solely to repeat their SHA-256 at process startup.
    pub fn open_trusted_immutable_artifact(
        manifest_path: impl AsRef<Path>,
    ) -> Result<Self, SdfAtlasError> {
        Self::open_with_page_hash_verification(manifest_path, false)
    }

    fn open_with_page_hash_verification(
        manifest_path: impl AsRef<Path>,
        verify_page_hashes: bool,
    ) -> Result<Self, SdfAtlasError> {
        let manifest_path = manifest_path.as_ref();
        let manifest_bytes = std::fs::read(manifest_path).map_err(|source| SdfAtlasError::Io {
            path: manifest_path.to_path_buf(),
            source,
        })?;
        let manifest: SdfAtlasManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|source| SdfAtlasError::Json {
                path: manifest_path.to_path_buf(),
                source,
            })?;
        validate_manifest(&manifest)?;
        let root = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let mut pages = Vec::with_capacity(manifest.pages.len());
        for page in &manifest.pages {
            pages.push(map_page(root, page, verify_page_hashes)?);
        }
        let glyph_by_codepoint = manifest
            .glyphs
            .iter()
            .enumerate()
            .map(|(index, glyph)| (glyph.codepoint, index))
            .collect();
        Ok(Self {
            manifest,
            manifest_sha256: hex::encode(Sha256::digest(&manifest_bytes)),
            pages,
            glyph_by_codepoint,
        })
    }

    pub fn manifest(&self) -> &SdfAtlasManifest {
        &self.manifest
    }

    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    pub fn pages(&self) -> &[MappedSdfAtlasPage] {
        &self.pages
    }

    pub fn glyph(&self, codepoint: u32) -> Option<&SdfAtlasGlyphManifest> {
        self.glyph_by_codepoint
            .get(&codepoint)
            .map(|index| &self.manifest.glyphs[*index])
    }
}

fn validate_manifest(manifest: &SdfAtlasManifest) -> Result<(), SdfAtlasError> {
    if manifest.schema != ATLAS_MANIFEST_SCHEMA {
        return Err(SdfAtlasError::Invalid(format!(
            "unsupported manifest schema {}",
            manifest.schema
        )));
    }
    if manifest.generator_contract.trim().is_empty()
        || manifest.font_family.trim().is_empty()
        || manifest.font_sha256.len() != 64
        || hex::decode(&manifest.font_sha256).is_err()
    {
        return Err(SdfAtlasError::Invalid(
            "missing generator/font identity".into(),
        ));
    }
    if !manifest.point_size.is_finite()
        || manifest.point_size <= 0.0
        || !manifest.spread.is_finite()
        || manifest.spread <= 0.0
        || manifest.pages.is_empty()
    {
        return Err(SdfAtlasError::Invalid(
            "invalid point size, spread or page count".into(),
        ));
    }
    for page in &manifest.pages {
        let path = Path::new(&page.file);
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
            || page.width == 0
            || page.height == 0
            || !page.width.is_multiple_of(SWIZZLED_BLOCK_WIDTH)
            || !page.height.is_multiple_of(SWIZZLED_BLOCK_HEIGHT)
            || page.file_sha256.len() != 64
            || hex::decode(&page.file_sha256).is_err()
        {
            return Err(SdfAtlasError::Invalid(format!(
                "invalid page descriptor {}",
                page.file
            )));
        }
    }
    let report = &manifest.generation;
    if report.page_width == 0
        || report.page_height == 0
        || !report.page_width.is_multiple_of(SWIZZLED_BLOCK_WIDTH)
        || !report.page_height.is_multiple_of(SWIZZLED_BLOCK_HEIGHT)
        || report.generated_glyph_count as usize != manifest.glyphs.len()
        || report.failed_glyph_count as usize != report.failures.len()
        || report.analytic_fallback_count as usize != report.analytic_fallback_codepoints.len()
        || report
            .generated_glyph_count
            .checked_add(report.failed_glyph_count)
            != Some(report.requested_codepoint_count)
        // Full atlases use one fixed page size; sparse fallback atlases use a
        // minimal block-aligned page per glyph. The report therefore records
        // the maximum page extent and every page must fit within it.
        || manifest
            .pages
            .iter()
            .any(|page| page.width > report.page_width || page.height > report.page_height)
        || report
            .failures
            .iter()
            .any(|failure| char::from_u32(failure.codepoint).is_none())
        || report
            .analytic_fallback_codepoints
            .iter()
            .any(|codepoint| char::from_u32(*codepoint).is_none())
    {
        return Err(SdfAtlasError::Invalid(
            "inconsistent atlas generation report".into(),
        ));
    }
    let mut seen = BTreeMap::new();
    for glyph in &manifest.glyphs {
        if char::from_u32(glyph.codepoint).is_none()
            || usize::from(glyph.page) >= manifest.pages.len()
            || seen.insert(glyph.codepoint, ()).is_some()
            || glyph.rect[2] == 0
            || glyph.rect[3] == 0
            || glyph
                .plane_bearing
                .iter()
                .chain(glyph.plane_size.iter())
                .chain(std::iter::once(&glyph.plane_advance_x))
                .any(|value| !value.is_finite())
        {
            return Err(SdfAtlasError::Invalid(format!(
                "invalid or duplicate glyph U+{:04X}",
                glyph.codepoint
            )));
        }
        let page = &manifest.pages[usize::from(glyph.page)];
        let [x, y, width, height] = glyph.rect;
        if x.checked_add(width).is_none_or(|right| right > page.width)
            || y.checked_add(height)
                .is_none_or(|bottom| bottom > page.height)
        {
            return Err(SdfAtlasError::Invalid(format!(
                "glyph U+{:04X} exceeds page {}",
                glyph.codepoint, glyph.page
            )));
        }
    }
    Ok(())
}

fn map_page(
    root: &Path,
    descriptor: &SdfAtlasPageManifest,
    verify_page_hash: bool,
) -> Result<MappedSdfAtlasPage, SdfAtlasError> {
    let path = root.join(&descriptor.file);
    let file = File::open(&path).map_err(|source| SdfAtlasError::Io {
        path: path.clone(),
        source,
    })?;
    // SAFETY: the mapping is read-only and owns no writable alias. Atlas files
    // are immutable, content-addressed artifacts for the lifetime of a worker.
    let mapping = unsafe { MmapOptions::new().map(&file) }.map_err(|source| SdfAtlasError::Io {
        path: path.clone(),
        source,
    })?;
    let expected_len = SWIZZLED_PAGE_HEADER_BYTES
        .checked_add(descriptor.width as usize * descriptor.height as usize)
        .ok_or_else(|| SdfAtlasError::Invalid("atlas page length overflow".into()))?;
    if mapping.len() != expected_len
        || mapping.get(..SWIZZLED_PAGE_MAGIC.len()) != Some(SWIZZLED_PAGE_MAGIC)
        || read_u32(&mapping, 12) != Some(SWIZZLED_PAGE_VERSION)
        || read_u32(&mapping, 16) != Some(descriptor.width)
        || read_u32(&mapping, 20) != Some(descriptor.height)
        || read_u32(&mapping, 24) != Some(SWIZZLED_BLOCK_WIDTH)
        || read_u32(&mapping, 28) != Some(SWIZZLED_BLOCK_HEIGHT)
    {
        return Err(SdfAtlasError::Invalid(format!(
            "invalid swizzled page header {}",
            path.display()
        )));
    }
    if verify_page_hash {
        let actual = hex::encode(Sha256::digest(&mapping));
        if actual != descriptor.file_sha256 {
            return Err(SdfAtlasError::HashMismatch {
                path,
                expected: descriptor.file_sha256.clone(),
                actual,
            });
        }
        // Hash validation necessarily faults the complete mapping once.
        // Release those clean pages before the explicit primary-tier prewarm.
        #[cfg(target_os = "linux")]
        // SAFETY: the digest borrow ended above; this is a read-only file
        // mapping, so refaulting restores the same immutable verified bytes.
        unsafe {
            let _ = mapping.unchecked_advise(memmap2::UncheckedAdvice::DontNeed);
        }
    }
    Ok(MappedSdfAtlasPage {
        width: descriptor.width,
        height: descriptor.height,
        mapping,
    })
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset + 4)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
}

#[cfg(test)]
mod tests;
