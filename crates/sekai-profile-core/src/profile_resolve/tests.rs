#[test]
fn honor_frames_resolve_custom_fallback_or_no_frame() {
    struct Available<'a>(&'a [&'a str]);
    impl super::ResourceMetadata for Available<'_> {
        fn metric(&self, _: &crate::ResourceKey) -> Option<super::ResourceMetric> {
            None
        }
        fn availability(&self, key: &crate::ResourceKey) -> super::ResourceAvailability {
            if self.0.contains(&key.key.as_str()) {
                super::ResourceAvailability::Available
            } else {
                super::ResourceAvailability::Unavailable
            }
        }
    }
    let masterdata = |rarity: &str| {
        let mut md = crate::masterdata::JsonMasterData::new("en");
        md.insert_value(
            "honors",
            serde_json::json!([{
                "id": 1, "assetbundleName": "honor_sample", "honorRarity": rarity,
                "groupId": 1, "levels": [{ "level": 1 }]
            }]),
        )
        .unwrap();
        md.insert_value(
            "honorGroups",
            serde_json::json!([{
                "id": 1, "honorType": "event", "frameName": "custom_frame"
            }]),
        )
        .unwrap();
        md
    };
    let card = serde_json::from_value(serde_json::json!({"honors": [{
        "objectData": {
            "layer": 0, "lock": false, "visible": true,
            "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
        },
        "id": 1, "honorLevel": 1, "fullSize": true
    }]}))
    .unwrap();
    let custom = "honor_frame/custom_frame/frame_degree_m_3";
    let fallback = "honor/frame_degree_m_3";
    let low_custom = "honor_frame/custom_frame/frame_degree_m_1";
    let low_fallback = "honor/frame_degree_m_1";
    for (rarity, available, expected) in [
        ("high", vec![custom, fallback], Some(("assets", custom))),
        ("high", vec![custom], Some(("assets", custom))),
        ("high", vec![fallback], Some(("static", fallback))),
        ("high", vec![], None),
        // Below the high rarity the group frame is never used.
        (
            "low",
            vec![low_custom, low_fallback],
            Some(("static", low_fallback)),
        ),
        ("low", vec![low_custom], None),
    ] {
        let visual = super::standard_honor_visual(
            "honor",
            1,
            1,
            true,
            None,
            &masterdata(rarity),
            &Available(&available),
        )
        .unwrap();
        let source_key = crate::profile_scene::ordered_profile_elements(&card, "frames")[0]
            .source_key
            .clone();
        let snapshot = crate::profile_scene::ProfileResolveSnapshot {
            honor_visuals: std::collections::BTreeMap::from([(source_key, visual)]),
            ..Default::default()
        };
        let scene =
            crate::profile_scene::resolve_profile_scene(&card, "frames", &snapshot).unwrap();
        let frames = scene
            .commands
            .iter()
            .filter_map(|command| match &command.payload {
                crate::SemanticCommandPayload::Image { resource, .. }
                    if resource.key.contains("frame_degree") =>
                {
                    Some((resource.namespace.as_str(), resource.key.as_str()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(frames, expected.into_iter().collect::<Vec<_>>());
    }
}
use super::*;
use crate::masterdata::JsonMasterData;

#[test]
fn compiles_a_text_and_shape_scene_from_synthetic_tables() {
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "texts": [{ "objectData": { "layer": 1, "lock": false, "position": {"x":0.0,"y":0.0,"z":0.0}, "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0}, "scale": {"x":1.0,"y":1.0,"z":1.0}, "visible": true }, "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 2, "outlineSize": 0.0, "size": 32.0, "text": "42", "type": 0 }],
        "shapes": [{ "objectData": { "layer": 2, "lock": false, "position": {"x":0.0,"y":0.0,"z":0.0}, "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0}, "scale": {"x":1.0,"y":1.0,"z":1.0}, "visible": true }, "alpha": 1.0, "colorId": 1, "id": 8, "outlineAlpha": 0.0, "outlineColorId": 2, "outlineSize": 0.0 }]
    })).unwrap();
    let mut data = JsonMasterData::new("cn");
    data.insert_value(
        "customProfileTextFonts",
        serde_json::json!([{ "id": 1, "fontName": "SyntheticSans" }]),
    )
    .unwrap();
    data.insert_value("customProfileTextColors", serde_json::json!([{ "id": 1, "colorCode": "#ffffff" }, { "id": 2, "colorCode": "#00000000" }])).unwrap();
    data.insert_value("customProfileShapeResources", serde_json::json!([{ "id": 8, "fileName": "shape_round", "resourceLoadVal": "custom_profile/shape", "customProfileResourceType": "shape" }])).unwrap();
    let scene =
        compile_profile_scene(&card, None, &data, "synthetic", "cn", &(), BTreeMap::new()).unwrap();
    assert_eq!(scene.layers.len(), 2);
    assert_eq!(scene.commands.len(), 2);
    assert_eq!(scene.commands[0].numeric_text_runs[0].text, "42");
}

fn visible_object(layer: i32, visible: bool) -> serde_json::Value {
    serde_json::json!({
        "layer": layer, "lock": false, "visible": visible,
        "position": {"x":0.0,"y":0.0,"z":0.0},
        "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0},
        "scale": {"x":1.0,"y":1.0,"z":1.0}
    })
}

fn text_and_shape_tables() -> JsonMasterData {
    let mut data = JsonMasterData::new("cn");
    data.insert_value(
        "customProfileTextFonts",
        serde_json::json!([{ "id": 1, "fontName": "SyntheticSans" }, { "id": 2, "fontName": "SyntheticRound" }]),
    )
    .unwrap();
    data.insert_value(
        "customProfileTextColors",
        serde_json::json!([{ "id": 7, "colorCode": "#444466" }, { "id": 2, "colorCode": "#ffffff" }]),
    )
    .unwrap();
    data.insert_value(
        "customProfileShapeResources",
        serde_json::json!([{ "id": 1, "fileName": "round", "resourceLoadVal": "custom_profile/shape", "customProfileResourceType": "shape" }]),
    )
    .unwrap();
    data
}

fn layer_commands(
    scene: &crate::profile_scene::ResolvedProfileScene,
    kind: crate::AuthoredElementKind,
    index: u32,
) -> Vec<&crate::SemanticCommandSource> {
    let layer = scene
        .layers
        .iter()
        .find(|layer| layer.authored_kind == kind && layer.authored_index == index)
        .expect("every authored element keeps its layer");
    scene
        .commands
        .iter()
        .filter(|command| command.layer_id == layer.id)
        .collect()
}

#[test]
fn elements_without_a_master_data_row_are_left_out_instead_of_requesting_invented_keys() {
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "shapes": [
            { "objectData": visible_object(1, true), "alpha": 1.0, "colorId": 7, "id": 1, "outlineAlpha": 0.0, "outlineColorId": 7, "outlineSize": 0.0 },
            { "objectData": visible_object(2, true), "alpha": 1.0, "colorId": 7, "id": 99, "outlineAlpha": 0.0, "outlineColorId": 7, "outlineSize": 0.0 }
        ],
        "stamps": [{ "objectData": visible_object(3, true), "id": 5 }],
        "others": [{ "objectData": visible_object(4, true), "id": 6 }],
        "cardMembers": [{ "objectData": visible_object(5, true), "id": 7, "type": 1 }]
    }))
    .unwrap();
    let data = text_and_shape_tables();
    let preparation = prepare_profile(&card, None, &data, "missing-rows", "cn").unwrap();
    assert_eq!(
        preparation
            .resources
            .iter()
            .map(|request| request.resource.key.as_str())
            .collect::<Vec<_>>(),
        vec!["custom_profile/shape/round"]
    );
    let scene = compile_profile_scene(
        &card,
        None,
        &data,
        "missing-rows",
        "cn",
        &(),
        BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(scene.layers.len(), 5);
    for (kind, index) in [
        (crate::AuthoredElementKind::Shape, 1),
        (crate::AuthoredElementKind::Stamp, 0),
        (crate::AuthoredElementKind::Other, 0),
        (crate::AuthoredElementKind::CardMember, 0),
    ] {
        let commands = layer_commands(&scene, kind, index);
        assert_eq!(commands.len(), 1, "{kind:?}");
        assert!(
            matches!(
                commands[0].payload,
                crate::SemanticCommandPayload::Composite { .. }
            ),
            "{kind:?}"
        );
    }
}

#[test]
fn hidden_elements_are_not_resolved_and_request_nothing() {
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "texts": [{ "objectData": visible_object(1, false), "colorId": 7, "fontId": 404, "lineSpacing": 0.0, "outlineColorId": 7, "outlineSize": 0.0, "size": 32.0, "text": "hidden", "type": 0 }],
        "shapes": [{ "objectData": visible_object(2, false), "alpha": 1.0, "colorId": 7, "id": 1, "outlineAlpha": 0.0, "outlineColorId": 7, "outlineSize": 0.0 }]
    }))
    .unwrap();
    let data = text_and_shape_tables();
    let preparation = prepare_profile(&card, None, &data, "hidden", "cn").unwrap();
    assert!(preparation.resources.is_empty());
    assert!(preparation.glyph_layers.is_empty());
    assert!(preparation.layout_layers.is_empty());
    let scene =
        compile_profile_scene(&card, None, &data, "hidden", "cn", &(), BTreeMap::new()).unwrap();
    assert_eq!(scene.layers.len(), 2);
    assert!(scene.layers.iter().all(|layer| !layer.authored_visible));
    assert!(scene.commands.iter().all(|command| matches!(
        command.payload,
        crate::SemanticCommandPayload::Composite { .. }
    )));
}

#[test]
fn unknown_font_keeps_the_default_face_and_unknown_colors_use_the_first_row() {
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "texts": [{ "objectData": visible_object(1, true), "colorId": 404, "fontId": 404, "lineSpacing": 0.0, "outlineColorId": 405, "outlineSize": 0.2, "size": 32.0, "text": "text", "type": 0 }],
        "shapes": [{ "objectData": visible_object(2, true), "alpha": 0.5, "colorId": 406, "id": 1, "outlineAlpha": 0.25, "outlineColorId": 407, "outlineSize": 0.3 }]
    }))
    .unwrap();
    let data = text_and_shape_tables();
    let preparation = prepare_profile(&card, None, &data, "fallback", "cn").unwrap();
    assert_eq!(preparation.layout_layers[0].font_family, "SyntheticSans");
    let scene =
        compile_profile_scene(&card, None, &data, "fallback", "cn", &(), BTreeMap::new()).unwrap();
    let first_row = [
        0x44 as f32 / 255.0,
        0x44 as f32 / 255.0,
        0x66 as f32 / 255.0,
    ];
    let text = layer_commands(&scene, crate::AuthoredElementKind::Text, 0);
    let crate::SemanticCommandPayload::Text {
        color,
        outline_color,
        ..
    } = &text[0].payload
    else {
        panic!("text command expected");
    };
    assert_eq!(color[..3], first_row);
    assert_eq!(outline_color[..3], first_row);
    let shape = layer_commands(&scene, crate::AuthoredElementKind::Shape, 0);
    let crate::SemanticCommandPayload::Shape { fill, stroke, .. } = &shape[0].payload else {
        panic!("shape command expected");
    };
    assert_eq!(*fill, [first_row[0], first_row[1], first_row[2], 0.5]);
    assert_eq!(*stroke, [first_row[0], first_row[1], first_row[2], 0.25]);
}

#[test]
fn deck_stars_follow_special_training_and_story_favorites_keep_share_slots() {
    let raw = serde_json::json!({
        "userCards": [{
            "cardId": 1, "defaultImage": "original",
            "specialTrainingStatus": "done", "masterRank": 0, "level": 5
        }],
        "userDeck": { "leader": 1, "member1": 1 },
        "userStoryFavorites": [
            { "storyId": 3, "storyType": "event_story", "shareNo": 3 },
            { "storyId": 4, "storyType": "unit_story", "shareNo": 1 }
        ]
    });
    let profile = ProfileData::from_json(&raw);
    let mut data = JsonMasterData::new("cn");
    data.insert_value(
        "cards",
        serde_json::json!([{ "id": 1, "assetbundleName": "card_sample", "cardRarityType": "rarity_4", "attr": "cool", "characterId": 1 }]),
    )
    .unwrap();
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({})).unwrap();
    let snapshot = build_profile_snapshot(
        &card,
        Some(&profile),
        &data,
        "deck",
        "cn",
        &(),
        BTreeMap::new(),
    )
    .unwrap();
    let component = snapshot.component.unwrap();
    let deck = &component.deck_members[0];
    assert!(deck.after_training, "trained stars");
    assert_eq!(
        deck.image.descriptor.as_ref().unwrap().resource.key,
        "character/member_cutout/card_sample/normal"
    );
    assert!(
        !component.leader_card.as_ref().unwrap().after_training,
        "the leader card's stars follow the shown illustration"
    );
    assert_eq!(
        component
            .story_favorites
            .iter()
            .map(|story| story.story_id)
            .collect::<Vec<_>>(),
        vec![4, 0, 3]
    );
}

#[test]
fn authored_card_member_overlay_reuses_deck_and_leader_recipes_conditionally() {
    let object = |layer| {
        serde_json::json!({
            "layer": layer,
            "lock": false,
            "position": {"x":0.0,"y":0.0,"z":0.0},
            "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0},
            "scale": {"x":1.0,"y":1.0,"z":1.0},
            "visible": true
        })
    };
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "cardMembers": [
            {
                "objectData": object(1), "id": 1001, "type": 1,
                "showMasterRank": true
            },
            {
                "objectData": object(2), "id": 1002, "type": 2,
                "showMasterRank": true, "useAfterSpecialTraining": false
            },
            {
                "objectData": object(3), "id": 1003, "type": 1,
                "showMasterRank": false
            },
            {
                "objectData": object(4), "id": 1003, "type": 1,
                "showMasterRank": true
            }
        ]
    }))
    .unwrap();
    let mut data = JsonMasterData::new("cn");
    data.insert_value(
        "customProfileTextFonts",
        serde_json::json!([{ "id": 1, "fontName": "SyntheticSans" }]),
    )
    .unwrap();
    data.insert_value(
        "cards",
        serde_json::json!([
            {"id":1001,"assetbundleName":"card_a","cardRarityType":"rarity_4","attr":"cute","characterId":1},
            {"id":1002,"assetbundleName":"card_b","cardRarityType":"rarity_3","attr":"cool","characterId":2},
            {"id":1003,"assetbundleName":"card_c","cardRarityType":"rarity_2","attr":"pure","characterId":3}
        ]),
    )
    .unwrap();
    let profile = ProfileData {
        user_cards: BTreeMap::from([
            (
                1001,
                crate::profile_data::CardState {
                    card_id: 1001,
                    after_training: true,
                    special_training_done: false,
                    master_rank: 4,
                    level: 37,
                },
            ),
            (
                1002,
                crate::profile_data::CardState {
                    card_id: 1002,
                    after_training: false,
                    special_training_done: true,
                    master_rank: 2,
                    level: 28,
                },
            ),
        ]),
        ..ProfileData::default()
    };
    let scene = compile_profile_scene(
        &card,
        Some(&profile),
        &data,
        "card-member-overlay",
        "cn",
        &(),
        BTreeMap::new(),
    )
    .unwrap();

    let commands_for = |index| {
        let layer = scene
            .layers
            .iter()
            .find(|layer| layer.authored_index == index)
            .unwrap();
        scene
            .commands
            .iter()
            .filter(|command| command.layer_id == layer.id)
            .collect::<Vec<_>>()
    };
    let cropped = commands_for(0);
    assert_eq!(
        cropped
            .iter()
            .map(|command| command.role.as_str())
            .collect::<Vec<_>>(),
        vec![
            "card-member",
            "card-member-level-bar",
            "card-member-level",
            "card-member-frame",
            "card-member-attribute",
            "card-member-star-0",
            "card-member-star-1",
            "card-member-star-2",
            "card-member-star-3",
            "card-member-master-rank",
        ]
    );
    assert!(matches!(
        &cropped[2].payload,
        crate::SemanticCommandPayload::Text {
            source: crate::TextSource::Localized { key, value, .. },
            font_role: crate::FontRole::RegionFontId(1),
            ..
        } if key == "custom_profile.general.card_level" && value == "Lv.37"
    ));
    // The cropped card shows the untrained illustration of a trained card
    // but still draws trained stars and the large master-rank badge.
    assert!(matches!(
        &cropped[5].payload,
        crate::SemanticCommandPayload::Image { resource, .. }
            if resource.key == "card/rarity_star_normal"
    ));
    assert!(matches!(
        &cropped[9].payload,
        crate::SemanticCommandPayload::Image { resource, .. }
            if resource.key == "card/masterRank_L_4"
    ));

    let full = commands_for(1);
    assert_eq!(
        full.iter()
            .map(|command| command.role.as_str())
            .collect::<Vec<_>>(),
        vec![
            "card-member",
            "card-member-frame",
            "card-member-attribute",
            "card-member-star-0",
            "card-member-star-1",
            "card-member-star-2",
            "card-member-master-rank",
        ]
    );
    assert!(matches!(
        &full[3].payload,
        crate::SemanticCommandPayload::Image { resource, .. }
            if resource.key == "card/rarity_star_afterTraining"
    ));
    assert!(matches!(
        &full[6].payload,
        crate::SemanticCommandPayload::Image { resource, .. }
            if resource.key == "card/masterRank_L_2"
    ));

    let image_only = commands_for(2);
    assert_eq!(image_only.len(), 1);
    assert_eq!(image_only[0].role, "card-member");

    // A card the player does not own has master rank 0: no badge.
    let unranked = commands_for(3);
    assert!(unranked
        .iter()
        .all(|command| command.role != "card-member-master-rank"));
    assert!(unranked
        .iter()
        .any(|command| command.role == "card-member-level"));
}

#[test]
fn preparation_requests_only_resources_used_by_authored_general_types() {
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({})).unwrap();
    let mut data = JsonMasterData::new("cn");
    data.insert_value(
        "customProfileTextFonts",
        serde_json::json!([{ "id": 1, "fontName": "SyntheticSans" }]),
    )
    .unwrap();
    data.insert_value(
        "cards",
        serde_json::json!([{
            "id": 1007,
            "assetbundleName": "card_sample",
            "cardRarityType": "rarity_3",
            "attr": "cool",
            "characterId": 7
        }]),
    )
    .unwrap();
    let profile = ProfileData {
        challenge_character_id: 7,
        leader_card: Some(crate::profile_data::CardState {
            card_id: 1007,
            after_training: true,
            ..crate::profile_data::CardState::default()
        }),
        ..ProfileData::default()
    };
    let preparation =
        prepare_profile(&card, Some(&profile), &data, "unused-components", "cn").unwrap();
    assert!(
        preparation.resources.is_empty(),
        "profile data alone must not demand challenge/player avatar resources"
    );

    let avatar_card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "generals": [{
            "objectData": {
                "layer": 1, "lock": false,
                "position": {"x":0.0,"y":0.0,"z":0.0},
                "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0},
                "scale": {"x":1.0,"y":1.0,"z":1.0}, "visible": true
            },
            "type": 18
        }]
    }))
    .unwrap();
    let mut no_font_data = JsonMasterData::new("cn");
    no_font_data
        .insert_value(
            "cards",
            serde_json::json!([{
                "id": 1007, "assetbundleName": "card_sample",
                "cardRarityType": "rarity_3", "attr": "cool", "characterId": 7
            }]),
        )
        .unwrap();
    let avatar_preparation = prepare_profile(
        &avatar_card,
        Some(&profile),
        &no_font_data,
        "avatar-only",
        "cn",
    )
    .unwrap();
    assert!(avatar_preparation.fonts.is_empty());
    assert_eq!(avatar_preparation.resources.len(), 1);
    assert_eq!(
        avatar_preparation.resources[0].resource.key,
        "thumbnail/chara/card_sample_after_training"
    );

    let used_generals = [10, 17, 18]
        .into_iter()
        .enumerate()
        .map(|(index, general_type)| {
            serde_json::json!({
                "objectData": {
                    "layer": index as i32 + 1, "lock": false,
                    "position": {"x":0.0,"y":0.0,"z":0.0},
                    "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0},
                    "scale": {"x":1.0,"y":1.0,"z":1.0}, "visible": true
                },
                "type": general_type
            })
        })
        .collect::<Vec<_>>();
    let used_card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "generals": used_generals
    }))
    .unwrap();
    let used = prepare_profile(&used_card, Some(&profile), &data, "used-components", "cn").unwrap();
    assert_eq!(
        used.resources
            .iter()
            .map(|request| request.resource.key.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "chara_avatar/chara07_02",
            "sprite/icon/icon_playerRank",
            "thumbnail/chara/card_sample_after_training",
        ])
    );
}

#[test]
fn preparation_owns_complete_text_layout_with_full_affine_matrix() {
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "texts": [{
            "objectData": {
                "layer": 1,
                "lock": false,
                "position": {"x": 12.0, "y": 34.0, "z": 0.0},
                "rotation": {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0},
                "scale": {"x": -1.0, "y": 1.0, "z": 1.0},
                "visible": true
            },
            "colorId": 1,
            "fontId": 1,
            "lineSpacing": 2.0,
            "outlineColorId": 2,
            "outlineSize": 1.5,
            "size": 32.0,
            "text": "<b>42</b>",
            "type": 2
        }]
    }))
    .unwrap();
    let mut data = JsonMasterData::new("cn");
    data.insert_value(
        "customProfileTextFonts",
        serde_json::json!([{ "id": 1, "fontName": "SyntheticSans" }]),
    )
    .unwrap();
    data.insert_value(
        "customProfileTextColors",
        serde_json::json!([
            { "id": 1, "colorCode": "#112233cc" },
            { "id": 2, "colorCode": "#445566aa" }
        ]),
    )
    .unwrap();

    let preparation = prepare_profile(&card, None, &data, "affine", "cn").unwrap();
    assert_eq!(preparation.layout_layers.len(), 1);
    let layer = &preparation.layout_layers[0];
    assert_eq!(layer.text, "<b>42</b>");
    assert_eq!(layer.font_family, "SyntheticSans");
    assert_eq!(layer.font_size, 32.0);
    assert_eq!(layer.outline_width, 1.5);
    assert_eq!(layer.text_type, 2);
    assert!(layer.transform_matrix[0] < 0.0);
    assert!(
        layer.transform_matrix[0] * layer.transform_matrix[3]
            - layer.transform_matrix[1] * layer.transform_matrix[2]
            < 0.0
    );
}

#[test]
fn extended_pipeline_lowers_all_authored_kinds_and_component_controls() {
    fn object(layer: i32) -> serde_json::Value {
        serde_json::json!({ "layer": layer, "lock": false, "position": {"x":0.0,"y":0.0,"z":0.0}, "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0}, "scale": {"x":1.0,"y":1.0,"z":1.0}, "visible": true })
    }
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "texts": [{ "objectData": object(1), "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 2, "outlineSize": 0.0, "size": 24.0, "text": "123", "type": 0 }],
        "shapes": [{ "objectData": object(2), "alpha": 1.0, "colorId": 1, "id": 1, "outlineAlpha": 1.0, "outlineColorId": 2, "outlineSize": 1.0 }],
        "cardMembers": [{ "objectData": object(3), "id": 1, "type": 2, "showMasterRank": false, "useAfterSpecialTraining": false }],
        "stamps": [{ "objectData": object(4), "id": 1 }],
        "others": [{ "objectData": object(5), "id": 1 }],
        "bondsHonors": [{ "objectData": object(6), "id": 2, "wordId": 3, "fullSize": true, "inverse": false, "useUnitVirtualSinger": false, "honorLevel": 1 }],
        "honors": [{ "objectData": object(7), "id": 1, "fullSize": false, "honorLevel": 1 }],
        "collections": [{ "objectData": object(8), "id": 1, "targetId": null }],
        "generals": [{ "objectData": object(9), "type": 15 }],
        "standMembers": [{ "objectData": object(10), "id": 1 }],
        "generalBackgrounds": [{ "objectData": object(11), "id": 1 }],
        "storyBackgrounds": [{ "objectData": object(12), "id": 1 }]
    })).unwrap();
    let mut data = JsonMasterData::new("cn");
    data.insert_value(
        "customProfileTextFonts",
        serde_json::json!([{ "id": 1, "fontName": "SyntheticSans" }]),
    )
    .unwrap();
    data.insert_value("customProfileTextColors", serde_json::json!([{ "id": 1, "colorCode": "#ffffff" }, { "id": 2, "colorCode": "#000000" }])).unwrap();
    data.insert_value("customProfileShapeResources", serde_json::json!([{ "id": 1, "fileName": "shape", "resourceLoadVal": "shape", "customProfileResourceType": "shape" }])).unwrap();
    data.insert_value("cards", serde_json::json!([{ "id": 1, "assetbundleName": "card_sample", "cardRarityType": "rarity_3", "attr": "cool", "characterId": 1 }])).unwrap();
    data.insert_value(
        "stamps",
        serde_json::json!([{ "id": 1, "assetbundleName": "stamp_sample" }]),
    )
    .unwrap();
    for (table, load) in [
        ("customProfileEtcResources", "etc"),
        ("customProfileCollectionResources", "collection"),
        ("customProfileMemberStandingPictureResources", "standing"),
        ("customProfileGeneralBackgroundResources", "general_bg"),
        ("customProfileStoryBackgroundResources", "story_bg"),
    ] {
        data.insert_value(table, serde_json::json!([{ "id": 1, "fileName": "sample", "resourceLoadVal": load, "customProfileResourceType": load }])).unwrap();
    }
    data.insert_value("honors", serde_json::json!([{ "id": 1, "assetbundleName": "honor_sample", "honorRarity": "high", "groupId": 1, "levels": [{"level":1,"assetbundleName":null,"honorRarity":null}], "honorMissionType": null }])).unwrap();
    data.insert_value(
        "honorGroups",
        serde_json::json!([{ "id": 1, "honorType": "normal", "frameName": "custom_frame" }]),
    )
    .unwrap();
    data.insert_value("bondsHonors", serde_json::json!([{ "id": 2, "gameCharacterUnitId1": 1, "gameCharacterUnitId2": 2, "honorRarity": "high" }])).unwrap();
    data.insert_value(
        "bondsHonorWords",
        serde_json::json!([{ "id": 3, "assetbundleName": "word_sample" }]),
    )
    .unwrap();
    let profile = ProfileData {
        user_name: "Sample".into(),
        character_ranks: (1..=12)
            .map(|id| crate::profile_data::CharacterRank {
                character_id: id,
                rank: 10 + id,
            })
            .collect(),
        challenge_ranks: (1..=12)
            .map(|id| crate::profile_data::CharacterRank {
                character_id: id,
                rank: id,
            })
            .collect(),
        ..ProfileData::default()
    };
    let preparation =
        prepare_profile(&card, Some(&profile), &data, "synthetic-all-kinds", "cn").unwrap();
    let scene = compile_profile_scene(
        &card,
        Some(&profile),
        &data,
        "synthetic-all-kinds",
        "cn",
        &(),
        BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(scene.layers.len(), 12);
    assert!(scene.commands.len() > 12);
    let bonds_commands = scene
        .commands
        .iter()
        .filter(|command| command.role.starts_with("bonds-2-"))
        .collect::<Vec<_>>();
    let bonds_roles = bonds_commands
        .iter()
        .map(|command| command.role.as_str())
        .collect::<Vec<_>>();
    let begin = bonds_roles
        .iter()
        .position(|role| *role == "bonds-2-isolation-begin")
        .unwrap();
    let mask = bonds_roles
        .iter()
        .position(|role| *role == "bonds-2-mask")
        .unwrap();
    let end = bonds_roles
        .iter()
        .position(|role| *role == "bonds-2-isolation-end")
        .unwrap();
    assert!(begin < mask && mask < end);
    assert_eq!(bonds_commands[mask].blend_mode, crate::BlendMode::DstIn);
    assert!(bonds_commands.iter().all(|command| match &command.payload {
        crate::SemanticCommandPayload::Image { alpha_mask, .. } => alpha_mask.is_none(),
        _ => true,
    }));
    assert!(scene.controls.iter().any(|control| matches!(
        &control.state,
        crate::profile_scene::ComponentControlState::Tabs { .. }
    )));
    assert!(scene.controls.iter().any(|control| matches!(
        &control.state,
        crate::profile_scene::ComponentControlState::Scroll { .. }
    )));
    assert!(scene
        .interaction_regions
        .iter()
        .any(|region| region.role.contains("character")));
    assert!(preparation.font_families.contains("SyntheticSans"));
    assert_eq!(
        preparation.fonts.get(&1).map(String::as_str),
        Some("SyntheticSans")
    );
    assert!(preparation.layout_layers.len() > 1);
    assert_eq!(
        preparation.layout_layers[0].dynamic_layer_id,
        "synthetic-all-kinds\0text-source-0"
    );
    assert_eq!(preparation.layout_layers[0].font_family, "SyntheticSans");
    let prepared_text = preparation
        .glyph_layers
        .iter()
        .map(|layer| (layer.text.as_str(), layer.font_id))
        .collect::<BTreeSet<_>>();
    let mut component_text_count = 0;
    for command in &scene.commands {
        if let crate::SemanticCommandPayload::Text {
            source,
            font_role: crate::FontRole::RegionFontId(font_id),
            ..
        } = &command.payload
        {
            let value = match source {
                crate::TextSource::Authored { value } => value,
                crate::TextSource::ProfileField { value, .. }
                | crate::TextSource::MasterData { value, .. }
                | crate::TextSource::Localized { value, .. } => {
                    component_text_count += 1;
                    value
                }
            };
            assert!(
                prepared_text.contains(&(value.as_str(), *font_id)),
                "missing prepared glyph text {value:?} for font {font_id}"
            );
        }
    }
    assert!(
        component_text_count > 0,
        "fixture must exercise component text demand"
    );
    assert!(preparation.resources.iter().any(|request| {
        request.resource.namespace == "static" && request.resource.key == "chara_avatar/chara01_02"
    }));
    for key in [
        "honor/icon_degreeLv",
        "honor/icon_degreeLv6",
        "honor/mask_degree_main",
        "honor/frame_degree_m_3",
        "honor/bonds/1",
        "honor/bonds/2",
    ] {
        assert!(
            preparation.resources.iter().any(|request| {
                request.resource.namespace == "static" && request.resource.key == key
            }),
            "canonical local honor resource must be static: {key}"
        );
    }
    for key in [
        "honor/live_master_honor_star_1",
        "honor/live_master_honor_star_2",
    ] {
        assert!(
            preparation
                .resources
                .iter()
                .all(|request| request.resource.key != key),
            "Live Master decoration must not request removed resource: {key}"
        );
    }
    assert!(
        preparation.resources.iter().any(|request| {
            request.resource.namespace == "assets"
                && request.resource.key == "honor_frame/custom_frame/frame_degree_s_3"
        }),
        "custom honor frame must remain an unpacked asset"
    );
    for key in ["bonds_honor/chr_sd_01_01", "bonds_honor/chr_sd_02_01"] {
        assert!(
            preparation.resources.iter().any(|request| {
                request.resource.namespace == "assets" && request.resource.key == key
            }),
            "bonds character must remain an unpacked asset: {key}"
        );
    }
    let prepared = preparation
        .resources
        .iter()
        .map(|request| format!("{}\0{}", request.resource.namespace, request.resource.key))
        .collect::<BTreeSet<_>>();
    for command in &scene.commands {
        let mut required = Vec::new();
        match &command.payload {
            crate::SemanticCommandPayload::Image {
                resource,
                alpha_mask,
                ..
            } => {
                required.push(resource);
                if let Some(mask) = alpha_mask {
                    required.push(mask);
                }
            }
            crate::SemanticCommandPayload::Shape {
                primitive: crate::ShapePrimitive::AssetMask { resource },
                ..
            } => required.push(resource),
            _ => {}
        }
        for resource in required {
            assert!(
                prepared.contains(&format!("{}\0{}", resource.namespace, resource.key)),
                "missing prepared resource {}:{}",
                resource.namespace,
                resource.key
            );
        }
    }
}

#[test]
fn strict_external_localization_snapshot_drives_general_glyphs_without_region_fallback() {
    fn object(layer: i32) -> serde_json::Value {
        serde_json::json!({ "layer": layer, "lock": false, "position": {"x":0.0,"y":0.0,"z":0.0}, "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0}, "scale": {"x":1.0,"y":1.0,"z":1.0}, "visible": true })
    }
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "generals": [{ "objectData": object(4), "type": 4 }]
    }))
    .unwrap();
    let mut data = JsonMasterData::new("cn");
    data.insert_value(
        "customProfileTextFonts",
        serde_json::json!([{ "id": 1, "fontName": "SyntheticSans" }]),
    )
    .unwrap();
    let profile = ProfileData {
        word: "玩家原文".into(),
        ..ProfileData::default()
    };
    let localized = BTreeMap::from([(
        "custom_profile.general.comment.title".into(),
        "External Bio".into(),
    )]);
    let prepared = prepare_profile_with_localizations(
        &card,
        Some(&profile),
        &data,
        "localized-general",
        "cn",
        &localized,
    )
    .unwrap();
    assert!(prepared
        .glyph_layers
        .iter()
        .any(|layer| layer.text == "External Bio"));
    assert!(prepared
        .glyph_layers
        .iter()
        .any(|layer| layer.text == "玩家原文"));

    let error = prepare_profile_with_localizations(
        &card,
        Some(&profile),
        &data,
        "missing-localized-general",
        "cn",
        &BTreeMap::new(),
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("custom_profile.general.comment.title"));
}

#[test]
fn unavailable_provider_resource_is_preserved_as_component_state_not_fake_dimensions() {
    struct Unavailable;
    impl super::ResourceMetadata for Unavailable {
        fn metric(&self, _: &crate::ResourceKey) -> Option<super::ResourceMetric> {
            None
        }

        fn availability(&self, _: &crate::ResourceKey) -> super::ResourceAvailability {
            super::ResourceAvailability::Unavailable
        }
    }

    let profile = ProfileData {
        challenge_character_id: 7,
        ..ProfileData::default()
    };
    let data = JsonMasterData::new("cn");
    let snapshot =
        super::build_component(&profile, &data, "cn", &Unavailable, None, false).unwrap();
    assert!(snapshot
        .challenge_avatar
        .as_ref()
        .is_some_and(|image| image.descriptor.is_none()));
}

fn honor_object() -> serde_json::Value {
    serde_json::json!({
        "layer": 1, "lock": false,
        "position": {"x": 0.0, "y": 0.0, "z": 0.0},
        "rotation": {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0},
        "scale": {"x": 1.0, "y": 1.0, "z": 1.0}, "visible": true
    })
}

#[test]
fn bonds_honor_word_art_follows_the_honor_rarity() {
    let mut data = JsonMasterData::new("cn");
    data.insert_value(
        "bondsHonors",
        serde_json::json!([
            { "id": 1, "gameCharacterUnitId1": 1, "gameCharacterUnitId2": 2, "honorRarity": "low" },
            { "id": 2, "gameCharacterUnitId1": 1, "gameCharacterUnitId2": 2, "honorRarity": "middle" }
        ]),
    )
    .unwrap();
    data.insert_value(
        "bondsHonorWords",
        serde_json::json!([{ "id": 9, "assetbundleName": "honorname_0102_01" }]),
    )
    .unwrap();
    for (id, expected) in [
        (1, "bonds_honor/word/honorname_0102_01_01"),
        (2, "bonds_honor/word/honorname_0102_01_02"),
    ] {
        let visual =
            super::bonds_honor_visual("bonds", id, 1, true, 9, false, false, &data, &()).unwrap();
        let HonorVisualKind::Bonds { word, .. } = visual.visual else {
            panic!("bonds visual expected");
        };
        assert_eq!(word.unwrap().resource.key, expected);
    }
}

#[test]
fn placed_honors_take_the_owned_level_and_skip_unowned_honors() {
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "honors": [
            { "objectData": honor_object(), "id": 10, "fullSize": true },
            { "objectData": honor_object(), "id": 11, "fullSize": false }
        ],
        "bondsHonors": [
            { "objectData": honor_object(), "id": 20, "wordId": 0, "fullSize": false, "inverse": false, "useUnitVirtualSinger": false },
            { "objectData": honor_object(), "id": 21, "wordId": 0, "fullSize": false, "inverse": false, "useUnitVirtualSinger": false }
        ]
    }))
    .unwrap();
    let mut data = JsonMasterData::new("cn");
    data.insert_value(
        "honors",
        serde_json::json!([
            { "id": 10, "assetbundleName": "honor_0010", "honorRarity": "low", "groupId": 1, "levels": [{"level": 1}, {"level": 2}] },
            { "id": 11, "assetbundleName": "honor_0011", "honorRarity": "low", "groupId": 1, "levels": [{"level": 1}, {"level": 2}] }
        ]),
    )
    .unwrap();
    data.insert_value(
        "honorGroups",
        serde_json::json!([{ "id": 1, "honorType": "character" }]),
    )
    .unwrap();
    data.insert_value(
        "bondsHonors",
        serde_json::json!([
            { "id": 20, "gameCharacterUnitId1": 1, "gameCharacterUnitId2": 2, "honorRarity": "low" },
            { "id": 21, "gameCharacterUnitId1": 1, "gameCharacterUnitId2": 3, "honorRarity": "low" }
        ]),
    )
    .unwrap();
    let levels = |snapshot: &crate::profile_scene::ProfileResolveSnapshot| {
        snapshot
            .honor_visuals
            .values()
            .map(|visual| (visual.honor_id, visual.honor_level))
            .collect::<BTreeMap<_, _>>()
    };
    let snapshot = |profile: Option<&ProfileData>| {
        build_profile_snapshot(
            &card,
            profile,
            &data,
            "owned-honors",
            "cn",
            &(),
            BTreeMap::new(),
        )
        .unwrap()
    };

    // Without a profile the element's own level is kept.
    assert_eq!(
        levels(&snapshot(None)),
        BTreeMap::from([(10, 1), (11, 1), (20, 1), (21, 1)])
    );

    // userHonors rows are [honorId, level, obtainedAt]; userBondsHonors rows
    // are objects. Honors missing from either list are not drawn.
    let profile = ProfileData::from_json(&serde_json::json!({
        "userHonors": [[10, 34, 1700000000000_i64], [99, 2, null]],
        "userBondsHonors": [{ "bondsHonorId": 21, "level": 7, "obtainedAt": 0 }]
    }));
    assert_eq!(
        levels(&snapshot(Some(&profile))),
        BTreeMap::from([(10, 34), (21, 7)])
    );

    // Object-shaped honor rows are read the same way.
    let profile = ProfileData::from_json(&serde_json::json!({
        "userHonors": [{ "honorId": 11, "level": 3 }],
        "userBondsHonors": []
    }));
    assert_eq!(levels(&snapshot(Some(&profile))), BTreeMap::from([(11, 3)]));
}

#[test]
fn hidden_honors_are_not_resolved_and_request_nothing() {
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "honors": [{ "objectData": visible_object(1, false), "id": 10, "fullSize": true, "honorLevel": 2 }],
        "bondsHonors": [{ "objectData": visible_object(2, false), "id": 20, "wordId": 0, "fullSize": false, "inverse": false, "useUnitVirtualSinger": false, "honorLevel": 1 }]
    }))
    .unwrap();
    let mut data = JsonMasterData::new("cn");
    data.insert_value(
        "honors",
        serde_json::json!([{ "id": 10, "assetbundleName": "honor_0010", "honorRarity": "low", "groupId": 1, "levels": [{"level": 1}, {"level": 2}] }]),
    )
    .unwrap();
    data.insert_value(
        "honorGroups",
        serde_json::json!([{ "id": 1, "honorType": "character" }]),
    )
    .unwrap();
    data.insert_value(
        "bondsHonors",
        serde_json::json!([{ "id": 20, "gameCharacterUnitId1": 1, "gameCharacterUnitId2": 2, "honorRarity": "low" }]),
    )
    .unwrap();
    let snapshot = build_profile_snapshot(
        &card,
        None,
        &data,
        "hidden-honors",
        "cn",
        &(),
        BTreeMap::new(),
    )
    .unwrap();
    assert!(snapshot.honor_visuals.is_empty());
    let preparation = prepare_profile(&card, None, &data, "hidden-honors", "cn").unwrap();
    assert!(preparation.resources.is_empty());
}

#[test]
fn card_honors_draw_level_stars_for_every_multi_level_honor_type() {
    let honor_card = |level: i32| -> CustomProfileCard {
        serde_json::from_value(serde_json::json!({
            "honors": [{ "objectData": honor_object(), "id": 30, "fullSize": true, "honorLevel": level }]
        }))
        .unwrap()
    };
    for (honor_type, level, stars, high_stars) in [
        ("limitevent", 3, 3, 0),
        ("limitevent", 7, 5, 2),
        ("limitevent", 13, 3, 0),
        ("character", 4, 4, 0),
        ("achievement", 10, 5, 5),
        ("event", 7, 0, 0),
    ] {
        let mut data = JsonMasterData::new("cn");
        data.insert_value(
            "honors",
            serde_json::json!([{ "id": 30, "assetbundleName": "honor_0030", "honorRarity": "middle", "groupId": 1, "levels": [{"level": 1}, {"level": 2}] }]),
        )
        .unwrap();
        data.insert_value(
            "honorGroups",
            serde_json::json!([{ "id": 1, "honorType": honor_type }]),
        )
        .unwrap();
        let card = honor_card(level);
        let scene = compile_profile_scene(
            &card,
            None,
            &data,
            "honor-stars",
            "cn",
            &(),
            BTreeMap::new(),
        )
        .unwrap();
        let roles = layer_commands(&scene, crate::AuthoredElementKind::Honor, 0)
            .into_iter()
            .map(|command| command.role.clone())
            .collect::<Vec<_>>();
        let count = |prefix: &str| roles.iter().filter(|role| role.starts_with(prefix)).count();
        let high = count("honor-30-star-high-");
        assert_eq!(
            (count("honor-30-star-") - high, high),
            (stars, high_stars),
            "{honor_type} level {level}: {roles:?}"
        );
    }
}

fn prepared_honor_resource_keys(honor_type: &str, asset_bundle_name: &str) -> Vec<String> {
    let object = serde_json::json!({
        "layer": 1, "lock": false,
        "position": {"x": 0.0, "y": 0.0, "z": 0.0},
        "rotation": {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0},
        "scale": {"x": 1.0, "y": 1.0, "z": 1.0}, "visible": true
    });
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "honors": [{ "objectData": object, "id": 42, "fullSize": false, "honorLevel": 1 }]
    }))
    .unwrap();
    let mut data = JsonMasterData::new("cn");
    data.insert_value(
        "honors",
        serde_json::json!([{
            "id": 42, "assetbundleName": asset_bundle_name, "honorRarity": "middle",
            "groupId": 1, "levels": [{"level": 1, "assetbundleName": null, "honorRarity": null}],
            "honorMissionType": null
        }]),
    )
    .unwrap();
    data.insert_value(
        "honorGroups",
        serde_json::json!([{
            "id": 1, "honorType": honor_type, "hasStar": false, "isLiveMaster": false
        }]),
    )
    .unwrap();
    let prepared = prepare_profile(&card, None, &data, "character-honor", "cn").unwrap();
    prepared
        .resources
        .into_iter()
        .map(|request| request.resource.key)
        .collect()
}

#[test]
fn honor_types_without_rank_art_request_no_rank_overlay() {
    for (honor_type, asset_bundle_name) in [
        ("character", "honor_0042"),
        ("achievement", "honor_0105"),
        ("limitevent", "honor_se_ln_1"),
        ("limitevent", "honor_top_000020"),
    ] {
        let keys = prepared_honor_resource_keys(honor_type, asset_bundle_name);
        assert!(keys.iter().any(|key| key.ends_with("/degree_sub")));
        assert!(
            !keys.iter().any(|key| key.ends_with("/rank_sub")),
            "{honor_type} {asset_bundle_name} unexpectedly requested rank_sub: {keys:?}"
        );
    }
}

#[test]
fn overlay_bearing_honor_bundles_request_rank_assets() {
    for (honor_type, asset_bundle_name) in [
        ("sekai_echo", "honor_0182"),
        ("event", "honor_memorial"),
        ("event", "honor_0305"),
    ] {
        let keys = prepared_honor_resource_keys(honor_type, asset_bundle_name);
        assert!(
            keys.iter()
                .any(|key| key == &format!("honor/{asset_bundle_name}/rank_sub")),
            "{honor_type} {asset_bundle_name} did not request rank_sub: {keys:?}"
        );
    }
}

/// Colours plus one row in each icon table: character icon 1, material 3
/// and user-interface icon 2. Colour 7 is the first row.
fn icon_tables() -> JsonMasterData {
    let mut data = text_and_shape_tables();
    data.insert_value(
        "customProfileTextColors",
        serde_json::json!([{ "id": 7, "colorCode": "#444466" }, { "id": 2, "colorCode": "#ff8000" }]),
    )
    .unwrap();
    for (table, resource_type, id, file_name) in [
        (
            "customProfileCharacterIconResources",
            "character_icon",
            1,
            "profile_chr_icon_ichika",
        ),
        (
            "customProfileMaterialResources",
            "material",
            3,
            "profile_icon_item_0003",
        ),
        (
            "customProfileUserInterfaceIconResources",
            "user_interface_icon",
            2,
            "profile_icon_0002",
        ),
    ] {
        data.insert_value(
            table,
            serde_json::json!([{
                "id": id, "seq": id, "customProfileResourceType": resource_type,
                "resourceLoadType": "assetbundle",
                "resourceLoadVal": format!("custom_profile/{resource_type}"),
                "fileName": file_name
            }]),
        )
        .unwrap();
    }
    data
}

/// A version-4 page: one element of each icon kind that resolves, one of
/// each kind whose row is missing, and a hidden character icon.
fn icon_card() -> CustomProfileCard {
    serde_json::from_value(serde_json::json!({
        "characterIcons": [
            { "objectData": visible_object(1, true), "id": 1 },
            { "objectData": visible_object(4, true), "id": 99 },
            { "objectData": visible_object(8, false), "id": 1 }
        ],
        "materials": [
            { "objectData": visible_object(2, true), "id": 3 },
            { "objectData": visible_object(5, true), "id": 98 }
        ],
        "userInterfaceIcons": [
            { "objectData": visible_object(3, true), "id": 2, "colorId": 2, "alpha": 0.5 },
            { "objectData": visible_object(6, true), "id": 97, "colorId": 2, "alpha": 1.0 },
            { "objectData": visible_object(7, true), "id": 2, "colorId": 404, "alpha": 0.3 }
        ]
    }))
    .unwrap()
}

fn game_layer_commands(
    scene: &crate::profile_scene::ResolvedProfileScene,
    game_layer: i32,
) -> Vec<&crate::SemanticCommandSource> {
    let layer = scene
        .layers
        .iter()
        .find(|layer| layer.game_layer == game_layer)
        .expect("every authored element keeps its layer");
    scene
        .commands
        .iter()
        .filter(|command| command.layer_id == layer.id)
        .collect()
}

#[test]
fn icon_elements_parse_and_request_the_keys_of_their_master_rows() {
    let card = icon_card();
    assert_eq!(card.element_count(), 8);
    let data = icon_tables();
    let preparation = prepare_profile(&card, None, &data, "icons", "jp").unwrap();
    let mut keys = preparation
        .resources
        .iter()
        .map(|request| request.resource.key.as_str())
        .collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "custom_profile/character_icon/profile_chr_icon_ichika",
            "custom_profile/material/profile_icon_item_0003",
            "custom_profile/user_interface_icon/profile_icon_0002",
        ]
    );
    let scene =
        compile_profile_scene(&card, None, &data, "icons", "jp", &(), BTreeMap::new()).unwrap();
    assert_eq!(
        scene
            .layers
            .iter()
            .map(|layer| layer.game_layer)
            .collect::<Vec<_>>(),
        (1..=8).collect::<Vec<_>>()
    );
}

#[test]
fn icon_elements_draw_their_images_and_skip_missing_rows_and_hidden_elements() {
    let card = icon_card();
    let data = icon_tables();
    let scene =
        compile_profile_scene(&card, None, &data, "icons", "jp", &(), BTreeMap::new()).unwrap();
    let image = |game_layer: i32| {
        let commands = game_layer_commands(&scene, game_layer);
        assert_eq!(commands.len(), 1, "layer {game_layer}");
        match &commands[0].payload {
            crate::SemanticCommandPayload::Image { resource, tint, .. } => {
                (resource.key.clone(), *tint)
            }
            other => panic!("layer {game_layer} draws {other:?}"),
        }
    };
    assert_eq!(
        image(1),
        (
            "custom_profile/character_icon/profile_chr_icon_ichika".into(),
            [1.0; 4]
        )
    );
    assert_eq!(
        image(2),
        (
            "custom_profile/material/profile_icon_item_0003".into(),
            [1.0; 4]
        )
    );
    for game_layer in [4, 5, 6, 8] {
        let commands = game_layer_commands(&scene, game_layer);
        assert_eq!(commands.len(), 1, "layer {game_layer}");
        assert!(
            matches!(
                commands[0].payload,
                crate::SemanticCommandPayload::Composite { .. }
            ),
            "layer {game_layer}"
        );
    }
    assert!(
        !scene
            .layers
            .iter()
            .find(|layer| layer.game_layer == 8)
            .unwrap()
            .authored_visible
    );
}

#[test]
fn user_interface_icons_are_tinted_by_their_colour_row_and_color32_alpha() {
    let card = icon_card();
    let data = icon_tables();
    let scene =
        compile_profile_scene(&card, None, &data, "icons", "jp", &(), BTreeMap::new()).unwrap();
    let tint = |game_layer: i32| match &game_layer_commands(&scene, game_layer)[0].payload {
        crate::SemanticCommandPayload::Image { resource, tint, .. } => {
            assert_eq!(
                resource.key,
                "custom_profile/user_interface_icon/profile_icon_0002"
            );
            *tint
        }
        other => panic!("layer {game_layer} draws {other:?}"),
    };
    // 0.5 * 255 = 127.5 and 0.3 * 255 = 76.5 both round to the even step.
    assert_eq!(tint(3), [1.0, 128.0 / 255.0, 0.0, 128.0 / 255.0]);
    // An unknown colour takes the first row, like text and shapes.
    assert_eq!(
        tint(7),
        [
            0x44 as f32 / 255.0,
            0x44 as f32 / 255.0,
            0x66 as f32 / 255.0,
            76.0 / 255.0
        ]
    );
}

#[test]
fn master_data_without_the_icon_tables_leaves_icons_out_and_draws_the_rest() {
    let mut value = serde_json::to_value(icon_card()).unwrap();
    value["shapes"] = serde_json::json!([{
        "objectData": visible_object(9, true), "alpha": 1.0, "colorId": 7, "id": 1,
        "outlineAlpha": 0.0, "outlineColorId": 7, "outlineSize": 0.0
    }]);
    let card: CustomProfileCard = serde_json::from_value(value).unwrap();
    for table in [
        "customProfileCharacterIconResources",
        "customProfileMaterialResources",
        "customProfileUserInterfaceIconResources",
    ] {
        assert!(
            !crate::masterdata::PROFILE_MASTERDATA_TABLES.contains(&table),
            "{table} is not shipped by every region"
        );
        assert!(crate::masterdata::PROFILE_OPTIONAL_MASTERDATA_TABLES.contains(&table));
    }
    let data = text_and_shape_tables();
    let preparation = prepare_profile(&card, None, &data, "no-icon-tables", "cn").unwrap();
    assert_eq!(
        preparation
            .resources
            .iter()
            .map(|request| request.resource.key.as_str())
            .collect::<Vec<_>>(),
        ["custom_profile/shape/round"]
    );
    let scene = compile_profile_scene(
        &card,
        None,
        &data,
        "no-icon-tables",
        "cn",
        &(),
        BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(scene.layers.len(), 9);
    assert!(matches!(
        game_layer_commands(&scene, 9)[0].payload,
        crate::SemanticCommandPayload::Shape { .. }
    ));
    for game_layer in 1..=8 {
        assert!(
            game_layer_commands(&scene, game_layer)
                .iter()
                .all(|command| matches!(
                    command.payload,
                    crate::SemanticCommandPayload::Composite { .. }
                )),
            "layer {game_layer}"
        );
    }
}

const BADGE_KEY: &str = "custom_profile/collection/crash/crash_fixture_canbadge";
const STAND_KEY: &str = "custom_profile/collection/acrylic/acrylic_fixture";
const PLAIN_KEY: &str = "custom_profile/collection/misc/plain_fixture";
const SECOND_BADGE_KEY: &str = "custom_profile/collection/crash/second_canbadge";

/// Collection rows 1 (can badge), 2 (omikuji prefab), 3 (acrylic stand),
/// 4 (no collection type), 5 (a second can badge) and 6 (an omikuji slip
/// prefab that is not laid out).
fn collection_tables() -> JsonMasterData {
    let mut data = text_and_shape_tables();
    let row = |id: i32, kind: serde_json::Value, key: &str| {
        // Slip prefabs sit in a folder of their bundle.
        let (load, file) = key
            .split_once("/Prefabs/")
            .map(|(load, _)| (load, crate::omikuji::PREFAB_FILE))
            .unwrap_or_else(|| key.rsplit_once('/').unwrap());
        serde_json::json!({
            "id": id, "customProfileResourceType": "collection",
            "customProfileResourceCollectionType": kind,
            "resourceLoadType": "assetbundle", "resourceLoadVal": load, "fileName": file
        })
    };
    data.insert_value(
        "customProfileCollectionResources",
        serde_json::json!([
            row(1, "can_badge".into(), BADGE_KEY),
            row(
                2,
                "omikuji".into(),
                "lottery_game/new_year_2022/Prefabs/Omikuji"
            ),
            row(3, "acrylic_stand".into(), STAND_KEY),
            row(4, serde_json::Value::Null, PLAIN_KEY),
            row(5, "can_badge".into(), SECOND_BADGE_KEY),
            row(
                6,
                "omikuji".into(),
                "lottery_game/new_year_2030/Prefabs/Omikuji"
            ),
        ]),
    )
    .unwrap();
    data
}

/// Collections as `(layer, id, visible)`.
fn collection_card(elements: &[(i32, i32, bool)]) -> CustomProfileCard {
    serde_json::from_value(serde_json::json!({
        "collections": elements
            .iter()
            .map(|(layer, id, visible)| serde_json::json!({
                "objectData": visible_object(*layer, *visible), "id": id, "targetId": null
            }))
            .collect::<Vec<_>>()
    }))
    .unwrap()
}

fn prepared_keys(card: &CustomProfileCard, data: &JsonMasterData) -> Vec<(String, String)> {
    let mut keys = prepare_profile(card, None, data, "collections", "jp")
        .unwrap()
        .resources
        .into_iter()
        .map(|request| (request.resource.namespace, request.resource.key))
        .collect::<Vec<_>>();
    keys.sort();
    keys
}

fn collection_scene(card: &CustomProfileCard) -> crate::profile_scene::ResolvedProfileScene {
    compile_profile_scene(
        card,
        None,
        &collection_tables(),
        "collections",
        "jp",
        &(),
        BTreeMap::new(),
    )
    .unwrap()
}

fn image_material(
    scene: &crate::profile_scene::ResolvedProfileScene,
    game_layer: i32,
) -> (String, crate::Rect, crate::ImageMaterial) {
    let commands = game_layer_commands(scene, game_layer);
    assert_eq!(commands.len(), 1, "layer {game_layer}");
    match &commands[0].payload {
        crate::SemanticCommandPayload::Image {
            resource, material, ..
        } => (resource.key.clone(), commands[0].bounds, material.clone()),
        other => panic!("layer {game_layer} draws {other:?}"),
    }
}

fn badge_normal_map() -> ResourceKey {
    ResourceKey {
        namespace: "static".into(),
        key: "ui/sekai_badge_normal".into(),
    }
}

#[test]
fn can_badge_collections_draw_their_image_with_the_lit_badge_material() {
    let scene = collection_scene(&collection_card(&[
        (1, 1, true),
        (2, 3, true),
        (3, 4, true),
    ]));
    let (key, bounds, material) = image_material(&scene, 1);
    assert_eq!(key, BADGE_KEY);
    assert_eq!(
        material,
        crate::ImageMaterial::LitBadge {
            normal_map: badge_normal_map()
        }
    );
    // The image keeps the size and placement of a plain collection image.
    let (_, plain_bounds, _) = image_material(&scene, 3);
    assert_eq!(bounds, plain_bounds);
    for (game_layer, key) in [(2, STAND_KEY), (3, PLAIN_KEY)] {
        assert_eq!(
            image_material(&scene, game_layer),
            (key.into(), plain_bounds, crate::ImageMaterial::Plain),
            "layer {game_layer}"
        );
    }
}

#[test]
fn image_materials_round_trip_and_plain_images_serialise_without_one() {
    let scene = collection_scene(&collection_card(&[(1, 1, true), (2, 4, true)]));
    let command = |game_layer| game_layer_commands(&scene, game_layer)[0].clone();
    let json = |game_layer| serde_json::to_value(command(game_layer)).unwrap();
    assert!(json(2)["payload"].get("material").is_none());
    assert_eq!(
        json(1)["payload"]["material"],
        serde_json::json!({
            "kind": "lit_badge",
            "normal_map": { "namespace": "static", "key": "ui/sekai_badge_normal" }
        })
    );
    for game_layer in [1, 2] {
        let parsed: crate::SemanticCommandSource =
            serde_json::from_value(json(game_layer)).unwrap();
        assert_eq!(parsed, command(game_layer));
        let bytes =
            bincode::encode_to_vec(command(game_layer), bincode::config::standard()).unwrap();
        let (decoded, _): (crate::SemanticCommandSource, usize) =
            bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        assert_eq!(decoded, command(game_layer));
    }
}

/// Omikuji collections as `(layer, collection id, targetId)`.
fn omikuji_card(elements: &[(i32, i32, Option<i32>)]) -> CustomProfileCard {
    serde_json::from_value(serde_json::json!({
        "collections": elements
            .iter()
            .map(|(layer, id, target)| serde_json::json!({
                "objectData": visible_object(*layer, true), "id": id, "targetId": target
            }))
            .collect::<Vec<_>>()
    }))
    .unwrap()
}

/// [`collection_tables`] with `omikujis` rows 7 and 8.
fn omikuji_tables() -> JsonMasterData {
    let mut data = collection_tables();
    data.insert_value(
        "omikujis",
        serde_json::json!([
            crate::masterdata::tests::omikuji_row_value(7, "idol"),
            crate::masterdata::tests::omikuji_row_value(8, "street"),
        ]),
    )
    .unwrap();
    data
}

#[test]
fn omikuji_collections_resolve_to_the_slip_of_their_target_row() {
    let resolve = |data: &JsonMasterData, id: i32, target: Option<i32>| {
        let card = omikuji_card(&[(1, id, target)]);
        authored_resource(
            crate::profile_scene::ProfileElementRef::Collection(&card.collections[0]),
            data,
        )
    };
    let data = omikuji_tables();
    let AuthoredResource::Omikuji(plan) = resolve(&data, 2, Some(8)) else {
        panic!("{:?}", resolve(&data, 2, Some(8)));
    };
    assert_eq!(plan.prefab, crate::omikuji::PREFABS[0]);
    assert_eq!(plan.row, data.resolve_omikuji(8).unwrap());
    assert_eq!(plan.row.unit, "street");
    // The game does not build an omikuji whose row is missing, nor one
    // without a target.
    assert_eq!(resolve(&data, 2, Some(9)), AuthoredResource::MissingRow);
    assert_eq!(resolve(&data, 2, None), AuthoredResource::MissingRow);
    assert_eq!(
        resolve(&collection_tables(), 2, Some(7)),
        AuthoredResource::MissingRow
    );
    // A slip prefab that is not laid out draws nothing.
    assert_eq!(resolve(&data, 6, Some(7)), AuthoredResource::NotDrawn);
}

#[test]
fn the_shared_resolver_draws_omikuji_slips_and_requests_their_images_and_font() {
    let data = omikuji_tables();
    // A drawn slip, a stand, a slip prefab that is not laid out and a slip
    // without a target.
    let card = omikuji_card(&[(1, 2, Some(7)), (2, 3, None), (3, 6, Some(7)), (4, 2, None)]);
    let assets = |key: &str| ("assets".to_string(), key.to_string());
    assert_eq!(
        prepared_keys(&card, &data),
        [
            assets(STAND_KEY),
            assets("lottery_game/new_year_2022_material/bg_omikuji_VIRTUAL SINGER"),
            assets("lottery_game/new_year_2022_material/unsei_daikichi"),
        ]
    );
    let preparation = prepare_profile(&card, None, &data, "collections", "jp").unwrap();
    // The slip font and both fallback faces of the CN client.
    let ugui_families = ["FOT-Omikuji", "Noto Sans CJK SC", "Roboto"];
    assert_eq!(
        preparation.ugui_font_families,
        BTreeSet::from(ugui_families.map(String::from))
    );
    for family in ugui_families {
        assert!(preparation.font_families.contains(family), "{family}");
        // uGUI fonts are not SDF fonts: they have no font id and no
        // glyph layers.
        assert!(preparation.fonts.values().all(|sdf| sdf != family));
    }
    assert!(preparation.glyph_layers.is_empty());

    let scene = compile_profile_scene(
        &card,
        None,
        &data,
        "collections",
        "jp",
        &(),
        BTreeMap::new(),
    )
    .unwrap();
    // The slip is lowered exactly as the native snapshot lowers it: the
    // tables are CN tables, so the client fits the title backgrounds.
    let slip = scene
        .layers
        .iter()
        .find(|layer| layer.game_layer == 1)
        .unwrap();
    let visual = match authored_resource(
        crate::profile_scene::ProfileElementRef::Collection(&card.collections[0]),
        &data,
    ) {
        AuthoredResource::Omikuji(plan) => plan.visual(OmikujiClient::for_region("cn")),
        other => panic!("{other:?}"),
    };
    assert!(visual.fit_title_backgrounds);
    let source_key = crate::profile_scene::ordered_profile_elements(&card, "collections")
        .into_iter()
        .find(|element| element.layer_id == slip.id)
        .unwrap()
        .source_key;
    assert_eq!(
        game_layer_commands(&scene, 1)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>(),
        crate::omikuji::lower_omikuji(&source_key, slip.id, &visual)
    );
    for game_layer in [3, 4] {
        let commands = game_layer_commands(&scene, game_layer);
        assert_eq!(commands.len(), 1, "layer {game_layer}");
        assert!(
            matches!(
                commands[0].payload,
                crate::SemanticCommandPayload::Composite { .. }
            ),
            "layer {game_layer}"
        );
    }
    assert_eq!(image_material(&scene, 2).0, STAND_KEY);
}

#[test]
fn the_omikuji_client_follows_the_region_of_the_tables() {
    let card = omikuji_card(&[(1, 2, Some(7))]);
    let fitted = |data: &JsonMasterData, locale: &str| {
        let scene = compile_profile_scene(
            &card,
            None,
            data,
            "collections",
            locale,
            &(),
            BTreeMap::new(),
        )
        .unwrap();
        game_layer_commands(&scene, 1)
            .into_iter()
            .filter_map(|command| match &command.payload {
                crate::SemanticCommandPayload::UguiText(source) => source
                    .backdrop
                    .as_ref()
                    .map(|backdrop| backdrop.fit_padding),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let mut jp = JsonMasterData::new("jp");
    for (name, table) in [
        (
            "customProfileCollectionResources",
            serde_json::json!([{
                "id": 2, "customProfileResourceType": "collection",
                "customProfileResourceCollectionType": "omikuji",
                "resourceLoadType": "assetbundle",
                "resourceLoadVal": "lottery_game/new_year_2022", "fileName": "Prefabs/Omikuji"
            }]),
        ),
        (
            "omikujis",
            serde_json::json!([crate::masterdata::tests::omikuji_row_value(7, "idol")]),
        ),
    ] {
        jp.insert_value(name, table).unwrap();
    }
    let fit = Some(crate::omikuji::TITLE_BACKGROUND_FIT_PADDING);
    // The tables' region decides, not the locale the page is shown in.
    assert_eq!(fitted(&omikuji_tables(), "ja-JP"), [fit; 3]);
    assert_eq!(fitted(&jp, "zh-CN"), [None; 3]);
}

#[test]
fn the_badge_normal_map_is_requested_only_for_a_visible_can_badge() {
    let data = collection_tables();
    let assets = |key: &str| ("assets".to_string(), key.to_string());
    // Plain collections and a hidden badge need no normal map.
    assert_eq!(
        prepared_keys(&collection_card(&[(1, 3, true), (2, 1, false)]), &data),
        [assets(STAND_KEY)]
    );
    // Two visible badges share one normal map.
    assert_eq!(
        prepared_keys(
            &collection_card(&[(1, 1, true), (2, 5, true), (3, 4, true)]),
            &data
        ),
        [
            assets(BADGE_KEY),
            assets(SECOND_BADGE_KEY),
            assets(PLAIN_KEY),
            ("static".to_string(), "ui/sekai_badge_normal".to_string()),
        ]
    );
    let preparation = prepare_profile(
        &collection_card(&[(1, 1, true)]),
        None,
        &data,
        "collections",
        "jp",
    )
    .unwrap();
    let normal_map = preparation
        .resources
        .iter()
        .find(|request| request.resource == badge_normal_map())
        .unwrap();
    assert_eq!(
        normal_map.fallback,
        ResourceMetric {
            width: 256.0,
            height: 256.0
        }
    );
}
