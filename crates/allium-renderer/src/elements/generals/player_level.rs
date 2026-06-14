// Auto-split from generals/mod.rs

use super::*;

pub(super) fn draw_player_level(
    canvas: &Canvas,
    profile: &ProfileData,
    _md: &MasterData,
    assets: Option<&AssetStore>,
) {
    // 药丸尺寸（更短）
    let pill_w = 220.0;
    let pill_h = 52.0;
    let corner_radius = pill_h / 2.0;

    // 深灰色半透明底
    let mut bg = Paint::default();
    bg.set_style(PaintStyle::Fill);
    bg.set_color4f(Color4f::new(0.15, 0.15, 0.20, 0.85), None);
    bg.set_anti_alias(true);
    let pill_rect = Rect::from_xywh(-pill_w / 2.0, -pill_h / 2.0, pill_w, pill_h);
    canvas.draw_round_rect(pill_rect, corner_radius, corner_radius, &bg);

    // 白色半透明细边框
    let mut border = Paint::default();
    border.set_style(PaintStyle::Stroke);
    border.set_stroke_width(1.0);
    border.set_color4f(Color4f::new(1.0, 1.0, 1.0, 0.15), None);
    border.set_anti_alias(true);
    canvas.draw_round_rect(pill_rect, corner_radius, corner_radius, &border);

    // 左侧：图标 + "等级"
    let icon_size = 32.0;
    let left_margin = 16.0;
    let icon_x = -pill_w / 2.0 + left_margin;
    let icon_y = -icon_size / 2.0;
    let icon_rect = Rect::from_xywh(icon_x, icon_y, icon_size, icon_size);

    // 绘制图标
    if let Some(store) = assets {
        if let Some(img) = store.get_image("sprite/icon/icon_playerRank") {
            let src = Rect::from_xywh(0.0, 0.0, img.width() as f32, img.height() as f32);
            canvas.draw_image_rect(
                img,
                Some((&src, skia_safe::canvas::SrcRectConstraint::Fast)),
                icon_rect,
                &Paint::default(),
            );
        }
    }

    // 绘制 "等级" 文本（与数字同字号）
    let label = "等级";
    let label_font_size = 26.0;
    let label_x = icon_x + icon_size + 8.0;

    let font_mgr = FontMgr::default();
    let typeface = font_mgr
        .match_family_style("Noto Sans CJK SC", FontStyle::normal())
        .or_else(|| font_mgr.legacy_make_typeface(None, FontStyle::normal()));

    if let Some(tf) = typeface.clone() {
        let font = Font::new(tf, Some(label_font_size));
        let mut paint = Paint::default();
        paint.set_color4f(Color4f::new(0.70, 0.70, 0.75, 1.0), None);
        paint.set_anti_alias(true);
        canvas.draw_str(
            label,
            Point::new(label_x, label_font_size * 0.35),
            &font,
            &paint,
        );
    }

    // 右侧：等级数字（大号，靠右对齐，白色）
    let rank_text = format!("{}", profile.user_rank);
    let rank_font_size = 26.0;
    let right_margin = 20.0;

    if let Some(tf) = typeface {
        let font = Font::new(tf, Some(rank_font_size));
        let mut paint = Paint::default();
        paint.set_color4f(Color4f::new(1.0, 1.0, 1.0, 1.0), None);
        paint.set_anti_alias(true);

        let text_width = font.measure_str(&rank_text, Some(&paint)).1.width();
        let rank_x = pill_w / 2.0 - right_margin - text_width;
        canvas.draw_str(
            &rank_text,
            Point::new(rank_x, rank_font_size * 0.35),
            &font,
            &paint,
        );
    }
}
