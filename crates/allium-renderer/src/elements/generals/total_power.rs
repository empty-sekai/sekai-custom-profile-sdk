// Auto-split from generals/mod.rs

use super::*;

pub(super) fn draw_total_power(canvas: &Canvas, profile: &ProfileData, md: &MasterData) {
    use layout::TOTAL_POWER;

    let els = &TOTAL_POWER.elements;

    // "综合力" 文本（文本元素，h=字号）— 取 customProfilePlayerInfoResources[id=2].name
    let title = md
        .resolve_player_info_label(2)
        .unwrap_or_else(|| "综合力".to_string());
    draw_general_text(
        canvas,
        &title,
        &els[0],
        1,
        md,
        Color4f::new(0.33, 0.33, 0.33, 1.0),
        Align::Left,
        els[0].h,
    );

    // "|" 竖线
    draw_general_text(
        canvas,
        "|",
        &els[2],
        1,
        md,
        Color4f::new(0.67, 0.67, 0.67, 1.0),
        Align::Left,
        els[2].h,
    );

    // 综合力数字（文本元素，h=字号）
    let text = format!("{}", profile.total_power);
    draw_general_text(
        canvas,
        &text,
        &els[3],
        1,
        md,
        Color4f::new(0.2, 0.2, 0.2, 1.0),
        Align::Left,
        els[3].h,
    );
}
