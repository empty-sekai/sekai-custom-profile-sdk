//! Animated output: presets, encoder configuration and cache identity.
//!
//! A card with moving elements is encoded as a short clip. [`AnimationPreset`]
//! names the quality/size tradeoffs available, [`H264EncoderConfig`] carries the
//! resolved encoder settings, and [`animation_encoder_cache_suffix`] derives the
//! identity a cached encode is keyed by.

#[cfg(test)]
use std::io::Cursor;

use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::types::CustomProfileCard;

mod compositor;
mod deferred;
mod encode;
mod export;
mod timeline;

pub use self::encode::mp4::{H264EncoderConfig, H264YuvConverter};
pub use self::encode::{
    encode_rgba_frames, validate_encoded_artifact, validate_magic, wrap_static_png,
};
pub(crate) use self::export::export_profile_animation;

#[cfg(test)]
use self::{
    compositor::*, deferred::*, encode::gif_encoder::*, encode::mp4::*, encode::yuv::*, encode::*,
    export::*, timeline::*,
};

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
        // MP4 streams frames directly into libx264, so it does not retain a
        // frame set. It still shares the same export-owned working-set budget:
        // oversized source layers are rendered through bounded viewport
        // windows instead of materializing a multi-gigabyte offscreen raster.
        export_memory_budget_bytes: memory_mib * 1024 * 1024,
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
    pub render_telemetry: Option<crate::telemetry::RenderTelemetry>,
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
    let static_execution = match static_policy {
        "passthrough" => PageExecution::StaticPassthrough,
        "wrap" => PageExecution::StaticWrap,
        value => return Err(format!("unsupported static_policy={value}")),
    };
    Ok(if animated {
        PageExecution::Animation
    } else {
        static_execution
    })
}

fn elapsed_ns(started: std::time::Instant) -> u64 {
    started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests;
