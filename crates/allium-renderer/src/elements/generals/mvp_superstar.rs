// Auto-split from generals/mod.rs

use super::*;

pub(super) fn draw_mvp_superstar(canvas: &Canvas, profile: &ProfileData, md: &MasterData) {
    use layout::MVP_SUPERSTAR;
    let els = &MVP_SUPERSTAR.elements;

    // 横线 [5]
    draw_horizontal_line(canvas, els[5].cx, els[5].cy, els[5].w);

    // "多人演出" [0]（文本元素，h=字号）
    draw_general_text(
        canvas,
        "多人演出",
        &els[0],
        1,
        md,
        Color4f::new(0.33, 0.33, 0.33, 1.0),
        Align::Center,
        els[0].h,
    );

    // MVP 图标 [1] (带白字)
    draw_gray_icon_bg(canvas, els[1].cx, els[1].cy, els[1].w, els[1].h, 16.0);
    draw_general_text(
        canvas,
        "MVP",
        &els[1],
        1,
        md,
        Color4f::new(1.0, 1.0, 1.0, 1.0),
        Align::Center,
        els[1].h * 0.55, // 容器内白字，字号适配框高
    );

    // MVP 次数 [2]（与 SUPERSTAR 统一字号，用 els[4].h = 31）
    draw_general_text(
        canvas,
        &format!("{}次", profile.mvp),
        &els[2],
        1,
        md,
        Color4f::new(0.27, 0.27, 0.27, 1.0),
        Align::Left,
        els[4].h, // 统一使用 SUPERSTAR 次数的元素高度作为字号
    );

    // SUPERSTAR 图标 [3] (两行白字)
    draw_gray_icon_bg(canvas, els[3].cx, els[3].cy, els[3].w, els[3].h, 16.0);
    let mut el_up = els[3].clone();
    el_up.cy += 10.0;
    el_up.h = 18.0;
    draw_general_text(
        canvas,
        "SUPER",
        &el_up,
        1,
        md,
        Color4f::new(1.0, 1.0, 1.0, 1.0),
        Align::Center,
        el_up.h, // 已手动设为 18.0
    );
    let mut el_down = els[3].clone();
    el_down.cy -= 10.0;
    el_down.h = 18.0;
    draw_general_text(
        canvas,
        "STAR",
        &el_down,
        1,
        md,
        Color4f::new(1.0, 1.0, 1.0, 1.0),
        Align::Center,
        el_down.h, // 已手动设为 18.0
    );

    // SUPERSTAR 次数 [4]（文本元素，h=字号）
    draw_general_text(
        canvas,
        &format!("{}次", profile.superstar),
        &els[4],
        1,
        md,
        Color4f::new(0.27, 0.27, 0.27, 1.0),
        Align::Left,
        els[4].h,
    );
}
