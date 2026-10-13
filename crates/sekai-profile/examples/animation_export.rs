//! Runs each card of a corpus through the animation export pipeline and
//! prints one line per requested format: the animated verdict, the frame
//! plan, and the encoded artifact's SHA-256 — a regression anchor for the
//! ordered animation path. Artifacts are written to `--out-dir` when given.
//!
//! ```sh
//! animation_export --masterdata <dir> --card <cards.json> --profile <profile.json> \
//!   --assets-dir <dir> --font-dir <dir> --atlas-dir <dir> \
//!   --render-objects <manifest.json> \
//!   --preset qq-v2 --formats gif,mp4 --executor scalar --out-dir <dir>
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use sekai_profile::animation::{resolve_preset, validate_magic, AnimationFormat};
use sekai_profile::profile::ProfileData;
use sekai_profile::types::UserCustomProfileCard;
use sekai_profile::{
    AssetStore, CustomProfileRenderer, JsonMasterDataProvider, RenderConfig, SdfExecutor,
};
use sha2::{Digest, Sha256};

struct Args {
    masterdata: PathBuf,
    card: PathBuf,
    profile: Option<PathBuf>,
    assets_dir: Option<PathBuf>,
    font_dir: Option<PathBuf>,
    atlas_dir: PathBuf,
    render_objects: Option<PathBuf>,
    out_dir: Option<PathBuf>,
    preset: String,
    formats: Vec<AnimationFormat>,
    executor: String,
    region: String,
}

fn parse_format(value: &str) -> Result<AnimationFormat, String> {
    match value {
        "gif" => Ok(AnimationFormat::Gif),
        "webp" => Ok(AnimationFormat::Webp),
        "apng" => Ok(AnimationFormat::Apng),
        "mp4" => Ok(AnimationFormat::Mp4),
        other => Err(format!("unknown format {other}")),
    }
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        masterdata: PathBuf::new(),
        card: PathBuf::new(),
        profile: None,
        assets_dir: None,
        font_dir: None,
        atlas_dir: PathBuf::from("atlases"),
        render_objects: None,
        out_dir: None,
        preset: "qq-v2".into(),
        formats: vec![AnimationFormat::Gif, AnimationFormat::Mp4],
        executor: "scalar".into(),
        region: "cn".into(),
    };
    let mut raw = std::env::args().skip(1);
    while let Some(flag) = raw.next() {
        let mut value = || raw.next().ok_or_else(|| format!("{flag} requires a value"));
        match flag.as_str() {
            "--masterdata" => args.masterdata = PathBuf::from(value()?),
            "--card" => args.card = PathBuf::from(value()?),
            "--profile" => args.profile = Some(PathBuf::from(value()?)),
            "--assets-dir" => args.assets_dir = Some(PathBuf::from(value()?)),
            "--font-dir" => args.font_dir = Some(PathBuf::from(value()?)),
            "--atlas-dir" => args.atlas_dir = PathBuf::from(value()?),
            "--render-objects" => args.render_objects = Some(PathBuf::from(value()?)),
            "--out-dir" => args.out_dir = Some(PathBuf::from(value()?)),
            "--preset" => args.preset = value()?,
            "--formats" => {
                args.formats = value()?
                    .split(',')
                    .map(parse_format)
                    .collect::<Result<Vec<_>, _>>()?;
            }
            "--executor" => args.executor = value()?,
            "--region" => args.region = value()?,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if args.masterdata.as_os_str().is_empty() || args.card.as_os_str().is_empty() {
        return Err("--masterdata and --card are required".into());
    }
    Ok(args)
}

fn parse_executor(value: &str) -> Result<SdfExecutor, String> {
    match value {
        "auto" => Ok(SdfExecutor::Auto),
        "scalar" => Ok(SdfExecutor::Scalar),
        "simd" => Ok(SdfExecutor::Simd),
        other => Err(format!("unknown executor {other}")),
    }
}

fn main() -> Result<(), String> {
    let args = parse_args()?;
    if let Some(font_dir) = &args.font_dir {
        sekai_profile::set_font_dirs([font_dir]);
    }

    let provider = JsonMasterDataProvider::from_dir(&args.masterdata)
        .map_err(|error| format!("load masterdata: {error}"))?;
    let asset_store = AssetStore::new(1024);
    if let Some(dir) = &args.assets_dir {
        let loaded = asset_store.load_png_dir(dir)?;
        eprintln!("assets loaded: {loaded}");
    }
    let mut renderer = CustomProfileRenderer::new(Arc::new(provider))
        .with_assets(Arc::new(asset_store))
        .with_atlas_dir(&args.atlas_dir)?;
    if let Some(manifest) = &args.render_objects {
        let store = sekai_profile::render_object::MappedRenderObjectStore::open(manifest)
            .map_err(|error| format!("open render objects: {error}"))?;
        renderer = renderer.with_render_object_store(Arc::new(store));
    }

    let card_text = std::fs::read_to_string(&args.card)
        .map_err(|error| format!("read {}: {error}", args.card.display()))?;
    let cards: Vec<UserCustomProfileCard> =
        serde_json::from_str(&card_text).map_err(|error| format!("parse cards: {error}"))?;
    let profile_body: Option<serde_json::Value> = match &args.profile {
        Some(path) => Some(
            serde_json::from_str(
                &std::fs::read_to_string(path)
                    .map_err(|error| format!("read {}: {error}", path.display()))?,
            )
            .map_err(|error| format!("parse profile: {error}"))?,
        ),
        None => None,
    };
    let profile = profile_body.as_ref().map(ProfileData::from_json);
    let config = RenderConfig {
        executor: parse_executor(&args.executor)?,
        ..RenderConfig::default()
    };
    if let Some(out_dir) = &args.out_dir {
        std::fs::create_dir_all(out_dir)
            .map_err(|error| format!("create {}: {error}", out_dir.display()))?;
    }

    for user_card in &cards {
        let card = &user_card.custom_profile_card;
        for format in &args.formats {
            let preset = resolve_preset(&args.preset, Some(*format))?;
            let document_key = format!("seq{}-{}", user_card.seq, preset.format.extension());
            let export = match renderer.render_animation(
                card,
                profile.as_ref(),
                &document_key,
                &args.region,
                &preset,
                &config,
            ) {
                Ok(export) => export,
                Err(error) => {
                    println!("seq{} {} ERROR {error}", user_card.seq, format.extension());
                    continue;
                }
            };
            if !export.animated {
                println!("seq{} static (not animated)", user_card.seq);
                break;
            }
            let Some(encoded) = export.encoded else {
                println!(
                    "seq{} {} ERROR animated without artifact",
                    user_card.seq,
                    format.extension()
                );
                continue;
            };
            validate_magic(encoded.format, &encoded.data)?;
            let digest = hex::encode(Sha256::digest(&encoded.data));
            println!(
                "seq{} {} {}x{} fps {} frames {} looped {} bytes {} sha256 {digest} layer_raster {} scratch_peak {} peak_export {} render_ms {:.1} encode_ms {:.1}",
                user_card.seq,
                encoded.format.extension(),
                encoded.width,
                encoded.height,
                encoded.fps,
                encoded.frame_count,
                encoded.looped,
                encoded.data.len(),
                export.telemetry.layer_raster_bytes,
                export.telemetry.layer_scratch_peak_bytes,
                export.telemetry.peak_export_bytes,
                export.telemetry.render_ms,
                export.telemetry.encode_ms,
            );
            if let Some(out_dir) = &args.out_dir {
                let path = out_dir.join(format!(
                    "seq{}-{}-{}.{}",
                    user_card.seq,
                    args.preset,
                    args.executor,
                    encoded.format.extension()
                ));
                std::fs::write(&path, &encoded.data)
                    .map_err(|error| format!("write {}: {error}", path.display()))?;
            }
        }
    }
    Ok(())
}
