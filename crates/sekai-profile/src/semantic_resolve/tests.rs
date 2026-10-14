use std::sync::Arc;

use sekai_profile_core::{FontRole, ParameterValue, SemanticCommandPayload};

use super::*;
use crate::masterdata::{MasterDataProvider, ResolvedColor, ResolvedHonor, ResourceInfo};
use crate::types::{BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry};

struct EmptyProvider;

fn assert_shared_semantic_parity(
    mut native: ResolvedProfileScene,
    mut shared: ResolvedProfileScene,
) {
    // Native server diagnostics retain table/id provenance. It is not a
    // rendering input; every rendering and interaction field stays gated.
    for scene in [&mut native, &mut shared] {
        for layer in &mut scene.layers {
            layer.resolved_parameters.remove("resource_source.table");
            layer.resolved_parameters.remove("resource_source.id");
        }
        for region in &mut scene.interaction_regions {
            region.resolved_data.remove("resource_source.table");
            region.resolved_data.remove("resource_source.id");
        }
    }
    assert_eq!(
        native, shared,
        "native semantic adapter drifted from shared core"
    );
}

impl MasterDataProvider for EmptyProvider {
    fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> {
        None
    }
    fn get_card(&self, id: i32) -> Option<CardEntry> {
        match id {
            9001 => Some(CardEntry {
                id,
                asset_bundle_name: "card_member_cropped".into(),
                card_rarity_type: "rarity_4".into(),
                attr: "cute".into(),
                character_id: 1,
            }),
            9002 => Some(CardEntry {
                id,
                asset_bundle_name: "card_member_full".into(),
                card_rarity_type: "rarity_3".into(),
                attr: "cool".into(),
                character_id: 2,
            }),
            _ => None,
        }
    }
    fn resolve_color(&self, _: i32) -> Option<ResolvedColor> {
        None
    }
    fn resolve_font(&self, _: i32) -> Option<String> {
        Some("FZLanTingHei-DB-GBK".into())
    }
    fn resolve_stamp(&self, _: i32) -> Option<String> {
        None
    }
    fn resolve_resource(&self, _: &str, _: i32) -> Option<ResourceInfo> {
        None
    }
    fn resolve_honor(&self, id: i32, level: i32) -> Option<ResolvedHonor> {
        (id == 4242).then(|| ResolvedHonor {
            asset_bundle_name: "live-master-fixture".into(),
            honor_rarity: "high".into(),
            honor_type: "achievement".into(),
            background_asset_bundle_name: None,
            frame_name: None,
            is_live_master: true,
            has_star: true,
            honor_level: level,
            honor_mission_type: Some("live_master".into()),
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
        1
    }
    fn color_count(&self) -> usize {
        0
    }
    fn resolve_localized_text(&self, key: &str) -> Option<String> {
        (key == "custom_profile.general.comment.title").then(|| "ひとこと".into())
    }
}

/// Master data holding one row in each table these tests read: font 1,
/// colour 3, which is also the first colour row, shape 1, which loads
/// [`SHAPE_KEY`], and `customProfileEtcResources` 1, which loads
/// [`OTHER_IMAGE_KEY`].
struct SparseRowsProvider;

const OTHER_IMAGE_KEY: &str = "custom_profile/etc/etc_001";
const SHAPE_KEY: &str = "custom_profile/shape_v2/shape_001";
const FIRST_COLOR: ResolvedColor = ResolvedColor {
    r: 12,
    g: 34,
    b: 56,
    a: 255,
};

impl MasterDataProvider for SparseRowsProvider {
    fn resolve_story_banner(&self, story_type: &str, story_id: i32) -> Option<String> {
        EmptyProvider.resolve_story_banner(story_type, story_id)
    }
    fn get_card(&self, id: i32) -> Option<CardEntry> {
        EmptyProvider.get_card(id)
    }
    fn resolve_color(&self, id: i32) -> Option<ResolvedColor> {
        (id == 3).then_some(FIRST_COLOR)
    }
    fn default_color(&self) -> Option<ResolvedColor> {
        Some(FIRST_COLOR)
    }
    fn resolve_font(&self, id: i32) -> Option<String> {
        (id == 1).then(|| "FZLanTingHei-DB-GBK".into())
    }
    fn resolve_stamp(&self, id: i32) -> Option<String> {
        EmptyProvider.resolve_stamp(id)
    }
    fn resolve_resource(&self, res_type: &str, id: i32) -> Option<ResourceInfo> {
        let (file_name, load_val) = match (res_type, id) {
            ("etc", 1) => ("etc_001", "custom_profile/etc"),
            ("shape", 1) => ("shape_001", "custom_profile/shape_v2"),
            _ => return None,
        };
        Some(ResourceInfo {
            file_name: file_name.into(),
            load_val: load_val.into(),
            resource_type: res_type.into(),
            collection_type: crate::masterdata::CollectionResourceType::None,
        })
    }
    fn resolve_honor(&self, id: i32, level: i32) -> Option<ResolvedHonor> {
        EmptyProvider.resolve_honor(id, level)
    }
    fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry> {
        EmptyProvider.get_bonds_honor(id)
    }
    fn get_bonds_honor_word(&self, id: i64) -> Option<BondsHonorWordEntry> {
        EmptyProvider.get_bonds_honor_word(id)
    }
    fn get_honor(&self, id: i32) -> Option<HonorEntry> {
        EmptyProvider.get_honor(id)
    }
    fn resolve_unit_vs_sd(&self, self_id: i32, partner_id: i32) -> i32 {
        EmptyProvider.resolve_unit_vs_sd(self_id, partner_id)
    }
    fn font_count(&self) -> usize {
        1
    }
    fn color_count(&self) -> usize {
        1
    }
}

/// A Live Master honor composed from its parts keeps its volatile progress
/// count as a separate text command and draws no live stars.
#[test]
fn live_master_honor_keeps_its_dynamic_progress_overlay() {
    let md = MasterData::new(Arc::new(EmptyProvider));
    let visual = build_standard_honor_visual(
        "slot-1",
        4242,
        3,
        true,
        None,
        &md,
        ResolveResourceContext::default(),
    )
    .unwrap();
    match visual.visual {
        HonorVisualKind::Standard {
            is_live_master,
            live_star_on,
            live_star_off,
            ..
        } => {
            assert!(is_live_master);
            assert!(live_star_on.is_none());
            assert!(live_star_off.is_none());
        }
        HonorVisualKind::Bonds { .. } => panic!("live-master changed semantic kind"),
    }

    let object = serde_json::json!({
        "layer": 7, "lock": false,
        "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
        "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
        "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
    });
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "generals": [{ "objectData": object, "type": 6 }]
    }))
    .unwrap();
    let profile = ProfileData {
        honor_slots: vec![crate::profile::HonorSlot {
            honor_id: 4242,
            honor_level: 3,
            full_size: true,
            profile_honor_type: "normal".into(),
            ..crate::profile::HonorSlot::default()
        }],
        user_honor_missions: std::collections::HashMap::from([("live_master".into(), 73)]),
        ..ProfileData::default()
    };
    let scene = resolve_card_commands_with_resources(
        &card,
        &md,
        "live-master-general",
        Some(&profile),
        "cn",
        ResolveResourceContext::default(),
    )
    .unwrap();
    assert!(scene
        .commands
        .iter()
        .any(|command| command.role == "honor-4242-progress"));
    assert!(!scene
        .commands
        .iter()
        .any(|command| command.role.starts_with("honor-4242-live-star-")));
}

#[test]
fn placed_honors_follow_the_profile_honor_lists() {
    let object = serde_json::json!({
        "layer": 7, "lock": false,
        "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
        "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
        "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
    });
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "honors": [{ "objectData": object, "id": 4242, "fullSize": true }]
    }))
    .expect("card fixture");
    let md = MasterData::new(Arc::new(EmptyProvider));
    let levels = |profile: Option<&ProfileData>| {
        build_resolve_snapshot_parts(
            &card,
            &md,
            "placed-honors",
            profile,
            "cn",
            ResolveResourceContext::default(),
            true,
            false,
        )
        .honor_visuals
        .values()
        .map(|visual| visual.honor_level)
        .collect::<Vec<_>>()
    };
    assert_eq!(levels(None), vec![1]);
    let owned = ProfileData::from_json(&serde_json::json!({ "userHonors": [[4242, 3, null]] }));
    assert_eq!(levels(Some(&owned)), vec![3]);
    let unowned = ProfileData::from_json(&serde_json::json!({ "userHonors": [[7, 3, null]] }));
    assert_eq!(levels(Some(&unowned)), Vec::<i32>::new());
    let scene =
        resolve_card_commands_with_profile(&card, &md, "placed-honors", Some(&unowned), "cn", None)
            .expect("scene");
    assert!(!scene
        .commands
        .iter()
        .any(|command| command.role.starts_with("honor-4242-")));
}

#[test]
fn every_authored_element_remains_exactly_one_game_layer() {
    let object = serde_json::json!({
        "layer": 7, "lock": false,
        "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
        "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
        "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
    });
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "texts": [{ "objectData": object, "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 1, "outlineSize": 0.0, "size": 24.0, "text": "<b>A</b>", "type": 1 }],
        "shapes": [{ "objectData": object, "alpha": 1.0, "colorId": 1, "id": 1, "outlineAlpha": 1.0, "outlineColorId": 1, "outlineSize": 0.0 }],
        "cardMembers": [{ "objectData": object, "id": 1 }], "stamps": [{ "objectData": object, "id": 1 }], "others": [{ "objectData": object, "id": 1 }],
        "bondsHonors": [{ "objectData": object, "id": 1, "wordId": 1, "fullSize": true, "inverse": false, "useUnitVirtualSinger": false }],
        "honors": [{ "objectData": object, "id": 1, "fullSize": true }], "collections": [{ "objectData": object, "id": 1 }],
        "generals": [{ "objectData": object, "type": 4 }], "standMembers": [{ "objectData": object, "id": 1 }],
        "generalBackgrounds": [{ "objectData": object, "id": 1 }], "storyBackgrounds": [{ "objectData": object, "id": 1 }]
    })).unwrap();
    let md = MasterData::new(Arc::new(EmptyProvider));
    let resolved = resolve_card_commands(&card, &md, "fixture").unwrap();
    let shared = sekai_profile_core::profile_resolve::compile_profile_scene(
        &card,
        None,
        &md,
        "fixture",
        "und",
        &(),
        std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert_shared_semantic_parity(resolved.clone(), shared);
    assert_eq!(
        (
            resolved.layers.len(),
            resolved.commands.len(),
            resolved.interaction_regions.len()
        ),
        (12, 12, 12)
    );
    assert_eq!(
        resolved
            .layers
            .iter()
            .map(|layer| layer.authored_kind)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        12
    );
    assert!(resolved.layers.iter().all(|layer| layer.game_layer == 7));
}

/// Collection 2, a 2022 omikuji slip, and its `omikujis` row 7, from the
/// tables of `region`.
struct OmikujiProvider {
    region: crate::region::Region,
}

impl MasterDataProvider for OmikujiProvider {
    fn resolve_story_banner(&self, story_type: &str, story_id: i32) -> Option<String> {
        EmptyProvider.resolve_story_banner(story_type, story_id)
    }
    fn get_card(&self, id: i32) -> Option<CardEntry> {
        EmptyProvider.get_card(id)
    }
    fn resolve_color(&self, id: i32) -> Option<ResolvedColor> {
        EmptyProvider.resolve_color(id)
    }
    fn resolve_font(&self, id: i32) -> Option<String> {
        EmptyProvider.resolve_font(id)
    }
    fn resolve_stamp(&self, id: i32) -> Option<String> {
        EmptyProvider.resolve_stamp(id)
    }
    fn resolve_resource(&self, res_type: &str, id: i32) -> Option<ResourceInfo> {
        (res_type == "collection" && id == 2).then(|| ResourceInfo {
            file_name: sekai_profile_core::omikuji::PREFAB_FILE.into(),
            load_val: "lottery_game/new_year_2022".into(),
            resource_type: res_type.into(),
            collection_type: crate::masterdata::CollectionResourceType::Omikuji,
        })
    }
    fn resolve_honor(&self, id: i32, level: i32) -> Option<ResolvedHonor> {
        EmptyProvider.resolve_honor(id, level)
    }
    fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry> {
        EmptyProvider.get_bonds_honor(id)
    }
    fn get_bonds_honor_word(&self, id: i64) -> Option<BondsHonorWordEntry> {
        EmptyProvider.get_bonds_honor_word(id)
    }
    fn get_honor(&self, id: i32) -> Option<HonorEntry> {
        EmptyProvider.get_honor(id)
    }
    fn resolve_unit_vs_sd(&self, self_id: i32, partner_id: i32) -> i32 {
        EmptyProvider.resolve_unit_vs_sd(self_id, partner_id)
    }
    fn font_count(&self) -> usize {
        1
    }
    fn color_count(&self) -> usize {
        0
    }
    fn region(&self) -> crate::region::Region {
        self.region
    }
    fn resolve_omikuji(&self, id: i32) -> Option<crate::masterdata::OmikujiRow> {
        (id == 7).then(|| crate::masterdata::OmikujiRow {
            id,
            unit: "street".into(),
            summary: "夢の実現に
近づく年"
                .into(),
            title1: "願望".into(),
            description1: "必ず 叶う".into(),
            title2: "健康".into(),
            description2: "大変良好".into(),
            title3: "待人".into(),
            description3: "必ず来る".into(),
            fortune_assetbundle_name: "lottery_game/new_year_2022_material".into(),
            fortune_file_path: "unsei_kichi".into(),
            omikuji_cover_assetbundle_name: "lottery_game/new_year_2022_material".into(),
            omikuji_cover_file_path: "omikuji_street".into(),
        })
    }
}

#[test]
fn omikuji_slips_lower_like_the_shared_core_in_every_client_region() {
    let object = |layer: i32| {
        serde_json::json!({
            "layer": layer, "lock": false,
            "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
        })
    };
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "collections": [
            { "objectData": object(1), "id": 2, "targetId": 7 },
            { "objectData": object(2), "id": 2, "targetId": 8 }
        ]
    }))
    .expect("card fixture");
    use crate::region::Region;
    for region in [Region::Cn, Region::Jp, Region::Tw, Region::Kr, Region::En] {
        let md = MasterData::new(Arc::new(OmikujiProvider { region }));
        let resolved = resolve_card_commands(&card, &md, "slips").expect("native scene");
        let shared = sekai_profile_core::profile_resolve::compile_profile_scene(
            &card,
            None,
            &md,
            "slips",
            "und",
            &(),
            std::collections::BTreeMap::new(),
        )
        .expect("shared scene");
        let texts = resolved
            .commands
            .iter()
            .filter(|command| matches!(command.payload, SemanticCommandPayload::UguiText(_)))
            .count();
        assert_eq!(texts, 7, "{region:?}");
        assert_shared_semantic_parity(resolved, shared);
    }
}

#[test]
fn hidden_elements_and_missing_master_rows_stay_empty_like_the_shared_core() {
    let object = |layer, visible| {
        serde_json::json!({
            "layer": layer, "lock": false,
            "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": visible
        })
    };
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "shapes": [{
            "objectData": object(1, true), "alpha": 1.0, "colorId": 1, "id": 5,
            "outlineAlpha": 0.0, "outlineColorId": 1, "outlineSize": 0.0
        }],
        "stamps": [
            { "objectData": object(2, true), "id": 3 },
            { "objectData": object(3, false), "id": 3 }
        ],
        "cardMembers": [{ "objectData": object(4, true), "id": 7, "type": 2 }]
    }))
    .expect("card fixture");
    let md = MasterData::new(Arc::new(EmptyProvider));
    let resolved = resolve_card_commands(&card, &md, "omitted").expect("native scene");
    let shared = sekai_profile_core::profile_resolve::compile_profile_scene(
        &card,
        None,
        &md,
        "omitted",
        "und",
        &(),
        std::collections::BTreeMap::new(),
    )
    .expect("shared scene");
    assert_shared_semantic_parity(resolved.clone(), shared);
    assert_eq!(resolved.layers.len(), 4);
    assert!(resolved
        .commands
        .iter()
        .all(|command| matches!(command.payload, SemanticCommandPayload::Composite { .. })));
    assert!(crate::asset_keys::collect_card_asset_keys(&card, &md).is_empty());
}

#[test]
fn hidden_honors_are_neither_drawn_nor_requested() {
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "honors": [{
            "objectData": {
                "layer": 1, "lock": false,
                "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": false
            },
            "id": 4242, "fullSize": true, "honorLevel": 3
        }]
    }))
    .expect("card fixture");
    let md = MasterData::new(Arc::new(EmptyProvider));
    let resolved = resolve_card_commands(&card, &md, "hidden-honor").expect("native scene");
    let shared = sekai_profile_core::profile_resolve::compile_profile_scene(
        &card,
        None,
        &md,
        "hidden-honor",
        "und",
        &(),
        std::collections::BTreeMap::new(),
    )
    .expect("shared scene");
    assert_shared_semantic_parity(resolved.clone(), shared);
    assert!(resolved
        .commands
        .iter()
        .all(|command| matches!(command.payload, SemanticCommandPayload::Composite { .. })));
    assert!(crate::asset_keys::collect_card_asset_keys(&card, &md).is_empty());
}

#[test]
fn authored_card_member_overlay_matches_shared_true_false_contract() {
    let object = |layer| {
        serde_json::json!({
            "layer": layer, "lock": false,
            "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
        })
    };
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "cardMembers": [
            {
                "objectData": object(1), "id": 9001, "type": 1,
                "showMasterRank": true
            },
            {
                "objectData": object(2), "id": 9002, "type": 2,
                "showMasterRank": true, "useAfterSpecialTraining": false
            },
            {
                "objectData": object(3), "id": 9001, "type": 1,
                "showMasterRank": false
            }
        ]
    }))
    .unwrap();
    let profile = ProfileData {
        user_cards: std::collections::HashMap::from([
            (
                9001,
                crate::profile::UserCardInfo {
                    after_training: true,
                    special_training_done: false,
                    master_rank: 4,
                    level: 37,
                },
            ),
            (
                9002,
                crate::profile::UserCardInfo {
                    after_training: false,
                    special_training_done: true,
                    master_rank: 2,
                    level: 28,
                },
            ),
        ]),
        ..ProfileData::default()
    };
    let md = MasterData::new(Arc::new(EmptyProvider));
    let resolved = resolve_card_commands_with_profile(
        &card,
        &md,
        "card-member-overlay",
        Some(&profile),
        "cn",
        None,
    )
    .unwrap();
    let shared = sekai_profile_core::profile_resolve::compile_profile_scene(
        &card,
        Some(&profile.to_core_profile()),
        &md,
        "card-member-overlay",
        "cn",
        &(),
        std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert_shared_semantic_parity(resolved.clone(), shared);

    let commands_for = |index| {
        let layer = resolved
            .layers
            .iter()
            .find(|layer| layer.authored_index == index)
            .unwrap();
        resolved
            .commands
            .iter()
            .filter(|command| command.layer_id == layer.id)
            .collect::<Vec<_>>()
    };
    let cropped = commands_for(0);
    assert_eq!(cropped.len(), 10);
    assert!(matches!(
        &cropped[2].payload,
        SemanticCommandPayload::Text {
            source: sekai_profile_core::TextSource::Localized { key, value, .. },
            font_role: FontRole::RegionFontId(1),
            ..
        } if key == "custom_profile.general.card_level" && value == "Lv.37"
    ));
    assert!(matches!(
        &cropped[5].payload,
        SemanticCommandPayload::Image { resource, .. }
            if resource.key == "card/rarity_star_normal"
    ));
    assert!(matches!(
        &cropped[9].payload,
        SemanticCommandPayload::Image { resource, .. }
            if resource.key == "card/masterRank_L_4"
    ));

    let full = commands_for(1);
    assert_eq!(full.len(), 7);
    assert!(matches!(
        &full[3].payload,
        SemanticCommandPayload::Image { resource, .. }
            if resource.key == "card/rarity_star_afterTraining"
    ));
    assert!(matches!(
        &full[6].payload,
        SemanticCommandPayload::Image { resource, .. }
            if resource.key == "card/masterRank_L_2"
    ));

    let image_only = commands_for(2);
    assert_eq!(image_only.len(), 1);
    assert_eq!(image_only[0].role, "card-member");
}

#[test]
fn player_name_and_signature_lower_inside_original_general_layers() {
    let object = serde_json::json!({
        "layer": 9, "lock": false,
        "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
        "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
        "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
    });
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "generals": [{ "objectData": object, "type": 13 }, { "objectData": object, "type": 4 }]
    }))
    .unwrap();
    let profile = ProfileData {
        user_name: "<b>Player</b>".into(),
        word: "<color=#ff0000>Bio</color>".into(),
        ..ProfileData::default()
    };
    let md = MasterData::new(Arc::new(EmptyProvider));
    let core_profile = profile.to_core_profile();
    let resolved = resolve_card_commands_with_profile(
        &card,
        &md,
        "identity-fixture",
        Some(&profile),
        "ja-JP",
        None,
    )
    .unwrap();
    let shared = sekai_profile_core::profile_resolve::compile_profile_scene(
        &card,
        Some(&core_profile),
        &md,
        "identity-fixture",
        "ja-JP",
        &(),
        std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert_shared_semantic_parity(resolved.clone(), shared);
    assert_eq!(
        (
            resolved.layers.len(),
            resolved.commands.len(),
            resolved.interaction_regions.len()
        ),
        (2, 5, 4)
    );
    let text_payloads = resolved
        .commands
        .iter()
        .filter_map(|command| match &command.payload {
            SemanticCommandPayload::Text {
                source, font_role, ..
            } => Some((source, font_role)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(text_payloads.len(), 3);
    assert!(text_payloads
        .iter()
        .all(|(_, role)| **role == FontRole::RegionFontId(1)));
    assert!(text_payloads.iter().any(|(source, _)| matches!(source, sekai_profile_core::TextSource::Localized { key, locale, .. } if key == "custom_profile.general.comment.title" && locale == "ja-JP")));
    assert_eq!(
        resolved
            .interaction_regions
            .iter()
            .filter(
                |region| region.capabilities == ["inspect", "select_text", "edit_text"]
                    && matches!(
                        region.resolved_data.get("field"),
                        Some(ParameterValue::Text(_))
                    )
            )
            .count(),
        2
    );
    assert_eq!(
        resolved
            .interaction_regions
            .iter()
            .filter(|region| region.capabilities == ["inspect", "select_layer"])
            .count(),
        2
    );
}

#[test]
fn page_without_generals_matches_full_profile_base_semantics() {
    if crate::sdf::outline::load_font_bytes_for_family("FZLanTingHei-DB-GBK").is_none() {
        eprintln!("skipping: FONT_DIR does not provide the test family");
        return;
    }

    let object = serde_json::json!({
        "layer": 3, "lock": false,
        "position": { "x": 12.0, "y": -7.0, "z": 0.0 },
        "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
        "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
    });
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "texts": [{
            "objectData": object, "colorId": 1, "fontId": 1, "lineSpacing": 0.0,
            "outlineColorId": 1, "outlineSize": 0.0, "size": 24.0,
            "text": "<line-indent=90.1%>No general component", "type": 513
        }]
    }))
    .unwrap();
    let profile = ProfileData {
        user_name: "Unused player component".into(),
        word: "Unused signature".into(),
        ..ProfileData::default()
    };
    let md = MasterData::new(Arc::new(EmptyProvider));
    let optimized = resolve_card_commands_with_resources(
        &card,
        &md,
        "page-only-fixture",
        Some(&profile),
        "ja-JP",
        ResolveResourceContext::default(),
    )
    .unwrap();
    let mut full_base = build_profile_resolve_base_snapshot(
        &card,
        &md,
        Some(&profile),
        "ja-JP",
        ResolveResourceContext::default(),
    );
    let full = resolve_card_commands_with_base(
        &mut full_base,
        &card,
        &md,
        "page-only-fixture",
        Some(&profile),
        ResolveResourceContext::default(),
    )
    .unwrap();
    assert_eq!(optimized, full);
    assert!(optimized.layers.iter().all(|layer| layer.game_layer == 3));

    let (full_scene, _) = crate::core_shadow::build_scene_with_resolved(
        &card,
        &md,
        "page-only-fixture",
        "cn",
        Some(&profile),
        "ja-JP",
        None,
    )
    .unwrap();
    let motion_scene =
        crate::core_shadow::build_text_scene(&card, &md, "page-only-fixture").unwrap();
    let full_preflight = full_scene.animation_preflight(120).unwrap();
    let motion_preflight = motion_scene.animation_preflight(120).unwrap();
    assert!(full_preflight.observable_program_count > 0);
    assert_eq!(full_preflight, motion_preflight);
    let authored =
        sekai_profile_core::profile_scene::ordered_profile_elements(&card, "page-only-fixture");
    assert!(motion_preflight
        .observable_layer_ids
        .iter()
        .all(|layer_id| authored.iter().any(|element| element.layer_id == *layer_id)));
}

#[test]
fn optional_resource_descriptors_follow_the_available_asset_snapshot() {
    let empty_assets = crate::assets::AssetStore::new(1);
    assert!(optional_descriptor(
        "honor/missing/rank_sub".into(),
        (180.0, 80.0),
        "honor_overlay",
        1,
        ResolveResourceContext {
            assets: Some(&empty_assets),
            ..ResolveResourceContext::default()
        },
    )
    .is_none());
    assert!(optional_descriptor(
        "honor/deferred/rank_sub".into(),
        (180.0, 80.0),
        "honor_overlay",
        1,
        ResolveResourceContext::default(),
    )
    .is_some());
}

#[test]
fn text_scene_layers_keep_layer_local_geometry() {
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "texts": [{
            "objectData": {
                "layer": 1, "lock": false,
                "position": { "x": 100.0, "y": 50.0, "z": 0.0 },
                "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
            },
            "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 1,
            "outlineSize": 0.0, "size": 24.0, "text": "A", "type": 1
        }]
    }))
    .expect("text card");
    let md = MasterData::new(Arc::new(EmptyProvider));
    let dump = crate::core_shadow::build_text_scene(&card, &md, "fixture")
        .expect("text scene")
        .dump();
    let layer = &dump.layers[0];
    assert_eq!(layer.bounds, sekai_profile_core::Rect::default());
    assert_eq!(layer.quad, [[0.0; 2]; 4]);
    assert_eq!(layer.hit_geometry, [[0.0; 2]; 4]);
    assert_eq!((layer.matrix[4], layer.matrix[5]), (1015.0, 356.0));
}

#[test]
fn text_scene_layers_use_the_default_font_and_color_rows_for_unknown_ids() {
    let text = |layer: i32, font_id: i32, color_id: i32, outline_color_id: i32| {
        serde_json::json!({
            "objectData": {
                "layer": layer, "lock": false,
                "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
            },
            "colorId": color_id, "fontId": font_id, "lineSpacing": 0.0,
            "outlineColorId": outline_color_id, "outlineSize": 0.2, "size": 24.0,
            "text": "A", "type": 1
        })
    };
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "texts": [text(1, 1, 3, 3), text(2, 404, 405, 406)]
    }))
    .expect("text card");
    let md = MasterData::new(Arc::new(SparseRowsProvider));
    let dump = crate::core_shadow::build_text_scene(&card, &md, "fallback-rows")
        .expect("text scene")
        .dump();
    let drawn = |index: usize| {
        let parameters = &dump.layers[index].resolved_parameters;
        ["font_family", "color", "outline_color"].map(|name| parameters.get(name).cloned())
    };
    assert!(drawn(0).iter().all(Option::is_some), "{:?}", drawn(0));
    assert_eq!(drawn(1), drawn(0));
}

#[test]
fn image_elements_request_the_keys_of_their_master_rows() {
    let object = |layer: i32| {
        serde_json::json!({
            "layer": layer, "lock": false,
            "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
        })
    };
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "shapes": [{
            "objectData": object(1), "alpha": 1.0, "colorId": 3, "id": 1,
            "outlineAlpha": 0.0, "outlineColorId": 3, "outlineSize": 0.0
        }],
        "others": [{ "objectData": object(2), "id": 1 }]
    }))
    .expect("image card");
    let md = MasterData::new(Arc::new(SparseRowsProvider));
    let mut keys = crate::asset_keys::collect_card_asset_keys(&card, &md);
    keys.sort();
    assert_eq!(keys, [OTHER_IMAGE_KEY, SHAPE_KEY]);
}

#[test]
fn source_images_draw_at_their_pixel_size_without_a_baked_object() {
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "others": [{
            "objectData": {
                "layer": 1, "lock": false,
                "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
            },
            "id": 1
        }]
    }))
    .expect("image card");
    let md = MasterData::new(Arc::new(SparseRowsProvider));
    let (width, height) = (64u32, 36u32);
    let assets = crate::assets::AssetStore::new(8);
    assets.put(
        OTHER_IMAGE_KEY.into(),
        crate::codec::png::encode_rgba(width, height, &[255, 0, 0, 255].repeat(64 * 36))
            .expect("png"),
    );

    let resolved =
        resolve_card_commands_with_profile(&card, &md, "fixture", None, "und", Some(&assets))
            .expect("resolved scene");
    let layer = &resolved.layers[0];
    assert_eq!(
        (layer.bounds.width, layer.bounds.height),
        (width as f32, height as f32)
    );

    // The store holds an unrelated object only, so the image falls back to
    // the asset store's source bytes.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut writer = crate::render_object::RenderObjectStoreWriter::create(
        temp.path().join("store"),
        "fixture",
        4096,
    )
    .expect("store writer");
    writer
        .add(crate::render_object::RenderObjectWrite {
            key: "unrelated",
            kind: crate::render_object::RenderObjectKind::Texture,
            source_sha256: &"0".repeat(64),
            width: 1,
            height: 1,
            row_bytes: 4,
            pixels: &[0; 4],
        })
        .expect("store object");
    let manifest = writer.finish().expect("store manifest");
    let store = crate::render_object::MappedRenderObjectStore::open(manifest).expect("store");
    let (canvas_width, canvas_height) = (
        sekai_profile_core::profile_transform::CANVAS_WIDTH as u32,
        sekai_profile_core::profile_transform::CANVAS_HEIGHT as u32,
    );
    let mut pixels = vec![0u8; canvas_width as usize * canvas_height as usize * 4];
    let stats = crate::profile_compositor::render_authored_profile_into_scalar(
        &resolved,
        &store,
        None,
        &md,
        Some(&assets),
        sekai_profile_core::AuthoredElementKind::Other,
        0,
        &mut pixels,
        canvas_width,
        canvas_height,
    )
    .expect("composited element");
    assert_eq!(stats.source_fallback_object_count, 1);

    let painted = pixels
        .iter()
        .skip(3)
        .step_by(4)
        .enumerate()
        .filter(|(_, alpha)| **alpha > 0)
        .map(|(index, _)| (index as u32 % canvas_width, index as u32 / canvas_width))
        .collect::<Vec<_>>();
    assert_eq!(painted.len(), (width * height) as usize);
    let (min_x, min_y, max_x, max_y) = painted.iter().fold(
        (u32::MAX, u32::MAX, 0, 0),
        |(min_x, min_y, max_x, max_y), &(x, y)| {
            (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
        },
    );
    assert_eq!((max_x - min_x + 1, max_y - min_y + 1), (width, height));
}
