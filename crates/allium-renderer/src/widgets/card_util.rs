//! 卡面合成公共工具。

/// object-fit: cover 的源矩形计算。
#[cfg_attr(not(feature = "skia-core"), allow(dead_code))]
pub fn cover_crop_rect(src_w: f32, src_h: f32, dst_w: f32, dst_h: f32) -> (f32, f32, f32, f32) {
    let img_ratio = src_w / src_h;
    let dst_ratio = dst_w / dst_h;
    if img_ratio > dst_ratio {
        let crop_w = src_h * dst_ratio;
        ((src_w - crop_w) / 2.0, 0.0, crop_w, src_h)
    } else {
        let crop_h = src_w / dst_ratio;
        (0.0, (src_h - crop_h) / 2.0, src_w, crop_h)
    }
}

/// 稀有度后缀映射。
#[cfg_attr(not(feature = "skia-core"), allow(dead_code))]
pub fn rarity_suffix(rarity: &str) -> &str {
    if rarity == "rarity_birthday" {
        "bd"
    } else {
        rarity.rsplit('_').next().unwrap_or("1")
    }
}

/// 稀有度对应的星级数量。
#[cfg_attr(not(feature = "skia-core"), allow(dead_code))]
pub fn rarity_count(rarity: &str) -> usize {
    if rarity == "rarity_birthday" {
        1
    } else {
        rarity
            .rsplit('_')
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1)
    }
}

/// 星图 key 映射。
#[cfg_attr(not(feature = "skia-core"), allow(dead_code))]
pub fn star_icon_key(rarity: &str, trained: bool) -> &'static str {
    if rarity == "rarity_birthday" {
        "card/rarity_birthday"
    } else if trained {
        "card/rarity_star_afterTraining"
    } else {
        "card/rarity_star_normal"
    }
}

#[cfg(feature = "skia-core")]
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
#[cfg(feature = "skia-core")]
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
#[cfg(feature = "skia-core")]
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
