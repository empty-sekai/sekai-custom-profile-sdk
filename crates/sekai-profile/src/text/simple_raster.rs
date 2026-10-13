//! Dependency-free raster for short white text runs (the Live Master progress
//! digits) with pixels identical to the Skia recipe it replaces.
//!
//! The replaced path was `SkFont(face, size)` + `Canvas::draw_str` with an
//! anti-aliased opaque white fill and no subpixel positioning. Its pixels are
//! reproduced exactly by three pieces:
//!
//! 1. FreeType glyphs loaded with normal hinting (`FT_LOAD_TARGET_NORMAL`) and
//!    rendered to 8-bit coverage; the pen advances by the hinted advance and
//!    every glyph blits at its rounded pen position.
//! 2. Skia's text-mask gamma for white paint, a monotone 256-entry coverage
//!    table. It was recovered empirically by rasterizing the same glyphs
//!    through both engines over sizes 6..=72 and comparing coverage byte for
//!    byte: 41,954 samples, zero conflicts, all 256 entries observed.
//! 3. The A8 source-over blend for an opaque white source:
//!    `out = d + ((255 - d) * (coverage + 1) >> 8)` per channel, fitted the
//!    same way over every observed `(coverage, destination)` pair.
//!
//! Under a transform the recipe follows Skia's FreeType scaler. Glyph
//! positions stay the hinted advances at the source size, mapped through the
//! transform and rounded to whole pixels. Glyph images are rendered in device
//! space: the transform, text size included, is split by a Givens rotation
//! into an axis scale folded into the character size and a remaining rotation
//! or skew set as the FreeType transform, and hinting is dropped when the
//! transform is not axis-aligned.

use freetype::face::LoadFlag;
use freetype::{Face, Library, Matrix, Vector};

/// A 2×3 affine transform `[a, b, c, d, e, f]` sending `(x, y)` to
/// `(a·x + c·y + e, b·x + d·y + f)`.
pub(crate) type Affine = [f32; 6];

/// Scales below this never reach a pixel; Skia draws nothing for them.
const NEARLY_ZERO: f32 = 1.0 / 4096.0;

/// Coverage remap Skia applies to anti-aliased text masks drawn with a white
/// paint (its mask-gamma preblend). Recovered empirically; see the module
/// documentation for the extraction and its sample counts.
const WHITE_TEXT_COVERAGE: [u8; 256] = [
    0, 13, 22, 28, 34, 38, 42, 46, 50, 53, 56, 59, 61, 64, 66, 69, 71, 73, 75, 77, 79, 81, 83, 85,
    86, 88, 90, 92, 93, 95, 96, 98, 99, 101, 102, 104, 105, 106, 108, 109, 110, 112, 113, 114, 115,
    117, 118, 119, 120, 121, 122, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 136,
    137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148, 148, 149, 150, 151, 152, 153, 154,
    155, 155, 156, 157, 158, 159, 159, 160, 161, 162, 163, 163, 164, 165, 166, 167, 167, 168, 169,
    170, 170, 171, 172, 173, 173, 174, 175, 175, 176, 177, 178, 178, 179, 180, 180, 181, 182, 182,
    183, 184, 185, 185, 186, 187, 187, 188, 189, 189, 190, 190, 191, 192, 192, 193, 194, 194, 195,
    196, 196, 197, 197, 198, 199, 199, 200, 200, 201, 202, 202, 203, 203, 204, 205, 205, 206, 206,
    207, 208, 208, 209, 209, 210, 210, 211, 212, 212, 213, 213, 214, 214, 215, 215, 216, 216, 217,
    218, 218, 219, 219, 220, 220, 221, 221, 222, 222, 223, 223, 224, 224, 225, 226, 226, 227, 227,
    228, 228, 229, 229, 230, 230, 231, 231, 232, 232, 233, 233, 234, 234, 235, 235, 236, 236, 237,
    237, 238, 238, 238, 239, 239, 240, 240, 241, 241, 242, 242, 243, 243, 244, 244, 245, 245, 246,
    246, 246, 247, 247, 248, 248, 249, 249, 250, 250, 251, 251, 251, 252, 252, 253, 253, 254, 254,
    255, 255,
];

/// Draws `text` in opaque white over premultiplied RGBA through `matrix`,
/// horizontally centered on the local origin with its baseline at local
/// `baseline`. Returns `false` when the family's font file is not available.
pub(crate) fn draw_centered_white_text(
    destination: &mut [u8],
    width: u32,
    height: u32,
    family: &str,
    text: &str,
    font_size: f32,
    matrix: Affine,
    baseline: f32,
) -> Result<bool, String> {
    let Some(bytes) = crate::sdf::outline::load_font_bytes_for_family(family) else {
        return Ok(false);
    };
    let library = Library::init().map_err(|error| format!("初始化 FreeType 失败: {error:?}"))?;
    let face = library
        .new_memory_face2(bytes.as_slice(), 0)
        .map_err(|error| format!("加载字体 {family} 失败: {error:?}"))?;
    set_char_size(&face, font_size, font_size)?;

    // Hinted advances at the source size feed both the measurement and the
    // pen, so centering and glyph placement stay on one metric.
    let mut advances = Vec::new();
    for ch in text.chars() {
        face.load_char(ch as usize, LoadFlag::TARGET_NORMAL)
            .map_err(|error| format!("加载字形 {ch:?} 失败: {error:?}"))?;
        advances.push(face.glyph().raw().advance.x as f32 / 64.0);
    }
    let text_width: f32 = advances.iter().sum();

    let Some(strike) = DeviceStrike::new(matrix, font_size) else {
        return Ok(true);
    };
    set_char_size(&face, strike.scale[0], strike.scale[1])?;
    let mut flags = if strike.hinted {
        LoadFlag::TARGET_NORMAL
    } else {
        LoadFlag::NO_HINTING
    };
    if let Some(mut remaining) = strike.remaining {
        face.set_transform(&mut remaining, &mut Vector { x: 0, y: 0 });
        flags |= LoadFlag::NO_BITMAP;
    }

    let width = width as i32;
    let height = height as i32;
    let [a, b, c, d, e, f] = matrix;
    let start = -text_width / 2.0;
    let mut pen_x = a * start + c * baseline + e;
    let mut pen_y = b * start + d * baseline + f;
    for (ch, advance) in text.chars().zip(advances) {
        face.load_char(ch as usize, flags | LoadFlag::RENDER)
            .map_err(|error| format!("渲染字形 {ch:?} 失败: {error:?}"))?;
        let glyph = face.glyph();
        let bitmap = glyph.bitmap();
        // No subpixel positioning: each glyph blits at its rounded pen point.
        let left = (pen_x + 0.5).floor() as i32 + glyph.bitmap_left();
        let top = (pen_y + 0.5).floor() as i32 - glyph.bitmap_top();
        let pitch = bitmap.pitch();
        let data = bitmap.buffer();
        for row in 0..bitmap.rows() {
            let y = top + row;
            if !(0..height).contains(&y) {
                continue;
            }
            for col in 0..bitmap.width() {
                let x = left + col;
                if !(0..width).contains(&x) {
                    continue;
                }
                let coverage =
                    u32::from(WHITE_TEXT_COVERAGE[data[(row * pitch + col) as usize] as usize]);
                if coverage == 0 {
                    continue;
                }
                let index = ((y * width + x) * 4) as usize;
                for channel in &mut destination[index..index + 4] {
                    let dst = u32::from(*channel);
                    *channel = (dst + (((255 - dst) * (coverage + 1)) >> 8)) as u8;
                }
            }
        }
        pen_x += a * advance;
        pen_y += b * advance;
    }
    Ok(true)
}

/// Sets the character size in points at 72 dpi, truncated to 26.6 like Skia.
fn set_char_size<B>(face: &Face<B>, width: f32, height: f32) -> Result<(), String> {
    face.set_char_size((width * 64.0) as isize, (height * 64.0) as isize, 72, 72)
        .map_err(|error| format!("设置字号失败: {error:?}"))
}

/// How a text size is realized under a device transform: the character size,
/// the FreeType transform left over, and whether glyphs are hinted.
#[derive(Debug)]
struct DeviceStrike {
    scale: [f32; 2],
    remaining: Option<Matrix>,
    hinted: bool,
}

impl DeviceStrike {
    /// Splits `matrix` scaled by `font_size`; `None` when it collapses the
    /// text to nothing or is not finite.
    fn new(matrix: Affine, font_size: f32) -> Option<Self> {
        let [a, b, c, d] = [matrix[0], matrix[1], matrix[2], matrix[3]].map(|v| v * font_size);
        // Rotate where the baseline lands onto +x so the diagonal left over
        // is the axis scale.
        let (scale_x, scale_y) = if b != 0.0 || c != 0.0 || a < 0.0 || d < 0.0 {
            let (cos, sin) = givens(a, b);
            (cos * a - sin * b, sin * c + cos * d)
        } else {
            (a, d)
        };
        let scale = [scale_x.abs(), scale_y.abs()];
        if !(scale[0] > NEARLY_ZERO && scale[1] > NEARLY_ZERO)
            || !scale.iter().all(|v| v.is_finite())
        {
            return None;
        }
        let [xx, yx, xy, yy] = [a / scale[0], b / scale[0], c / scale[1], d / scale[1]];
        let remaining = (xx != 1.0 || yx != 0.0 || xy != 0.0 || yy != 1.0).then(|| Matrix {
            // FreeType's y axis points up, so the off-diagonal terms flip.
            xx: fixed(xx),
            xy: fixed(-xy),
            yx: fixed(-yx),
            yy: fixed(yy),
        });
        let [a, b, c, d] = [matrix[0], matrix[1], matrix[2], matrix[3]];
        let axis_aligned = (b == 0.0 && c == 0.0) || (a == 0.0 && d == 0.0);
        Some(Self {
            scale,
            remaining,
            hinted: axis_aligned,
        })
    }
}

/// Cosine and sine of the rotation `[cos, -sin; sin, cos]` that takes the
/// vector `(x, y)` onto the positive x axis.
fn givens(x: f32, y: f32) -> (f32, f32) {
    if y == 0.0 {
        (1.0f32.copysign(x), 0.0)
    } else if x == 0.0 {
        (0.0, -(1.0f32.copysign(y)))
    } else if y.abs() > x.abs() {
        let t = x / y;
        let u = (1.0 + t * t).sqrt().copysign(y);
        let sin = -1.0 / u;
        (-sin * t, sin)
    } else {
        let t = y / x;
        let u = (1.0 + t * t).sqrt().copysign(x);
        let cos = 1.0 / u;
        (cos, -cos * t)
    }
}

/// 16.16 fixed point, truncated toward zero.
fn fixed(value: f32) -> freetype::ffi::FT_Fixed {
    (value * 65536.0) as i32 as freetype::ffi::FT_Fixed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_coverage_table_is_monotone_and_full_range() {
        assert_eq!(WHITE_TEXT_COVERAGE[0], 0);
        assert_eq!(WHITE_TEXT_COVERAGE[255], 255);
        for pair in WHITE_TEXT_COVERAGE.windows(2) {
            assert!(pair[0] <= pair[1], "the remap must stay monotone");
        }
    }

    #[test]
    fn an_untransformed_or_scaled_run_keeps_hinting_and_needs_no_freetype_transform() {
        for (matrix, scale) in [
            ([1.0, 0.0, 0.0, 1.0, 80.0, 20.0], [20.0, 20.0]),
            ([0.8, 0.0, 0.0, 0.8, 0.0, 0.0], [16.0, 16.0]),
            ([2.0, 0.0, 0.0, 0.5, 0.0, 0.0], [40.0, 10.0]),
        ] {
            let strike = DeviceStrike::new(matrix, 20.0).unwrap();
            assert_eq!(strike.scale, scale);
            assert!(strike.remaining.is_none() && strike.hinted, "{matrix:?}");
        }
    }

    #[test]
    fn rotations_and_flips_move_into_the_freetype_transform() {
        let (sin, cos) = 30f32.to_radians().sin_cos();
        let rotated = DeviceStrike::new([cos, sin, -sin, cos, 0.0, 0.0], 20.0).unwrap();
        assert!((rotated.scale[0] - 20.0).abs() < 1e-4 && (rotated.scale[1] - 20.0).abs() < 1e-4);
        let remaining = rotated.remaining.unwrap();
        assert_eq!(remaining.xy, -remaining.yx);
        assert!(!rotated.hinted, "rotated text is drawn unhinted");

        let quarter = DeviceStrike::new([0.0, 1.0, -1.0, 0.0, 0.0, 0.0], 20.0).unwrap();
        assert_eq!(quarter.scale, [20.0, 20.0]);
        assert!(quarter.remaining.is_some() && quarter.hinted);

        let mirrored = DeviceStrike::new([-1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 20.0).unwrap();
        assert_eq!(mirrored.scale, [20.0, 20.0]);
        let remaining = mirrored.remaining.unwrap();
        assert_eq!((remaining.xx, remaining.yy), (-65536, 65536));
    }

    #[test]
    fn a_collapsed_or_invalid_transform_draws_nothing() {
        assert!(DeviceStrike::new([0.0, 0.0, 0.0, 1.0, 0.0, 0.0], 20.0).is_none());
        assert!(DeviceStrike::new([1.0, 0.0, 0.0, 1e-6, 0.0, 0.0], 20.0).is_none());
        assert!(DeviceStrike::new([f32::NAN, 0.0, 0.0, 1.0, 0.0, 0.0], 20.0).is_none());
    }

    #[test]
    fn the_blend_is_exact_at_the_coverage_endpoints() {
        for dst in 0..=255u32 {
            assert_eq!(dst + (((255 - dst) * 256) >> 8), 255, "full coverage");
            assert_eq!(dst + (((255 - dst) * 1) >> 8), dst, "zero coverage stays");
        }
    }
}
