//! 卡面合成公共工具。
//!
//! 稀有度、星标与裁切规则由 `sekai_profile_renderer_core::general_recipe`
//! 定义，这里只做转发。

use sekai_profile_renderer_core::general_recipe;

/// object-fit: cover 的源矩形计算，返回 `(x, y, w, h)`。
#[cfg_attr(not(feature = "skia-oracle"), allow(dead_code))]
pub fn cover_crop_rect(src_w: f32, src_h: f32, dst_w: f32, dst_h: f32) -> (f32, f32, f32, f32) {
    let rect = general_recipe::cover_source_rect(src_w, src_h, dst_w, dst_h);
    (rect.x, rect.y, rect.width, rect.height)
}

/// 稀有度后缀映射。
#[cfg_attr(not(feature = "skia-oracle"), allow(dead_code))]
pub fn rarity_suffix(rarity: &str) -> &str {
    general_recipe::card_rarity_suffix(rarity)
}

/// 稀有度对应的星级数量。
#[cfg_attr(not(feature = "skia-oracle"), allow(dead_code))]
pub fn rarity_count(rarity: &str) -> usize {
    general_recipe::card_rarity_star_count(rarity)
}

/// 星图 key 映射。
#[cfg_attr(not(feature = "skia-oracle"), allow(dead_code))]
pub fn star_icon_key(rarity: &str, trained: bool) -> &'static str {
    general_recipe::card_rarity_star_key(rarity, trained)
}

#[cfg(feature = "skia-oracle")]
fn draw_repeated_image(
    canvas: &skia_safe::Canvas,
    image: &skia_safe::Image,
    positions: &[(f32, f32)],
    size: (f32, f32),
) {
    for (x, y) in positions {
        let dst = skia_safe::Rect::from_xywh(*x, *y, size.0, size.1);
        canvas.draw_image_rect(image, None, dst, &skia_safe::Paint::default());
    }
}

/// 横排星级绘制。
#[cfg(feature = "skia-oracle")]
pub fn draw_stars_horizontal(
    canvas: &skia_safe::Canvas,
    star_img: &skia_safe::Image,
    count: usize,
    start_xy: (f32, f32),
    star_size: (f32, f32),
) {
    let positions: Vec<(f32, f32)> = (0..count)
        .map(|index| (start_xy.0 + index as f32 * star_size.0, start_xy.1))
        .collect();
    draw_repeated_image(canvas, star_img, &positions, star_size);
}

/// 竖排星级绘制（从底部填充）。
#[cfg(feature = "skia-oracle")]
pub fn draw_stars_vertical(
    canvas: &skia_safe::Canvas,
    star_img: &skia_safe::Image,
    count: usize,
    start_xy: (f32, f32),
    star_size: (f32, f32),
    step_y: f32,
    total_slots: usize,
) {
    let start_y = start_xy.1 + (total_slots.saturating_sub(count) as f32) * step_y;
    let positions: Vec<(f32, f32)> = (0..count)
        .map(|index| (start_xy.0, start_y + index as f32 * step_y))
        .collect();
    draw_repeated_image(canvas, star_img, &positions, star_size);
}
