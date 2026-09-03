//! Renders each card of a corpus through the ordered scalar path and prints
//! the RGBA digest per page — a regression anchor for the ordered pipeline
//! that needs no raster backend.
//!
//! ```sh
//! ordered_pages --masterdata <dir> --card <cards.json> --profile <profile.json> \
//!   --assets-dir <dir> --font-dir <dir> \
//!   --text-atlas <manifest.json>... --shape-atlas <manifest.json> \
//!   --render-objects <manifest.json>
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sekai_profile_renderer::assets::AssetStore;
use sekai_profile_renderer::profile::ProfileData;
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

    for user_card in &cards {
        let mut card = user_card.custom_profile_card.clone();
        if let Some(body) = &profile_body {
            let (honor_levels, bonds_levels, char_ranks) =
                sekai_profile_renderer::profile::build_honor_maps(body);
            renderer.enrich_honor_levels(&mut card, &honor_levels, &bonds_levels, &char_ranks);
        }
        match renderer.render_full_card_sdf_scalar_f32_candidate(&card, profile.as_ref()) {
            Ok(output) => {
                let digest = hex::encode(Sha256::digest(&output.rgba));
                println!(
                    "seq{} {}x{} rgba-sha256 {digest} legacy-draws {}",
                    user_card.seq, output.width, output.height, output.legacy_element_count
                );
            }
            Err(error) => println!("seq{} ERROR {error}", user_card.seq),
        }
    }
    Ok(())
}
