// Auto-split from generals/mod.rs

use super::*;

pub(super) fn draw_honors_panel(
    canvas: &Canvas,
    profile: &ProfileData,
    md: &MasterData,
    assets: Option<&AssetStore>,
) {
    use layout::HONORS;
    let els = &HONORS.elements;

    // 称号栏边框 [0] — 788×179 圆角矩形描边
    let border = &els[0];
    let border_rect = Rect::from_xywh(
        border.cx - border.w / 2.0,
        -border.cy - border.h / 2.0,
        border.w,
        border.h,
    );
    // 半透明底色（与个性签名 textbox 一致）
    let mut fill = Paint::default();
    fill.set_style(PaintStyle::Fill);
    fill.set_color4f(Color4f::new(0.53, 0.53, 0.53, 0.25), None);
    fill.set_anti_alias(true);
    canvas.draw_round_rect(border_rect, 12.0, 12.0, &fill);
    // 不绘制边框描边线

    let mut sorted_slots: Vec<&HonorSlot> = profile.honor_slots.iter().collect();
    sorted_slots.sort_by(|a, b| b.full_size.cmp(&a.full_size));
    if sorted_slots.len() >= 2 {
        sorted_slots.swap(0, 1);
    }
    for (i, slot) in sorted_slots.iter().enumerate() {
        if i >= 3 {
            break;
        }
        let el = &els[i + 1];
        let render_full = i == 0;

        canvas.save();
        canvas.translate(Point::new(el.cx, -el.cy));

        if let Some(store) = assets {
            if slot.profile_honor_type == "bonds" {
                let word_id = slot.bonds_honor_word_id.unwrap_or(0);
                let inverse = slot.bonds_honor_view_type.as_deref() == Some("reverse");
                crate::elements::honor::render_bonds_honor(
                    canvas,
                    slot.honor_id,
                    slot.honor_level,
                    render_full,
                    word_id,
                    inverse,
                    false,
                    md,
                    store,
                );
            } else {
                crate::elements::honor::render_honor(
                    canvas,
                    slot.honor_id,
                    slot.honor_level,
                    render_full,
                    md,
                    store,
                    Some(profile),
                );
            }
        }

        canvas.restore();
    }
}
