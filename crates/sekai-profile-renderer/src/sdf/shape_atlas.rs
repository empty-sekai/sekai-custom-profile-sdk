//! Versioned mmap-backed RG8 atlas for custom-profile Shape distance fields.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Component, Path, PathBuf};

use memmap2::{Mmap, MmapOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::atlas::{prewarm_mapped_page, SdfAtlasPrewarmReport};
use super::shape::ShapeSdfTexel;

pub const SHAPE_ATLAS_MANIFEST_SCHEMA: &str = "allium.shape-sdf-atlas-manifest.v2";
pub const SHAPE_ATLAS_GENERATOR_CONTRACT: &str = "allium.shape-sdf-rg8-copy.v2";
pub const SHAPE_ATLAS_PIXEL_FORMAT: &str = "rg8-distance-alpha";
pub const SHAPE_PAGE_MAGIC: &[u8; 10] = b"ALLIUMRG8S";
pub const SHAPE_PAGE_VERSION: u32 = 1;
pub const SHAPE_PAGE_HEADER_BYTES: usize = 64;
pub const SHAPE_BLOCK_WIDTH: u32 = 8;
pub const SHAPE_BLOCK_HEIGHT: u32 = 8;
pub const SHAPE_CHANNELS: u32 = 2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShapeSdfAtlasManifest {
    pub schema: String,
    pub generator_contract: String,
    pub pixel_format: String,
    pub pages: Vec<ShapeSdfAtlasPageManifest>,
    pub shapes: Vec<ShapeSdfAtlasEntry>,
    pub generation: ShapeSdfAtlasGenerationReport,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShapeSdfAtlasPageManifest {
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub file_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShapeSdfAtlasEntry {
    pub shape_id: i32,
    pub asset_key: String,
    pub source_sha256: String,
    /// SHA-256 of canonical row-major `[distance, alpha_gate]` texels. This can
    /// be recomputed from the decoded runtime image, unlike encoded-file SHA.
    pub source_rg8_sha256: String,
    pub page: u16,
    /// Linear texel coordinates `[x, y, width, height]` before swizzling.
    pub rect: [u32; 4],
    pub source_size: [u32; 2],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShapeSdfAtlasGenerationReport {
    pub requested_shape_count: u32,
    pub packed_shape_count: u32,
    pub failed_shape_count: u32,
    pub page_width: u32,
    pub page_height: u32,
    pub gutter: u32,
    pub failures: Vec<ShapeSdfAtlasGenerationFailure>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShapeSdfAtlasGenerationFailure {
    pub shape_id: i32,
    pub asset_key: String,
    pub reason: String,
}

#[derive(Debug, Error)]
pub enum ShapeSdfAtlasError {
    #[error("shape atlas I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("shape atlas JSON failed for {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid shape atlas contract: {0}")]
    Invalid(String),
    #[error("shape atlas page hash mismatch for {path}: expected {expected}, got {actual}")]
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
}

pub struct MappedShapeSdfAtlasPage {
    width: u32,
    height: u32,
    mapping: Mmap,
}

impl MappedShapeSdfAtlasPage {
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
        &self.mapping[SHAPE_PAGE_HEADER_BYTES..]
    }

    pub fn texel(&self, x: u32, y: u32) -> Option<ShapeSdfTexel> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let blocks_per_row = self.width / SHAPE_BLOCK_WIDTH;
        let block = (y / SHAPE_BLOCK_HEIGHT) * blocks_per_row + x / SHAPE_BLOCK_WIDTH;
        let in_block = (y % SHAPE_BLOCK_HEIGHT) * SHAPE_BLOCK_WIDTH + x % SHAPE_BLOCK_WIDTH;
        let offset = (block * 128 + in_block * SHAPE_CHANNELS) as usize;
        Some(ShapeSdfTexel {
            distance: *self.swizzled_payload().get(offset)?,
            gate: *self.swizzled_payload().get(offset + 1)?,
        })
    }
}

pub struct MappedShapeSdfAtlas {
    manifest: ShapeSdfAtlasManifest,
    manifest_sha256: String,
    pages: Vec<MappedShapeSdfAtlasPage>,
    shape_by_id: BTreeMap<i32, usize>,
}

impl MappedShapeSdfAtlas {
    pub fn open(manifest_path: impl AsRef<Path>) -> Result<Self, ShapeSdfAtlasError> {
        let manifest_path = manifest_path.as_ref();
        let bytes = std::fs::read(manifest_path).map_err(|source| ShapeSdfAtlasError::Io {
            path: manifest_path.to_path_buf(),
            source,
        })?;
        let manifest: ShapeSdfAtlasManifest =
            serde_json::from_slice(&bytes).map_err(|source| ShapeSdfAtlasError::Json {
                path: manifest_path.to_path_buf(),
                source,
            })?;
        validate_manifest(&manifest)?;
        let root = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let pages = manifest
            .pages
            .iter()
            .map(|page| map_page(root, page))
            .collect::<Result<Vec<_>, _>>()?;
        let shape_by_id = manifest
            .shapes
            .iter()
            .enumerate()
            .map(|(index, shape)| (shape.shape_id, index))
            .collect();
        Ok(Self {
            manifest,
            manifest_sha256: hex::encode(Sha256::digest(&bytes)),
            pages,
            shape_by_id,
        })
    }

    pub fn manifest(&self) -> &ShapeSdfAtlasManifest {
        &self.manifest
    }

    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    pub fn pages(&self) -> &[MappedShapeSdfAtlasPage] {
        &self.pages
    }

    pub fn shape(&self, shape_id: i32) -> Option<&ShapeSdfAtlasEntry> {
        self.shape_by_id
            .get(&shape_id)
            .map(|index| &self.manifest.shapes[*index])
    }

    pub fn mapped_bytes(&self) -> u64 {
        self.pages
            .iter()
            .map(|page| page.mapped_bytes() as u64)
            .fold(0u64, u64::saturating_add)
    }

    pub fn prewarm_pages(&self) -> SdfAtlasPrewarmReport {
        let mut report = SdfAtlasPrewarmReport {
            atlas_count: 1,
            ..SdfAtlasPrewarmReport::default()
        };
        for page in &self.pages {
            prewarm_mapped_page(&page.mapping, &mut report);
        }
        std::hint::black_box(report)
    }
}

fn validate_manifest(manifest: &ShapeSdfAtlasManifest) -> Result<(), ShapeSdfAtlasError> {
    if manifest.schema != SHAPE_ATLAS_MANIFEST_SCHEMA
        || manifest.generator_contract != SHAPE_ATLAS_GENERATOR_CONTRACT
        || manifest.pixel_format != SHAPE_ATLAS_PIXEL_FORMAT
        || manifest.pages.is_empty()
    {
        return Err(ShapeSdfAtlasError::Invalid(
            "unsupported schema, generator, pixel format or empty pages".into(),
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
            || !page.width.is_multiple_of(SHAPE_BLOCK_WIDTH)
            || !page.height.is_multiple_of(SHAPE_BLOCK_HEIGHT)
            || page.file_sha256.len() != 64
            || hex::decode(&page.file_sha256).is_err()
        {
            return Err(ShapeSdfAtlasError::Invalid(format!(
                "invalid page descriptor {}",
                page.file
            )));
        }
    }
    let report = &manifest.generation;
    if report.page_width == 0
        || report.page_height == 0
        || report.packed_shape_count as usize != manifest.shapes.len()
        || report.failed_shape_count as usize != report.failures.len()
        || report
            .packed_shape_count
            .checked_add(report.failed_shape_count)
            != Some(report.requested_shape_count)
        || manifest
            .pages
            .iter()
            .any(|page| page.width != report.page_width || page.height != report.page_height)
    {
        return Err(ShapeSdfAtlasError::Invalid(
            "inconsistent generation report".into(),
        ));
    }
    let mut seen = BTreeMap::new();
    for shape in &manifest.shapes {
        let [x, y, width, height] = shape.rect;
        if shape.shape_id <= 0
            || shape.asset_key.trim().is_empty()
            || shape.source_sha256.len() != 64
            || hex::decode(&shape.source_sha256).is_err()
            || shape.source_rg8_sha256.len() != 64
            || hex::decode(&shape.source_rg8_sha256).is_err()
            || usize::from(shape.page) >= manifest.pages.len()
            || seen.insert(shape.shape_id, ()).is_some()
            || width == 0
            || height == 0
            || shape.source_size != [width, height]
        {
            return Err(ShapeSdfAtlasError::Invalid(format!(
                "invalid or duplicate shape {}",
                shape.shape_id
            )));
        }
        let page = &manifest.pages[usize::from(shape.page)];
        if x.checked_add(width).is_none_or(|right| right > page.width)
            || y.checked_add(height)
                .is_none_or(|bottom| bottom > page.height)
        {
            return Err(ShapeSdfAtlasError::Invalid(format!(
                "shape {} exceeds page {}",
                shape.shape_id, shape.page
            )));
        }
    }
    Ok(())
}

fn map_page(
    root: &Path,
    descriptor: &ShapeSdfAtlasPageManifest,
) -> Result<MappedShapeSdfAtlasPage, ShapeSdfAtlasError> {
    let path = root.join(&descriptor.file);
    let file = File::open(&path).map_err(|source| ShapeSdfAtlasError::Io {
        path: path.clone(),
        source,
    })?;
    // SAFETY: immutable atlas artifacts are mapped read-only for the worker lifetime.
    let mapping =
        unsafe { MmapOptions::new().map(&file) }.map_err(|source| ShapeSdfAtlasError::Io {
            path: path.clone(),
            source,
        })?;
    let payload_bytes = descriptor
        .width
        .checked_mul(descriptor.height)
        .and_then(|pixels| pixels.checked_mul(SHAPE_CHANNELS))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or_else(|| ShapeSdfAtlasError::Invalid("shape page length overflow".into()))?;
    let expected_len = SHAPE_PAGE_HEADER_BYTES
        .checked_add(payload_bytes)
        .ok_or_else(|| ShapeSdfAtlasError::Invalid("shape page length overflow".into()))?;
    if mapping.len() != expected_len
        || mapping.get(..SHAPE_PAGE_MAGIC.len()) != Some(SHAPE_PAGE_MAGIC)
        || read_u32(&mapping, 12) != Some(SHAPE_PAGE_VERSION)
        || read_u32(&mapping, 16) != Some(descriptor.width)
        || read_u32(&mapping, 20) != Some(descriptor.height)
        || read_u32(&mapping, 24) != Some(SHAPE_BLOCK_WIDTH)
        || read_u32(&mapping, 28) != Some(SHAPE_BLOCK_HEIGHT)
        || read_u32(&mapping, 32) != Some(SHAPE_CHANNELS)
    {
        return Err(ShapeSdfAtlasError::Invalid(format!(
            "invalid shape page header {}",
            path.display()
        )));
    }
    let actual = hex::encode(Sha256::digest(&mapping));
    if actual != descriptor.file_sha256 {
        return Err(ShapeSdfAtlasError::HashMismatch {
            path,
            expected: descriptor.file_sha256.clone(),
            actual,
        });
    }
    Ok(MappedShapeSdfAtlasPage {
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
mod tests {
    use std::io::Write;

    use super::*;

    fn write_page(root: &Path) -> ShapeSdfAtlasPageManifest {
        let mut bytes = vec![0u8; SHAPE_PAGE_HEADER_BYTES + 8 * 8 * 2];
        bytes[..SHAPE_PAGE_MAGIC.len()].copy_from_slice(SHAPE_PAGE_MAGIC);
        bytes[12..16].copy_from_slice(&SHAPE_PAGE_VERSION.to_le_bytes());
        bytes[16..20].copy_from_slice(&8u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&8u32.to_le_bytes());
        bytes[24..28].copy_from_slice(&SHAPE_BLOCK_WIDTH.to_le_bytes());
        bytes[28..32].copy_from_slice(&SHAPE_BLOCK_HEIGHT.to_le_bytes());
        bytes[32..36].copy_from_slice(&SHAPE_CHANNELS.to_le_bytes());
        for index in 0..64usize {
            bytes[SHAPE_PAGE_HEADER_BYTES + index * 2] = index as u8;
            bytes[SHAPE_PAGE_HEADER_BYTES + index * 2 + 1] = 255 - index as u8;
        }
        let path = root.join("shape-page-000.rg8swz");
        let mut file = File::create(&path).expect("create test page");
        file.write_all(&bytes).expect("write test page");
        ShapeSdfAtlasPageManifest {
            file: "shape-page-000.rg8swz".into(),
            width: 8,
            height: 8,
            file_sha256: hex::encode(Sha256::digest(&bytes)),
        }
    }

    #[test]
    fn opens_and_addresses_rg8_swizzled_page() {
        let root = tempfile::tempdir().expect("shape atlas tempdir");
        let manifest = ShapeSdfAtlasManifest {
            schema: SHAPE_ATLAS_MANIFEST_SCHEMA.into(),
            generator_contract: SHAPE_ATLAS_GENERATOR_CONTRACT.into(),
            pixel_format: SHAPE_ATLAS_PIXEL_FORMAT.into(),
            pages: vec![write_page(root.path())],
            shapes: vec![ShapeSdfAtlasEntry {
                shape_id: 1,
                asset_key: "custom_profile/shape/round".into(),
                source_sha256: "00".repeat(32),
                source_rg8_sha256: "11".repeat(32),
                page: 0,
                rect: [0, 0, 8, 8],
                source_size: [8, 8],
            }],
            generation: ShapeSdfAtlasGenerationReport {
                requested_shape_count: 1,
                packed_shape_count: 1,
                failed_shape_count: 0,
                page_width: 8,
                page_height: 8,
                gutter: 0,
                failures: Vec::new(),
            },
        };
        let manifest_path = root.path().join("manifest.json");
        std::fs::write(
            &manifest_path,
            serde_json::to_vec(&manifest).expect("serialize shape manifest"),
        )
        .expect("write shape manifest");
        let atlas = MappedShapeSdfAtlas::open(&manifest_path).expect("open shape atlas");
        assert_eq!(
            atlas.manifest_sha256(),
            hex::encode(Sha256::digest(
                std::fs::read(&manifest_path).expect("read shape manifest identity")
            ))
        );
        assert_eq!(
            atlas.pages()[0].texel(3, 5),
            Some(ShapeSdfTexel {
                distance: 43,
                gate: 212,
            })
        );
        assert_eq!(atlas.shape(1).map(|shape| shape.page), Some(0));
        assert_eq!(atlas.mapped_bytes(), 192);
        let prewarm = atlas.prewarm_pages();
        assert_eq!(prewarm.atlas_count, 1);
        assert_eq!(prewarm.page_count, 1);
        assert_eq!(prewarm.mapped_bytes, 192);
        assert_eq!(prewarm.page_touch_count, 1);
        assert_eq!(prewarm.checksum, 66);
    }

    #[test]
    fn rejects_path_traversal_and_channel_mismatch() {
        let manifest = ShapeSdfAtlasManifest {
            schema: SHAPE_ATLAS_MANIFEST_SCHEMA.into(),
            generator_contract: SHAPE_ATLAS_GENERATOR_CONTRACT.into(),
            pixel_format: SHAPE_ATLAS_PIXEL_FORMAT.into(),
            pages: vec![ShapeSdfAtlasPageManifest {
                file: "../shape.rg8swz".into(),
                width: 8,
                height: 8,
                file_sha256: "00".repeat(32),
            }],
            shapes: Vec::new(),
            generation: ShapeSdfAtlasGenerationReport {
                requested_shape_count: 0,
                packed_shape_count: 0,
                failed_shape_count: 0,
                page_width: 8,
                page_height: 8,
                gutter: 0,
                failures: Vec::new(),
            },
        };
        assert!(validate_manifest(&manifest).is_err());
    }
}
