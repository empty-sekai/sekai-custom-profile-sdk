//! Prebaked General base tiles and deck art variants: their identities, their
//! builders and their lookup while a scene is composed.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use sekai_profile_core::profile_scene::ResolvedProfileScene;
use sekai_profile_core::{
    AuthoredElementKind, BlendMode, FontRole, LayerSource, ParameterValue, Rect,
    SemanticCommandPayload, SemanticCommandSource, StableId, TextSource,
};
use sha2::{Digest, Sha256};

use super::{
    canvas_bytes, elapsed_ns, evaluate_command_control_state, raster_image_command,
    render_image_commands_into, semantic_text_glyph, unsupported, DeckArtVariantBuildOutput,
    GeneralBaseAttempt, GeneralBaseBuildOutput, GeneralBasePlan, GeneralBaseTilePlan,
    ImageClipGeometry, ImageExecutor, ProfileCompositorError, ProfileCompositorStats,
    SemanticSdfContext, DECK_ART_SLOT_HEIGHT, DECK_ART_SLOT_WIDTH, DECK_ART_VARIANT_CONTRACT,
    DECK_ART_VARIANT_HEIGHT, DECK_ART_VARIANT_WIDTH, GENERAL_BASE_CONTRACT,
};
use crate::assets::AssetStore;
use crate::masterdata::MasterData;
use crate::render_object::{
    render_object_key_for_resource, MappedRenderObject, MappedRenderObjectStore, RenderObjectKind,
};

pub fn build_general_base_objects_simd(
    scene: &ResolvedProfileScene,
    store: &MappedRenderObjectStore,
    text_atlases: &crate::sdf::atlas::MappedSdfAtlasSet,
    md: &MasterData,
    authored_index: u32,
) -> Result<Vec<GeneralBaseBuildOutput>, ProfileCompositorError> {
    #[cfg(target_arch = "x86_64")]
    if !(std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw"))
    {
        return unsupported("general-base-builder", "AVX-512F+AVX-512BW packet executor");
    }
    #[cfg(not(target_arch = "x86_64"))]
    return unsupported("general-base-builder", "x86_64 packet executor");

    let semantic = SemanticSdfContext { text_atlases, md };
    let Some(plan) = general_base_plan(scene, store, semantic, authored_index)? else {
        return Ok(Vec::new());
    };
    let mut output = Vec::with_capacity(plan.tiles.len());
    for tile in &plan.tiles {
        let width = tile.bounds.width as u32;
        let height = tile.bounds.height as u32;
        let row_bytes = width
            .checked_mul(4)
            .ok_or(ProfileCompositorError::InvalidCanvas { width, height })?;
        let mut pixels = vec![0; canvas_bytes(width, height)?];
        let mut canonical = scene.clone();
        canonical.layers.retain(|layer| layer.id == plan.layer_id);
        let layer = canonical.layers.first_mut().ok_or_else(|| {
            ProfileCompositorError::MissingLayer(format!("general:{}", authored_index))
        })?;
        layer.matrix = [1.0, 0.0, 0.0, 1.0, -tile.bounds.x, -tile.bounds.y];
        canonical
            .commands
            .retain(|command| tile.command_ids.contains(&command.id));
        canonical.interaction_regions.clear();
        let layer_ids = BTreeSet::from([plan.layer_id]);
        let stats = render_image_commands_into(
            &canonical,
            store,
            width,
            height,
            ImageExecutor::Simd,
            &mut pixels,
            Some(&layer_ids),
            Some(semantic),
            Some(&tile.command_ids),
            None,
        )?;
        output.push(GeneralBaseBuildOutput {
            object_key: tile.object_key.clone(),
            source_sha256: tile.source_sha256.clone(),
            general_type: plan.general_type,
            group: tile.group.clone(),
            bounds: tile.bounds,
            width,
            height,
            row_bytes,
            pixels,
            baked_command_count: tile.command_ids.len() as u64,
            overlay_command_count: plan.overlay_command_ids.len() as u64,
            avoided_source_bytes: tile.avoided_source_bytes,
            stats,
        });
    }
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn try_render_general_base_into(
    scene: &ResolvedProfileScene,
    store: &MappedRenderObjectStore,
    semantic: SemanticSdfContext<'_>,
    assets: Option<&AssetStore>,
    authored_index: u32,
    destination: &mut [u8],
    width: u32,
    height: u32,
    executor: ImageExecutor,
) -> Result<GeneralBaseAttempt, ProfileCompositorError> {
    let Some(plan) = general_base_plan(scene, store, semantic, authored_index)? else {
        return Ok(GeneralBaseAttempt::NotEligible);
    };
    let mut bases = Vec::with_capacity(plan.tiles.len());
    for tile in &plan.tiles {
        let Some(base) = store.object(&tile.object_key) else {
            return Ok(GeneralBaseAttempt::Miss);
        };
        if base.entry.kind != RenderObjectKind::Component
            || base.entry.source_sha256 != tile.source_sha256
            || base.entry.width != tile.bounds.width as u32
            || base.entry.height != tile.bounds.height as u32
        {
            return Ok(GeneralBaseAttempt::Miss);
        }
        bases.push((tile, base));
    }
    let layer = scene
        .layers
        .iter()
        .find(|layer| layer.id == plan.layer_id)
        .ok_or_else(|| ProfileCompositorError::MissingLayer(format!("{:?}", plan.layer_id)))?;
    let started = Instant::now();
    let mut base_fragments = 0u64;
    let mut base_packets = 0u64;
    let mut base_scalar_fragments = 0u64;
    let mut base_bytes = 0u64;
    let mut avoided_source_bytes = 0u64;
    let mut baked_command_count = 0u64;
    for (tile, base) in bases {
        let raster = raster_image_command(
            destination,
            width,
            height,
            base,
            tile.bounds,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            [1.0; 4],
            layer.matrix,
            BlendMode::SrcOver,
            "general-base-tile",
            None,
            None,
            None,
            None,
            executor,
        )?;
        base_fragments = base_fragments.saturating_add(raster.fragments);
        base_packets = base_packets.saturating_add(raster.simd_packets);
        base_scalar_fragments = base_scalar_fragments.saturating_add(raster.scalar_fragments);
        base_bytes = base_bytes.saturating_add(base.entry.length);
        avoided_source_bytes = avoided_source_bytes.saturating_add(tile.avoided_source_bytes);
        baked_command_count = baked_command_count.saturating_add(tile.command_ids.len() as u64);
    }
    let base_composite_ns = elapsed_ns(started);
    let layer_ids = BTreeSet::from([plan.layer_id]);
    let mut stats = render_image_commands_into(
        scene,
        store,
        width,
        height,
        executor,
        destination,
        Some(&layer_ids),
        Some(semantic),
        Some(&plan.overlay_command_ids),
        assets,
    )?;
    stats.image_command_count = stats
        .image_command_count
        .saturating_add(plan.tiles.len() as u64);
    stats.sampled_fragment_count = stats.sampled_fragment_count.saturating_add(base_fragments);
    stats.blended_fragment_count = stats.blended_fragment_count.saturating_add(base_fragments);
    stats.simd_packet_count = stats.simd_packet_count.saturating_add(base_packets);
    stats.scalar_fragment_count = stats
        .scalar_fragment_count
        .saturating_add(base_scalar_fragments);
    stats.general_base_hit_count = 1;
    stats.general_base_baked_command_count = baked_command_count;
    stats.general_base_overlay_command_count = plan.overlay_command_ids.len() as u64;
    stats.general_base_bytes = base_bytes;
    stats.general_base_avoided_source_bytes = avoided_source_bytes;
    stats.general_base_composite_ns = base_composite_ns;
    stats.render_ns = stats.render_ns.saturating_add(base_composite_ns);
    Ok(GeneralBaseAttempt::Hit(stats))
}

pub(super) fn general_base_plan(
    scene: &ResolvedProfileScene,
    store: &MappedRenderObjectStore,
    semantic: SemanticSdfContext<'_>,
    authored_index: u32,
) -> Result<Option<GeneralBasePlan>, ProfileCompositorError> {
    let mut layers = scene.layers.iter().filter(|layer| {
        layer.authored_kind == AuthoredElementKind::General
            && layer.authored_index == authored_index
            && layer.authored_visible
    });
    let Some(layer) = layers.next() else {
        return Ok(None);
    };
    if layers.next().is_some() {
        return Ok(None);
    }
    let general_type = match layer.resolved_parameters.get("general_type") {
        Some(ParameterValue::I64(value)) => i32::try_from(*value).ok(),
        _ => None,
    };
    // Type 15 scroll clips remain command-local until a tile-specific clip
    // raster contract is exact. Type 11 has no content clip.
    let Some(general_type @ 11) = general_type else {
        return Ok(None);
    };
    let commands = scene
        .commands
        .iter()
        .filter(|command| command.layer_id == layer.id)
        .map(|command| {
            evaluate_command_control_state(scene, command).map(|state| (command, state.visible))
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter_map(|(command, visible)| visible.then_some(command))
        .collect::<Vec<_>>();
    let mut grouped_static = BTreeMap::<String, Vec<&SemanticCommandSource>>::new();
    let mut overlay_command_ids = BTreeSet::new();
    let mut static_commands = Vec::new();
    let mut overlay_commands = Vec::new();
    for command in commands {
        if command.blend_mode != BlendMode::SrcOver {
            return Ok(None);
        }
        match &command.payload {
            SemanticCommandPayload::Text {
                source: TextSource::ProfileField { .. },
                ..
            } => {
                overlay_command_ids.insert(command.id);
                overlay_commands.push(command);
            }
            SemanticCommandPayload::Text { .. }
            | SemanticCommandPayload::Image { .. }
            | SemanticCommandPayload::Shape { .. } => {
                grouped_static
                    .entry("full".into())
                    .or_default()
                    .push(command);
                static_commands.push(command);
            }
            SemanticCommandPayload::UguiText(_) | SemanticCommandPayload::Composite { .. } => {
                return Ok(None)
            }
        }
    }
    if static_commands.is_empty() || overlay_commands.is_empty() || grouped_static.len() != 1 {
        return Ok(None);
    }
    for dynamic in &overlay_commands {
        let dynamic_position = scene
            .commands
            .iter()
            .position(|candidate| candidate.id == dynamic.id)
            .unwrap_or(usize::MAX);
        for fixed in &static_commands {
            let fixed_position = scene
                .commands
                .iter()
                .position(|candidate| candidate.id == fixed.id)
                .unwrap_or_default();
            if fixed_position > dynamic_position && rects_overlap(dynamic.bounds, fixed.bounds) {
                return Ok(None);
            }
        }
    }
    let mut tiles = Vec::with_capacity(grouped_static.len());
    for (group, commands) in grouped_static {
        let Some(tile) = general_base_tile_plan(general_type, group, commands, store, semantic)?
        else {
            return Ok(None);
        };
        tiles.push(tile);
    }
    Ok(Some(GeneralBasePlan {
        layer_id: layer.id,
        general_type,
        tiles,
        overlay_command_ids,
    }))
}

fn general_base_tile_plan(
    general_type: i32,
    group: String,
    commands: Vec<&SemanticCommandSource>,
    store: &MappedRenderObjectStore,
    semantic: SemanticSdfContext<'_>,
) -> Result<Option<GeneralBaseTilePlan>, ProfileCompositorError> {
    let left = commands
        .iter()
        .map(|command| command.bounds.x)
        .fold(f32::INFINITY, f32::min)
        .floor();
    let top = commands
        .iter()
        .map(|command| command.bounds.y)
        .fold(f32::INFINITY, f32::min)
        .floor();
    let right = commands
        .iter()
        .map(|command| command.bounds.x + command.bounds.width)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil();
    let bottom = commands
        .iter()
        .map(|command| command.bounds.y + command.bounds.height)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil();
    if !left.is_finite()
        || !top.is_finite()
        || !right.is_finite()
        || !bottom.is_finite()
        || right <= left
        || bottom <= top
        || right - left > 4096.0
        || bottom - top > 4096.0
    {
        return Ok(None);
    }
    let bounds = Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    };
    let mut digest = Sha256::new();
    digest.update(GENERAL_BASE_CONTRACT.as_bytes());
    digest.update(general_type.to_le_bytes());
    digest.update(group.as_bytes());
    digest.update(serde_json::to_vec(&bounds).map_err(|error| {
        ProfileCompositorError::UnsupportedFeature {
            role: "general-base".into(),
            feature: format!("serialize bounds: {error}"),
        }
    })?);
    let mut atlas_identities = BTreeSet::new();
    let mut avoided_objects = BTreeSet::new();
    let mut avoided_source_bytes = 0u64;
    let mut command_ids = BTreeSet::new();
    for command in commands {
        command_ids.insert(command.id);
        let mut normalized = command.clone();
        normalized.id = StableId(0);
        normalized.layer_id = StableId(0);
        for binding in &mut normalized.control_bindings {
            match binding {
                sekai_profile_core::CommandControlBinding::TabOption { control_id, .. }
                | sekai_profile_core::CommandControlBinding::ScrollContent { control_id }
                | sekai_profile_core::CommandControlBinding::ScrollThumb { control_id }
                | sekai_profile_core::CommandControlBinding::ScrollViewport { control_id } => {
                    *control_id = StableId(0)
                }
            }
        }
        digest.update(serde_json::to_vec(&normalized).map_err(|error| {
            ProfileCompositorError::UnsupportedFeature {
                role: command.role.clone(),
                feature: format!("serialize General base command: {error}"),
            }
        })?);
        match &command.payload {
            SemanticCommandPayload::Text {
                source, font_role, ..
            } => {
                let value = match source {
                    TextSource::Authored { value }
                    | TextSource::MasterData { value, .. }
                    | TextSource::Localized { value, .. } => value,
                    TextSource::ProfileField { .. } => unreachable!(),
                };
                let font_id = match font_role {
                    FontRole::RegionFontId(font_id) => *font_id,
                };
                let Some(primary_family) = semantic.md.resolve_font_or_default(font_id) else {
                    return Ok(None);
                };
                digest.update(primary_family.as_bytes());
                for codepoint in value.chars().filter(|value| !value.is_whitespace()) {
                    let Some((_, atlas, _)) =
                        semantic_text_glyph(semantic, font_id, u32::from(codepoint))
                    else {
                        return Ok(None);
                    };
                    atlas_identities.insert((
                        atlas.manifest().font_family.clone(),
                        atlas.manifest_sha256().to_string(),
                    ));
                }
            }
            SemanticCommandPayload::Image { resource, .. } => {
                let key = render_object_key_for_resource(resource);
                let Some(object) = store.object(&key) else {
                    return Ok(None);
                };
                digest.update(object.entry.pixel_sha256.as_bytes());
                if avoided_objects.insert(key) {
                    avoided_source_bytes = avoided_source_bytes.saturating_add(object.entry.length);
                }
            }
            SemanticCommandPayload::Shape { .. } => {}
            SemanticCommandPayload::UguiText(_) | SemanticCommandPayload::Composite { .. } => {
                unreachable!()
            }
        }
    }
    for (family, identity) in atlas_identities {
        digest.update(family.as_bytes());
        digest.update(identity.as_bytes());
    }
    let source_sha256 = hex::encode(digest.finalize());
    Ok(Some(GeneralBaseTilePlan {
        object_key: format!(
            "component:general-base/{GENERAL_BASE_CONTRACT}/type-{general_type}/{group}/{source_sha256}"
        ),
        group,
        source_sha256,
        bounds,
        command_ids,
        avoided_source_bytes,
    }))
}

fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

pub fn deck_art_variant_identity(source: MappedRenderObject<'_>) -> (String, String) {
    deck_art_variant_identity_from_source(&source.entry.source_sha256)
}

pub fn deck_art_variant_identity_from_source(source_identity: &str) -> (String, String) {
    let mut digest = Sha256::new();
    digest.update(DECK_ART_VARIANT_CONTRACT.as_bytes());
    digest.update(source_identity.as_bytes());
    let source_sha256 = hex::encode(digest.finalize());
    (
        format!("component:deck-art-variant/{DECK_ART_VARIANT_CONTRACT}/{source_sha256}"),
        source_sha256,
    )
}

pub fn build_deck_art_variant_simd(
    source: MappedRenderObject<'_>,
) -> Result<DeckArtVariantBuildOutput, ProfileCompositorError> {
    #[cfg(target_arch = "x86_64")]
    if !(std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw"))
    {
        return unsupported(
            "deck-art-variant-builder",
            "AVX-512F+AVX-512BW packet executor",
        );
    }
    #[cfg(not(target_arch = "x86_64"))]
    return unsupported("deck-art-variant-builder", "x86_64 packet executor");

    const WIDTH: u32 = DECK_ART_VARIANT_WIDTH;
    const HEIGHT: u32 = DECK_ART_VARIANT_HEIGHT;
    let row_bytes = WIDTH * 4;
    let mut pixels = vec![0; canvas_bytes(WIDTH, HEIGHT)?];
    let uv = deck_art_source_uv(source.entry.width, source.entry.height);
    let started = Instant::now();
    let raster = raster_image_command(
        &mut pixels,
        WIDTH,
        HEIGHT,
        source,
        Rect {
            x: 0.0,
            y: 0.0,
            width: DECK_ART_SLOT_WIDTH,
            height: DECK_ART_SLOT_HEIGHT,
        },
        uv,
        [1.0; 4],
        [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        BlendMode::SrcOver,
        "deck-art-variant-builder",
        None,
        None,
        None,
        None,
        ImageExecutor::Simd,
    )?;
    let (object_key, source_sha256) = deck_art_variant_identity(source);
    Ok(DeckArtVariantBuildOutput {
        object_key,
        source_sha256,
        width: WIDTH,
        height: HEIGHT,
        row_bytes,
        pixels,
        source_bytes: source.entry.length,
        stats: ProfileCompositorStats {
            image_command_count: 1,
            sampled_fragment_count: raster.fragments,
            blended_fragment_count: raster.fragments,
            simd_packet_count: raster.simd_packets,
            scalar_fragment_count: raster.scalar_fragments,
            output_bytes: u64::from(row_bytes) * u64::from(HEIGHT),
            render_ns: elapsed_ns(started),
            ..ProfileCompositorStats::default()
        },
    })
}

/// The part of a baked variant that covers the slot. Sampling exactly this
/// extent maps each slot pixel back onto the variant pixel baked from the same
/// source position.
pub(super) fn deck_art_variant_uv() -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        width: DECK_ART_SLOT_WIDTH / DECK_ART_VARIANT_WIDTH as f32,
        height: DECK_ART_SLOT_HEIGHT / DECK_ART_VARIANT_HEIGHT as f32,
    }
}

pub(super) fn deck_art_source_uv(width: u32, height: u32) -> Rect {
    let width = width as f32;
    let height = height as f32;
    let crop_width = width.min(312.0);
    let crop_height = height.min(512.0);
    Rect {
        x: (width - 312.0).max(0.0) * 0.5 / width.max(f32::EPSILON),
        y: 0.0,
        width: crop_width / width.max(f32::EPSILON),
        height: crop_height / height.max(f32::EPSILON),
    }
}

pub(super) fn deck_art_variant_for_command<'a>(
    store: &'a MappedRenderObjectStore,
    layer: &LayerSource,
    command: &SemanticCommandSource,
    source: MappedRenderObject<'a>,
    uv: Rect,
    tint: [f32; 4],
    image_clip: Option<ImageClipGeometry>,
) -> Option<MappedRenderObject<'a>> {
    let candidate = command.role.starts_with("deck-slot-") && command.role.ends_with("-artwork");
    if !candidate {
        return None;
    }
    let expected_uv = deck_art_source_uv(source.entry.width, source.entry.height);
    let reject_reason = if !matches!(
        layer.resolved_parameters.get("general_type"),
        Some(ParameterValue::I64(3))
    ) {
        Some("general_type")
    } else if tint != [1.0; 4] {
        Some("tint")
    } else if image_clip.is_some() {
        Some("image_clip")
    } else if command.blend_mode != BlendMode::SrcOver {
        Some("blend_mode")
    } else if (command.bounds.width - DECK_ART_SLOT_WIDTH).abs() > 0.001
        || (command.bounds.height - DECK_ART_SLOT_HEIGHT).abs() > 0.001
    {
        Some("bounds")
    } else if (uv.x - expected_uv.x).abs() > f32::EPSILON
        || (uv.y - expected_uv.y).abs() > f32::EPSILON
        || (uv.width - expected_uv.width).abs() > f32::EPSILON
        || (uv.height - expected_uv.height).abs() > f32::EPSILON
    {
        Some("uv")
    } else {
        None
    };
    if let Some(reason) = reject_reason {
        tracing::debug!(
            role = command.role,
            reason,
            bounds = ?command.bounds,
            actual_uv = ?uv,
            expected_uv = ?expected_uv,
            source_width = source.entry.width,
            source_height = source.entry.height,
            "deck art variant command rejected"
        );
        return None;
    }
    let (key, source_sha256) = deck_art_variant_identity(source);
    let Some(variant) = store.object(&key) else {
        tracing::debug!(
            role = command.role,
            key,
            source_key = source.entry.key,
            source_pixel_sha256 = source.entry.pixel_sha256,
            "deck art variant object is missing"
        );
        return None;
    };
    let valid = variant.entry.kind == RenderObjectKind::Component
        && variant.entry.source_sha256 == source_sha256
        && variant.entry.width == DECK_ART_VARIANT_WIDTH
        && variant.entry.height == DECK_ART_VARIANT_HEIGHT;
    if !valid {
        tracing::debug!(
            role = command.role,
            key,
            kind = ?variant.entry.kind,
            width = variant.entry.width,
            height = variant.entry.height,
            "deck art variant object identity mismatch"
        );
        return None;
    }
    Some(variant)
}
