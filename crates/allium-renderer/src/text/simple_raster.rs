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

use freetype::face::LoadFlag;
use freetype::Library;

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

/// Draws `text` in opaque white, horizontally centered on `center_x` with its
/// baseline at `baseline_y`, over premultiplied RGBA. Returns `false` when the
/// family's font file is not available.
pub(crate) fn draw_centered_white_text(
    destination: &mut [u8],
    width: u32,
    height: u32,
    family: &str,
    text: &str,
    font_size: f32,
    center_x: f32,
    baseline_y: f32,
) -> Result<bool, String> {
    let Some(bytes) = crate::sdf::outline::load_font_bytes_for_family(family) else {
        return Ok(false);
    };
    let library = Library::init().map_err(|error| format!("初始化 FreeType 失败: {error:?}"))?;
    let face = library
        .new_memory_face2(bytes.as_slice(), 0)
        .map_err(|error| format!("加载字体 {family} 失败: {error:?}"))?;
    face.set_char_size((font_size * 64.0).round() as isize, 0, 72, 72)
        .map_err(|error| format!("设置字号失败: {error:?}"))?;

    // Hinted advances feed both the measurement and the pen, so centering and
    // glyph placement stay on one metric.
    let mut text_width = 0.0f32;
    for ch in text.chars() {
        face.load_char(ch as usize, LoadFlag::TARGET_NORMAL)
            .map_err(|error| format!("加载字形 {ch:?} 失败: {error:?}"))?;
        text_width += face.glyph().raw().advance.x as f32 / 64.0;
    }

    let width = width as i32;
    let height = height as i32;
    let mut pen_x = center_x - text_width / 2.0;
    for ch in text.chars() {
        face.load_char(ch as usize, LoadFlag::TARGET_NORMAL | LoadFlag::RENDER)
            .map_err(|error| format!("渲染字形 {ch:?} 失败: {error:?}"))?;
        let glyph = face.glyph();
        let bitmap = glyph.bitmap();
        // No subpixel positioning: each glyph blits at its rounded pen point.
        let left = (pen_x + 0.5).floor() as i32 + glyph.bitmap_left();
        let top = (baseline_y + 0.5).floor() as i32 - glyph.bitmap_top();
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
        pen_x += glyph.advance().x as f32 / 64.0;
    }
    Ok(true)
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
    fn the_blend_is_exact_at_the_coverage_endpoints() {
        for dst in 0..=255u32 {
            assert_eq!(dst + (((255 - dst) * 256) >> 8), 255, "full coverage");
            assert_eq!(dst + (((255 - dst) * 1) >> 8), dst, "zero coverage stays");
        }
    }
}
