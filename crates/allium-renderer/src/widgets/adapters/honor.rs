//! 称号类元素 adapter。

use crate::context::RenderContext;
use crate::types::{BondsHonorElement, HonorElement};
use crate::widgets::Widget;

#[cfg(feature = "skia-core")]
use crate::elements::honor::{render_bonds_honor, render_honor};

/// 普通称号元素 Widget adapter。
pub struct HonorWidget {
    honor_id: i32,
    honor_level: i32,
    full_size: bool,
}

impl HonorWidget {
    /// 从普通称号元素构建 adapter。
    pub fn from_element(elem: &HonorElement) -> Self {
        Self {
            honor_id: elem.id,
            honor_level: elem.honor_level,
            full_size: elem.full_size,
        }
    }
}

impl Widget for HonorWidget {
    fn name(&self) -> &'static str {
        "honor"
    }

    fn measure(&self, _ctx: &RenderContext<'_>) -> (f32, f32) {
        if self.full_size {
            (380.0, 80.0)
        } else {
            (180.0, 80.0)
        }
    }

    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, ctx: &RenderContext<'_>) {
        let Some(masterdata) = ctx.masterdata else {
            return;
        };
        canvas.save();
        canvas.translate((x, y));
        render_honor(
            canvas,
            self.honor_id,
            self.honor_level,
            self.full_size,
            masterdata,
            ctx.assets,
            ctx.profile,
        );
        canvas.restore();
    }

    fn asset_keys(&self, ctx: &RenderContext<'_>) -> Vec<String> {
        collect_honor_keys(ctx, self.honor_id, self.honor_level, self.full_size)
    }
}

/// 羁绊称号元素 Widget adapter。
pub struct BondsHonorWidget {
    bonds_honor_id: i32,
    honor_level: i32,
    full_size: bool,
    word_id: i64,
    inverse: bool,
    use_unit_virtual_singer: bool,
}

impl BondsHonorWidget {
    /// 从羁绊称号元素构建 adapter。
    pub fn from_element(elem: &BondsHonorElement) -> Self {
        Self {
            bonds_honor_id: elem.id,
            honor_level: elem.honor_level,
            full_size: elem.full_size,
            word_id: elem.word_id,
            inverse: elem.inverse,
            use_unit_virtual_singer: elem.use_unit_virtual_singer,
        }
    }
}

impl Widget for BondsHonorWidget {
    fn name(&self) -> &'static str {
        "bonds_honor"
    }

    fn measure(&self, _ctx: &RenderContext<'_>) -> (f32, f32) {
        let _ = self.honor_level;
        if self.full_size {
            (380.0, 80.0)
        } else {
            (180.0, 80.0)
        }
    }

    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, ctx: &RenderContext<'_>) {
        let Some(masterdata) = ctx.masterdata else {
            return;
        };
        canvas.save();
        canvas.translate((x, y));
        render_bonds_honor(
            canvas,
            self.bonds_honor_id,
            self.honor_level,
            self.full_size,
            self.word_id,
            self.inverse,
            self.use_unit_virtual_singer,
            masterdata,
            ctx.assets,
        );
        canvas.restore();
    }

    fn asset_keys(&self, ctx: &RenderContext<'_>) -> Vec<String> {
        collect_bonds_honor_keys(
            ctx,
            self.bonds_honor_id,
            self.full_size,
            self.word_id,
            self.inverse,
            self.use_unit_virtual_singer,
        )
    }
}

fn collect_honor_keys(
    ctx: &RenderContext<'_>,
    honor_id: i32,
    honor_level: i32,
    full_size: bool,
) -> Vec<String> {
    let Some(masterdata) = ctx.masterdata else {
        return Vec::new();
    };
    let Some(resolved) = masterdata.resolve_honor(honor_id, honor_level) else {
        return Vec::new();
    };

    let mut keys = Vec::new();
    let suffix = if full_size { "main" } else { "sub" };
    let bg_abn = resolved
        .background_asset_bundle_name
        .as_deref()
        .unwrap_or(&resolved.asset_bundle_name);
    let bg_dir = if resolved.honor_type == "rank_match" {
        "rank_live/honor"
    } else {
        "honor"
    };
    keys.push(format!("{}/{}/degree_{}", bg_dir, bg_abn, suffix));

    let overlay_name = if resolved.honor_type == "rank_match" {
        Some(suffix.to_string())
    } else if resolved.is_live_master {
        Some("scroll".to_string())
    } else if resolved.honor_type == "character" {
        let tier = (resolved.honor_level / 10) + 1;
        Some(format!("rank_{}_{tier}", suffix))
    } else {
        Some(format!("rank_{}", suffix))
    };
    if let Some(name) = overlay_name {
        let overlay_dir = if resolved.honor_type == "rank_match" {
            "rank_live/honor"
        } else {
            "honor"
        };
        keys.push(format!(
            "{}/{}/{}",
            overlay_dir, resolved.asset_bundle_name, name
        ));
    }

    if let Some(frame_name) = resolved.frame_name {
        let size_char = if full_size { "m" } else { "s" };
        let rarity_num = match resolved.honor_rarity.as_str() {
            "low" => 1,
            "middle" => 2,
            "high" => 3,
            _ => 4,
        };
        keys.push(format!(
            "honor_frame/{}/frame_degree_{}_{}",
            frame_name, size_char, rarity_num
        ));
    } else {
        let size_char = if full_size { "m" } else { "s" };
        let rarity_num = match resolved.honor_rarity.as_str() {
            "low" => 1,
            "middle" => 2,
            "high" => 3,
            _ => 4,
        };
        keys.push(format!("honor/frame_degree_{}_{}", size_char, rarity_num));
    }

    if resolved.is_live_master {
        keys.push("honor/live_master_honor_star_1".to_string());
        keys.push("honor/live_master_honor_star_2".to_string());
    } else if resolved.has_star
        && matches!(resolved.honor_type.as_str(), "character" | "achievement")
    {
        keys.push("honor/icon_degreeLv".to_string());
        if resolved.honor_level % 10 > 5 {
            keys.push("honor/icon_degreeLv6".to_string());
        }
    }

    keys
}

fn collect_bonds_honor_keys(
    ctx: &RenderContext<'_>,
    bonds_honor_id: i32,
    full_size: bool,
    word_id: i64,
    inverse: bool,
    use_unit_virtual_singer: bool,
) -> Vec<String> {
    let Some(masterdata) = ctx.masterdata else {
        return Vec::new();
    };
    let Some(entry) = masterdata.get_bonds_honor(bonds_honor_id) else {
        return Vec::new();
    };

    let (mut cid1, mut cid2) = if inverse {
        (entry.game_character_unit_id2, entry.game_character_unit_id1)
    } else {
        (entry.game_character_unit_id1, entry.game_character_unit_id2)
    };
    if use_unit_virtual_singer {
        let resolved1 = masterdata.resolve_unit_vs_sd(cid1, cid2);
        let resolved2 = masterdata.resolve_unit_vs_sd(cid2, cid1);
        cid1 = resolved1;
        cid2 = resolved2;
    }

    let size_char = if full_size { "main" } else { "sub" };
    let mut keys = vec![
        format!("honor/mask_degree_{}", size_char),
        format!("bonds_honor/chr_sd_{:02}_01", cid1),
        format!("bonds_honor/chr_sd_{:02}_01", cid2),
    ];

    if full_size {
        if let Some(word) = masterdata.get_bonds_honor_word(word_id) {
            keys.push(format!("bonds_honor/word/{}_01", word.assetbundle_name));
        }
    }

    let rarity_num = match entry.honor_rarity.as_str() {
        "low" => 1,
        "middle" => 2,
        "high" => 3,
        _ => 4,
    };
    let frame_size = if full_size { "m" } else { "s" };
    keys.push(format!("honor/frame_degree_{}_{}", frame_size, rarity_num));
    keys.push("honor/icon_degreeLv".to_string());
    keys.push("honor/icon_degreeLv6".to_string());
    if full_size {
        keys.push(format!("honor/bonds/{}", cid1));
        keys.push(format!("honor/bonds/{}", cid2));
    } else {
        keys.push(format!("honor/bonds/{}_sub", cid1));
        keys.push(format!("honor/bonds/{}_sub", cid2));
    }

    keys
}

#[cfg(test)]
mod tests {
    use super::{BondsHonorWidget, HonorWidget};
    use crate::assets::AssetStore;
    use crate::context::RenderContext;
    use crate::masterdata::{
        MasterData, MasterDataProvider, ResolvedColor, ResolvedHonor, ResourceInfo,
    };
    use crate::types::{BondsHonorElement, HonorElement, ObjectData, Quaternion, Vec3};
    use crate::types::{BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry};
    use crate::widgets::theme::Theme;
    use crate::widgets::Widget;
    use std::sync::Arc;

    struct TestProvider;

    impl MasterDataProvider for TestProvider {
        fn resolve_story_banner(&self, _story_type: &str, _story_id: i32) -> Option<String> {
            None
        }
        fn get_card(&self, _card_id: i32) -> Option<CardEntry> {
            None
        }
        fn resolve_color(&self, _color_id: i32) -> Option<ResolvedColor> {
            None
        }
        fn resolve_font(&self, _font_id: i32) -> Option<String> {
            None
        }
        fn resolve_stamp(&self, _stamp_id: i32) -> Option<String> {
            None
        }
        fn resolve_resource(&self, _res_type: &str, _id: i32) -> Option<ResourceInfo> {
            None
        }
        fn resolve_honor(&self, honor_id: i32, honor_level: i32) -> Option<ResolvedHonor> {
            Some(ResolvedHonor {
                asset_bundle_name: format!("honor_{honor_id}"),
                honor_rarity: "high".to_string(),
                honor_type: "character".to_string(),
                background_asset_bundle_name: None,
                frame_name: None,
                is_live_master: false,
                has_star: true,
                honor_level,
                honor_mission_type: None,
            })
        }
        fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry> {
            Some(BondsHonorEntry {
                id,
                game_character_unit_id1: 1,
                game_character_unit_id2: 2,
                honor_rarity: "high".to_string(),
                configurable_unit_virtual_singer: false,
            })
        }
        fn get_bonds_honor_word(&self, word_id: i64) -> Option<BondsHonorWordEntry> {
            Some(BondsHonorWordEntry {
                id: word_id as i32,
                assetbundle_name: "word_test".to_string(),
                bonds_group_id: 1,
                seq: 1,
            })
        }
        fn get_honor(&self, _honor_id: i32) -> Option<HonorEntry> {
            None
        }
        fn resolve_unit_vs_sd(&self, self_id: i32, _partner_id: i32) -> i32 {
            self_id
        }
        fn font_count(&self) -> usize {
            0
        }
        fn color_count(&self) -> usize {
            0
        }
    }

    fn object_data() -> ObjectData {
        ObjectData {
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
        }
    }

    fn ctx() -> RenderContext<'static> {
        let assets = Box::leak(Box::new(AssetStore::new(8)));
        let theme = Box::leak(Box::new(Theme::default()));
        let masterdata = Box::leak(Box::new(MasterData::new(Arc::new(TestProvider))));
        RenderContext::new(assets, theme).with_masterdata(masterdata)
    }

    #[test]
    fn honor_widget_reports_size_and_asset_keys() {
        let widget = HonorWidget::from_element(&HonorElement {
            object_data: object_data(),
            id: 10,
            full_size: true,
            honor_level: 7,
        });
        let ctx = ctx();

        assert_eq!(widget.measure(&ctx), (380.0, 80.0));
        assert!(widget
            .asset_keys(&ctx)
            .iter()
            .any(|key| key.contains("honor_10")));
    }

    #[test]
    fn bonds_honor_widget_reports_size_and_asset_keys() {
        let widget = BondsHonorWidget::from_element(&BondsHonorElement {
            object_data: object_data(),
            id: 5,
            word_id: 9,
            full_size: false,
            inverse: false,
            use_unit_virtual_singer: false,
            honor_level: 3,
        });
        let ctx = ctx();

        assert_eq!(widget.measure(&ctx), (180.0, 80.0));
        assert!(widget
            .asset_keys(&ctx)
            .iter()
            .any(|key| key.contains("chr_sd_01_01")));
    }
}
