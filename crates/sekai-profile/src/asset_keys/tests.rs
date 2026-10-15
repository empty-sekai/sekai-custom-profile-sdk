use super::{key_to_s3_path, logical_path_to_asset_key};

const PREFIX: &str = "assets/cn/";

#[test]
fn game_assets_resolve_under_the_prefix() {
    assert_eq!(
        key_to_s3_path("character/member_small/abn/card_normal", PREFIX),
        "assets/cn/character/member_small/abn/card_normal.png"
    );
}

#[test]
fn indexed_bonds_honor_paths_restore_runtime_keys() {
    for key in [
        "bonds_honor/chr_sd_01_01",
        "bonds_honor/word/honorname_0102_01_01",
        "honor/example/degree_main",
    ] {
        let logical_path = key_to_s3_path(key, "");
        assert_eq!(
            logical_path_to_asset_key(&logical_path).as_deref(),
            Some(key)
        );
    }
}
struct PairHonorProvider;

impl crate::masterdata::MasterDataProvider for PairHonorProvider {
    fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> {
        None
    }
    fn get_card(&self, _: i32) -> Option<crate::types::CardEntry> {
        None
    }
    fn resolve_color(&self, _: i32) -> Option<crate::masterdata::ResolvedColor> {
        None
    }
    fn resolve_font(&self, _: i32) -> Option<String> {
        None
    }
    fn resolve_stamp(&self, _: i32) -> Option<String> {
        None
    }
    fn resolve_resource(&self, _: &str, _: i32) -> Option<crate::masterdata::ResourceInfo> {
        None
    }
    fn resolve_honor(&self, _: i32, _: i32) -> Option<crate::masterdata::ResolvedHonor> {
        None
    }
    fn get_bonds_honor(&self, id: i32) -> Option<crate::types::BondsHonorEntry> {
        Some(crate::types::BondsHonorEntry {
            id,
            game_character_unit_id1: 21,
            game_character_unit_id2: 1,
            honor_rarity: "middle".to_string(),
            configurable_unit_virtual_singer: true,
        })
    }
    fn get_bonds_honor_word(&self, id: i64) -> Option<crate::types::BondsHonorWordEntry> {
        Some(crate::types::BondsHonorWordEntry {
            id: id as i32,
            assetbundle_name: "honorname_0121_01".to_string(),
            bonds_group_id: 1,
            seq: 1,
        })
    }
    fn get_honor(&self, _: i32) -> Option<crate::types::HonorEntry> {
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

#[test]
fn card_honors_follow_the_levels_the_player_owns() {
    let card: crate::types::CustomProfileCard = serde_json::from_value(serde_json::json!({
        "bondsHonors": [
            {
                "objectData": {
                    "layer": 0, "lock": false, "visible": true,
                    "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                    "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
                },
                "id": 1, "wordId": 5, "fullSize": true, "inverse": false,
                "useUnitVirtualSinger": false
            }
        ]
    }))
    .expect("card fixture");
    let md = crate::masterdata::MasterData::new(std::sync::Arc::new(PairHonorProvider));
    let unowned = crate::profile::ProfileData::from_json(&serde_json::json!({
        "userHonors": [],
        "userBondsHonors": [{ "bondsHonorId": 2, "level": 3 }]
    }));
    assert!(
        super::missing_card_asset_keys_with_profile(&card, Some(&unowned), &md, |_| false)
            .is_empty()
    );
    let owned = crate::profile::ProfileData::from_json(&serde_json::json!({
        "userBondsHonors": [{ "bondsHonorId": 1, "level": 3 }]
    }));
    assert!(
        super::missing_card_asset_keys_with_profile(&card, Some(&owned), &md, |_| false)
            .contains(&"honor/bonds/21".to_string())
    );
}

#[test]
fn hidden_bonds_honors_request_no_assets() {
    let card: crate::types::CustomProfileCard = serde_json::from_value(serde_json::json!({
        "bondsHonors": [{
            "objectData": {
                "layer": 0, "lock": false, "visible": false,
                "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
            },
            "id": 1, "wordId": 5, "fullSize": true, "inverse": false,
            "useUnitVirtualSinger": false
        }]
    }))
    .expect("card fixture");
    let md = crate::masterdata::MasterData::new(std::sync::Arc::new(PairHonorProvider));
    assert!(super::collect_card_asset_keys(&card, &md).is_empty());
    assert!(super::missing_card_asset_keys(&card, &md, |_| false).is_empty());
}

#[test]
fn card_bonds_honors_request_every_layer_of_the_game_art() {
    let card: crate::types::CustomProfileCard = serde_json::from_value(serde_json::json!({
        "bondsHonors": [{
            "objectData": {
                "layer": 0, "lock": false, "visible": true,
                "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
            },
            "id": 1, "wordId": 5, "fullSize": true, "inverse": false,
            "useUnitVirtualSinger": true
        }]
    }))
    .expect("card fixture");
    let md = crate::masterdata::MasterData::new(std::sync::Arc::new(PairHonorProvider));
    let mut keys = super::collect_card_asset_keys(&card, &md);
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "bonds_honor/chr_sd_01_01",
            "bonds_honor/chr_sd_27_01",
            "bonds_honor/word/honorname_0121_01_02",
            "honor/bonds/1",
            "honor/bonds/21",
            "honor/frame_degree_m_2",
            "honor/icon_degreeLv",
            "honor/icon_degreeLv6",
            "honor/mask_degree_main",
        ]
    );
}

/// A character honor whose masterdata row carries neither a background
/// bundle nor a frame still has to produce a downloadable key. The
/// background falls back to the honor's own bundle name, so no key may
/// collapse into an empty path segment.
#[test]
fn a_character_honor_without_background_or_frame_keeps_every_path_segment() {
    use crate::masterdata::{
        MasterData, MasterDataProvider, ResolvedColor, ResolvedHonor, ResourceInfo,
    };
    use crate::types::{BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry};

    struct BackgroundlessHonorProvider;

    impl MasterDataProvider for BackgroundlessHonorProvider {
        fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> {
            None
        }
        fn get_card(&self, _: i32) -> Option<CardEntry> {
            None
        }
        fn resolve_color(&self, _: i32) -> Option<ResolvedColor> {
            None
        }
        fn resolve_font(&self, _: i32) -> Option<String> {
            None
        }
        fn resolve_stamp(&self, _: i32) -> Option<String> {
            None
        }
        fn resolve_resource(&self, _: &str, _: i32) -> Option<ResourceInfo> {
            None
        }
        fn resolve_honor(&self, _: i32, honor_level: i32) -> Option<ResolvedHonor> {
            Some(ResolvedHonor {
                asset_bundle_name: "honor_0001".to_string(),
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
        fn get_bonds_honor(&self, _: i32) -> Option<BondsHonorEntry> {
            None
        }
        fn get_bonds_honor_word(&self, _: i64) -> Option<BondsHonorWordEntry> {
            None
        }
        fn get_honor(&self, _: i32) -> Option<HonorEntry> {
            None
        }
        fn resolve_unit_vs_sd(&self, self_id: i32, _: i32) -> i32 {
            self_id
        }
        fn font_count(&self) -> usize {
            0
        }
        fn color_count(&self) -> usize {
            0
        }
    }

    let md = MasterData::new(std::sync::Arc::new(BackgroundlessHonorProvider));
    let mut requirements = super::AssetRequirements::default();
    super::collect_honor_keys(1, 1, false, &md, &mut requirements);
    let keys = requirements.keys();

    let background = keys
        .iter()
        .find(|key| key.contains("degree_sub"))
        .expect("a background key");
    assert_eq!(background, "honor/honor_0001/degree_sub");
    for key in &keys {
        assert!(!key.contains("//"), "key has an empty path segment: {key}");
    }
}
