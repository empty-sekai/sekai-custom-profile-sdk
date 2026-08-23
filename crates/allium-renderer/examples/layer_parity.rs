//! Layer parity probe: renders every visible element of each card as a
//! single-element layer through both the legacy renderer and the ordered SDF
//! path, and reports whether the cropped rasters match byte for byte.
//!
//! Text and Shape layers resolve their pixels through the installed SDF
//! atlases, so their verdicts are only meaningful against the atlas
//! generation the comparison targets. Image layers depend on the
//! render-object store alone.
//!
//! ```sh
//! layer-parity --masterdata <dir> --card <cards.json> --profile <profile.json> \
//!   --assets-dir <dir> --font-dir <dir> \
//!   --text-atlas <manifest.json>... --shape-atlas <manifest.json> \
//!   --render-objects <manifest.json> [--page <seq>]
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use allium_renderer::assets::AssetStore;
use allium_renderer::profile::ProfileData;
use allium_renderer::profile_backend::{ShapeSdfExecutor, TextSdfExecutor};
use allium_renderer::renderer::CustomProfileRenderer;
use allium_renderer::types::{CustomProfileCard, UserCustomProfileCard};
use allium_renderer_host::JsonMasterDataProvider;

struct Args {
    masterdata: PathBuf,
    card: PathBuf,
    profile: Option<PathBuf>,
    assets_dir: Option<PathBuf>,
    font_dir: Option<PathBuf>,
    text_atlases: Vec<PathBuf>,
    shape_atlas: Option<PathBuf>,
    render_objects: Option<PathBuf>,
    page: Option<i32>,
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
        page: None,
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
            "--page" => {
                args.page = Some(
                    value()?
                        .parse()
                        .map_err(|error| format!("invalid --page: {error}"))?,
                )
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if args.masterdata.as_os_str().is_empty() || args.card.as_os_str().is_empty() {
        return Err("--masterdata and --card are required".into());
    }
    Ok(args)
}

/// Retains exactly one element of one kind on an otherwise empty card.
fn single_element_cards(card: &CustomProfileCard) -> Vec<(String, CustomProfileCard)> {
    let empty = CustomProfileCard {
        texts: Vec::new(),
        shapes: Vec::new(),
        card_members: Vec::new(),
        stamps: Vec::new(),
        others: Vec::new(),
        bonds_honors: Vec::new(),
        honors: Vec::new(),
        collections: Vec::new(),
        generals: Vec::new(),
        stand_members: Vec::new(),
        general_backgrounds: Vec::new(),
        story_backgrounds: Vec::new(),
        ..card.clone()
    };
    let mut layers = Vec::new();
    macro_rules! split {
        ($field:ident, $label:expr) => {
            for (index, element) in card.$field.iter().enumerate() {
                let mut layer = empty.clone();
                layer.$field = vec![element.clone()];
                layers.push((format!("{}[{index}]", $label), layer));
            }
        };
    }
    split!(texts, "Text");
    split!(shapes, "Shape");
    split!(card_members, "CardMember");
    split!(stamps, "Stamp");
    split!(others, "Other");
    split!(bonds_honors, "BondsHonor");
    split!(honors, "Honor");
    split!(collections, "Collection");
    split!(generals, "General");
    split!(stand_members, "StandMember");
    split!(general_backgrounds, "GeneralBackground");
    split!(story_backgrounds, "StoryBackground");
    layers
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
        let atlas = allium_renderer::sdf::atlas::MappedSdfAtlas::open(manifest)
            .map_err(|error| format!("open text atlas {}: {error}", manifest.display()))?;
        renderer = renderer
            .with_sdf_atlas(Arc::new(atlas))
            .map_err(|error| format!("install text atlas: {error}"))?;
    }
    if let Some(manifest) = &args.shape_atlas {
        let atlas = allium_renderer::sdf::shape_atlas::MappedShapeSdfAtlas::open(manifest)
            .map_err(|error| format!("open shape atlas: {error}"))?;
        renderer = renderer.with_shape_sdf_atlas(Arc::new(atlas));
    }
    if let Some(manifest) = &args.render_objects {
        let store = allium_renderer::render_object::MappedRenderObjectStore::open(manifest)
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

    // The packet shape executor needs AVX-512; hosts without it compare the
    // legacy shape draw on both sides instead.
    let shape_sdf = if std::arch::is_x86_feature_detected!("avx512f") {
        ShapeSdfExecutor::Simd
    } else {
        ShapeSdfExecutor::Skia
    };
    let mut total = 0usize;
    let mut equal = 0usize;
    let mut legacy_fallbacks = 0usize;
    for user_card in &cards {
        if args.page.is_some_and(|page| page != user_card.seq) {
            continue;
        }
        let mut card = user_card.custom_profile_card.clone();
        if let Some(body) = &profile_body {
            let (honor_levels, bonds_levels, char_ranks) =
                allium_renderer::profile::build_honor_maps(body);
            renderer.enrich_honor_levels(&mut card, &honor_levels, &bonds_levels, &char_ranks);
        }
        for (label, layer_card) in single_element_cards(&card) {
            total += 1;
            let legacy =
                match renderer.render_element_layer_cropped(&layer_card, profile.as_ref(), 100) {
                    Ok(legacy) => legacy,
                    Err(error) if error.contains("图层裁剪结果为空") => {
                        // The legacy API refuses fully transparent layers; the
                        // ordered raster reports them as zero-sized instead.
                        match renderer.render_ordered_element_layer_cropped(
                            &layer_card,
                            profile.as_ref(),
                            TextSdfExecutor::ScalarOracle,
                            shape_sdf,
                            true,
                            false,
                        ) {
                            Ok(ordered) if ordered.width == 0 && ordered.height == 0 => {
                                equal += 1;
                                println!("seq{} {label:<24} IDENTICAL (empty)", user_card.seq);
                            }
                            Ok(ordered) => println!(
                                "seq{} {label:<24} DIFF legacy empty vs ordered {}x{}",
                                user_card.seq, ordered.width, ordered.height
                            ),
                            Err(error) => {
                                println!("seq{} {label:<24} ORDERED-ERROR {error}", user_card.seq)
                            }
                        }
                        continue;
                    }
                    Err(error) => {
                        println!("seq{} {label:<24} LEGACY-ERROR {error}", user_card.seq);
                        continue;
                    }
                };
            let ordered = match renderer.render_ordered_element_layer_cropped(
                &layer_card,
                profile.as_ref(),
                TextSdfExecutor::ScalarOracle,
                shape_sdf,
                true,
                false,
            ) {
                Ok(ordered) => ordered,
                Err(error) => {
                    println!("seq{} {label:<24} ORDERED-ERROR {error}", user_card.seq);
                    continue;
                }
            };
            if ordered.legacy_element_count > 0 {
                legacy_fallbacks += 1;
            }
            let decoded = allium_renderer::codec::png::decode(&legacy.data)
                .map_err(|error| format!("decode legacy {label}: {error}"))?;
            // The legacy PNG stores straight RGBA; bring the ordered raster to
            // the same representation with the encoder's own division.
            let mut ordered_straight = ordered.pixels.clone();
            for pixel in ordered_straight.chunks_exact_mut(4) {
                let alpha = pixel[3];
                if alpha != 0 && alpha != u8::MAX {
                    for channel in 0..3 {
                        pixel[channel] = allium_renderer::codec::unpremultiply_channel_like_skia(
                            pixel[channel],
                            alpha,
                        );
                    }
                }
            }
            let bounds_equal = (legacy.x, legacy.y, legacy.width, legacy.height)
                == (ordered.x, ordered.y, ordered.width, ordered.height);
            let pixels_equal = bounds_equal && decoded.pixels == ordered_straight;
            if pixels_equal {
                equal += 1;
                println!(
                    "seq{} {label:<24} IDENTICAL {}x{}{}",
                    user_card.seq,
                    ordered.width,
                    ordered.height,
                    if ordered.legacy_element_count > 0 {
                        "  (via legacy draw)"
                    } else {
                        ""
                    }
                );
            } else if !bounds_equal {
                println!(
                    "seq{} {label:<24} BOUNDS legacy {},{} {}x{} vs ordered {},{} {}x{}",
                    user_card.seq,
                    legacy.x,
                    legacy.y,
                    legacy.width,
                    legacy.height,
                    ordered.x,
                    ordered.y,
                    ordered.width,
                    ordered.height
                );
            } else {
                let mut diff = 0usize;
                let mut max_delta = 0u8;
                for (a, b) in decoded
                    .pixels
                    .chunks_exact(4)
                    .zip(ordered_straight.chunks_exact(4))
                {
                    if a != b {
                        diff += 1;
                        for channel in 0..4 {
                            max_delta = max_delta.max(a[channel].abs_diff(b[channel]));
                        }
                    }
                }
                println!(
                    "seq{} {label:<24} DIFF {diff} px (of {}) max-delta {max_delta}",
                    user_card.seq,
                    ordered.width as usize * ordered.height as usize
                );
                if let Ok(dump_dir) = std::env::var("LAYER_PARITY_DUMP_DIR") {
                    let stem = format!(
                        "{dump_dir}/seq{}-{}",
                        user_card.seq,
                        label.replace(['[', ']'], "-")
                    );
                    let _ = std::fs::write(format!("{stem}legacy.png"), &legacy.data);
                    if let Ok(encoded) = allium_renderer::codec::png::encode_rgba(
                        ordered.width,
                        ordered.height,
                        &ordered_straight,
                    ) {
                        let _ = std::fs::write(format!("{stem}ordered.png"), encoded);
                    }
                }
            }
        }
    }
    println!("total {total} layers, identical {equal}, legacy-draw fallbacks {legacy_fallbacks}");
    Ok(())
}
