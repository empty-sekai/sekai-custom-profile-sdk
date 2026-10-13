//! Renders each card of a corpus on the scalar executor and prints the
//! premultiplied RGBA digest per page — a regression anchor for the ordered
//! pipeline.
//!
//! ```sh
//! ordered_pages --masterdata <dir> --card <cards.json> --profile <profile.json> \
//!   --assets-dir <dir> --font-dir <dir> --atlas-dir <dir> \
//!   --render-objects <manifest.json>
//! ```

use std::path::PathBuf;
use std::sync::Arc;

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
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if args.masterdata.as_os_str().is_empty() || args.card.as_os_str().is_empty() {
        return Err("--masterdata and --card are required".into());
    }
    Ok(args)
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
        executor: SdfExecutor::Scalar,
        ..RenderConfig::default()
    };

    for user_card in &cards {
        let card = &user_card.custom_profile_card;
        match renderer.render(card, profile.as_ref(), &config) {
            Ok(page) => {
                let digest = hex::encode(Sha256::digest(page.premultiplied_rgba()));
                println!(
                    "seq{} {}x{} rgba-sha256 {digest}",
                    user_card.seq,
                    page.width(),
                    page.height()
                );
            }
            Err(error) => println!("seq{} ERROR {error}", user_card.seq),
        }
    }
    Ok(())
}
