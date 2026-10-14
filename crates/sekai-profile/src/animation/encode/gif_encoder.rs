//! GIF encoding: palette training and lookup, per-frame index deltas and the LZW
//! writer.

use super::{checked_frame_timed, output_budget_exceeded, AnimationEncodeTelemetry};
use crate::animation::AnimationEncodeSpec;

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

pub(in crate::animation) struct GifPaletteLookup {
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

    pub(in crate::animation) fn new(memory_budget: usize, active_colors: usize) -> Self {
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

    pub(in crate::animation) fn set_palette(&mut self, palette: &[u8]) {
        for (index, color) in palette
            .as_chunks::<3>()
            .0
            .iter()
            .take(self.active_colors)
            .enumerate()
        {
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

pub(in crate::animation) struct FastGifLzwEncoder {
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

    pub(in crate::animation) fn new() -> Self {
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

    pub(in crate::animation) fn encode(&mut self, pixels: &[u8]) -> Vec<u8> {
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

pub(super) fn encode_gif<F>(
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
    // The logical screen and every image descriptor store 16-bit extents.
    if spec.width > u32::from(u16::MAX) || spec.height > u32::from(u16::MAX) {
        return Err(format!(
            "GIF frames are limited to 65535x65535 pixels, got {}x{}",
            spec.width, spec.height
        ));
    }
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
            rgba.as_chunks::<4>()
                .0
                .iter()
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
    for (index, color) in palette
        .as_chunks::<3>()
        .0
        .iter()
        .take(active_colors)
        .enumerate()
    {
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
                        &mut previous_indexed,
                    )
                } else {
                    let pixels = index_gif_pixels(&rgba, &palette, &mut palette_lookup);
                    previous_indexed.clone_from(&pixels);
                    (0, 0, spec.width, spec.height, pixels)
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
            // The stream only grows, so stop as soon as it crosses the budget.
            let written = encoder.get_ref().len();
            if written > spec.output_budget_bytes {
                return Err(output_budget_exceeded(written, spec.output_budget_bytes));
            }
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
            let mut delta = GifIndexDelta::new(*left, *top, *width, *height);
            for (byte_index, &byte) in changed.iter().enumerate() {
                let mut bits = byte;
                while bits != 0 {
                    let bit = bits.trailing_zeros() as usize;
                    let index = byte_index * 8 + bit;
                    bits &= bits - 1;
                    if index >= pixel_count {
                        continue;
                    }
                    let value = indexed_changed[rgba_index / 4];
                    rgba_index += 4;
                    delta.update(
                        index as u32 % *width,
                        index as u32 / *width,
                        value,
                        spec.width,
                        previous_indexed,
                    );
                }
            }
            debug_assert_eq!(rgba_index, rgba.len());
            delta.finish()
        }
        GifRetainedFrame::Unchanged => (0, 0, 1, 1, vec![255]),
    }
}

/// One delta frame in palette-index space.
///
/// Only pixels whose index differs from the index already on the canvas are
/// written; everything else stays transparent, and the frame is cropped to
/// the written pixels. The retained and the streamed encode pass both build
/// their deltas through this type, so the bytes do not depend on which pass
/// ran.
struct GifIndexDelta {
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
}

impl GifIndexDelta {
    fn new(left: u32, top: u32, width: u32, height: u32) -> Self {
        Self {
            left,
            top,
            width,
            height,
            pixels: vec![255u8; width as usize * height as usize],
            min_x: width,
            min_y: height,
            max_x: 0,
            max_y: 0,
        }
    }

    /// Records the index of a pixel whose RGBA changed, at `(x, y)` inside
    /// the delta rectangle, against the indices already on the canvas.
    fn update(&mut self, x: u32, y: u32, value: u8, canvas_width: u32, canvas: &mut [u8]) {
        let canvas_index = ((self.top + y) * canvas_width + self.left + x) as usize;
        if canvas[canvas_index] == value {
            return;
        }
        canvas[canvas_index] = value;
        self.pixels[(y * self.width + x) as usize] = value;
        self.min_x = self.min_x.min(x);
        self.min_y = self.min_y.min(y);
        self.max_x = self.max_x.max(x);
        self.max_y = self.max_y.max(y);
    }

    fn finish(self) -> (u32, u32, u32, u32, Vec<u8>) {
        let Self {
            left,
            top,
            width,
            height,
            pixels,
            min_x,
            min_y,
            max_x,
            max_y,
        } = self;
        if min_x == width || min_y == height {
            return (0, 0, 1, 1, vec![255]);
        }
        if min_x != 0 || min_y != 0 || max_x + 1 != width || max_y + 1 != height {
            let cropped_width = max_x - min_x + 1;
            let cropped_height = max_y - min_y + 1;
            let mut cropped = Vec::with_capacity(cropped_width as usize * cropped_height as usize);
            for y in min_y..=max_y {
                let start = (y * width + min_x) as usize;
                cropped.extend_from_slice(&pixels[start..start + cropped_width as usize]);
            }
            return (
                left + min_x,
                top + min_y,
                cropped_width,
                cropped_height,
                cropped,
            );
        }
        (left, top, width, height, pixels)
    }
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
    for pixel in samples.as_chunks_mut::<4>().0 {
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
    for pixel in samples.as_chunks_mut::<4>().0 {
        pixel[3] = 255;
    }
    color_quant::NeuQuant::new(speed, 255, samples).color_map_rgb()
}

fn gif_palette_color_count(quality: u8) -> usize {
    let _ = quality;
    255
}

pub(in crate::animation) fn gif_delay_centiseconds(frame_index: u32, fps: u32) -> u16 {
    let start = (frame_index as u64 * 100 + fps as u64 / 2) / fps as u64;
    let end = ((frame_index as u64 + 1) * 100 + fps as u64 / 2) / fps as u64;
    (end - start).max(1) as u16
}

/// Streams one delta frame: indexes the pixels whose RGBA changed since
/// `previous`, and keeps those whose index differs from `previous_indexed`.
pub(in crate::animation) fn gif_delta_indexed(
    previous: &[u8],
    current: &[u8],
    width: u32,
    height: u32,
    palette: &[u8],
    palette_lookup: &mut GifPaletteLookup,
    previous_indexed: &mut [u8],
) -> (u32, u32, u32, u32, Vec<u8>) {
    let Some((min_x, min_y, max_x, max_y)) = gif_changed_bounds(previous, current, width, height)
    else {
        return (0, 0, 1, 1, vec![255]);
    };
    let mut delta = GifIndexDelta::new(min_x, min_y, max_x - min_x + 1, max_y - min_y + 1);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let index = (y * width + x) as usize * 4;
            if previous[index..index + 4] == current[index..index + 4] {
                continue;
            }
            let value =
                nearest_gif_palette_index(&current[index..index + 4], palette, palette_lookup);
            delta.update(x - min_x, y - min_y, value, width, previous_indexed);
        }
    }
    delta.finish()
}

pub(in crate::animation) fn gif_changed_bounds(
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

pub(in crate::animation) fn gif_changed_bounds_scalar(
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
        .as_chunks::<4>()
        .0
        .iter()
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

/// Vectorized changed-pixel bounds over two RGBA frames.
///
/// # Safety
///
/// The caller must verify `avx512f` support, since the body loads ZMM
/// registers unconditionally.
///
/// `previous` and `current` must each be valid for
/// `width as usize * height as usize * 4` bytes, and that product must not
/// overflow `usize`. For every row `y` the body forms raw row pointers at
/// `y * width * 4`, loads 64 bytes at `x * 4` while `x + 16 <= width`, and
/// reads the remaining columns of the row as unaligned `u32`s.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
pub(in crate::animation) unsafe fn gif_changed_bounds_avx512(
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
    rgba.as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| nearest_gif_palette_index(pixel, palette, palette_lookup))
        .collect()
}

/// Indexes a whole RGBA buffer against the palette, with a vectorized main
/// loop over the direct-mapped cache.
///
/// # Safety
///
/// The caller must verify `avx512f` support, since the body uses AVX-512
/// loads, gathers and scatters unconditionally.
///
/// When `palette_lookup.direct` is `Some`, its table must hold at least
/// `GifPaletteLookup::DIRECT_SLOTS` entries, as `GifPaletteLookup::new`
/// allocates it: the body gathers from and scatters to that table through a
/// raw pointer at slot indices masked with `DIRECT_SLOTS - 1`.
///
/// The vector loop reads 64 bytes of `rgba` at `offset * 4` only while
/// `offset + 16 <= rgba.len() / 4` and stores 16 bytes of the output at
/// `offset`, so those accesses stay in bounds for any `rgba`.
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
                .as_chunks::<3>()
                .0
                .iter()
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

pub(in crate::animation) fn nearest_gif_palette_index_scalar(
    pixel: &[u8],
    palette: &[u8],
    active_colors: usize,
) -> u8 {
    let mut best_index = 0u8;
    let mut best_distance = u32::MAX;
    for (index, color) in palette
        .as_chunks::<3>()
        .0
        .iter()
        .take(active_colors)
        .enumerate()
    {
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

/// Nearest palette entry to one opaque RGBA pixel, scanning the palette in
/// 16-entry vector batches.
///
/// # Safety
///
/// The caller must verify `avx512f` support, since the body loads ZMM
/// registers unconditionally.
///
/// `palette.active_colors` must not exceed 256. The body loads 16 lanes at a
/// time from `palette_r`, `palette_g` and `palette_b`, each a `[u32; 256]`, up
/// to `active_colors.div_ceil(16) * 16` entries, which stays inside the arrays
/// only under that bound. `GifPaletteLookup::new` clamps `active_colors` to
/// `1..=255`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
pub(in crate::animation) unsafe fn nearest_gif_palette_index_avx512(
    pixel: &[u8],
    palette: &GifPaletteLookup,
) -> u8 {
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
