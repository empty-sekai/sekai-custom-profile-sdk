// Auto-split from generals/mod.rs

use super::*;

pub(super) fn draw_story_favorite(
    canvas: &Canvas,
    profile: &ProfileData,
    md: &MasterData,
    assets: Option<&AssetStore>,
) {
    use layout::STORY_FAVORITE;
    let els = &STORY_FAVORITE.elements;

    // 标题文本 [0]（使用游戏字体 fontId=1）
    draw_general_text(
        canvas,
        "最喜欢的剧情",
        &els[0],
        1,
        md,
        Color4f::new(0.33, 0.33, 0.33, 1.0),
        Align::Center,
        els[0].h,
    );

    // 横线 [1]（统一 2px 细条）
    draw_horizontal_line(canvas, els[1].cx, els[1].cy, els[1].w);

    // 图片槽位 — 直接使用 layout.rs 中每格的精确定位 [2..N]
    let img_slots = &els[2..]; // [2]~[5] 各有独立 cx/cy/w/h

    for (i, sf) in profile.story_favorites.iter().enumerate() {
        if i >= img_slots.len() {
            break; // 超出已定义的槽位数
        }
        let slot = &img_slots[i];
        // Skia 坐标
        let x = slot.cx - slot.w / 2.0;
        let y = -slot.cy - slot.h / 2.0;

        let drawn = if let Some(store) = assets {
            // 查 MasterData 获取 banner 素材 key
            let key = md.resolve_story_banner(&sf.story_type, sf.story_id);
            if let Some(img) = key.as_deref().and_then(|k| store.get_image(k)) {
                let paint = Paint::default();
                canvas.draw_image_rect(img, None, Rect::from_xywh(x, y, slot.w, slot.h), &paint);
                true
            } else {
                false
            }
        } else {
            false
        };

        if !drawn {
            // 灰色圆角占位
            let mut p = Paint::default();
            p.set_style(PaintStyle::Fill);
            p.set_color4f(Color4f::new(0.82, 0.82, 0.82, 0.4), None);
            p.set_anti_alias(true);
            canvas.draw_round_rect(Rect::from_xywh(x, y, slot.w, slot.h), 8.0, 8.0, &p);
        }
    }
}
