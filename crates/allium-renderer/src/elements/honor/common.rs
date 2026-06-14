//! 称号渲染公共辅助函数。

use skia_safe::{
    Canvas, Color4f, Font, FontMgr, FontStyle, Paint, PaintStyle, Point, Rect, Typeface,
};

pub(super) fn draw_placeholder(canvas: &Canvas, label: &str, id: i32, w: f32, h: f32) {
    let mut bg = Paint::default();
    bg.set_style(PaintStyle::Fill);
    bg.set_color4f(Color4f::new(0.85, 0.85, 0.85, 0.6), None);
    bg.set_anti_alias(true);
    let rect = Rect::from_xywh(-w / 2.0, -h / 2.0, w, h);
    canvas.draw_round_rect(rect, 6.0, 6.0, &bg);

    let font_mgr = FontMgr::default();
    if let Some(tf) = font_mgr.legacy_make_typeface(None, FontStyle::default()) {
        let font = Font::new(tf as Typeface, Some(10.0));
        let mut tp = Paint::default();
        tp.set_color4f(Color4f::new(0.3, 0.3, 0.3, 1.0), None);
        let text = format!("{}#{}", label, id);
        canvas.draw_str(&text, Point::new(-w / 4.0, 4.0), &font, &tp);
    }
}
