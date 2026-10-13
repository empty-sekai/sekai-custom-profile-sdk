//! Frame encoders: format dispatch, frame and output-budget checks, and container
//! validation of finished artifacts.

mod apng;
pub(super) mod gif_encoder;
pub(super) mod mp4;
mod webp;
pub(super) mod yuv;

use self::apng::encode_apng;
use self::gif_encoder::{encode_gif, gif_delay_centiseconds};
use self::mp4::{encode_mp4, H264EncoderConfig};
use self::webp::encode_webp;
use super::compositor::{blit_layer, read_canvas_rgba_into, FrameRect, PixelRect};
use super::{AnimationEncodeSpec, AnimationFormat, EncodedAnimation, ResolvedAnimationPreset};

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
pub(super) struct AnimationEncodeTelemetry {
    pub(super) frame_callback_calls: u64,
    pub(super) frame_callback_ns: u64,
    pub(super) gif_palette_sample_ns: u64,
    pub(super) gif_palette_quantize_ns: u64,
    pub(super) gif_delta_index_ns: u64,
    pub(super) gif_lzw_ns: u64,
    pub(super) gif_retained_frame_bytes: usize,
    pub(super) gif_second_frame_pass: bool,
    pub(super) h264_yuv_ns: u64,
    pub(super) h264_pipe_write_ns: u64,
    pub(super) h264_codec_wait_ns: u64,
    pub(super) h264_output_read_ns: u64,
    pub(super) h264_direct_encode_ns: u64,
    pub(super) h264_mux_ns: u64,
    pub(super) h264_mbinfo_constant_blocks: u64,
    pub(super) h264_mbinfo_total_blocks: u64,
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

pub(super) fn encode_rgba_frames_with_h264_and_telemetry<F>(
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
    let expected = frame_byte_len(spec.width, spec.height)?;
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

pub(super) fn finish_encoded_animation(
    format: AnimationFormat,
    spec: &AnimationEncodeSpec,
    expected: usize,
    data: Vec<u8>,
) -> Result<EncodedAnimation, String> {
    if data.len() > spec.output_budget_bytes {
        return Err(output_budget_exceeded(data.len(), spec.output_budget_bytes));
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
    let decoded = crate::codec::png::decode(png)
        .map_err(|error| format!("decode static PNG for animation wrap failed: {error}"))?;
    let source_width = decoded.width;
    let source_height = decoded.height;
    let scale = (preset.maximum_long_edge as f32 / source_width.max(source_height) as f32).min(1.0);
    let width = (source_width as f32 * scale).round() as u32;
    let height = (source_height as f32 * scale).round() as u32;
    let bytes = frame_byte_len(width, height)?;
    // The decoder yields non-premultiplied samples; the blit composites in
    // premultiplied space, so premultiply on the way in and recover on the way
    // out.
    let mut source = decoded.pixels;
    for pixel in source.chunks_exact_mut(4) {
        let alpha = pixel[3];
        for channel in 0..3 {
            pixel[channel] = crate::codec::premultiply_channel(pixel[channel], alpha);
        }
    }
    let mut canvas = vec![0u8; bytes];
    blit_layer(
        &mut canvas,
        width,
        height,
        &source,
        source_width,
        source_height,
        FrameRect::from_xywh(0.0, 0.0, width as f32, height as f32),
        PixelRect::full(width, height),
    );
    let mut rgba = vec![0u8; bytes];
    read_canvas_rgba_into(&canvas, width, height, &mut rgba)?;
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

pub(super) fn validate_spec(spec: &AnimationEncodeSpec) -> Result<(), String> {
    if spec.width == 0 || spec.height == 0 || spec.frame_count == 0 || spec.fps == 0 {
        return Err("animation dimensions/frame_count/fps must be positive".into());
    }
    if spec.fps > 60 {
        return Err("animation fps must not exceed the 60Hz core clock".into());
    }
    frame_byte_len(spec.width, spec.height)?;
    Ok(())
}

/// Bytes in one tight RGBA8 frame.
pub(super) fn frame_byte_len(width: u32, height: u32) -> Result<usize, String> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| format!("animation frame size overflow: {width}x{height}"))
}

pub(super) fn output_budget_exceeded(bytes: usize, budget: usize) -> String {
    format!("OUTPUT_BUDGET_EXCEEDED: {bytes} > {budget}")
}

pub(super) fn checked_frame<F>(
    frame_at: &mut F,
    index: u32,
    expected: usize,
) -> Result<Vec<u8>, String>
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

pub(super) fn checked_frame_timed<F>(
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

pub(super) fn validate_mp4_container(data: &[u8]) -> Result<(), &'static str> {
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
