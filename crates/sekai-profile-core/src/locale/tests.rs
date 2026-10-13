use super::*;

#[test]
fn comment_title_is_resolved_for_every_supported_region() {
    for (region, expected) in [
        ("cn", "个性签名"),
        ("jp", "ひとこと"),
        ("tw", "個性簽名"),
        ("en", "Bio"),
        ("kr", "한마디"),
    ] {
        assert_eq!(
            resolve(region, "custom_profile.general.comment.title"),
            Some(expected.to_string()),
            "region={region}"
        );
    }
    assert_eq!(
        resolve("unknown", "custom_profile.general.comment.title"),
        None
    );
    assert_eq!(resolve("cn", "unknown.key"), None);
}

#[test]
fn locale_aliases_and_every_general_label_are_complete() {
    for (region, alias) in [
        ("cn", "zh-CN"),
        ("cn", "sc"),
        ("jp", "ja-JP"),
        ("tw", "zh-TW"),
        ("tw", "tc"),
        ("en", "en-US"),
        ("en", "world"),
        ("kr", "ko-KR"),
    ] {
        for key in GENERAL_LOCALIZATION_KEYS {
            assert_eq!(resolve(alias, key), resolve(region, key), "{alias}:{key}");
            assert!(resolve(alias, key).is_some(), "missing {alias}:{key}");
        }
    }
}

#[test]
fn profile_localization_demands_are_exact_deduplicated_and_region_scoped() {
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "generals": [
            { "objectData": object(1), "type": 4 },
            { "objectData": object(2), "type": 16 },
            { "objectData": object(3), "type": 16 },
            { "objectData": object(4), "type": 13 }
        ]
    }))
    .unwrap();
    let demands = profile_localization_demands(&card, "en", "en-US");
    assert_eq!(
        demands
            .iter()
            .map(|value| value.key.as_str())
            .collect::<Vec<_>>(),
        vec![
            "custom_profile.general.comment.title",
            "custom_profile.general.music.clear",
            "custom_profile.general.music.full_combo",
            "custom_profile.general.music.all_perfect",
            "custom_profile.general.music.difficulty.easy",
            "custom_profile.general.music.difficulty.normal",
            "custom_profile.general.music.difficulty.hard",
            "custom_profile.general.music.difficulty.expert",
            "custom_profile.general.music.difficulty.master",
            "custom_profile.general.music.difficulty.append",
        ]
    );
    assert!(demands.iter().all(|value| value.region == "en"));
    assert!(demands.iter().all(|value| value.locale == "en-US"));
}

#[test]
fn localization_demands_cover_every_key_consumed_by_each_general_recipe() {
    let localized_text = GENERAL_LOCALIZATION_KEYS
        .iter()
        .map(|key| ((*key).into(), (*key).into()))
        .collect();
    let snapshot = crate::profile_scene::ProfileComponentSnapshot {
        locale: "en-US".into(),
        region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
        localized_text,
        deck_members: vec![crate::profile_scene::CardVisualSnapshot {
            card_id: 1,
            after_training: false,
            master_rank: 0,
            level: 1,
            rarity: "rarity_1".into(),
            attribute: "cool".into(),
            image: crate::profile_scene::ComponentImageSnapshot {
                source_field: "userProfile.deckMembers".into(),
                source_id: "1".into(),
                descriptor: None,
            },
        }],
        ..crate::profile_scene::ProfileComponentSnapshot::default()
    };
    for general_type in crate::general_recipe::SUPPORTED_GENERAL_TYPES {
        let recipe = crate::general_recipe::build_general_recipe(
            general_type,
            crate::StableId(general_type as u64),
            &format!("general:{general_type}"),
            &snapshot,
        )
        .unwrap()
        .unwrap();
        let consumed = recipe
            .nodes
            .iter()
            .filter_map(|node| match &node.payload {
                crate::general_recipe::GeneralRecipePayload::Text {
                    source: crate::TextSource::Localized { key, .. },
                    ..
                } => Some(key.as_str()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            consumed,
            general_localization_keys(general_type)
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            "general type={general_type}"
        );
    }
}

#[test]
fn card_level_is_demanded_by_deck_not_leader_member() {
    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "generals": [
            { "objectData": object(1), "type": 3 },
            { "objectData": object(2), "type": 5 }
        ]
    }))
    .unwrap();
    let demands = profile_localization_demands(&card, "cn", "zh-CN");
    assert_eq!(
        demands,
        vec![LocalizationDemand {
            region: "cn".into(),
            locale: "zh-CN".into(),
            key: "custom_profile.general.card_level".into(),
        }]
    );

    let leader_only: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "generals": [{ "objectData": object(1), "type": 5 }]
    }))
    .unwrap();
    assert!(profile_localization_demands(&leader_only, "cn", "zh-CN").is_empty());
}

fn object(layer: i32) -> serde_json::Value {
    serde_json::json!({
        "layer": layer,
        "lock": false,
        "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
        "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
        "scale": { "x": 1.0, "y": 1.0, "z": 1.0 },
        "visible": true
    })
}
