// Auto-split from generals/mod.rs

use super::*;

pub(super) fn draw_player_name(canvas: &Canvas, profile: &ProfileData, md: &MasterData) {
    use layout::PLAYER_NAME;

    // 文本框（浅灰色圆角矩形）
    let tb = &PLAYER_NAME.elements[0]; // 名称文本框
    draw_textbox(canvas, tb.cx, tb.cy, tb.w, tb.h);

    // 玩家名称文本（文本元素，h=字号）
    let name_el = &PLAYER_NAME.elements[1];
    draw_general_text(
        canvas,
        &profile.user_name,
        name_el,
        1,
        md,
        Color4f::new(0.2, 0.2, 0.2, 1.0),
        Align::Left,
        name_el.h,
    );
}
