// Auto-split from generals/mod.rs

use super::*;
use crate::elements::image::cover_crop_source_rect;
use crate::widgets::card_util::{draw_stars_vertical, rarity_count, rarity_suffix, star_icon_key};

pub(super) fn draw_leader_member(
    canvas: &Canvas,
    profile: &ProfileData,
    md: &MasterData,
    assets: Option<&AssetStore>,
) {
    use layout::LEADER_MEMBER;
    let cover = &LEADER_MEMBER.elements[0];
    let cover_rect = Rect::from_xywh(
        cover.cx - cover.w / 2.0,
        -cover.cy - cover.h / 2.0,
        cover.w,
        cover.h,
    );

    let mut card_drawn = false;
    if let (Some(lc), Some(store)) = (&profile.leader_card, assets) {
        let suffix = if lc.after_training {
            "after_training"
        } else {
            "normal"
        };
        if let Some(key) = crate::asset_keys::resolve_card_member_key(lc.card_id, 2, suffix, md) {
            if let Some(img) = store.get_image(&key) {
                let src = cover_crop_source_rect(
                    img.width() as f32,
                    img.height() as f32,
                    cover_rect.width(),
                    cover_rect.height(),
                );
                canvas.draw_image_rect(
                    img,
                    Some((&src, skia_safe::canvas::SrcRectConstraint::Fast)),
                    cover_rect,
                    &Paint::default(),
                );
                card_drawn = true;

                if let Some(card) = md.get_card(lc.card_id) {
                    let frame_key =
                        format!("card/cardFrame_L_{}", rarity_suffix(&card.card_rarity_type));
                    if let Some(frame) = store.get_image(&frame_key) {
                        canvas.draw_image_rect(frame, None, cover_rect, &Paint::default());
                    }

                    let attr_key = format!("card/icon_attribute_{}_88", card.attr);
                    if let Some(attr) = store.get_image(&attr_key) {
                        canvas.draw_image_rect(
                            attr,
                            None,
                            Rect::from_xywh(
                                cover_rect.right - 88.0 - 40.0,
                                cover_rect.top,
                                88.0,
                                92.0,
                            ),
                            &Paint::default(),
                        );
                    }

                    if let Some(star) =
                        store.get_image(star_icon_key(&card.card_rarity_type, lc.after_training))
                    {
                        draw_stars_vertical(
                            canvas,
                            &star,
                            rarity_count(&card.card_rarity_type),
                            (cover_rect.left + 24.0, cover_rect.bottom - 208.0 - 17.0),
                            (56.0, 56.0),
                            48.0,
                            4,
                        );
                    }

                    let rank_key = format!("card/masterRank_L_{}", lc.master_rank.clamp(0, 5));
                    if let Some(rank) = store.get_image(&rank_key) {
                        canvas.draw_image_rect(
                            rank,
                            None,
                            Rect::from_xywh(
                                cover_rect.right - 104.0 - 24.0,
                                cover_rect.bottom - 104.0 - 24.0,
                                104.0,
                                104.0,
                            ),
                            &Paint::default(),
                        );
                    }
                }
            }
        }
    }

    if !card_drawn {
        let mut paint = Paint::default();
        paint.set_style(PaintStyle::Fill);
        paint.set_color4f(Color4f::new(0.4, 0.4, 0.4, 1.0), None);
        paint.set_anti_alias(true);
        canvas.draw_rect(cover_rect, &paint);
    }
}
