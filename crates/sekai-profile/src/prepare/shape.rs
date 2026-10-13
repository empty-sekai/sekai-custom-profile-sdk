//! Shape SDF atlas construction.
//!
//! Each `customProfileShapeResources` row names a sprite whose red channel is
//! a distance field and whose alpha is a coverage gate. The atlas copies both
//! channels unchanged onto RG8 pages, 8×8-block swizzled, the layout
//! [`MappedShapeSdfAtlas`] maps.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::sdf::shape_atlas::{
    MappedShapeSdfAtlas, ShapeSdfAtlasEntry, ShapeSdfAtlasGenerationFailure,
    ShapeSdfAtlasGenerationReport, ShapeSdfAtlasManifest, ShapeSdfAtlasPageManifest,
    ShapeSdfSource, SHAPE_ATLAS_GENERATOR_CONTRACT, SHAPE_ATLAS_MANIFEST_SCHEMA,
    SHAPE_ATLAS_PIXEL_FORMAT, SHAPE_BLOCK_HEIGHT, SHAPE_BLOCK_WIDTH, SHAPE_CHANNELS,
    SHAPE_PAGE_HEADER_BYTES, SHAPE_PAGE_MAGIC, SHAPE_PAGE_VERSION,
};

/// Page edge, in texels, of a shape atlas built with default options.
pub const DEFAULT_SHAPE_PAGE_SIZE: u32 = 2048;
/// Empty texels kept around each shape on a page.
pub const DEFAULT_SHAPE_GUTTER: u32 = 2;

/// One row of the `customProfileShapeResources` master data table.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ShapeResource {
    pub id: i32,
    pub file_name: String,
    pub resource_load_val: String,
}

impl ShapeResource {
    /// Asset key of the shape sprite, `<resourceLoadVal>/<fileName>`.
    pub fn asset_key(&self) -> String {
        format!("{}/{}", self.resource_load_val, self.file_name)
    }
}

/// How to build a shape atlas.
#[derive(Clone, Debug)]
pub struct ShapeAtlasOptions {
    /// Directory the atlas is written to. It must be empty or absent.
    pub output: PathBuf,
    pub page_size: u32,
    pub gutter: u32,
}

impl ShapeAtlasOptions {
    pub fn new(output: impl Into<PathBuf>) -> Self {
        Self {
            output: output.into(),
            page_size: DEFAULT_SHAPE_PAGE_SIZE,
            gutter: DEFAULT_SHAPE_GUTTER,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.page_size == 0
            || !self.page_size.is_multiple_of(SHAPE_BLOCK_WIDTH)
            || !self.page_size.is_multiple_of(SHAPE_BLOCK_HEIGHT)
        {
            return Err("page size must be a non-zero multiple of 8".into());
        }
        if self.gutter > self.page_size / 4 {
            return Err("gutter is too large for the page size".into());
        }
        Ok(())
    }
}

/// Builds a shape atlas from `resources` and returns the path of its
/// `manifest.json`.
///
/// `source` returns the encoded PNG of a resource's sprite. A resource whose
/// sprite cannot be read, decoded or packed is listed as a failure in the
/// manifest; the build fails only when no shape could be packed. The written
/// atlas is reopened and checked against the manifest before this returns.
pub fn build_shape_atlas(
    resources: &[ShapeResource],
    mut source: impl FnMut(&ShapeResource) -> Result<Vec<u8>, String>,
    options: &ShapeAtlasOptions,
) -> Result<PathBuf, String> {
    options.validate()?;
    if resources.is_empty() {
        return Err("there are no shape resources".into());
    }
    super::ensure_empty_dir(&options.output)?;
    let mut resources = resources.to_vec();
    resources.sort_by_key(|resource| resource.id);

    let size = options.page_size;
    let gutter = options.gutter;
    let mut page = ShelfPage::new(size)?;
    let mut pages = Vec::new();
    let mut shapes = Vec::new();
    let mut failures = Vec::new();
    for resource in &resources {
        let asset_key = resource.asset_key();
        let decoded = source(resource).and_then(|encoded| {
            let shape = ShapeSdfSource::from_png(&encoded)
                .map_err(|error| format!("decode failed: {error}"))?;
            Ok((shape, hex::encode(Sha256::digest(&encoded))))
        });
        let (shape, source_sha256) = match decoded {
            Ok(decoded) => decoded,
            Err(reason) => {
                failures.push(ShapeSdfAtlasGenerationFailure {
                    shape_id: resource.id,
                    asset_key,
                    reason,
                });
                continue;
            }
        };
        let padding = gutter.saturating_mul(2);
        if shape.width.saturating_add(padding) > size || shape.height.saturating_add(padding) > size
        {
            failures.push(ShapeSdfAtlasGenerationFailure {
                shape_id: resource.id,
                asset_key,
                reason: format!(
                    "source {}x{} plus gutter does not fit a {size}x{size} page",
                    shape.width, shape.height
                ),
            });
            continue;
        }
        let mut origin = page.place(size, shape.width, shape.height, gutter);
        if origin.is_none() {
            pages.push(write_page(&options.output, pages.len(), size, &page.rg)?);
            page = ShelfPage::new(size)?;
            origin = page.place(size, shape.width, shape.height, gutter);
        }
        let origin = origin.ok_or_else(|| "a fresh page rejected a shape that fits".to_string())?;
        page.copy_shape(size, origin, &shape);
        shapes.push(ShapeSdfAtlasEntry {
            shape_id: resource.id,
            asset_key,
            source_sha256,
            source_rg8_sha256: shape.rg8_sha256,
            page: u16::try_from(pages.len())
                .map_err(|_| "shape atlas page count exceeds u16".to_string())?,
            rect: [origin[0], origin[1], shape.width, shape.height],
            source_size: [shape.width, shape.height],
        });
    }
    if shapes.is_empty() {
        return Err("no shape resource could be packed".into());
    }
    pages.push(write_page(&options.output, pages.len(), size, &page.rg)?);
    let count =
        |value: usize| u32::try_from(value).map_err(|_| "shape count exceeds u32".to_string());
    let manifest = ShapeSdfAtlasManifest {
        schema: SHAPE_ATLAS_MANIFEST_SCHEMA.into(),
        generator_contract: SHAPE_ATLAS_GENERATOR_CONTRACT.into(),
        pixel_format: SHAPE_ATLAS_PIXEL_FORMAT.into(),
        pages,
        generation: ShapeSdfAtlasGenerationReport {
            requested_shape_count: count(resources.len())?,
            packed_shape_count: count(shapes.len())?,
            failed_shape_count: count(failures.len())?,
            page_width: size,
            page_height: size,
            gutter,
            failures,
        },
        shapes,
    };
    let manifest_path = options.output.join(super::MANIFEST_FILE);
    super::write_manifest(&manifest_path, &manifest)?;
    let reopened = MappedShapeSdfAtlas::open(&manifest_path)
        .map_err(|error| format!("reopening the written atlas failed: {error}"))?;
    if reopened.manifest() != &manifest {
        return Err("the reopened atlas manifest differs from the written one".into());
    }
    Ok(manifest_path)
}

#[derive(Debug)]
struct ShelfPage {
    rg: Vec<u8>,
    cursor_x: u32,
    cursor_y: u32,
    shelf_height: u32,
}

impl ShelfPage {
    fn new(size: u32) -> Result<Self, String> {
        let len = usize::try_from(size)
            .ok()
            .and_then(|size| size.checked_mul(size))
            .and_then(|pixels| pixels.checked_mul(SHAPE_CHANNELS as usize))
            .ok_or_else(|| "shape atlas page size overflow".to_string())?;
        Ok(Self {
            rg: vec![0; len],
            cursor_x: 0,
            cursor_y: 0,
            shelf_height: 0,
        })
    }

    fn place(&mut self, size: u32, width: u32, height: u32, gutter: u32) -> Option<[u32; 2]> {
        let packed_width = width.checked_add(gutter.checked_mul(2)?)?;
        let packed_height = height.checked_add(gutter.checked_mul(2)?)?;
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
        let origin = [self.cursor_x + gutter, self.cursor_y + gutter];
        self.cursor_x += packed_width;
        self.shelf_height = self.shelf_height.max(packed_height);
        Some(origin)
    }

    fn copy_shape(&mut self, size: u32, origin: [u32; 2], shape: &ShapeSdfSource) {
        let stride = size as usize * SHAPE_CHANNELS as usize;
        let source_stride = shape.width as usize * SHAPE_CHANNELS as usize;
        for row in 0..shape.height as usize {
            let destination =
                (origin[1] as usize + row) * stride + origin[0] as usize * SHAPE_CHANNELS as usize;
            let source = row * source_stride;
            self.rg[destination..destination + source_stride]
                .copy_from_slice(&shape.rg[source..source + source_stride]);
        }
    }
}

fn swizzle_page(linear_rg: &[u8], size: u32) -> Vec<u8> {
    let mut swizzled = vec![0; linear_rg.len()];
    let blocks_per_row = size / SHAPE_BLOCK_WIDTH;
    for y in 0..size {
        for x in 0..size {
            let block = (y / SHAPE_BLOCK_HEIGHT) * blocks_per_row + x / SHAPE_BLOCK_WIDTH;
            let in_block = (y % SHAPE_BLOCK_HEIGHT) * SHAPE_BLOCK_WIDTH + x % SHAPE_BLOCK_WIDTH;
            let source = ((y * size + x) * SHAPE_CHANNELS) as usize;
            let destination = (block * 128 + in_block * SHAPE_CHANNELS) as usize;
            swizzled[destination..destination + 2].copy_from_slice(&linear_rg[source..source + 2]);
        }
    }
    swizzled
}

fn write_page(
    output: &Path,
    index: usize,
    size: u32,
    linear_rg: &[u8],
) -> Result<ShapeSdfAtlasPageManifest, String> {
    let payload = swizzle_page(linear_rg, size);
    let mut bytes = vec![0; SHAPE_PAGE_HEADER_BYTES];
    bytes[..SHAPE_PAGE_MAGIC.len()].copy_from_slice(SHAPE_PAGE_MAGIC);
    bytes[12..16].copy_from_slice(&SHAPE_PAGE_VERSION.to_le_bytes());
    bytes[16..20].copy_from_slice(&size.to_le_bytes());
    bytes[20..24].copy_from_slice(&size.to_le_bytes());
    bytes[24..28].copy_from_slice(&SHAPE_BLOCK_WIDTH.to_le_bytes());
    bytes[28..32].copy_from_slice(&SHAPE_BLOCK_HEIGHT.to_le_bytes());
    bytes[32..36].copy_from_slice(&SHAPE_CHANNELS.to_le_bytes());
    bytes.extend_from_slice(&payload);
    let file = format!("shape-page-{index:03}.rg8swz");
    fs::write(output.join(&file), &bytes)
        .map_err(|error| format!("write {file} failed: {error}"))?;
    Ok(ShapeSdfAtlasPageManifest {
        file,
        width: size,
        height: size,
        file_sha256: hex::encode(Sha256::digest(&bytes)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(id: i32, file_name: &str) -> ShapeResource {
        ShapeResource {
            id,
            file_name: file_name.into(),
            resource_load_val: "custom_profile/shape".into(),
        }
    }

    #[test]
    fn rg8_swizzle_matches_scalar_block_addressing() {
        let size = 16u32;
        let mut linear = vec![0u8; (size * size * 2) as usize];
        for pixel in 0..size * size {
            linear[(pixel * 2) as usize] = pixel as u8;
            linear[(pixel * 2 + 1) as usize] = 255 - pixel as u8;
        }
        let swizzled = swizzle_page(&linear, size);
        for y in 0..size {
            for x in 0..size {
                let block = (y / 8) * (size / 8) + x / 8;
                let in_block = (y % 8) * 8 + x % 8;
                let destination = (block * 128 + in_block * 2) as usize;
                let source = ((y * size + x) * 2) as usize;
                assert_eq!(
                    &swizzled[destination..destination + 2],
                    &linear[source..source + 2]
                );
            }
        }
    }

    #[test]
    fn packs_decodable_shapes_and_records_the_rest_as_failures() {
        let output = tempfile::tempdir().expect("output directory");
        let pixels = [200, 0, 0, 3, 120, 0, 0, 64, 255, 0, 0, 255, 90, 0, 0, 0];
        let png = crate::codec::png::encode_rgba(2, 2, &pixels).expect("encode sprite");
        let resources = [resource(2, "missing"), resource(1, "square")];
        let mut options = ShapeAtlasOptions::new(output.path().join("shape"));
        options.page_size = 64;
        let manifest = build_shape_atlas(
            &resources,
            |resource| match resource.file_name.as_str() {
                "square" => Ok(png.clone()),
                _ => Err("not found".into()),
            },
            &options,
        )
        .expect("build atlas");
        let atlas = MappedShapeSdfAtlas::open(&manifest).expect("open atlas");
        let manifest = atlas.manifest();
        assert_eq!(manifest.shapes.len(), 1);
        assert_eq!(manifest.shapes[0].shape_id, 1);
        assert_eq!(manifest.shapes[0].asset_key, "custom_profile/shape/square");
        assert_eq!(manifest.generation.failed_shape_count, 1);
        assert_eq!(manifest.generation.failures[0].shape_id, 2);
    }

    #[test]
    fn a_build_without_any_packable_shape_fails() {
        let output = tempfile::tempdir().expect("output directory");
        let result = build_shape_atlas(
            &[resource(1, "missing")],
            |_| Err("not found".into()),
            &ShapeAtlasOptions::new(output.path().join("shape")),
        );
        assert!(result.is_err());
    }
}
