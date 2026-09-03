//! Runs each card of a corpus through the animation export pipeline and
//! prints one line per requested format: the animated verdict, the frame
//! plan, and the encoded artifact's SHA-256 — a regression anchor for the
//! ordered animation path. Artifacts are written to `--out-dir` when given.
//!
//! ```sh
//! animation_export --masterdata <dir> --card <cards.json> --profile <profile.json> \
//!   --assets-dir <dir> --font-dir <dir> \
//!   --text-atlas <manifest.json>... --shape-atlas <manifest.json> \
//!   --render-objects <manifest.json> \
//!   --preset qq-v2 --formats gif,mp4 --executor scalar --out-dir <dir>
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sekai_profile_renderer::animation::{resolve_preset, validate_magic, AnimationFormat};
use sekai_profile_renderer::assets::AssetStore;
use sekai_profile_renderer::profile::ProfileData;
use sekai_profile_renderer::profile_backend::{
    BackendFallbackPolicy, ProfileBackendConfig, ProfileSurfaceBackend, ShapeSdfExecutor,
    TextSdfExecutor,
};
use sekai_profile_renderer::renderer::CustomProfileRenderer;
use sekai_profile_renderer::types::UserCustomProfileCard;
use sekai_profile_renderer_host::JsonMasterDataProvider;
use sha2::{Digest, Sha256};

struct Args {
    masterdata: PathBuf,
    card: PathBuf,
    profile: Option<PathBuf>,
    assets_dir: Option<PathBuf>,
    font_dir: Option<PathBuf>,
    text_atlases: Vec<PathBuf>,
    shape_atlas: Option<PathBuf>,
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
        text_atlases: Vec::new(),
        shape_atlas: None,
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
            "--text-atlas" => args.text_atlases.push(PathBuf::from(value()?)),
            "--shape-atlas" => args.shape_atlas = Some(PathBuf::from(value()?)),
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

fn load_assets(dir: &Path, store: &AssetStore) -> Result<usize, String> {
    let mut count = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current)
            .map_err(|error| format!("read {} failed: {error}", current.display()))?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("png") {
                continue;
            }
            let relative = path
                .strip_prefix(dir)
                .map_err(|error| error.to_string())?
                .with_extension("");
            let key = relative.to_string_lossy().replace('\\', "/");
            let bytes =
                std::fs::read(&path).map_err(|error| format!("read {key} failed: {error}"))?;
            store.put(key, bytes);
            count += 1;
        }
    }
    Ok(count)
}

fn backend_config(executor: &str) -> Result<Option<ProfileBackendConfig>, String> {
    let (text_sdf, shape_sdf) = match executor {
        // A pre-retirement build renders the legacy layer raster when no
        // backend config is supplied; current builds reject the request.
        "legacy" => return Ok(None),
        "scalar" => (
            TextSdfExecutor::ScalarOracle,
            ShapeSdfExecutor::ScalarOracle,
        ),
        "simd" => (TextSdfExecutor::Simd, ShapeSdfExecutor::Simd),
        other => return Err(format!("unknown executor {other}")),
    };
    Ok(Some(ProfileBackendConfig {
        surface: ProfileSurfaceBackend::SkiaRasterCpu,
        text_sdf,
        shape_sdf,
        tile_width: 32,
        tile_height: 32,
        collect_telemetry: true,
        fallback_policy: BackendFallbackPolicy::FailClosed,
        ..ProfileBackendConfig::default()
    }))
}

fn main() -> Result<(), String> {
    let args = parse_args()?;
    if let Some(font_dir) = &args.font_dir {
        std::env::set_var("FONT_DIR", font_dir);
    }

    let provider = JsonMasterDataProvider::from_dir(&args.masterdata)
        .map_err(|error| format!("load masterdata: {error}"))?;
    let asset_store = AssetStore::new(1024);
    if let Some(dir) = &args.assets_dir {
        let injected = load_assets(dir, &asset_store)?;
        eprintln!("assets injected: {injected}");
    }
    let mut renderer =
        CustomProfileRenderer::new(Arc::new(provider)).with_assets(Arc::new(asset_store));
    for manifest in &args.text_atlases {
        let atlas = sekai_profile_renderer::sdf::atlas::MappedSdfAtlas::open(manifest)
            .map_err(|error| format!("open text atlas {}: {error}", manifest.display()))?;
        renderer = renderer
            .with_sdf_atlas(Arc::new(atlas))
            .map_err(|error| format!("install text atlas: {error}"))?;
    }
    if let Some(manifest) = &args.shape_atlas {
        let atlas = sekai_profile_renderer::sdf::shape_atlas::MappedShapeSdfAtlas::open(manifest)
            .map_err(|error| format!("open shape atlas: {error}"))?;
        renderer = renderer.with_shape_sdf_atlas(Arc::new(atlas));
    }
    if let Some(manifest) = &args.render_objects {
        let store = sekai_profile_renderer::render_object::MappedRenderObjectStore::open(manifest)
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
    let config = backend_config(&args.executor)?;
    if let Some(out_dir) = &args.out_dir {
        std::fs::create_dir_all(out_dir)
            .map_err(|error| format!("create {}: {error}", out_dir.display()))?;
    }

    for user_card in &cards {
        let mut card = user_card.custom_profile_card.clone();
        if let Some(body) = &profile_body {
            let (honor_levels, bonds_levels, char_ranks) =
                sekai_profile_renderer::profile::build_honor_maps(body);
            renderer.enrich_honor_levels(&mut card, &honor_levels, &bonds_levels, &char_ranks);
        }
        for format in &args.formats {
            let preset = resolve_preset(&args.preset, Some(*format))?;
            let document_key = format!("seq{}-{}", user_card.seq, preset.format.extension());
            let export = match renderer.render_animation_with_profile_backend(
                &card,
                profile.as_ref(),
                &document_key,
                &args.region,
                &preset,
                config.clone(),
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
