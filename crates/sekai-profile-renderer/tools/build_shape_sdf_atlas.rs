use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use sekai_profile_renderer::sdf::shape_atlas::{
    MappedShapeSdfAtlas, ShapeSdfAtlasEntry, ShapeSdfAtlasGenerationFailure,
    ShapeSdfAtlasGenerationReport, ShapeSdfAtlasManifest, ShapeSdfAtlasPageManifest,
    SHAPE_ATLAS_GENERATOR_CONTRACT, SHAPE_ATLAS_MANIFEST_SCHEMA, SHAPE_ATLAS_PIXEL_FORMAT,
    SHAPE_BLOCK_HEIGHT, SHAPE_BLOCK_WIDTH, SHAPE_CHANNELS, SHAPE_PAGE_HEADER_BYTES,
    SHAPE_PAGE_MAGIC, SHAPE_PAGE_VERSION,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use skia_safe::{AlphaType, ColorType, Data, Image, ImageInfo};

const DEFAULT_PAGE_SIZE: u32 = 2048;
const DEFAULT_GUTTER: u32 = 2;

#[derive(Debug)]
struct Args {
    resources_json: PathBuf,
    input_dir: PathBuf,
    output: PathBuf,
    page_size: u32,
    gutter: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShapeResource {
    id: i32,
    file_name: String,
    resource_load_val: String,
}

#[derive(Debug)]
struct DecodedShape {
    width: u32,
    height: u32,
    rg: Vec<u8>,
    source_sha256: String,
    source_rg8_sha256: String,
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

    fn copy_shape(&mut self, size: u32, origin: [u32; 2], shape: &DecodedShape) {
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

fn usage() -> &'static str {
    "usage: build-shape-sdf-atlas --resources-json <customProfileShapeResources.json> \
     --input-dir <png-dir> --output <empty-dir> [--page-size 2048] [--gutter 2]\n\
     build-shape-sdf-atlas --verify <manifest.json>"
}

fn parse_args() -> Result<Args, String> {
    let mut resources_json = None;
    let mut input_dir = None;
    let mut output = None;
    let mut page_size = DEFAULT_PAGE_SIZE;
    let mut gutter = DEFAULT_GUTTER;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = match arg.as_str() {
            "--resources-json" | "--input-dir" | "--output" | "--page-size" | "--gutter" => args
                .next()
                .ok_or_else(|| format!("{arg} requires a value\n{}", usage()))?,
            "--help" | "-h" => return Err(usage().into()),
            _ => return Err(format!("unknown argument {arg}\n{}", usage())),
        };
        match arg.as_str() {
            "--resources-json" => resources_json = Some(PathBuf::from(value)),
            "--input-dir" => input_dir = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--page-size" => {
                page_size = value
                    .parse()
                    .map_err(|_| format!("invalid --page-size {value}"))?
            }
            "--gutter" => {
                gutter = value
                    .parse()
                    .map_err(|_| format!("invalid --gutter {value}"))?
            }
            _ => unreachable!(),
        }
    }
    if page_size == 0
        || !page_size.is_multiple_of(SHAPE_BLOCK_WIDTH)
        || !page_size.is_multiple_of(SHAPE_BLOCK_HEIGHT)
    {
        return Err("page-size must be a non-zero multiple of 8".into());
    }
    if gutter > page_size / 4 {
        return Err("gutter is unreasonably large".into());
    }
    Ok(Args {
        resources_json: resources_json.ok_or_else(|| usage().to_string())?,
        input_dir: input_dir.ok_or_else(|| usage().to_string())?,
        output: output.ok_or_else(|| usage().to_string())?,
        page_size,
        gutter,
    })
}

fn ensure_empty_output(output: &Path) -> Result<(), String> {
    if output.exists() {
        if fs::read_dir(output)
            .map_err(|error| format!("read {} failed: {error}", output.display()))?
            .next()
            .is_some()
        {
            return Err(format!(
                "output directory {} is not empty",
                output.display()
            ));
        }
    } else {
        fs::create_dir_all(output)
            .map_err(|error| format!("create {} failed: {error}", output.display()))?;
    }
    Ok(())
}

fn decode_shape(path: &Path) -> Result<DecodedShape, String> {
    let encoded =
        fs::read(path).map_err(|error| format!("read {} failed: {error}", path.display()))?;
    let source_sha256 = hex::encode(Sha256::digest(&encoded));
    let image = Image::from_encoded(Data::new_copy(&encoded))
        .ok_or_else(|| format!("decode {} failed", path.display()))?;
    let width = u32::try_from(image.width()).map_err(|_| "negative image width".to_string())?;
    let height = u32::try_from(image.height()).map_err(|_| "negative image height".to_string())?;
    let rgba_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or_else(|| format!("image {} size overflow", path.display()))?;
    let mut rgba = vec![0u8; rgba_len];
    let info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        None,
    );
    if !image.read_pixels(
        &info,
        &mut rgba,
        width as usize * 4,
        (0, 0),
        skia_safe::image::CachingHint::Allow,
    ) {
        return Err(format!("read pixels {} failed", path.display()));
    }
    let mut rg = Vec::with_capacity(width as usize * height as usize * 2);
    for pixel in rgba.chunks_exact(4) {
        rg.push(pixel[0]);
        rg.push(pixel[3]);
    }
    let source_rg8_sha256 = hex::encode(Sha256::digest(&rg));
    Ok(DecodedShape {
        width,
        height,
        rg,
        source_sha256,
        source_rg8_sha256,
    })
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

fn build(args: &Args) -> Result<PathBuf, String> {
    ensure_empty_output(&args.output)?;
    let resources_bytes = fs::read(&args.resources_json)
        .map_err(|error| format!("read {} failed: {error}", args.resources_json.display()))?;
    let mut resources: Vec<ShapeResource> = serde_json::from_slice(&resources_bytes)
        .map_err(|error| format!("parse resources failed: {error}"))?;
    resources.sort_by_key(|resource| resource.id);
    if resources.is_empty() {
        return Err("shape resources are empty".into());
    }

    let mut page = ShelfPage::new(args.page_size)?;
    let mut pages = Vec::new();
    let mut shapes = Vec::new();
    let mut failures = Vec::new();
    for resource in &resources {
        let asset_key = format!("{}/{}", resource.resource_load_val, resource.file_name);
        let path = args.input_dir.join(format!("{}.png", resource.file_name));
        let decoded = match decode_shape(&path) {
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
        let packed_width = decoded.width.saturating_add(args.gutter.saturating_mul(2));
        let packed_height = decoded.height.saturating_add(args.gutter.saturating_mul(2));
        if packed_width > args.page_size || packed_height > args.page_size {
            failures.push(ShapeSdfAtlasGenerationFailure {
                shape_id: resource.id,
                asset_key,
                reason: format!(
                    "source {}x{} plus gutter does not fit {} page",
                    decoded.width, decoded.height, args.page_size
                ),
            });
            continue;
        }
        let mut origin = page.place(args.page_size, decoded.width, decoded.height, args.gutter);
        if origin.is_none() {
            pages.push(write_page(
                &args.output,
                pages.len(),
                args.page_size,
                &page.rg,
            )?);
            page = ShelfPage::new(args.page_size)?;
            origin = page.place(args.page_size, decoded.width, decoded.height, args.gutter);
        }
        let origin = origin.ok_or_else(|| "fresh page rejected a validated shape".to_string())?;
        page.copy_shape(args.page_size, origin, &decoded);
        shapes.push(ShapeSdfAtlasEntry {
            shape_id: resource.id,
            asset_key,
            source_sha256: decoded.source_sha256,
            source_rg8_sha256: decoded.source_rg8_sha256,
            page: u16::try_from(pages.len())
                .map_err(|_| "shape atlas page count exceeds u16".to_string())?,
            rect: [origin[0], origin[1], decoded.width, decoded.height],
            source_size: [decoded.width, decoded.height],
        });
    }
    if shapes.is_empty() {
        return Err("all shape resources failed".into());
    }
    pages.push(write_page(
        &args.output,
        pages.len(),
        args.page_size,
        &page.rg,
    )?);
    let manifest = ShapeSdfAtlasManifest {
        schema: SHAPE_ATLAS_MANIFEST_SCHEMA.into(),
        generator_contract: SHAPE_ATLAS_GENERATOR_CONTRACT.into(),
        pixel_format: SHAPE_ATLAS_PIXEL_FORMAT.into(),
        pages,
        shapes,
        generation: ShapeSdfAtlasGenerationReport {
            requested_shape_count: resources.len() as u32,
            packed_shape_count: (resources.len() - failures.len()) as u32,
            failed_shape_count: failures.len() as u32,
            page_width: args.page_size,
            page_height: args.page_size,
            gutter: args.gutter,
            failures,
        },
    };
    let manifest_path = args.output.join("manifest.json");
    let mut bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("serialize manifest failed: {error}"))?;
    bytes.push(b'\n');
    fs::write(&manifest_path, bytes).map_err(|error| format!("write manifest failed: {error}"))?;
    let reopened = MappedShapeSdfAtlas::open(&manifest_path)
        .map_err(|error| format!("post-write mmap validation failed: {error}"))?;
    if reopened.manifest() != &manifest {
        return Err("post-write manifest mismatch".into());
    }
    Ok(manifest_path)
}

fn verify(manifest_path: &Path) -> Result<(), String> {
    let manifest_bytes = fs::read(manifest_path)
        .map_err(|error| format!("read {} failed: {error}", manifest_path.display()))?;
    let atlas = MappedShapeSdfAtlas::open(manifest_path)
        .map_err(|error| format!("mmap validation failed: {error}"))?;
    let report = &atlas.manifest().generation;
    let summary = serde_json::json!({
        "schema": &atlas.manifest().schema,
        "manifest_sha256": hex::encode(Sha256::digest(&manifest_bytes)),
        "generator_contract": &atlas.manifest().generator_contract,
        "pixel_format": &atlas.manifest().pixel_format,
        "page_count": atlas.pages().len(),
        "mapped_bytes": atlas.mapped_bytes(),
        "requested_shape_count": report.requested_shape_count,
        "packed_shape_count": report.packed_shape_count,
        "failed_shape_count": report.failed_shape_count,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&summary)
            .map_err(|error| format!("serialize verification failed: {error}"))?
    );
    Ok(())
}

fn run() -> Result<(), String> {
    let mut raw_args = env::args().skip(1);
    if raw_args.next().as_deref() == Some("--verify") {
        let manifest = raw_args
            .next()
            .ok_or_else(|| format!("--verify requires a manifest\n{}", usage()))?;
        if raw_args.next().is_some() {
            return Err(format!("--verify accepts one manifest\n{}", usage()));
        }
        return verify(Path::new(&manifest));
    }
    let args = parse_args()?;
    println!("{}", build(&args)?.display());
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
