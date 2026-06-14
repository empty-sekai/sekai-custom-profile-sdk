//! 卡面合成公共工具。

#[cfg(feature = "skia-core")]
use crate::assets::AssetStore;

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

/// 信息底栏绘制参数。
#[cfg(feature = "skia-core")]
pub(crate) struct InfoBarSpec {
    pub height: f32,
    pub font_size: f32,
    pub text_rect_size: (f32, f32),
    pub text_rect_pos: (f32, f32),
    pub y_offset: f32,
    pub tint: (u8, u8, u8, u8),
}

#[cfg(feature = "skia-core")]
fn draw_level_text(
    canvas: &skia_safe::Canvas,
    text: &str,
    bar_rect: skia_safe::Rect,
    font_size: f32,
    text_rect_size: (f32, f32),
    text_rect_pos: (f32, f32),
) {
    let font_mgr = skia_safe::FontMgr::default();
    let typeface = font_mgr
        .match_family_style(
            crate::widgets::theme::fonts::EMPHASIS,
            skia_safe::FontStyle::normal(),
        )
        .or_else(|| {
            font_mgr.match_family_style(
                crate::widgets::theme::fonts::PRIMARY,
                skia_safe::FontStyle::normal(),
            )
        })
        .or_else(|| font_mgr.legacy_make_typeface(None, skia_safe::FontStyle::normal()));
    let Some(typeface) = typeface else {
        return;
    };

    let font = skia_safe::Font::new(typeface, Some(font_size));
    let mut paint = skia_safe::Paint::default();
    paint.set_anti_alias(true);
    paint.set_color4f(skia_safe::Color4f::new(1.0, 1.0, 1.0, 1.0), None);

    let bbox = font.measure_str(text, Some(&paint)).1;
    let text_h = bbox.height();
    let rect_left = text_rect_pos.0 - text_rect_size.0 / 2.0;
    let rect_top = bar_rect.height() + text_rect_pos.1 - text_rect_size.1 / 2.0;
    let x = bar_rect.left + rect_left - bbox.left;
    let y = bar_rect.top + rect_top + (text_rect_size.1 - text_h) / 2.0 - bbox.top;
    canvas.draw_str(text, (x, y), &font, &paint);
}

/// 绘制带纹理和等级文字的信息底栏。
#[cfg(feature = "skia-core")]
pub(crate) fn draw_info_bar(
    canvas: &skia_safe::Canvas,
    assets: Option<&AssetStore>,
    card_rect: skia_safe::Rect,
    text: &str,
    spec: &InfoBarSpec,
) {
    let bar_rect = skia_safe::Rect::from_xywh(
        card_rect.left,
        card_rect.bottom - spec.height + spec.y_offset,
        card_rect.width(),
        spec.height,
    );
    let layer = skia_safe::canvas::SaveLayerRec::default().bounds(&bar_rect);
    canvas.save_layer(&layer);

    if let Some(texture) = assets.and_then(|store| store.get_image("card/bg_base_wh")) {
        canvas.draw_image_rect(texture, None, bar_rect, &skia_safe::Paint::default());
        let mut tint = skia_safe::Paint::default();
        tint.set_anti_alias(true);
        tint.set_blend_mode(skia_safe::BlendMode::SrcIn);
        tint.set_color4f(
            skia_safe::Color4f::new(
                spec.tint.0 as f32 / 255.0,
                spec.tint.1 as f32 / 255.0,
                spec.tint.2 as f32 / 255.0,
                spec.tint.3 as f32 / 255.0,
            ),
            None,
        );
        canvas.draw_rect(bar_rect, &tint);
    } else {
        let mut fill = skia_safe::Paint::default();
        fill.set_anti_alias(true);
        fill.set_style(skia_safe::PaintStyle::Fill);
        fill.set_color4f(
            skia_safe::Color4f::new(
                spec.tint.0 as f32 / 255.0,
                spec.tint.1 as f32 / 255.0,
                spec.tint.2 as f32 / 255.0,
                spec.tint.3 as f32 / 255.0,
            ),
            None,
        );
        canvas.draw_rect(bar_rect, &fill);
    }

    canvas.restore();
    draw_level_text(
        canvas,
        text,
        bar_rect,
        spec.font_size,
        spec.text_rect_size,
        spec.text_rect_pos,
    );
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
