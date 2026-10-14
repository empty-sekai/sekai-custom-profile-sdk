//! Profile animation export: groups a card's elements into static and dynamic layers,
//! prepares a raster for each group, and streams composited frames into the encoder.

use crate::render_config::RenderError;
use crate::renderer::{CardSources, CustomProfileRenderer, SdfPass};
use crate::types::CustomProfileCard;

use super::compositor::{AnimationFrameCompositor, RasterLayer, RasterLayerContent};
use super::encode::mp4::encode_mp4_into;
use super::encode::{
    encode_rgba_frames_with_h264_and_telemetry, finish_encoded_animation, frame_byte_len,
    validate_spec, AnimationEncodeTelemetry,
};
use super::timeline::{animation_loop_period_ticks, plan_ticks};
use super::{
    elapsed_ns, AnimationDocument, AnimationEncodeSpec, AnimationError, AnimationExportTelemetry,
    AnimationFormat, AnimationPreset, ProfileAnimationExport, ResolvedAnimationPreset,
};

/// Atlas-identity usage label for the animation layer raster.
pub(super) const ANIMATION_LAYER_RASTER_USAGE: &str = "animation-layer-raster";

pub(super) type AnimationRasterGroup = (
    Option<sekai_profile_core::LayerId>,
    Vec<(sekai_profile_core::AuthoredElementKind, usize)>,
);

pub(crate) fn export_profile_animation(
    renderer: &CustomProfileRenderer,
    card: &CustomProfileCard,
    document: AnimationDocument<'_>,
    sources: CardSources<'_>,
    preset: &ResolvedAnimationPreset,
    config: &crate::render_config::RenderConfig,
) -> Result<ProfileAnimationExport, AnimationError> {
    let CardSources {
        md,
        assets,
        profile,
        ..
    } = sources;
    let (mut scene, resolved_scene) = if card.generals.is_empty() {
        let text_atlases = renderer.mapped_text_sdf_atlases();
        (
            crate::core_shadow::build_text_scene_with_atlases(
                card,
                md,
                document.key,
                text_atlases.as_deref(),
            )?,
            None,
        )
    } else {
        let (scene, resolved) = crate::core_shadow::build_scene_with_resolved(
            card,
            md,
            document.key,
            document.region,
            profile,
            document.region,
            assets,
        )
        .map_err(RenderError::Render)?;
        (scene, Some(resolved))
    };
    let preflight = scene.animation_preflight(preset.maximum_ticks)?;
    if !preflight.animated {
        return Ok(ProfileAnimationExport {
            animated: false,
            encoded: None,
            telemetry: AnimationExportTelemetry {
                compiled_program_count: preflight.compiled_program_count,
                observable_program_count: 0,
                sampled_frames: 0,
                dynamic_evaluations: 0,
                layout_rebuilds: 0,
                command_rebuilds: 0,
                atlas_uploads: 0,
                layer_raster_bytes: 0,
                layer_scratch_peak_bytes: 0,
                peak_export_bytes: 0,
                render_ms: 0.0,
                encode_ms: 0.0,
                frame_full_composites: 0,
                frame_dirty_composites: 0,
                frame_dirty_regions: 0,
                frame_reused_composites: 0,
                frame_dirty_pixels: 0,
                frame_composite_ns: 0,
                frame_readback_ns: 0,
                frame_readback_bytes: 0,
                encoder_frame_callback_calls: 0,
                encoder_frame_callback_ns: 0,
                gif_palette_sample_ns: 0,
                gif_palette_quantize_ns: 0,
                gif_delta_index_ns: 0,
                gif_lzw_ns: 0,
                gif_retained_frame_bytes: 0,
                gif_second_frame_pass: false,
                h264_yuv_ns: 0,
                h264_pipe_write_ns: 0,
                h264_codec_wait_ns: 0,
                h264_output_read_ns: 0,
                h264_direct_encode_ns: 0,
                h264_mux_ns: 0,
                h264_mbinfo_constant_blocks: 0,
                h264_mbinfo_total_blocks: 0,
                budget_step: preset.budget_step,
                gif_quality: preset.gif_quality,
                encode_attempts: 0,
                render_telemetry: None,
            },
        });
    }

    let render_started = std::time::Instant::now();
    let dynamic_layer_ids = preflight.observable_layer_ids.clone();
    let ticks = plan_ticks(
        &mut scene,
        &dynamic_layer_ids,
        preset.maximum_ticks,
        preset.fps,
    );
    let looped = animation_loop_period_ticks(
        dynamic_layer_ids.iter().filter_map(|id| {
            scene
                .state(*id)
                .and_then(|state| state.dynamic.as_ref())
                .and_then(|state| state.timeline)
        }),
        preset.maximum_ticks,
    )
    .is_some();
    let dynamic_expansions = if preset.format == AnimationFormat::Mp4 {
        animation_layer_expansions(&mut scene, &dynamic_layer_ids, &ticks)
    } else {
        scene.advance_to_tick(0);
        std::collections::BTreeMap::new()
    };
    let authored = sekai_profile_core::profile_scene::ordered_profile_elements(card, document.key);
    let mut groups: Vec<AnimationRasterGroup> = Vec::new();
    let mut static_span = Vec::new();
    for element in &authored {
        if dynamic_layer_ids.contains(&element.layer_id) {
            if !static_span.is_empty() {
                groups.push((None, std::mem::take(&mut static_span)));
            }
            groups.push((
                Some(element.layer_id),
                vec![(element.kind, element.source_index)],
            ));
        } else {
            static_span.push((element.kind, element.source_index));
        }
    }
    if !static_span.is_empty() {
        groups.push((None, static_span));
    }
    let source_width = crate::transform::CANVAS_WIDTH as u32;
    let source_height = crate::transform::CANVAS_HEIGHT as u32;
    let scale = (preset.maximum_long_edge as f32 / source_width.max(source_height) as f32).min(1.0);
    let content_width = (source_width as f32 * scale).round() as u32;
    let content_height = (source_height as f32 * scale).round() as u32;
    let (width, height) = if preset.preset == AnimationPreset::QqV2 {
        (align_to_8(content_width), align_to_8(content_height))
    } else {
        (content_width, content_height)
    };
    let frame_bytes = frame_byte_len(width, height)?;
    let retained_layer_budget = preset
        .export_memory_budget_bytes
        .saturating_sub(frame_bytes.saturating_mul(2));
    let rasterizer = GroupRasterizer {
        renderer,
        card,
        sources,
    };
    let (layers, layer_raster_bytes, layer_scratch_peak_bytes, render_telemetry) = rasterizer
        .rasterize_animation_groups(
            resolved_scene.as_ref(),
            &dynamic_expansions,
            &groups,
            config,
            retained_layer_budget,
        )?;

    let base_peak_export_bytes = animation_peak_export_bytes(layer_raster_bytes, frame_bytes);
    if base_peak_export_bytes > preset.export_memory_budget_bytes {
        return Err(AnimationError::ExportMemoryExceeded {
            bytes: base_peak_export_bytes,
            budget: preset.export_memory_budget_bytes,
        });
    }
    let gif_retention_budget = preset
        .export_memory_budget_bytes
        .saturating_sub(base_peak_export_bytes);

    let render_ms = render_started.elapsed().as_secs_f64() * 1000.0;

    let encode_spec = AnimationEncodeSpec {
        width,
        height,
        fps: preset.fps,
        frame_count: ticks.len() as u32,
        looped,
        output_budget_bytes: preset.output_budget_bytes,
        gif_quality: preset.gif_quality,
    };
    scene.advance_to_tick(0);
    let mut render_scene = scene;
    let deferred_cache_budget = if preset.format == AnimationFormat::Mp4 {
        preset
            .export_memory_budget_bytes
            .saturating_sub(base_peak_export_bytes)
            .saturating_sub(frame_bytes)
    } else {
        0
    };
    let mut compositor = AnimationFrameCompositor::new_for_export(
        renderer,
        width,
        height,
        scale,
        render_telemetry,
        deferred_cache_budget,
    )?;
    let encode_started = std::time::Instant::now();
    let (encoded, encode_telemetry) = if preset.format == AnimationFormat::Mp4 {
        validate_spec(&encode_spec)?;
        let mut telemetry = AnimationEncodeTelemetry::default();
        let data = encode_mp4_into(
            &encode_spec,
            frame_bytes,
            &preset.h264,
            &mut telemetry,
            |index, rgba| {
                let tick = ticks[index as usize];
                render_scene.advance_to_tick(tick);
                compositor.composite_into(index, &render_scene, &layers, rgba)
            },
        )?;
        (
            finish_encoded_animation(preset.format, &encode_spec, frame_bytes, data)?,
            telemetry,
        )
    } else {
        encode_rgba_frames_with_h264_and_telemetry(
            preset.format,
            &encode_spec,
            &preset.h264,
            gif_retention_budget,
            |index| {
                let tick = ticks[index as usize];
                render_scene.advance_to_tick(tick);
                compositor.composite(index, &render_scene, &layers)
            },
        )?
    };
    let encode_ms = encode_started.elapsed().as_secs_f64() * 1000.0;
    let frame_telemetry = compositor.telemetry;
    if let Some(telemetry) = compositor.render_telemetry.as_mut() {
        telemetry.bytes.layer_cache_bytes = telemetry
            .bytes
            .layer_cache_bytes
            .saturating_add(compositor.deferred_cache.peak_bytes as u64);
    }
    let render_telemetry = compositor.render_telemetry.take();
    let core_telemetry = render_scene.dump().telemetry;
    Ok(ProfileAnimationExport {
        animated: true,
        encoded: Some(encoded),
        telemetry: AnimationExportTelemetry {
            compiled_program_count: preflight.compiled_program_count,
            observable_program_count: preflight.observable_program_count,
            sampled_frames: ticks.len() as u32,
            dynamic_evaluations: core_telemetry.dynamic_evaluations,
            layout_rebuilds: 0,
            command_rebuilds: 0,
            atlas_uploads: 0,
            layer_raster_bytes,
            layer_scratch_peak_bytes,
            peak_export_bytes: base_peak_export_bytes
                .saturating_add(frame_telemetry.deferred_cache_peak_bytes)
                .saturating_add(encode_telemetry.gif_retained_frame_bytes),
            render_ms,
            encode_ms,
            frame_full_composites: frame_telemetry.full_composites,
            frame_dirty_composites: frame_telemetry.dirty_composites,
            frame_dirty_regions: frame_telemetry.dirty_regions,
            frame_reused_composites: frame_telemetry.reused_composites,
            frame_dirty_pixels: frame_telemetry.dirty_pixels,
            frame_composite_ns: frame_telemetry.composite_ns,
            frame_readback_ns: frame_telemetry.readback_ns,
            frame_readback_bytes: frame_telemetry.readback_bytes,
            encoder_frame_callback_calls: encode_telemetry.frame_callback_calls,
            encoder_frame_callback_ns: encode_telemetry.frame_callback_ns,
            gif_palette_sample_ns: encode_telemetry.gif_palette_sample_ns,
            gif_palette_quantize_ns: encode_telemetry.gif_palette_quantize_ns,
            gif_delta_index_ns: encode_telemetry.gif_delta_index_ns,
            gif_lzw_ns: encode_telemetry.gif_lzw_ns,
            gif_retained_frame_bytes: encode_telemetry.gif_retained_frame_bytes,
            gif_second_frame_pass: encode_telemetry.gif_second_frame_pass,
            h264_yuv_ns: encode_telemetry.h264_yuv_ns,
            h264_pipe_write_ns: encode_telemetry.h264_pipe_write_ns,
            h264_codec_wait_ns: encode_telemetry.h264_codec_wait_ns,
            h264_output_read_ns: encode_telemetry.h264_output_read_ns,
            h264_direct_encode_ns: encode_telemetry.h264_direct_encode_ns,
            h264_mux_ns: encode_telemetry.h264_mux_ns,
            h264_mbinfo_constant_blocks: encode_telemetry.h264_mbinfo_constant_blocks,
            h264_mbinfo_total_blocks: encode_telemetry.h264_mbinfo_total_blocks,
            budget_step: preset.budget_step,
            gif_quality: preset.gif_quality,
            encode_attempts: 1,
            render_telemetry,
        },
    })
}

pub(super) fn animation_peak_export_bytes(layer_raster_bytes: usize, frame_bytes: usize) -> usize {
    layer_raster_bytes.saturating_add(frame_bytes.saturating_mul(2))
}

pub(super) const fn align_to_8(value: u32) -> u32 {
    value.saturating_add(7) & !7
}

fn animation_layer_expansions(
    scene: &mut sekai_profile_core::Scene,
    dynamic_layer_ids: &[sekai_profile_core::LayerId],
    ticks: &[u64],
) -> std::collections::BTreeMap<sekai_profile_core::LayerId, [i32; 4]> {
    let mut ranges = dynamic_layer_ids
        .iter()
        .copied()
        .map(|id| (id, [0.0f32; 4]))
        .collect::<std::collections::BTreeMap<_, _>>();
    for tick in ticks {
        scene.advance_to_tick(*tick);
        for id in dynamic_layer_ids {
            let transform = scene
                .state(*id)
                .and_then(|state| state.dynamic.as_ref())
                .map(|state| state.transform)
                .unwrap_or_default();
            let range = ranges.get_mut(id).unwrap();
            range[0] = range[0].min(transform.dx);
            range[1] = range[1].max(transform.dx);
            range[2] = range[2].min(transform.dy);
            range[3] = range[3].max(transform.dy);
        }
    }
    scene.advance_to_tick(0);
    ranges
        .into_iter()
        .map(|(id, [min_dx, max_dx, min_dy, max_dy])| {
            const PAD: i32 = 8;
            (
                id,
                [
                    max_dx.max(0.0).ceil() as i32 + PAD,
                    (-min_dx).max(0.0).ceil() as i32 + PAD,
                    max_dy.max(0.0).ceil() as i32 + PAD,
                    (-min_dy).max(0.0).ceil() as i32 + PAD,
                ],
            )
        })
        .collect()
}

/// Rasterizes the element groups of one card's animation export through the
/// renderer's ordered SDF path.
#[derive(Clone, Copy)]
pub(super) struct GroupRasterizer<'a> {
    pub(super) renderer: &'a CustomProfileRenderer,
    /// The card the groups' elements index into.
    pub(super) card: &'a CustomProfileCard,
    pub(super) sources: CardSources<'a>,
}

impl GroupRasterizer<'_> {
    /// Rasterizes `groups` on the executor `config` resolves to, preparing the
    /// fallback glyphs of `resolved_scene` (or of the card itself when it
    /// resolved to no scene) first, and reports the layers with their
    /// retained and peak scratch bytes and the render telemetry.
    pub(super) fn rasterize_animation_groups(
        &self,
        resolved_scene: Option<&sekai_profile_core::profile_scene::ResolvedProfileScene>,
        _dynamic_expansions: &std::collections::BTreeMap<sekai_profile_core::LayerId, [i32; 4]>,
        groups: &[AnimationRasterGroup],
        config: &crate::render_config::RenderConfig,
        retained_layer_budget: usize,
    ) -> Result<
        (
            Vec<RasterLayer>,
            usize,
            usize,
            Option<crate::telemetry::RenderTelemetry>,
        ),
        AnimationError,
    > {
        use crate::telemetry::RenderTelemetry;

        let Self {
            renderer,
            card,
            sources,
        } = *self;
        let started = std::time::Instant::now();
        let executor = renderer.resolve_executor(config)?;
        let mut telemetry = RenderTelemetry::new(config.clone(), executor.public());
        telemetry.fallback_glyph_cache = match resolved_scene {
            Some(scene) => renderer.prepare_profile_fallback_for_animation_scene(scene, sources.md),
            None => renderer.prepare_profile_fallback_for_animation_card(card, sources.md),
        }
        .map_err(RenderError::Render)?;
        telemetry.work.page_count = 1;
        telemetry.work.dynamic_layer_count = groups
            .iter()
            .filter(|(dynamic_layer_id, _)| dynamic_layer_id.is_some())
            .count() as u64;

        renderer.record_atlas_identities(&mut telemetry, ANIMATION_LAYER_RASTER_USAGE);
        let (layers, bytes, scratch) = self.rasterize_animation_groups_ordered(
            groups,
            SdfPass::new(config, executor),
            &mut telemetry,
            retained_layer_budget,
        )?;
        telemetry.bytes.layer_cache_bytes = bytes as u64;
        telemetry.bytes.scratch_peak_bytes = scratch as u64;
        telemetry.timings.total_ns = elapsed_ns(started);
        Ok((layers, bytes, scratch, Some(telemetry)))
    }

    /// Rasterizes each animation group through the ordered SDF path — the same
    /// pipeline pages use — with `pass`.
    ///
    /// Oversized glyphs are generated once for the whole export rather than once
    /// per layer: every group's card contributes to a single realtime-EDT batch
    /// that stays installed for the layer loop, so layers that share a glyph share
    /// the page generated for it.
    fn rasterize_animation_groups_ordered(
        &self,
        groups: &[AnimationRasterGroup],
        pass: SdfPass,
        telemetry: &mut crate::telemetry::RenderTelemetry,
        retained_layer_budget: usize,
    ) -> Result<(Vec<RasterLayer>, usize, usize), AnimationError> {
        let Self {
            renderer,
            card,
            sources,
        } = *self;
        let mut layers = Vec::with_capacity(groups.len());
        let mut layer_raster_bytes = 0usize;
        let mut scratch_peak_bytes = 0usize;
        let layer_cards = groups
            .iter()
            .map(|(_, members)| grouped_layer_card(card, members))
            .collect::<Vec<_>>();
        let prepared = renderer
            .prepare_realtime_edt_batch(&layer_cards, sources)
            .map_err(RenderError::Render)?;
        telemetry.work.realtime_edt_glyph_count = prepared
            .glyphs
            .iter()
            .map(|glyph| glyph.substitution_count)
            .sum();
        telemetry.bytes.realtime_edt_page_bytes =
            prepared.glyphs.iter().map(|glyph| glyph.page_bytes).sum();
        telemetry.timings.realtime_edt_generation_ns = prepared
            .glyphs
            .iter()
            .map(|glyph| glyph.generation_ns)
            .sum();
        telemetry
            .realtime_edt_glyphs
            .extend(prepared.glyphs.iter().cloned());
        telemetry.realtime_edt_batch.accumulate(&prepared.telemetry);

        renderer.with_realtime_edt_batch(prepared, || {
            for ((dynamic_layer_id, _), layer_card) in groups.iter().zip(&layer_cards) {
                let output = renderer.render_ordered_element_layer_cropped(
                    layer_card,
                    sources,
                    pass,
                    animation_group_uses_dynamic_bounds(*dynamic_layer_id),
                    retained_layer_budget.saturating_sub(layer_raster_bytes),
                )?;
                if let Some(execution) = output.execution.as_ref() {
                    record_animation_layer_execution(telemetry, execution);
                }
                push_animation_raster(
                    &mut layers,
                    &mut layer_raster_bytes,
                    &mut scratch_peak_bytes,
                    *dynamic_layer_id,
                    output.raster,
                )?;
            }
            Ok((layers, layer_raster_bytes, scratch_peak_bytes))
        })
    }
}

/// Folds one layer's SDF execution record into the export-wide telemetry.
fn record_animation_layer_execution(
    telemetry: &mut crate::telemetry::RenderTelemetry,
    output: &crate::renderer::SurfaceOutput,
) {
    telemetry.work.realtime_edt_glyph_count =
        telemetry.work.realtime_edt_glyph_count.saturating_add(
            output
                .realtime_edt_glyphs
                .iter()
                .map(|glyph| glyph.substitution_count)
                .sum::<u64>(),
        );
    telemetry.bytes.realtime_edt_page_bytes =
        telemetry.bytes.realtime_edt_page_bytes.saturating_add(
            output
                .realtime_edt_glyphs
                .iter()
                .map(|glyph| glyph.page_bytes)
                .sum::<u64>(),
        );
    telemetry.timings.realtime_edt_generation_ns =
        telemetry.timings.realtime_edt_generation_ns.saturating_add(
            output
                .realtime_edt_glyphs
                .iter()
                .map(|glyph| glyph.generation_ns)
                .sum::<u64>(),
        );
    telemetry
        .realtime_edt_glyphs
        .extend(output.realtime_edt_glyphs.iter().cloned());
    telemetry
        .realtime_edt_batch
        .accumulate(&output.realtime_edt_batch);
    telemetry.work.element_run_count = telemetry
        .work
        .element_run_count
        .saturating_add(output.sdf_run_count);
    telemetry.bytes.atlas_mapped_bytes = output.atlas_mapped_bytes;
    telemetry.timings.surface_create_ns = telemetry
        .timings
        .surface_create_ns
        .saturating_add(output.timings.surface_create_ns);
    telemetry.timings.surface_clear_ns = telemetry
        .timings
        .surface_clear_ns
        .saturating_add(output.timings.surface_clear_ns);
    telemetry.timings.sdf_capture_ns = telemetry
        .timings
        .sdf_capture_ns
        .saturating_add(output.timings.capture_ns);
    telemetry.timings.sdf_capture_rich_parse_ns = telemetry
        .timings
        .sdf_capture_rich_parse_ns
        .saturating_add(output.timings.capture_rich_parse_ns);
    telemetry.timings.sdf_capture_font_resolve_ns = telemetry
        .timings
        .sdf_capture_font_resolve_ns
        .saturating_add(output.timings.capture_font_resolve_ns);
    telemetry.timings.sdf_capture_layout_setup_ns = telemetry
        .timings
        .sdf_capture_layout_setup_ns
        .saturating_add(output.timings.capture_layout_setup_ns);
    telemetry.timings.sdf_capture_measure_ns = telemetry
        .timings
        .sdf_capture_measure_ns
        .saturating_add(output.timings.capture_measure_ns);
    telemetry.timings.sdf_capture_command_build_ns = telemetry
        .timings
        .sdf_capture_command_build_ns
        .saturating_add(output.timings.capture_command_build_ns);
    telemetry.timings.sdf_capture_emit_ns = telemetry
        .timings
        .sdf_capture_emit_ns
        .saturating_add(output.timings.capture_emit_ns);
    telemetry.timings.sdf_command_mapping_ns = telemetry
        .timings
        .sdf_command_mapping_ns
        .saturating_add(output.timings.command_mapping_ns);
    telemetry.timings.sdf_plan_build_ns = telemetry
        .timings
        .sdf_plan_build_ns
        .saturating_add(output.timings.plan_build_ns);
    telemetry.timings.sdf_execute_ns = telemetry
        .timings
        .sdf_execute_ns
        .saturating_add(output.timings.execute_ns);
    telemetry.timings.rgba_snapshot_ns = telemetry
        .timings
        .rgba_snapshot_ns
        .saturating_add(output.timings.rgba_snapshot_ns);
    telemetry.timings.dynamic_layer_bounds_ns = telemetry
        .timings
        .dynamic_layer_bounds_ns
        .saturating_add(output.timings.postprocess_bounds_ns);
    telemetry.timings.dynamic_layer_crop_ns = telemetry
        .timings
        .dynamic_layer_crop_ns
        .saturating_add(output.timings.postprocess_crop_ns);
    telemetry.record_executed_sdf_commands(
        output.sdf_text_element_count,
        output.sdf_shape_element_count,
        output.captured_text_count,
    );
    telemetry.record_sdf_plan(output.plan_stats, output.plan_bytes, output.span_bytes);
    telemetry.record_sdf_execution(output.execution_stats);
    telemetry.record_commands(
        crate::telemetry::CommandKind::Image,
        output.software_image_count,
        output.timings.image_composite_ns,
    );
}

fn push_animation_raster(
    layers: &mut Vec<RasterLayer>,
    layer_raster_bytes: &mut usize,
    scratch_peak_bytes: &mut usize,
    dynamic_layer_id: Option<sekai_profile_core::LayerId>,
    output: crate::renderer::AnimationLayerRaster,
) -> Result<(), AnimationError> {
    match output {
        crate::renderer::AnimationLayerRaster::Ordered(output) => {
            if output.width == 0 || output.height == 0 {
                return Ok(());
            }
            *layer_raster_bytes = layer_raster_bytes
                .checked_add(output.width as usize * output.height as usize * 4)
                .ok_or(AnimationError::Internal(
                    "animation layer raster byte overflow",
                ))?;
            *scratch_peak_bytes = (*scratch_peak_bytes).max(output.scratch_peak_bytes);
            layers.push(RasterLayer {
                dynamic_layer_id,
                content: RasterLayerContent::Pixels(output.pixels),
                x: output.x as f32,
                y: output.y as f32,
                width: output.width,
                height: output.height,
            });
            Ok(())
        }
        crate::renderer::AnimationLayerRaster::DeferredSdf(layer) => {
            layers.push(RasterLayer {
                dynamic_layer_id,
                x: layer.x as f32,
                y: layer.y as f32,
                width: layer.width,
                height: layer.height,
                content: RasterLayerContent::Deferred(layer),
            });
            Ok(())
        }
    }
}

pub(super) fn animation_group_uses_dynamic_bounds(
    dynamic_layer_id: Option<sekai_profile_core::LayerId>,
) -> bool {
    dynamic_layer_id.is_some()
}

fn grouped_layer_card(
    base: &CustomProfileCard,
    members: &[(sekai_profile_core::AuthoredElementKind, usize)],
) -> CustomProfileCard {
    let mut card = CustomProfileCard {
        texts: Vec::new(),
        shapes: Vec::new(),
        card_members: Vec::new(),
        stamps: Vec::new(),
        others: Vec::new(),
        bonds_honors: Vec::new(),
        honors: Vec::new(),
        collections: Vec::new(),
        generals: Vec::new(),
        general_backgrounds: Vec::new(),
        stand_members: Vec::new(),
        story_backgrounds: Vec::new(),
        character_icons: Vec::new(),
        materials: Vec::new(),
        user_interface_icons: Vec::new(),
    };
    use sekai_profile_core::AuthoredElementKind::*;
    for (kind, index) in members {
        match kind {
            Text => card.texts.push(base.texts[*index].clone()),
            Shape => card.shapes.push(base.shapes[*index].clone()),
            CardMember => card.card_members.push(base.card_members[*index].clone()),
            Stamp => card.stamps.push(base.stamps[*index].clone()),
            Other => card.others.push(base.others[*index].clone()),
            BondsHonor => card.bonds_honors.push(base.bonds_honors[*index].clone()),
            Honor => card.honors.push(base.honors[*index].clone()),
            Collection => card.collections.push(base.collections[*index].clone()),
            General => card.generals.push(base.generals[*index].clone()),
            StandMember => card.stand_members.push(base.stand_members[*index].clone()),
            GeneralBackground => card
                .general_backgrounds
                .push(base.general_backgrounds[*index].clone()),
            StoryBackground => card
                .story_backgrounds
                .push(base.story_backgrounds[*index].clone()),
            CharacterIcon => card
                .character_icons
                .push(base.character_icons[*index].clone()),
            Material => card.materials.push(base.materials[*index].clone()),
            UserInterfaceIcon => card
                .user_interface_icons
                .push(base.user_interface_icons[*index].clone()),
        }
    }
    card
}
