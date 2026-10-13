use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use sekai_profile::prepare::shape::{DEFAULT_SHAPE_GUTTER, DEFAULT_SHAPE_PAGE_SIZE};
use sekai_profile::prepare::{build_shape_atlas, ShapeAtlasOptions, ShapeResource};
use sekai_profile::sdf::shape_atlas::MappedShapeSdfAtlas;
use sha2::{Digest, Sha256};

#[derive(Debug)]
struct Args {
    resources_json: PathBuf,
    input_dir: PathBuf,
    output: PathBuf,
    page_size: u32,
    gutter: u32,
}

fn usage() -> &'static str {
    "usage: build-shape-sdf-atlas --resources-json <customProfileShapeResources.json>      --input-dir <png-dir> --output <empty-dir> [--page-size 2048] [--gutter 2]
     build-shape-sdf-atlas --verify <manifest.json>"
}

fn parse_args() -> Result<Args, String> {
    let mut resources_json = None;
    let mut input_dir = None;
    let mut output = None;
    let mut page_size = DEFAULT_SHAPE_PAGE_SIZE;
    let mut gutter = DEFAULT_SHAPE_GUTTER;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = match arg.as_str() {
            "--resources-json" | "--input-dir" | "--output" | "--page-size" | "--gutter" => {
                args.next().ok_or_else(|| {
                    format!(
                        "{arg} requires a value
{}",
                        usage()
                    )
                })?
            }
            "--help" | "-h" => return Err(usage().into()),
            _ => {
                return Err(format!(
                    "unknown argument {arg}
{}",
                    usage()
                ))
            }
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
    Ok(Args {
        resources_json: resources_json.ok_or_else(|| usage().to_string())?,
        input_dir: input_dir.ok_or_else(|| usage().to_string())?,
        output: output.ok_or_else(|| usage().to_string())?,
        page_size,
        gutter,
    })
}

/// Reads the resources table and packs each `<input-dir>/<fileName>.png`.
fn build(args: &Args) -> Result<PathBuf, String> {
    let bytes = fs::read(&args.resources_json)
        .map_err(|error| format!("read {} failed: {error}", args.resources_json.display()))?;
    let resources: Vec<ShapeResource> = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse resources failed: {error}"))?;
    let options = ShapeAtlasOptions {
        output: args.output.clone(),
        page_size: args.page_size,
        gutter: args.gutter,
    };
    build_shape_atlas(
        &resources,
        |resource| {
            let path = args.input_dir.join(format!("{}.png", resource.file_name));
            fs::read(&path).map_err(|error| format!("read {} failed: {error}", path.display()))
        },
        &options,
    )
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
