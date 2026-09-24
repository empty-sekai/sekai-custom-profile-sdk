//! FreeType 轮廓到共享分段表示的适配。

use std::slice;

use freetype::ffi;
use sekai_profile_renderer_core::sdf_geometry::{Segment, Vec2};
use sekai_profile_renderer_core::sdf_glyph::outline_contours;

/// # Safety
///
/// `outline` 必须是 FreeType 刚加载的有效轮廓（点、tag、轮廓端点数组与计数一致）。
pub(super) unsafe fn extract_segments(outline: &ffi::FT_Outline) -> Vec<Vec<Segment>> {
    let contour_ends = slice::from_raw_parts(outline.contours, outline.n_contours as usize);
    let points = slice::from_raw_parts(outline.points, outline.n_points as usize);
    let tags = slice::from_raw_parts(outline.tags, outline.n_points as usize);
    let points = points
        .iter()
        .map(|point| Vec2::new(point.x as f32, point.y as f32))
        .collect::<Vec<_>>();
    let tags = tags.iter().map(|&tag| tag as u8).collect::<Vec<_>>();
    let contour_ends = contour_ends
        .iter()
        .map(|&end| end as usize)
        .collect::<Vec<_>>();
    outline_contours(&points, &tags, &contour_ends)
}
