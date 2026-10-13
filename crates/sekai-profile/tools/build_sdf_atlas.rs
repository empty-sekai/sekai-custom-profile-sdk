use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use sekai_profile::prepare::{build_text_atlas, TextAtlasOptions};
use sekai_profile::sdf::atlas::MappedSdfAtlas;
use sekai_profile::sdf::outline::{self, OfflineGenerationMethod};
use sha2::{Digest, Sha256};

const DEFAULT_PAGE_SIZE: u32 = sekai_profile::prepare::text::DEFAULT_TEXT_PAGE_SIZE;
const DEFAULT_GUTTER: u32 = sekai_profile::prepare::text::DEFAULT_TEXT_GUTTER;

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

fn build(args: Args) -> Result<PathBuf, String> {
    let mut options = TextAtlasOptions::new(args.font_family, args.output);
    options.method = args.method;
    options.point_size = args.point_size;
    options.spread = args.spread;
    options.page_size = args.page_size;
    options.gutter = args.gutter;
    options.codepoints = args.codepoints;
    build_text_atlas(&options)
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
    let manifest = build(args)?;
    println!("{}", manifest.display());
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}
