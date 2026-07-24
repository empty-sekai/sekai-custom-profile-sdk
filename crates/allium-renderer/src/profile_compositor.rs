//! Ordered software composition for compiled custom-profile image commands.
//!
//! The scalar path is the correctness oracle for the Turin-C compositor. It
//! consumes mmap-backed premultiplied RGBA8 objects directly and deliberately
//! fails closed on command features whose pixel contract is not implemented.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use allium_renderer_core::profile_scene::{ComponentControlState, ResolvedProfileScene};
use allium_renderer_core::{
    AuthoredElementKind, BlendMode, CompositeOperation, FontRole, LayerSource, Matrix2d,
    ParameterValue, Rect, ResourceKey, SemanticCommandPayload, SemanticCommandSource,
    ShapePrimitive, StableId, TextSource,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::assets::AssetStore;
use crate::masterdata::MasterData;
use crate::render_object::{
    render_object_key_for_resource, MappedRenderObject, MappedRenderObjectStore, RenderObjectEntry,
    RenderObjectKind,
};

#[cfg(target_arch = "x86_64")]
mod simd_x86;

pub const PROFILE_COMPOSITOR_SCHEMA: &str = "allium.profile-compositor.v1";
pub const GENERAL_BASE_CONTRACT: &str =
    "allium.general-base.sdk-6b0dae58.rgba8-premul-canonical-type11.v4";
pub const DECK_ART_VARIANT_CONTRACT: &str =
    "allium.deck-art-variant.sdk-6b0dae58.crop312x512.slot148x243.v1";

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileCompositorStats {
    pub text_command_count: u64,
    pub shape_command_count: u64,
    pub image_command_count: u64,
    pub composite_command_count: u64,
    pub skipped_text_command_count: u64,
    pub skipped_shape_command_count: u64,
    pub sampled_fragment_count: u64,
    pub blended_fragment_count: u64,
    pub simd_packet_count: u64,
    pub scalar_fragment_count: u64,
    pub isolation_count: u64,
    pub maximum_isolation_depth: u64,
    pub scratch_peak_bytes: u64,
    pub general_base_hit_count: u64,
    pub general_base_miss_count: u64,
    pub general_base_baked_command_count: u64,
    pub general_base_overlay_command_count: u64,
    pub general_base_bytes: u64,
    pub general_base_avoided_source_bytes: u64,
    pub general_base_composite_ns: u64,
    pub deck_art_variant_hit_count: u64,
    pub deck_art_variant_miss_count: u64,
    pub deck_art_variant_bytes: u64,
    pub deck_art_variant_avoided_source_bytes: u64,
    pub source_fallback_object_count: u64,
    pub source_fallback_bytes: u64,
    pub output_bytes: u64,
    pub render_ns: u64,
}

#[derive(Clone, Debug)]
pub struct GeneralBaseBuildOutput {
    pub object_key: String,
    pub source_sha256: String,
    pub general_type: i32,
    pub group: String,
    pub bounds: Rect,
    pub width: u32,
    pub height: u32,
    pub row_bytes: u32,
    pub pixels: Vec<u8>,
    pub baked_command_count: u64,
    pub overlay_command_count: u64,
    pub avoided_source_bytes: u64,
    pub stats: ProfileCompositorStats,
}

#[derive(Clone, Debug)]
pub struct DeckArtVariantBuildOutput {
    pub object_key: String,
    pub source_sha256: String,
    pub width: u32,
    pub height: u32,
    pub row_bytes: u32,
    pub pixels: Vec<u8>,
    pub source_bytes: u64,
    pub stats: ProfileCompositorStats,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileCompositorOutput {
    pub schema: String,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub stats: ProfileCompositorStats,
}

#[derive(Debug, Error)]
pub enum ProfileCompositorError {
    #[error("profile compositor canvas dimensions are invalid: {width}x{height}")]
    InvalidCanvas { width: u32, height: u32 },
    #[error("profile compositor scene contains duplicate layer {0}")]
    DuplicateLayer(String),
    #[error("profile compositor command references missing layer {0}")]
    MissingLayer(String),
    #[error("profile compositor is missing render object {0}")]
    MissingObject(String),
    #[error("profile compositor render object {0} has an invalid pixel layout")]
    InvalidObject(String),
    #[error("profile compositor command {role} has unsupported feature {feature}")]
    UnsupportedFeature { role: String, feature: String },
    #[error("profile compositor command {role} has invalid geometry")]
    InvalidGeometry { role: String },
    #[error("profile compositor isolation stack underflow at command {0}")]
    IsolationUnderflow(String),
    #[error("profile compositor finished with {0} unclosed isolation layer(s)")]
    UnclosedIsolation(usize),
    #[error("profile compositor Skia reference failed: {0}")]
    SkiaReference(String),
    #[error("profile compositor semantic SDF command {role} failed: {reason}")]
    SemanticSdf { role: String, reason: String },
}

struct CompositionTarget {
    pixels: Vec<u8>,
    bounds: CompositionBounds,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompositionBounds {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ImageExecutor {
    Scalar,
    Simd,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RasterImageStats {
    fragments: u64,
    simd_packets: u64,
    scalar_fragments: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AxisAlignedClip {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ImageClipGeometry {
    RoundedRect { radius: [f32; 2] },
    Ellipse,
}

struct OwnedRenderObject {
    entry: RenderObjectEntry,
    pixels: Vec<u8>,
}

impl OwnedRenderObject {
    fn mapped(&self) -> MappedRenderObject<'_> {
        MappedRenderObject {
            entry: &self.entry,
            pixels: &self.pixels,
        }
    }
}

/// Render only Image and Composite commands. Text and Shape are intentionally
/// skipped so this path can be quantified before it is joined to the SDF tile
/// executor; unsupported image semantics return an error instead of drifting.
pub fn render_image_scene_scalar(
    scene: &ResolvedProfileScene,
    store: &MappedRenderObjectStore,
    width: u32,
    height: u32,
) -> Result<ProfileCompositorOutput, ProfileCompositorError> {
    render_image_scene(scene, store, width, height, ImageExecutor::Scalar)
}

/// Render the same Image/Composite subset with the Turin-C packet executor.
/// The scalar entry point remains the independent byte oracle.
pub fn render_image_scene_simd(
    scene: &ResolvedProfileScene,
    store: &MappedRenderObjectStore,
    width: u32,
    height: u32,
) -> Result<ProfileCompositorOutput, ProfileCompositorError> {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
    {
        return render_image_scene(scene, store, width, height, ImageExecutor::Simd);
    }
    unsupported("profile-compositor", "AVX-512F+AVX-512BW packet executor")
}

fn render_image_scene(
    scene: &ResolvedProfileScene,
    store: &MappedRenderObjectStore,
    width: u32,
    height: u32,
    executor: ImageExecutor,
) -> Result<ProfileCompositorOutput, ProfileCompositorError> {
    let pixel_bytes = canvas_bytes(width, height)?;
    let mut pixels = vec![0; pixel_bytes];
    let stats = render_image_commands_into(
        scene,
        store,
        width,
        height,
        executor,
        &mut pixels,
        None,
        None,
        None,
        None,
    )?;
    Ok(ProfileCompositorOutput {
        schema: PROFILE_COMPOSITOR_SCHEMA.into(),
        width,
        height,
        pixels,
        stats,
    })
}

/// Composite one authored element from the shared semantic scene directly
/// into an existing tight premultiplied RGBA8 surface.
pub fn render_authored_image_into_simd(
    scene: &ResolvedProfileScene,
    store: &MappedRenderObjectStore,
    authored_kind: AuthoredElementKind,
    authored_index: u32,
    destination: &mut [u8],
    width: u32,
    height: u32,
) -> Result<ProfileCompositorStats, ProfileCompositorError> {
    #[cfg(target_arch = "x86_64")]
    if !(std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw"))
    {
        return unsupported("profile-compositor", "AVX-512F+AVX-512BW packet executor");
    }
    #[cfg(not(target_arch = "x86_64"))]
    return unsupported("profile-compositor", "x86_64 packet executor");

    let layer_ids = scene
        .layers
        .iter()
        .filter(|layer| {
            layer.authored_kind == authored_kind && layer.authored_index == authored_index
        })
        .map(|layer| layer.id)
        .collect::<BTreeSet<_>>();
    if layer_ids.is_empty() {
        return Err(ProfileCompositorError::MissingLayer(format!(
            "{authored_kind:?}:{authored_index}"
        )));
    }
    render_image_commands_into(
        scene,
        store,
        width,
        height,
        ImageExecutor::Simd,
        destination,
        Some(&layer_ids),
        None,
        None,
        None,
    )
}

pub fn render_authored_profile_into_simd(
    scene: &ResolvedProfileScene,
    store: &MappedRenderObjectStore,
    text_atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
    md: &MasterData,
    assets: Option<&AssetStore>,
    authored_kind: AuthoredElementKind,
    authored_index: u32,
    destination: &mut [u8],
    width: u32,
    height: u32,
) -> Result<ProfileCompositorStats, ProfileCompositorError> {
    #[cfg(target_arch = "x86_64")]
    if !(std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw"))
    {
        return unsupported("profile-compositor", "AVX-512F+AVX-512BW packet executor");
    }
    #[cfg(not(target_arch = "x86_64"))]
    return unsupported("profile-compositor", "x86_64 packet executor");

    let layer_ids = scene
        .layers
        .iter()
        .filter(|layer| {
            layer.authored_kind == authored_kind && layer.authored_index == authored_index
        })
        .map(|layer| layer.id)
        .collect::<BTreeSet<_>>();
    if layer_ids.is_empty() {
        return Err(ProfileCompositorError::MissingLayer(format!(
            "{authored_kind:?}:{authored_index}"
        )));
    }
    let semantic = text_atlases.map(|text_atlases| SemanticSdfContext { text_atlases, md });
    let mut general_base_miss = false;
    if authored_kind == AuthoredElementKind::General {
        if let Some(semantic) = semantic {
            match try_render_general_base_into(
                scene,
                store,
                semantic,
                assets,
                authored_index,
                destination,
                width,
                height,
            )? {
                GeneralBaseAttempt::Hit(stats) => return Ok(stats),
                GeneralBaseAttempt::Miss => general_base_miss = true,
                GeneralBaseAttempt::NotEligible => {}
            }
        }
    }
    let mut stats = render_image_commands_into(
        scene,
        store,
        width,
        height,
        ImageExecutor::Simd,
        destination,
        Some(&layer_ids),
        semantic,
        None,
        assets,
    )?;
    stats.general_base_miss_count = u64::from(general_base_miss);
    Ok(stats)
}

#[derive(Clone, Copy)]
struct SemanticSdfContext<'a> {
    text_atlases: &'a crate::sdf::atlas::MappedSdfAtlasSet,
    md: &'a MasterData,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct EvaluatedCommandControlState {
    visible: bool,
    translate_y: f32,
}

fn evaluate_command_control_state(
    scene: &ResolvedProfileScene,
    command: &SemanticCommandSource,
) -> Result<EvaluatedCommandControlState, ProfileCompositorError> {
    let mut evaluated = EvaluatedCommandControlState {
        visible: true,
        translate_y: 0.0,
    };
    for binding in &command.control_bindings {
        let control_id = match binding {
            allium_renderer_core::CommandControlBinding::TabOption { control_id, .. }
            | allium_renderer_core::CommandControlBinding::ScrollContent { control_id }
            | allium_renderer_core::CommandControlBinding::ScrollThumb { control_id }
            | allium_renderer_core::CommandControlBinding::ScrollViewport { control_id } => {
                *control_id
            }
        };
        let control = scene
            .controls
            .iter()
            .find(|control| control.id == control_id)
            .ok_or_else(|| ProfileCompositorError::UnsupportedFeature {
                role: command.role.clone(),
                feature: format!("missing component control {:016x}", control_id.0),
            })?;
        match (binding, &control.state) {
            (
                allium_renderer_core::CommandControlBinding::TabOption { value, .. },
                ComponentControlState::Tabs { active, .. },
            ) => evaluated.visible &= value == active,
            (
                allium_renderer_core::CommandControlBinding::ScrollContent { .. },
                ComponentControlState::Scroll { offset, .. },
            ) => evaluated.translate_y -= *offset,
            (
                allium_renderer_core::CommandControlBinding::ScrollThumb { .. },
                ComponentControlState::Scroll {
                    offset,
                    min,
                    max,
                    viewport_extent,
                    ..
                },
            ) => {
                let travel = (*viewport_extent - command.bounds.height).max(0.0);
                let progress = if max > min {
                    (*offset - *min) / (*max - *min)
                } else {
                    0.0
                };
                evaluated.translate_y += travel * progress;
            }
            (
                allium_renderer_core::CommandControlBinding::ScrollViewport { .. },
                ComponentControlState::Scroll { .. },
            ) => {}
            _ => {
                return Err(ProfileCompositorError::UnsupportedFeature {
                    role: command.role.clone(),
                    feature: format!("mismatched component control {:016x}", control_id.0),
                });
            }
        }
    }
    Ok(evaluated)
}

fn semantic_text_glyph<'a>(
    context: SemanticSdfContext<'a>,
    font_id: i32,
    codepoint: u32,
) -> Option<(
    u16,
    &'a crate::sdf::atlas::MappedSdfAtlas,
    &'a crate::sdf::atlas::SdfAtlasGlyphManifest,
)> {
    let primary_family = context.md.resolve_font(font_id)?;
    context
        .text_atlases
        .profile_glyph_for_font_family(&primary_family, codepoint)
}

#[derive(Clone, Debug)]
struct GeneralBaseTilePlan {
    group: String,
    object_key: String,
    source_sha256: String,
    bounds: Rect,
    command_ids: BTreeSet<StableId>,
    avoided_source_bytes: u64,
}

#[derive(Clone, Debug)]
struct GeneralBasePlan {
    layer_id: StableId,
    general_type: i32,
    tiles: Vec<GeneralBaseTilePlan>,
    overlay_command_ids: BTreeSet<StableId>,
}

enum GeneralBaseAttempt {
    NotEligible,
    Miss,
    Hit(ProfileCompositorStats),
}

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

fn try_render_general_base_into(
    scene: &ResolvedProfileScene,
    store: &MappedRenderObjectStore,
    semantic: SemanticSdfContext<'_>,
    assets: Option<&AssetStore>,
    authored_index: u32,
    destination: &mut [u8],
    width: u32,
    height: u32,
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
            ImageExecutor::Simd,
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
        ImageExecutor::Simd,
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

fn general_base_plan(
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
            SemanticCommandPayload::Composite { .. } => return Ok(None),
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
                allium_renderer_core::CommandControlBinding::TabOption { control_id, .. }
                | allium_renderer_core::CommandControlBinding::ScrollContent { control_id }
                | allium_renderer_core::CommandControlBinding::ScrollThumb { control_id }
                | allium_renderer_core::CommandControlBinding::ScrollViewport { control_id } => {
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
                let Some(primary_family) = semantic.md.resolve_font(font_id) else {
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
            SemanticCommandPayload::Composite { .. } => unreachable!(),
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

    const WIDTH: u32 = 149;
    const HEIGHT: u32 = 243;
    const CARD_WIDTH: f32 = 148.078_13;
    const CARD_HEIGHT: f32 = 243.0;
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
            width: CARD_WIDTH,
            height: CARD_HEIGHT,
        },
        uv,
        [1.0; 4],
        [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        BlendMode::SrcOver,
        "deck-art-variant-builder",
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

fn deck_art_source_uv(width: u32, height: u32) -> Rect {
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

fn deck_art_variant_for_command<'a>(
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
    } else if (command.bounds.width - 148.078_13).abs() > 0.001
        || (command.bounds.height - 243.0).abs() > 0.001
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
        && variant.entry.width == 149
        && variant.entry.height == 243;
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

/// Returns true only when an authored element is a non-empty sequence of
/// SrcOver image commands. Rendering that sequence over transparent RGBA and
/// selecting its alpha-255 pixels then proves those destination pixels are
/// independent of everything below the element.
pub fn authored_image_is_exact_opaque_mask_source(
    scene: &ResolvedProfileScene,
    authored_kind: AuthoredElementKind,
    authored_index: u32,
) -> bool {
    let layer_ids = scene
        .layers
        .iter()
        .filter(|layer| {
            layer.authored_visible
                && layer.authored_kind == authored_kind
                && layer.authored_index == authored_index
        })
        .map(|layer| layer.id)
        .collect::<BTreeSet<_>>();
    if layer_ids.is_empty() {
        return false;
    }
    let mut image_count = 0usize;
    for command in scene
        .commands
        .iter()
        .filter(|command| layer_ids.contains(&command.layer_id))
    {
        match &command.payload {
            SemanticCommandPayload::Image { .. } if command.blend_mode == BlendMode::SrcOver => {
                image_count += 1;
            }
            _ => return false,
        }
    }
    image_count != 0
}

#[allow(clippy::too_many_arguments)]
fn render_image_commands_into(
    scene: &ResolvedProfileScene,
    store: &MappedRenderObjectStore,
    width: u32,
    height: u32,
    executor: ImageExecutor,
    destination: &mut [u8],
    layer_filter: Option<&BTreeSet<StableId>>,
    semantic_sdf: Option<SemanticSdfContext<'_>>,
    command_filter: Option<&BTreeSet<StableId>>,
    assets: Option<&AssetStore>,
) -> Result<ProfileCompositorStats, ProfileCompositorError> {
    let started = Instant::now();
    let pixel_bytes = canvas_bytes(width, height)?;
    if destination.len() != pixel_bytes {
        return Err(ProfileCompositorError::InvalidCanvas { width, height });
    }
    let layers = collect_layers(scene)?;
    let mut targets = Vec::<CompositionTarget>::new();
    let mut target_scratch = Vec::<Vec<u8>>::new();
    let mut stats = ProfileCompositorStats {
        output_bytes: pixel_bytes as u64,
        scratch_peak_bytes: pixel_bytes as u64,
        ..ProfileCompositorStats::default()
    };

    if let Some(semantic_sdf) = semantic_sdf {
        let mut text_batch = Vec::new();
        let mut text_only = true;
        for command in &scene.commands {
            if layer_filter.is_some_and(|filter| !filter.contains(&command.layer_id))
                || command_filter.is_some_and(|filter| !filter.contains(&command.id))
            {
                continue;
            }
            let layer = layers.get(&command.layer_id).copied().ok_or_else(|| {
                ProfileCompositorError::MissingLayer(format!("{:?}", command.layer_id))
            })?;
            let control_state = evaluate_command_control_state(scene, command)?;
            if !layer.authored_visible || !control_state.visible {
                continue;
            }
            if is_live_master_progress_command(command) {
                text_only = false;
                break;
            } else if matches!(&command.payload, SemanticCommandPayload::Text { .. })
                && command.blend_mode == BlendMode::SrcOver
            {
                text_batch.push((command, layer, control_state.translate_y));
            } else {
                text_only = false;
                break;
            }
        }
        if text_only && !text_batch.is_empty() {
            let execution = render_semantic_text_commands(
                &text_batch,
                semantic_sdf,
                destination,
                width,
                height,
                executor,
            )?;
            stats.text_command_count = text_batch.len() as u64;
            stats.blended_fragment_count = execution.blended_fragment_count;
            stats.simd_packet_count = execution.simd_packet_count;
            stats.render_ns = elapsed_ns(started);
            return Ok(stats);
        }
    }

    for command in &scene.commands {
        if layer_filter.is_some_and(|filter| !filter.contains(&command.layer_id)) {
            continue;
        }
        if command_filter.is_some_and(|filter| !filter.contains(&command.id)) {
            continue;
        }
        let layer = layers.get(&command.layer_id).copied().ok_or_else(|| {
            ProfileCompositorError::MissingLayer(format!("{:?}", command.layer_id))
        })?;
        if !layer.authored_visible {
            continue;
        }
        let control_state = evaluate_command_control_state(scene, command)?;
        if !control_state.visible {
            continue;
        }
        match &command.payload {
            SemanticCommandPayload::Text { .. } if is_live_master_progress_command(command) => {
                let target = targets
                    .last_mut()
                    .map(|target| target.pixels.as_mut_slice())
                    .unwrap_or(destination);
                #[cfg(feature = "skia-core")]
                render_live_master_progress_text(
                    command,
                    layer,
                    control_state.translate_y,
                    target,
                    width,
                    height,
                )?;
                #[cfg(not(feature = "skia"))]
                {
                    let _ = (target, width, height);
                    stats.skipped_text_command_count =
                        stats.skipped_text_command_count.saturating_add(1);
                    continue;
                }
                stats.text_command_count = stats.text_command_count.saturating_add(1);
            }
            SemanticCommandPayload::Text { .. } => {
                let Some(semantic_sdf) = semantic_sdf else {
                    stats.skipped_text_command_count =
                        stats.skipped_text_command_count.saturating_add(1);
                    continue;
                };
                let target = targets
                    .last_mut()
                    .map(|target| target.pixels.as_mut_slice())
                    .unwrap_or(destination);
                let execution = render_semantic_text_command(
                    command,
                    layer,
                    control_state.translate_y,
                    semantic_sdf,
                    target,
                    width,
                    height,
                    executor,
                )?;
                stats.text_command_count = stats.text_command_count.saturating_add(1);
                stats.blended_fragment_count = stats
                    .blended_fragment_count
                    .saturating_add(execution.blended_fragment_count);
                stats.simd_packet_count = stats
                    .simd_packet_count
                    .saturating_add(execution.simd_packet_count);
            }
            SemanticCommandPayload::Shape {
                primitive,
                fill,
                gradient,
                stroke,
                stroke_width,
            } => {
                if matches!(primitive, ShapePrimitive::AssetMask { .. }) {
                    stats.skipped_shape_command_count =
                        stats.skipped_shape_command_count.saturating_add(1);
                    continue;
                }
                let target = targets
                    .last_mut()
                    .map(|target| target.pixels.as_mut_slice())
                    .unwrap_or(destination);
                let raster = raster_semantic_shape_command(
                    target,
                    width,
                    height,
                    command,
                    layer,
                    primitive,
                    *fill,
                    gradient.as_ref(),
                    *stroke,
                    *stroke_width,
                    control_state.translate_y,
                    executor,
                )?;
                stats.shape_command_count = stats.shape_command_count.saturating_add(1);
                stats.blended_fragment_count = stats
                    .blended_fragment_count
                    .saturating_add(raster.fragments);
                stats.simd_packet_count =
                    stats.simd_packet_count.saturating_add(raster.simd_packets);
                stats.scalar_fragment_count = stats
                    .scalar_fragment_count
                    .saturating_add(raster.scalar_fragments);
            }
            SemanticCommandPayload::Image {
                resource,
                uv,
                tint,
                clip,
                alpha_mask,
            } => {
                let mut translated_matrix = command.matrix;
                translated_matrix[5] += control_state.translate_y;
                let matrix = compose_matrix(layer.matrix, translated_matrix);
                let (command_clip, image_clip) = validate_image_command(
                    command.role.as_str(),
                    command.clip.as_ref(),
                    command.bounds,
                    layer.matrix,
                    matrix,
                    clip.as_ref(),
                    alpha_mask.as_ref(),
                    &command.metadata,
                )?;
                let object_key = render_object_key_for_resource(resource);
                let mapped_object = store.object(&object_key);
                let source_fallback = if mapped_object.is_none() {
                    load_source_fallback_object(assets, resource)?
                } else {
                    None
                };
                let object = mapped_object
                    .or_else(|| source_fallback.as_ref().map(OwnedRenderObject::mapped))
                    .ok_or_else(|| ProfileCompositorError::MissingObject(object_key.clone()))?;
                if source_fallback.is_some() {
                    stats.source_fallback_object_count =
                        stats.source_fallback_object_count.saturating_add(1);
                    stats.source_fallback_bytes = stats
                        .source_fallback_bytes
                        .saturating_add(object.pixels.len() as u64);
                }
                let variant = deck_art_variant_for_command(
                    store, layer, command, object, *uv, *tint, image_clip,
                );
                let (object, effective_uv) = if let Some(variant) = variant {
                    stats.deck_art_variant_hit_count =
                        stats.deck_art_variant_hit_count.saturating_add(1);
                    stats.deck_art_variant_bytes = stats
                        .deck_art_variant_bytes
                        .saturating_add(variant.entry.length);
                    stats.deck_art_variant_avoided_source_bytes = stats
                        .deck_art_variant_avoided_source_bytes
                        .saturating_add(object.entry.length);
                    (
                        variant,
                        Rect {
                            x: 0.0,
                            y: 0.0,
                            width: 1.0,
                            height: 1.0,
                        },
                    )
                } else {
                    if matches!(
                        layer.resolved_parameters.get("general_type"),
                        Some(ParameterValue::I64(3))
                    ) && command.role.starts_with("deck-slot-")
                        && command.role.ends_with("-artwork")
                    {
                        stats.deck_art_variant_miss_count =
                            stats.deck_art_variant_miss_count.saturating_add(1);
                    }
                    (object, *uv)
                };
                let target = targets
                    .last_mut()
                    .map(|target| target.pixels.as_mut_slice())
                    .unwrap_or(destination);
                let raster = raster_image_command(
                    target,
                    width,
                    height,
                    object,
                    command.bounds,
                    effective_uv,
                    *tint,
                    matrix,
                    command.blend_mode,
                    &command.role,
                    command_clip,
                    image_clip,
                    executor,
                )?;
                stats.image_command_count = stats.image_command_count.saturating_add(1);
                stats.sampled_fragment_count = stats
                    .sampled_fragment_count
                    .saturating_add(raster.fragments);
                stats.blended_fragment_count = stats
                    .blended_fragment_count
                    .saturating_add(raster.fragments);
                stats.simd_packet_count =
                    stats.simd_packet_count.saturating_add(raster.simd_packets);
                stats.scalar_fragment_count = stats
                    .scalar_fragment_count
                    .saturating_add(raster.scalar_fragments);
            }
            SemanticCommandPayload::Composite {
                operation,
                opacity,
                clip,
            } => {
                if control_state.translate_y.to_bits() != 0.0f32.to_bits() {
                    return unsupported(&command.role, "scroll-translated composite");
                }
                stats.composite_command_count = stats.composite_command_count.saturating_add(1);
                validate_composite_command(&command.role, *opacity, clip.as_ref())?;
                match operation {
                    CompositeOperation::Marker => {}
                    CompositeOperation::BeginIsolation => {
                        let bounds = composition_bounds(
                            command.bounds,
                            compose_matrix(layer.matrix, command.matrix),
                            width,
                            height,
                        );
                        let mut pixels =
                            target_scratch.pop().unwrap_or_else(|| vec![0; pixel_bytes]);
                        clear_composition_bounds(&mut pixels, width, bounds);
                        targets.push(CompositionTarget { pixels, bounds });
                        stats.isolation_count = stats.isolation_count.saturating_add(1);
                        let depth = targets.len() as u64;
                        stats.maximum_isolation_depth = stats.maximum_isolation_depth.max(depth);
                        let scratch_buffers = targets.len().saturating_add(target_scratch.len());
                        stats.scratch_peak_bytes = stats
                            .scratch_peak_bytes
                            .max((scratch_buffers as u64 + 1).saturating_mul(pixel_bytes as u64));
                    }
                    CompositeOperation::EndIsolation => {
                        if targets.is_empty() {
                            return Err(ProfileCompositorError::IsolationUnderflow(
                                command.role.clone(),
                            ));
                        }
                        let child = targets.pop().ok_or_else(|| {
                            ProfileCompositorError::IsolationUnderflow(command.role.clone())
                        })?;
                        {
                            let parent = targets
                                .last_mut()
                                .map(|target| target.pixels.as_mut_slice())
                                .unwrap_or(destination);
                            composite_isolation(
                                parent,
                                &child.pixels,
                                width,
                                child.bounds,
                                executor,
                            );
                        }
                        target_scratch.push(child.pixels);
                    }
                }
            }
        }
    }

    if !targets.is_empty() {
        return Err(ProfileCompositorError::UnclosedIsolation(targets.len()));
    }
    stats.render_ns = elapsed_ns(started);
    Ok(stats)
}

fn is_live_master_progress_command(command: &SemanticCommandSource) -> bool {
    command.role.starts_with("honor-") && command.role.ends_with("-progress")
}

#[cfg(feature = "skia-core")]
fn render_live_master_progress_text(
    command: &SemanticCommandSource,
    layer: &LayerSource,
    translate_y: f32,
    destination: &mut [u8],
    width: u32,
    height: u32,
) -> Result<(), ProfileCompositorError> {
    use skia_safe::{surfaces, AlphaType, ColorType, ImageInfo, Matrix};

    let SemanticCommandPayload::Text { source, size, .. } = &command.payload else {
        return Err(ProfileCompositorError::SemanticSdf {
            role: command.role.clone(),
            reason: "live-master progress command is not text".into(),
        });
    };
    let text = match source {
        TextSource::Authored { value }
        | TextSource::ProfileField { value, .. }
        | TextSource::MasterData { value, .. }
        | TextSource::Localized { value, .. } => value,
    };
    let lowered_general = command.render_placement.is_some();
    let font_size = if lowered_general { *size * 2.0 } else { *size };
    let baseline = command
        .render_placement
        .as_ref()
        .and_then(|placement| placement.baseline)
        .map(|value| value * 2.0)
        .unwrap_or(font_size / 2.0);
    let dimensions = (
        i32::try_from(width)
            .map_err(|_| ProfileCompositorError::InvalidCanvas { width, height })?,
        i32::try_from(height)
            .map_err(|_| ProfileCompositorError::InvalidCanvas { width, height })?,
    );
    let info = ImageInfo::new(dimensions, ColorType::RGBA8888, AlphaType::Premul, None);
    let mut surface = surfaces::wrap_pixels(&info, destination, Some(width as usize * 4), None)
        .ok_or_else(|| ProfileCompositorError::SemanticSdf {
            role: command.role.clone(),
            reason: "wrap live-master progress destination".into(),
        })?;
    let mut local_matrix = command.matrix;
    local_matrix[5] += translate_y;
    let matrix = compose_matrix(layer.matrix, local_matrix);
    let canvas = surface.canvas();
    canvas.save();
    canvas.concat(&Matrix::from_affine(&matrix));
    let rendered =
        crate::elements::honor::draw_live_master_progress_text(canvas, text, 0.0, baseline);
    canvas.restore();
    if !rendered {
        return Err(ProfileCompositorError::SemanticSdf {
            role: command.role.clone(),
            reason: "live-master progress typeface is unavailable".into(),
        });
    }
    Ok(())
}

/// Validation-only Skia implementation of the same image command subset.
/// It is intentionally separate from the scalar oracle and is never selected
/// by the production backend.
#[cfg(feature = "skia-core")]
pub fn render_image_scene_skia_reference(
    scene: &ResolvedProfileScene,
    store: &MappedRenderObjectStore,
    width: u32,
    height: u32,
) -> Result<ProfileCompositorOutput, ProfileCompositorError> {
    use skia_safe::canvas::{SaveLayerRec, SrcRectConstraint};
    use skia_safe::{
        images, surfaces, AlphaType, Color, ColorType, Data, Image, ImageInfo, Matrix, Paint,
        Rect as SkiaRect,
    };

    let started = Instant::now();
    let pixel_bytes = canvas_bytes(width, height)?;
    let layers = collect_layers(scene)?;
    let dimensions = (
        i32::try_from(width)
            .map_err(|_| ProfileCompositorError::InvalidCanvas { width, height })?,
        i32::try_from(height)
            .map_err(|_| ProfileCompositorError::InvalidCanvas { width, height })?,
    );
    let mut surface = surfaces::raster_n32_premul(dimensions)
        .ok_or_else(|| ProfileCompositorError::SkiaReference("create raster surface".into()))?;
    surface.canvas().clear(Color::TRANSPARENT);
    let mut images_by_key = BTreeMap::<String, Image>::new();
    let mut isolation_depth = 0usize;
    let mut stats = ProfileCompositorStats {
        output_bytes: pixel_bytes as u64,
        scratch_peak_bytes: pixel_bytes as u64,
        ..ProfileCompositorStats::default()
    };

    for command in &scene.commands {
        let layer = layers.get(&command.layer_id).copied().ok_or_else(|| {
            ProfileCompositorError::MissingLayer(format!("{:?}", command.layer_id))
        })?;
        if !layer.authored_visible {
            continue;
        }
        match &command.payload {
            SemanticCommandPayload::Text { .. } => {
                stats.skipped_text_command_count =
                    stats.skipped_text_command_count.saturating_add(1);
            }
            SemanticCommandPayload::Shape { .. } => {
                stats.skipped_shape_command_count =
                    stats.skipped_shape_command_count.saturating_add(1);
            }
            SemanticCommandPayload::Image {
                resource,
                uv,
                tint,
                clip,
                alpha_mask,
            } => {
                let matrix = compose_matrix(layer.matrix, command.matrix);
                let (command_clip, image_clip) = validate_image_command(
                    &command.role,
                    command.clip.as_ref(),
                    command.bounds,
                    layer.matrix,
                    matrix,
                    clip.as_ref(),
                    alpha_mask.as_ref(),
                    &command.metadata,
                )?;
                if !tint.iter().all(|value| value.to_bits() == 1.0f32.to_bits()) {
                    return unsupported(&command.role, "Skia reference tint");
                }
                let object_key = render_object_key_for_resource(resource);
                if !images_by_key.contains_key(&object_key) {
                    let object = store
                        .object(&object_key)
                        .ok_or_else(|| ProfileCompositorError::MissingObject(object_key.clone()))?;
                    validate_object(object, &command.role)?;
                    let info = ImageInfo::new(
                        (object.entry.width as i32, object.entry.height as i32),
                        ColorType::RGBA8888,
                        AlphaType::Premul,
                        None,
                    );
                    let image = images::raster_from_data(
                        &info,
                        Data::new_copy(object.pixels),
                        object.entry.row_bytes as usize,
                    )
                    .ok_or_else(|| {
                        ProfileCompositorError::SkiaReference(format!("create image {object_key}"))
                    })?;
                    images_by_key.insert(object_key.clone(), image);
                }
                let image = images_by_key
                    .get(&object_key)
                    .ok_or_else(|| ProfileCompositorError::MissingObject(object_key.clone()))?;
                validate_geometry(command.bounds, *uv, *tint, matrix, &command.role)?;
                let canvas = surface.canvas();
                canvas.save();
                if let Some(clip) = command_clip {
                    canvas.clip_rect(
                        SkiaRect::new(clip.min_x, clip.min_y, clip.max_x, clip.max_y),
                        None,
                        Some(false),
                    );
                }
                canvas.concat(&Matrix::from_affine(&matrix));
                let destination = SkiaRect::from_xywh(
                    command.bounds.x,
                    command.bounds.y,
                    command.bounds.width,
                    command.bounds.height,
                );
                if let Some(clip) = image_clip {
                    match clip {
                        ImageClipGeometry::RoundedRect { radius } => {
                            canvas.clip_rrect(
                                skia_safe::RRect::new_rect_xy(destination, radius[0], radius[1]),
                                None,
                                Some(false),
                            );
                        }
                        ImageClipGeometry::Ellipse => {
                            let mut builder = skia_safe::PathBuilder::new();
                            builder.add_oval(destination, None, None);
                            canvas.clip_path(
                                &builder.detach(),
                                skia_safe::ClipOp::Intersect,
                                false,
                            );
                        }
                    }
                }
                let mut paint = Paint::default();
                paint.set_blend_mode(skia_blend_mode(command.blend_mode, &command.role)?);
                let source = SkiaRect::from_xywh(
                    uv.x * object_width(image),
                    uv.y * object_height(image),
                    uv.width * object_width(image),
                    uv.height * object_height(image),
                );
                if uses_implicit_full_source(*uv, &command.metadata) {
                    canvas.draw_image_rect(image, None, destination, &paint);
                } else {
                    let constraint = match command.metadata.get("source_constraint") {
                        Some(ParameterValue::Text(value)) if value == "strict" => {
                            SrcRectConstraint::Strict
                        }
                        _ => SrcRectConstraint::Fast,
                    };
                    canvas.draw_image_rect(image, Some((&source, constraint)), destination, &paint);
                }
                canvas.restore();
                stats.image_command_count = stats.image_command_count.saturating_add(1);
            }
            SemanticCommandPayload::Composite {
                operation,
                opacity,
                clip,
            } => {
                stats.composite_command_count = stats.composite_command_count.saturating_add(1);
                validate_composite_command(&command.role, *opacity, clip.as_ref())?;
                match operation {
                    CompositeOperation::Marker => {}
                    CompositeOperation::BeginIsolation => {
                        surface.canvas().save_layer(&SaveLayerRec::default());
                        isolation_depth = isolation_depth.saturating_add(1);
                        stats.isolation_count = stats.isolation_count.saturating_add(1);
                        stats.maximum_isolation_depth =
                            stats.maximum_isolation_depth.max(isolation_depth as u64);
                    }
                    CompositeOperation::EndIsolation => {
                        if isolation_depth == 0 {
                            return Err(ProfileCompositorError::IsolationUnderflow(
                                command.role.clone(),
                            ));
                        }
                        surface.canvas().restore();
                        isolation_depth -= 1;
                    }
                }
            }
        }
    }
    if isolation_depth != 0 {
        return Err(ProfileCompositorError::UnclosedIsolation(isolation_depth));
    }

    let info = ImageInfo::new(dimensions, ColorType::RGBA8888, AlphaType::Premul, None);
    let mut pixels = vec![0; pixel_bytes];
    if !surface.read_pixels(&info, &mut pixels, width as usize * 4, (0, 0)) {
        return Err(ProfileCompositorError::SkiaReference(
            "read RGBA8 pixels".into(),
        ));
    }
    stats.render_ns = elapsed_ns(started);
    Ok(ProfileCompositorOutput {
        schema: PROFILE_COMPOSITOR_SCHEMA.into(),
        width,
        height,
        pixels,
        stats,
    })
}

#[cfg(feature = "skia-core")]
fn skia_blend_mode(
    blend_mode: BlendMode,
    role: &str,
) -> Result<skia_safe::BlendMode, ProfileCompositorError> {
    Ok(match blend_mode {
        BlendMode::SrcOver => skia_safe::BlendMode::SrcOver,
        BlendMode::SrcIn => skia_safe::BlendMode::SrcIn,
        BlendMode::DstIn => skia_safe::BlendMode::DstIn,
        BlendMode::Multiply | BlendMode::Screen | BlendMode::Add => {
            return unsupported(role, &format!("blend mode {blend_mode:?}"));
        }
    })
}

#[cfg(feature = "skia-core")]
fn object_width(image: &skia_safe::Image) -> f32 {
    image.width() as f32
}

#[cfg(feature = "skia-core")]
fn object_height(image: &skia_safe::Image) -> f32 {
    image.height() as f32
}

#[cfg(feature = "skia-core")]
fn uses_implicit_full_source(uv: Rect, metadata: &BTreeMap<String, ParameterValue>) -> bool {
    let full = uv.x.to_bits() == 0.0f32.to_bits()
        && uv.y.to_bits() == 0.0f32.to_bits()
        && uv.width.to_bits() == 1.0f32.to_bits()
        && uv.height.to_bits() == 1.0f32.to_bits();
    full && !matches!(
        metadata.get("source_constraint"),
        Some(ParameterValue::Text(value)) if value == "fast" || value == "strict"
    )
}

#[cfg(feature = "skia-core")]
pub fn encode_profile_compositor_png(
    pixels: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, ProfileCompositorError> {
    use skia_safe::{images, AlphaType, ColorType, Data, EncodedImageFormat, ImageInfo};

    let expected = canvas_bytes(width, height)?;
    if pixels.len() != expected {
        return Err(ProfileCompositorError::InvalidCanvas { width, height });
    }
    let info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::RGBA8888,
        AlphaType::Premul,
        None,
    );
    let image = images::raster_from_data(&info, Data::new_copy(pixels), width as usize * 4)
        .ok_or_else(|| ProfileCompositorError::SkiaReference("create PNG image".into()))?;
    let context: Option<&mut skia_safe::gpu::DirectContext> = None;
    let encoded = image
        .encode(context, EncodedImageFormat::PNG, Some(100))
        .ok_or_else(|| ProfileCompositorError::SkiaReference("encode PNG".into()))?;
    Ok(encoded.as_bytes().to_vec())
}

fn collect_layers(
    scene: &ResolvedProfileScene,
) -> Result<BTreeMap<allium_renderer_core::StableId, &LayerSource>, ProfileCompositorError> {
    let mut layers = BTreeMap::new();
    for layer in &scene.layers {
        if layers.insert(layer.id, layer).is_some() {
            return Err(ProfileCompositorError::DuplicateLayer(format!(
                "{:?}",
                layer.id
            )));
        }
    }
    Ok(layers)
}

fn validate_image_command(
    role: &str,
    command_clip: Option<&allium_renderer_core::Quad>,
    bounds: Rect,
    clip_matrix: Matrix2d,
    matrix: Matrix2d,
    image_clip: Option<&allium_renderer_core::ImageClip>,
    alpha_mask: Option<&ResourceKey>,
    metadata: &BTreeMap<String, ParameterValue>,
) -> Result<(Option<AxisAlignedClip>, Option<ImageClipGeometry>), ProfileCompositorError> {
    let command_clip = command_clip
        .map(|clip| axis_aligned_command_clip(clip, clip_matrix))
        .transpose()
        .map_err(|()| ProfileCompositorError::UnsupportedFeature {
            role: role.into(),
            feature: "command clip".into(),
        })?
        .filter(|clip| !command_clip_contains_bounds(*clip, bounds, matrix));
    let image_clip = image_clip.map(|clip| match clip {
        allium_renderer_core::ImageClip::RoundedRect { radius } => {
            ImageClipGeometry::RoundedRect { radius: *radius }
        }
        allium_renderer_core::ImageClip::Ellipse => ImageClipGeometry::Ellipse,
    });
    if alpha_mask.is_some() {
        return unsupported(role, "alpha mask");
    }
    if matches!(metadata.get("sampling"), Some(ParameterValue::Text(value)) if value != "nearest") {
        return unsupported(role, "non-nearest sampling");
    }
    Ok((command_clip, image_clip))
}

fn axis_aligned_command_clip(
    clip: &allium_renderer_core::Quad,
    matrix: Matrix2d,
) -> Result<AxisAlignedClip, ()> {
    const EPSILON: f32 = 1.0e-4;

    let clip = clip.map(|point| {
        let (x, y) = transform_point(matrix, point[0], point[1]);
        [x, y]
    });
    if !clip.iter().flatten().all(|value| value.is_finite()) {
        return Err(());
    }
    let horizontal = |left: [f32; 2], right: [f32; 2]| {
        (left[1] - right[1]).abs() <= EPSILON && (left[0] - right[0]).abs() > EPSILON
    };
    let vertical = |top: [f32; 2], bottom: [f32; 2]| {
        (top[0] - bottom[0]).abs() <= EPSILON && (top[1] - bottom[1]).abs() > EPSILON
    };
    let axis_aligned_rectangle = (horizontal(clip[0], clip[1])
        && vertical(clip[1], clip[2])
        && horizontal(clip[2], clip[3])
        && vertical(clip[3], clip[0]))
        || (vertical(clip[0], clip[1])
            && horizontal(clip[1], clip[2])
            && vertical(clip[2], clip[3])
            && horizontal(clip[3], clip[0]));
    if !axis_aligned_rectangle {
        return Err(());
    }

    let clip_min_x = clip
        .iter()
        .map(|point| point[0])
        .fold(f32::INFINITY, f32::min);
    let clip_min_y = clip
        .iter()
        .map(|point| point[1])
        .fold(f32::INFINITY, f32::min);
    let clip_max_x = clip
        .iter()
        .map(|point| point[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let clip_max_y = clip
        .iter()
        .map(|point| point[1])
        .fold(f32::NEG_INFINITY, f32::max);
    Ok(AxisAlignedClip {
        min_x: clip_min_x,
        min_y: clip_min_y,
        max_x: clip_max_x,
        max_y: clip_max_y,
    })
}

fn command_clip_contains_bounds(clip: AxisAlignedClip, bounds: Rect, matrix: Matrix2d) -> bool {
    const EPSILON: f32 = 1.0e-4;
    let corners = [
        transform_point(matrix, bounds.x, bounds.y),
        transform_point(matrix, bounds.x + bounds.width, bounds.y),
        transform_point(matrix, bounds.x + bounds.width, bounds.y + bounds.height),
        transform_point(matrix, bounds.x, bounds.y + bounds.height),
    ];
    corners.iter().all(|&(x, y)| {
        x.is_finite()
            && y.is_finite()
            && x >= clip.min_x - EPSILON
            && x <= clip.max_x + EPSILON
            && y >= clip.min_y - EPSILON
            && y <= clip.max_y + EPSILON
    })
}

#[inline]
fn image_clip_contains(
    clip: Option<ImageClipGeometry>,
    bounds: Rect,
    local_x: f32,
    local_y: f32,
) -> bool {
    let Some(clip) = clip else {
        return true;
    };
    let half_width = bounds.width * 0.5;
    let half_height = bounds.height * 0.5;
    if half_width <= 0.0 || half_height <= 0.0 {
        return false;
    }
    let center_x = bounds.x + half_width;
    let center_y = bounds.y + half_height;
    match clip {
        ImageClipGeometry::Ellipse => {
            let normalized_x = (local_x - center_x) / half_width;
            let normalized_y = (local_y - center_y) / half_height;
            normalized_x.mul_add(normalized_x, normalized_y * normalized_y) <= 1.0
        }
        ImageClipGeometry::RoundedRect { radius } => {
            let radius_x = radius[0].abs().min(half_width);
            let radius_y = radius[1].abs().min(half_height);
            if radius_x == 0.0 || radius_y == 0.0 {
                return true;
            }
            let distance_x = (local_x - center_x).abs() - (half_width - radius_x);
            let distance_y = (local_y - center_y).abs() - (half_height - radius_y);
            if distance_x <= 0.0 || distance_y <= 0.0 {
                return true;
            }
            let normalized_x = distance_x / radius_x;
            let normalized_y = distance_y / radius_y;
            normalized_x.mul_add(normalized_x, normalized_y * normalized_y) <= 1.0
        }
    }
}

fn validate_composite_command(
    role: &str,
    opacity: f32,
    clip: Option<&allium_renderer_core::Quad>,
) -> Result<(), ProfileCompositorError> {
    if clip.is_some() {
        return unsupported(role, "composite clip");
    }
    if opacity.to_bits() != 1.0f32.to_bits() {
        return unsupported(role, "composite opacity");
    }
    Ok(())
}

fn unsupported<T>(role: &str, feature: &str) -> Result<T, ProfileCompositorError> {
    Err(ProfileCompositorError::UnsupportedFeature {
        role: role.into(),
        feature: feature.into(),
    })
}

fn render_semantic_text_command(
    command: &SemanticCommandSource,
    layer: &LayerSource,
    translate_y: f32,
    context: SemanticSdfContext<'_>,
    destination: &mut [u8],
    width: u32,
    height: u32,
    executor: ImageExecutor,
) -> Result<crate::sdf::tile::SdfExecutionStats, ProfileCompositorError> {
    render_semantic_text_commands(
        &[(command, layer, translate_y)],
        context,
        destination,
        width,
        height,
        executor,
    )
}

fn render_semantic_text_commands(
    commands: &[(&SemanticCommandSource, &LayerSource, f32)],
    context: SemanticSdfContext<'_>,
    destination: &mut [u8],
    width: u32,
    height: u32,
    executor: ImageExecutor,
) -> Result<crate::sdf::tile::SdfExecutionStats, ProfileCompositorError> {
    let mut draws = Vec::new();
    let capture_canvas = skia_safe::Canvas::new_null();
    for &(command, layer, translate_y) in commands {
        append_semantic_text_draws(
            command,
            layer,
            translate_y,
            context,
            &capture_canvas,
            &mut draws,
        )?;
    }
    if draws.is_empty() {
        return Ok(crate::sdf::tile::SdfExecutionStats::default());
    }
    let source = crate::sdf::tile::MixedSdfAtlasSource::new(context.text_atlases, None).map_err(
        |error| ProfileCompositorError::SemanticSdf {
            role: "semantic-text-batch".into(),
            reason: error.to_string(),
        },
    )?;
    let plan = crate::sdf::tile::SdfTilePlan::build(
        crate::sdf::tile::TileGrid::new(width, height),
        &draws,
        &source,
    )
    .map_err(|error| ProfileCompositorError::SemanticSdf {
        role: "semantic-text-batch".into(),
        reason: error.to_string(),
    })?;
    match executor {
        ImageExecutor::Scalar => plan.execute_scalar_f32_over(&source, destination),
        ImageExecutor::Simd => plan.execute_simd_f32_over(&source, destination),
    }
    .map_err(|error| ProfileCompositorError::SemanticSdf {
        role: "semantic-text-batch".into(),
        reason: error.to_string(),
    })
}

fn append_semantic_text_draws(
    command: &SemanticCommandSource,
    layer: &LayerSource,
    translate_y: f32,
    context: SemanticSdfContext<'_>,
    capture_canvas: &skia_safe::Canvas,
    draws: &mut Vec<crate::sdf::tile::SdfDrawCommand>,
) -> Result<(), ProfileCompositorError> {
    let SemanticCommandPayload::Text {
        source,
        font_role,
        size,
        color,
        outline_color,
        outline_size,
        line_spacing,
        alignment,
        max_width,
        ..
    } = &command.payload
    else {
        unreachable!("semantic text renderer received non-text command");
    };
    if *outline_size != 0.0 || outline_color.iter().any(|value| *value != 0.0) {
        return unsupported(&command.role, "semantic text outline");
    }
    let font_id = match font_role {
        FontRole::RegionFontId(font_id) => *font_id,
    };
    let family =
        context
            .md
            .resolve_font(font_id)
            .ok_or_else(|| ProfileCompositorError::SemanticSdf {
                role: command.role.clone(),
                reason: format!("missing region fontId={font_id}"),
            })?;
    if context
        .text_atlases
        .atlas_for_font_family(&family)
        .is_none()
    {
        return Err(ProfileCompositorError::SemanticSdf {
            role: command.role.clone(),
            reason: format!("font atlas not installed for {family}"),
        });
    }
    let value = match source {
        TextSource::Authored { value }
        | TextSource::ProfileField { value, .. }
        | TextSource::MasterData { value, .. }
        | TextSource::Localized { value, .. } => value,
    };
    if !size.is_finite() || *size <= 0.0 {
        return Err(ProfileCompositorError::InvalidGeometry {
            role: command.role.clone(),
        });
    }
    let placement =
        command
            .render_placement
            .unwrap_or(allium_renderer_core::TextRenderPlacementSource {
                anchor_x: 0.0,
                baseline: None,
            });
    let device_clip = command
        .clip
        .as_ref()
        .map(|clip| axis_aligned_command_clip(clip, layer.matrix))
        .transpose()
        .map_err(|()| ProfileCompositorError::UnsupportedFeature {
            role: command.role.clone(),
            feature: "non-axis text clip".into(),
        })?
        .map(|clip| crate::sdf::tile::SdfDeviceClip {
            min_x: clip.min_x,
            min_y: clip.min_y,
            max_x: clip.max_x,
            max_y: clip.max_y,
        });
    let mut content_matrix = command.matrix;
    content_matrix[5] += translate_y;
    let content_matrix = compose_matrix(layer.matrix, content_matrix);
    let mut capture_error = None;
    let mut observer = |result: Result<
        crate::text::ResolvedTextSdfGlyph,
        crate::text::TextSdfCaptureError,
    >| match result {
        Ok(glyph) => match glyph.to_sdf_command(context.text_atlases) {
            Ok(mut draw) => {
                draw.device_clip = device_clip;
                draws.push(draw);
            }
            Err(error) => capture_error = Some(error.to_string()),
        },
        Err(error) => capture_error = Some(format!("{error:?}")),
    };
    capture_canvas.save();
    capture_canvas.concat(&skia_safe::Matrix::from_affine(&content_matrix));
    let capture = crate::elements::generals::sdf_text::capture_general_sdf_text_from_lowered(
        capture_canvas,
        value,
        command.bounds.width,
        *color,
        *alignment,
        *size,
        *line_spacing,
        font_id,
        *max_width,
        crate::text::TextRenderPlacement {
            anchor_x: placement.anchor_x,
            baseline: placement.baseline,
        },
        context.md,
        context.text_atlases,
        &mut observer,
    );
    capture_canvas.restore();
    capture.map_err(|reason| ProfileCompositorError::SemanticSdf {
        role: command.role.clone(),
        reason,
    })?;
    if let Some(reason) = capture_error {
        return Err(ProfileCompositorError::SemanticSdf {
            role: command.role.clone(),
            reason,
        });
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn raster_semantic_shape_command(
    destination: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    command: &SemanticCommandSource,
    layer: &LayerSource,
    primitive: &ShapePrimitive,
    fill: [f32; 4],
    gradient: Option<&allium_renderer_core::LinearGradient>,
    stroke: [f32; 4],
    stroke_width: f32,
    translate_y: f32,
    executor: ImageExecutor,
) -> Result<RasterImageStats, ProfileCompositorError> {
    if command.blend_mode != BlendMode::SrcOver {
        return unsupported(
            &command.role,
            &format!("shape blend mode {:?}", command.blend_mode),
        );
    }
    let bounds = command.bounds;
    if !bounds.x.is_finite()
        || !bounds.y.is_finite()
        || !bounds.width.is_finite()
        || !bounds.height.is_finite()
        || bounds.width <= 0.0
        || bounds.height <= 0.0
        || !stroke_width.is_finite()
        || stroke_width < 0.0
    {
        return Err(ProfileCompositorError::InvalidGeometry {
            role: command.role.clone(),
        });
    }
    let mut translated_matrix = command.matrix;
    translated_matrix[5] += translate_y;
    let matrix = compose_matrix(layer.matrix, translated_matrix);
    let inverse = invert_matrix(matrix).ok_or_else(|| ProfileCompositorError::InvalidGeometry {
        role: command.role.clone(),
    })?;
    let command_clip = command
        .clip
        .as_ref()
        .map(|clip| axis_aligned_command_clip(clip, layer.matrix))
        .transpose()
        .map_err(|()| ProfileCompositorError::UnsupportedFeature {
            role: command.role.clone(),
            feature: "non-axis shape clip".into(),
        })?;
    let corners = [
        transform_point(matrix, bounds.x, bounds.y),
        transform_point(matrix, bounds.x + bounds.width, bounds.y),
        transform_point(matrix, bounds.x + bounds.width, bounds.y + bounds.height),
        transform_point(matrix, bounds.x, bounds.y + bounds.height),
    ];
    let min_x = corners
        .iter()
        .map(|value| value.0)
        .fold(f32::INFINITY, f32::min);
    let min_y = corners
        .iter()
        .map(|value| value.1)
        .fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|value| value.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = corners
        .iter()
        .map(|value| value.1)
        .fold(f32::NEG_INFINITY, f32::max);
    let clip_x0 = command_clip.map_or(0.0, |clip| (clip.min_x - 0.5).ceil());
    let clip_y0 = command_clip.map_or(0.0, |clip| (clip.min_y - 0.5).ceil());
    let clip_x1 = command_clip.map_or(canvas_width as f32, |clip| (clip.max_x - 0.5).ceil());
    let clip_y1 = command_clip.map_or(canvas_height as f32, |clip| (clip.max_y - 0.5).ceil());
    let x0 = min_x.floor().max(clip_x0).clamp(0.0, canvas_width as f32) as u32;
    let y0 = min_y.floor().max(clip_y0).clamp(0.0, canvas_height as f32) as u32;
    let x1 = max_x.ceil().min(clip_x1).clamp(0.0, canvas_width as f32) as u32;
    let y1 = max_y.ceil().min(clip_y1).clamp(0.0, canvas_height as f32) as u32;
    let samples = [(0.25f32, 0.25f32), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];
    let mut stats = RasterImageStats::default();
    for y in y0..y1 {
        #[cfg(target_arch = "x86_64")]
        if executor == ImageExecutor::Simd && std::arch::is_x86_feature_detected!("fma") {
            let mut x = x0;
            while x < x1 {
                let packet = (x1 - x).min(16);
                let destination_offset = ((y as usize * canvas_width as usize) + x as usize) * 4;
                let active = unsafe {
                    simd_x86::raster_semantic_shape_packet(
                        destination.as_mut_ptr().add(destination_offset),
                        x,
                        y,
                        packet,
                        &inverse,
                        bounds,
                        primitive,
                        fill,
                        gradient,
                        stroke,
                        stroke_width,
                    )
                };
                if active != 0 {
                    stats.fragments = stats
                        .fragments
                        .saturating_add(u64::from(active.count_ones()));
                    stats.simd_packets = stats.simd_packets.saturating_add(1);
                }
                x += packet;
            }
            continue;
        }
        for x in x0..x1 {
            let mut accumulated = [0.0f32; 4];
            let mut covered = false;
            for (offset_x, offset_y) in samples {
                let (local_x, local_y) =
                    transform_point(inverse, x as f32 + offset_x, y as f32 + offset_y);
                if !semantic_shape_contains(primitive, bounds, local_x, local_y, 0.0) {
                    continue;
                }
                covered = true;
                let use_stroke = stroke_width > 0.0
                    && !semantic_shape_contains(primitive, bounds, local_x, local_y, stroke_width);
                let color = if use_stroke {
                    stroke
                } else {
                    semantic_shape_fill(fill, gradient, bounds, local_x, local_y)
                };
                let alpha = color[3].clamp(0.0, 1.0);
                accumulated[0] += color[0].clamp(0.0, 1.0) * alpha * 0.25;
                accumulated[1] += color[1].clamp(0.0, 1.0) * alpha * 0.25;
                accumulated[2] += color[2].clamp(0.0, 1.0) * alpha * 0.25;
                accumulated[3] += alpha * 0.25;
            }
            if !covered {
                continue;
            }
            let source = accumulated.map(quantize);
            let offset = ((y as usize * canvas_width as usize) + x as usize) * 4;
            let pixel = destination.get_mut(offset..offset + 4).ok_or(
                ProfileCompositorError::InvalidCanvas {
                    width: canvas_width,
                    height: canvas_height,
                },
            )?;
            blend_pixel(pixel, source, BlendMode::SrcOver);
            stats.fragments = stats.fragments.saturating_add(1);
            stats.scalar_fragments = stats.scalar_fragments.saturating_add(1);
        }
    }
    Ok(stats)
}

fn semantic_shape_contains(
    primitive: &ShapePrimitive,
    bounds: Rect,
    x: f32,
    y: f32,
    inset: f32,
) -> bool {
    let left = bounds.x + inset;
    let top = bounds.y + inset;
    let right = bounds.x + bounds.width - inset;
    let bottom = bounds.y + bounds.height - inset;
    if left >= right || top >= bottom || x < left || x >= right || y < top || y >= bottom {
        return false;
    }
    match primitive {
        ShapePrimitive::Rect => true,
        ShapePrimitive::Ellipse => {
            let rx = (right - left) * 0.5;
            let ry = (bottom - top) * 0.5;
            let nx = (x - (left + right) * 0.5) / rx;
            let ny = (y - (top + bottom) * 0.5) / ry;
            nx.mul_add(nx, ny * ny) <= 1.0
        }
        ShapePrimitive::RoundedRect { radius } => {
            let rx = (radius[0] - inset).max(0.0).min((right - left) * 0.5);
            let ry = (radius[1] - inset).max(0.0).min((bottom - top) * 0.5);
            if rx == 0.0 || ry == 0.0 {
                return true;
            }
            let cx = x.clamp(left + rx, right - rx);
            let cy = y.clamp(top + ry, bottom - ry);
            let nx = (x - cx) / rx;
            let ny = (y - cy) / ry;
            nx.mul_add(nx, ny * ny) <= 1.0
        }
        ShapePrimitive::AssetMask { .. } => false,
    }
}

fn semantic_shape_fill(
    fill: [f32; 4],
    gradient: Option<&allium_renderer_core::LinearGradient>,
    bounds: Rect,
    x: f32,
    y: f32,
) -> [f32; 4] {
    let Some(gradient) = gradient else {
        return fill;
    };
    let u = (x - bounds.x) / bounds.width;
    let v = (y - bounds.y) / bounds.height;
    let dx = gradient.end[0] - gradient.start[0];
    let dy = gradient.end[1] - gradient.start[1];
    let denominator = dx.mul_add(dx, dy * dy);
    let t = if denominator <= f32::EPSILON {
        0.0
    } else {
        ((u - gradient.start[0]).mul_add(dx, (v - gradient.start[1]) * dy) / denominator)
            .clamp(0.0, 1.0)
    };
    std::array::from_fn(|channel| {
        (gradient.end_color[channel] - gradient.start_color[channel])
            .mul_add(t, gradient.start_color[channel])
    })
}

#[allow(clippy::too_many_arguments)]
fn raster_image_command(
    destination: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    source: MappedRenderObject<'_>,
    bounds: Rect,
    uv: Rect,
    tint: [f32; 4],
    matrix: Matrix2d,
    blend_mode: BlendMode,
    role: &str,
    command_clip: Option<AxisAlignedClip>,
    image_clip: Option<ImageClipGeometry>,
    executor: ImageExecutor,
) -> Result<RasterImageStats, ProfileCompositorError> {
    validate_geometry(bounds, uv, tint, matrix, role)?;
    validate_object(source, role)?;
    if bounds.width == 0.0 || bounds.height == 0.0 {
        return Ok(RasterImageStats::default());
    }
    if !matches!(
        blend_mode,
        BlendMode::SrcOver | BlendMode::SrcIn | BlendMode::DstIn
    ) {
        return unsupported(role, &format!("blend mode {blend_mode:?}"));
    }

    let inverse = invert_matrix(matrix)
        .ok_or_else(|| ProfileCompositorError::InvalidGeometry { role: role.into() })?;
    let corners = [
        transform_point(matrix, bounds.x, bounds.y),
        transform_point(matrix, bounds.x + bounds.width, bounds.y),
        transform_point(matrix, bounds.x + bounds.width, bounds.y + bounds.height),
        transform_point(matrix, bounds.x, bounds.y + bounds.height),
    ];
    let min_x = corners
        .iter()
        .map(|value| value.0)
        .fold(f32::INFINITY, f32::min);
    let min_y = corners
        .iter()
        .map(|value| value.1)
        .fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|value| value.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = corners
        .iter()
        .map(|value| value.1)
        .fold(f32::NEG_INFINITY, f32::max);
    let clip_x0 = command_clip.map_or(0.0, |clip| (clip.min_x - 0.5).ceil());
    let clip_y0 = command_clip.map_or(0.0, |clip| (clip.min_y - 0.5).ceil());
    let clip_x1 = command_clip.map_or(canvas_width as f32, |clip| (clip.max_x - 0.5).ceil());
    let clip_y1 = command_clip.map_or(canvas_height as f32, |clip| (clip.max_y - 0.5).ceil());
    let x0 = min_x.floor().max(clip_x0).max(0.0).min(canvas_width as f32) as u32;
    let y0 = min_y
        .floor()
        .max(clip_y0)
        .max(0.0)
        .min(canvas_height as f32) as u32;
    let x1 = max_x.ceil().min(clip_x1).max(0.0).min(canvas_width as f32) as u32;
    let y1 = max_y.ceil().min(clip_y1).max(0.0).min(canvas_height as f32) as u32;
    let mut stats = RasterImageStats::default();
    let packet_blend = executor == ImageExecutor::Simd
        && tint.iter().all(|value| value.to_bits() == 1.0f32.to_bits());
    #[cfg(target_arch = "x86_64")]
    let affine_packet_blend = packet_blend
        && source.pixels.len() <= i32::MAX as usize
        && source.entry.row_bytes <= i32::MAX as u32;

    if packet_blend
        && matrix[0] > 0.0
        && matrix[3] > 0.0
        && matrix[1].to_bits() == 0.0f32.to_bits()
        && matrix[2].to_bits() == 0.0f32.to_bits()
    {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            return Ok(raster_axis_aligned_image_command(
                destination,
                canvas_width,
                source,
                bounds,
                uv,
                inverse,
                blend_mode,
                image_clip,
                x0,
                y0,
                x1,
                y1,
            ));
        }
    }

    for y in y0..y1 {
        let py = y as f32 + 0.5;
        let mut x = x0;
        while x < x1 {
            let packet_end = x.saturating_add(16).min(x1);
            #[cfg(target_arch = "x86_64")]
            if affine_packet_blend {
                let destination_offset = ((y as usize * canvas_width as usize) + x as usize) * 4;
                let destination_packet =
                    destination.get_mut(destination_offset..).ok_or_else(|| {
                        ProfileCompositorError::InvalidCanvas {
                            width: canvas_width,
                            height: canvas_height,
                        }
                    })?;
                let active_mask = unsafe {
                    simd_x86::sample_affine_and_blend_rgba8_packet(
                        destination_packet.as_mut_ptr(),
                        source.pixels.as_ptr(),
                        source.entry.row_bytes as i32,
                        source.entry.width,
                        source.entry.height,
                        &inverse,
                        bounds,
                        uv,
                        x,
                        y,
                        packet_end - x,
                        blend_mode,
                        image_clip,
                    )
                };
                let active_fragments = u64::from(active_mask.count_ones());
                stats.fragments = stats.fragments.saturating_add(active_fragments);
                if active_mask != 0 {
                    stats.simd_packets = stats.simd_packets.saturating_add(1);
                }
                x = packet_end;
                continue;
            }
            let mut source_pixels = [0u32; 16];
            let mut active_mask = 0u16;
            for packet_x in x..packet_end {
                let px = packet_x as f32 + 0.5;
                let (local_x, local_y) = transform_point(inverse, px, py);
                let u = (local_x - bounds.x) / bounds.width;
                let v = (local_y - bounds.y) / bounds.height;
                if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                    continue;
                }
                if !image_clip_contains(image_clip, bounds, local_x, local_y) {
                    continue;
                }
                let source_x = ((uv.x + u * uv.width) * source.entry.width as f32).floor();
                let source_y = ((uv.y + v * uv.height) * source.entry.height as f32).floor();
                let sx = source_x
                    .max(0.0)
                    .min(source.entry.width.saturating_sub(1) as f32)
                    as u32;
                let sy = source_y
                    .max(0.0)
                    .min(source.entry.height.saturating_sub(1) as f32)
                    as u32;
                let source_pixel = object_pixel(source, sx, sy)
                    .ok_or_else(|| ProfileCompositorError::InvalidObject(role.into()))?;
                let lane = usize::try_from(packet_x - x).unwrap_or_default();
                source_pixels[lane] = u32::from_le_bytes(source_pixel);
                active_mask |= 1u16 << lane;
            }

            let active_fragments = u64::from(active_mask.count_ones());
            if active_mask != 0 && packet_blend {
                let destination_offset = ((y as usize * canvas_width as usize) + x as usize) * 4;
                let destination_packet =
                    destination.get_mut(destination_offset..).ok_or_else(|| {
                        ProfileCompositorError::InvalidCanvas {
                            width: canvas_width,
                            height: canvas_height,
                        }
                    })?;
                #[cfg(target_arch = "x86_64")]
                unsafe {
                    simd_x86::blend_rgba8_packet(
                        destination_packet.as_mut_ptr(),
                        source_pixels.as_ptr(),
                        active_mask,
                        blend_mode,
                    );
                }
                stats.simd_packets = stats.simd_packets.saturating_add(1);
            } else {
                for lane in 0..usize::try_from(packet_end - x).unwrap_or_default() {
                    if active_mask & (1u16 << lane) == 0 {
                        continue;
                    }
                    let packet_x = x + lane as u32;
                    let destination_offset =
                        ((y as usize * canvas_width as usize) + packet_x as usize) * 4;
                    let destination_pixel = destination
                        .get_mut(destination_offset..destination_offset + 4)
                        .ok_or_else(|| ProfileCompositorError::InvalidCanvas {
                            width: canvas_width,
                            height: canvas_height,
                        })?;
                    let source_pixel = apply_tint(source_pixels[lane].to_le_bytes(), tint);
                    blend_pixel(destination_pixel, source_pixel, blend_mode);
                }
                stats.scalar_fragments = stats.scalar_fragments.saturating_add(active_fragments);
            }
            stats.fragments = stats.fragments.saturating_add(active_fragments);
            x = packet_end;
        }
    }
    Ok(stats)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
#[allow(clippy::too_many_arguments)]
unsafe fn raster_axis_aligned_image_command(
    destination: &mut [u8],
    canvas_width: u32,
    source: MappedRenderObject<'_>,
    bounds: Rect,
    uv: Rect,
    inverse: Matrix2d,
    blend_mode: BlendMode,
    image_clip: Option<ImageClipGeometry>,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
) -> RasterImageStats {
    const INACTIVE_SOURCE_COLUMN: u32 = u32::MAX;

    let source_width = source.entry.width as f32;
    let source_height = source.entry.height as f32;
    let maximum_source_x = source.entry.width - 1;
    let maximum_source_y = source.entry.height - 1;
    let source_row_bytes = source.entry.row_bytes as usize;
    let canvas_row_bytes = canvas_width as usize * 4;
    let mut source_columns = Vec::with_capacity((x1 - x0) as usize);
    let mut local_columns = Vec::with_capacity((x1 - x0) as usize);
    for x in x0..x1 {
        let px = x as f32 + 0.5;
        let local_x = inverse[0].mul_add(px, inverse[4]);
        local_columns.push(local_x);
        let u = (local_x - bounds.x) / bounds.width;
        if !(0.0..1.0).contains(&u) {
            source_columns.push(INACTIVE_SOURCE_COLUMN);
            continue;
        }
        let source_x = ((uv.x + u * uv.width) * source_width).floor();
        source_columns.push(source_x.max(0.0).min(maximum_source_x as f32) as u32);
    }

    struct AxisPacket {
        destination_x: u32,
        column_offset: usize,
        active_mask: u16,
        first_source_x: u32,
        contiguous: bool,
    }
    let mut packets = Vec::with_capacity((x1 - x0).div_ceil(16) as usize);
    let mut packet_x = x0;
    while packet_x < x1 {
        let packet_end = packet_x.saturating_add(16).min(x1);
        let packet_length = (packet_end - packet_x) as usize;
        let column_offset = (packet_x - x0) as usize;
        let columns = &source_columns[column_offset..column_offset + packet_length];
        let mut active_mask = 0u16;
        let mut contiguous = true;
        let mut first_source_x = 0u32;
        let mut active_count = 0usize;
        for (lane, &source_x) in columns.iter().enumerate() {
            if source_x == INACTIVE_SOURCE_COLUMN {
                contiguous = false;
                continue;
            }
            if active_count == 0 {
                first_source_x = source_x;
                contiguous &= lane == 0;
            } else {
                contiguous &= source_x == first_source_x + active_count as u32;
            }
            active_mask |= 1u16 << lane;
            active_count += 1;
        }
        let expected_mask = if active_count == 16 {
            u16::MAX
        } else {
            (1u16 << active_count) - 1
        };
        packets.push(AxisPacket {
            destination_x: packet_x,
            column_offset,
            active_mask,
            first_source_x,
            contiguous: contiguous && active_mask == expected_mask,
        });
        packet_x = packet_end;
    }

    let destination_base = destination.as_mut_ptr();
    let source_base = source.pixels.as_ptr();
    let mut stats = RasterImageStats::default();
    for y in y0..y1 {
        let py = y as f32 + 0.5;
        let (_, local_y) = transform_point(inverse, 0.5, py);
        let v = (local_y - bounds.y) / bounds.height;
        if !(0.0..1.0).contains(&v) {
            continue;
        }
        let source_y = ((uv.y + v * uv.height) * source_height).floor();
        let source_y = source_y.max(0.0).min(maximum_source_y as f32) as usize;
        let source_row = source_base.add(source_y * source_row_bytes);
        let destination_row = destination_base.add(y as usize * canvas_row_bytes);

        for packet in &packets {
            let active_mask = if let Some(clip) = image_clip {
                simd_x86::axis_aligned_image_clip_mask(
                    local_columns.as_ptr().add(packet.column_offset),
                    local_y,
                    bounds,
                    clip,
                    packet.active_mask,
                )
            } else {
                packet.active_mask
            };
            if active_mask != 0 {
                let destination_packet = destination_row.add(packet.destination_x as usize * 4);
                if packet.contiguous {
                    let source_packet = source_row
                        .add(packet.first_source_x as usize * 4)
                        .cast::<u32>();
                    simd_x86::blend_rgba8_packet(
                        destination_packet,
                        source_packet,
                        active_mask,
                        blend_mode,
                    );
                } else {
                    simd_x86::gather_and_blend_rgba8_packet(
                        destination_packet,
                        source_row,
                        source_columns.as_ptr().add(packet.column_offset),
                        active_mask,
                        blend_mode,
                    );
                }
                stats.fragments = stats
                    .fragments
                    .saturating_add(u64::from(active_mask.count_ones()));
                stats.simd_packets = stats.simd_packets.saturating_add(1);
            }
        }
    }
    stats
}

fn validate_geometry(
    bounds: Rect,
    uv: Rect,
    tint: [f32; 4],
    matrix: Matrix2d,
    role: &str,
) -> Result<(), ProfileCompositorError> {
    let finite = [
        bounds.x,
        bounds.y,
        bounds.width,
        bounds.height,
        uv.x,
        uv.y,
        uv.width,
        uv.height,
    ]
    .into_iter()
    .chain(tint)
    .chain(matrix)
    .all(f32::is_finite);
    if !finite || bounds.width < 0.0 || bounds.height < 0.0 || uv.width <= 0.0 || uv.height <= 0.0 {
        return Err(ProfileCompositorError::InvalidGeometry { role: role.into() });
    }
    Ok(())
}

fn validate_object(
    object: MappedRenderObject<'_>,
    role: &str,
) -> Result<(), ProfileCompositorError> {
    let expected = u64::from(object.entry.row_bytes).checked_mul(u64::from(object.entry.height));
    if object.entry.width == 0
        || object.entry.height == 0
        || object.entry.row_bytes < object.entry.width.saturating_mul(4)
        || expected.and_then(|value| usize::try_from(value).ok()) != Some(object.pixels.len())
    {
        return Err(ProfileCompositorError::InvalidObject(role.into()));
    }
    Ok(())
}

#[cfg(feature = "skia-core")]
fn load_source_fallback_object(
    assets: Option<&AssetStore>,
    resource: &ResourceKey,
) -> Result<Option<OwnedRenderObject>, ProfileCompositorError> {
    let Some(assets) = assets else {
        return Ok(None);
    };
    if resource.namespace != "assets" {
        return Ok(None);
    }
    let Some((width, height, pixels)) = assets.get_premultiplied_rgba(&resource.key) else {
        return Ok(None);
    };
    let row_bytes = usize::try_from(width)
        .ok()
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| {
            ProfileCompositorError::SkiaReference(format!("fallback row overflow {}", resource.key))
        })?;
    if pixels.len() != row_bytes.saturating_mul(height as usize) {
        return Err(ProfileCompositorError::SkiaReference(format!(
            "fallback pixel size mismatch {}",
            resource.key
        )));
    }
    let object_key = render_object_key_for_resource(resource);
    let pixel_sha256 = hex::encode(Sha256::digest(&pixels));
    Ok(Some(OwnedRenderObject {
        entry: RenderObjectEntry {
            key: object_key,
            kind: RenderObjectKind::Texture,
            source_sha256: pixel_sha256.clone(),
            page: 0,
            offset: 0,
            length: pixels.len() as u64,
            width,
            height,
            row_bytes: row_bytes as u32,
            pixel_sha256,
        },
        pixels,
    }))
}

fn object_pixel(object: MappedRenderObject<'_>, x: u32, y: u32) -> Option<[u8; 4]> {
    let offset = usize::try_from(y)
        .ok()?
        .checked_mul(usize::try_from(object.entry.row_bytes).ok()?)?
        .checked_add(usize::try_from(x).ok()?.checked_mul(4)?)?;
    let pixel = object.pixels.get(offset..offset.checked_add(4)?)?;
    Some([pixel[0], pixel[1], pixel[2], pixel[3]])
}

fn apply_tint(source: [u8; 4], tint: [f32; 4]) -> [u8; 4] {
    if tint.iter().all(|value| value.to_bits() == 1.0f32.to_bits()) {
        return source;
    }
    let tint_alpha = tint[3].clamp(0.0, 1.0);
    [
        quantize(f32::from(source[0]) / 255.0 * tint[0].clamp(0.0, 1.0) * tint_alpha),
        quantize(f32::from(source[1]) / 255.0 * tint[1].clamp(0.0, 1.0) * tint_alpha),
        quantize(f32::from(source[2]) / 255.0 * tint[2].clamp(0.0, 1.0) * tint_alpha),
        quantize(f32::from(source[3]) / 255.0 * tint_alpha),
    ]
}

fn blend_pixel(destination: &mut [u8], source: [u8; 4], blend_mode: BlendMode) {
    let output: [u8; 4] = match blend_mode {
        BlendMode::SrcOver => {
            let inverse_alpha = u8::MAX - source[3];
            std::array::from_fn(|channel| {
                u16::from(source[channel])
                    .saturating_add(u16::from(mul_div_255_round(
                        destination[channel],
                        inverse_alpha,
                    )))
                    .min(u16::from(u8::MAX)) as u8
            })
        }
        BlendMode::SrcIn => {
            std::array::from_fn(|channel| mul_div_255_round(source[channel], destination[3]))
        }
        BlendMode::DstIn => {
            std::array::from_fn(|channel| mul_div_255_round(destination[channel], source[3]))
        }
        BlendMode::Multiply | BlendMode::Screen | BlendMode::Add => return,
    };
    destination.copy_from_slice(&output);
}

#[inline]
fn mul_div_255_round(left: u8, right: u8) -> u8 {
    let product = u32::from(left) * u32::from(right);
    ((product + 127) / 255) as u8
}

fn composition_bounds(
    bounds: Rect,
    matrix: Matrix2d,
    canvas_width: u32,
    canvas_height: u32,
) -> CompositionBounds {
    if bounds.width <= 0.0 || bounds.height <= 0.0 {
        return CompositionBounds {
            x0: 0,
            y0: 0,
            x1: canvas_width,
            y1: canvas_height,
        };
    }
    let corners = [
        transform_point(matrix, bounds.x, bounds.y),
        transform_point(matrix, bounds.x + bounds.width, bounds.y),
        transform_point(matrix, bounds.x + bounds.width, bounds.y + bounds.height),
        transform_point(matrix, bounds.x, bounds.y + bounds.height),
    ];
    let min_x = corners
        .iter()
        .map(|value| value.0)
        .fold(f32::INFINITY, f32::min);
    let min_y = corners
        .iter()
        .map(|value| value.1)
        .fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|value| value.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = corners
        .iter()
        .map(|value| value.1)
        .fold(f32::NEG_INFINITY, f32::max);
    CompositionBounds {
        x0: min_x.floor().clamp(0.0, canvas_width as f32) as u32,
        y0: min_y.floor().clamp(0.0, canvas_height as f32) as u32,
        x1: max_x.ceil().clamp(0.0, canvas_width as f32) as u32,
        y1: max_y.ceil().clamp(0.0, canvas_height as f32) as u32,
    }
}

fn clear_composition_bounds(pixels: &mut [u8], canvas_width: u32, bounds: CompositionBounds) {
    let row_bytes = canvas_width as usize * 4;
    let x0 = bounds.x0 as usize * 4;
    let x1 = bounds.x1 as usize * 4;
    for y in bounds.y0..bounds.y1 {
        let row = y as usize * row_bytes;
        pixels[row + x0..row + x1].fill(0);
    }
}

fn composite_isolation(
    parent: &mut [u8],
    child: &[u8],
    canvas_width: u32,
    bounds: CompositionBounds,
    executor: ImageExecutor,
) {
    let row_bytes = canvas_width as usize * 4;
    for y in bounds.y0..bounds.y1 {
        let row = y as usize * row_bytes;
        let mut x = bounds.x0;
        while x < bounds.x1 {
            let packet = (bounds.x1 - x).min(16);
            let offset = row + x as usize * 4;
            #[cfg(target_arch = "x86_64")]
            if executor == ImageExecutor::Simd {
                let active = if packet == 16 {
                    u16::MAX
                } else {
                    (1u16 << packet) - 1
                };
                unsafe {
                    simd_x86::blend_rgba8_packet(
                        parent.as_mut_ptr().add(offset),
                        child.as_ptr().add(offset).cast(),
                        active,
                        BlendMode::SrcOver,
                    );
                }
                x += packet;
                continue;
            }
            for lane in 0..packet {
                let pixel_offset = offset + lane as usize * 4;
                let source = &child[pixel_offset..pixel_offset + 4];
                if source[3] != 0 {
                    blend_pixel(
                        &mut parent[pixel_offset..pixel_offset + 4],
                        [source[0], source[1], source[2], source[3]],
                        BlendMode::SrcOver,
                    );
                }
            }
            x += packet;
        }
    }
}

fn compose_matrix(parent: Matrix2d, child: Matrix2d) -> Matrix2d {
    [
        parent[0] * child[0] + parent[2] * child[1],
        parent[1] * child[0] + parent[3] * child[1],
        parent[0] * child[2] + parent[2] * child[3],
        parent[1] * child[2] + parent[3] * child[3],
        parent[0] * child[4] + parent[2] * child[5] + parent[4],
        parent[1] * child[4] + parent[3] * child[5] + parent[5],
    ]
}

fn invert_matrix(matrix: Matrix2d) -> Option<Matrix2d> {
    let determinant = matrix[0] * matrix[3] - matrix[1] * matrix[2];
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return None;
    }
    Some([
        matrix[3] / determinant,
        -matrix[1] / determinant,
        -matrix[2] / determinant,
        matrix[0] / determinant,
        (matrix[2] * matrix[5] - matrix[3] * matrix[4]) / determinant,
        (matrix[1] * matrix[4] - matrix[0] * matrix[5]) / determinant,
    ])
}

fn transform_point(matrix: Matrix2d, x: f32, y: f32) -> (f32, f32) {
    (
        matrix[0].mul_add(x, matrix[2].mul_add(y, matrix[4])),
        matrix[1].mul_add(x, matrix[3].mul_add(y, matrix[5])),
    )
}

fn canvas_bytes(width: u32, height: u32) -> Result<usize, ProfileCompositorError> {
    if width == 0 || height == 0 {
        return Err(ProfileCompositorError::InvalidCanvas { width, height });
    }
    usize::try_from(width)
        .ok()
        .and_then(|value| value.checked_mul(height as usize))
        .and_then(|value| value.checked_mul(4))
        .ok_or(ProfileCompositorError::InvalidCanvas { width, height })
}

fn quantize(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use allium_renderer_core::profile_scene::{
        ComponentControlSource, ComponentControlState, ResolvedProfileScene,
    };
    use allium_renderer_core::{
        AuthoredElementKind, CommandControlBinding, CompositeOperation, FontRole, LayerKind,
        LayerSource, LinearGradient, Rect, ResourceKey, SemanticCommandPayload,
        SemanticCommandSource, ShapePrimitive, StableId, TextSource,
    };
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::render_object::{
        MappedRenderObjectStore, RenderObjectKind, RenderObjectStoreWriter, RenderObjectWrite,
    };

    fn packet_simd_available() -> bool {
        #[cfg(target_arch = "x86_64")]
        {
            std::arch::is_x86_feature_detected!("avx512f")
                && std::arch::is_x86_feature_detected!("avx512bw")
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            false
        }
    }

    fn test_layer(id: StableId, matrix: Matrix2d) -> LayerSource {
        LayerSource {
            id,
            parent_id: None,
            kind: LayerKind::Image,
            authored_kind: AuthoredElementKind::Other,
            authored_index: 0,
            game_layer: 0,
            z: 0,
            authored_visible: true,
            source_content: String::new(),
            resolved_parameters: BTreeMap::new(),
            bounds: Rect::default(),
            quad: [[0.0; 2]; 4],
            matrix,
            hit_geometry: [[0.0; 2]; 4],
            line_indent: None,
        }
    }

    fn image_command(
        id: &str,
        layer_id: StableId,
        resource_key: &str,
        bounds: Rect,
        uv: Rect,
        blend_mode: BlendMode,
    ) -> SemanticCommandSource {
        let mut command = SemanticCommandSource::image(
            StableId::derive("command", id.as_bytes()),
            layer_id,
            id,
            ResourceKey {
                namespace: "assets".into(),
                key: resource_key.into(),
            },
            bounds,
        );
        command.blend_mode = blend_mode;
        if let SemanticCommandPayload::Image { uv: target, .. } = &mut command.payload {
            *target = uv;
        }
        command
    }

    fn composite_command(
        id: &str,
        layer_id: StableId,
        operation: CompositeOperation,
    ) -> SemanticCommandSource {
        let mut command = SemanticCommandSource::composite(
            StableId::derive("command", id.as_bytes()),
            layer_id,
            id,
            Rect::default(),
        );
        if let SemanticCommandPayload::Composite {
            operation: target, ..
        } = &mut command.payload
        {
            *target = operation;
        }
        command
    }

    fn shape_command(
        id: &str,
        layer_id: StableId,
        bounds: Rect,
        primitive: ShapePrimitive,
        fill: [f32; 4],
        gradient: Option<LinearGradient>,
        stroke: [f32; 4],
        stroke_width: f32,
    ) -> SemanticCommandSource {
        let mut command = SemanticCommandSource::shape(
            StableId::derive("command", id.as_bytes()),
            layer_id,
            id,
            bounds,
            primitive,
        );
        if let SemanticCommandPayload::Shape {
            fill: target_fill,
            gradient: target_gradient,
            stroke: target_stroke,
            stroke_width: target_stroke_width,
            ..
        } = &mut command.payload
        {
            *target_fill = fill;
            *target_gradient = gradient;
            *target_stroke = stroke;
            *target_stroke_width = stroke_width;
        }
        command
    }

    fn scene(layer: LayerSource, commands: Vec<SemanticCommandSource>) -> ResolvedProfileScene {
        ResolvedProfileScene {
            layers: vec![layer],
            commands,
            interaction_regions: Vec::new(),
            controls: Vec::new(),
        }
    }

    #[test]
    fn native_compositor_filters_inactive_tab_commands() {
        let layer_id = StableId(10);
        let control_id = StableId(20);
        let mut active = image_command(
            "active",
            layer_id,
            "active",
            Rect::default(),
            Rect::default(),
            BlendMode::SrcOver,
        );
        active
            .control_bindings
            .push(CommandControlBinding::TabOption {
                control_id,
                value: "character_rank".into(),
            });
        let mut inactive = active.clone();
        inactive.role = "inactive".into();
        inactive.control_bindings = vec![CommandControlBinding::TabOption {
            control_id,
            value: "challenge_live_rank".into(),
        }];
        let mut scene = scene(test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]), vec![]);
        scene.controls.push(ComponentControlSource {
            id: control_id,
            layer_id,
            role: "rank-mode".into(),
            state: ComponentControlState::Tabs {
                options: vec!["character_rank".into(), "challenge_live_rank".into()],
                active: "character_rank".into(),
            },
        });

        assert!(
            evaluate_command_control_state(&scene, &active)
                .expect("active state")
                .visible
        );
        assert!(
            !evaluate_command_control_state(&scene, &inactive)
                .expect("inactive state")
                .visible
        );
    }

    #[test]
    fn native_compositor_resolves_scroll_content_translation() {
        let layer_id = StableId(11);
        let control_id = StableId(21);
        let mut command = image_command(
            "scroll-content",
            layer_id,
            "scroll-content",
            Rect {
                width: 10.0,
                height: 10.0,
                ..Rect::default()
            },
            Rect::default(),
            BlendMode::SrcOver,
        );
        command
            .control_bindings
            .push(CommandControlBinding::ScrollContent { control_id });
        let mut scene = scene(test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]), vec![]);
        scene.controls.push(ComponentControlSource {
            id: control_id,
            layer_id,
            role: "scroll".into(),
            state: ComponentControlState::Scroll {
                offset: 10.0,
                min: 0.0,
                max: 20.0,
                viewport_extent: 10.0,
                content_extent: 30.0,
                step: 5.0,
            },
        });

        let state = evaluate_command_control_state(&scene, &command).expect("scroll state");
        assert!(state.visible);
        assert_eq!(state.translate_y, -10.0);
    }

    #[test]
    fn missing_mmap_texture_uses_asset_store_without_leaving_software_compositor() {
        let layer_id = StableId(301);
        let scene = scene(
            test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            vec![image_command(
                "cos-fallback",
                layer_id,
                "missing/source",
                Rect {
                    width: 2.0,
                    height: 2.0,
                    ..Rect::default()
                },
                Rect {
                    width: 1.0,
                    height: 1.0,
                    ..Rect::default()
                },
                BlendMode::SrcOver,
            )],
        );
        let (_temp, store) = store(vec![("dummy", 1, 1, vec![0; 4])]);
        let assets = AssetStore::new(8);
        let mut surface = skia_safe::surfaces::raster_n32_premul((2, 2)).unwrap();
        surface.canvas().clear(skia_safe::Color::RED);
        let encoded = surface
            .image_snapshot()
            .encode(None, skia_safe::EncodedImageFormat::PNG, Some(100))
            .unwrap();
        assets.put("missing/source".into(), encoded.as_bytes().to_vec());
        let mut pixels = vec![0; 16];
        let stats = render_image_commands_into(
            &scene,
            &store,
            2,
            2,
            ImageExecutor::Scalar,
            &mut pixels,
            None,
            None,
            None,
            Some(&assets),
        )
        .unwrap();

        assert_eq!(pixels, [255, 0, 0, 255].repeat(4));
        assert_eq!(stats.source_fallback_object_count, 1);
        assert_eq!(stats.source_fallback_bytes, 16);
        assert_eq!(stats.image_command_count, 1);
    }

    #[test]
    fn live_master_progress_uses_legacy_simple_text_pixels_without_sdf() {
        use skia_safe::{
            surfaces, AlphaType, Color4f, ColorType, Font, FontMgr, FontStyle, ImageInfo, Paint,
            PaintStyle, Point,
        };

        let layer_id = StableId(302);
        let mut command = SemanticCommandSource::profile_text(
            StableId(303),
            layer_id,
            "honor-4242-progress",
            "userHonorMissions.4242",
            "73",
            FontRole::RegionFontId(1),
        );
        command.matrix = [1.0, 0.0, 0.0, 1.0, 80.0, 20.0];
        if let SemanticCommandPayload::Text {
            source,
            size,
            color,
            alignment,
            ..
        } = &mut command.payload
        {
            *source = TextSource::ProfileField {
                field: "userHonorMissions.4242".into(),
                value: "73".into(),
            };
            *size = 20.0;
            *color = [1.0; 4];
            *alignment = 2;
        }
        let scene = scene(
            test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            vec![command],
        );
        let (_temp, store) = store(vec![("dummy", 1, 1, vec![0; 4])]);
        let mut actual = vec![0; 200 * 80 * 4];
        let stats = render_image_commands_into(
            &scene,
            &store,
            200,
            80,
            ImageExecutor::Scalar,
            &mut actual,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let mut expected = vec![0; actual.len()];
        let info = ImageInfo::new((200, 80), ColorType::RGBA8888, AlphaType::Premul, None);
        let mut surface =
            surfaces::wrap_pixels(&info, expected.as_mut_slice(), Some(200 * 4), None).unwrap();
        let font_mgr = FontMgr::default();
        let typeface = font_mgr
            .match_family_style("Noto Sans CJK SC", FontStyle::bold())
            .or_else(|| font_mgr.legacy_make_typeface(None, FontStyle::bold()))
            .unwrap();
        let font = Font::new(typeface, Some(20.0));
        let text_width = font.measure_str("73", None).0;
        let mut paint = Paint::default();
        paint.set_style(PaintStyle::Fill);
        paint.set_color4f(Color4f::new(1.0, 1.0, 1.0, 1.0), None);
        paint.set_anti_alias(true);
        surface.canvas().draw_str(
            "73",
            Point::new(80.0 - text_width / 2.0, 30.0),
            &font,
            &paint,
        );

        assert_eq!(actual, expected);
        assert_eq!(stats.text_command_count, 1);
        assert_eq!(stats.skipped_text_command_count, 0);
    }

    fn store(
        objects: Vec<(&str, u32, u32, Vec<u8>)>,
    ) -> (tempfile::TempDir, MappedRenderObjectStore) {
        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("store");
        let mut writer = RenderObjectStoreWriter::create(&output, "profile-compositor-test", 4096)
            .expect("writer");
        for (key, width, height, pixels) in objects {
            let source_sha = hex::encode(Sha256::digest(key.as_bytes()));
            writer
                .add(RenderObjectWrite {
                    key,
                    kind: RenderObjectKind::Texture,
                    source_sha256: &source_sha,
                    width,
                    height,
                    row_bytes: width * 4,
                    pixels: &pixels,
                })
                .expect("add object");
        }
        let manifest = writer.finish().expect("manifest");
        let mapped = MappedRenderObjectStore::open(manifest).expect("mapped store");
        (temp, mapped)
    }

    #[test]
    fn integer_premultiplied_multiply_matches_float_quantization() {
        for left in u8::MIN..=u8::MAX {
            for right in u8::MIN..=u8::MAX {
                let reference = quantize(f32::from(left) / 255.0 * f32::from(right) / 255.0);
                assert_eq!(
                    mul_div_255_round(left, right),
                    reference,
                    "{left} * {right}"
                );
                let product = u32::from(left) * u32::from(right);
                let biased = product + 128;
                let packet_formula = ((biased + (biased >> 8)) >> 8) as u8;
                assert_eq!(packet_formula, reference, "packet {left} * {right}");
            }
        }
    }

    #[test]
    fn semantic_shape_simd_matches_scalar_for_gradient_stroke_and_affine_geometry() {
        let layer_id = StableId(91);
        let bounds = Rect {
            x: 3.25,
            y: 2.5,
            width: 31.5,
            height: 19.25,
        };
        let mut rounded = shape_command(
            "rounded-gradient",
            layer_id,
            bounds,
            ShapePrimitive::RoundedRect { radius: [6.5, 4.0] },
            [0.2, 0.7, 0.3, 0.8],
            Some(LinearGradient {
                start: [0.1, 0.2],
                end: [0.9, 0.8],
                start_color: [0.9, 0.1, 0.4, 0.7],
                end_color: [0.1, 0.8, 0.6, 0.9],
            }),
            [0.8, 0.2, 0.1, 0.65],
            2.25,
        );
        rounded.matrix = [0.97, 0.13, -0.09, 1.02, 4.0, 3.0];
        let mut ellipse = shape_command(
            "ellipse-solid",
            layer_id,
            Rect {
                x: 20.0,
                y: 13.0,
                width: 27.0,
                height: 21.0,
            },
            ShapePrimitive::Ellipse,
            [0.15, 0.35, 0.95, 0.72],
            None,
            [0.9, 0.8, 0.1, 0.55],
            1.5,
        );
        ellipse.matrix = [1.0, -0.07, 0.11, 0.96, -1.0, 2.0];
        let scene = scene(
            test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            vec![rounded, ellipse],
        );
        let (_temp, store) = store(vec![("dummy", 1, 1, vec![0; 4])]);
        let background = [17, 31, 47, 191].repeat(73 * 47);
        let mut scalar = background.clone();
        render_image_commands_into(
            &scene,
            &store,
            73,
            47,
            ImageExecutor::Scalar,
            &mut scalar,
            None,
            None,
            None,
            None,
        )
        .expect("scalar semantic Shape oracle");
        let mut simd = background;

        if packet_simd_available() && std::arch::is_x86_feature_detected!("fma") {
            let stats = render_image_commands_into(
                &scene,
                &store,
                73,
                47,
                ImageExecutor::Simd,
                &mut simd,
                None,
                None,
                None,
                None,
            )
            .expect("SIMD semantic Shape candidate");
            assert_eq!(simd, scalar);
            assert!(stats.simd_packet_count > 0);
            assert_eq!(stats.scalar_fragment_count, 0);
        } else {
            assert_eq!(simd, [17, 31, 47, 191].repeat(73 * 47));
        }
    }

    #[test]
    fn partial_uv_and_layer_translation_sample_nearest_texels() {
        let (_temp, store) = store(vec![(
            "texture:assets/strip",
            4,
            1,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
        )]);
        let layer_id = StableId::derive("layer", b"uv");
        let command = image_command(
            "right-half",
            layer_id,
            "strip",
            Rect {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 1.0,
            },
            Rect {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
            BlendMode::SrcOver,
        );
        let output = render_image_scene_scalar(
            &scene(
                test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 1.0, 1.0]),
                vec![command],
            ),
            &store,
            4,
            3,
        )
        .expect("render");
        assert_eq!(
            &output.pixels[20..28],
            &[0, 0, 255, 255, 255, 255, 255, 255]
        );
        assert_eq!(output.stats.sampled_fragment_count, 2);
        #[cfg(feature = "skia-core")]
        {
            let reference = render_image_scene_skia_reference(
                &scene(
                    test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 1.0, 1.0]),
                    vec![image_command(
                        "right-half-reference",
                        layer_id,
                        "strip",
                        Rect {
                            x: 0.0,
                            y: 0.0,
                            width: 2.0,
                            height: 1.0,
                        },
                        Rect {
                            x: 0.5,
                            y: 0.0,
                            width: 0.5,
                            height: 1.0,
                        },
                        BlendMode::SrcOver,
                    )],
                ),
                &store,
                4,
                3,
            )
            .expect("Skia reference");
            assert_eq!(output.pixels, reference.pixels);
        }
    }

    #[test]
    fn isolation_dst_in_masks_only_the_isolated_group() {
        let (_temp, store) = store(vec![
            (
                "texture:assets/background",
                2,
                1,
                vec![255, 0, 0, 255, 255, 0, 0, 255],
            ),
            (
                "texture:assets/mask",
                2,
                1,
                vec![255, 255, 255, 255, 0, 0, 0, 0],
            ),
        ]);
        let layer_id = StableId::derive("layer", b"mask");
        let bounds = Rect {
            x: 0.0,
            y: 0.0,
            width: 2.0,
            height: 1.0,
        };
        let commands = vec![
            composite_command("begin", layer_id, CompositeOperation::BeginIsolation),
            image_command(
                "background",
                layer_id,
                "background",
                bounds,
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                },
                BlendMode::SrcOver,
            ),
            image_command(
                "mask",
                layer_id,
                "mask",
                bounds,
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                },
                BlendMode::DstIn,
            ),
            composite_command("end", layer_id, CompositeOperation::EndIsolation),
        ];
        let output = render_image_scene_scalar(
            &scene(
                test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
                commands,
            ),
            &store,
            2,
            1,
        )
        .expect("render");
        assert_eq!(output.pixels, vec![255, 0, 0, 255, 0, 0, 0, 0]);
        assert_eq!(output.stats.isolation_count, 1);
        assert_eq!(output.stats.maximum_isolation_depth, 1);
        #[cfg(feature = "skia-core")]
        {
            let reference_commands = vec![
                composite_command("begin-ref", layer_id, CompositeOperation::BeginIsolation),
                image_command(
                    "background-ref",
                    layer_id,
                    "background",
                    bounds,
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 1.0,
                        height: 1.0,
                    },
                    BlendMode::SrcOver,
                ),
                image_command(
                    "mask-ref",
                    layer_id,
                    "mask",
                    bounds,
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 1.0,
                        height: 1.0,
                    },
                    BlendMode::DstIn,
                ),
                composite_command("end-ref", layer_id, CompositeOperation::EndIsolation),
            ];
            let reference = render_image_scene_skia_reference(
                &scene(
                    test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
                    reference_commands,
                ),
                &store,
                2,
                1,
            )
            .expect("Skia reference");
            assert_eq!(output.pixels, reference.pixels);
        }
    }

    #[test]
    fn containing_axis_aligned_clip_is_a_noop() {
        let (_temp, store) = store(vec![(
            "texture:assets/pixel",
            1,
            1,
            vec![255, 255, 255, 255],
        )]);
        let layer_id = StableId::derive("layer", b"containing-clip");
        let bounds = Rect {
            x: 0.25,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        let uv = Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        let mut clipped =
            image_command("clipped", layer_id, "pixel", bounds, uv, BlendMode::SrcOver);
        clipped.clip = Some([[-0.5, 0.0], [1.5, 0.0], [1.5, 1.0], [-0.5, 1.0]]);
        let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.5, 0.0]);
        let output =
            render_image_scene_scalar(&scene(layer.clone(), vec![clipped.clone()]), &store, 2, 1)
                .expect("containing clip");
        let unclipped = render_image_scene_scalar(
            &scene(
                layer.clone(),
                vec![image_command(
                    "unclipped",
                    layer_id,
                    "pixel",
                    bounds,
                    uv,
                    BlendMode::SrcOver,
                )],
            ),
            &store,
            2,
            1,
        )
        .expect("unclipped");
        assert_eq!(output.pixels, unclipped.pixels);
        #[cfg(feature = "skia-core")]
        {
            let reference =
                render_image_scene_skia_reference(&scene(layer, vec![clipped]), &store, 2, 1)
                    .expect("Skia reference");
            assert_eq!(output.pixels, reference.pixels);
        }
    }

    #[test]
    fn partial_axis_aligned_clip_matches_skia() {
        let (_temp, store) = store(vec![(
            "texture:assets/pixel",
            1,
            1,
            vec![255, 255, 255, 255],
        )]);
        let layer_id = StableId::derive("layer", b"clip");
        let mut command = image_command(
            "clipped",
            layer_id,
            "pixel",
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            BlendMode::SrcOver,
        );
        command.clip = Some([[0.0, 0.0], [0.49, 0.0], [0.49, 1.0], [0.0, 1.0]]);
        let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let output =
            render_image_scene_scalar(&scene(layer.clone(), vec![command.clone()]), &store, 1, 1)
                .expect("partial axis-aligned clip");
        #[cfg(feature = "skia-core")]
        {
            let reference =
                render_image_scene_skia_reference(&scene(layer, vec![command]), &store, 1, 1)
                    .expect("Skia reference");
            assert_eq!(output.pixels, reference.pixels);
        }
    }

    #[test]
    fn ellipse_image_clip_has_only_the_known_skia_edge_tie_difference() {
        if !packet_simd_available() {
            return;
        }
        let (_temp, store) = store(vec![(
            "texture:assets/pixel",
            1,
            1,
            vec![255, 255, 255, 255],
        )]);
        let layer_id = StableId::derive("layer", b"ellipse-image-clip");
        let mut command = image_command(
            "ellipse",
            layer_id,
            "pixel",
            Rect {
                x: 0.0,
                y: 0.0,
                width: 4.0,
                height: 4.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            BlendMode::SrcOver,
        );
        let SemanticCommandPayload::Image { clip, .. } = &mut command.payload else {
            unreachable!("image command helper must create an image payload");
        };
        *clip = Some(allium_renderer_core::ImageClip::Ellipse);
        let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let output =
            render_image_scene_scalar(&scene(layer.clone(), vec![command.clone()]), &store, 4, 4)
                .expect("ellipse clip");
        let simd =
            render_image_scene_simd(&scene(layer.clone(), vec![command.clone()]), &store, 4, 4)
                .expect("SIMD ellipse clip");
        assert_eq!(simd.pixels, output.pixels);
        assert_eq!(&output.pixels[0..4], &[0, 0, 0, 0]);
        #[cfg(feature = "skia-core")]
        {
            let reference =
                render_image_scene_skia_reference(&scene(layer, vec![command]), &store, 4, 4)
                    .expect("Skia reference");
            let different_pixels = output
                .pixels
                .chunks_exact(4)
                .zip(reference.pixels.chunks_exact(4))
                .filter(|(actual, expected)| actual != expected)
                .count();
            assert_eq!(different_pixels, 2, "only Skia's aliased edge ties differ");
        }
    }

    #[test]
    fn rounded_image_clip_simd_matches_scalar() {
        if !packet_simd_available() {
            return;
        }
        let (_temp, store) = store(vec![(
            "texture:assets/pixel",
            1,
            1,
            vec![255, 255, 255, 255],
        )]);
        let layer_id = StableId::derive("layer", b"rounded-image-clip");
        let mut command = image_command(
            "rounded",
            layer_id,
            "pixel",
            Rect {
                x: 0.0,
                y: 0.0,
                width: 8.0,
                height: 6.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            BlendMode::SrcOver,
        );
        let SemanticCommandPayload::Image { clip, .. } = &mut command.payload else {
            unreachable!("image command helper must create an image payload");
        };
        *clip = Some(allium_renderer_core::ImageClip::RoundedRect { radius: [2.0, 1.5] });
        let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let scalar =
            render_image_scene_scalar(&scene(layer.clone(), vec![command.clone()]), &store, 8, 6)
                .expect("scalar rounded clip");
        let simd = render_image_scene_simd(&scene(layer, vec![command]), &store, 8, 6)
            .expect("SIMD rounded clip");
        assert_eq!(simd.pixels, scalar.pixels);
    }

    #[test]
    fn scaled_translated_rounded_image_clip_simd_matches_scalar() {
        if !packet_simd_available() {
            return;
        }
        let (_temp, store) = store(vec![(
            "texture:assets/pattern",
            3,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255, 0, 255, 255, 255,
                255, 0, 255, 255,
            ],
        )]);
        let layer_id = StableId::derive("layer", b"scaled-rounded-image-clip");
        let mut command = image_command(
            "scaled-rounded",
            layer_id,
            "pattern",
            Rect {
                x: 1.0,
                y: 1.0,
                width: 7.0,
                height: 5.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            BlendMode::SrcOver,
        );
        let SemanticCommandPayload::Image { clip, .. } = &mut command.payload else {
            unreachable!("image command helper must create an image payload");
        };
        *clip = Some(allium_renderer_core::ImageClip::RoundedRect { radius: [1.5, 1.0] });
        let layer = test_layer(layer_id, [1.75, 0.0, 0.0, 1.4, 0.75, 0.5]);
        let scalar =
            render_image_scene_scalar(&scene(layer.clone(), vec![command.clone()]), &store, 19, 11)
                .expect("scalar scaled rounded clip");
        let simd = render_image_scene_simd(&scene(layer, vec![command]), &store, 19, 11)
            .expect("SIMD scaled rounded clip");
        assert_eq!(simd.pixels, scalar.pixels);
    }

    #[test]
    fn unsupported_non_axis_aligned_clip_fails_closed_without_pixels() {
        let (_temp, store) = store(vec![(
            "texture:assets/pixel",
            1,
            1,
            vec![255, 255, 255, 255],
        )]);
        let layer_id = StableId::derive("layer", b"non-axis-clip");
        let mut command = image_command(
            "clipped",
            layer_id,
            "pixel",
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            BlendMode::SrcOver,
        );
        command.clip = Some([[0.0, 0.0], [1.0, 0.0], [0.75, 1.0], [0.0, 1.0]]);
        let error = render_image_scene_scalar(
            &scene(
                test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
                vec![command],
            ),
            &store,
            1,
            1,
        )
        .expect_err("clip must fail closed");
        assert!(matches!(
            error,
            ProfileCompositorError::UnsupportedFeature { .. }
        ));
    }
}
