use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use allium_renderer::sdf::atlas::{
    MappedSdfAtlas, SdfAtlasGenerationFailure, SdfAtlasGenerationReport, SdfAtlasGlyphManifest,
    SdfAtlasManifest, SdfAtlasPageManifest, ATLAS_MANIFEST_SCHEMA, SWIZZLED_BLOCK_HEIGHT,
    SWIZZLED_BLOCK_WIDTH, SWIZZLED_PAGE_HEADER_BYTES, SWIZZLED_PAGE_MAGIC, SWIZZLED_PAGE_VERSION,
};
use allium_renderer::sdf::outline::{
    self, OfflineAtlasGlyphGenerator, OfflineGenerationMethod, OutlineSdfGlyph,
};
use sha2::{Digest, Sha256};
use ttf_parser::Face;

const DEFAULT_PAGE_SIZE: u32 = 2048;
const DEFAULT_GUTTER: u32 = 1;

#[derive(Debug)]
struct Args {
    font_family: String,
    output: PathBuf,
    page_size: u32,
    gutter: u32,
    method: OfflineGenerationMethod,
    point_size: f32,
    spread: f32,
    codepoints: Option<BTreeSet<u32>>,
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

fn usage() -> &'static str {
    "usage: build-sdf-atlas --font-family <family> --output <empty-dir> \
     [--method analytic|edt1|edt2|edt3|edt4] [--point-size 75] [--spread 6] \
     [--page-size 2048] [--gutter 1] \
     [--codepoints U+0041,U+4E00 | --codepoints-file <path>]\n       build-sdf-atlas --verify <manifest.json>"
}

fn parse_args() -> Result<Args, String> {
    let mut font_family = None;
    let mut output = None;
    let mut page_size = DEFAULT_PAGE_SIZE;
    let mut gutter = DEFAULT_GUTTER;
    let mut method = OfflineGenerationMethod::Edt { supersample: 2 };
    let mut point_size = outline::sampling_point_size();
    let mut spread = outline::sampling_spread();
    let mut codepoints = None;
    let mut codepoints_file = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = match arg.as_str() {
            "--font-family" | "--output" | "--method" | "--page-size" | "--gutter"
            | "--point-size" | "--spread" | "--codepoints" | "--codepoints-file" => args
                .next()
                .ok_or_else(|| format!("{arg} requires a value\n{}", usage()))?,
            "--help" | "-h" => return Err(usage().into()),
            _ => return Err(format!("unknown argument {arg}\n{}", usage())),
        };
        match arg.as_str() {
            "--font-family" => font_family = Some(value),
            "--output" => output = Some(PathBuf::from(value)),
            "--method" => method = parse_method(&value)?,
            "--point-size" => point_size = parse_f32("point-size", &value)?,
            "--spread" => spread = parse_f32("spread", &value)?,
            "--page-size" => page_size = parse_u32("page-size", &value)?,
            "--gutter" => gutter = parse_u32("gutter", &value)?,
            "--codepoints" => codepoints = Some(parse_codepoints(&value)?),
            "--codepoints-file" => codepoints_file = Some(PathBuf::from(value)),
            _ => unreachable!(),
        }
    }
    if page_size == 0
        || !page_size.is_multiple_of(SWIZZLED_BLOCK_WIDTH)
        || !page_size.is_multiple_of(SWIZZLED_BLOCK_HEIGHT)
    {
        return Err("page-size must be a non-zero multiple of 8".into());
    }
    if gutter > page_size / 4 {
        return Err("gutter is unreasonably large for the selected page-size".into());
    }
    if !point_size.is_finite() || point_size <= 0.0 {
        return Err("point-size must be finite and positive".into());
    }
    if !spread.is_finite() || spread <= 0.0 {
        return Err("spread must be finite and positive".into());
    }
    if codepoints.is_some() && codepoints_file.is_some() {
        return Err("--codepoints and --codepoints-file are mutually exclusive".into());
    }
    if let Some(path) = codepoints_file {
        let value = fs::read_to_string(&path)
            .map_err(|error| format!("read codepoints file {} failed: {error}", path.display()))?;
        codepoints = Some(parse_codepoints(&value.replace(['\r', '\n'], ","))?);
    }
    Ok(Args {
        font_family: font_family.ok_or_else(|| usage().to_string())?,
        output: output.ok_or_else(|| usage().to_string())?,
        page_size,
        gutter,
        method,
        point_size,
        spread,
        codepoints,
    })
}

fn parse_u32(name: &str, value: &str) -> Result<u32, String> {
    value
        .parse()
        .map_err(|_| format!("invalid --{name} value {value}"))
}

fn parse_f32(name: &str, value: &str) -> Result<f32, String> {
    value
        .parse()
        .map_err(|_| format!("invalid --{name} value {value}"))
}

fn parse_method(value: &str) -> Result<OfflineGenerationMethod, String> {
    match value {
        "analytic" => Ok(OfflineGenerationMethod::Analytic),
        "edt1" => Ok(OfflineGenerationMethod::Edt { supersample: 1 }),
        "edt2" => Ok(OfflineGenerationMethod::Edt { supersample: 2 }),
        "edt3" => Ok(OfflineGenerationMethod::Edt { supersample: 3 }),
        "edt4" => Ok(OfflineGenerationMethod::Edt { supersample: 4 }),
        _ => Err(format!("unsupported --method {value}")),
    }
}

fn parse_codepoints(value: &str) -> Result<BTreeSet<u32>, String> {
    let mut result = BTreeSet::new();
    for part in value.split(',').filter(|part| !part.is_empty()) {
        let digits = part
            .strip_prefix("U+")
            .or_else(|| part.strip_prefix("u+"))
            .unwrap_or(part);
        let codepoint =
            u32::from_str_radix(digits, 16).map_err(|_| format!("invalid codepoint {part}"))?;
        if char::from_u32(codepoint).is_none() {
            return Err(format!("invalid Unicode scalar U+{codepoint:04X}"));
        }
        result.insert(codepoint);
    }
    if result.is_empty() {
        return Err("--codepoints resolved to an empty set".into());
    }
    Ok(result)
}

fn method_contract(method: OfflineGenerationMethod) -> String {
    match method {
        OfflineGenerationMethod::Analytic => "outline-analytic-v1".into(),
        OfflineGenerationMethod::Edt { supersample } => {
            format!("outline-edt-v1:ss={supersample}:fallback=analytic-v1")
        }
    }
}

fn enumerate_codepoints(font_bytes: &[u8]) -> Result<BTreeSet<u32>, String> {
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

fn ensure_empty_output(output: &Path) -> Result<(), String> {
    if output.exists() {
        let mut entries = fs::read_dir(output).map_err(|error| {
            format!("read output directory {} failed: {error}", output.display())
        })?;
        if entries.next().is_some() {
            return Err(format!(
                "output directory {} is not empty",
                output.display()
            ));
        }
    } else {
        fs::create_dir_all(output).map_err(|error| {
            format!(
                "create output directory {} failed: {error}",
                output.display()
            )
        })?;
    }
    Ok(())
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

fn build(args: &Args) -> Result<PathBuf, String> {
    ensure_empty_output(&args.output)?;
    let font_path = outline::resolve_font_path(&args.font_family)
        .ok_or_else(|| format!("cannot resolve font family {}", args.font_family))?;
    let font_bytes = fs::read(&font_path)
        .map_err(|error| format!("read font {} failed: {error}", font_path.display()))?;
    let font_sha256 = hex::encode(Sha256::digest(&font_bytes));
    let cmap_codepoints = enumerate_codepoints(&font_bytes)?;
    let requested = args
        .codepoints
        .clone()
        .unwrap_or_else(|| cmap_codepoints.clone());
    if requested.is_empty() {
        return Err("no requested codepoint exists in the font cmap".into());
    }

    let mut pages = Vec::new();
    let mut page = ShelfPage::new(args.page_size)?;
    let mut glyphs = Vec::with_capacity(requested.len());
    let mut failures = Vec::new();
    let mut fallback_codepoints = Vec::new();
    let generator = OfflineAtlasGlyphGenerator::new_at_sampling(
        &args.font_family,
        args.point_size,
        args.spread,
    )?;

    for codepoint in requested.iter().copied() {
        if !cmap_codepoints.contains(&codepoint) {
            failures.push(SdfAtlasGenerationFailure {
                codepoint,
                reason: "codepoint is absent from the font cmap".into(),
            });
            continue;
        }
        let ch = char::from_u32(codepoint).expect("requested set contains valid Unicode scalars");
        let (glyph, used_fallback) = match generator.generate(ch, args.method) {
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
        let packed_width = glyph_width
            .checked_add(args.gutter.saturating_mul(2))
            .ok_or_else(|| format!("glyph U+{codepoint:04X} packed width overflow"))?;
        let packed_height = glyph_height
            .checked_add(args.gutter.saturating_mul(2))
            .ok_or_else(|| format!("glyph U+{codepoint:04X} packed height overflow"))?;
        if packed_width > args.page_size || packed_height > args.page_size {
            failures.push(SdfAtlasGenerationFailure {
                codepoint,
                reason: format!(
                    "glyph {}x{} plus gutter does not fit {}x{} page",
                    glyph_width, glyph_height, args.page_size, args.page_size
                ),
            });
            continue;
        }
        let mut origin = page.place(args.page_size, glyph_width, glyph_height, args.gutter);
        if origin.is_none() {
            pages.push(write_page(
                &args.output,
                pages.len(),
                args.page_size,
                &page.pixels,
            )?);
            page = ShelfPage::new(args.page_size)?;
            origin = page.place(args.page_size, glyph_width, glyph_height, args.gutter);
        }
        let origin = origin
            .ok_or_else(|| format!("fresh page unexpectedly rejected glyph U+{codepoint:04X}"))?;
        page.copy_glyph(args.page_size, origin, &glyph);
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
        return Err("all requested glyphs failed generation".into());
    }
    pages.push(write_page(
        &args.output,
        pages.len(),
        args.page_size,
        &page.pixels,
    )?);

    let generated_glyph_count =
        u32::try_from(glyphs.len()).map_err(|_| "generated glyph count exceeds u32".to_string())?;
    let manifest = SdfAtlasManifest {
        schema: ATLAS_MANIFEST_SCHEMA.into(),
        generator_contract: method_contract(args.method),
        font_family: args.font_family.clone(),
        font_sha256,
        point_size: args.point_size,
        spread: args.spread,
        pages,
        glyphs,
        generation: SdfAtlasGenerationReport {
            cmap_codepoint_count: u32::try_from(cmap_codepoints.len())
                .map_err(|_| "cmap codepoint count exceeds u32".to_string())?,
            requested_codepoint_count: u32::try_from(requested.len())
                .map_err(|_| "requested codepoint count exceeds u32".to_string())?,
            generated_glyph_count,
            failed_glyph_count: u32::try_from(failures.len())
                .map_err(|_| "failed glyph count exceeds u32".to_string())?,
            analytic_fallback_count: u32::try_from(fallback_codepoints.len())
                .map_err(|_| "fallback count exceeds u32".to_string())?,
            page_width: args.page_size,
            page_height: args.page_size,
            gutter: args.gutter,
            failures,
            analytic_fallback_codepoints: fallback_codepoints,
        },
    };
    let manifest_path = args.output.join("manifest.json");
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("serialize manifest failed: {error}"))?;
    manifest_bytes.push(b'\n');
    fs::write(&manifest_path, manifest_bytes)
        .map_err(|error| format!("write manifest failed: {error}"))?;

    let reopened = MappedSdfAtlas::open(&manifest_path)
        .map_err(|error| format!("post-write mmap validation failed: {error}"))?;
    if reopened.manifest() != &manifest {
        return Err("post-write manifest roundtrip mismatch".into());
    }
    Ok(manifest_path)
}

fn verify(manifest_path: &Path) -> Result<(), String> {
    let manifest_bytes = fs::read(manifest_path)
        .map_err(|error| format!("read manifest {} failed: {error}", manifest_path.display()))?;
    let atlas = MappedSdfAtlas::open(manifest_path)
        .map_err(|error| format!("mmap validation failed: {error}"))?;
    let mapped_bytes = atlas
        .pages()
        .iter()
        .try_fold(0u64, |total, page| {
            total.checked_add(page.mapped_bytes() as u64)
        })
        .ok_or_else(|| "mapped byte count overflow".to_string())?;
    let report = &atlas.manifest().generation;
    let summary = serde_json::json!({
        "schema": &atlas.manifest().schema,
        "manifest_sha256": hex::encode(Sha256::digest(&manifest_bytes)),
        "font_sha256": &atlas.manifest().font_sha256,
        "generator_contract": &atlas.manifest().generator_contract,
        "page_count": atlas.pages().len(),
        "mapped_bytes": mapped_bytes,
        "requested_codepoint_count": report.requested_codepoint_count,
        "generated_glyph_count": report.generated_glyph_count,
        "failed_glyph_count": report.failed_glyph_count,
        "analytic_fallback_count": report.analytic_fallback_count,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&summary)
            .map_err(|error| format!("serialize verification summary failed: {error}"))?
    );
    Ok(())
}

fn run() -> Result<(), String> {
    let mut raw_args = env::args().skip(1);
    if raw_args.next().as_deref() == Some("--verify") {
        let manifest = raw_args
            .next()
            .ok_or_else(|| format!("--verify requires a manifest path\n{}", usage()))?;
        if raw_args.next().is_some() {
            return Err(format!("--verify accepts one manifest path\n{}", usage()));
        }
        return verify(Path::new(&manifest));
    }
    let args = parse_args()?;
    let manifest = build(&args)?;
    println!("{}", manifest.display());
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
}
