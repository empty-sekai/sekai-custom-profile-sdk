//! CardMember 元素 adapter。

use crate::asset_keys::resolve_card_member_key;
use crate::context::RenderContext;
use crate::types::CardMemberElement;
use crate::widgets::card_util::{rarity_count, rarity_suffix, star_icon_key};
use crate::widgets::Widget;

#[cfg(feature = "skia-core")]
use crate::elements::image::{draw_card_member_cropped, draw_card_member_small, CardBadgeData};

#[derive(Clone)]
struct CardMemberBadgeData {
    rarity: String,
    attr: String,
    master_rank: i32,
    trained: bool,
    level: i32,
}

/// CardMember 元素 Widget adapter。
pub struct CardMemberWidget {
    id: i32,
    member_type: i32,
    asset_key: String,
    badge: Option<CardMemberBadgeData>,
}

impl CardMemberWidget {
    /// 从 CardMember 元素构建 adapter。
    pub fn from_element(elem: &CardMemberElement, ctx: &RenderContext<'_>) -> Option<Self> {
        let md = ctx.masterdata?;
        let suffix = if elem.use_after_special_training.unwrap_or(false) {
            "after_training"
        } else {
            "normal"
        };
        let member_type = elem.member_type.unwrap_or(2);
        let asset_key = resolve_card_member_key(elem.id, member_type, suffix, md)?;
        let badge = if elem.show_master_rank.unwrap_or(false) {
            md.get_card(elem.id).map(|card| {
                let user_card = ctx.profile.and_then(|profile| profile.user_card(elem.id));
                CardMemberBadgeData {
                    rarity: card.card_rarity_type,
                    attr: card.attr,
                    master_rank: user_card.map(|info| info.master_rank).unwrap_or(0),
                    trained: elem.use_after_special_training.unwrap_or_else(|| {
                        ctx.profile
                            .and_then(|profile| profile.user_card(elem.id))
                            .map(|info| info.after_training)
                            .unwrap_or(false)
                    }),
                    level: user_card.map(|info| info.level).unwrap_or(60),
                }
            })
        } else {
            None
        };

        Some(Self {
            id: elem.id,
            member_type,
            asset_key,
            badge,
        })
    }
}

impl Widget for CardMemberWidget {
    fn name(&self) -> &'static str {
        let _ = self.id;
        "card_member"
    }

    fn measure(&self, _ctx: &RenderContext<'_>) -> (f32, f32) {
        if self.member_type == 1 {
            return (312.0, 512.0);
        }
        #[cfg(feature = "skia-core")]
        {
            if let Some(image) = _ctx.assets.get_image(&self.asset_key) {
                return (image.width() as f32, image.height() as f32);
            }
        }
        (156.0, 156.0)
    }

    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, ctx: &RenderContext<'_>) {
        canvas.save();
        canvas.translate((x, y));
        if self.member_type == 1 {
            draw_card_member_cropped(
                canvas,
                Some(ctx.assets),
                &self.asset_key,
                self.id,
                self.badge.clone().map(|badge| CardBadgeData {
                    rarity: badge.rarity,
                    attr: badge.attr,
                    master_rank: badge.master_rank,
                    trained: badge.trained,
                    level: badge.level,
                }),
            );
        } else {
            draw_card_member_small(
                canvas,
                Some(ctx.assets),
                &self.asset_key,
                self.id,
                self.badge.clone().map(|badge| CardBadgeData {
                    rarity: badge.rarity,
                    attr: badge.attr,
                    master_rank: badge.master_rank,
                    trained: badge.trained,
                    level: badge.level,
                }),
            );
        }
        canvas.restore();
    }

    fn asset_keys(&self, _ctx: &RenderContext<'_>) -> Vec<String> {
        let mut keys = vec![self.asset_key.clone()];
        if let Some(badge) = &self.badge {
            let _ = badge.level;
            keys.push("card/bg_base_wh".to_string());
            keys.push(format!("card/cardFrame_M_{}", rarity_suffix(&badge.rarity)));
            keys.push(format!("card/cardFrame_L_{}", rarity_suffix(&badge.rarity)));
            keys.push(format!("card/icon_attribute_{}_64", badge.attr));
            keys.push(format!("card/icon_attribute_{}_88", badge.attr));
            keys.push(star_icon_key(&badge.rarity, badge.trained).to_string());
            keys.push(format!(
                "card/masterRank_S_{}",
                badge.master_rank.clamp(0, 5)
            ));
            keys.push(format!(
                "card/masterRank_L_{}",
                badge.master_rank.clamp(0, 5)
            ));
            let _ = rarity_count(&badge.rarity);
        }
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::CardMemberWidget;
    use crate::assets::AssetStore;
    use crate::context::RenderContext;
    use crate::types::{CardMemberElement, ObjectData, Quaternion, Vec3};
    use crate::widgets::theme::Theme;
    use crate::widgets::Widget;

    fn card_member(member_type: Option<i32>) -> CardMemberElement {
        CardMemberElement {
            object_data: ObjectData {
                layer: 0,
                lock: false,
                position: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                rotation: Quaternion {
                    w: 1.0,
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                scale: Vec3 {
                    x: 1.0,
                    y: 1.0,
                    z: 1.0,
                },
                visible: true,
            },
            id: 1,
            member_type,
            show_master_rank: None,
            use_after_special_training: None,
        }
    }

    #[test]
    fn card_member_measure_uses_fixed_crop_size_for_type1() {
        let assets = AssetStore::new(8);
        let theme = Theme::default();
        let ctx = RenderContext::new(&assets, &theme);
        let widget = CardMemberWidget {
            id: 1,
            member_type: 1,
            asset_key: "member/1".to_string(),
            badge: None,
        };

        assert_eq!(widget.measure(&ctx), (312.0, 512.0));
    }

    #[test]
    fn card_member_asset_keys_include_main_asset() {
        let assets = AssetStore::new(8);
        let theme = Theme::default();
        let ctx = RenderContext::new(&assets, &theme);
        let _ = card_member(Some(2));
        let widget = CardMemberWidget {
            id: 1,
            member_type: 2,
            asset_key: "character/member_small/sample/card_normal".to_string(),
            badge: None,
        };

        assert_eq!(
            widget.asset_keys(&ctx),
            vec!["character/member_small/sample/card_normal"]
        );
    }
}
