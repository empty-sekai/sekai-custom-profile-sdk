// Auto-split from generals/mod.rs

use super::*;

pub(super) fn draw_comment(canvas: &Canvas, profile: &ProfileData, md: &MasterData) {
    use layout::COMMENT;

    let els = &COMMENT.elements;
    let txtbox = &els[0]; // 文本框
    let title = &els[1]; // "个性签名" 标题

    // 文本框
    draw_textbox(canvas, txtbox.cx, txtbox.cy, txtbox.w, txtbox.h);

    // "个性签名" 标题（文本元素，h=字号）
    draw_general_text(
        canvas,
        "个性签名",
        title,
        1,
        md,
        Color4f::new(0.53, 0.53, 0.53, 1.0),
        Align::Center,
        title.h,
    );

    // 签名内容（文本框内带 Padding，fontId=3）
    let pad_top = 18.0_f32;
    let pad_left = 16.0_f32;
    let line_h = 32.0_f32;
    let max_w = txtbox.w - pad_left * 2.0;
    let base_x = txtbox.cx - txtbox.w / 2.0 + pad_left;
    let mut cur_y = -txtbox.cy - txtbox.h / 2.0 + pad_top + 26.0;

    let font_mgr = FontMgr::default();
    let resolved_name = md.resolve_font(3);
    let typeface = resolved_name
        .and_then(|name| font_mgr.match_family_style(name, FontStyle::normal()))
        .or_else(|| font_mgr.legacy_make_typeface(None, FontStyle::normal()));
    let typeface = match typeface {
        Some(tf) => tf,
        None => {
            tracing::warn!("draw_comment: 无法创建 Typeface, 跳过绘制");
            return;
        }
    };

    let font = Font::new(typeface, Some(26.0));
    let mut paint = Paint::default();
    paint.set_color4f(Color4f::new(0.2, 0.2, 0.2, 1.0), None);
    paint.set_anti_alias(true);

    // 按 \n 切分后逐行自动折行
    let bottom_limit = -txtbox.cy + txtbox.h / 2.0;
    for line in profile.word.split('\n') {
        if cur_y > bottom_limit {
            break;
        }
        // 按字符逐个测量做自动折行
        let chars: Vec<char> = line.chars().collect();
        let mut start = 0;
        while start < chars.len() && cur_y <= bottom_limit {
            let mut end = start;
            let mut last_fit = start;
            while end < chars.len() {
                let sub: String = chars[start..=end].iter().collect();
                let tw = font.measure_str(&sub, Some(&paint)).1.width();
                if tw > max_w && end > start {
                    break;
                }
                last_fit = end;
                end += 1;
            }
            let seg: String = chars[start..=last_fit].iter().collect();
            canvas.draw_str(&seg, Point::new(base_x, cur_y), &font, &paint);
            cur_y += line_h;
            start = last_fit + 1;
        }
    }
}
