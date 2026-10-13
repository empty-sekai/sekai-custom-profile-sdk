//! Where the software rasteriser samples a device pixel, shared by the native
//! compositor and the WebGL2 shaders.
//!
//! A draw maps each device pixel back into its own space through the inverse
//! of its device matrix, and decides there what the pixel shows:
//!
//! * An image, an alpha mask or a glyph cell takes one sample at the pixel's
//!   centre ([`PIXEL_CENTRE`]). The pixel is drawn when that point lies in the
//!   half-open bounds `[x, x + width) × [y, y + height)`, and inside the
//!   image's clip ([`ellipse_clip_contains`], [`rounded_rect_clip_contains`]).
//!   An image or mask reads the texel under the sample ([`nearest_texel`]); a
//!   bilinearly filtered texture weighs the four texels whose centres surround
//!   it ([`bilinear_taps`]).
//! * A shape counts the [`SHAPE_SAMPLES`] of the pixel that
//!   [`shape_contains`] accepts. Each covered sample adds [`SHAPE_SAMPLE_WEIGHT`]
//!   of the premultiplied colour at that point, the stroke colour where the
//!   shape inset by the stroke width does not contain it and [`shape_fill`]
//!   elsewhere; a pixel with no covered sample is left untouched.
//! * A command's clip keeps the pixels whose centres lie in its device bounds,
//!   which are half-open on both axes ([`span_contains`], [`pixel_span`]). The
//!   bounds are taken from the clip's corners on the canvas, which must form an
//!   axis-aligned rectangle ([`axis_aligned_clip_bounds`]). A glyph quad covers
//!   the pixels whose centres lie, on their row, in the half-open span between
//!   the quad's edges.

use crate::{LinearGradient, Rect, ShapePrimitive};

/// Offset from a device pixel's top-left corner to the point images, masks
/// and glyph cells are sampled at, in pixels.
pub const PIXEL_CENTRE: f32 = 0.5;

/// Offset from a texel's corner to its centre, in texels. Bilinear filtering
/// interpolates between texel centres.
pub const TEXEL_CENTRE: f32 = 0.5;

/// Offsets from a device pixel's top-left corner of the points shape
/// coverage is counted at, in pixels: a two-by-two grid.
pub const SHAPE_SAMPLES: [[f32; 2]; 4] = [[0.25, 0.25], [0.75, 0.25], [0.25, 0.75], [0.75, 0.75]];

/// Share of a pixel each covered shape sample contributes.
pub const SHAPE_SAMPLE_WEIGHT: f32 = 1.0 / SHAPE_SAMPLES.len() as f32;

/// Largest squared length of a gradient line that counts as no line: the
/// whole shape then takes the start colour.
pub const GRADIENT_MIN_LENGTH_SQUARED: f32 = f32::EPSILON;

/// Largest difference, in device pixels, between the coordinates two
/// neighbouring corners of a clip share for the side between them to count as
/// horizontal or vertical.
pub const CLIP_AXIS_TOLERANCE: f32 = 1.0e-4;

/// The texel column (or row) under `coordinate`, a position in `0..=1` across
/// a texture `size` texels wide: the texel whose left edge is nearest below
/// `coordinate * size`, clamped to the texture.
pub fn nearest_texel(coordinate: f32, size: u32) -> u32 {
    (coordinate * size as f32)
        .floor()
        .max(0.0)
        .min(size.saturating_sub(1) as f32) as u32
}

/// The two texel columns (or rows) a bilinear sample at `coordinate` weighs,
/// for a position in `0..=1` across a texture `size` texels wide, and the
/// weight of the second: the sample less [`TEXEL_CENTRE`], clamped between the
/// first and the last texel centre, so the edge texels extend to the border.
pub fn bilinear_taps(coordinate: f32, size: u32) -> (u32, u32, f32) {
    let last = size.saturating_sub(1) as f32;
    let position = (coordinate * size as f32 - TEXEL_CENTRE).clamp(0.0, last);
    let low = position.floor();
    let high = (low + 1.0).min(last);
    (low as u32, high as u32, position - low)
}

/// Whether `(x, y)`, in the shape's local space, lies in the shape after its
/// bounds, and the corner radii of a rounded rectangle, are inset by `inset`.
/// The bounds are half-open; an inset that empties them contains nothing. An
/// asset mask is not a geometric shape and contains nothing.
pub fn shape_contains(
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

/// Straight fill colour of a shape at `(x, y)` in its local space: `fill`, or
/// the gradient's colour at the point's projection onto the gradient line in
/// normalised bounds coordinates, clamped to the line's ends.
pub fn shape_fill(
    fill: [f32; 4],
    gradient: Option<&LinearGradient>,
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
    let t = if denominator <= GRADIENT_MIN_LENGTH_SQUARED {
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

/// Whether `(x, y)`, in the image's local space, lies in the ellipse
/// inscribed in `bounds`. Empty bounds contain nothing.
pub fn ellipse_clip_contains(bounds: Rect, x: f32, y: f32) -> bool {
    let half_width = bounds.width * 0.5;
    let half_height = bounds.height * 0.5;
    if half_width <= 0.0 || half_height <= 0.0 {
        return false;
    }
    let normalized_x = (x - (bounds.x + half_width)) / half_width;
    let normalized_y = (y - (bounds.y + half_height)) / half_height;
    normalized_x.mul_add(normalized_x, normalized_y * normalized_y) <= 1.0
}

/// Whether `(x, y)`, in the image's local space, lies in `bounds` with its
/// corners rounded by `radius`, each radius limited to half the bounds. Empty
/// bounds contain nothing; a zero radius keeps the full rectangle.
pub fn rounded_rect_clip_contains(radius: [f32; 2], bounds: Rect, x: f32, y: f32) -> bool {
    let half_width = bounds.width * 0.5;
    let half_height = bounds.height * 0.5;
    if half_width <= 0.0 || half_height <= 0.0 {
        return false;
    }
    let radius_x = radius[0].abs().min(half_width);
    let radius_y = radius[1].abs().min(half_height);
    if radius_x == 0.0 || radius_y == 0.0 {
        return true;
    }
    let distance_x = (x - (bounds.x + half_width)).abs() - (half_width - radius_x);
    let distance_y = (y - (bounds.y + half_height)).abs() - (half_height - radius_y);
    if distance_x <= 0.0 || distance_y <= 0.0 {
        return true;
    }
    let normalized_x = distance_x / radius_x;
    let normalized_y = distance_y / radius_y;
    normalized_x.mul_add(normalized_x, normalized_y * normalized_y) <= 1.0
}

/// Device bounds `[min_x, min_y, max_x, max_y]` of a clip whose corners, in
/// order around it, form an axis-aligned rectangle: every side is horizontal
/// or vertical within [`CLIP_AXIS_TOLERANCE`] and longer than it. Any other
/// quad, or one with a non-finite corner, has none.
pub fn axis_aligned_clip_bounds(corners: [[f32; 2]; 4]) -> Option<[f32; 4]> {
    if !corners.iter().flatten().all(|value| value.is_finite()) {
        return None;
    }
    let horizontal = |left: [f32; 2], right: [f32; 2]| {
        (left[1] - right[1]).abs() <= CLIP_AXIS_TOLERANCE
            && (left[0] - right[0]).abs() > CLIP_AXIS_TOLERANCE
    };
    let vertical = |top: [f32; 2], bottom: [f32; 2]| {
        (top[0] - bottom[0]).abs() <= CLIP_AXIS_TOLERANCE
            && (top[1] - bottom[1]).abs() > CLIP_AXIS_TOLERANCE
    };
    let rectangle = (horizontal(corners[0], corners[1])
        && vertical(corners[1], corners[2])
        && horizontal(corners[2], corners[3])
        && vertical(corners[3], corners[0]))
        || (vertical(corners[0], corners[1])
            && horizontal(corners[1], corners[2])
            && vertical(corners[2], corners[3])
            && horizontal(corners[3], corners[0]));
    if !rectangle {
        return None;
    }
    let min = |axis: usize| {
        corners
            .iter()
            .map(|corner| corner[axis])
            .fold(f32::INFINITY, f32::min)
    };
    let max = |axis: usize| {
        corners
            .iter()
            .map(|corner| corner[axis])
            .fold(f32::NEG_INFINITY, f32::max)
    };
    Some([min(0), min(1), max(0), max(1)])
}

/// Whether a span reaching from `min` to `max` along one axis holds the point
/// at `value` on that axis: the span is half-open, `[min, max)`, so of two
/// spans that meet exactly one holds the point where they meet.
pub fn span_contains(min: f32, max: f32, value: f32) -> bool {
    value >= min && value < max
}

/// The device pixels along one axis whose centres a span from `min` to `max`
/// holds ([`span_contains`]), as the first of them and the one after the last.
pub fn pixel_span(min: f32, max: f32) -> (f32, f32) {
    ((min - PIXEL_CENTRE).ceil(), (max - PIXEL_CENTRE).ceil())
}

#[cfg(test)]
mod tests;
