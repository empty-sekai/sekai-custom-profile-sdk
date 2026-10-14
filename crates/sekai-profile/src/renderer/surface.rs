//! Ordered SDF surfaces: the elements of a card drawn in layer order, text and
//! shapes as SDF runs and every other element through the image compositor.

use super::authored::{
    profile_command_kind, render_authored_image_into_pixels, render_element_authored_identity,
    resolve_native_profile_scene,
};
use super::capture::{map_captured_sdf_commands, CapturedSdfPrimitive, SdfAtlases};
use super::occlusion::{add_sdf_occlusion_stats, build_pixel_occlusion_dry_run};
use super::realtime_edt::active_realtime_edt_batch;
use super::{elapsed_ns, CardSources, CustomProfileRenderer};
use crate::elements::SdfCaptureObserver;
use crate::render_config::{Executor, PixelOcclusion, RenderConfig};
use crate::types::CustomProfileCard;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

impl CustomProfileRenderer {
    /// Draws a full page. Glyphs larger than every installed atlas tier are
    /// generated once for the page first, unless the caller already installed
    /// a batch of them.
    pub(super) fn render_full_card(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        pass: SdfPass,
        clear_rgba: [u8; 4],
        pre_resolved_scene: Option<&sekai_profile_core::profile_scene::ResolvedProfileScene>,
        render_object_store: &crate::render_object::MappedRenderObjectStore,
    ) -> Result<SurfaceOutput, String> {
        let md = self.snapshot();
        let sources = CardSources {
            md: &md,
            assets: self.assets.as_deref(),
            profile,
            render_objects: render_object_store,
        };
        let render = || {
            self.render_ordered_sdf_surface(
                card,
                sources,
                pass,
                OrderedSdfSurfaceSpec::full_card(clear_rgba),
                pre_resolved_scene,
            )
        };
        if !self.realtime_oversized_glyph_generation || active_realtime_edt_batch().is_some() {
            return render();
        }
        let prepared = self.prepare_realtime_edt_batch(std::slice::from_ref(card), sources)?;
        let prepared_for_telemetry = Arc::clone(&prepared);
        let mut output = self.with_realtime_edt_batch(prepared, render)?;
        output
            .realtime_edt_glyphs
            .extend(prepared_for_telemetry.glyphs.iter().cloned());
        output
            .realtime_edt_batch
            .accumulate(&prepared_for_telemetry.telemetry);
        Ok(output)
    }

    /// Draws the visible elements of `card` in layer order onto a surface
    /// `spec` describes: text and shapes as ordered SDF runs, every other
    /// element through the image compositor.
    pub(super) fn render_ordered_sdf_surface(
        &self,
        card: &CustomProfileCard,
        sources: CardSources<'_>,
        pass: SdfPass,
        spec: OrderedSdfSurfaceSpec,
        pre_resolved_scene: Option<&sekai_profile_core::profile_scene::ResolvedProfileScene>,
    ) -> Result<SurfaceOutput, String> {
        let CardSources {
            md,
            assets,
            profile,
            render_objects: render_object_store,
        } = sources;
        let total_started = std::time::Instant::now();
        let semantic_resolve_started = std::time::Instant::now();
        let resolved_scene = if pre_resolved_scene.is_none() {
            Some(
                resolve_native_profile_scene(card, md, profile, assets, Some(render_object_store))
                    .map_err(|error| format!("resolve ordered profile scene: {error}"))?,
            )
        } else {
            None
        };
        let semantic_scene = pre_resolved_scene
            .or(resolved_scene.as_ref())
            .ok_or_else(|| "the page scene was not resolved".to_string())?;
        let semantic_resolve_ns = elapsed_ns(semantic_resolve_started);
        let surface_started = std::time::Instant::now();
        let width = spec.surface_width;
        let height = spec.surface_height;
        let row_bytes = width as usize * 4;
        let mut rgba = self.take_profile_rgba_scratch(row_bytes * height as usize, spec.clear_rgba);
        let surface_create_ns = elapsed_ns(surface_started);

        let sdf_atlases = self.sdf_atlases.load_full();
        let text_atlases = sdf_atlases.as_ref();
        let shape_atlas = self.shape_sdf_atlas.as_deref();
        let source = crate::sdf::tile::MixedSdfAtlasSource::new(text_atlases, shape_atlas)
            .map_err(|error| error.to_string())?;
        let grid = crate::sdf::tile::TileGrid {
            canvas_width: width,
            canvas_height: height,
            tile_width: pass.tile_width,
            tile_height: pass.tile_height,
        };
        let elements = crate::elements::flatten_and_sort(card);
        let run_context = SdfRunContext {
            atlases: SdfAtlases {
                text: text_atlases,
                shape: shape_atlas,
                source: &source,
            },
            grid,
            shape_program_cache: &self.shape_row_program_cache,
            execute_occlusion: pass.pixel_occlusion == PixelOcclusion::Cull,
        };
        let occlusion_dry_run = if pass.pixel_occlusion.enabled() {
            Some(build_pixel_occlusion_dry_run(
                card,
                &elements,
                semantic_scene,
                render_object_store,
                width,
                height,
            )?)
        } else {
            None
        };
        let mut pending = PendingSdfRun::default();
        let mut aggregate = SurfaceAggregate::default();
        aggregate.prepare_direct_axis_shape = spec.prepare_direct_axis_shape;
        aggregate.realtime_oversized_glyph_generation = self.realtime_oversized_glyph_generation;
        aggregate.render_object_mapped_bytes = render_object_store.mapped_bytes();
        aggregate.timings.semantic_resolve_ns = semantic_resolve_ns;
        aggregate.timings.surface_create_ns = surface_create_ns;
        if let Some(dry_run) = occlusion_dry_run.as_ref() {
            aggregate.occlusion_eligible_image_count = dry_run.eligible_image_count;
            aggregate.occlusion_mask_snapshot_count = dry_run.mask_snapshot_count;
            aggregate.occlusion_mask_bytes = dry_run.mask_bytes;
            aggregate.timings.occlusion_mask_build_ns = dry_run.build_ns;
        }
        let clear_started = std::time::Instant::now();
        // `rgba` was allocated with `vec![0; len]`, which is already the exact
        // premultiplied transparent-black surface required by animation
        // layers, so only a non-transparent clear touches the buffer.
        if spec.clear_rgba != [0, 0, 0, 0] {
            fill_premultiplied_clear(&mut rgba, spec.clear_rgba);
        }
        aggregate.timings.surface_clear_ns = elapsed_ns(clear_started);
        let canvas = sekai_profile_core::Rect {
            x: spec.canvas_origin_x,
            y: spec.canvas_origin_y,
            width: spec.canvas_width as f32,
            height: spec.canvas_height as f32,
        };

        for (element_index, element) in elements.into_iter().enumerate() {
            if !element.visible() {
                continue;
            }
            match element {
                crate::elements::RenderElement::Text(_) => {
                    if text_atlases.is_empty() {
                        return Err("the card draws text but no text atlas is installed".into());
                    }
                    aggregate.sdf_text_element_count =
                        aggregate.sdf_text_element_count.saturating_add(1);
                    pending.select(
                        pass.executor,
                        occlusion_dry_run
                            .as_ref()
                            .and_then(|dry_run| dry_run.masks_by_element[element_index].clone()),
                        run_context,
                        rgba.as_mut_slice(),
                        &mut aggregate,
                    )?;
                    let capture_started = std::time::Instant::now();
                    let mut failure = None;
                    let mut observer = |result: Result<
                        crate::text::ResolvedTextSdfGlyph,
                        crate::text::TextSdfCaptureError,
                    >| match result {
                        Ok(glyph) => {
                            aggregate.captured_text_count =
                                aggregate.captured_text_count.saturating_add(1);
                            pending.primitives.push(CapturedSdfPrimitive::Text(glyph));
                        }
                        Err(error) => failure = Some(format!("Text: {error:?}")),
                    };
                    let observation_timings = crate::elements::capture_element_sdf(
                        &element,
                        md,
                        assets,
                        canvas,
                        Some(text_atlases),
                        SdfCaptureObserver::Text(&mut observer),
                    );
                    aggregate.timings.capture_rich_parse_ns = aggregate
                        .timings
                        .capture_rich_parse_ns
                        .saturating_add(observation_timings.text_capture.rich_parse_ns);
                    aggregate.timings.capture_font_resolve_ns = aggregate
                        .timings
                        .capture_font_resolve_ns
                        .saturating_add(observation_timings.text_capture.font_resolve_ns);
                    aggregate.timings.capture_layout_setup_ns = aggregate
                        .timings
                        .capture_layout_setup_ns
                        .saturating_add(observation_timings.text_capture.layout_setup_ns);
                    aggregate.timings.capture_measure_ns = aggregate
                        .timings
                        .capture_measure_ns
                        .saturating_add(observation_timings.text_capture.measure_ns);
                    aggregate.timings.capture_command_build_ns = aggregate
                        .timings
                        .capture_command_build_ns
                        .saturating_add(observation_timings.text_capture.command_build_ns);
                    aggregate.timings.capture_emit_ns = aggregate
                        .timings
                        .capture_emit_ns
                        .saturating_add(observation_timings.text_capture.emit_ns);
                    aggregate.timings.capture_ns = aggregate
                        .timings
                        .capture_ns
                        .saturating_add(elapsed_ns(capture_started));
                    if let Some(error) = failure {
                        return Err(error);
                    }
                }
                crate::elements::RenderElement::Shape(_) => {
                    if shape_atlas.is_none() {
                        return Err("the card draws shapes but no shape atlas is installed".into());
                    }
                    aggregate.sdf_shape_element_count =
                        aggregate.sdf_shape_element_count.saturating_add(1);
                    pending.select(
                        pass.executor,
                        occlusion_dry_run
                            .as_ref()
                            .and_then(|dry_run| dry_run.masks_by_element[element_index].clone()),
                        run_context,
                        rgba.as_mut_slice(),
                        &mut aggregate,
                    )?;
                    let capture_started = std::time::Instant::now();
                    let mut failure = None;
                    let mut observer = |result: Result<
                        crate::elements::shape::ResolvedShapeSdfCommand,
                        crate::elements::shape::ShapeSdfCaptureError,
                    >| match result {
                        Ok(shape) => {
                            pending.primitives.push(CapturedSdfPrimitive::Shape(shape));
                        }
                        Err(error) => failure = Some(format!("Shape: {error}")),
                    };
                    crate::elements::capture_element_sdf(
                        &element,
                        md,
                        assets,
                        canvas,
                        Some(text_atlases),
                        SdfCaptureObserver::Shape(&mut observer),
                    );
                    aggregate.timings.capture_ns = aggregate
                        .timings
                        .capture_ns
                        .saturating_add(elapsed_ns(capture_started));
                    if let Some(error) = failure {
                        return Err(error);
                    }
                }
                _ => {
                    pending.flush(run_context, rgba.as_mut_slice(), &mut aggregate)?;
                    let (kind, index) = render_element_authored_identity(card, &element)
                        .ok_or_else(|| {
                            format!(
                                "the {:?} element is not part of the card",
                                profile_command_kind(&element)
                            )
                        })?;
                    let draw_started = std::time::Instant::now();
                    let stats = render_authored_image_into_pixels(
                        rgba.as_mut_slice(),
                        semantic_scene,
                        render_object_store,
                        Some(text_atlases),
                        md,
                        assets,
                        kind,
                        index,
                        spec.surface_width,
                        spec.surface_height,
                        spec.canvas_origin_x,
                        spec.canvas_origin_y,
                        // The image compositor follows the page's executor
                        // choice so a scalar host runs a scalar pipeline.
                        pass.executor,
                    )?;
                    aggregate.software_image_count =
                        aggregate.software_image_count.saturating_add(1);
                    aggregate.software_text_command_count = aggregate
                        .software_text_command_count
                        .saturating_add(stats.text_command_count);
                    aggregate.software_shape_command_count = aggregate
                        .software_shape_command_count
                        .saturating_add(stats.shape_command_count);
                    aggregate.software_skipped_text_command_count = aggregate
                        .software_skipped_text_command_count
                        .saturating_add(stats.skipped_text_command_count);
                    aggregate.software_skipped_shape_command_count = aggregate
                        .software_skipped_shape_command_count
                        .saturating_add(stats.skipped_shape_command_count);
                    aggregate.image_fragment_count = aggregate
                        .image_fragment_count
                        .saturating_add(stats.blended_fragment_count);
                    aggregate.image_simd_packet_count = aggregate
                        .image_simd_packet_count
                        .saturating_add(stats.simd_packet_count);
                    aggregate.general_base_hit_count = aggregate
                        .general_base_hit_count
                        .saturating_add(stats.general_base_hit_count);
                    aggregate.general_base_miss_count = aggregate
                        .general_base_miss_count
                        .saturating_add(stats.general_base_miss_count);
                    aggregate.general_base_baked_command_count = aggregate
                        .general_base_baked_command_count
                        .saturating_add(stats.general_base_baked_command_count);
                    aggregate.general_base_overlay_command_count = aggregate
                        .general_base_overlay_command_count
                        .saturating_add(stats.general_base_overlay_command_count);
                    aggregate.general_base_bytes = aggregate
                        .general_base_bytes
                        .saturating_add(stats.general_base_bytes);
                    aggregate.general_base_avoided_source_bytes = aggregate
                        .general_base_avoided_source_bytes
                        .saturating_add(stats.general_base_avoided_source_bytes);
                    aggregate.deck_art_variant_hit_count = aggregate
                        .deck_art_variant_hit_count
                        .saturating_add(stats.deck_art_variant_hit_count);
                    aggregate.deck_art_variant_miss_count = aggregate
                        .deck_art_variant_miss_count
                        .saturating_add(stats.deck_art_variant_miss_count);
                    aggregate.deck_art_variant_bytes = aggregate
                        .deck_art_variant_bytes
                        .saturating_add(stats.deck_art_variant_bytes);
                    aggregate.deck_art_variant_avoided_source_bytes = aggregate
                        .deck_art_variant_avoided_source_bytes
                        .saturating_add(stats.deck_art_variant_avoided_source_bytes);
                    aggregate.timings.general_base_composite_ns = aggregate
                        .timings
                        .general_base_composite_ns
                        .saturating_add(stats.general_base_composite_ns);
                    aggregate.timings.image_composite_ns = aggregate
                        .timings
                        .image_composite_ns
                        .saturating_add(elapsed_ns(draw_started));
                }
            }
        }
        pending.flush(run_context, rgba.as_mut_slice(), &mut aggregate)?;

        let snapshot_started = std::time::Instant::now();
        if rgba.len() != row_bytes * height as usize {
            return Err(format!(
                "ordered SDF raster surface length mismatch: expected {}, got {}",
                row_bytes * height as usize,
                rgba.len()
            ));
        }
        aggregate.timings.rgba_snapshot_ns = elapsed_ns(snapshot_started);
        aggregate.timings.total_ns = elapsed_ns(total_started);
        Ok(SurfaceOutput {
            rgba,
            width,
            height,
            sdf_run_count: aggregate.sdf_run_count,
            software_image_count: aggregate.software_image_count,
            software_text_command_count: aggregate.software_text_command_count,
            software_shape_command_count: aggregate.software_shape_command_count,
            software_skipped_text_command_count: aggregate.software_skipped_text_command_count,
            software_skipped_shape_command_count: aggregate.software_skipped_shape_command_count,
            image_fragment_count: aggregate.image_fragment_count,
            image_simd_packet_count: aggregate.image_simd_packet_count,
            general_base_hit_count: aggregate.general_base_hit_count,
            general_base_miss_count: aggregate.general_base_miss_count,
            general_base_baked_command_count: aggregate.general_base_baked_command_count,
            general_base_overlay_command_count: aggregate.general_base_overlay_command_count,
            general_base_bytes: aggregate.general_base_bytes,
            general_base_avoided_source_bytes: aggregate.general_base_avoided_source_bytes,
            deck_art_variant_hit_count: aggregate.deck_art_variant_hit_count,
            deck_art_variant_miss_count: aggregate.deck_art_variant_miss_count,
            deck_art_variant_bytes: aggregate.deck_art_variant_bytes,
            deck_art_variant_avoided_source_bytes: aggregate.deck_art_variant_avoided_source_bytes,
            render_object_mapped_bytes: aggregate.render_object_mapped_bytes,
            sdf_text_element_count: aggregate.sdf_text_element_count,
            sdf_shape_element_count: aggregate.sdf_shape_element_count,
            captured_text_count: aggregate.captured_text_count,
            realtime_edt_glyphs: aggregate.realtime_edt_glyphs,
            realtime_edt_batch: aggregate.realtime_edt_batch,
            plan_stats: aggregate.plan_stats,
            execution_stats: aggregate.execution_stats,
            atlas_mapped_bytes: text_atlases
                .mapped_bytes()
                .saturating_add(shape_atlas.map_or(
                    0,
                    crate::sdf::shape_atlas::MappedShapeSdfAtlas::mapped_bytes,
                )),
            plan_bytes: aggregate.plan_bytes,
            span_bytes: aggregate.span_bytes,
            occlusion: aggregate.occlusion,
            occlusion_eligible_image_count: aggregate.occlusion_eligible_image_count,
            occlusion_mask_snapshot_count: aggregate.occlusion_mask_snapshot_count,
            occlusion_mask_bytes: aggregate.occlusion_mask_bytes,
            timings: aggregate.timings,
        })
    }
}

/// One drawn surface: premultiplied RGBA8 pixels and what drawing them took.
#[derive(Default)]
pub(crate) struct SurfaceOutput {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub sdf_run_count: u64,
    pub software_image_count: u64,
    pub software_text_command_count: u64,
    pub software_shape_command_count: u64,
    pub software_skipped_text_command_count: u64,
    pub software_skipped_shape_command_count: u64,
    pub image_fragment_count: u64,
    pub image_simd_packet_count: u64,
    pub general_base_hit_count: u64,
    pub general_base_miss_count: u64,
    pub general_base_baked_command_count: u64,
    pub general_base_overlay_command_count: u64,
    pub general_base_bytes: u64,
    pub general_base_avoided_source_bytes: u64,
    pub deck_art_variant_hit_count: u64,
    pub deck_art_variant_miss_count: u64,
    pub deck_art_variant_bytes: u64,
    pub deck_art_variant_avoided_source_bytes: u64,
    pub render_object_mapped_bytes: u64,
    pub sdf_text_element_count: u64,
    pub sdf_shape_element_count: u64,
    pub captured_text_count: u64,
    pub realtime_edt_glyphs: Vec<crate::telemetry::RealtimeEdtGlyphTelemetry>,
    pub realtime_edt_batch: crate::telemetry::RealtimeEdtBatchTelemetry,
    pub plan_stats: crate::sdf::tile::SdfPlanStats,
    pub execution_stats: crate::sdf::tile::SdfExecutionStats,
    pub atlas_mapped_bytes: u64,
    pub plan_bytes: u64,
    pub span_bytes: u64,
    pub occlusion: crate::sdf::tile::SdfOcclusionStats,
    pub occlusion_eligible_image_count: u64,
    pub occlusion_mask_snapshot_count: u64,
    pub occlusion_mask_bytes: u64,
    pub timings: SurfaceTimings,
}

/// The executor and tiling an ordered SDF surface is drawn with.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SdfPass {
    pub(crate) executor: Executor,
    pub(crate) tile_width: u16,
    pub(crate) tile_height: u16,
    pub(crate) pixel_occlusion: PixelOcclusion,
}

impl SdfPass {
    pub(crate) fn new(config: &RenderConfig, executor: Executor) -> Self {
        Self {
            executor,
            tile_width: config.tile_width,
            tile_height: config.tile_height,
            pixel_occlusion: config.pixel_occlusion,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct OrderedSdfSurfaceSpec {
    pub(super) surface_width: u32,
    pub(super) surface_height: u32,
    pub(super) canvas_width: u32,
    pub(super) canvas_height: u32,
    pub(super) canvas_origin_x: f32,
    pub(super) canvas_origin_y: f32,
    pub(super) clear_rgba: [u8; 4],
    pub(super) prepare_direct_axis_shape: bool,
}

impl OrderedSdfSurfaceSpec {
    fn full_card(clear_rgba: [u8; 4]) -> Self {
        Self {
            surface_width: crate::transform::CANVAS_WIDTH as u32,
            surface_height: crate::transform::CANVAS_HEIGHT as u32,
            canvas_width: crate::transform::CANVAS_WIDTH as u32,
            canvas_height: crate::transform::CANVAS_HEIGHT as u32,
            canvas_origin_x: 0.0,
            canvas_origin_y: 0.0,
            clear_rgba,
            prepare_direct_axis_shape: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct SurfaceTimings {
    #[serde(default)]
    pub semantic_resolve_ns: u64,
    pub surface_create_ns: u64,
    pub surface_clear_ns: u64,
    pub capture_ns: u64,
    pub capture_rich_parse_ns: u64,
    pub capture_font_resolve_ns: u64,
    pub capture_layout_setup_ns: u64,
    pub capture_measure_ns: u64,
    pub capture_command_build_ns: u64,
    pub capture_emit_ns: u64,
    pub command_mapping_ns: u64,
    pub plan_build_ns: u64,
    pub execute_ns: u64,
    pub image_composite_ns: u64,
    pub general_base_composite_ns: u64,
    pub occlusion_mask_build_ns: u64,
    pub occlusion_intersection_ns: u64,
    pub rgba_snapshot_ns: u64,
    /// Dynamic-layer post-processing: scan the executor-owned RGBA buffer for
    /// its non-transparent bounds.
    #[serde(default)]
    pub postprocess_bounds_ns: u64,
    /// Dynamic-layer post-processing: copy the tight RGBA rows and construct
    /// the final cropped raster image.
    #[serde(default)]
    pub postprocess_crop_ns: u64,
    pub total_ns: u64,
}

#[derive(Default)]
struct SurfaceAggregate {
    prepare_direct_axis_shape: bool,
    realtime_oversized_glyph_generation: bool,
    sdf_run_count: u64,
    software_image_count: u64,
    software_text_command_count: u64,
    software_shape_command_count: u64,
    software_skipped_text_command_count: u64,
    software_skipped_shape_command_count: u64,
    image_fragment_count: u64,
    image_simd_packet_count: u64,
    general_base_hit_count: u64,
    general_base_miss_count: u64,
    general_base_baked_command_count: u64,
    general_base_overlay_command_count: u64,
    general_base_bytes: u64,
    general_base_avoided_source_bytes: u64,
    deck_art_variant_hit_count: u64,
    deck_art_variant_miss_count: u64,
    deck_art_variant_bytes: u64,
    deck_art_variant_avoided_source_bytes: u64,
    render_object_mapped_bytes: u64,
    sdf_text_element_count: u64,
    sdf_shape_element_count: u64,
    captured_text_count: u64,
    realtime_edt_glyphs: Vec<crate::telemetry::RealtimeEdtGlyphTelemetry>,
    realtime_edt_batch: crate::telemetry::RealtimeEdtBatchTelemetry,
    plan_stats: crate::sdf::tile::SdfPlanStats,
    execution_stats: crate::sdf::tile::SdfExecutionStats,
    plan_bytes: u64,
    span_bytes: u64,
    occlusion: crate::sdf::tile::SdfOcclusionStats,
    occlusion_eligible_image_count: u64,
    occlusion_mask_snapshot_count: u64,
    occlusion_mask_bytes: u64,
    timings: SurfaceTimings,
}

/// Fills a premultiplied RGBA8 buffer with a clear colour, matching the
/// backend clear it replaces: the alpha endpoints — the only clear colours the
/// profile paths pass — premultiply exactly, and any other alpha follows the
/// engine's premultiply rounding.
fn fill_premultiplied_clear(pixels: &mut [u8], clear: [u8; 4]) {
    let alpha = clear[3];
    let premultiplied = [
        crate::codec::premultiply_channel(clear[0], alpha),
        crate::codec::premultiply_channel(clear[1], alpha),
        crate::codec::premultiply_channel(clear[2], alpha),
        alpha,
    ];
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.copy_from_slice(&premultiplied);
    }
}

/// What every SDF run of one surface is drawn with: the atlases, the tile grid
/// over the surface, the cache of shape row programs, and whether fragments
/// under the run's occlusion mask are skipped rather than only counted.
#[derive(Clone, Copy)]
struct SdfRunContext<'a> {
    atlases: SdfAtlases<'a>,
    grid: crate::sdf::tile::TileGrid,
    shape_program_cache: &'a crate::sdf::tile::ShapeRowProgramCache,
    execute_occlusion: bool,
}

/// The text and shape primitives captured since the last run was drawn, with
/// the executor and occlusion mask they are drawn with. Consecutive SDF
/// elements on one executor share a run.
#[derive(Default)]
struct PendingSdfRun {
    primitives: Vec<CapturedSdfPrimitive>,
    executor: Option<Executor>,
    occlusion_mask: Option<Arc<crate::sdf::tile::PixelOcclusionMask>>,
}

impl PendingSdfRun {
    /// Continues the run on `executor`, drawing the pending primitives first
    /// when the run uses another executor. A run keeps the first occlusion
    /// mask it is given.
    fn select(
        &mut self,
        executor: Executor,
        occlusion_mask: Option<Arc<crate::sdf::tile::PixelOcclusionMask>>,
        context: SdfRunContext<'_>,
        pixels: &mut [u8],
        aggregate: &mut SurfaceAggregate,
    ) -> Result<(), String> {
        if self.executor.is_some_and(|active| active != executor) {
            self.flush(context, pixels, aggregate)?;
        }
        self.executor = Some(executor);
        if self.occlusion_mask.is_none() {
            self.occlusion_mask = occlusion_mask;
        }
        Ok(())
    }

    /// Draws the pending primitives into `pixels` and ends the run.
    fn flush(
        &mut self,
        context: SdfRunContext<'_>,
        pixels: &mut [u8],
        aggregate: &mut SurfaceAggregate,
    ) -> Result<(), String> {
        let Some(executor) = self.executor.take() else {
            debug_assert!(self.primitives.is_empty());
            self.occlusion_mask = None;
            return Ok(());
        };
        let occlusion_mask = self.occlusion_mask.take();
        if self.primitives.is_empty() {
            return Ok(());
        }
        flush_full_card_sdf_run(
            &mut self.primitives,
            executor,
            occlusion_mask.as_deref(),
            context,
            pixels,
            aggregate,
        )
    }
}

fn flush_full_card_sdf_run(
    pending: &mut Vec<CapturedSdfPrimitive>,
    executor: Executor,
    occlusion_mask: Option<&crate::sdf::tile::PixelOcclusionMask>,
    context: SdfRunContext<'_>,
    pixels: &mut [u8],
    aggregate: &mut SurfaceAggregate,
) -> Result<(), String> {
    if pending.is_empty() {
        return Ok(());
    }
    let SdfRunContext {
        atlases,
        grid,
        shape_program_cache,
        execute_occlusion,
    } = context;

    let mapping_started = std::time::Instant::now();
    let mut runtime_text_pages = Vec::new();
    let prepared_batch = active_realtime_edt_batch();
    let runtime_pages = aggregate
        .realtime_oversized_glyph_generation
        .then_some(&mut runtime_text_pages);
    let commands = map_captured_sdf_commands(
        pending,
        atlases,
        runtime_pages,
        Some(&mut aggregate.realtime_edt_glyphs),
        Some(&mut aggregate.realtime_edt_batch),
        None,
    )
    .map_err(|error| format!("full-card SDF command mapping: {error}"))?;
    let runtime_pages = prepared_batch
        .as_deref()
        .map(|batch| batch.pages.as_slice())
        .unwrap_or(runtime_text_pages.as_slice());
    let execution_source = crate::sdf::tile::MixedSdfAtlasSource::with_runtime_text(
        atlases.text,
        atlases.shape,
        runtime_pages,
    )
    .map_err(|error| format!("full-card SDF runtime source: {error}"))?;
    aggregate.timings.command_mapping_ns = aggregate
        .timings
        .command_mapping_ns
        .saturating_add(elapsed_ns(mapping_started));

    let plan_started = std::time::Instant::now();
    let plan = if aggregate.prepare_direct_axis_shape {
        crate::sdf::tile::SdfTilePlan::build_static_one_shot_with_shape_program_cache(
            grid,
            &commands,
            &execution_source,
            shape_program_cache,
        )
    } else {
        crate::sdf::tile::SdfTilePlan::build_for_one_shot_dynamic_layer(
            grid,
            &commands,
            &execution_source,
        )
    }
    .map_err(|error| format!("full-card SDF plan build: {error}"))?;
    aggregate.timings.plan_build_ns = aggregate
        .timings
        .plan_build_ns
        .saturating_add(elapsed_ns(plan_started));

    let mut visible_plan = None;
    if let Some(mask) = occlusion_mask {
        let intersection_started = std::time::Instant::now();
        let measured = if execute_occlusion {
            let (filtered, measured) = plan
                .visible_plan(mask)
                .map_err(|error| format!("full-card SDF visible plan: {error}"))?;
            visible_plan = Some(filtered);
            measured
        } else {
            plan.measure_occlusion(mask)
                .map_err(|error| format!("full-card SDF occlusion measure: {error}"))?
        };
        aggregate.timings.occlusion_intersection_ns = aggregate
            .timings
            .occlusion_intersection_ns
            .saturating_add(elapsed_ns(intersection_started));
        add_sdf_occlusion_stats(&mut aggregate.occlusion, measured);
    }

    let expected_len = grid.canvas_width as usize * grid.canvas_height as usize * 4;
    if pixels.len() != expected_len {
        return Err(format!(
            "ordered SDF compositor requires a tight RGBA8888 premul buffer of {expected_len} bytes, got {}",
            pixels.len()
        ));
    }
    let execution_plan = visible_plan.as_ref().unwrap_or(&plan);
    let execute_started = std::time::Instant::now();
    let execution = match executor {
        Executor::Scalar => execution_plan
            .execute_scalar_f32_over(&execution_source, pixels)
            .map_err(|error| format!("full-card scalar SDF execute: {error}"))?,
        Executor::Simd => execution_plan
            .execute_simd_f32_over(&execution_source, pixels)
            .map_err(|error| format!("full-card SIMD SDF execute: {error}"))?,
    };
    aggregate.timings.execute_ns = aggregate
        .timings
        .execute_ns
        .saturating_add(elapsed_ns(execute_started));
    add_sdf_plan_stats(&mut aggregate.plan_stats, plan.stats());
    add_sdf_execution_stats(&mut aggregate.execution_stats, execution);
    aggregate.plan_bytes = aggregate.plan_bytes.saturating_add(plan.resident_bytes());
    aggregate.span_bytes = aggregate.span_bytes.saturating_add(plan.span_bytes());
    if let Some(visible_plan) = visible_plan.as_ref() {
        aggregate.plan_bytes = aggregate
            .plan_bytes
            .saturating_add(visible_plan.resident_bytes());
        aggregate.span_bytes = aggregate
            .span_bytes
            .saturating_add(visible_plan.span_bytes());
    }
    aggregate.sdf_run_count = aggregate.sdf_run_count.saturating_add(1);
    pending.clear();
    Ok(())
}

fn add_sdf_plan_stats(
    total: &mut crate::sdf::tile::SdfPlanStats,
    value: crate::sdf::tile::SdfPlanStats,
) {
    total.command_count = total.command_count.saturating_add(value.command_count);
    total.text_command_count = total
        .text_command_count
        .saturating_add(value.text_command_count);
    total.shape_command_count = total
        .shape_command_count
        .saturating_add(value.shape_command_count);
    total.span_count = total.span_count.saturating_add(value.span_count);
    total.text_span_count = total.text_span_count.saturating_add(value.text_span_count);
    total.shape_span_count = total
        .shape_span_count
        .saturating_add(value.shape_span_count);
    total.tile_count = total.tile_count.saturating_add(value.tile_count);
    total.nonempty_tile_count = total
        .nonempty_tile_count
        .saturating_add(value.nonempty_tile_count);
    total.covered_fragment_count = total
        .covered_fragment_count
        .saturating_add(value.covered_fragment_count);
    total.text_covered_fragment_count = total
        .text_covered_fragment_count
        .saturating_add(value.text_covered_fragment_count);
    total.shape_covered_fragment_count = total
        .shape_covered_fragment_count
        .saturating_add(value.shape_covered_fragment_count);
}

fn add_sdf_execution_stats(
    total: &mut crate::sdf::tile::SdfExecutionStats,
    value: crate::sdf::tile::SdfExecutionStats,
) {
    total.shaded_fragment_count = total
        .shaded_fragment_count
        .saturating_add(value.shaded_fragment_count);
    total.text_shaded_fragment_count = total
        .text_shaded_fragment_count
        .saturating_add(value.text_shaded_fragment_count);
    total.shape_shaded_fragment_count = total
        .shape_shaded_fragment_count
        .saturating_add(value.shape_shaded_fragment_count);
    total.sampled_texel_count = total
        .sampled_texel_count
        .saturating_add(value.sampled_texel_count);
    total.blended_fragment_count = total
        .blended_fragment_count
        .saturating_add(value.blended_fragment_count);
    total.text_blended_fragment_count = total
        .text_blended_fragment_count
        .saturating_add(value.text_blended_fragment_count);
    total.shape_blended_fragment_count = total
        .shape_blended_fragment_count
        .saturating_add(value.shape_blended_fragment_count);
    total.simd_packet_count = total
        .simd_packet_count
        .saturating_add(value.simd_packet_count);
    total.swizzled_packet_count = total
        .swizzled_packet_count
        .saturating_add(value.swizzled_packet_count);
    total.gather_fallback_packet_count = total
        .gather_fallback_packet_count
        .saturating_add(value.gather_fallback_packet_count);
    total.precomputed_shape_fragment_count = total
        .precomputed_shape_fragment_count
        .saturating_add(value.precomputed_shape_fragment_count);
    total.precomputed_shape_span_count = total
        .precomputed_shape_span_count
        .saturating_add(value.precomputed_shape_span_count);
    total.direct_output_run_count = total
        .direct_output_run_count
        .saturating_add(value.direct_output_run_count);
    total.direct_output_packet_count = total
        .direct_output_packet_count
        .saturating_add(value.direct_output_packet_count);
}
