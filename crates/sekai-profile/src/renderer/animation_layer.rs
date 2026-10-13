//! Animation layers: element layers rasterized through the ordered SDF path and
//! cropped to their opaque bounds, or kept as commands when too large to retain.

use super::capture::{capture_sdf_primitives, map_captured_sdf_commands, SdfCaptureKinds};
use super::realtime_edt::active_realtime_edt_batch;
use super::surface::{OrderedSdfSurfaceSpec, SdfPass, SurfaceOutput};
use super::{elapsed_ns, CustomProfileRenderer, RenderObjectGenerationPin};
use crate::assets::AssetStore;
use crate::masterdata::MasterData;
use crate::render_config::{sdf_simd_available, Executor, PixelOcclusion, RenderConfig};
use crate::types::CustomProfileCard;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum LayerDynamic {
    TmpLineIndent {
        fps: u32,
        #[serde(rename = "loop")]
        looped: bool,
        frames: Vec<LayerDynamicFrame>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LayerDynamicFrame {
    frame: u32,
    dx: f32,
    dy: f32,
}

/// What a rasterized animation layer carries: either the pixels themselves,
/// or the commands needed to produce them on demand.
pub(crate) enum AnimationLayerRaster {
    /// Pixels held for the whole export.
    Ordered(OrderedLayerRaster),
    /// Commands held instead of pixels, for a layer whose surface exceeds the
    /// bytes an export may retain. Windows of it are rasterized per frame.
    DeferredSdf(DeferredAnimationSdfLayer),
}

/// A rasterized animation layer together with the execution telemetry of the
/// SDF run that produced it.
///
/// `execution` is `None` when no run took place: the layer covered no pixels,
/// or its commands were retained in place of its pixels.
pub(crate) struct CroppedLayerRaster {
    pub raster: AnimationLayerRaster,
    pub execution: Option<SurfaceOutput>,
}

/// An animation layer kept as commands rather than pixels.
///
/// The commands are in canvas coordinates — un-shifted — so a window of any
/// size can be rasterized from them by translating into that window's origin.
pub(crate) struct DeferredAnimationSdfLayer {
    pub(crate) commands: Vec<crate::sdf::tile::SdfDrawCommand>,
    pub(crate) text_atlases: std::sync::Arc<crate::sdf::atlas::MappedSdfAtlasSet>,
    pub(crate) shape_atlas: Option<std::sync::Arc<crate::sdf::shape_atlas::MappedShapeSdfAtlas>>,
    pub(crate) runtime_text: Vec<crate::sdf::tile::RuntimeTextSdfPage>,
    pub(crate) executor: Executor,
    pub(crate) tile_width: u16,
    pub(crate) tile_height: u16,
    /// Layer bounds in canvas coordinates.
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// One rasterized window of a deferred SDF animation layer: the pixels of a
/// source-space sub-rectangle, regenerated from the retained commands.
pub(crate) struct RenderedDeferredAnimationWindow {
    /// Tight premultiplied RGBA8, `width * 4` bytes per row.
    pub(crate) pixels: Vec<u8>,
    /// Window origin in canvas coordinates.
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Peak transient bytes: the window surface plus the cropped copy.
    pub(crate) scratch_peak_bytes: usize,
}

/// Premultiplied RGBA8 layer raster produced by the ordered SDF path,
/// cropped to the tight non-transparent bounds.
pub(crate) struct OrderedLayerRaster {
    /// Tight premultiplied RGBA8, `width * 4` bytes per row.
    pub pixels: Vec<u8>,
    /// Crop offset in canvas coordinates (any dynamic expansion removed).
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Peak transient bytes: the full surface plus the cropped copy.
    pub scratch_peak_bytes: usize,
}

impl CustomProfileRenderer {
    /// Reports whether a card animates, without rendering it: builds the
    /// scene the export would render and preflights its compiled programs up
    /// to `maximum_tick`.
    pub fn animation_preflight(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        document_key: &str,
        region: &str,
        maximum_tick: u64,
    ) -> Result<sekai_profile_core::AnimationPreflight, String> {
        let md = self.snapshot();
        let scene = crate::core_shadow::build_scene(
            card,
            &md,
            document_key,
            region,
            profile,
            region,
            self.assets.as_deref(),
        )?;
        scene
            .animation_preflight(maximum_tick)
            .map_err(|error| error.to_string())
    }

    /// Renders a card's animation export: preflights the compiled scene,
    /// rasterizes each element layer through the ordered SDF path, composites
    /// the frames, and encodes them with the preset's format. Static cards
    /// return `animated: false` without an artifact.
    ///
    /// Images the render-object store lacks, or every image when no store is
    /// installed, are decoded from the asset store.
    pub fn render_animation(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        document_key: &str,
        region: &str,
        preset: &crate::animation::ResolvedAnimationPreset,
        config: &RenderConfig,
    ) -> Result<crate::animation::ProfileAnimationExport, String> {
        let generation = self.pin_render_object_generation();
        self.render_animation_pinned(
            card,
            profile,
            document_key,
            region,
            preset,
            config,
            &generation,
        )
    }

    /// [`Self::render_animation`] against the render objects `generation`
    /// pins.
    #[allow(clippy::too_many_arguments)]
    pub fn render_animation_pinned(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        document_key: &str,
        region: &str,
        preset: &crate::animation::ResolvedAnimationPreset,
        config: &RenderConfig,
        generation: &RenderObjectGenerationPin,
    ) -> Result<crate::animation::ProfileAnimationExport, String> {
        let md = self.snapshot();
        crate::animation::export_profile_animation(
            self,
            card,
            profile,
            document_key,
            region,
            &md,
            self.assets.as_deref(),
            preset,
            config,
            generation
                .store()
                .unwrap_or_else(|| crate::render_object::MappedRenderObjectStore::empty()),
        )
    }

    /// Resolves the executor an animation export runs on.
    pub(crate) fn resolve_executor(&self, config: &RenderConfig) -> Result<Executor, String> {
        config
            .resolve(sdf_simd_available())
            .map_err(|error| error.to_string())
    }

    /// Renders one element layer through the ordered SDF path — the same
    /// pipeline full pages use — onto a transparent surface expanded for the
    /// layer's dynamic travel, then crops to the tight non-transparent bounds.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_ordered_element_layer_cropped(
        &self,
        card: &CustomProfileCard,
        md: &MasterData,
        assets: Option<&AssetStore>,
        profile: Option<&crate::profile::ProfileData>,
        executor: Executor,
        include_dynamic_bounds: bool,
        tile_width: u16,
        tile_height: u16,
        render_object_store: &crate::render_object::MappedRenderObjectStore,
        maximum_retained_layer_bytes: usize,
    ) -> Result<CroppedLayerRaster, String> {
        let w = crate::transform::CANVAS_WIDTH as i32;
        let h = crate::transform::CANVAS_HEIGHT as i32;
        let elements = crate::elements::flatten_and_sort(card);
        let dynamic = if include_dynamic_bounds {
            layer_dynamic_for_elements(&elements, md)
        } else {
            None
        };
        let expansion = dynamic
            .as_ref()
            .map(dynamic_canvas_expansion)
            .unwrap_or_default();
        if layer_draws_only_sdf(&elements) {
            return self.render_animation_sdf_layer_tight(
                card,
                md,
                assets,
                profile,
                executor,
                tile_width,
                tile_height,
                expansion,
                maximum_retained_layer_bytes,
            );
        }
        let surface_w = w + expansion.left + expansion.right;
        let surface_h = h + expansion.top + expansion.bottom;
        let spec = OrderedSdfSurfaceSpec {
            surface_width: u32::try_from(surface_w)
                .map_err(|_| "layer surface width overflow".to_string())?,
            surface_height: u32::try_from(surface_h)
                .map_err(|_| "layer surface height overflow".to_string())?,
            canvas_width: w as u32,
            canvas_height: h as u32,
            canvas_origin_x: expansion.left as f32,
            canvas_origin_y: expansion.top as f32,
            clear_rgba: [0, 0, 0, 0],
            prepare_direct_axis_shape: false,
        };
        let pass = SdfPass {
            executor,
            tile_width,
            tile_height,
            pixel_occlusion: PixelOcclusion::Off,
        };
        let mut output = self.render_ordered_sdf_surface(
            card,
            profile,
            pass,
            spec,
            md,
            assets,
            None,
            render_object_store,
        )?;
        let rgba = std::mem::take(&mut output.rgba);
        let row_bytes = surface_w as usize * 4;
        let surface_bytes = row_bytes * surface_h as usize;
        let bounds_started = std::time::Instant::now();
        let (bx, by, bw, bh) = opaque_bounds_for_pixels(&rgba, surface_w, surface_h, row_bytes)?;
        output.timings.postprocess_bounds_ns = elapsed_ns(bounds_started);
        let crop_started = std::time::Instant::now();
        let raster = if bw == 0 || bh == 0 {
            OrderedLayerRaster {
                pixels: Vec::new(),
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                scratch_peak_bytes: surface_bytes,
            }
        } else {
            OrderedLayerRaster {
                pixels: crop_pixels_lossless(
                    &rgba,
                    row_bytes,
                    surface_w as u32,
                    surface_h as u32,
                    bx,
                    by,
                    bw,
                    bh,
                )?,
                x: bx as i32 - expansion.left,
                y: by as i32 - expansion.top,
                width: bw,
                height: bh,
                scratch_peak_bytes: surface_bytes.saturating_add(bw as usize * bh as usize * 4),
            }
        };
        output.timings.postprocess_crop_ns = elapsed_ns(crop_started);
        output.timings.total_ns = output
            .timings
            .total_ns
            .saturating_add(output.timings.postprocess_bounds_ns)
            .saturating_add(output.timings.postprocess_crop_ns);
        let raster = AnimationLayerRaster::Ordered(raster);
        self.recycle_profile_rgba_scratch(rgba);
        Ok(CroppedLayerRaster {
            raster,
            execution: Some(output),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn render_animation_sdf_layer_tight(
        &self,
        card: &CustomProfileCard,
        md: &MasterData,
        assets: Option<&AssetStore>,
        profile: Option<&crate::profile::ProfileData>,
        executor: Executor,
        tile_width: u16,
        tile_height: u16,
        reachable: CanvasExpansion,
        maximum_retained_layer_bytes: usize,
    ) -> Result<CroppedLayerRaster, String> {
        let total_started = std::time::Instant::now();
        let sdf_atlases = self.sdf_atlases.load_full();
        let capture_started = std::time::Instant::now();
        let captured = capture_sdf_primitives(
            card,
            md,
            assets,
            Some(&sdf_atlases),
            profile,
            crate::transform::CANVAS_WIDTH as u32,
            crate::transform::CANVAS_HEIGHT as u32,
            SdfCaptureKinds::TEXT_AND_SHAPE,
        )?;
        let capture_ns = elapsed_ns(capture_started);

        let base_source = crate::sdf::tile::MixedSdfAtlasSource::new(
            &sdf_atlases,
            self.shape_sdf_atlas.as_deref(),
        )
        .map_err(|error| error.to_string())?;

        let mut runtime_text_pages = Vec::new();
        let mut realtime_edt_glyphs = Vec::new();
        let mut realtime_edt_batch = crate::telemetry::RealtimeEdtBatchTelemetry::default();
        let prepared_batch = active_realtime_edt_batch();
        let runtime_pages = self
            .realtime_oversized_glyph_generation
            .then_some(&mut runtime_text_pages);
        let mapping_started = std::time::Instant::now();
        let mut commands = map_captured_sdf_commands(
            &captured.primitives,
            &sdf_atlases,
            self.shape_sdf_atlas.as_deref(),
            &base_source,
            runtime_pages,
            Some(&mut realtime_edt_glyphs),
            Some(&mut realtime_edt_batch),
            None,
        )?;
        let command_mapping_ns = elapsed_ns(mapping_started);

        let empty = CroppedLayerRaster {
            raster: AnimationLayerRaster::Ordered(OrderedLayerRaster {
                pixels: Vec::new(),
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                scratch_peak_bytes: 0,
            }),
            execution: None,
        };
        let Some((content_x, content_y, content_w, content_h)) =
            tight_sdf_command_bounds(&commands)
        else {
            return Ok(empty);
        };
        // Keep only the part of the content a frame can ever show. The layer's
        // travel bounds the canvas positions it can occupy, so intersecting the
        // command extent with the travelled canvas drops pixels no frame can
        // reach while keeping every pixel some frame can.
        let canvas_w = crate::transform::CANVAS_WIDTH as i32;
        let canvas_h = crate::transform::CANVAS_HEIGHT as i32;
        let left = content_x.max(-reachable.left);
        let top = content_y.max(-reachable.top);
        let right = content_x
            .checked_add(i32::try_from(content_w).map_err(|_| "layer width overflow".to_string())?)
            .ok_or_else(|| "layer right edge overflow".to_string())?
            .min(canvas_w + reachable.right);
        let bottom = content_y
            .checked_add(i32::try_from(content_h).map_err(|_| "layer height overflow".to_string())?)
            .ok_or_else(|| "layer bottom edge overflow".to_string())?
            .min(canvas_h + reachable.bottom);
        if right <= left || bottom <= top {
            return Ok(empty);
        }
        let (origin_x, origin_y) = (left, top);
        let width = u32::try_from(right - left).map_err(|_| "layer width overflow".to_string())?;
        let height =
            u32::try_from(bottom - top).map_err(|_| "layer height overflow".to_string())?;
        let (surface_bytes, retained_peak_bytes) =
            animation_sdf_retained_surface_bytes(width, height)?;

        // A layer whose surface would outlive the bytes this export may retain
        // keeps its commands instead of its pixels; windows of it are
        // rasterized per frame. The commands stay in canvas coordinates here,
        // so any window origin can be applied later.
        if retained_peak_bytes > maximum_retained_layer_bytes {
            return Ok(CroppedLayerRaster {
                raster: AnimationLayerRaster::DeferredSdf(DeferredAnimationSdfLayer {
                    commands,
                    text_atlases: sdf_atlases,
                    shape_atlas: self.shape_sdf_atlas.clone(),
                    runtime_text: prepared_batch
                        .as_deref()
                        .map(|batch| batch.pages.clone())
                        .unwrap_or(runtime_text_pages),
                    executor,
                    tile_width,
                    tile_height,
                    x: origin_x,
                    y: origin_y,
                    width,
                    height,
                }),
                execution: None,
            });
        }

        shift_sdf_commands(&mut commands, -(origin_x as f32), -(origin_y as f32));
        let runtime_pages = prepared_batch
            .as_deref()
            .map(|batch| batch.pages.as_slice())
            .unwrap_or(runtime_text_pages.as_slice());
        let source = crate::sdf::tile::MixedSdfAtlasSource::with_runtime_text(
            &sdf_atlases,
            self.shape_sdf_atlas.as_deref(),
            runtime_pages,
        )
        .map_err(|error| error.to_string())?;

        let plan_started = std::time::Instant::now();
        let plan = crate::sdf::tile::SdfTilePlan::build_for_one_shot_dynamic_layer(
            crate::sdf::tile::TileGrid {
                canvas_width: width,
                canvas_height: height,
                tile_width,
                tile_height,
            },
            &commands,
            &source,
        )
        .map_err(|error| error.to_string())?;
        let plan_build_ns = elapsed_ns(plan_started);

        let mut rgba = self.take_profile_rgba_scratch(surface_bytes, [0, 0, 0, 0]);
        let execute_started = std::time::Instant::now();
        let execution_stats = match executor {
            Executor::Scalar => plan
                .execute_scalar_f32(&source, [0, 0, 0, 0], &mut rgba)
                .map_err(|error| error.to_string())?,
            Executor::Simd => plan
                .execute_simd(
                    &source,
                    [0, 0, 0, 0],
                    &mut rgba,
                    crate::sdf::tile::SdfAccumulationMode::F32Tile,
                )
                .map_err(|error| error.to_string())?,
        };
        let execute_ns = elapsed_ns(execute_started);

        let crop_started = std::time::Instant::now();
        let pixels = crop_pixels_lossless(
            &rgba,
            width as usize * 4,
            width,
            height,
            0,
            0,
            width,
            height,
        )?;
        let crop_ns = elapsed_ns(crop_started);
        // The surface is already the intersection of the content bounds and
        // the reachable canvas, so the full surface is the layer raster; the
        // accounting matches the page path (surface plus the full surface
        // copy taken during the crop).
        let scratch_peak_bytes = surface_bytes.saturating_mul(2);
        self.recycle_profile_rgba_scratch(rgba);
        let mut execution = SurfaceOutput {
            width,
            height,
            ..SurfaceOutput::default()
        };
        execution.sdf_run_count = 1;
        execution.sdf_text_element_count = captured.text_count;
        execution.sdf_shape_element_count = captured.shape_count;
        execution.captured_text_count = captured.text_count;
        execution.realtime_edt_glyphs = realtime_edt_glyphs;
        execution.realtime_edt_batch = realtime_edt_batch;
        execution.plan_stats = plan.stats();
        execution.execution_stats = execution_stats;
        execution.atlas_mapped_bytes =
            sdf_atlases
                .mapped_bytes()
                .saturating_add(self.shape_sdf_atlas.as_deref().map_or(
                    0,
                    crate::sdf::shape_atlas::MappedShapeSdfAtlas::mapped_bytes,
                ));
        execution.plan_bytes = plan.resident_bytes();
        execution.span_bytes = plan.span_bytes();
        execution.timings.capture_ns = capture_ns;
        execution.timings.command_mapping_ns = command_mapping_ns;
        execution.timings.plan_build_ns = plan_build_ns;
        execution.timings.execute_ns = execute_ns;
        execution.timings.postprocess_crop_ns = crop_ns;
        execution.timings.total_ns = elapsed_ns(total_started);
        Ok(CroppedLayerRaster {
            raster: AnimationLayerRaster::Ordered(OrderedLayerRaster {
                pixels,
                x: origin_x,
                y: origin_y,
                width,
                height,
                scratch_peak_bytes,
            }),
            execution: Some(execution),
        })
    }

    /// Rasterizes one source-space window of a deferred SDF animation layer
    /// from its retained commands, returning the window's pixels.
    ///
    /// `x`/`y`/`width`/`height` select a sub-rectangle of the layer's bounds in
    /// canvas coordinates. The commands are cloned and translated into that
    /// window's origin before the plan is built, so the window is self-contained
    /// and any frame can rasterize any window of the same layer.
    pub(crate) fn render_deferred_animation_sdf_window(
        &self,
        layer: &DeferredAnimationSdfLayer,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Result<RenderedDeferredAnimationWindow, String> {
        if width == 0 || height == 0 {
            return Err("deferred animation SDF window must be non-empty".into());
        }
        let right = x
            .checked_add(i32::try_from(width).map_err(|_| "deferred window width overflow")?)
            .ok_or("deferred window right overflow")?;
        let bottom = y
            .checked_add(i32::try_from(height).map_err(|_| "deferred window height overflow")?)
            .ok_or("deferred window bottom overflow")?;
        let layer_right = layer
            .x
            .checked_add(i32::try_from(layer.width).map_err(|_| "deferred layer width overflow")?)
            .ok_or("deferred layer right overflow")?;
        let layer_bottom = layer
            .y
            .checked_add(i32::try_from(layer.height).map_err(|_| "deferred layer height overflow")?)
            .ok_or("deferred layer bottom overflow")?;
        if x < layer.x || y < layer.y || right > layer_right || bottom > layer_bottom {
            return Err("deferred animation SDF window is outside the source layer".into());
        }

        let mut commands = layer.commands.clone();
        shift_sdf_commands(&mut commands, -(x as f32), -(y as f32));
        let source = crate::sdf::tile::MixedSdfAtlasSource::with_runtime_text(
            &layer.text_atlases,
            layer.shape_atlas.as_deref(),
            &layer.runtime_text,
        )
        .map_err(|error| error.to_string())?;
        let plan = crate::sdf::tile::SdfTilePlan::build_for_one_shot_dynamic_layer(
            crate::sdf::tile::TileGrid {
                canvas_width: width,
                canvas_height: height,
                tile_width: layer.tile_width,
                tile_height: layer.tile_height,
            },
            &commands,
            &source,
        )
        .map_err(|error| error.to_string())?;
        let surface_bytes = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "deferred animation SDF surface byte length overflow".to_string())?;
        let mut rgba = self.take_profile_rgba_scratch(surface_bytes, [0, 0, 0, 0]);
        match layer.executor {
            Executor::Scalar => plan
                .execute_scalar_f32(&source, [0, 0, 0, 0], &mut rgba)
                .map_err(|error| error.to_string())?,
            Executor::Simd => plan
                .execute_simd(
                    &source,
                    [0, 0, 0, 0],
                    &mut rgba,
                    crate::sdf::tile::SdfAccumulationMode::F32Tile,
                )
                .map_err(|error| error.to_string())?,
        };
        let pixels = crop_pixels_lossless(
            &rgba,
            width as usize * 4,
            width,
            height,
            0,
            0,
            width,
            height,
        )?;
        self.recycle_profile_rgba_scratch(rgba);
        Ok(RenderedDeferredAnimationWindow {
            pixels,
            x,
            y,
            width,
            height,
            scratch_peak_bytes: surface_bytes.saturating_mul(2),
        })
    }
}

/// Whether every visible element of a layer is text or a shape, so the layer
/// can be drawn from its SDF commands alone onto a surface sized to them.
fn layer_draws_only_sdf(elements: &[crate::elements::RenderElement<'_>]) -> bool {
    let mut visible = elements
        .iter()
        .filter(|element| element.visible())
        .peekable();
    visible.peek().is_some()
        && visible.all(|element| {
            matches!(
                element,
                crate::elements::RenderElement::Text(_) | crate::elements::RenderElement::Shape(_)
            )
        })
}

fn tight_sdf_command_bounds(
    commands: &[crate::sdf::tile::SdfDrawCommand],
) -> Option<(i32, i32, u32, u32)> {
    const GUARD: f32 = 2.0;
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    for command in commands {
        let mut command_min_x = command
            .quad
            .iter()
            .map(|point| point.x)
            .fold(f32::INFINITY, f32::min);
        let mut command_min_y = command
            .quad
            .iter()
            .map(|point| point.y)
            .fold(f32::INFINITY, f32::min);
        let mut command_max_x = command
            .quad
            .iter()
            .map(|point| point.x)
            .fold(f32::NEG_INFINITY, f32::max);
        let mut command_max_y = command
            .quad
            .iter()
            .map(|point| point.y)
            .fold(f32::NEG_INFINITY, f32::max);

        if let Some(clip) = command.device_clip {
            command_min_x = command_min_x.max(clip.min_x);
            command_min_y = command_min_y.max(clip.min_y);
            command_max_x = command_max_x.min(clip.max_x);
            command_max_y = command_max_y.min(clip.max_y);
        }

        if command_min_x >= command_max_x || command_min_y >= command_max_y {
            continue;
        }

        min_x = min_x.min(command_min_x);
        min_y = min_y.min(command_min_y);
        max_x = max_x.max(command_max_x);
        max_y = max_y.max(command_max_y);
    }

    if ![min_x, min_y, max_x, max_y].into_iter().all(f32::is_finite) {
        return None;
    }

    let origin_x = (min_x - GUARD).floor() as i32;
    let origin_y = (min_y - GUARD).floor() as i32;
    let right = (max_x + GUARD).ceil() as i32;
    let bottom = (max_y + GUARD).ceil() as i32;
    let width = u32::try_from(right.checked_sub(origin_x)?).ok()?;
    let height = u32::try_from(bottom.checked_sub(origin_y)?).ok()?;
    (width != 0 && height != 0).then_some((origin_x, origin_y, width, height))
}

pub(super) fn animation_sdf_retained_surface_bytes(
    width: u32,
    height: u32,
) -> Result<(usize, usize), String> {
    let surface_bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "animation SDF layer surface byte length overflow".to_string())?;
    Ok((surface_bytes, surface_bytes.saturating_mul(2)))
}

fn shift_sdf_commands(commands: &mut [crate::sdf::tile::SdfDrawCommand], dx: f32, dy: f32) {
    for command in commands {
        for point in &mut command.quad {
            point.x += dx;
            point.y += dy;
        }
        if let Some(clip) = &mut command.device_clip {
            clip.min_x += dx;
            clip.max_x += dx;
            clip.min_y += dy;
            clip.max_y += dy;
        }
    }
}

/// Copies a tight sub-rectangle out of a premultiplied RGBA8 buffer.
pub(super) fn crop_pixels_lossless(
    pixels: &[u8],
    source_row_bytes: usize,
    surface_width: u32,
    surface_height: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let required_row_bytes = (surface_width as usize)
        .checked_mul(4)
        .ok_or_else(|| "layer surface row stride overflow".to_string())?;
    if source_row_bytes < required_row_bytes {
        return Err("layer surface row stride is shorter than one row".to_string());
    }
    let required_bytes = source_row_bytes
        .checked_mul(surface_height as usize)
        .ok_or_else(|| "layer surface byte length overflow".to_string())?;
    if pixels.len() < required_bytes {
        return Err("layer surface buffer is truncated".to_string());
    }
    let right = x
        .checked_add(width)
        .ok_or_else(|| "crop x overflow".to_string())?;
    let bottom = y
        .checked_add(height)
        .ok_or_else(|| "crop y overflow".to_string())?;
    if width == 0 || height == 0 || right > surface_width || bottom > surface_height {
        return Err("crop rect is empty or outside the layer surface".to_string());
    }
    let row_bytes = (width as usize)
        .checked_mul(4)
        .ok_or_else(|| "crop row stride overflow".to_string())?;
    let start_of = |row: usize| (y as usize + row) * source_row_bytes + x as usize * 4;
    if pixels.len() < start_of(height as usize - 1) + row_bytes {
        return Err("crop rect exceeds the layer pixel buffer".to_string());
    }
    let mut cropped = vec![0u8; row_bytes * height as usize];
    for row in 0..height as usize {
        let source = start_of(row);
        cropped[row * row_bytes..(row + 1) * row_bytes]
            .copy_from_slice(&pixels[source..source + row_bytes]);
    }
    Ok(cropped)
}

fn layer_dynamic_for_elements(
    elements: &[crate::elements::RenderElement<'_>],
    md: &MasterData,
) -> Option<LayerDynamic> {
    let visible_text = elements.iter().find_map(|elem| match elem {
        crate::elements::RenderElement::Text(text) if text.object_data.visible => Some(*text),
        _ => None,
    })?;
    let animation = crate::text::line_indent_x_animation(visible_text, md)?;
    let (_, _, angle_deg, sx, _) = crate::transform::extract_transform(&visible_text.object_data);
    let radians = angle_deg.to_radians();
    let cos = radians.cos();
    let sin = radians.sin();
    let frames = animation
        .frames
        .into_iter()
        .map(|frame| {
            let local_x = frame.dx_local * sx;
            LayerDynamicFrame {
                frame: frame.frame,
                dx: local_x * cos,
                dy: local_x * sin,
            }
        })
        .collect();

    Some(LayerDynamic::TmpLineIndent {
        fps: animation.fps,
        looped: animation.looped,
        frames,
    })
}

#[derive(Default)]
struct CanvasExpansion {
    left: i32,
    right: i32,
    top: i32,
    bottom: i32,
}

fn dynamic_canvas_expansion(dynamic: &LayerDynamic) -> CanvasExpansion {
    const PAD: i32 = 8;
    let mut min_dx = 0.0_f32;
    let mut max_dx = 0.0_f32;
    let mut min_dy = 0.0_f32;
    let mut max_dy = 0.0_f32;

    match dynamic {
        LayerDynamic::TmpLineIndent { frames, .. } => {
            for frame in frames {
                min_dx = min_dx.min(frame.dx);
                max_dx = max_dx.max(frame.dx);
                min_dy = min_dy.min(frame.dy);
                max_dy = max_dy.max(frame.dy);
            }
        }
    }

    CanvasExpansion {
        left: max_dx.max(0.0).ceil() as i32 + PAD,
        right: (-min_dx).max(0.0).ceil() as i32 + PAD,
        top: max_dy.max(0.0).ceil() as i32 + PAD,
        bottom: (-min_dy).max(0.0).ceil() as i32 + PAD,
    }
}

/// Tight non-transparent bounds of a premultiplied RGBA8 layer buffer.
///
/// Only the alpha channel takes part, so premultiplication does not affect the
/// result.
fn opaque_bounds_for_pixels(
    pixels: &[u8],
    w: i32,
    h: i32,
    row_bytes: usize,
) -> Result<(u32, u32, u32, u32), String> {
    if pixels.len() < row_bytes * h as usize {
        return Err("layer surface buffer is truncated".to_string());
    }
    Ok(find_opaque_bounds(pixels, w as u32, h as u32, row_bytes))
}

/// The smallest rectangle holding every pixel with non-zero alpha.
pub(super) fn find_opaque_bounds(
    pixels: &[u8],
    width: u32,
    height: u32,
    row_bytes: usize,
) -> (u32, u32, u32, u32) {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
        && std::arch::is_x86_feature_detected!("bmi2")
    {
        // SAFETY: the feature checks above satisfy the implementation's
        // target-feature contract. Callers already provide a complete RGBA8
        // surface, exactly as required by the scalar implementation.
        return unsafe { find_opaque_bounds_avx512(pixels, width, height, row_bytes) };
    }

    find_opaque_bounds_scalar(pixels, width, height, row_bytes)
}

pub(super) fn find_opaque_bounds_scalar(
    pixels: &[u8],
    width: u32,
    height: u32,
    row_bytes: usize,
) -> (u32, u32, u32, u32) {
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x: u32 = 0;
    let mut max_y: u32 = 0;

    for y in 0..height {
        let row_start = y as usize * row_bytes;
        for x in 0..width {
            // RGBA8888, alpha is byte offset 3
            let pixel_offset = row_start + (x as usize) * 4;
            let alpha = pixels[pixel_offset + 3];
            if alpha > 0 {
                if x < min_x {
                    min_x = x;
                }
                if x > max_x {
                    max_x = x;
                }
                if y < min_y {
                    min_y = y;
                }
                if y > max_y {
                    max_y = y;
                }
            }
        }
    }

    if max_x < min_x || max_y < min_y {
        return (0, 0, 0, 0);
    }

    (min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
}

/// Checks 16 RGBA8 pixels per ZMM load. The compare produces one bit per byte;
/// BMI2 PEXT compacts byte positions 3, 7, ... 63 into a 16-bit pixel mask.
/// The tail remains scalar, avoiding masked-load setup once per scanline.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw,bmi2")]
pub(super) unsafe fn find_opaque_bounds_avx512(
    pixels: &[u8],
    width: u32,
    height: u32,
    row_bytes: usize,
) -> (u32, u32, u32, u32) {
    use std::arch::x86_64::*;

    const ALPHA_BYTE_BITS: u64 = 0x8888_8888_8888_8888;
    const PIXELS_PER_ZMM: u32 = 16;

    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let zero = _mm512_setzero_si512();

    for y in 0..height {
        let row = pixels.as_ptr().add(y as usize * row_bytes);
        let mut x = 0u32;
        while x + PIXELS_PER_ZMM <= width {
            let rgba = _mm512_loadu_si512(row.add(x as usize * 4).cast());
            let nonzero_bytes = _mm512_cmpneq_epi8_mask(rgba, zero);
            let alpha_pixels = _pext_u64(nonzero_bytes, ALPHA_BYTE_BITS) as u16;
            if alpha_pixels != 0 {
                min_y = min_y.min(y);
                max_y = max_y.max(y);
                min_x = min_x.min(x + alpha_pixels.trailing_zeros());
                max_x = max_x.max(x + (u16::BITS - 1 - alpha_pixels.leading_zeros()));
            }
            x += PIXELS_PER_ZMM;
        }
        while x < width {
            if *row.add(x as usize * 4 + 3) != 0 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
            x += 1;
        }
    }

    if min_x == width || min_y == height {
        (0, 0, 0, 0)
    } else {
        (min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
    }
}

#[cfg(test)]
mod tests;
