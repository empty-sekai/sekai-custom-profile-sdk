// Auto-split from generals/mod.rs

use super::*;
use crate::elements::image::{draw_card_member_cropped, CardBadgeData};

/// 属性色映射（用于无素材时绘制占位卡片）。
fn attr_color4f(attr: &str, alpha: f32) -> Color4f {
    let c = match attr {
        "cool" => (97, 148, 199),
        "cute" => (235, 133, 141),
        "happy" => (242, 184, 102),
        "mysterious" => (148, 112, 204),
        _ => (138, 189, 143), // pure
    };
    Color4f::new(
        c.0 as f32 / 255.0,
        c.1 as f32 / 255.0,
        c.2 as f32 / 255.0,
        alpha,
    )
}

pub(super) fn draw_deck(
    canvas: &Canvas,
    profile: &ProfileData,
    md: &MasterData,
    assets: Option<&AssetStore>,
) {
    use layout::DECK;
    let container = &DECK.elements[0];
    let card_count = 5;
    let card_w = container.w / card_count as f32;
    let card_h = container.h;
    // 栏左上角坐标（游戏坐标 → Skia 坐标）
    let left = container.cx - container.w / 2.0;
    let top = -container.cy - container.h / 2.0;

    for i in 0..card_count {
        let x = left + i as f32 * card_w;
        let member = profile.deck_members.get(i);

        // 尝试使用真实卡面素材，`draw_card_member_cropped` 返回 true 表示素材已绘制
        let drawn = if let Some(m) = member {
            let suffix = if m.after_training {
                "after_training"
            } else {
                "normal"
            };
            match crate::asset_keys::resolve_card_member_key(m.card_id, 1, suffix, md) {
                Some(key) => {
                    let badge = md.get_card(m.card_id).map(|card| CardBadgeData {
                        rarity: card.card_rarity_type,
                        attr: card.attr,
                        master_rank: m.master_rank,
                        trained: m.after_training,
                        level: m.level,
                    });
                    let scale = (card_w / 312.0).min(card_h / 512.0);
                    let clip_rect = Rect::from_xywh(x, top, card_w, card_h);
                    canvas.save();
                    canvas.clip_rect(clip_rect, None, None);
                    canvas.translate((x + card_w / 2.0, top + card_h / 2.0));
                    canvas.scale((scale, scale));
                    let ok = draw_card_member_cropped(canvas, assets, &key, m.card_id, badge);
                    canvas.restore();
                    ok
                }
                None => false,
            }
        } else {
            false
        };

        if drawn {
            continue;
        }

        // 无素材时绘制属性色占位卡片
        let color = member
            .and_then(|m| md.get_card(m.card_id))
            .map(|card| attr_color4f(&card.attr, 0.7))
            .unwrap_or(Color4f::new(0.45, 0.45, 0.55, 0.6));

        let card_rect = Rect::from_xywh(x, top, card_w, card_h);

        let mut bg = Paint::default();
        bg.set_style(PaintStyle::Fill);
        bg.set_color4f(color, None);
        bg.set_anti_alias(true);
        canvas.draw_round_rect(card_rect, 6.0, 6.0, &bg);

        let mut border = Paint::default();
        border.set_style(PaintStyle::Stroke);
        border.set_stroke_width(1.0);
        border.set_color4f(Color4f::new(1.0, 1.0, 1.0, 0.25), None);
        border.set_anti_alias(true);
        canvas.draw_round_rect(card_rect, 6.0, 6.0, &border);

        // 绘制卡牌编号与等级
        if let Some(m) = member {
            let font_mgr = FontMgr::default();
            if let Some(tf) = font_mgr.legacy_make_typeface(None, FontStyle::default()) {
                let info = format!("#{}", m.card_id);
                let lv = format!("Lv.{}", m.level);
                let font_size = (card_h * 0.12).max(10.0);
                let font = Font::new(tf as Typeface, Some(font_size));
                let mut tp = Paint::default();
                tp.set_color4f(Color4f::new(1.0, 1.0, 1.0, 0.9), None);
                tp.set_anti_alias(true);

                let text_x = x + 6.0;
                let text_y = top + card_h - 10.0;
                canvas.draw_str(
                    &info,
                    Point::new(text_x, text_y - font_size - 2.0),
                    &font,
                    &tp,
                );
                canvas.draw_str(&lv, Point::new(text_x, text_y), &font, &tp);
            }
        }
    }
}
