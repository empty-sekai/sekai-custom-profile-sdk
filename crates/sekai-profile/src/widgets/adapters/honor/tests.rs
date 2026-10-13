use super::{BondsHonorWidget, HonorWidget};
use crate::assets::AssetStore;
use crate::context::RenderContext;
use crate::masterdata::{
    MasterData, MasterDataProvider, ResolvedColor, ResolvedHonor, ResourceInfo,
};
use crate::types::{BondsHonorElement, HonorElement, ObjectData, Quaternion, Vec3};
use crate::types::{BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry};
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
            is_live_master: honor_id == 11,
            has_star: true,
            honor_level,
            honor_mission_type: (honor_id == 11).then(|| "live_master".to_string()),
        })
    }
    fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry> {
        // Honor 6 pairs a virtual singer (unit 21) with unit 1.
        let (first, second, rarity) = if id == 6 {
            (21, 1, "middle")
        } else {
            (1, 2, "high")
        };
        Some(BondsHonorEntry {
            id,
            game_character_unit_id1: first,
            game_character_unit_id2: second,
            honor_rarity: rarity.to_string(),
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
    fn resolve_unit_vs_sd(&self, self_id: i32, partner_id: i32) -> i32 {
        match (self_id, partner_id) {
            (21, 1) => 27,
            // Only reachable when the partner was already substituted.
            (1, 27) => 99,
            _ => self_id,
        }
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
    let masterdata = Box::leak(Box::new(MasterData::new(Arc::new(TestProvider))));
    RenderContext::new(assets).with_masterdata(masterdata)
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
fn live_master_honor_does_not_request_decorative_stars() {
    let widget = HonorWidget::from_element(&HonorElement {
        object_data: object_data(),
        id: 11,
        full_size: true,
        honor_level: 7,
    });
    let ctx = ctx();
    let keys = widget.asset_keys(&ctx);

    assert!(keys.iter().any(|key| key.ends_with("/scroll")));
    assert!(keys
        .iter()
        .all(|key| !key.contains("live_master_honor_star")));
}

#[test]
fn bonds_honor_widget_keys_follow_the_game_asset_rules() {
    let widget = BondsHonorWidget::from_element(&BondsHonorElement {
        object_data: object_data(),
        id: 6,
        word_id: 9,
        full_size: true,
        inverse: false,
        use_unit_virtual_singer: true,
        honor_level: 3,
    });
    let keys = widget.asset_keys(&ctx());

    // Backgrounds keep the listed units; only the character art switches
    // to the unit's virtual-singer variant.
    for key in [
        "honor/bonds/21",
        "honor/bonds/1",
        "bonds_honor/chr_sd_27_01",
        "bonds_honor/chr_sd_01_01",
        "bonds_honor/word/word_test_02",
        "honor/frame_degree_m_2",
    ] {
        assert!(keys.iter().any(|value| value == key), "{key} in {keys:?}");
    }
    assert!(!keys.iter().any(|value| value == "honor/bonds/27"));
    assert!(!keys.iter().any(|value| value.contains("chr_sd_99")));
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
