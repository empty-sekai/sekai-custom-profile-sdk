//! Ordered execution of semantic commands: control-state evaluation, per-command
//! dispatch with the isolation stack, image command validation and source fallbacks.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use sekai_profile_core::profile_scene::{ComponentControlState, ResolvedProfileScene};
use sekai_profile_core::{
    BlendMode, CompositeOperation, ImageMaterial, LayerSource, Matrix2d, ParameterValue, Rect,
    ResourceKey, SemanticCommandPayload, SemanticCommandSource, ShapePrimitive, StableId,
};
use sha2::{Digest, Sha256};

use super::ugui_text;
use super::{
    axis_aligned_command_clip, canvas_bytes, clear_composition_bounds, compose_matrix,
    composite_isolation, composition_bounds, deck_art_variant_for_command, deck_art_variant_uv,
    elapsed_ns, is_live_master_progress_command, raster_image_command,
    raster_semantic_shape_command, render_live_master_progress_text, render_semantic_text_command,
    render_semantic_text_commands, transform_point, unsupported, AxisAlignedClip, BadgeShading,
    CompositionTarget, EvaluatedCommandControlState, ImageClipGeometry, ImageExecutor,
    OwnedRenderObject, ProfileCompositorError, ProfileCompositorStats, SemanticSdfContext,
};
use crate::assets::AssetStore;
use crate::render_object::{
    render_object_key_for_resource, MappedRenderObjectStore, RenderObjectEntry, RenderObjectKind,
};

pub(super) fn evaluate_command_control_state(
    scene: &ResolvedProfileScene,
    command: &SemanticCommandSource,
) -> Result<EvaluatedCommandControlState, ProfileCompositorError> {
    let mut evaluated = EvaluatedCommandControlState {
        visible: true,
        translate_y: 0.0,
    };
    for binding in &command.control_bindings {
        let control_id = match binding {
            sekai_profile_core::CommandControlBinding::TabOption { control_id, .. }
            | sekai_profile_core::CommandControlBinding::ScrollContent { control_id }
            | sekai_profile_core::CommandControlBinding::ScrollThumb { control_id }
            | sekai_profile_core::CommandControlBinding::ScrollViewport { control_id } => {
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
                sekai_profile_core::CommandControlBinding::TabOption { value, .. },
                ComponentControlState::Tabs { active, .. },
            ) => evaluated.visible &= value == active,
            (
                sekai_profile_core::CommandControlBinding::ScrollContent { .. },
                ComponentControlState::Scroll { offset, .. },
            ) => evaluated.translate_y -= *offset,
            (
                sekai_profile_core::CommandControlBinding::ScrollThumb { .. },
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
                sekai_profile_core::CommandControlBinding::ScrollViewport { .. },
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

#[allow(clippy::too_many_arguments)]
pub(super) fn render_image_commands_into(
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
                render_live_master_progress_text(
                    command,
                    layer,
                    control_state.translate_y,
                    target,
                    width,
                    height,
                )?;
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
            SemanticCommandPayload::UguiText(source) => {
                let target = targets
                    .last_mut()
                    .map(|target| target.pixels.as_mut_slice())
                    .unwrap_or(destination);
                let raster = ugui_text::raster_ugui_text_command(
                    target,
                    width,
                    height,
                    command,
                    layer,
                    source,
                    control_state.translate_y,
                    executor,
                )?;
                stats.text_command_count = stats.text_command_count.saturating_add(1);
                stats.blended_fragment_count = stats
                    .blended_fragment_count
                    .saturating_add(raster.fragments);
                stats.simd_packet_count =
                    stats.simd_packet_count.saturating_add(raster.simd_packets);
                stats.scalar_fragment_count = stats
                    .scalar_fragment_count
                    .saturating_add(raster.scalar_fragments);
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
                material,
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
                    (variant, deck_art_variant_uv())
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
                // The mask resolves like any other resource, including the
                // source fallback, so a mask that has no baked object still
                // renders instead of dropping the command.
                let mask_key = alpha_mask.as_ref().map(render_object_key_for_resource);
                let mask_fallback = match (&mask_key, alpha_mask.as_ref()) {
                    (Some(key), Some(resource)) if store.object(key).is_none() => {
                        load_source_fallback_object(assets, resource)?
                    }
                    _ => None,
                };
                let mask_object = match (&mask_key, alpha_mask.as_ref()) {
                    (Some(key), Some(_)) => Some(
                        store
                            .object(key)
                            .or_else(|| mask_fallback.as_ref().map(OwnedRenderObject::mapped))
                            .ok_or_else(|| ProfileCompositorError::MissingObject(key.clone()))?,
                    ),
                    _ => None,
                };
                // The normal map is a required resource like the mask: it
                // resolves the same way and a missing one fails the command.
                let normal_map = match material {
                    ImageMaterial::Plain => None,
                    ImageMaterial::LitBadge { normal_map } => Some(normal_map),
                };
                let normal_map_key = normal_map.map(render_object_key_for_resource);
                let normal_map_fallback = match (&normal_map_key, normal_map) {
                    (Some(key), Some(resource)) if store.object(key).is_none() => {
                        load_source_fallback_object(assets, resource)?
                    }
                    _ => None,
                };
                let badge = match &normal_map_key {
                    Some(key) => Some(BadgeShading {
                        normal_map: store
                            .object(key)
                            .or_else(|| normal_map_fallback.as_ref().map(OwnedRenderObject::mapped))
                            .ok_or_else(|| ProfileCompositorError::MissingObject(key.clone()))?,
                        tangent: sekai_profile_core::badge_material::tangent(matrix),
                    }),
                    None => None,
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
                    mask_object,
                    badge,
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
                // A group inside scrolled content translates as a unit: its
                // recorded extent and its merge clip follow the same offset the
                // member commands apply to themselves.
                let mut translated_matrix = command.matrix;
                translated_matrix[5] += control_state.translate_y;
                stats.composite_command_count = stats.composite_command_count.saturating_add(1);
                validate_composite_command(&command.role, *opacity, clip.as_ref())?;
                match operation {
                    CompositeOperation::Marker => {}
                    CompositeOperation::BeginIsolation => {
                        let bounds = composition_bounds(
                            command.bounds,
                            compose_matrix(layer.matrix, translated_matrix),
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
                            // The group's opacity and clip are declared on the
                            // command that ends it, and apply to the merge.
                            let device_clip = clip
                                .as_ref()
                                .map(|clip| {
                                    axis_aligned_command_clip(
                                        clip,
                                        compose_matrix(layer.matrix, translated_matrix),
                                    )
                                })
                                .transpose()
                                .map_err(|()| ProfileCompositorError::UnsupportedFeature {
                                    role: command.role.clone(),
                                    feature: "rotated composite clip".into(),
                                })?;
                            composite_isolation(
                                parent,
                                &child.pixels,
                                width,
                                child.bounds,
                                *opacity,
                                device_clip,
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

pub(super) fn collect_layers(
    scene: &ResolvedProfileScene,
) -> Result<BTreeMap<sekai_profile_core::StableId, &LayerSource>, ProfileCompositorError> {
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
    command_clip: Option<&sekai_profile_core::Quad>,
    bounds: Rect,
    clip_matrix: Matrix2d,
    matrix: Matrix2d,
    image_clip: Option<&sekai_profile_core::ImageClip>,
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
        sekai_profile_core::ImageClip::RoundedRect { radius } => {
            ImageClipGeometry::RoundedRect { radius: *radius }
        }
        sekai_profile_core::ImageClip::Ellipse => ImageClipGeometry::Ellipse,
    });
    if matches!(metadata.get("sampling"), Some(ParameterValue::Text(value)) if value != "nearest") {
        return unsupported(role, "non-nearest sampling");
    }
    Ok((command_clip, image_clip))
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

fn validate_composite_command(
    role: &str,
    opacity: f32,
    clip: Option<&sekai_profile_core::Quad>,
) -> Result<(), ProfileCompositorError> {
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return Err(ProfileCompositorError::InvalidGeometry { role: role.into() });
    }
    if let Some(clip) = clip {
        if !clip.iter().flatten().all(|value| value.is_finite()) {
            return Err(ProfileCompositorError::InvalidGeometry { role: role.into() });
        }
    }
    Ok(())
}

fn load_source_fallback_object(
    assets: Option<&AssetStore>,
    resource: &ResourceKey,
) -> Result<Option<OwnedRenderObject>, ProfileCompositorError> {
    let Some(assets) = assets else {
        return Ok(None);
    };
    // Static and dynamic assets share one key space in the asset store.
    if resource.namespace != "assets" && resource.namespace != "static" {
        return Ok(None);
    }
    let Some((width, height, pixels)) = assets.get_premultiplied_rgba(&resource.key) else {
        return Ok(None);
    };
    let row_bytes = usize::try_from(width)
        .ok()
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| ProfileCompositorError::InvalidObject(resource.key.clone()))?;
    if pixels.len() != row_bytes.saturating_mul(height as usize) {
        return Err(ProfileCompositorError::InvalidObject(resource.key.clone()));
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
