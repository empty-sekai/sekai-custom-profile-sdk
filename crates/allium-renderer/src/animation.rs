#[cfg(test)]
use std::io::Cursor;

use serde::{Deserialize, Serialize};

use crate::assets::AssetStore;
use crate::masterdata::MasterData;
use crate::types::CustomProfileCard;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnimationFormat {
    Gif,
    Webp,
    Apng,
    Mp4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnimationPreset {
    QqV1,
    QqV2,
    WebV1,
    ArchiveV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum H264YuvConverter {
    Auto,
    Scalar,
    Avx512,
}

impl H264YuvConverter {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "scalar" => Ok(Self::Scalar),
            "avx512" => Ok(Self::Avx512),
            _ => Err(format!("unsupported H.264 YUV converter: {value}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Scalar => "scalar",
            Self::Avx512 => "avx512",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct H264EncoderConfig {
    pub qp: u8,
    pub yuv_converter: H264YuvConverter,
    pub require_avx512: bool,
    pub validate_avx512: bool,
    pub encoder_revision: String,
}

impl Default for H264EncoderConfig {
    fn default() -> Self {
        Self {
            qp: 18,
            yuv_converter: H264YuvConverter::Auto,
            require_avx512: false,
            validate_avx512: false,
            encoder_revision: "direct-libx264-lazy-font-mp4-v9".into(),
        }
    }
}

impl H264EncoderConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.qp > 51 {
            return Err(format!("invalid H.264 QP: {}", self.qp));
        }
        if (self.require_avx512 || self.validate_avx512)
            && self.yuv_converter == H264YuvConverter::Scalar
        {
            return Err("H.264 AVX-512 was required but the scalar converter was selected".into());
        }
        if self.encoder_revision.is_empty()
            || !self
                .encoder_revision
                .bytes()
                .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'.' | b'_' | b'-'))
        {
            return Err("H.264 encoder revision must use [A-Za-z0-9._-]".into());
        }
        Ok(())
    }

    pub fn cache_identity(&self) -> Result<String, String> {
        self.validate()?;
        Ok(format!(
            "h264-v2-{}-qp-{}-yuv-{}-require512-{}",
            self.encoder_revision,
            self.qp,
            self.yuv_converter.as_str(),
            self.require_avx512
        ))
    }
}

pub fn animation_encoder_cache_suffix(preset: &ResolvedAnimationPreset) -> Result<String, String> {
    match preset.format {
        AnimationFormat::Mp4 => Ok(format!("-{}", preset.h264.cache_identity()?)),
        AnimationFormat::Gif | AnimationFormat::Webp | AnimationFormat::Apng => Ok(String::new()),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedAnimationPreset {
    pub preset: AnimationPreset,
    pub format: AnimationFormat,
    pub fps: u32,
    pub maximum_ticks: u64,
    pub maximum_long_edge: u32,
    pub output_budget_bytes: usize,
    pub export_memory_budget_bytes: usize,
    pub budget_step: u8,
    pub gif_quality: u8,
    pub h264: H264EncoderConfig,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnimationBudgetStep {
    pub index: u8,
    pub maximum_long_edge: u32,
    pub fps: u32,
    pub gif_quality: u8,
    pub output_budget_bytes: usize,
}

pub fn animation_budget_steps(preset: &ResolvedAnimationPreset) -> Vec<AnimationBudgetStep> {
    let variants: &[(u32, u32, u8)] = match preset.preset {
        AnimationPreset::QqV1 => &[
            (1280, 20, 80),
            (1280, 20, 65),
            (1280, 15, 65),
            (960, 15, 65),
        ],
        AnimationPreset::QqV2 => &[(1830, 12, 80)],
        AnimationPreset::WebV1 | AnimationPreset::ArchiveV1 => {
            &[(preset.maximum_long_edge, preset.fps, 80)]
        }
    };
    variants
        .iter()
        .enumerate()
        .map(
            |(index, &(maximum_long_edge, fps, gif_quality))| AnimationBudgetStep {
                index: index as u8,
                maximum_long_edge,
                fps,
                gif_quality,
                output_budget_bytes: preset.output_budget_bytes,
            },
        )
        .collect()
}

pub fn run_animation_budget_ladder<T, F>(
    preset: &ResolvedAnimationPreset,
    mut attempt: F,
) -> Result<(T, AnimationBudgetStep, u8), String>
where
    F: FnMut(&AnimationBudgetStep) -> Result<T, String>,
{
    let mut last_budget_error = None;
    for step in animation_budget_steps(preset) {
        match attempt(&step) {
            Ok(value) => return Ok((value, step.clone(), step.index + 1)),
            Err(error) if error.contains("OUTPUT_BUDGET_EXCEEDED") => {
                last_budget_error = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_budget_error
        .unwrap_or_else(|| "OUTPUT_BUDGET_EXCEEDED: animation budget ladder exhausted".into()))
}

pub fn resolve_preset(
    alias: &str,
    requested_format: Option<AnimationFormat>,
) -> Result<ResolvedAnimationPreset, String> {
    let (preset, auto_format, fps, seconds, edge, output_mib, memory_mib) = match alias {
        "qq-v1" => (
            AnimationPreset::QqV1,
            AnimationFormat::Gif,
            20,
            8,
            1280,
            10,
            64,
        ),
        "qq" | "qq-v2" => (
            AnimationPreset::QqV2,
            AnimationFormat::Gif,
            12,
            8,
            1830,
            10,
            64,
        ),
        "web" | "web-v1" => (
            AnimationPreset::WebV1,
            AnimationFormat::Webp,
            30,
            10,
            1830,
            24,
            128,
        ),
        "archive" | "archive-v1" => (
            AnimationPreset::ArchiveV1,
            AnimationFormat::Apng,
            60,
            30,
            1830,
            256,
            512,
        ),
        value => return Err(format!("ANIMATION_PRESET_UNSUPPORTED: {value}")),
    };
    let format = requested_format.unwrap_or(auto_format);
    Ok(ResolvedAnimationPreset {
        preset,
        format,
        fps,
        maximum_ticks: seconds * 60,
        maximum_long_edge: edge,
        output_budget_bytes: output_mib * 1024 * 1024,
        // MP4 streams frames directly into libx264 and the service serializes
        // animation exports globally. Bitmap formats still need a strict
        // retained-frame budget, but MP4 must not inherit that 64 MiB GIF cap.
        export_memory_budget_bytes: if format == AnimationFormat::Mp4 {
            usize::MAX
        } else {
            memory_mib * 1024 * 1024
        },
        budget_step: 0,
        gif_quality: 80,
        h264: H264EncoderConfig {
            qp: if preset == AnimationPreset::QqV2 {
                36
            } else {
                18
            },
            ..H264EncoderConfig::default()
        },
    })
}

impl AnimationFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::Webp => "webp",
            Self::Apng => "apng",
            Self::Mp4 => "mp4",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Apng => "image/apng",
            Self::Mp4 => "video/mp4",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimationEncodeSpec {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub frame_count: u32,
    pub looped: bool,
    pub output_budget_bytes: usize,
    pub gif_quality: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EncodedAnimation {
    pub data: Vec<u8>,
    pub format: AnimationFormat,
    pub content_type: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub frame_count: u32,
    pub duration_ms: u64,
    pub looped: bool,
    pub peak_frame_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimationExportTelemetry {
    pub compiled_program_count: u32,
    pub observable_program_count: u32,
    pub sampled_frames: u32,
    pub dynamic_evaluations: u64,
    pub layout_rebuilds: u64,
    pub command_rebuilds: u64,
    pub atlas_uploads: u64,
    pub layer_raster_bytes: usize,
    pub layer_scratch_peak_bytes: usize,
    pub peak_export_bytes: usize,
    pub render_ms: f64,
    pub encode_ms: f64,
    #[serde(default)]
    pub frame_full_composites: u64,
    #[serde(default)]
    pub frame_dirty_composites: u64,
    #[serde(default)]
    pub frame_dirty_regions: u64,
    #[serde(default)]
    pub frame_reused_composites: u64,
    #[serde(default)]
    pub frame_dirty_pixels: u64,
    #[serde(default)]
    pub frame_composite_ns: u64,
    #[serde(default)]
    pub frame_readback_ns: u64,
    #[serde(default)]
    pub frame_readback_bytes: u64,
    #[serde(default)]
    pub encoder_frame_callback_calls: u64,
    #[serde(default)]
    pub encoder_frame_callback_ns: u64,
    #[serde(default)]
    pub gif_palette_sample_ns: u64,
    #[serde(default)]
    pub gif_palette_quantize_ns: u64,
    #[serde(default)]
    pub gif_delta_index_ns: u64,
    #[serde(default)]
    pub gif_lzw_ns: u64,
    #[serde(default)]
    pub gif_retained_frame_bytes: usize,
    #[serde(default)]
    pub gif_second_frame_pass: bool,
    #[serde(default)]
    pub h264_yuv_ns: u64,
    #[serde(default)]
    pub h264_pipe_write_ns: u64,
    #[serde(default)]
    pub h264_codec_wait_ns: u64,
    #[serde(default)]
    pub h264_output_read_ns: u64,
    #[serde(default)]
    pub h264_direct_encode_ns: u64,
    #[serde(default)]
    pub h264_mux_ns: u64,
    #[serde(default)]
    pub h264_mbinfo_constant_blocks: u64,
    #[serde(default)]
    pub h264_mbinfo_total_blocks: u64,
    pub budget_step: u8,
    pub gif_quality: u8,
    pub encode_attempts: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_backend: Option<crate::profile_backend::ProfileRenderTelemetry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileAnimationExport {
    pub animated: bool,
    pub encoded: Option<EncodedAnimation>,
    pub telemetry: AnimationExportTelemetry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageExecution {
    StaticPassthrough,
    StaticWrap,
    Animation,
}

pub fn plan_page_execution(animated: bool, static_policy: &str) -> Result<PageExecution, String> {
    match (animated, static_policy) {
        (true, _) => Ok(PageExecution::Animation),
        (false, "passthrough") => Ok(PageExecution::StaticPassthrough),
        (false, "wrap") => Ok(PageExecution::StaticWrap),
        (_, value) => Err(format!("unsupported static_policy={value}")),
    }
}

struct RasterLayer {
    dynamic_layer_id: Option<allium_renderer_core::LayerId>,
    image: skia_safe::Image,
    x: f32,
    y: f32,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PixelRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl PixelRect {
    fn full(width: u32, height: u32) -> Self {
        Self {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        }
    }

    fn is_empty(self) -> bool {
        self.left >= self.right || self.top >= self.bottom
    }

    fn area(self) -> u64 {
        if self.is_empty() {
            return 0;
        }
        (self.right - self.left) as u64 * (self.bottom - self.top) as u64
    }

    fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    fn intersects(self, other: Self) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }

    fn contains(self, other: Self) -> bool {
        !other.is_empty()
            && self.left <= other.left
            && self.top <= other.top
            && self.right >= other.right
            && self.bottom >= other.bottom
    }

    fn as_skia(self) -> skia_safe::Rect {
        skia_safe::Rect::from_ltrb(
            self.left as f32,
            self.top as f32,
            self.right as f32,
            self.bottom as f32,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct LayerFrameState {
    visible: bool,
    destination: skia_safe::Rect,
    pixel_bounds: PixelRect,
}

#[derive(Clone, Copy, Debug, Default)]
struct FrameCompositeTelemetry {
    full_composites: u64,
    dirty_composites: u64,
    dirty_regions: u64,
    reused_composites: u64,
    dirty_pixels: u64,
    composite_ns: u64,
    readback_ns: u64,
    readback_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FrameCompositeUpdate {
    Full,
    Dirty(Vec<u32>),
    Reused,
}

struct AnimationFrameCompositor {
    surface: skia_safe::Surface,
    width: u32,
    height: u32,
    scale: f32,
    previous_index: Option<u32>,
    previous_states: Vec<LayerFrameState>,
    rgba: Vec<u8>,
    readback_scratch: Vec<u8>,
    changed_macroblock_flags: Vec<u8>,
    telemetry: FrameCompositeTelemetry,
}

#[derive(Default)]
struct DirtyRegionSet {
    regions: Vec<PixelRect>,
}

impl DirtyRegionSet {
    fn add(&mut self, mut region: PixelRect) {
        if region.is_empty()
            || self
                .regions
                .iter()
                .copied()
                .any(|value| value.contains(region))
        {
            return;
        }
        self.regions.retain(|value| !region.contains(*value));
        loop {
            let Some(index) = self.regions.iter().position(|value| {
                let union = value.union(region);
                union.area() <= value.area().saturating_add(region.area())
            }) else {
                break;
            };
            region = region.union(self.regions.swap_remove(index));
            self.regions.retain(|value| !region.contains(*value));
        }
        self.regions.push(region);
    }

    fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    fn total_area(&self) -> u64 {
        self.regions.iter().map(|value| value.area()).sum()
    }
}

type AnimationRasterGroup = (
    Option<allium_renderer_core::LayerId>,
    Vec<(allium_renderer_core::AuthoredElementKind, usize)>,
);

pub(crate) fn export_profile_animation(
    renderer: &crate::renderer::CustomProfileRenderer,
    card: &CustomProfileCard,
    profile: Option<&crate::profile::ProfileData>,
    document_key: &str,
    region: &str,
    md: &MasterData,
    assets: Option<&AssetStore>,
    preset: &ResolvedAnimationPreset,
    backend: Option<crate::profile_backend::ProfileBackendConfig>,
    render_object_store: Option<&crate::render_object::MappedRenderObjectStore>,
) -> Result<ProfileAnimationExport, String> {
    let (mut scene, resolved_scene) = if card.generals.is_empty() {
        let text_atlases = renderer.mapped_text_sdf_atlases();
        (
            crate::core_shadow::build_text_scene_with_atlases(
                card,
                md,
                document_key,
                text_atlases.as_deref(),
            )
            .map_err(|error| error.to_string())?,
            None,
        )
    } else {
        let (scene, resolved) = crate::core_shadow::build_scene_with_resolved(
            card,
            md,
            document_key,
            region,
            profile,
            region,
            assets,
        )?;
        (scene, Some(resolved))
    };
    let preflight = scene
        .animation_preflight(preset.maximum_ticks)
        .map_err(|error| error.to_string())?;
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
                profile_backend: None,
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
    let authored =
        allium_renderer_core::profile_scene::ordered_profile_elements(card, document_key);
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
    let (layers, layer_raster_bytes, layer_scratch_peak_bytes, profile_backend) =
        rasterize_animation_groups(
            renderer,
            card,
            profile,
            md,
            assets,
            resolved_scene.as_ref(),
            &dynamic_expansions,
            &groups,
            backend,
            render_object_store,
        )?;

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
    let frame_bytes = width as usize * height as usize * 4;
    let base_peak_export_bytes = animation_peak_export_bytes(layer_raster_bytes, frame_bytes);
    if base_peak_export_bytes > preset.export_memory_budget_bytes {
        return Err(format!(
            "ANIMATION_EXPORT_MEMORY_EXCEEDED: {base_peak_export_bytes} > {}",
            preset.export_memory_budget_bytes
        ));
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
    let mut compositor = AnimationFrameCompositor::new(width, height, scale)?;
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
            profile_backend,
        },
    })
}

fn animation_peak_export_bytes(layer_raster_bytes: usize, frame_bytes: usize) -> usize {
    layer_raster_bytes.saturating_add(frame_bytes.saturating_mul(2))
}

const fn align_to_8(value: u32) -> u32 {
    value.saturating_add(7) & !7
}

fn animation_layer_expansions(
    scene: &mut allium_renderer_core::Scene,
    dynamic_layer_ids: &[allium_renderer_core::LayerId],
    ticks: &[u64],
) -> std::collections::BTreeMap<allium_renderer_core::LayerId, [i32; 4]> {
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

fn rasterize_animation_groups(
    renderer: &crate::renderer::CustomProfileRenderer,
    card: &CustomProfileCard,
    profile: Option<&crate::profile::ProfileData>,
    md: &MasterData,
    assets: Option<&AssetStore>,
    _resolved_scene: Option<&allium_renderer_core::profile_scene::ResolvedProfileScene>,
    _dynamic_expansions: &std::collections::BTreeMap<allium_renderer_core::LayerId, [i32; 4]>,
    groups: &[AnimationRasterGroup],
    backend: Option<crate::profile_backend::ProfileBackendConfig>,
    _render_object_store: Option<&crate::render_object::MappedRenderObjectStore>,
) -> Result<
    (
        Vec<RasterLayer>,
        usize,
        usize,
        Option<crate::profile_backend::ProfileRenderTelemetry>,
    ),
    String,
> {
    use crate::profile_backend::{
        ProfileRenderTelemetry, ShapeSdfExecutor, TextSdfExecutor,
        PROFILE_RENDER_CONTRACT_LEGACY_SKIA,
    };

    let Some(config) = backend else {
        let (layers, bytes, scratch) =
            rasterize_animation_groups_legacy(card, profile, md, assets, groups)?;
        return Ok((layers, bytes, scratch, None));
    };
    let started = std::time::Instant::now();
    // The OSS animation exporter currently owns only the Skia layer raster path.
    // Resolve against that truthful capability surface so candidate requests
    // either fail closed or take the configured whole-page fallback.
    let mut capabilities = renderer.profile_backend_capabilities();
    capabilities.text_simd = false;
    capabilities.text_scalar_oracle = false;
    capabilities.shape_simd = false;
    let selection = config
        .resolve(capabilities)
        .map_err(|error| error.to_string())?;
    let mut telemetry = ProfileRenderTelemetry::new(config, PROFILE_RENDER_CONTRACT_LEGACY_SKIA);
    telemetry.apply_selection(selection.clone());
    telemetry.work.page_count = 1;
    telemetry.work.dynamic_layer_count = groups
        .iter()
        .filter(|(dynamic_layer_id, _)| dynamic_layer_id.is_some())
        .count() as u64;

    let (layers, bytes, scratch) =
        rasterize_animation_groups_legacy(card, profile, md, assets, groups)?;
    telemetry.actual_text_sdf = TextSdfExecutor::LegacySkia;
    telemetry.actual_shape_sdf = ShapeSdfExecutor::Skia;
    telemetry.render_contract = PROFILE_RENDER_CONTRACT_LEGACY_SKIA.into();
    telemetry.bytes.layer_cache_bytes = bytes as u64;
    telemetry.bytes.scratch_peak_bytes = scratch as u64;
    telemetry.timings.total_ns = elapsed_ns(started);
    Ok((layers, bytes, scratch, Some(telemetry)))
}

fn rasterize_animation_groups_legacy(
    card: &CustomProfileCard,
    profile: Option<&crate::profile::ProfileData>,
    md: &MasterData,
    assets: Option<&AssetStore>,
    groups: &[AnimationRasterGroup],
) -> Result<(Vec<RasterLayer>, usize, usize), String> {
    let mut layers = Vec::with_capacity(groups.len());
    let mut layer_raster_bytes = 0usize;
    let mut scratch_peak_bytes = 0usize;
    for (dynamic_layer_id, members) in groups {
        let layer_card = grouped_layer_card(card, members);
        let output = crate::renderer::render_element_layer_cropped_animation_raster(
            &layer_card,
            md,
            assets,
            profile,
            animation_group_uses_dynamic_bounds(*dynamic_layer_id),
        )?;
        push_animation_raster(
            &mut layers,
            &mut layer_raster_bytes,
            &mut scratch_peak_bytes,
            *dynamic_layer_id,
            output,
        )?;
    }
    Ok((layers, layer_raster_bytes, scratch_peak_bytes))
}

fn push_animation_raster(
    layers: &mut Vec<RasterLayer>,
    layer_raster_bytes: &mut usize,
    scratch_peak_bytes: &mut usize,
    dynamic_layer_id: Option<allium_renderer_core::LayerId>,
    output: crate::renderer::CroppedLayerRaster,
) -> Result<(), String> {
    if output.width == 0 || output.height == 0 {
        return Ok(());
    }
    *layer_raster_bytes = layer_raster_bytes
        .checked_add(output.width as usize * output.height as usize * 4)
        .ok_or_else(|| "animation layer raster byte overflow".to_string())?;
    *scratch_peak_bytes = (*scratch_peak_bytes).max(output.scratch_peak_bytes);
    layers.push(RasterLayer {
        dynamic_layer_id,
        image: output.image,
        x: output.x as f32,
        y: output.y as f32,
        width: output.width,
        height: output.height,
    });
    Ok(())
}

fn elapsed_ns(started: std::time::Instant) -> u64 {
    started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}

fn animation_group_uses_dynamic_bounds(
    dynamic_layer_id: Option<allium_renderer_core::LayerId>,
) -> bool {
    dynamic_layer_id.is_some()
}

fn animation_loop_period_ticks(
    timelines: impl IntoIterator<Item = allium_renderer_core::TimelineDescriptor>,
    maximum_tick: u64,
) -> Option<u64> {
    let mut timelines = timelines.into_iter();
    let first = timelines.next()?;
    if first.loop_start_tick != 0 || first.period_ticks == 0 {
        return None;
    }
    let mut common_period = first.period_ticks;
    for timeline in timelines {
        if timeline.loop_start_tick != first.loop_start_tick || timeline.period_ticks == 0 {
            return None;
        }
        let divisor = gcd(common_period, timeline.period_ticks);
        common_period = common_period
            .checked_div(divisor)?
            .checked_mul(timeline.period_ticks)?;
    }
    (common_period <= maximum_tick && maximum_tick % common_period == 0).then_some(common_period)
}

fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

fn plan_running_ticks(maximum_tick: u64, fps: u32) -> Vec<u64> {
    let frame_count = maximum_tick.saturating_mul(fps as u64).div_ceil(60);
    (0..frame_count)
        .map(|index| index.saturating_mul(60) / fps as u64)
        .collect()
}

fn plan_ticks(
    scene: &mut allium_renderer_core::Scene,
    dynamic_layer_ids: &[allium_renderer_core::LayerId],
    maximum_tick: u64,
    fps: u32,
) -> Vec<u64> {
    let running_ticks = plan_running_ticks(maximum_tick, fps);
    let mut ticks = Vec::new();
    let hold_frames = (fps / 2).max(1) as usize;
    for (index, tick) in running_ticks.iter().copied().enumerate() {
        scene.advance_to_tick(tick);
        ticks.push(tick);
        let settled = !dynamic_layer_ids.is_empty()
            && dynamic_layer_ids.iter().all(|id| {
                scene
                    .state(*id)
                    .and_then(|state| state.dynamic.as_ref())
                    .is_some_and(|state| {
                        matches!(
                            state.status,
                            allium_renderer_core::DynamicStatus::Settled
                                | allium_renderer_core::DynamicStatus::Held
                        )
                    })
            });
        if settled && ticks.len() > 1 {
            ticks.extend(
                running_ticks
                    .iter()
                    .skip(index + 1)
                    .take(hold_frames)
                    .copied(),
            );
            break;
        }
    }
    ticks
}

impl AnimationFrameCompositor {
    fn new(width: u32, height: u32, scale: f32) -> Result<Self, String> {
        let surface = skia_safe::surfaces::raster_n32_premul((width as i32, height as i32))
            .ok_or_else(|| "create animation compositor surface failed".to_string())?;
        Ok(Self {
            surface,
            width,
            height,
            scale,
            previous_index: None,
            previous_states: Vec::new(),
            rgba: vec![0; width as usize * height as usize * 4],
            readback_scratch: Vec::new(),
            changed_macroblock_flags: Vec::new(),
            telemetry: FrameCompositeTelemetry::default(),
        })
    }

    fn composite(
        &mut self,
        index: u32,
        scene: &allium_renderer_core::Scene,
        layers: &[RasterLayer],
    ) -> Result<Vec<u8>, String> {
        let mut rgba = std::mem::take(&mut self.rgba);
        self.composite_into(index, scene, layers, &mut rgba)?;
        self.rgba = rgba;
        Ok(self.rgba.clone())
    }

    fn composite_into(
        &mut self,
        index: u32,
        scene: &allium_renderer_core::Scene,
        layers: &[RasterLayer],
        rgba: &mut Vec<u8>,
    ) -> Result<FrameCompositeUpdate, String> {
        rgba.resize(self.width as usize * self.height as usize * 4, 0);
        let states = layers
            .iter()
            .map(|layer| layer_frame_state(scene, layer, self.width, self.height, self.scale))
            .collect::<Vec<_>>();
        self.composite_states(index, layers, states, rgba)
    }

    fn composite_states(
        &mut self,
        index: u32,
        layers: &[RasterLayer],
        states: Vec<LayerFrameState>,
        rgba: &mut Vec<u8>,
    ) -> Result<FrameCompositeUpdate, String> {
        let sequential = self
            .previous_index
            .is_some_and(|previous| index == previous.saturating_add(1))
            && self.previous_states.len() == states.len();
        let composite_started = std::time::Instant::now();
        let (mut update, dirty_regions) = if !sequential {
            redraw_full_frame(&mut self.surface, layers, &states);
            self.telemetry.full_composites = self.telemetry.full_composites.saturating_add(1);
            (FrameCompositeUpdate::Full, None)
        } else {
            let mut dirty = DirtyRegionSet::default();
            for ((layer, previous), current) in layers
                .iter()
                .zip(self.previous_states.iter())
                .zip(states.iter())
            {
                if layer.dynamic_layer_id.is_some() && previous != current {
                    dirty.add(previous.pixel_bounds);
                    dirty.add(current.pixel_bounds);
                }
            }
            if dirty.is_empty() {
                self.telemetry.reused_composites =
                    self.telemetry.reused_composites.saturating_add(1);
                (FrameCompositeUpdate::Reused, None)
            } else {
                for region in &dirty.regions {
                    redraw_dirty_frame(&mut self.surface, layers, &states, *region);
                }
                self.telemetry.dirty_composites = self.telemetry.dirty_composites.saturating_add(1);
                self.telemetry.dirty_regions = self
                    .telemetry
                    .dirty_regions
                    .saturating_add(dirty.regions.len() as u64);
                self.telemetry.dirty_pixels = self
                    .telemetry
                    .dirty_pixels
                    .saturating_add(dirty.total_area());
                (FrameCompositeUpdate::Dirty(Vec::new()), Some(dirty.regions))
            }
        };
        self.telemetry.composite_ns = self
            .telemetry
            .composite_ns
            .saturating_add(elapsed_ns(composite_started));
        self.previous_index = Some(index);
        self.previous_states = states;

        let readback_started = std::time::Instant::now();
        let readback_bytes = match dirty_regions.as_deref() {
            Some(regions) => {
                let changed_macroblocks = read_animation_surface_regions_rgba_into_exact(
                    &mut self.surface,
                    self.width,
                    self.height,
                    regions,
                    rgba,
                    &mut self.readback_scratch,
                    &mut self.changed_macroblock_flags,
                )?;
                update = FrameCompositeUpdate::Dirty(changed_macroblocks);
                regions.iter().map(|region| region.area() * 4).sum()
            }
            None if update == FrameCompositeUpdate::Full => {
                read_animation_surface_rgba_into(&mut self.surface, self.width, self.height, rgba)?;
                rgba.len() as u64
            }
            None => 0,
        };
        self.telemetry.readback_ns = self
            .telemetry
            .readback_ns
            .saturating_add(elapsed_ns(readback_started));
        self.telemetry.readback_bytes =
            self.telemetry.readback_bytes.saturating_add(readback_bytes);
        Ok(update)
    }
}

fn layer_frame_state(
    scene: &allium_renderer_core::Scene,
    layer: &RasterLayer,
    width: u32,
    height: u32,
    scale: f32,
) -> LayerFrameState {
    let state = layer.dynamic_layer_id.and_then(|id| scene.state(id));
    let visible = !state.is_some_and(|value| !value.render_mask);
    let transform = state
        .and_then(|value| value.dynamic.as_ref())
        .map(|value| value.transform)
        .unwrap_or_default();
    let destination = skia_safe::Rect::from_xywh(
        (layer.x + transform.dx) * scale,
        (layer.y + transform.dy) * scale,
        layer.width as f32 * scale,
        layer.height as f32 * scale,
    );
    LayerFrameState {
        visible,
        destination,
        pixel_bounds: if visible {
            pixel_bounds_for_destination(destination, width, height)
        } else {
            PixelRect::default()
        },
    }
}

fn pixel_bounds_for_destination(
    destination: skia_safe::Rect,
    width: u32,
    height: u32,
) -> PixelRect {
    // Keep one guard pixel around the geometric destination so filtered or
    // sub-pixel Skia coverage never survives just outside the dirty clip.
    PixelRect {
        left: (destination.left().floor() as i32 - 1).max(0),
        top: (destination.top().floor() as i32 - 1).max(0),
        right: (destination.right().ceil() as i32 + 1).min(width as i32),
        bottom: (destination.bottom().ceil() as i32 + 1).min(height as i32),
    }
}

fn redraw_full_frame(
    surface: &mut skia_safe::Surface,
    layers: &[RasterLayer],
    states: &[LayerFrameState],
) {
    let full = PixelRect::full(surface.width() as u32, surface.height() as u32);
    let canvas = surface.canvas();
    canvas.clear(skia_safe::Color::WHITE);
    draw_animation_layers(canvas, layers, states, full);
}

fn redraw_dirty_frame(
    surface: &mut skia_safe::Surface,
    layers: &[RasterLayer],
    states: &[LayerFrameState],
    dirty: PixelRect,
) {
    let canvas = surface.canvas();
    canvas.save();
    canvas.clip_rect(dirty.as_skia(), None, Some(false));
    let mut clear = skia_safe::Paint::default();
    clear.set_color(skia_safe::Color::WHITE);
    clear.set_blend_mode(skia_safe::BlendMode::Src);
    canvas.draw_rect(dirty.as_skia(), &clear);
    draw_animation_layers(canvas, layers, states, dirty);
    canvas.restore();
}

fn draw_animation_layers(
    canvas: &skia_safe::Canvas,
    layers: &[RasterLayer],
    states: &[LayerFrameState],
    dirty: PixelRect,
) {
    for (layer, state) in layers.iter().zip(states) {
        if !state.visible || !state.pixel_bounds.intersects(dirty) {
            continue;
        }
        canvas.draw_image_rect(
            &layer.image,
            None,
            &state.destination,
            &skia_safe::Paint::default(),
        );
    }
}

fn read_animation_surface_rgba(
    surface: &mut skia_safe::Surface,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    read_animation_surface_rgba_into(surface, width, height, &mut rgba)?;
    Ok(rgba)
}

fn read_animation_surface_rgba_into(
    surface: &mut skia_safe::Surface,
    width: u32,
    height: u32,
    rgba: &mut [u8],
) -> Result<(), String> {
    let required = width as usize * height as usize * 4;
    if rgba.len() != required {
        return Err("animation frame RGBA buffer has the wrong length".into());
    }
    let image = surface.image_snapshot();
    let info = skia_safe::ImageInfo::new(
        (width as i32, height as i32),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        None,
    );
    if !image.read_pixels(
        &info,
        rgba,
        width as usize * 4,
        skia_safe::IPoint::new(0, 0),
        skia_safe::image::CachingHint::Disallow,
    ) {
        return Err("read animation frame pixels failed".into());
    }
    Ok(())
}

fn read_animation_surface_regions_rgba_into_exact(
    surface: &mut skia_safe::Surface,
    width: u32,
    height: u32,
    regions: &[PixelRect],
    rgba: &mut [u8],
    scratch: &mut Vec<u8>,
    changed_macroblock_flags: &mut Vec<u8>,
) -> Result<Vec<u32>, String> {
    let row_bytes = width as usize * 4;
    let required = row_bytes * height as usize;
    if rgba.len() != required {
        return Err("animation frame RGBA buffer has the wrong length".into());
    }
    let mb_width = width.div_ceil(16) as usize;
    let mb_height = height.div_ceil(16) as usize;
    let mb_count = mb_width
        .checked_mul(mb_height)
        .ok_or_else(|| "animation macroblock map size overflow".to_string())?;
    changed_macroblock_flags.resize(mb_count, 0);
    changed_macroblock_flags.fill(0);
    let image = surface.image_snapshot();
    for region in regions {
        if region.is_empty() {
            continue;
        }
        let region_width = (region.right - region.left) as usize;
        let region_height = (region.bottom - region.top) as usize;
        let packed_row_bytes = region_width * 4;
        scratch.resize(packed_row_bytes * region_height, 0);
        let info = skia_safe::ImageInfo::new(
            (region_width as i32, region_height as i32),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Unpremul,
            None,
        );
        if !image.read_pixels(
            &info,
            scratch,
            packed_row_bytes,
            skia_safe::IPoint::new(region.left, region.top),
            skia_safe::image::CachingHint::Disallow,
        ) {
            return Err("read animation dirty region pixels failed".into());
        }
        mark_changed_macroblocks_for_region(
            rgba,
            scratch,
            width,
            *region,
            changed_macroblock_flags,
        )?;
        for local_y in 0..region_height {
            let destination =
                (region.top as usize + local_y) * row_bytes + region.left as usize * 4;
            let source = local_y * packed_row_bytes;
            rgba[destination..destination + packed_row_bytes]
                .copy_from_slice(&scratch[source..source + packed_row_bytes]);
        }
    }
    Ok(changed_macroblock_flags
        .iter()
        .enumerate()
        .filter_map(|(index, &changed)| (changed != 0).then_some(index as u32))
        .collect())
}

fn mark_changed_macroblocks_for_region(
    rgba: &[u8],
    packed_region: &[u8],
    width: u32,
    region: PixelRect,
    changed_macroblock_flags: &mut [u8],
) -> Result<(), String> {
    if region.is_empty() {
        return Ok(());
    }
    let width = width as usize;
    let row_bytes = width * 4;
    let region_width = (region.right - region.left) as usize;
    let region_height = (region.bottom - region.top) as usize;
    let packed_row_bytes = region_width * 4;
    if rgba.len() % row_bytes != 0 || packed_region.len() != packed_row_bytes * region_height {
        return Err("invalid exact dirty-region buffers".into());
    }
    let mb_width = width.div_ceil(16);
    let first_mb_x = region.left.max(0) as usize / 16;
    let last_mb_x = (region.right.max(0) as usize).div_ceil(16).min(mb_width);
    for local_y in 0..region_height {
        let y = region.top as usize + local_y;
        let mb_y = y / 16;
        for mb_x in first_mb_x..last_mb_x {
            let mb_index = mb_y * mb_width + mb_x;
            if changed_macroblock_flags.get(mb_index).copied() == Some(1) {
                continue;
            }
            let left = (mb_x * 16).max(region.left as usize);
            let right = ((mb_x + 1) * 16).min(region.right as usize);
            let bytes = (right - left) * 4;
            let before = y * row_bytes + left * 4;
            let after = local_y * packed_row_bytes + (left - region.left as usize) * 4;
            if rgba_segment_changed(
                &rgba[before..before + bytes],
                &packed_region[after..after + bytes],
            ) {
                changed_macroblock_flags[mb_index] = 1;
            }
        }
    }
    Ok(())
}

fn rgba_segment_changed(before: &[u8], after: &[u8]) -> bool {
    if before.len() != after.len() {
        return true;
    }
    #[cfg(target_arch = "x86_64")]
    if before.len() == 64
        && std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
    {
        // SAFETY: both slices contain one complete 16-pixel macroblock row.
        return unsafe { rgba_segment_changed_avx512(before, after) };
    }
    before != after
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn rgba_segment_changed_avx512(before: &[u8], after: &[u8]) -> bool {
    use std::arch::x86_64::*;
    let left = _mm512_loadu_si512(before.as_ptr().cast());
    let right = _mm512_loadu_si512(after.as_ptr().cast());
    _mm512_cmpeq_epi8_mask(left, right) != u64::MAX
}

fn composite_frame(
    scene: &allium_renderer_core::Scene,
    layers: &[RasterLayer],
    width: u32,
    height: u32,
    scale: f32,
) -> Result<Vec<u8>, String> {
    let mut surface = skia_safe::surfaces::raster_n32_premul((width as i32, height as i32))
        .ok_or_else(|| "create animation frame surface failed".to_string())?;
    let canvas = surface.canvas();
    // The game always renders a complete custom-profile card over its default
    // white canvas. Keeping full animation frames opaque also lets GIF delta
    // frames erase a moving layer by writing white pixels at its old position.
    canvas.clear(skia_safe::Color::WHITE);
    for layer in layers {
        let state = layer.dynamic_layer_id.and_then(|id| scene.state(id));
        if state.is_some_and(|value| !value.render_mask) {
            continue;
        }
        let transform = state
            .and_then(|value| value.dynamic.as_ref())
            .map(|value| value.transform)
            .unwrap_or_default();
        let destination = skia_safe::Rect::from_xywh(
            (layer.x + transform.dx) * scale,
            (layer.y + transform.dy) * scale,
            layer.width as f32 * scale,
            layer.height as f32 * scale,
        );
        canvas.draw_image_rect(
            &layer.image,
            None,
            &destination,
            &skia_safe::Paint::default(),
        );
    }
    read_animation_surface_rgba(&mut surface, width, height)
}

fn grouped_layer_card(
    base: &CustomProfileCard,
    members: &[(allium_renderer_core::AuthoredElementKind, usize)],
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
    };
    use allium_renderer_core::AuthoredElementKind::*;
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
        }
    }
    card
}

pub fn encode_rgba_frames<F>(
    format: AnimationFormat,
    spec: &AnimationEncodeSpec,
    frame_at: F,
) -> Result<EncodedAnimation, String>
where
    F: FnMut(u32) -> Result<Vec<u8>, String>,
{
    encode_rgba_frames_with_h264(format, spec, &H264EncoderConfig::default(), frame_at)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct AnimationEncodeTelemetry {
    frame_callback_calls: u64,
    frame_callback_ns: u64,
    gif_palette_sample_ns: u64,
    gif_palette_quantize_ns: u64,
    gif_delta_index_ns: u64,
    gif_lzw_ns: u64,
    gif_retained_frame_bytes: usize,
    gif_second_frame_pass: bool,
    h264_yuv_ns: u64,
    h264_pipe_write_ns: u64,
    h264_codec_wait_ns: u64,
    h264_output_read_ns: u64,
    h264_direct_encode_ns: u64,
    h264_mux_ns: u64,
    h264_mbinfo_constant_blocks: u64,
    h264_mbinfo_total_blocks: u64,
}

fn encode_rgba_frames_with_h264<F>(
    format: AnimationFormat,
    spec: &AnimationEncodeSpec,
    h264: &H264EncoderConfig,
    frame_at: F,
) -> Result<EncodedAnimation, String>
where
    F: FnMut(u32) -> Result<Vec<u8>, String>,
{
    encode_rgba_frames_with_h264_and_telemetry(format, spec, h264, 0, frame_at)
        .map(|(encoded, _)| encoded)
}

fn encode_rgba_frames_with_h264_and_telemetry<F>(
    format: AnimationFormat,
    spec: &AnimationEncodeSpec,
    h264: &H264EncoderConfig,
    gif_retention_budget: usize,
    mut frame_at: F,
) -> Result<(EncodedAnimation, AnimationEncodeTelemetry), String>
where
    F: FnMut(u32) -> Result<Vec<u8>, String>,
{
    validate_spec(spec)?;
    let expected = spec.width as usize * spec.height as usize * 4;
    let mut telemetry = AnimationEncodeTelemetry::default();
    let data = match format {
        AnimationFormat::Gif => encode_gif(
            spec,
            expected,
            gif_retention_budget,
            &mut telemetry,
            &mut frame_at,
        )?,
        AnimationFormat::Webp => encode_webp(spec, expected, &mut frame_at)?,
        AnimationFormat::Apng => encode_apng(spec, expected, &mut frame_at)?,
        AnimationFormat::Mp4 => encode_mp4(spec, expected, h264, &mut telemetry, &mut frame_at)?,
    };
    let encoded = finish_encoded_animation(format, spec, expected, data)?;
    Ok((encoded, telemetry))
}

fn finish_encoded_animation(
    format: AnimationFormat,
    spec: &AnimationEncodeSpec,
    expected: usize,
    data: Vec<u8>,
) -> Result<EncodedAnimation, String> {
    if data.len() > spec.output_budget_bytes {
        return Err(format!(
            "OUTPUT_BUDGET_EXCEEDED: {} > {}",
            data.len(),
            spec.output_budget_bytes
        ));
    }
    validate_magic(format, &data)?;
    Ok(EncodedAnimation {
        data,
        format,
        content_type: format.content_type().into(),
        width: spec.width,
        height: spec.height,
        fps: spec.fps,
        frame_count: spec.frame_count,
        duration_ms: encoded_duration_ms(format, spec),
        looped: spec.looped && format != AnimationFormat::Mp4,
        peak_frame_bytes: expected,
    })
}

fn encoded_duration_ms(format: AnimationFormat, spec: &AnimationEncodeSpec) -> u64 {
    match format {
        AnimationFormat::Gif => (0..spec.frame_count)
            .map(|index| gif_delay_centiseconds(index, spec.fps) as u64 * 10)
            .sum(),
        AnimationFormat::Webp | AnimationFormat::Apng | AnimationFormat::Mp4 => {
            spec.frame_count as u64 * 1000 / spec.fps as u64
        }
    }
}

pub fn wrap_static_png(
    png: &[u8],
    preset: &ResolvedAnimationPreset,
) -> Result<EncodedAnimation, String> {
    let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(png))
        .ok_or_else(|| "decode static PNG for animation wrap failed".to_string())?;
    let source_width = image.width() as u32;
    let source_height = image.height() as u32;
    let scale = (preset.maximum_long_edge as f32 / source_width.max(source_height) as f32).min(1.0);
    let width = (source_width as f32 * scale).round() as u32;
    let height = (source_height as f32 * scale).round() as u32;
    let mut surface = skia_safe::surfaces::raster_n32_premul((width as i32, height as i32))
        .ok_or_else(|| "create static wrap surface failed".to_string())?;
    surface.canvas().clear(skia_safe::Color::TRANSPARENT);
    surface.canvas().draw_image_rect(
        &image,
        None,
        &skia_safe::Rect::from_xywh(0.0, 0.0, width as f32, height as f32),
        &skia_safe::Paint::default(),
    );
    let snapshot = surface.image_snapshot();
    let info = skia_safe::ImageInfo::new(
        (width as i32, height as i32),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        None,
    );
    let mut rgba = vec![0; width as usize * height as usize * 4];
    if !snapshot.read_pixels(
        &info,
        &mut rgba,
        width as usize * 4,
        skia_safe::IPoint::new(0, 0),
        skia_safe::image::CachingHint::Disallow,
    ) {
        return Err("read static wrap pixels failed".into());
    }
    encode_rgba_frames_with_h264(
        preset.format,
        &AnimationEncodeSpec {
            width,
            height,
            fps: preset.fps,
            frame_count: 1,
            looped: false,
            output_budget_bytes: preset.output_budget_bytes,
            gif_quality: preset.gif_quality,
        },
        &preset.h264,
        |_| Ok(rgba.clone()),
    )
}

fn validate_spec(spec: &AnimationEncodeSpec) -> Result<(), String> {
    if spec.width == 0 || spec.height == 0 || spec.frame_count == 0 || spec.fps == 0 {
        return Err("animation dimensions/frame_count/fps must be positive".into());
    }
    if spec.fps > 60 {
        return Err("animation fps must not exceed the 60Hz core clock".into());
    }
    Ok(())
}

fn checked_frame<F>(frame_at: &mut F, index: u32, expected: usize) -> Result<Vec<u8>, String>
where
    F: FnMut(u32) -> Result<Vec<u8>, String>,
{
    let frame = frame_at(index)?;
    if frame.len() != expected {
        return Err(format!(
            "frame {index} has {} bytes, expected {expected}",
            frame.len()
        ));
    }
    Ok(frame)
}

fn checked_frame_timed<F>(
    frame_at: &mut F,
    index: u32,
    expected: usize,
    telemetry: &mut AnimationEncodeTelemetry,
) -> Result<Vec<u8>, String>
where
    F: FnMut(u32) -> Result<Vec<u8>, String>,
{
    let started = std::time::Instant::now();
    let frame = checked_frame(frame_at, index, expected);
    telemetry.frame_callback_calls += 1;
    telemetry.frame_callback_ns = telemetry
        .frame_callback_ns
        .saturating_add(started.elapsed().as_nanos() as u64);
    frame
}

enum GifRetainedFrame {
    Full(Vec<u8>),
    Delta {
        left: u32,
        top: u32,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        changed: Vec<u8>,
    },
    Unchanged,
}

impl GifRetainedFrame {
    fn heap_bytes(&self) -> usize {
        match self {
            Self::Full(rgba) => rgba.capacity(),
            Self::Delta { rgba, changed, .. } => rgba.capacity().saturating_add(changed.capacity()),
            Self::Unchanged => 0,
        }
    }
}

struct GifPaletteLookup {
    // Palette entries are limited to 0..=254, so 255 is an untouched
    // sentinel. The direct-mapped RGB cache preserves exact nearest-color
    // results while keeping the indexed-frame hot set to 4 MiB.
    direct: Option<Vec<u32>>,
    sparse: std::collections::HashMap<u32, u8>,
    active_colors: usize,
    palette_r: [u32; 256],
    palette_g: [u32; 256],
    palette_b: [u32; 256],
}

impl GifPaletteLookup {
    const DIRECT_SLOTS: usize = 1 << 20;
    const DIRECT_BYTES: usize = Self::DIRECT_SLOTS * std::mem::size_of::<u32>();

    fn new(memory_budget: usize, active_colors: usize) -> Self {
        Self {
            direct: (memory_budget >= Self::DIRECT_BYTES)
                .then(|| vec![u32::MAX; Self::DIRECT_SLOTS]),
            sparse: std::collections::HashMap::with_capacity(4096),
            active_colors: active_colors.clamp(1, 255),
            palette_r: [0; 256],
            palette_g: [0; 256],
            palette_b: [0; 256],
        }
    }

    fn set_palette(&mut self, palette: &[u8]) {
        for (index, color) in palette.chunks_exact(3).take(self.active_colors).enumerate() {
            self.palette_r[index] = color[0] as u32;
            self.palette_g[index] = color[1] as u32;
            self.palette_b[index] = color[2] as u32;
        }
    }

    fn get(&self, key: u32) -> Option<u8> {
        self.direct
            .as_ref()
            .and_then(|table| {
                let packed = table[gif_palette_cache_slot(key)];
                (packed != u32::MAX && packed & 0x00ff_ffff == key).then_some((packed >> 24) as u8)
            })
            .or_else(|| self.sparse.get(&key).copied())
    }

    fn insert(&mut self, key: u32, value: u8) {
        if let Some(table) = self.direct.as_mut() {
            table[gif_palette_cache_slot(key)] = key | ((value as u32) << 24);
        } else {
            self.sparse.insert(key, value);
        }
    }

    fn heap_bytes(&self) -> usize {
        self.direct.as_ref().map_or(0, Vec::capacity)
    }
}

#[inline(always)]
fn gif_palette_cache_slot(key: u32) -> usize {
    key.wrapping_mul(0x9e37_79b1) as usize & (GifPaletteLookup::DIRECT_SLOTS - 1)
}

struct FastGifLzwEncoder {
    // A GIF dictionary key is exactly (prefix_code << 8) | suffix_byte, so it
    // fits in 20 bits. Direct addressing removes hash probes from the serial
    // LZW dependency chain. The high 20 bits are an epoch and the low 12 bits
    // are the code, making dictionary clear O(1).
    entries: Vec<u32>,
    epoch: u32,
    buffer: Vec<u8>,
}

impl FastGifLzwEncoder {
    const KEY_COUNT: usize = 1 << 20;
    const CODE_MASK: u32 = (1 << 12) - 1;
    const MAX_EPOCH: u32 = (1 << 20) - 1;
    const HEAP_BYTES: usize = Self::KEY_COUNT * std::mem::size_of::<u32>();
    const MIN_CODE_SIZE: u8 = 8;
    const CLEAR_CODE: u16 = 1 << Self::MIN_CODE_SIZE;
    const END_CODE: u16 = Self::CLEAR_CODE + 1;
    const FIRST_CODE: u16 = Self::END_CODE + 1;

    fn new() -> Self {
        Self {
            entries: vec![0; Self::KEY_COUNT],
            epoch: 0,
            buffer: Vec::new(),
        }
    }

    #[inline]
    fn reset_dictionary(&mut self) {
        if self.epoch == Self::MAX_EPOCH {
            self.entries.fill(0);
            self.epoch = 1;
        } else {
            self.epoch += 1;
        }
    }

    fn encode(&mut self, pixels: &[u8]) -> Vec<u8> {
        let mut output = std::mem::take(&mut self.buffer);
        output.clear();
        output.reserve(pixels.len() / 4 + 64);
        output.push(Self::MIN_CODE_SIZE);
        self.reset_dictionary();

        let mut writer = GifLsbBitWriter::new(output);
        let mut code_size = Self::MIN_CODE_SIZE + 1;
        let mut next_code = Self::FIRST_CODE;
        writer.write(Self::CLEAR_CODE, code_size);

        let Some((&first, rest)) = pixels.split_first() else {
            writer.write(Self::END_CODE, code_size);
            return writer.finish();
        };
        let mut prefix = first as u16;
        for &suffix in rest {
            let key = ((prefix as u32) << 8) | suffix as u32;
            let entry = self.entries[key as usize];
            if entry >> 12 == self.epoch {
                prefix = (entry & Self::CODE_MASK) as u16;
                continue;
            }

            writer.write(prefix, code_size);
            if next_code < 4096 {
                self.entries[key as usize] = (self.epoch << 12) | next_code as u32;
                next_code += 1;
                if next_code > (1u16 << code_size) && code_size < 12 {
                    code_size += 1;
                }
            } else {
                writer.write(Self::CLEAR_CODE, code_size);
                self.reset_dictionary();
                code_size = Self::MIN_CODE_SIZE + 1;
                next_code = Self::FIRST_CODE;
            }
            prefix = suffix as u16;
        }

        writer.write(prefix, code_size);
        // The decoder adds its final dictionary entry after reading the last
        // data code, so it may cross a width boundary before the end code.
        if next_code >= (1u16 << code_size) && code_size < 12 {
            code_size += 1;
        }
        writer.write(Self::END_CODE, code_size);
        writer.finish()
    }

    fn recycle(&mut self, mut buffer: Vec<u8>) {
        buffer.clear();
        self.buffer = buffer;
    }
}

struct GifLsbBitWriter {
    output: Vec<u8>,
    bits: u64,
    bit_count: u32,
}

impl GifLsbBitWriter {
    fn new(output: Vec<u8>) -> Self {
        Self {
            output,
            bits: 0,
            bit_count: 0,
        }
    }

    #[inline(always)]
    fn write(&mut self, code: u16, width: u8) {
        self.bits |= (code as u64) << self.bit_count;
        self.bit_count += width as u32;
        while self.bit_count >= 8 {
            self.output.push(self.bits as u8);
            self.bits >>= 8;
            self.bit_count -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bit_count != 0 {
            self.output.push(self.bits as u8);
        }
        self.output
    }
}

fn encode_gif<F>(
    spec: &AnimationEncodeSpec,
    expected: usize,
    retention_budget: usize,
    telemetry: &mut AnimationEncodeTelemetry,
    frame_at: &mut F,
) -> Result<Vec<u8>, String>
where
    F: FnMut(u32) -> Result<Vec<u8>, String>,
{
    const MAX_PALETTE_SAMPLE_PIXELS: usize = 65_536;
    let pixels_per_frame = (MAX_PALETTE_SAMPLE_PIXELS / spec.frame_count as usize).max(1);
    let mut quantization_pixels = Vec::with_capacity(MAX_PALETTE_SAMPLE_PIXELS * 4);
    let palette_sample_started = std::time::Instant::now();
    let retained_metadata_bytes =
        spec.frame_count as usize * std::mem::size_of::<GifRetainedFrame>();
    let mut retained = (retention_budget >= retained_metadata_bytes)
        .then(|| Vec::<GifRetainedFrame>::with_capacity(spec.frame_count as usize));
    let mut retained_bytes = retained.as_ref().map_or(0, |_| retained_metadata_bytes);
    let mut retained_peak_bytes = retained_bytes;
    let mut previous: Option<Vec<u8>> = None;
    for index in 0..spec.frame_count {
        let rgba = checked_frame_timed(frame_at, index, expected, telemetry)?;
        let pixel_count = rgba.len() / 4;
        let stride = pixel_count.div_ceil(pixels_per_frame).max(1);
        quantization_pixels.extend(
            rgba.chunks_exact(4)
                .step_by(stride)
                .take(pixels_per_frame)
                .flatten()
                .copied(),
        );
        if let Some(frames) = retained.as_mut() {
            let remaining = retention_budget.saturating_sub(retained_bytes);
            let frame = if let Some(before) = previous.as_deref() {
                try_retain_gif_delta(before, &rgba, spec.width, spec.height, remaining)
            } else if rgba.capacity() <= remaining {
                Some(GifRetainedFrame::Full(rgba.clone()))
            } else {
                None
            };
            if let Some(frame) = frame {
                let frame_bytes = frame.heap_bytes();
                retained_bytes = retained_bytes.saturating_add(frame_bytes);
                retained_peak_bytes = retained_peak_bytes.max(retained_bytes);
                frames.push(frame);
            } else {
                retained = None;
                retained_bytes = 0;
            }
        }
        previous = Some(rgba);
    }
    telemetry.gif_palette_sample_ns = palette_sample_started.elapsed().as_nanos() as u64;
    if quantization_pixels.is_empty() {
        return Err("GIF palette analysis produced no pixels".into());
    }
    let palette_quantize_started = std::time::Instant::now();
    let sampled_pixels = quantization_pixels.len() / 4;
    let quant_width = sampled_pixels.min(256);
    let quant_height = sampled_pixels.div_ceil(quant_width);
    quantization_pixels.resize(quant_width * quant_height * 4, 0);
    let mut palette = build_gif_palette(
        &mut quantization_pixels,
        gif_quantizer_speed(spec.gif_quality),
    );
    // Index 255 is reserved for transparent delta pixels. Training exactly
    // 255 opaque colors avoids silently discarding a color from a 256-entry
    // quantizer result.
    palette.resize(256 * 3, 0);
    telemetry.gif_palette_quantize_ns = palette_quantize_started.elapsed().as_nanos() as u64;
    let active_colors = gif_palette_color_count(spec.gif_quality);
    let lookup_budget = retention_budget
        .saturating_sub(retained_peak_bytes)
        .saturating_sub(FastGifLzwEncoder::HEAP_BYTES);
    let mut palette_lookup = GifPaletteLookup::new(lookup_budget, active_colors);
    palette_lookup.set_palette(&palette);
    let palette_lookup_bytes = palette_lookup.heap_bytes();
    for (index, color) in palette.chunks_exact(3).take(active_colors).enumerate() {
        let key = rgb_key(color);
        if palette_lookup.get(key).is_none() {
            palette_lookup.insert(key, index as u8);
        }
    }
    let mut output = Vec::new();
    {
        // Analyze a bounded sample across the complete sequence, then reuse one
        // global palette during the deterministic encode pass. No full frame is
        // retained between the two passes.
        let mut encoder =
            gif::Encoder::new(&mut output, spec.width as u16, spec.height as u16, &palette)
                .map_err(|error| format!("GIF encoder init: {error}"))?;
        encoder
            .set_repeat(if spec.looped {
                gif::Repeat::Infinite
            } else {
                gif::Repeat::Finite(0)
            })
            .map_err(|error| format!("GIF repeat: {error}"))?;
        telemetry.gif_retained_frame_bytes = retained_peak_bytes
            .saturating_add(palette_lookup_bytes)
            .saturating_add(FastGifLzwEncoder::HEAP_BYTES);
        telemetry.gif_second_frame_pass = retained.is_none();
        let mut previous: Option<Vec<u8>> = None;
        let mut previous_indexed = Vec::new();
        let mut lzw = FastGifLzwEncoder::new();
        for index in 0..spec.frame_count {
            let delta_started = std::time::Instant::now();
            let (left, top, frame_width, frame_height, pixels) = if let Some(frames) = &retained {
                gif_retained_indexed(
                    &frames[index as usize],
                    spec,
                    &palette,
                    &mut palette_lookup,
                    &mut previous_indexed,
                )
            } else {
                let rgba = checked_frame_timed(frame_at, index, expected, telemetry)?;
                let result = if let Some(before) = previous.as_ref() {
                    gif_delta_indexed(
                        before,
                        &rgba,
                        spec.width,
                        spec.height,
                        &palette,
                        &mut palette_lookup,
                    )
                } else {
                    (
                        0,
                        0,
                        spec.width,
                        spec.height,
                        index_gif_pixels(&rgba, &palette, &mut palette_lookup),
                    )
                };
                previous = Some(rgba);
                result
            };
            telemetry.gif_delta_index_ns = telemetry
                .gif_delta_index_ns
                .saturating_add(delta_started.elapsed().as_nanos() as u64);
            let mut frame = gif::Frame::from_indexed_pixels(
                frame_width as u16,
                frame_height as u16,
                pixels,
                Some(255),
            );
            frame.left = left as u16;
            frame.top = top as u16;
            frame.delay = gif_delay_centiseconds(index, spec.fps);
            frame.dispose = gif::DisposalMethod::Keep;
            let lzw_started = std::time::Instant::now();
            let encoded_lzw = lzw.encode(&frame.buffer);
            frame.buffer = std::borrow::Cow::Owned(encoded_lzw);
            encoder
                .write_lzw_pre_encoded_frame(&frame)
                .map_err(|error| format!("GIF frame {index}: {error}"))?;
            if let std::borrow::Cow::Owned(buffer) = frame.buffer {
                lzw.recycle(buffer);
            }
            telemetry.gif_lzw_ns = telemetry
                .gif_lzw_ns
                .saturating_add(lzw_started.elapsed().as_nanos() as u64);
        }
    }
    Ok(output)
}

fn try_retain_gif_delta(
    previous: &[u8],
    current: &[u8],
    width: u32,
    height: u32,
    budget: usize,
) -> Option<GifRetainedFrame> {
    let Some((min_x, min_y, max_x, max_y)) = gif_changed_bounds(previous, current, width, height)
    else {
        return Some(GifRetainedFrame::Unchanged);
    };
    let rect_width = max_x - min_x + 1;
    let rect_height = max_y - min_y + 1;
    let pixel_count = rect_width as usize * rect_height as usize;
    let mut changed_pixel_count = 0usize;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let source_index = (y * width + x) as usize * 4;
            changed_pixel_count += usize::from(
                previous[source_index..source_index + 4] != current[source_index..source_index + 4],
            );
        }
    }
    let rgba_bytes = changed_pixel_count.saturating_mul(4);
    let changed_bytes = pixel_count.div_ceil(8);
    if rgba_bytes.saturating_add(changed_bytes) > budget {
        return None;
    }
    let mut rgba = Vec::with_capacity(rgba_bytes);
    let mut changed = vec![0u8; changed_bytes];
    let mut retained_index = 0usize;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let source_index = (y * width + x) as usize * 4;
            if previous[source_index..source_index + 4] != current[source_index..source_index + 4] {
                changed[retained_index / 8] |= 1 << (retained_index % 8);
                rgba.extend_from_slice(&current[source_index..source_index + 4]);
            }
            retained_index += 1;
        }
    }
    Some(GifRetainedFrame::Delta {
        left: min_x,
        top: min_y,
        width: rect_width,
        height: rect_height,
        rgba,
        changed,
    })
}

fn gif_retained_indexed(
    frame: &GifRetainedFrame,
    spec: &AnimationEncodeSpec,
    palette: &[u8],
    palette_lookup: &mut GifPaletteLookup,
    previous_indexed: &mut Vec<u8>,
) -> (u32, u32, u32, u32, Vec<u8>) {
    match frame {
        GifRetainedFrame::Full(rgba) => {
            let pixels = index_gif_pixels(rgba, palette, palette_lookup);
            previous_indexed.clone_from(&pixels);
            (0, 0, spec.width, spec.height, pixels)
        }
        GifRetainedFrame::Delta {
            left,
            top,
            width,
            height,
            rgba,
            changed,
        } => {
            let mut rgba_index = 0usize;
            let indexed_changed = index_gif_pixels(rgba, palette, palette_lookup);
            let pixel_count = width.saturating_mul(*height) as usize;
            let mut pixels = vec![255u8; pixel_count];
            let mut min_x = *width;
            let mut min_y = *height;
            let mut max_x = 0u32;
            let mut max_y = 0u32;
            for (byte_index, &byte) in changed.iter().enumerate() {
                let mut bits = byte;
                while bits != 0 {
                    let bit = bits.trailing_zeros() as usize;
                    let index = byte_index * 8 + bit;
                    bits &= bits - 1;
                    if index >= pixel_count {
                        continue;
                    }
                    let x = index as u32 % *width;
                    let y = index as u32 / *width;
                    let value = indexed_changed[rgba_index / 4];
                    rgba_index += 4;
                    let canvas_index = ((*top + y) * spec.width + *left + x) as usize;
                    if previous_indexed[canvas_index] != value {
                        previous_indexed[canvas_index] = value;
                        pixels[index] = value;
                        min_x = min_x.min(x);
                        min_y = min_y.min(y);
                        max_x = max_x.max(x);
                        max_y = max_y.max(y);
                    }
                }
            }
            debug_assert_eq!(rgba_index, rgba.len());
            if min_x == *width || min_y == *height {
                return (0, 0, 1, 1, vec![255]);
            }
            if min_x != 0 || min_y != 0 || max_x + 1 != *width || max_y + 1 != *height {
                let cropped_width = max_x - min_x + 1;
                let cropped_height = max_y - min_y + 1;
                let mut cropped =
                    Vec::with_capacity(cropped_width as usize * cropped_height as usize);
                for y in min_y..=max_y {
                    let start = (y * *width + min_x) as usize;
                    cropped.extend_from_slice(&pixels[start..start + cropped_width as usize]);
                }
                return (
                    *left + min_x,
                    *top + min_y,
                    cropped_width,
                    cropped_height,
                    cropped,
                );
            }
            (*left, *top, *width, *height, pixels)
        }
        GifRetainedFrame::Unchanged => (0, 0, 1, 1, vec![255]),
    }
}

fn encode_mp4<F>(
    spec: &AnimationEncodeSpec,
    expected: usize,
    config: &H264EncoderConfig,
    telemetry: &mut AnimationEncodeTelemetry,
    frame_at: &mut F,
) -> Result<Vec<u8>, String>
where
    F: FnMut(u32) -> Result<Vec<u8>, String>,
{
    encode_mp4_into(spec, expected, config, telemetry, |index, rgba| {
        *rgba = checked_frame(frame_at, index, expected)?;
        Ok(FrameCompositeUpdate::Full)
    })
}

#[repr(C)]
struct NativeX264Encoder {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn allium_x264_create(
        width: std::ffi::c_int,
        height: std::ffi::c_int,
        fps: std::ffi::c_int,
        crf: std::ffi::c_int,
    ) -> *mut NativeX264Encoder;
    fn allium_x264_encode(
        encoder: *mut NativeX264Encoder,
        yuv420p: *const u8,
        mb_info: *const u8,
        pts: i64,
        output: *mut *const u8,
        output_size: *mut usize,
    ) -> std::ffi::c_int;
    fn allium_x264_flush(
        encoder: *mut NativeX264Encoder,
        output: *mut *const u8,
        output_size: *mut usize,
    ) -> std::ffi::c_int;
    fn allium_x264_delayed_frames(encoder: *const NativeX264Encoder) -> std::ffi::c_int;
    fn allium_x264_destroy(encoder: *mut NativeX264Encoder);
}

struct DirectX264Encoder(*mut NativeX264Encoder);

impl DirectX264Encoder {
    fn new(spec: &AnimationEncodeSpec, config: &H264EncoderConfig) -> Result<Self, String> {
        let encoder = unsafe {
            allium_x264_create(
                spec.width as std::ffi::c_int,
                spec.height as std::ffi::c_int,
                spec.fps as std::ffi::c_int,
                config.qp as std::ffi::c_int,
            )
        };
        if encoder.is_null() {
            Err("initialize direct libx264 encoder failed".into())
        } else {
            Ok(Self(encoder))
        }
    }

    fn encode(
        &mut self,
        yuv: &[u8],
        mb_info: &[u8],
        pts: i64,
        output: &mut Vec<u8>,
    ) -> Result<(), String> {
        let mut data = std::ptr::null();
        let mut size = 0usize;
        let encoded = unsafe {
            allium_x264_encode(
                self.0,
                yuv.as_ptr(),
                mb_info.as_ptr(),
                pts,
                &mut data,
                &mut size,
            )
        };
        if encoded < 0 || (size != 0 && data.is_null()) {
            return Err(format!("direct libx264 frame {pts} failed"));
        }
        if size != 0 {
            output.extend_from_slice(unsafe { std::slice::from_raw_parts(data, size) });
        }
        Ok(())
    }

    fn flush(&mut self, output: &mut Vec<u8>) -> Result<(), String> {
        while unsafe { allium_x264_delayed_frames(self.0) } > 0 {
            let mut data = std::ptr::null();
            let mut size = 0usize;
            let encoded = unsafe { allium_x264_flush(self.0, &mut data, &mut size) };
            if encoded < 0 || (size != 0 && data.is_null()) {
                return Err("flush direct libx264 encoder failed".into());
            }
            if size != 0 {
                output.extend_from_slice(unsafe { std::slice::from_raw_parts(data, size) });
            }
        }
        Ok(())
    }
}

impl Drop for DirectX264Encoder {
    fn drop(&mut self) {
        unsafe { allium_x264_destroy(self.0) };
    }
}

fn encode_mp4_into<F>(
    spec: &AnimationEncodeSpec,
    expected: usize,
    config: &H264EncoderConfig,
    telemetry: &mut AnimationEncodeTelemetry,
    mut fill_frame: F,
) -> Result<Vec<u8>, String>
where
    F: FnMut(u32, &mut Vec<u8>) -> Result<FrameCompositeUpdate, String>,
{
    if spec.width % 2 != 0 || spec.height % 2 != 0 {
        return Err("H.264 yuv420p requires even animation dimensions".into());
    }
    config.validate()?;
    let mut encoder = DirectX264Encoder::new(spec, config)?;
    let mut access_units = Vec::with_capacity(spec.frame_count as usize);
    let mut yuv = Vec::new();
    let mut rgba = Vec::new();
    let mut mb_info = Vec::new();
    for index in 0..spec.frame_count {
        let frame_started = std::time::Instant::now();
        let update = fill_frame(index, &mut rgba)?;
        telemetry.frame_callback_calls += 1;
        telemetry.frame_callback_ns = telemetry
            .frame_callback_ns
            .saturating_add(frame_started.elapsed().as_nanos() as u64);
        if rgba.len() != expected {
            return Err(format!(
                "frame {index} has {} bytes, expected {expected}",
                rgba.len()
            ));
        }
        let yuv_started = std::time::Instant::now();
        match &update {
            FrameCompositeUpdate::Full => {
                rgba_to_yuv420p_into(rgba.as_slice(), spec.width, spec.height, config, &mut yuv)?;
            }
            FrameCompositeUpdate::Dirty(changed_macroblocks) => {
                rgba_to_yuv420p_macroblocks_into(
                    rgba.as_slice(),
                    spec.width,
                    spec.height,
                    changed_macroblocks,
                    config,
                    &mut yuv,
                )?;
            }
            FrameCompositeUpdate::Reused => {}
        }
        telemetry.h264_yuv_ns = telemetry
            .h264_yuv_ns
            .saturating_add(yuv_started.elapsed().as_nanos() as u64);
        let (constant_blocks, total_blocks) =
            fill_x264_mb_info(&update, spec.width, spec.height, &mut mb_info)?;
        telemetry.h264_mbinfo_constant_blocks = telemetry
            .h264_mbinfo_constant_blocks
            .saturating_add(constant_blocks as u64);
        telemetry.h264_mbinfo_total_blocks = telemetry
            .h264_mbinfo_total_blocks
            .saturating_add(total_blocks as u64);
        let encode_started = std::time::Instant::now();
        let mut access_unit = Vec::new();
        encoder.encode(&yuv, &mb_info, index as i64, &mut access_unit)?;
        if access_unit.is_empty() {
            return Err(format!("direct libx264 delayed frame {index}"));
        }
        access_units.push(access_unit);
        telemetry.h264_direct_encode_ns = telemetry
            .h264_direct_encode_ns
            .saturating_add(encode_started.elapsed().as_nanos() as u64);
    }
    let mut delayed = Vec::new();
    encoder.flush(&mut delayed)?;
    if !delayed.is_empty() {
        return Err("direct libx264 produced an unexpected delayed access unit".into());
    }
    drop(encoder);

    let mux_started = std::time::Instant::now();
    let data = mux_avc_mp4(spec.width, spec.height, spec.fps, &access_units)?;
    telemetry.h264_mux_ns = mux_started.elapsed().as_nanos() as u64;
    Ok(data)
}

fn fill_x264_mb_info(
    update: &FrameCompositeUpdate,
    width: u32,
    height: u32,
    output: &mut Vec<u8>,
) -> Result<(usize, usize), String> {
    const X264_MBINFO_CONSTANT: u8 = 1;
    let mb_width = width.div_ceil(16) as usize;
    let mb_height = height.div_ceil(16) as usize;
    let total = mb_width
        .checked_mul(mb_height)
        .ok_or_else(|| "H.264 macroblock map size overflow".to_string())?;
    output.resize(total, X264_MBINFO_CONSTANT);
    output.fill(X264_MBINFO_CONSTANT);
    match update {
        FrameCompositeUpdate::Full => output.fill(0),
        FrameCompositeUpdate::Reused => {}
        FrameCompositeUpdate::Dirty(changed_macroblocks) => {
            for &index in changed_macroblocks {
                let index = index as usize;
                if index >= total {
                    return Err("H.264 changed macroblock index is out of range".into());
                }
                output[index] = 0;
            }
        }
    }
    Ok((
        output
            .iter()
            .filter(|&&flags| flags & X264_MBINFO_CONSTANT != 0)
            .count(),
        total,
    ))
}

fn mux_avc_mp4(
    width: u32,
    height: u32,
    fps: u32,
    access_units: &[Vec<u8>],
) -> Result<Vec<u8>, String> {
    let mut sps = None;
    let mut pps = None;
    let mut samples = Vec::with_capacity(access_units.len());
    let mut sync_samples = Vec::new();
    for (sample_index, access_unit) in access_units.iter().enumerate() {
        let mut sample = Vec::new();
        let mut sync = false;
        for nal in annex_b_nals(access_unit) {
            if nal.is_empty() {
                continue;
            }
            match nal[0] & 0x1f {
                7 => {
                    sps.get_or_insert_with(|| nal.to_vec());
                }
                8 => {
                    pps.get_or_insert_with(|| nal.to_vec());
                }
                9 => continue,
                5 => sync = true,
                _ => {}
            };
            if matches!(nal[0] & 0x1f, 7 | 8) {
                continue;
            }
            mp4_u32(
                &mut sample,
                u32::try_from(nal.len()).map_err(|_| "AVC NAL too large")?,
            );
            sample.extend_from_slice(nal);
        }
        if sample.is_empty() {
            return Err(format!("AVC access unit {sample_index} has no sample NAL"));
        }
        if sync {
            sync_samples.push(sample_index as u32 + 1);
        }
        samples.push(sample);
    }
    let sps = sps.ok_or_else(|| "AVC stream is missing SPS".to_string())?;
    let pps = pps.ok_or_else(|| "AVC stream is missing PPS".to_string())?;
    if sps.len() < 4
        || access_units.is_empty()
        || fps == 0
        || width > u16::MAX as u32
        || height > u16::MAX as u32
    {
        return Err("invalid AVC MP4 track parameters".into());
    }

    let ftyp = mp4_box(*b"ftyp", |payload| {
        payload.extend_from_slice(b"isom");
        mp4_u32(payload, 512);
        payload.extend_from_slice(b"isomiso2avc1mp41");
    })?;
    let sample_sizes = samples
        .iter()
        .map(|sample| u32::try_from(sample.len()).map_err(|_| "AVC sample too large".to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let mdat_payload_bytes = sample_sizes.iter().try_fold(0u32, |total, &size| {
        total
            .checked_add(size)
            .ok_or_else(|| "AVC mdat exceeds 4GiB".to_string())
    })?;
    let mdat_data_offset = u32::try_from(ftyp.len() + 8).map_err(|_| "AVC mdat offset overflow")?;
    let duration = u32::try_from(samples.len()).map_err(|_| "AVC frame count overflow")?;
    let moov = build_avc_moov(
        width as u16,
        height as u16,
        fps,
        duration,
        &sps,
        &pps,
        &sample_sizes,
        &sync_samples,
        mdat_data_offset,
    )?;
    let mut output = Vec::with_capacity(ftyp.len() + 8 + mdat_payload_bytes as usize + moov.len());
    output.extend_from_slice(&ftyp);
    mp4_u32(
        &mut output,
        mdat_payload_bytes
            .checked_add(8)
            .ok_or("AVC mdat size overflow")?,
    );
    output.extend_from_slice(b"mdat");
    for sample in samples {
        output.extend_from_slice(&sample);
    }
    output.extend_from_slice(&moov);
    Ok(output)
}

fn annex_b_nals(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut index = 0usize;
    while index + 3 <= data.len() {
        let prefix = if index + 4 <= data.len() && data[index..index + 4] == [0, 0, 0, 1] {
            4
        } else if data[index..index + 3] == [0, 0, 1] {
            3
        } else {
            index += 1;
            continue;
        };
        starts.push((index, prefix));
        index += prefix;
    }
    starts
        .iter()
        .enumerate()
        .filter_map(|(position, &(start, prefix))| {
            let end = starts
                .get(position + 1)
                .map_or(data.len(), |&(next, _)| next);
            (start + prefix < end).then_some(&data[start + prefix..end])
        })
        .collect()
}

fn mp4_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn mp4_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn mp4_box(kind: [u8; 4], fill: impl FnOnce(&mut Vec<u8>)) -> Result<Vec<u8>, String> {
    let mut output = vec![0, 0, 0, 0];
    output.extend_from_slice(&kind);
    fill(&mut output);
    let size = u32::try_from(output.len()).map_err(|_| "MP4 box exceeds 4GiB".to_string())?;
    output[..4].copy_from_slice(&size.to_be_bytes());
    Ok(output)
}

fn mp4_full_box(
    kind: [u8; 4],
    version_and_flags: u32,
    fill: impl FnOnce(&mut Vec<u8>),
) -> Result<Vec<u8>, String> {
    mp4_box(kind, |output| {
        mp4_u32(output, version_and_flags);
        fill(output);
    })
}

#[allow(clippy::too_many_arguments)]
fn build_avc_moov(
    width: u16,
    height: u16,
    timescale: u32,
    duration: u32,
    sps: &[u8],
    pps: &[u8],
    sample_sizes: &[u32],
    sync_samples: &[u32],
    chunk_offset: u32,
) -> Result<Vec<u8>, String> {
    let matrix = [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];
    let mvhd = mp4_full_box(*b"mvhd", 0, |output| {
        mp4_u32(output, 0);
        mp4_u32(output, 0);
        mp4_u32(output, timescale);
        mp4_u32(output, duration);
        mp4_u32(output, 0x0001_0000);
        mp4_u16(output, 0x0100);
        output.extend_from_slice(&[0; 10]);
        for value in matrix {
            mp4_u32(output, value);
        }
        output.extend_from_slice(&[0; 24]);
        mp4_u32(output, 2);
    })?;
    let tkhd = mp4_full_box(*b"tkhd", 7, |output| {
        mp4_u32(output, 0);
        mp4_u32(output, 0);
        mp4_u32(output, 1);
        mp4_u32(output, 0);
        mp4_u32(output, duration);
        output.extend_from_slice(&[0; 8]);
        mp4_u16(output, 0);
        mp4_u16(output, 0);
        mp4_u16(output, 0);
        mp4_u16(output, 0);
        for value in matrix {
            mp4_u32(output, value);
        }
        mp4_u32(output, u32::from(width) << 16);
        mp4_u32(output, u32::from(height) << 16);
    })?;
    let mdhd = mp4_full_box(*b"mdhd", 0, |output| {
        mp4_u32(output, 0);
        mp4_u32(output, 0);
        mp4_u32(output, timescale);
        mp4_u32(output, duration);
        mp4_u16(output, 0x55c4);
        mp4_u16(output, 0);
    })?;
    let hdlr = mp4_full_box(*b"hdlr", 0, |output| {
        mp4_u32(output, 0);
        output.extend_from_slice(b"vide");
        output.extend_from_slice(&[0; 12]);
        output.extend_from_slice(b"VideoHandler\0");
    })?;
    let vmhd = mp4_full_box(*b"vmhd", 1, |output| output.extend_from_slice(&[0; 8]))?;
    let url = mp4_full_box(*b"url ", 1, |_| {})?;
    let dref = mp4_full_box(*b"dref", 0, |output| {
        mp4_u32(output, 1);
        output.extend_from_slice(&url);
    })?;
    let dinf = mp4_box(*b"dinf", |output| output.extend_from_slice(&dref))?;
    let avcc = mp4_box(*b"avcC", |output| {
        output.extend_from_slice(&[1, sps[1], sps[2], sps[3], 0xff, 0xe1]);
        mp4_u16(output, sps.len() as u16);
        output.extend_from_slice(sps);
        output.push(1);
        mp4_u16(output, pps.len() as u16);
        output.extend_from_slice(pps);
    })?;
    let avc1 = mp4_box(*b"avc1", |output| {
        output.extend_from_slice(&[0; 6]);
        mp4_u16(output, 1);
        output.extend_from_slice(&[0; 16]);
        mp4_u16(output, width);
        mp4_u16(output, height);
        mp4_u32(output, 0x0048_0000);
        mp4_u32(output, 0x0048_0000);
        mp4_u32(output, 0);
        mp4_u16(output, 1);
        let mut compressor = [0u8; 32];
        let name = b"allium libx264";
        compressor[0] = name.len() as u8;
        compressor[1..1 + name.len()].copy_from_slice(name);
        output.extend_from_slice(&compressor);
        mp4_u16(output, 0x0018);
        mp4_u16(output, u16::MAX);
        output.extend_from_slice(&avcc);
    })?;
    let stsd = mp4_full_box(*b"stsd", 0, |output| {
        mp4_u32(output, 1);
        output.extend_from_slice(&avc1);
    })?;
    let stts = mp4_full_box(*b"stts", 0, |output| {
        mp4_u32(output, 1);
        mp4_u32(output, sample_sizes.len() as u32);
        mp4_u32(output, 1);
    })?;
    let stsc = mp4_full_box(*b"stsc", 0, |output| {
        mp4_u32(output, 1);
        mp4_u32(output, 1);
        mp4_u32(output, sample_sizes.len() as u32);
        mp4_u32(output, 1);
    })?;
    let stsz = mp4_full_box(*b"stsz", 0, |output| {
        mp4_u32(output, 0);
        mp4_u32(output, sample_sizes.len() as u32);
        for &size in sample_sizes {
            mp4_u32(output, size);
        }
    })?;
    let stco = mp4_full_box(*b"stco", 0, |output| {
        mp4_u32(output, 1);
        mp4_u32(output, chunk_offset);
    })?;
    let stss = mp4_full_box(*b"stss", 0, |output| {
        mp4_u32(output, sync_samples.len() as u32);
        for &sample in sync_samples {
            mp4_u32(output, sample);
        }
    })?;
    let stbl = mp4_box(*b"stbl", |output| {
        for child in [&stsd, &stts, &stsc, &stsz, &stco, &stss] {
            output.extend_from_slice(child);
        }
    })?;
    let minf = mp4_box(*b"minf", |output| {
        output.extend_from_slice(&vmhd);
        output.extend_from_slice(&dinf);
        output.extend_from_slice(&stbl);
    })?;
    let mdia = mp4_box(*b"mdia", |output| {
        output.extend_from_slice(&mdhd);
        output.extend_from_slice(&hdlr);
        output.extend_from_slice(&minf);
    })?;
    let trak = mp4_box(*b"trak", |output| {
        output.extend_from_slice(&tkhd);
        output.extend_from_slice(&mdia);
    })?;
    mp4_box(*b"moov", |output| {
        output.extend_from_slice(&mvhd);
        output.extend_from_slice(&trak);
    })
}

fn rgba_to_yuv420p(
    rgba: &[u8],
    width: u32,
    height: u32,
    config: &H264EncoderConfig,
) -> Result<Vec<u8>, String> {
    let mut yuv = Vec::new();
    rgba_to_yuv420p_into(rgba, width, height, config, &mut yuv)?;
    Ok(yuv)
}

fn rgba_to_yuv420p_into(
    rgba: &[u8],
    width: u32,
    height: u32,
    config: &H264EncoderConfig,
    yuv: &mut Vec<u8>,
) -> Result<(), String> {
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err("YUV420 conversion requires positive even dimensions".into());
    }
    let pixels = width as usize * height as usize;
    if rgba.len() != pixels * 4 {
        return Err("YUV420 conversion received a truncated RGBA frame".into());
    }
    yuv.resize(pixels + pixels / 2, 0);
    config.validate()?;
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
        && config.yuv_converter != H264YuvConverter::Scalar
    {
        // SAFETY: runtime feature detection satisfies the target-feature
        // contract and the output size/dimensions were checked above.
        unsafe { rgba_to_yuv420p_avx512(rgba, width as usize, height as usize, yuv) };
        if config.validate_avx512 {
            let mut scalar = vec![0u8; yuv.len()];
            rgba_to_yuv420p_scalar(rgba, width as usize, height as usize, &mut scalar);
            if let Some((offset, (&actual, &expected))) = yuv
                .iter()
                .zip(&scalar)
                .enumerate()
                .find(|(_, (actual, expected))| actual != expected)
            {
                return Err(format!(
                    "H.264 AVX-512 YUV mismatch at byte {offset}: {actual} != {expected}"
                ));
            }
        }
        return Ok(());
    }
    if config.require_avx512
        || config.validate_avx512
        || config.yuv_converter == H264YuvConverter::Avx512
    {
        return Err("H.264 AVX-512 RGBA-to-YUV420 is unavailable".into());
    }
    rgba_to_yuv420p_scalar(rgba, width as usize, height as usize, yuv);
    Ok(())
}

fn rgba_to_yuv420p_macroblocks_into(
    rgba: &[u8],
    width: u32,
    height: u32,
    changed_macroblocks: &[u32],
    config: &H264EncoderConfig,
    yuv: &mut Vec<u8>,
) -> Result<(), String> {
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err("YUV420 macroblock conversion requires positive even dimensions".into());
    }
    let pixels = width as usize * height as usize;
    if rgba.len() != pixels * 4 || yuv.len() != pixels + pixels / 2 {
        return Err("YUV420 macroblock conversion requires an initialized frame".into());
    }
    config.validate()?;
    let mb_count = width.div_ceil(16) as usize * height.div_ceil(16) as usize;
    if changed_macroblocks
        .iter()
        .any(|&index| index as usize >= mb_count)
    {
        return Err("YUV420 changed macroblock index is out of range".into());
    }
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
        && config.yuv_converter != H264YuvConverter::Scalar
    {
        let scalar_oracle = config.validate_avx512.then(|| yuv.clone());
        // SAFETY: runtime feature detection and the bounds checks above satisfy
        // the target-feature and memory contracts.
        unsafe {
            rgba_to_yuv420p_macroblocks_avx512(
                rgba,
                width as usize,
                height as usize,
                changed_macroblocks,
                yuv,
            )
        };
        if let Some(mut scalar) = scalar_oracle {
            rgba_to_yuv420p_macroblocks_scalar(
                rgba,
                width as usize,
                height as usize,
                changed_macroblocks,
                &mut scalar,
            );
            if let Some((offset, (&actual, &expected))) = yuv
                .iter()
                .zip(&scalar)
                .enumerate()
                .find(|(_, (actual, expected))| actual != expected)
            {
                return Err(format!(
                    "H.264 AVX-512 macroblock YUV mismatch at byte {offset}: {actual} != {expected}"
                ));
            }
        }
        return Ok(());
    }
    if config.require_avx512
        || config.validate_avx512
        || config.yuv_converter == H264YuvConverter::Avx512
    {
        return Err("H.264 AVX-512 macroblock RGBA-to-YUV420 is unavailable".into());
    }
    rgba_to_yuv420p_macroblocks_scalar(
        rgba,
        width as usize,
        height as usize,
        changed_macroblocks,
        yuv,
    );
    Ok(())
}

fn rgba_to_yuv420p_macroblocks_scalar(
    rgba: &[u8],
    width: usize,
    height: usize,
    changed_macroblocks: &[u32],
    yuv: &mut [u8],
) {
    let mb_width = width.div_ceil(16);
    for &index in changed_macroblocks {
        let index = index as usize;
        let x = index % mb_width * 16;
        let y = index / mb_width * 16;
        rgba_to_yuv420p_block_scalar(
            rgba,
            width,
            height,
            x,
            y,
            (width - x).min(16),
            (height - y).min(16),
            yuv,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn rgba_to_yuv420p_block_scalar(
    rgba: &[u8],
    width: usize,
    _height: usize,
    start_x: usize,
    start_y: usize,
    block_width: usize,
    block_height: usize,
    yuv: &mut [u8],
) {
    let pixels = yuv.len() * 2 / 3;
    let (y_plane, chroma) = yuv.split_at_mut(pixels);
    let (u_plane, v_plane) = chroma.split_at_mut(pixels / 4);
    for y in (start_y..start_y + block_height).step_by(2) {
        for x in (start_x..start_x + block_width).step_by(2) {
            let mut r_sum = 0i32;
            let mut g_sum = 0i32;
            let mut b_sum = 0i32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let source = ((y + dy) * width + x + dx) * 4;
                    let r = rgba[source] as i32;
                    let g = rgba[source + 1] as i32;
                    let b = rgba[source + 2] as i32;
                    y_plane[(y + dy) * width + x + dx] = rgb_to_limited_y(r, g, b);
                    r_sum += r;
                    g_sum += g;
                    b_sum += b;
                }
            }
            let chroma_index = (y / 2) * (width / 2) + x / 2;
            u_plane[chroma_index] =
                rgb_to_limited_u((r_sum + 2) >> 2, (g_sum + 2) >> 2, (b_sum + 2) >> 2);
            v_plane[chroma_index] =
                rgb_to_limited_v((r_sum + 2) >> 2, (g_sum + 2) >> 2, (b_sum + 2) >> 2);
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn rgba_to_yuv420p_macroblocks_avx512(
    rgba: &[u8],
    width: usize,
    height: usize,
    changed_macroblocks: &[u32],
    yuv: &mut [u8],
) {
    use std::arch::x86_64::*;
    let pixels = width * height;
    let y_plane = yuv.as_mut_ptr();
    let u_plane = y_plane.add(pixels);
    let v_plane = u_plane.add(pixels / 4);
    let even = _mm512_setr_epi32(0, 2, 4, 6, 8, 10, 12, 14, 0, 0, 0, 0, 0, 0, 0, 0);
    let odd = _mm512_setr_epi32(1, 3, 5, 7, 9, 11, 13, 15, 0, 0, 0, 0, 0, 0, 0, 0);
    let mb_width = width.div_ceil(16);
    for &index in changed_macroblocks {
        let index = index as usize;
        let start_x = index % mb_width * 16;
        let start_y = index / mb_width * 16;
        let block_width = (width - start_x).min(16);
        let block_height = (height - start_y).min(16);
        if block_width != 16 {
            rgba_to_yuv420p_block_scalar(
                rgba,
                width,
                height,
                start_x,
                start_y,
                block_width,
                block_height,
                yuv,
            );
            continue;
        }
        for y in (start_y..start_y + block_height).step_by(2) {
            let top = _mm512_loadu_si512(rgba.as_ptr().add((y * width + start_x) * 4).cast());
            let bottom =
                _mm512_loadu_si512(rgba.as_ptr().add(((y + 1) * width + start_x) * 4).cast());
            let (top_r, top_g, top_b) = avx512_rgb_channels(top);
            let (bottom_r, bottom_g, bottom_b) = avx512_rgb_channels(bottom);
            _mm_storeu_si128(
                y_plane.add(y * width + start_x).cast(),
                avx512_limited_y(top_r, top_g, top_b),
            );
            _mm_storeu_si128(
                y_plane.add((y + 1) * width + start_x).cast(),
                avx512_limited_y(bottom_r, bottom_g, bottom_b),
            );
            let r = avx512_pair_average(top_r, bottom_r, even, odd);
            let g = avx512_pair_average(top_g, bottom_g, even, odd);
            let b = avx512_pair_average(top_b, bottom_b, even, odd);
            let chroma = (y / 2) * (width / 2) + start_x / 2;
            _mm_storel_epi64(u_plane.add(chroma).cast(), avx512_limited_u(r, g, b));
            _mm_storel_epi64(v_plane.add(chroma).cast(), avx512_limited_v(r, g, b));
        }
    }
}

fn rgba_to_yuv420p_scalar(rgba: &[u8], width: usize, height: usize, yuv: &mut [u8]) {
    let pixels = width * height;
    let (y_plane, chroma) = yuv.split_at_mut(pixels);
    let (u_plane, v_plane) = chroma.split_at_mut(pixels / 4);
    for y in (0..height).step_by(2) {
        for x in (0..width).step_by(2) {
            let mut r_sum = 0i32;
            let mut g_sum = 0i32;
            let mut b_sum = 0i32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let source = ((y + dy) * width + x + dx) * 4;
                    let r = rgba[source] as i32;
                    let g = rgba[source + 1] as i32;
                    let b = rgba[source + 2] as i32;
                    y_plane[(y + dy) * width + x + dx] = rgb_to_limited_y(r, g, b);
                    r_sum += r;
                    g_sum += g;
                    b_sum += b;
                }
            }
            let r = (r_sum + 2) >> 2;
            let g = (g_sum + 2) >> 2;
            let b = (b_sum + 2) >> 2;
            let chroma_index = (y / 2) * (width / 2) + x / 2;
            u_plane[chroma_index] = rgb_to_limited_u(r, g, b);
            v_plane[chroma_index] = rgb_to_limited_v(r, g, b);
        }
    }
}

fn rgb_to_limited_y(r: i32, g: i32, b: i32) -> u8 {
    (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16).clamp(0, 255) as u8
}

fn rgb_to_limited_u(r: i32, g: i32, b: i32) -> u8 {
    (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255) as u8
}

fn rgb_to_limited_v(r: i32, g: i32, b: i32) -> u8 {
    (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255) as u8
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
#[inline(never)]
unsafe fn rgba_to_yuv420p_avx512(rgba: &[u8], width: usize, height: usize, yuv: &mut [u8]) {
    use std::arch::x86_64::*;

    let pixels = width * height;
    let y_plane = yuv.as_mut_ptr();
    let u_plane = y_plane.add(pixels);
    let v_plane = u_plane.add(pixels / 4);
    let even = _mm512_setr_epi32(0, 2, 4, 6, 8, 10, 12, 14, 0, 0, 0, 0, 0, 0, 0, 0);
    let odd = _mm512_setr_epi32(1, 3, 5, 7, 9, 11, 13, 15, 0, 0, 0, 0, 0, 0, 0, 0);
    // Thirty-two RGBA pixels produce sixteen 2x2 chroma samples. Keeping two
    // adjacent 16-pixel blocks together therefore gives both luma and chroma
    // arithmetic a genuinely full 512-bit, sixteen-i32-lane batch.
    let simd_width = width & !31;
    for y in (0..height).step_by(2) {
        let top = rgba.as_ptr().add(y * width * 4);
        let bottom = top.add(width * 4);
        let mut x = 0usize;
        while x < simd_width {
            let top_pixels0 = _mm512_loadu_si512(top.add(x * 4).cast());
            let top_pixels1 = _mm512_loadu_si512(top.add((x + 16) * 4).cast());
            let bottom_pixels0 = _mm512_loadu_si512(bottom.add(x * 4).cast());
            let bottom_pixels1 = _mm512_loadu_si512(bottom.add((x + 16) * 4).cast());
            let (top_r0, top_g0, top_b0) = avx512_rgb_channels(top_pixels0);
            let (top_r1, top_g1, top_b1) = avx512_rgb_channels(top_pixels1);
            let (bottom_r0, bottom_g0, bottom_b0) = avx512_rgb_channels(bottom_pixels0);
            let (bottom_r1, bottom_g1, bottom_b1) = avx512_rgb_channels(bottom_pixels1);

            _mm_storeu_si128(
                y_plane.add(y * width + x).cast(),
                avx512_limited_y(top_r0, top_g0, top_b0),
            );
            _mm_storeu_si128(
                y_plane.add(y * width + x + 16).cast(),
                avx512_limited_y(top_r1, top_g1, top_b1),
            );
            _mm_storeu_si128(
                y_plane.add((y + 1) * width + x).cast(),
                avx512_limited_y(bottom_r0, bottom_g0, bottom_b0),
            );
            _mm_storeu_si128(
                y_plane.add((y + 1) * width + x + 16).cast(),
                avx512_limited_y(bottom_r1, bottom_g1, bottom_b1),
            );

            let r0 = avx512_pair_average(top_r0, bottom_r0, even, odd);
            let r1 = avx512_pair_average(top_r1, bottom_r1, even, odd);
            let g0 = avx512_pair_average(top_g0, bottom_g0, even, odd);
            let g1 = avx512_pair_average(top_g1, bottom_g1, even, odd);
            let b0 = avx512_pair_average(top_b0, bottom_b0, even, odd);
            let b1 = avx512_pair_average(top_b1, bottom_b1, even, odd);
            let r = _mm512_inserti64x4::<1>(r0, _mm512_castsi512_si256(r1));
            let g = _mm512_inserti64x4::<1>(g0, _mm512_castsi512_si256(g1));
            let b = _mm512_inserti64x4::<1>(b0, _mm512_castsi512_si256(b1));
            let u = avx512_limited_u(r, g, b);
            let v = avx512_limited_v(r, g, b);
            let chroma = (y / 2) * (width / 2) + x / 2;
            _mm_storeu_si128(u_plane.add(chroma).cast(), u);
            _mm_storeu_si128(v_plane.add(chroma).cast(), v);
            x += 32;
        }

        for x in (simd_width..width).step_by(2) {
            let mut r_sum = 0i32;
            let mut g_sum = 0i32;
            let mut b_sum = 0i32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let source = ((y + dy) * width + x + dx) * 4;
                    let r = rgba[source] as i32;
                    let g = rgba[source + 1] as i32;
                    let b = rgba[source + 2] as i32;
                    *y_plane.add((y + dy) * width + x + dx) = rgb_to_limited_y(r, g, b);
                    r_sum += r;
                    g_sum += g;
                    b_sum += b;
                }
            }
            let r = (r_sum + 2) >> 2;
            let g = (g_sum + 2) >> 2;
            let b = (b_sum + 2) >> 2;
            let chroma = (y / 2) * (width / 2) + x / 2;
            *u_plane.add(chroma) = rgb_to_limited_u(r, g, b);
            *v_plane.add(chroma) = rgb_to_limited_v(r, g, b);
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
unsafe fn avx512_rgb_channels(
    pixels: std::arch::x86_64::__m512i,
) -> (
    std::arch::x86_64::__m512i,
    std::arch::x86_64::__m512i,
    std::arch::x86_64::__m512i,
) {
    use std::arch::x86_64::*;
    let mask = _mm512_set1_epi32(255);
    (
        _mm512_and_si512(pixels, mask),
        _mm512_and_si512(_mm512_srli_epi32::<8>(pixels), mask),
        _mm512_and_si512(_mm512_srli_epi32::<16>(pixels), mask),
    )
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn avx512_limited_y(
    r: std::arch::x86_64::__m512i,
    g: std::arch::x86_64::__m512i,
    b: std::arch::x86_64::__m512i,
) -> std::arch::x86_64::__m128i {
    use std::arch::x86_64::*;
    let value = _mm512_add_epi32(
        _mm512_srli_epi32::<8>(_mm512_add_epi32(
            _mm512_add_epi32(
                _mm512_add_epi32(
                    _mm512_mullo_epi32(r, _mm512_set1_epi32(66)),
                    _mm512_mullo_epi32(g, _mm512_set1_epi32(129)),
                ),
                _mm512_mullo_epi32(b, _mm512_set1_epi32(25)),
            ),
            _mm512_set1_epi32(128),
        )),
        _mm512_set1_epi32(16),
    );
    _mm512_cvtusepi32_epi8(value)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
unsafe fn avx512_pair_average(
    top: std::arch::x86_64::__m512i,
    bottom: std::arch::x86_64::__m512i,
    even: std::arch::x86_64::__m512i,
    odd: std::arch::x86_64::__m512i,
) -> std::arch::x86_64::__m512i {
    use std::arch::x86_64::*;
    _mm512_srli_epi32::<2>(_mm512_add_epi32(
        _mm512_add_epi32(
            _mm512_add_epi32(
                _mm512_permutexvar_epi32(even, top),
                _mm512_permutexvar_epi32(odd, top),
            ),
            _mm512_add_epi32(
                _mm512_permutexvar_epi32(even, bottom),
                _mm512_permutexvar_epi32(odd, bottom),
            ),
        ),
        _mm512_set1_epi32(2),
    ))
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn avx512_limited_u(
    r: std::arch::x86_64::__m512i,
    g: std::arch::x86_64::__m512i,
    b: std::arch::x86_64::__m512i,
) -> std::arch::x86_64::__m128i {
    use std::arch::x86_64::*;
    let value = _mm512_add_epi32(
        _mm512_srai_epi32::<8>(_mm512_add_epi32(
            _mm512_add_epi32(
                _mm512_add_epi32(
                    _mm512_mullo_epi32(r, _mm512_set1_epi32(-38)),
                    _mm512_mullo_epi32(g, _mm512_set1_epi32(-74)),
                ),
                _mm512_mullo_epi32(b, _mm512_set1_epi32(112)),
            ),
            _mm512_set1_epi32(128),
        )),
        _mm512_set1_epi32(128),
    );
    _mm512_cvtusepi32_epi8(value)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn avx512_limited_v(
    r: std::arch::x86_64::__m512i,
    g: std::arch::x86_64::__m512i,
    b: std::arch::x86_64::__m512i,
) -> std::arch::x86_64::__m128i {
    use std::arch::x86_64::*;
    let value = _mm512_add_epi32(
        _mm512_srai_epi32::<8>(_mm512_add_epi32(
            _mm512_add_epi32(
                _mm512_add_epi32(
                    _mm512_mullo_epi32(r, _mm512_set1_epi32(112)),
                    _mm512_mullo_epi32(g, _mm512_set1_epi32(-94)),
                ),
                _mm512_mullo_epi32(b, _mm512_set1_epi32(-18)),
            ),
            _mm512_set1_epi32(128),
        )),
        _mm512_set1_epi32(128),
    );
    _mm512_cvtusepi32_epi8(value)
}

fn gif_quantizer_speed(quality: u8) -> i32 {
    match quality {
        80..=u8::MAX => 6,
        65..=79 => 10,
        _ => 15,
    }
}

fn build_gif_palette(samples: &mut [u8], speed: i32) -> Vec<u8> {
    let mut exact = std::collections::BTreeSet::new();
    for pixel in samples.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            continue;
        }
        pixel[3] = 255;
        exact.insert([pixel[0], pixel[1], pixel[2]]);
        if exact.len() > 255 {
            break;
        }
    }
    if exact.len() <= 255 {
        return exact.into_iter().flatten().collect();
    }

    // GIF only supports one-bit transparency. Palette training therefore uses
    // opaque RGB samples and reserves the 256th table entry for transparent
    // delta pixels instead of spending color capacity on alpha shades.
    for pixel in samples.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    color_quant::NeuQuant::new(speed, 255, samples).color_map_rgb()
}

fn gif_palette_color_count(quality: u8) -> usize {
    let _ = quality;
    255
}

fn gif_delay_centiseconds(frame_index: u32, fps: u32) -> u16 {
    let start = (frame_index as u64 * 100 + fps as u64 / 2) / fps as u64;
    let end = ((frame_index as u64 + 1) * 100 + fps as u64 / 2) / fps as u64;
    (end - start).max(1) as u16
}

fn gif_delta_indexed(
    previous: &[u8],
    current: &[u8],
    width: u32,
    height: u32,
    palette: &[u8],
    palette_lookup: &mut GifPaletteLookup,
) -> (u32, u32, u32, u32, Vec<u8>) {
    let Some((min_x, min_y, max_x, max_y)) = gif_changed_bounds(previous, current, width, height)
    else {
        return (0, 0, 1, 1, vec![255]);
    };
    gif_delta_indexed_for_bounds(
        previous,
        current,
        width,
        (min_x, min_y, max_x, max_y),
        palette,
        palette_lookup,
    )
}

fn gif_delta_indexed_for_bounds(
    previous: &[u8],
    current: &[u8],
    width: u32,
    (min_x, min_y, max_x, max_y): (u32, u32, u32, u32),
    palette: &[u8],
    palette_lookup: &mut GifPaletteLookup,
) -> (u32, u32, u32, u32, Vec<u8>) {
    let rect_width = max_x - min_x + 1;
    let rect_height = max_y - min_y + 1;
    let mut pixels = Vec::with_capacity(rect_width as usize * rect_height as usize);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let index = (y * width + x) as usize * 4;
            if previous[index..index + 4] == current[index..index + 4] {
                pixels.push(255);
            } else {
                pixels.push(nearest_gif_palette_index(
                    &current[index..index + 4],
                    palette,
                    palette_lookup,
                ));
            }
        }
    }
    (min_x, min_y, rect_width, rect_height, pixels)
}

fn gif_changed_bounds(
    previous: &[u8],
    current: &[u8],
    width: u32,
    height: u32,
) -> Option<(u32, u32, u32, u32)> {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx512f") {
        // SAFETY: runtime feature detection satisfies the target-feature
        // contract. Both frames have already passed checked_frame().
        return unsafe { gif_changed_bounds_avx512(previous, current, width, height) };
    }
    gif_changed_bounds_scalar(previous, current, width, height)
}

fn gif_changed_bounds_scalar(
    previous: &[u8],
    current: &[u8],
    width: u32,
    height: u32,
) -> Option<(u32, u32, u32, u32)> {
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0;
    let mut max_y = 0;
    for (index, (before, after)) in previous
        .chunks_exact(4)
        .zip(current.chunks_exact(4))
        .enumerate()
    {
        if before == after {
            continue;
        }
        let x = index as u32 % width;
        let y = index as u32 / width;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    if min_x == width || min_y == height {
        None
    } else {
        Some((min_x, min_y, max_x, max_y))
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
unsafe fn gif_changed_bounds_avx512(
    previous: &[u8],
    current: &[u8],
    width: u32,
    height: u32,
) -> Option<(u32, u32, u32, u32)> {
    use std::arch::x86_64::*;

    const PIXELS_PER_ZMM: u32 = 16;
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0u32;
    let mut max_y = 0u32;

    for y in 0..height {
        let row_offset = y as usize * width as usize * 4;
        let previous_row = previous.as_ptr().add(row_offset);
        let current_row = current.as_ptr().add(row_offset);
        let mut x = 0u32;
        while x + PIXELS_PER_ZMM <= width {
            let before = _mm512_loadu_si512(previous_row.add(x as usize * 4).cast());
            let after = _mm512_loadu_si512(current_row.add(x as usize * 4).cast());
            let equal = _mm512_cmpeq_epi32_mask(before, after);
            let changed = !equal;
            if changed != 0 {
                min_y = min_y.min(y);
                max_y = max_y.max(y);
                min_x = min_x.min(x + changed.trailing_zeros());
                max_x = max_x.max(x + (u16::BITS - 1 - changed.leading_zeros()));
            }
            x += PIXELS_PER_ZMM;
        }
        while x < width {
            let offset = x as usize * 4;
            let before = previous_row.add(offset).cast::<u32>().read_unaligned();
            let after = current_row.add(offset).cast::<u32>().read_unaligned();
            if before != after {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
            x += 1;
        }
    }

    if min_x == width || min_y == height {
        None
    } else {
        Some((min_x, min_y, max_x, max_y))
    }
}

fn rgb_key(color: &[u8]) -> u32 {
    ((color[0] as u32) << 16) | ((color[1] as u32) << 8) | color[2] as u32
}

fn index_gif_pixels(rgba: &[u8], palette: &[u8], palette_lookup: &mut GifPaletteLookup) -> Vec<u8> {
    debug_assert_eq!(rgba.len() % 4, 0);
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx512f") {
        // SAFETY: runtime feature detection satisfies the target-feature
        // contract and the kernel handles its non-ZMM tail separately.
        return unsafe { index_gif_pixels_avx512(rgba, palette, palette_lookup) };
    }
    rgba.chunks_exact(4)
        .map(|pixel| nearest_gif_palette_index(pixel, palette, palette_lookup))
        .collect()
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
unsafe fn index_gif_pixels_avx512(
    rgba: &[u8],
    palette: &[u8],
    palette_lookup: &mut GifPaletteLookup,
) -> Vec<u8> {
    use std::arch::x86_64::*;

    const LANES: usize = 16;
    let pixel_count = rgba.len() / 4;
    let mut indexed = vec![0u8; pixel_count];
    let channel_mask = _mm512_set1_epi32(0xff);
    let rgb_mask = _mm512_set1_epi32(0x00ff_ffff);
    let cache_slot_mask = _mm512_set1_epi32((GifPaletteLookup::DIRECT_SLOTS - 1) as i32);
    let cache_hash = _mm512_set1_epi32(0x9e37_79b1u32 as i32);
    let transparent_index = _mm512_set1_epi32(255);
    let cache_ptr = palette_lookup
        .direct
        .as_mut()
        .map(|table| table.as_mut_ptr());
    let mut offset = 0usize;

    while offset + LANES <= pixel_count {
        let packed = _mm512_loadu_si512(rgba.as_ptr().add(offset * 4).cast());
        let r = _mm512_and_si512(packed, channel_mask);
        let g = _mm512_and_si512(_mm512_srli_epi32(packed, 8), channel_mask);
        let b = _mm512_and_si512(_mm512_srli_epi32(packed, 16), channel_mask);
        let a = _mm512_srli_epi32(packed, 24);
        let keys = _mm512_or_si512(
            _mm512_or_si512(_mm512_slli_epi32(r, 16), _mm512_slli_epi32(g, 8)),
            b,
        );
        let transparent = _mm512_cmpeq_epi32_mask(a, _mm512_setzero_si512());
        let mut best_indices = _mm512_setzero_si512();
        let mut missing = !transparent;
        let mut cache_slots = _mm512_setzero_si512();

        if let Some(table) = cache_ptr {
            cache_slots = _mm512_and_si512(_mm512_mullo_epi32(keys, cache_hash), cache_slot_mask);
            let cached = _mm512_i32gather_epi32(cache_slots, table.cast::<i32>().cast_const(), 4);
            let key_matches = _mm512_cmpeq_epi32_mask(_mm512_and_si512(cached, rgb_mask), keys);
            let initialized = _mm512_cmpneq_epi32_mask(cached, _mm512_set1_epi32(-1));
            let hits = key_matches & initialized & !transparent;
            best_indices = _mm512_mask_mov_epi32(best_indices, hits, _mm512_srli_epi32(cached, 24));
            missing &= !hits;
        }

        if missing != 0 {
            let mut best_distances = _mm512_set1_epi32(i32::MAX);
            for (index, color) in palette
                .chunks_exact(3)
                .take(palette_lookup.active_colors)
                .enumerate()
            {
                let dr = _mm512_sub_epi32(r, _mm512_set1_epi32(color[0] as i32));
                let dg = _mm512_sub_epi32(g, _mm512_set1_epi32(color[1] as i32));
                let db = _mm512_sub_epi32(b, _mm512_set1_epi32(color[2] as i32));
                let distance = _mm512_add_epi32(
                    _mm512_add_epi32(_mm512_mullo_epi32(dr, dr), _mm512_mullo_epi32(dg, dg)),
                    _mm512_mullo_epi32(db, db),
                );
                let better = missing & _mm512_cmplt_epi32_mask(distance, best_distances);
                best_distances = _mm512_mask_mov_epi32(best_distances, better, distance);
                best_indices =
                    _mm512_mask_mov_epi32(best_indices, better, _mm512_set1_epi32(index as i32));
            }

            if let Some(table) = cache_ptr {
                let cache_values = _mm512_or_si512(keys, _mm512_slli_epi32(best_indices, 24));
                _mm512_mask_i32scatter_epi32(
                    table.cast::<i32>(),
                    missing,
                    cache_slots,
                    cache_values,
                    4,
                );
            }
        }

        best_indices = _mm512_mask_mov_epi32(best_indices, transparent, transparent_index);
        let packed_indices = _mm512_cvtepi32_epi8(best_indices);
        _mm_storeu_si128(indexed.as_mut_ptr().add(offset).cast(), packed_indices);
        offset += LANES;
    }

    for (output, pixel) in indexed[offset..]
        .iter_mut()
        .zip(rgba[offset * 4..].chunks_exact(4))
    {
        *output = nearest_gif_palette_index(pixel, palette, palette_lookup);
    }
    indexed
}

fn nearest_gif_palette_index(
    pixel: &[u8],
    palette: &[u8],
    palette_lookup: &mut GifPaletteLookup,
) -> u8 {
    if pixel[3] == 0 {
        return 255;
    }
    let key = rgb_key(pixel);
    if let Some(index) = palette_lookup.get(key) {
        return index;
    }

    #[cfg(target_arch = "x86_64")]
    let best_index = if std::arch::is_x86_feature_detected!("avx512f") {
        // SAFETY: the runtime feature gate satisfies the target-feature
        // contract; the palette channel arrays are padded to 256 lanes.
        unsafe { nearest_gif_palette_index_avx512(pixel, palette_lookup) }
    } else {
        nearest_gif_palette_index_scalar(pixel, palette, palette_lookup.active_colors)
    };
    #[cfg(not(target_arch = "x86_64"))]
    let best_index = nearest_gif_palette_index_scalar(pixel, palette, palette_lookup.active_colors);

    palette_lookup.insert(key, best_index);
    best_index
}

fn nearest_gif_palette_index_scalar(pixel: &[u8], palette: &[u8], active_colors: usize) -> u8 {
    let mut best_index = 0u8;
    let mut best_distance = u32::MAX;
    for (index, color) in palette.chunks_exact(3).take(active_colors).enumerate() {
        let dr = pixel[0] as i32 - color[0] as i32;
        let dg = pixel[1] as i32 - color[1] as i32;
        let db = pixel[2] as i32 - color[2] as i32;
        let distance = (dr * dr + dg * dg + db * db) as u32;
        if distance < best_distance {
            best_distance = distance;
            best_index = index as u8;
            if distance == 0 {
                break;
            }
        }
    }
    best_index
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
unsafe fn nearest_gif_palette_index_avx512(pixel: &[u8], palette: &GifPaletteLookup) -> u8 {
    use std::arch::x86_64::*;

    let pr = _mm512_set1_epi32(pixel[0] as i32);
    let pg = _mm512_set1_epi32(pixel[1] as i32);
    let pb = _mm512_set1_epi32(pixel[2] as i32);
    let mut best_index = 0u8;
    let mut best_distance = u32::MAX;
    let mut distances = [0u32; 16];
    let rounded_colors = palette.active_colors.div_ceil(16) * 16;

    for base in (0..rounded_colors).step_by(16) {
        let r = _mm512_loadu_si512(palette.palette_r.as_ptr().add(base).cast());
        let g = _mm512_loadu_si512(palette.palette_g.as_ptr().add(base).cast());
        let b = _mm512_loadu_si512(palette.palette_b.as_ptr().add(base).cast());
        let dr = _mm512_sub_epi32(pr, r);
        let dg = _mm512_sub_epi32(pg, g);
        let db = _mm512_sub_epi32(pb, b);
        let distance = _mm512_add_epi32(
            _mm512_add_epi32(_mm512_mullo_epi32(dr, dr), _mm512_mullo_epi32(dg, dg)),
            _mm512_mullo_epi32(db, db),
        );
        _mm512_storeu_si512(distances.as_mut_ptr().cast(), distance);
        let valid = (palette.active_colors - base).min(16);
        for (lane, &value) in distances[..valid].iter().enumerate() {
            if value < best_distance {
                best_distance = value;
                best_index = (base + lane) as u8;
            }
        }
        if best_distance == 0 {
            break;
        }
    }
    best_index
}

fn encode_webp<F>(
    spec: &AnimationEncodeSpec,
    expected: usize,
    frame_at: &mut F,
) -> Result<Vec<u8>, String>
where
    F: FnMut(u32) -> Result<Vec<u8>, String>,
{
    use webp_animation::prelude::*;
    let mut encoder = Encoder::new_with_options(
        (spec.width, spec.height),
        EncoderOptions {
            anim_params: webp_animation::AnimParams {
                loop_count: if spec.looped { 0 } else { 1 },
            },
            ..Default::default()
        },
    )
    .map_err(|error| format!("WebP encoder init: {error}"))?;
    for index in 0..spec.frame_count {
        let rgba = checked_frame(frame_at, index, expected)?;
        let timestamp = index as i32 * 1000 / spec.fps as i32;
        encoder
            .add_frame(&rgba, timestamp)
            .map_err(|error| format!("WebP frame {index}: {error}"))?;
    }
    let final_timestamp = spec.frame_count as i32 * 1000 / spec.fps as i32;
    let data = encoder
        .finalize(final_timestamp)
        .map_err(|error| format!("WebP finalize: {error}"))?;
    Ok(data.to_vec())
}

fn encode_apng<F>(
    spec: &AnimationEncodeSpec,
    expected: usize,
    frame_at: &mut F,
) -> Result<Vec<u8>, String>
where
    F: FnMut(u32) -> Result<Vec<u8>, String>,
{
    use flate2::{write::ZlibEncoder, Compression};
    use std::io::Write;

    let mut output = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&spec.width.to_be_bytes());
    ihdr.extend_from_slice(&spec.height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // RGBA8, deflate, no interlace.
    write_png_chunk(&mut output, b"IHDR", &ihdr);
    let mut actl = Vec::with_capacity(8);
    actl.extend_from_slice(&spec.frame_count.to_be_bytes());
    actl.extend_from_slice(&(if spec.looped { 0u32 } else { 1u32 }).to_be_bytes());
    write_png_chunk(&mut output, b"acTL", &actl);

    let mut sequence = 0u32;
    let mut previous: Option<Vec<u8>> = None;
    for index in 0..spec.frame_count {
        let rgba = checked_frame(frame_at, index, expected)?;
        let (offset_x, offset_y, frame_width, frame_height, pixels) =
            if let Some(before) = &previous {
                rgba_delta_rect(before, &rgba, spec.width, spec.height)
            } else {
                (0, 0, spec.width, spec.height, rgba.clone())
            };
        let mut fctl = Vec::with_capacity(26);
        fctl.extend_from_slice(&sequence.to_be_bytes());
        sequence += 1;
        fctl.extend_from_slice(&frame_width.to_be_bytes());
        fctl.extend_from_slice(&frame_height.to_be_bytes());
        fctl.extend_from_slice(&offset_x.to_be_bytes());
        fctl.extend_from_slice(&offset_y.to_be_bytes());
        fctl.extend_from_slice(&1u16.to_be_bytes());
        fctl.extend_from_slice(&(spec.fps as u16).to_be_bytes());
        fctl.extend_from_slice(&[0, 0]); // dispose NONE, blend SOURCE.
        write_png_chunk(&mut output, b"fcTL", &fctl);

        let mut scanlines = Vec::with_capacity(pixels.len() + frame_height as usize);
        let row_bytes = frame_width as usize * 4;
        for row in pixels.chunks_exact(row_bytes) {
            scanlines.push(0); // PNG filter NONE.
            scanlines.extend_from_slice(row);
        }
        let mut zlib = ZlibEncoder::new(Vec::new(), Compression::best());
        zlib.write_all(&scanlines)
            .map_err(|error| format!("APNG frame {index} compress: {error}"))?;
        let compressed = zlib
            .finish()
            .map_err(|error| format!("APNG frame {index} finish: {error}"))?;
        if index == 0 {
            write_png_chunk(&mut output, b"IDAT", &compressed);
        } else {
            let mut fdat = Vec::with_capacity(compressed.len() + 4);
            fdat.extend_from_slice(&sequence.to_be_bytes());
            sequence += 1;
            fdat.extend_from_slice(&compressed);
            write_png_chunk(&mut output, b"fdAT", &fdat);
        }
        previous = Some(rgba);
    }
    write_png_chunk(&mut output, b"IEND", &[]);
    Ok(output)
}

fn write_png_chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    output.extend_from_slice(&crc.finalize().to_be_bytes());
}

fn rgba_delta_rect(
    previous: &[u8],
    current: &[u8],
    width: u32,
    height: u32,
) -> (u32, u32, u32, u32, Vec<u8>) {
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut changed = false;
    for (index, (before, after)) in previous
        .chunks_exact(4)
        .zip(current.chunks_exact(4))
        .enumerate()
    {
        if before == after {
            continue;
        }
        let x = index as u32 % width;
        let y = index as u32 / width;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
        changed = true;
    }
    if !changed {
        return (0, 0, 1, 1, current[..4].to_vec());
    }
    let rect_width = max_x - min_x + 1;
    let rect_height = max_y - min_y + 1;
    let mut pixels = Vec::with_capacity(rect_width as usize * rect_height as usize * 4);
    for y in min_y..=max_y {
        let start = ((y * width + min_x) * 4) as usize;
        let end = start + rect_width as usize * 4;
        pixels.extend_from_slice(&current[start..end]);
    }
    (min_x, min_y, rect_width, rect_height, pixels)
}

pub fn validate_magic(format: AnimationFormat, data: &[u8]) -> Result<(), String> {
    let valid = match format {
        AnimationFormat::Gif => validate_gif_container(data),
        AnimationFormat::Webp => validate_webp_container(data),
        AnimationFormat::Apng => validate_apng_container(data),
        AnimationFormat::Mp4 => validate_mp4_container(data),
    };
    valid.map_err(|reason| format!("ANIMATION_ARTIFACT_INVALID: {format:?} {reason}"))
}

fn validate_gif_container(data: &[u8]) -> Result<(), &'static str> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::Indexed);
    let mut decoder = options.read_info(data).map_err(|_| "header")?;
    let mut frames = 0u32;
    while decoder
        .read_next_frame()
        .map_err(|_| "frame stream")?
        .is_some()
    {
        frames += 1;
    }
    if frames == 0 || data.last() != Some(&0x3b) {
        return Err("missing frame or trailer");
    }
    Ok(())
}

fn validate_webp_container(data: &[u8]) -> Result<(), &'static str> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return Err("header");
    }
    let declared = u32::from_le_bytes(data[4..8].try_into().map_err(|_| "size")?) as usize + 8;
    if declared != data.len() {
        return Err("RIFF size");
    }
    let decoder = webp_animation::Decoder::new(data).map_err(|_| "decode")?;
    if decoder.into_iter().next().is_none() {
        return Err("missing frame");
    }
    Ok(())
}

fn validate_apng_container(data: &[u8]) -> Result<(), &'static str> {
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("header");
    }
    let mut cursor = 8usize;
    let mut has_animation_control = false;
    let mut has_end = false;
    while cursor < data.len() {
        let header_end = cursor.checked_add(8).ok_or("chunk overflow")?;
        if header_end > data.len() {
            return Err("truncated chunk header");
        }
        let length = u32::from_be_bytes(
            data[cursor..cursor + 4]
                .try_into()
                .map_err(|_| "chunk size")?,
        ) as usize;
        let chunk_type = &data[cursor + 4..cursor + 8];
        let payload_end = header_end.checked_add(length).ok_or("chunk overflow")?;
        let chunk_end = payload_end.checked_add(4).ok_or("chunk overflow")?;
        if chunk_end > data.len() {
            return Err("truncated chunk");
        }
        let mut crc = crc32fast::Hasher::new();
        crc.update(chunk_type);
        crc.update(&data[header_end..payload_end]);
        let stored_crc = u32::from_be_bytes(
            data[payload_end..chunk_end]
                .try_into()
                .map_err(|_| "chunk crc")?,
        );
        if crc.finalize() != stored_crc {
            return Err("chunk crc mismatch");
        }
        has_animation_control |= chunk_type == b"acTL";
        if chunk_type == b"IEND" {
            has_end = true;
            cursor = chunk_end;
            break;
        }
        cursor = chunk_end;
    }
    if !has_animation_control || !has_end || cursor != data.len() {
        return Err("missing acTL/IEND or trailing bytes");
    }
    Ok(())
}

fn validate_mp4_container(data: &[u8]) -> Result<(), &'static str> {
    if data.len() < 16 || &data[4..8] != b"ftyp" {
        return Err("ftyp header");
    }
    if !data.windows(4).any(|value| value == b"moov")
        || !data.windows(4).any(|value| value == b"mdat")
    {
        return Err("missing moov/mdat");
    }
    Ok(())
}

pub fn validate_encoded_artifact(
    format: AnimationFormat,
    path: &str,
    content_type: &str,
    expected_bytes: usize,
    actual_bytes: usize,
    prefix: &[u8],
) -> Result<(), String> {
    if !path.ends_with(&format!(".{}", format.extension()))
        || content_type != format.content_type()
        || expected_bytes != actual_bytes
    {
        return Err("ANIMATION_ARTIFACT_INVALID: path/mime/size mismatch".into());
    }
    validate_magic(format, prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deterministic_rgba(width: usize, height: usize) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&[
                    ((x * 17 + y * 29 + 3) & 255) as u8,
                    ((x * 71 + y * 11 + 19) & 255) as u8,
                    ((x * 7 + y * 97 + 113) & 255) as u8,
                    ((x * 43 + y * 31 + 251) & 255) as u8,
                ]);
            }
        }
        rgba
    }

    #[cfg(target_arch = "x86_64")]
    fn assert_avx512_yuv420_matches_scalar(width: usize, height: usize) {
        if !std::arch::is_x86_feature_detected!("avx512f")
            || !std::arch::is_x86_feature_detected!("avx512bw")
        {
            return;
        }
        let rgba = deterministic_rgba(width, height);
        let mut scalar = vec![0u8; width * height * 3 / 2];
        let mut simd = vec![0u8; scalar.len()];
        rgba_to_yuv420p_scalar(&rgba, width, height, &mut scalar);
        // SAFETY: feature detection above satisfies the target-feature contract.
        unsafe { rgba_to_yuv420p_avx512(&rgba, width, height, &mut simd) };
        assert_eq!(simd, scalar);
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn avx512_yuv420_matches_scalar_across_a_non_zmm_tail() {
        assert_avx512_yuv420_matches_scalar(34, 6);
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn avx512_yuv420_matches_scalar_at_profile_export_resolution() {
        assert_avx512_yuv420_matches_scalar(1832, 816);
    }

    #[test]
    fn yuv420_scratch_reuse_overwrites_dirty_storage() {
        let width = 4;
        let height = 2;
        let mut config = H264EncoderConfig::default();
        config.yuv_converter = H264YuvConverter::Scalar;
        let first = [17, 91, 203, 255].repeat((width * height) as usize);
        let second = [241, 37, 11, 255].repeat((width * height) as usize);
        let expected_first = rgba_to_yuv420p(&first, width, height, &config).unwrap();
        let expected_second = rgba_to_yuv420p(&second, width, height, &config).unwrap();
        let mut scratch = vec![0xa5; expected_first.len()];

        rgba_to_yuv420p_into(&first, width, height, &config, &mut scratch).unwrap();
        assert_eq!(scratch, expected_first);
        scratch.fill(0x5a);
        rgba_to_yuv420p_into(&second, width, height, &config, &mut scratch).unwrap();
        assert_eq!(scratch, expected_second);
    }

    #[test]
    fn exact_dirty_scan_marks_only_rgba_changed_macroblocks() {
        let width = 34u32;
        let height = 18u32;
        let mut before = vec![17u8; width as usize * height as usize * 4];
        let mut after = before.clone();
        let pixel = |x: usize, y: usize| (y * width as usize + x) * 4;
        after[pixel(17, 0)] = 99;
        after[pixel(33, 17) + 2] = 201;
        let mut flags = vec![0u8; width.div_ceil(16) as usize * height.div_ceil(16) as usize];
        mark_changed_macroblocks_for_region(
            &before,
            &after,
            width,
            PixelRect {
                left: 0,
                top: 0,
                right: width as i32,
                bottom: height as i32,
            },
            &mut flags,
        )
        .unwrap();
        assert_eq!(flags, [0, 1, 0, 0, 0, 1]);

        before.copy_from_slice(&after);
        flags.fill(0);
        mark_changed_macroblocks_for_region(
            &before,
            &after,
            width,
            PixelRect {
                left: 0,
                top: 0,
                right: width as i32,
                bottom: height as i32,
            },
            &mut flags,
        )
        .unwrap();
        assert!(flags.iter().all(|&value| value == 0));
    }

    #[test]
    fn macroblock_yuv_updates_match_full_frame_conversion() {
        let width = 34u32;
        let height = 18u32;
        let config = H264EncoderConfig::default();
        let first = deterministic_rgba(width as usize, height as usize);
        let mut second = first.clone();
        for (x, y, color) in [
            (17usize, 0usize, [9, 81, 173, 255]),
            (33, 17, [233, 7, 61, 255]),
        ] {
            let offset = (y * width as usize + x) * 4;
            second[offset..offset + 4].copy_from_slice(&color);
        }
        let mut partial = rgba_to_yuv420p(&first, width, height, &config).unwrap();
        let expected = rgba_to_yuv420p(&second, width, height, &config).unwrap();
        rgba_to_yuv420p_macroblocks_into(&second, width, height, &[1, 5], &config, &mut partial)
            .unwrap();
        assert_eq!(partial, expected);
    }

    #[test]
    fn h264_cache_identity_covers_every_output_affecting_setting() {
        let base = H264EncoderConfig::default();
        let base_identity = base.cache_identity().unwrap();
        let mut changed = base.clone();
        changed.qp = 19;
        assert_ne!(base_identity, changed.cache_identity().unwrap());
        changed = base.clone();
        changed.yuv_converter = H264YuvConverter::Avx512;
        assert_ne!(base_identity, changed.cache_identity().unwrap());
        changed = base;
        changed.encoder_revision = "cache-identity-test-revision".into();
        assert_ne!(base_identity, changed.cache_identity().unwrap());
    }

    #[test]
    fn x264_mb_info_marks_only_macroblocks_outside_dirty_regions_constant() {
        let mut mb_info = Vec::new();
        assert_eq!(
            fill_x264_mb_info(&FrameCompositeUpdate::Full, 50, 33, &mut mb_info).unwrap(),
            (0, 12)
        );
        assert!(mb_info.iter().all(|&flags| flags == 0));

        assert_eq!(
            fill_x264_mb_info(&FrameCompositeUpdate::Reused, 50, 33, &mut mb_info).unwrap(),
            (12, 12)
        );
        assert!(mb_info.iter().all(|&flags| flags == 1));

        assert_eq!(
            fill_x264_mb_info(
                &FrameCompositeUpdate::Dirty(vec![1, 5]),
                50,
                33,
                &mut mb_info,
            )
            .unwrap(),
            (10, 12)
        );
        assert_eq!(&mb_info[..4], &[1, 0, 1, 1]);
        assert_eq!(&mb_info[4..8], &[1, 0, 1, 1]);
        assert_eq!(&mb_info[8..], &[1, 1, 1, 1]);
    }

    #[test]
    fn non_mp4_encoder_suffix_preserves_existing_animation_cache_contract() {
        let gif = resolve_preset("qq", Some(AnimationFormat::Gif)).unwrap();
        assert_eq!(animation_encoder_cache_suffix(&gif).unwrap(), "");
        let mp4 = resolve_preset("qq", Some(AnimationFormat::Mp4)).unwrap();
        assert!(animation_encoder_cache_suffix(&mp4)
            .unwrap()
            .starts_with("-h264-v2-"));
    }

    #[test]
    fn animation_memory_gate_preserves_retained_buffer_contract() {
        assert_eq!(animation_peak_export_bytes(100, 300), 700);
        assert_eq!(animation_peak_export_bytes(usize::MAX, 1), usize::MAX);

        let gif = resolve_preset("qq", Some(AnimationFormat::Gif)).unwrap();
        assert_eq!(gif.export_memory_budget_bytes, 64 * 1024 * 1024);
        let mp4 = resolve_preset("qq", Some(AnimationFormat::Mp4)).unwrap();
        assert_eq!(mp4.export_memory_budget_bytes, usize::MAX);
        assert!(animation_peak_export_bytes(1_115_785_640, 0) <= mp4.export_memory_budget_bytes);
    }

    #[test]
    fn dynamic_animation_groups_keep_motion_bounds_during_rasterization() {
        assert!(!animation_group_uses_dynamic_bounds(None));
        assert!(animation_group_uses_dynamic_bounds(Some(
            allium_renderer_core::StableId(1)
        )));
    }

    #[test]
    fn profile_animation_frame_starts_with_opaque_white_canvas() {
        let scene = allium_renderer_core::Scene::new(allium_renderer_core::SceneSource {
            scene_id: allium_renderer_core::StableId(1),
            region: "cn".into(),
            font_engine_fingerprint: "test".into(),
            raster_contract: "test".into(),
            layers: Vec::new(),
            glyphs: Vec::new(),
            semantic_commands: Vec::new(),
            interaction_regions: Vec::new(),
            component_controls: Vec::new(),
        })
        .unwrap();

        let rgba = composite_frame(&scene, &[], 2, 2, 1.0).unwrap();
        assert_eq!(rgba, [255, 255, 255, 255].repeat(4));
    }

    #[test]
    fn compositor_into_matches_owned_frame_output() {
        let scene = allium_renderer_core::Scene::new(allium_renderer_core::SceneSource {
            scene_id: allium_renderer_core::StableId(1),
            region: "cn".into(),
            font_engine_fingerprint: "test".into(),
            raster_contract: "test".into(),
            layers: Vec::new(),
            glyphs: Vec::new(),
            semantic_commands: Vec::new(),
            interaction_regions: Vec::new(),
            component_controls: Vec::new(),
        })
        .unwrap();
        let mut owned = AnimationFrameCompositor::new(2, 2, 1.0).unwrap();
        let expected = owned.composite(0, &scene, &[]).unwrap();
        let mut direct = AnimationFrameCompositor::new(2, 2, 1.0).unwrap();
        let mut actual = Vec::new();
        direct.composite_into(0, &scene, &[], &mut actual).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn dirty_compositor_matches_full_redraw_with_interleaved_static_layer() {
        fn solid_layer(
            color: skia_safe::Color,
            x: f32,
            y: f32,
            width: u32,
            height: u32,
            dynamic_layer_id: Option<allium_renderer_core::LayerId>,
        ) -> RasterLayer {
            let mut surface =
                skia_safe::surfaces::raster_n32_premul((width as i32, height as i32)).unwrap();
            surface.canvas().clear(color);
            RasterLayer {
                dynamic_layer_id,
                image: surface.image_snapshot(),
                x,
                y,
                width,
                height,
            }
        }

        fn state(x: f32, y: f32, width: f32, height: f32) -> LayerFrameState {
            let destination = skia_safe::Rect::from_xywh(x, y, width, height);
            LayerFrameState {
                visible: true,
                destination,
                pixel_bounds: pixel_bounds_for_destination(destination, 8, 4),
            }
        }

        let layers = vec![
            solid_layer(skia_safe::Color::BLUE, 0.0, 0.0, 8, 4, None),
            solid_layer(
                skia_safe::Color::RED,
                1.0,
                1.0,
                2,
                2,
                Some(allium_renderer_core::StableId(1)),
            ),
            solid_layer(skia_safe::Color::GREEN, 3.0, 0.0, 2, 4, None),
        ];
        let before = vec![
            state(0.0, 0.0, 8.0, 4.0),
            state(1.0, 1.0, 2.0, 2.0),
            state(3.0, 0.0, 2.0, 4.0),
        ];
        let after = vec![
            state(0.0, 0.0, 8.0, 4.0),
            state(5.0, 1.0, 2.0, 2.0),
            state(3.0, 0.0, 2.0, 4.0),
        ];
        let dirty = before[1].pixel_bounds.union(after[1].pixel_bounds);

        let mut incremental = skia_safe::surfaces::raster_n32_premul((8, 4)).unwrap();
        redraw_full_frame(&mut incremental, &layers, &before);
        redraw_dirty_frame(&mut incremental, &layers, &after, dirty);
        let incremental = read_animation_surface_rgba(&mut incremental, 8, 4).unwrap();

        let mut oracle = skia_safe::surfaces::raster_n32_premul((8, 4)).unwrap();
        redraw_full_frame(&mut oracle, &layers, &after);
        let oracle = read_animation_surface_rgba(&mut oracle, 8, 4).unwrap();

        assert_eq!(incremental, oracle);
    }

    #[test]
    fn all_animation_encoders_stream_valid_two_frame_artifacts() {
        let spec = AnimationEncodeSpec {
            width: 2,
            height: 2,
            fps: 20,
            frame_count: 2,
            looped: true,
            output_budget_bytes: 1_000_000,
            gif_quality: 80,
        };
        for format in [
            AnimationFormat::Gif,
            AnimationFormat::Webp,
            AnimationFormat::Apng,
        ] {
            let encoded = encode_rgba_frames(format, &spec, |index| {
                Ok(if index == 0 {
                    [255, 0, 0, 255].repeat(4)
                } else {
                    [0, 0, 255, 255].repeat(4)
                })
            })
            .unwrap();
            validate_magic(format, &encoded.data).unwrap();
            assert_eq!(encoded.frame_count, 2);
            assert_eq!(encoded.duration_ms, 100);
            assert!(encoded.data.len() < spec.output_budget_bytes);
            match format {
                AnimationFormat::Gif => {
                    let mut options = gif::DecodeOptions::new();
                    options.set_color_output(gif::ColorOutput::RGBA);
                    let mut decoder = options.read_info(Cursor::new(&encoded.data)).unwrap();
                    let mut frames = 0;
                    while decoder.read_next_frame().unwrap().is_some() {
                        frames += 1;
                    }
                    assert_eq!(frames, 2);
                }
                AnimationFormat::Webp => {
                    let decoder = webp_animation::Decoder::new(&encoded.data).unwrap();
                    assert_eq!(decoder.into_iter().count(), 2);
                }
                AnimationFormat::Apng => {
                    let marker = encoded
                        .data
                        .windows(4)
                        .position(|chunk| chunk == b"acTL")
                        .unwrap();
                    assert_eq!(
                        u32::from_be_bytes(
                            encoded.data[marker + 4..marker + 8].try_into().unwrap()
                        ),
                        2
                    );
                }
                AnimationFormat::Mp4 => unreachable!("MP4 is tested with the external encoder"),
            }
        }
    }

    #[test]
    fn direct_x264_streams_a_valid_mp4() {
        let spec = AnimationEncodeSpec {
            width: 16,
            height: 8,
            fps: 12,
            frame_count: 2,
            looped: false,
            output_budget_bytes: 1_000_000,
            gif_quality: 80,
        };
        let frames = [
            [255, 0, 0, 255].repeat(16 * 8),
            [0, 255, 0, 255].repeat(16 * 8),
        ];
        let encoded = encode_rgba_frames(AnimationFormat::Mp4, &spec, |index| {
            Ok(frames[index as usize].clone())
        })
        .unwrap();
        validate_mp4_container(&encoded.data).unwrap();
        assert_eq!((encoded.width, encoded.height, encoded.fps), (16, 8, 12));
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
        {
            use std::io::Write;
            let mut decoder = std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-i", "pipe:0", "-f", "null", "-"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            decoder
                .stdin
                .take()
                .unwrap()
                .write_all(&encoded.data)
                .unwrap();
            let output = decoder.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn direct_x264_accepts_constant_macroblock_hints() {
        let spec = AnimationEncodeSpec {
            width: 16,
            height: 16,
            fps: 12,
            frame_count: 2,
            looped: false,
            output_budget_bytes: 1_000_000,
            gif_quality: 80,
        };
        let frame = [17, 91, 203, 255].repeat(16 * 16);
        let mut telemetry = AnimationEncodeTelemetry::default();
        let data = encode_mp4_into(
            &spec,
            frame.len(),
            &H264EncoderConfig::default(),
            &mut telemetry,
            |index, rgba| {
                *rgba = frame.clone();
                Ok(if index == 0 {
                    FrameCompositeUpdate::Full
                } else {
                    FrameCompositeUpdate::Reused
                })
            },
        )
        .unwrap();
        validate_mp4_container(&data).unwrap();
        assert_eq!(telemetry.h264_mbinfo_total_blocks, 2);
        assert_eq!(telemetry.h264_mbinfo_constant_blocks, 1);
    }

    #[test]
    fn gif_retained_deltas_remove_the_second_frame_pass_without_changing_bytes() {
        let spec = AnimationEncodeSpec {
            width: 4,
            height: 2,
            fps: 20,
            frame_count: 4,
            looped: true,
            output_budget_bytes: 1_000_000,
            gif_quality: 80,
        };
        let mut frames = vec![[255, 0, 0, 255].repeat(8)];
        let mut second = frames[0].clone();
        second[4..8].copy_from_slice(&[0, 255, 0, 255]);
        frames.push(second.clone());
        frames.push(second.clone());
        let mut fourth = second;
        fourth[24..28].copy_from_slice(&[0, 0, 255, 255]);
        frames.push(fourth);

        let (two_pass, two_pass_telemetry) = encode_rgba_frames_with_h264_and_telemetry(
            AnimationFormat::Gif,
            &spec,
            &H264EncoderConfig::default(),
            0,
            |index| Ok(frames[index as usize].clone()),
        )
        .unwrap();
        let (retained, retained_telemetry) = encode_rgba_frames_with_h264_and_telemetry(
            AnimationFormat::Gif,
            &spec,
            &H264EncoderConfig::default(),
            1_000_000,
            |index| Ok(frames[index as usize].clone()),
        )
        .unwrap();

        assert_eq!(retained.data, two_pass.data);
        assert!(two_pass_telemetry.gif_second_frame_pass);
        assert_eq!(two_pass_telemetry.frame_callback_calls, 8);
        assert!(!retained_telemetry.gif_second_frame_pass);
        assert_eq!(retained_telemetry.frame_callback_calls, 4);
        assert!(retained_telemetry.gif_retained_frame_bytes > 0);
    }

    #[test]
    fn gif_preserves_source_palette_without_outline_color_bleed() {
        let source = vec![
            239, 207, 211, 255, 255, 238, 240, 255, 202, 105, 145, 255, 255, 95, 145, 255, 207,
            198, 209, 255, 67, 87, 190, 255,
        ];
        let spec = AnimationEncodeSpec {
            width: 6,
            height: 1,
            fps: 20,
            frame_count: 1,
            looped: false,
            output_budget_bytes: 1_000_000,
            gif_quality: 80,
        };
        let encoded =
            encode_rgba_frames(AnimationFormat::Gif, &spec, |_| Ok(source.clone())).unwrap();
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(Cursor::new(&encoded.data)).unwrap();
        let decoded = decoder.read_next_frame().unwrap().unwrap().buffer.to_vec();
        let max_channel_error = source
            .iter()
            .zip(decoded.iter())
            .enumerate()
            .filter(|(index, _)| index % 4 != 3)
            .map(|(_, (&expected, &actual))| expected.abs_diff(actual))
            .max()
            .unwrap();
        assert!(
            max_channel_error <= 2,
            "GIF color error was {max_channel_error}"
        );
    }

    #[test]
    fn gif_delta_writes_opaque_white_over_a_moving_layers_old_position() {
        let white = [255, 255, 255, 255];
        let red = [255, 0, 0, 255];
        let frames = [[red, white, white].concat(), [white, white, red].concat()];
        let spec = AnimationEncodeSpec {
            width: 3,
            height: 1,
            fps: 20,
            frame_count: 2,
            looped: true,
            output_budget_bytes: 1_000_000,
            gif_quality: 80,
        };
        let encoded = encode_rgba_frames(AnimationFormat::Gif, &spec, |index| {
            Ok(frames[index as usize].clone())
        })
        .unwrap();

        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(Cursor::new(&encoded.data)).unwrap();
        decoder.read_next_frame().unwrap().unwrap();
        let delta = decoder.read_next_frame().unwrap().unwrap();

        assert_eq!((delta.left, delta.width), (0, 3));
        assert_eq!(&delta.buffer[0..4], &white);
        assert_eq!(
            delta.buffer[7], 0,
            "unchanged middle pixel stays transparent"
        );
        assert_eq!(&delta.buffer[8..12], &red);
    }

    #[test]
    fn gif_palette_covers_colors_introduced_after_the_first_frame() {
        let frames = [
            [255, 0, 0, 255].repeat(4),
            [0, 255, 0, 255].repeat(4),
            [0, 0, 255, 255].repeat(4),
        ];
        let spec = AnimationEncodeSpec {
            width: 2,
            height: 2,
            fps: 20,
            frame_count: frames.len() as u32,
            looped: false,
            output_budget_bytes: 1_000_000,
            gif_quality: 80,
        };
        let encoded = encode_rgba_frames(AnimationFormat::Gif, &spec, |index| {
            Ok(frames[index as usize].clone())
        })
        .unwrap();
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(Cursor::new(&encoded.data)).unwrap();
        for expected in [[255, 0, 0], [0, 255, 0], [0, 0, 255]] {
            let decoded = decoder.read_next_frame().unwrap().unwrap();
            assert!(decoded.buffer.chunks_exact(4).all(|pixel| {
                pixel[..3]
                    .iter()
                    .zip(expected)
                    .all(|(&actual, expected)| actual.abs_diff(expected) <= 2)
            }));
        }
    }

    #[test]
    fn gif_delay_uses_rational_centisecond_boundaries() {
        assert_eq!(
            (0..8)
                .map(|index| gif_delay_centiseconds(index, 15))
                .collect::<Vec<_>>(),
            vec![7, 6, 7, 7, 6, 7, 7, 6]
        );
        assert_eq!(
            (0..6)
                .map(|index| gif_delay_centiseconds(index, 60))
                .collect::<Vec<_>>(),
            vec![2, 1, 2, 2, 1, 2]
        );
    }

    #[test]
    fn gif_delta_encodes_an_unchanged_frame_as_one_transparent_index() {
        let rgba = vec![12, 34, 56, 255];
        let palette = vec![0; 256 * 3];
        let mut lookup = GifPaletteLookup::new(0, 255);
        assert_eq!(
            gif_delta_indexed(&rgba, &rgba, 1, 1, &palette, &mut lookup),
            (0, 0, 1, 1, vec![255])
        );
    }

    #[test]
    fn gif_changed_bounds_matches_scalar_across_zmm_tail() {
        let width = 19u32;
        let height = 2u32;
        let previous = vec![255u8; width as usize * height as usize * 4];
        let mut current = previous.clone();
        for (x, y, rgba) in [
            (17u32, 0u32, [1u8, 2, 3, 255]),
            (2u32, 1u32, [9u8, 8, 7, 255]),
        ] {
            let offset = (y * width + x) as usize * 4;
            current[offset..offset + 4].copy_from_slice(&rgba);
        }
        let expected = Some((2, 0, 17, 1));
        assert_eq!(
            gif_changed_bounds_scalar(&previous, &current, width, height),
            expected
        );
        assert_eq!(
            gif_changed_bounds(&previous, &current, width, height),
            expected
        );
        assert_eq!(
            gif_changed_bounds_scalar(&previous, &previous, width, height),
            None
        );

        #[cfg(target_arch = "x86_64")]
        if std::arch::is_x86_feature_detected!("avx512f") {
            assert_eq!(
                unsafe { gif_changed_bounds_avx512(&previous, &current, width, height) },
                expected
            );
        }
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn avx512_gif_palette_distance_matches_scalar() {
        if !std::arch::is_x86_feature_detected!("avx512f") {
            return;
        }
        let mut palette = Vec::with_capacity(255 * 3);
        for index in 0..255u32 {
            palette.extend_from_slice(&[
                index.wrapping_mul(73) as u8,
                index.wrapping_mul(151).wrapping_add(19) as u8,
                index.wrapping_mul(211).wrapping_add(37) as u8,
            ]);
        }
        let mut lookup = GifPaletteLookup::new(0, 255);
        lookup.set_palette(&palette);
        for index in 0..4096u32 {
            let pixel = [
                index.wrapping_mul(17) as u8,
                index.wrapping_mul(43).wrapping_add(7) as u8,
                index.wrapping_mul(97).wrapping_add(31) as u8,
                255,
            ];
            let scalar = nearest_gif_palette_index_scalar(&pixel, &palette, 255);
            let avx512 = unsafe { nearest_gif_palette_index_avx512(&pixel, &lookup) };
            assert_eq!(avx512, scalar, "pixel={pixel:?}");
        }
    }

    #[test]
    fn fast_gif_lzw_roundtrips_dictionary_resets() {
        let width = 320u16;
        let height = 200u16;
        let pixel_count = width as usize * height as usize;
        let mut pixels = Vec::with_capacity(pixel_count);
        let mut value = 0x1234_5678u32;
        for _ in 0..pixel_count {
            value = value.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            pixels.push((value >> 24) as u8);
        }
        let mut palette = Vec::with_capacity(256 * 3);
        for index in 0..=255u8 {
            palette.extend_from_slice(&[index, index, index]);
        }
        let mut lzw = FastGifLzwEncoder::new();
        for _ in 0..2 {
            let encoded_lzw = lzw.encode(&pixels);
            let mut output = Vec::new();
            {
                let mut encoder = gif::Encoder::new(&mut output, width, height, &palette).unwrap();
                let mut frame =
                    gif::Frame::from_indexed_pixels(width, height, pixels.clone(), Some(255));
                frame.buffer = std::borrow::Cow::Owned(encoded_lzw);
                encoder.write_lzw_pre_encoded_frame(&frame).unwrap();
            }
            let mut options = gif::DecodeOptions::new();
            options.set_color_output(gif::ColorOutput::Indexed);
            let mut decoder = options.read_info(output.as_slice()).unwrap();
            let decoded = decoder.read_next_frame().unwrap().unwrap();
            assert_eq!(decoded.buffer.as_ref(), pixels.as_slice());
        }
    }

    #[test]
    fn apng_delta_frames_keep_the_previous_canvas() {
        let spec = AnimationEncodeSpec {
            width: 2,
            height: 1,
            fps: 20,
            frame_count: 2,
            looped: false,
            output_budget_bytes: 1_000_000,
            gif_quality: 80,
        };
        let frames = [
            vec![255, 0, 0, 255, 0, 0, 0, 255],
            vec![255, 0, 0, 255, 0, 255, 0, 255],
        ];
        let encoded = encode_rgba_frames(AnimationFormat::Apng, &spec, |index| {
            Ok(frames[index as usize].clone())
        })
        .unwrap();
        let mut controls = Vec::new();
        let mut offset = 8usize;
        while offset + 12 <= encoded.data.len() {
            let length =
                u32::from_be_bytes(encoded.data[offset..offset + 4].try_into().unwrap()) as usize;
            let kind = &encoded.data[offset + 4..offset + 8];
            let payload_start = offset + 8;
            let payload_end = payload_start + length;
            assert!(payload_end + 4 <= encoded.data.len());
            if kind == b"fcTL" {
                controls.push(&encoded.data[payload_start..payload_end]);
            }
            offset = payload_end + 4;
        }
        assert_eq!(controls.len(), 2);
        assert_eq!(
            controls[1][24], 0,
            "delta frame must use APNG_DISPOSE_OP_NONE"
        );
        assert_eq!(
            controls[1][25], 0,
            "delta frame must use APNG_BLEND_OP_SOURCE"
        );
    }

    #[test]
    fn frame_size_and_output_budget_fail_closed() {
        let spec = AnimationEncodeSpec {
            width: 2,
            height: 2,
            fps: 20,
            frame_count: 1,
            looped: false,
            output_budget_bytes: 1,
            gif_quality: 80,
        };
        assert!(
            encode_rgba_frames(AnimationFormat::Apng, &spec, |_| Ok(vec![0; 15]))
                .unwrap_err()
                .contains("expected 16")
        );
        assert!(
            encode_rgba_frames(AnimationFormat::Apng, &spec, |_| Ok(vec![0; 16]))
                .unwrap_err()
                .contains("OUTPUT_BUDGET_EXCEEDED")
        );
    }

    #[test]
    fn ready_artifact_validation_rejects_path_mime_size_and_magic_mismatches() {
        let spec = AnimationEncodeSpec {
            width: 1,
            height: 1,
            fps: 20,
            frame_count: 1,
            looped: false,
            output_budget_bytes: 4096,
            gif_quality: 80,
        };
        let gif = encode_rgba_frames(AnimationFormat::Gif, &spec, |_| Ok(vec![255, 0, 0, 255]))
            .unwrap()
            .data;
        validate_encoded_artifact(
            AnimationFormat::Gif,
            "page.gif",
            "image/gif",
            gif.len(),
            gif.len(),
            &gif,
        )
        .unwrap();
        let truncated = &gif[..gif.len() - 1];
        for result in [
            validate_encoded_artifact(
                AnimationFormat::Gif,
                "page.apng",
                "image/gif",
                gif.len(),
                gif.len(),
                &gif,
            ),
            validate_encoded_artifact(
                AnimationFormat::Gif,
                "page.gif",
                "image/png",
                gif.len(),
                gif.len(),
                &gif,
            ),
            validate_encoded_artifact(
                AnimationFormat::Gif,
                "page.gif",
                "image/gif",
                gif.len() - 1,
                gif.len(),
                &gif,
            ),
            validate_encoded_artifact(
                AnimationFormat::Gif,
                "page.gif",
                "image/gif",
                10,
                10,
                b"\x89PNG\r\n\x1a\n00",
            ),
            validate_encoded_artifact(
                AnimationFormat::Gif,
                "page.gif",
                "image/gif",
                truncated.len(),
                truncated.len(),
                truncated,
            ),
        ] {
            assert!(result.is_err());
        }
    }

    #[test]
    fn immutable_presets_resolve_auto_without_format_guessing() {
        let qq = resolve_preset("qq", None).unwrap();
        assert_eq!(qq.preset, AnimationPreset::QqV2);
        assert_eq!(qq.format, AnimationFormat::Gif);
        assert_eq!(qq.maximum_ticks, 480);
        assert_eq!(qq.maximum_long_edge, 1830);
        assert_eq!(qq.fps, 12);
        assert_eq!((align_to_8(1830), align_to_8(812)), (1832, 816));
        let qq_v1 = resolve_preset("qq-v1", None).unwrap();
        assert_eq!(qq_v1.preset, AnimationPreset::QqV1);
        assert_eq!(qq_v1.maximum_long_edge, 1280);
        let archive = resolve_preset("archive", None).unwrap();
        assert_eq!(archive.format, AnimationFormat::Apng);
        assert_eq!(
            resolve_preset("qq", Some(AnimationFormat::Apng))
                .unwrap()
                .format,
            AnimationFormat::Apng
        );
        assert!(resolve_preset("future", None)
            .unwrap_err()
            .contains("ANIMATION_PRESET_UNSUPPORTED"));
    }

    #[test]
    fn qq_v1_budget_steps_are_immutable_and_ordered() {
        let preset = resolve_preset("qq-v1", None).unwrap();
        let steps = animation_budget_steps(&preset);
        assert_eq!(
            steps
                .iter()
                .map(|step| (step.maximum_long_edge, step.fps, step.gif_quality))
                .collect::<Vec<_>>(),
            vec![
                (1280, 20, 80),
                (1280, 20, 65),
                (1280, 15, 65),
                (960, 15, 65)
            ]
        );
        assert!(steps
            .iter()
            .all(|step| step.output_budget_bytes == 10 * 1024 * 1024));

        let web = resolve_preset("web", None).unwrap();
        assert_eq!(animation_budget_steps(&web).len(), 1);
    }

    #[test]
    fn budget_ladder_retries_only_output_budget_failures_in_order() {
        let preset = resolve_preset("qq-v1", None).unwrap();
        let mut attempted = Vec::new();
        let (value, step, attempts) = run_animation_budget_ladder(&preset, |step| {
            attempted.push(step.index);
            if step.index < 3 {
                Err(format!("OUTPUT_BUDGET_EXCEEDED: step {}", step.index))
            } else {
                Ok("encoded")
            }
        })
        .unwrap();
        assert_eq!(value, "encoded");
        assert_eq!(step.index, 3);
        assert_eq!(attempts, 4);
        assert_eq!(attempted, vec![0, 1, 2, 3]);

        let error = run_animation_budget_ladder::<(), _>(&preset, |_| {
            Err("ANIMATION_ENCODE_FAILED: corrupt input".into())
        })
        .unwrap_err();
        assert!(error.contains("ANIMATION_ENCODE_FAILED"));
    }

    #[test]
    fn static_passthrough_never_becomes_animation_work() {
        assert_eq!(
            plan_page_execution(false, "passthrough").unwrap(),
            PageExecution::StaticPassthrough
        );
        assert_eq!(
            plan_page_execution(false, "wrap").unwrap(),
            PageExecution::StaticWrap
        );
        assert_eq!(
            plan_page_execution(true, "passthrough").unwrap(),
            PageExecution::Animation
        );
    }

    #[test]
    fn loop_policy_only_repeats_periodic_programs() {
        use allium_renderer_core::TimelineDescriptor;
        let at_zero = |period_ticks| TimelineDescriptor {
            loop_start_tick: 0,
            period_ticks,
        };

        assert_eq!(
            animation_loop_period_ticks([at_zero(2), at_zero(3)], 6),
            Some(6)
        );
        assert_eq!(
            animation_loop_period_ticks([at_zero(2), at_zero(3)], 5),
            None
        );
        assert_eq!(
            animation_loop_period_ticks(
                [
                    at_zero(2),
                    TimelineDescriptor {
                        loop_start_tick: 1,
                        period_ticks: 2,
                    },
                ],
                8,
            ),
            None
        );
        assert_eq!(animation_loop_period_ticks([], 8), None);
    }

    #[test]
    fn running_tick_window_never_exceeds_the_preset_duration() {
        assert_eq!(plan_running_ticks(6, 20), vec![0, 3]);
        assert_eq!(plan_running_ticks(10, 24), vec![0, 2, 5, 7]);
        assert!(plan_running_ticks(6, 20).iter().all(|tick| *tick < 6));
    }

    #[test]
    fn gif_manifest_duration_uses_encoded_centisecond_delays() {
        let spec = AnimationEncodeSpec {
            width: 1,
            height: 1,
            fps: 15,
            frame_count: 8,
            looped: false,
            output_budget_bytes: 4096,
            gif_quality: 80,
        };
        let encoded =
            encode_rgba_frames(AnimationFormat::Gif, &spec, |_| Ok(vec![255, 0, 0, 255])).unwrap();
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::Indexed);
        let mut decoder = options.read_info(encoded.data.as_slice()).unwrap();
        let mut duration_ms = 0u64;
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            duration_ms += frame.delay as u64 * 10;
        }
        assert_eq!(encoded.duration_ms, duration_ms);
        assert_eq!(duration_ms, 530);
    }

    #[test]
    fn webp_container_loop_count_matches_manifest() {
        for looped in [false, true] {
            let spec = AnimationEncodeSpec {
                width: 1,
                height: 1,
                fps: 20,
                frame_count: 2,
                looped,
                output_budget_bytes: 4096,
                gif_quality: 80,
            };
            let encoded = encode_rgba_frames(AnimationFormat::Webp, &spec, |index| {
                Ok(if index == 0 {
                    vec![255, 0, 0, 255]
                } else {
                    vec![0, 0, 255, 255]
                })
            })
            .unwrap();
            assert_eq!(
                webp_loop_count(&encoded.data),
                Some(if looped { 0 } else { 1 })
            );
            assert_eq!(encoded.looped, looped);
        }
    }

    fn webp_loop_count(data: &[u8]) -> Option<u16> {
        let mut cursor = 12usize;
        while cursor.checked_add(8)? <= data.len() {
            let name = &data[cursor..cursor + 4];
            let size = u32::from_le_bytes(data[cursor + 4..cursor + 8].try_into().ok()?) as usize;
            let payload = cursor.checked_add(8)?;
            let end = payload.checked_add(size)?;
            if end > data.len() {
                return None;
            }
            if name == b"ANIM" && size >= 6 {
                return Some(u16::from_le_bytes(
                    data[payload + 4..payload + 6].try_into().ok()?,
                ));
            }
            cursor = end + (size & 1);
        }
        None
    }
}
