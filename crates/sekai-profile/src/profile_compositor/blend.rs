//! RGBA8 blend equations and the merge of isolated groups into their parent.

use sekai_profile_core::{BlendMode, Matrix2d, Rect};

#[cfg(target_arch = "x86_64")]
use super::simd_x86;
use super::{transform_point, AxisAlignedClip, CompositionBounds, ImageExecutor};

/// Narrows composition bounds to a device-space clip.
///
/// The clip edges are rounded outward so a partially covered pixel is kept; the
/// group's own coverage decides how much of it shows. Returns `None` when the
/// intersection is empty.
fn intersect_bounds_with_clip(
    bounds: CompositionBounds,
    clip: AxisAlignedClip,
    canvas_width: u32,
) -> Option<CompositionBounds> {
    let clamp = |value: f32, limit: u32| value.clamp(0.0, limit as f32) as u32;
    let x0 = bounds.x0.max(clamp(clip.min_x.floor(), canvas_width));
    let x1 = bounds.x1.min(clamp(clip.max_x.ceil(), canvas_width));
    let y0 = bounds.y0.max(clamp(clip.min_y.floor(), u32::MAX));
    let y1 = bounds.y1.min(clamp(clip.max_y.ceil(), u32::MAX));
    (x0 < x1 && y0 < y1).then_some(CompositionBounds { x0, y0, x1, y1 })
}

pub(super) fn blend_pixel(destination: &mut [u8], source: [u8; 4], blend_mode: BlendMode) {
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
        // The separable modes below are written for premultiplied input, so the
        // same expression applies to alpha as to the colour channels. For
        // Multiply and Screen that yields `Sa + Da - Sa*Da`, the coverage union
        // both modes are defined to produce.
        BlendMode::Multiply => std::array::from_fn(|channel| {
            let source_channel = source[channel];
            let destination_channel = destination[channel];
            let product = mul_div_255_round(source_channel, destination_channel);
            let source_outside = mul_div_255_round(source_channel, u8::MAX - destination[3]);
            let destination_outside = mul_div_255_round(destination_channel, u8::MAX - source[3]);
            u16::from(product)
                .saturating_add(u16::from(source_outside))
                .saturating_add(u16::from(destination_outside))
                .min(u16::from(u8::MAX)) as u8
        }),
        BlendMode::Screen => std::array::from_fn(|channel| {
            let sum = u16::from(source[channel]).saturating_add(u16::from(destination[channel]));
            sum.saturating_sub(u16::from(mul_div_255_round(
                source[channel],
                destination[channel],
            )))
            .min(u16::from(u8::MAX)) as u8
        }),
        BlendMode::Add => std::array::from_fn(|channel| {
            u16::from(source[channel])
                .saturating_add(u16::from(destination[channel]))
                .min(u16::from(u8::MAX)) as u8
        }),
    };
    destination.copy_from_slice(&output);
}

#[inline]
pub(super) fn mul_div_255_round(left: u8, right: u8) -> u8 {
    let product = u32::from(left) * u32::from(right);
    ((product + 127) / 255) as u8
}

pub(super) fn composition_bounds(
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

pub(super) fn clear_composition_bounds(
    pixels: &mut [u8],
    canvas_width: u32,
    bounds: CompositionBounds,
) {
    let row_bytes = canvas_width as usize * 4;
    let x0 = bounds.x0 as usize * 4;
    let x1 = bounds.x1 as usize * 4;
    for y in bounds.y0..bounds.y1 {
        let row = y as usize * row_bytes;
        pixels[row + x0..row + x1].fill(0);
    }
}

/// Merges an isolated group into its parent.
///
/// `opacity` scales the group as a whole, which is why it is applied here rather
/// than per command: scaling each command instead would let overlapping strokes
/// within the group show through one another. The group is premultiplied, so the
/// scale applies to all four channels.
///
/// `clip` restricts the merge to a device-space rectangle. Rows and columns
/// outside it are skipped rather than blended with zero coverage, so an
/// unclipped merge keeps its original packet stride.
pub(super) fn composite_isolation(
    parent: &mut [u8],
    child: &[u8],
    canvas_width: u32,
    bounds: CompositionBounds,
    opacity: f32,
    clip: Option<AxisAlignedClip>,
    executor: ImageExecutor,
) {
    let bounds = match clip {
        Some(clip) => match intersect_bounds_with_clip(bounds, clip, canvas_width) {
            Some(clipped) => clipped,
            None => return,
        },
        None => bounds,
    };
    // Anything below half a quantisation step contributes nothing.
    let opaque = opacity >= 1.0;
    let scale = if opaque {
        u8::MAX
    } else if opacity <= 0.0 {
        return;
    } else {
        let quantized = (opacity * 255.0).round().clamp(0.0, 255.0) as u8;
        if quantized == 0 {
            return;
        }
        quantized
    };
    let row_bytes = canvas_width as usize * 4;
    for y in bounds.y0..bounds.y1 {
        let row = y as usize * row_bytes;
        let mut x = bounds.x0;
        while x < bounds.x1 {
            let packet = (bounds.x1 - x).min(16);
            if !opaque {
                // A scaled group cannot use the packet blender, which reads the
                // source straight from memory.
                for lane in 0..packet {
                    let pixel_offset = row + (x + lane) as usize * 4;
                    let source = &child[pixel_offset..pixel_offset + 4];
                    let scaled: [u8; 4] =
                        std::array::from_fn(|channel| mul_div_255_round(source[channel], scale));
                    if scaled[3] != 0 {
                        blend_pixel(
                            &mut parent[pixel_offset..pixel_offset + 4],
                            scaled,
                            BlendMode::SrcOver,
                        );
                    }
                }
                x += packet;
                continue;
            }
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
