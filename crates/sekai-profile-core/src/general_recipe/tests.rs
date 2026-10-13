use std::collections::BTreeMap;

use crate::profile_scene::{
    MusicDifficultySnapshot, MusicResultsSnapshot, ProfileComponentSnapshot,
};
use crate::{FontRole, StableId, TextSource};

#[test]
fn all_supported_general_types_resolve_from_one_recipe_entrypoint() {
    let snapshot = ProfileComponentSnapshot {
        locale: "en-US".into(),
        region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
        localized_text: crate::locale::GENERAL_LOCALIZATION_KEYS
            .iter()
            .map(|key| ((*key).into(), (*key).into()))
            .collect(),
        ..ProfileComponentSnapshot::default()
    };
    for general_type in super::SUPPORTED_GENERAL_TYPES {
        let recipe = super::build_general_recipe(
            general_type,
            StableId(general_type as u64),
            &format!("general:{general_type}"),
            &snapshot,
        )
        .unwrap()
        .unwrap();
        assert_eq!(recipe.general_type, general_type);
        assert!(recipe.nodes.iter().all(|node| {
            !matches!(node.payload, super::GeneralRecipePayload::Text { .. })
                || node.clips.is_empty()
        }));
    }
    for unsupported in [0, 1, 7, 8, 19] {
        assert!(
            super::build_general_recipe(unsupported, StableId(0), "unsupported", &snapshot,)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn identity_general_recipe_is_the_exact_native_player_name_and_comment_sequence() {
    let snapshot = ProfileComponentSnapshot {
        locale: "en-US".into(),
        region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
        localized_text: BTreeMap::from([(
            "custom_profile.general.comment.title".into(),
            "Bio".into(),
        )]),
        user_name: "Player".into(),
        word: "Hello".into(),
        ..ProfileComponentSnapshot::default()
    };
    let name = super::build_general_recipe(13, StableId(1), "general:13", &snapshot)
        .unwrap()
        .unwrap();
    assert_eq!(name.nodes.len(), 2);
    assert!(matches!(
        &name.nodes[0].payload,
        super::GeneralRecipePayload::Shape {
            geometry: super::GeneralGeometry::RoundedRect { radius: [8.0, 8.0] },
            fill: super::GeneralFill::Solid([0.922, 0.922, 0.941, 1.0]),
            stroke: None,
        }
    ));
    assert_eq!(
        name.nodes[0].bounds,
        crate::Rect {
            x: -273.0,
            y: -32.0,
            width: 544.0,
            height: 64.0
        }
    );
    assert!(matches!(
        &name.nodes[1].payload,
        super::GeneralRecipePayload::Text {
            source: TextSource::ProfileField { field, value },
            font_role: FontRole::RegionFontId(1),
            font_size: 34.0,
            color: [0.2, 0.2, 0.2, 1.0],
            align: super::GeneralTextAlign::Left,
            ..
        } if field == "user.name" && value == "Player"
    ));
    assert!(
        name.nodes[1].clips.is_empty(),
        "measured label bounds are draw anchors, not a text clipping box"
    );
    let lowered_name =
        crate::profile_scene::lower_identity_general(13, StableId(1), "general:13", &snapshot)
            .unwrap()
            .unwrap();
    assert!(matches!(
        &lowered_name.commands[1].payload,
        crate::SemanticCommandPayload::Text {
            size,
            alignment: 1,
            max_width: None,
            max_height: None,
            ..
        } if (*size - 17.0).abs() < 1e-6
    ));
    assert_eq!(
        lowered_name.commands[1].matrix[4],
        name.nodes[1].bounds.x + name.nodes[1].bounds.width / 2.0
    );
    let comment = super::build_general_recipe(4, StableId(2), "general:4", &snapshot)
        .unwrap()
        .unwrap();
    assert_eq!(comment.nodes.len(), 3);
    assert!(matches!(
        &comment.nodes[1].payload,
        super::GeneralRecipePayload::Text {
            source: TextSource::Localized { key, locale, value },
            color: [0.53, 0.53, 0.53, 1.0],
            ..
        } if key == "custom_profile.general.comment.title" && locale == "en-US" && value == "Bio"
    ));
    assert!(matches!(
        &comment.nodes[2].payload,
        super::GeneralRecipePayload::Text {
            source: TextSource::ProfileField { field, value },
            color: [0.2, 0.2, 0.2, 1.0],
            line_spacing: 6.0,
            render_baseline: Some(baseline),
            ..
        } if field == "userProfile.word" && value == "Hello"
            && (*baseline + 13.5).abs() < 1e-6
    ));
    assert!(
        comment.nodes[2].clips.is_empty(),
        "General text bounds may guide wrapping but must never clip glyphs"
    );

    let lowered =
        crate::profile_scene::lower_identity_general(4, StableId(2), "general:4", &snapshot)
            .unwrap()
            .unwrap();
    assert_eq!(
        lowered
            .commands
            .iter()
            .map(|value| value.id)
            .collect::<Vec<_>>(),
        comment
            .nodes
            .iter()
            .map(|value| value.id)
            .collect::<Vec<_>>()
    );
    for (command, node) in lowered.commands.iter().zip(&comment.nodes) {
        assert_eq!(command.bounds, node.bounds);
        assert_eq!(command.control_bindings, node.control_bindings);
    }
    assert_eq!(
        lowered.commands[2].render_placement,
        Some(crate::TextRenderPlacementSource {
            anchor_x: -comment.nodes[2].bounds.width / 4.0,
            baseline: Some(-13.5),
        })
    );
    assert!(matches!(
        &lowered.commands[2].payload,
        crate::SemanticCommandPayload::Text {
            max_width: Some(width),
            max_height: None,
            ..
        } if *width == comment.nodes[2].bounds.width
    ));
}

#[test]
fn stats_general_recipe_preserves_native_divider_and_two_line_superstar_text() {
    let snapshot = ProfileComponentSnapshot {
        locale: "en-US".into(),
        region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
        localized_text: BTreeMap::from([
            (
                "custom_profile.general.total_power.title".into(),
                "Power".into(),
            ),
            (
                "custom_profile.general.multiplayer_live.title".into(),
                "Multiplayer".into(),
            ),
            (
                "custom_profile.general.multiplayer_live.mvp".into(),
                "Region MVP".into(),
            ),
            (
                "custom_profile.general.multiplayer_live.superstar".into(),
                "Region Super Star".into(),
            ),
            (
                "custom_profile.general.count_times".into(),
                "{count} times".into(),
            ),
        ]),
        total_power: 123_456,
        mvp: 7,
        superstar: 8,
        ..ProfileComponentSnapshot::default()
    };
    let power = super::build_general_recipe(2, StableId(3), "general:2", &snapshot)
        .unwrap()
        .unwrap();
    assert_eq!(power.nodes.len(), 3);
    assert!(matches!(
        &power.nodes[1].payload,
        super::GeneralRecipePayload::Text {
            source: TextSource::Authored { value },
            font_role: FontRole::RegionFontId(1),
            font_size: 34.0,
            color: [0.67, 0.67, 0.67, 1.0],
            ..
        } if value == "|"
    ));

    let multiplayer = super::build_general_recipe(9, StableId(4), "general:9", &snapshot)
        .unwrap()
        .unwrap();
    assert_eq!(
        multiplayer
            .nodes
            .iter()
            .map(|node| node.role.as_str())
            .collect::<Vec<_>>(),
        vec![
            "divider",
            "title",
            "mvp-background",
            "mvp-label",
            "mvp-count",
            "superstar-background",
            "superstar-upper",
            "superstar-lower",
            "superstar-count",
        ]
    );
    assert!(matches!(
        &multiplayer.nodes[3].payload,
        super::GeneralRecipePayload::Text {
            source: TextSource::Localized { key, locale, value },
            ..
        } if key == "custom_profile.general.multiplayer_live.mvp"
            && locale == "en-US"
            && value == "Region MVP"
    ));
    assert!(matches!(
        &multiplayer.nodes[3].payload,
        super::GeneralRecipePayload::Text { font_size, .. }
            if (*font_size - 30.8).abs() < 1e-5
    ));
    assert!(matches!(
        &multiplayer.nodes[6].payload,
        super::GeneralRecipePayload::Text {
            source: TextSource::Localized { key, locale, value },
            font_size: 18.0,
            ..
        } if key == "custom_profile.general.multiplayer_live.superstar"
            && locale == "en-US"
            && value == "Region Super"
    ));
    assert!(matches!(
        &multiplayer.nodes[7].payload,
        super::GeneralRecipePayload::Text {
            source: TextSource::Localized { key, locale, value },
            font_size: 18.0,
            ..
        } if key == "custom_profile.general.multiplayer_live.superstar"
            && locale == "en-US"
            && value == "Star"
    ));
}

#[test]
fn detailed_music_recipe_expands_native_three_by_six_sequence_without_backend_loops() {
    let localized_text = crate::locale::GENERAL_LOCALIZATION_KEYS
        .iter()
        .map(|key| ((*key).into(), (*key).into()))
        .collect();
    let snapshot = ProfileComponentSnapshot {
        locale: "en-US".into(),
        region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
        localized_text,
        music_results: Some(MusicResultsSnapshot {
            easy: MusicDifficultySnapshot {
                clear: 1,
                full_combo: 2,
                all_perfect: 3,
            },
            normal: MusicDifficultySnapshot {
                clear: 4,
                full_combo: 5,
                all_perfect: 6,
            },
            ..MusicResultsSnapshot::default()
        }),
        ..ProfileComponentSnapshot::default()
    };
    let recipe = super::build_general_recipe(12, StableId(5), "general:12", &snapshot)
        .unwrap()
        .unwrap();
    assert_eq!(recipe.nodes.len(), 60);
    assert_eq!(
        recipe
            .nodes
            .iter()
            .take(5)
            .map(|node| node.role.as_str())
            .collect::<Vec<_>>(),
        vec![
            "clear-heading-background",
            "clear-heading",
            "clear-easy-label-background",
            "clear-easy-label",
            "clear-easy-count",
        ]
    );
    assert!(matches!(
        &recipe.nodes[4].payload,
        super::GeneralRecipePayload::Text {
            source: TextSource::ProfileField { field, value },
            ..
        } if field == "userMusicDifficultyClearCount.easy.liveClear" && value == "1"
    ));
    assert!(matches!(
        &recipe.nodes[24].payload,
        super::GeneralRecipePayload::Text {
            source: TextSource::ProfileField { field, value },
            ..
        } if field == "userMusicDifficultyClearCount.easy.fullCombo" && value == "2"
    ));
}

#[test]
fn tabbed_music_recipe_binds_counts_selected_background_and_text_color_to_one_control() {
    let localized_text = crate::locale::GENERAL_LOCALIZATION_KEYS
        .iter()
        .map(|key| ((*key).into(), (*key).into()))
        .collect();
    let snapshot = ProfileComponentSnapshot {
        locale: "en-US".into(),
        region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
        localized_text,
        music_results: Some(MusicResultsSnapshot {
            easy: MusicDifficultySnapshot {
                clear: 1,
                full_combo: 2,
                all_perfect: 3,
            },
            ..MusicResultsSnapshot::default()
        }),
        ..ProfileComponentSnapshot::default()
    };
    let recipe = super::build_general_recipe(16, StableId(6), "general:16", &snapshot)
        .unwrap()
        .unwrap();
    assert_eq!(recipe.controls.len(), 1);
    assert!(matches!(
        &recipe.controls[0].state,
        crate::profile_scene::ComponentControlState::Tabs { options, active }
            if options == &["clear", "full_combo", "all_perfect"] && active == "clear"
    ));
    let control_id = recipe.controls[0].id;
    for option in ["clear", "full_combo", "all_perfect"] {
        assert_eq!(
            recipe.nodes.iter().filter(|node| node.control_bindings.iter().any(|binding| {
                matches!(binding, crate::CommandControlBinding::TabOption { control_id: id, value }
                    if *id == control_id && value == option)
            })).count(),
            10,
            "option={option} must own six counts, one selected background and three state-colored tab labels"
        );
    }
    assert_eq!(recipe.interaction_regions.len(), 3);
    for (region, option) in
        recipe
            .interaction_regions
            .iter()
            .zip(["clear", "full_combo", "all_perfect"])
    {
        assert!(
            region.control_bindings.is_empty(),
            "inactive tab {option} must remain hittable"
        );
        assert_eq!(
            region.resolved_data.get("action"),
            Some(&crate::ParameterValue::Text("set_tab".into()))
        );
        assert_eq!(
            region.resolved_data.get("value"),
            Some(&crate::ParameterValue::Text(option.into()))
        );
    }
}

#[test]
fn challenge_rank_and_player_avatar_are_owned_by_the_shared_recipe() {
    fn image(
        field: &str,
        id: &str,
        namespace: &str,
        key: &str,
        width: f32,
        height: f32,
    ) -> crate::profile_scene::ComponentImageSnapshot {
        crate::profile_scene::ComponentImageSnapshot {
            source_field: field.into(),
            source_id: id.into(),
            descriptor: Some(crate::profile_scene::ResourceDescriptor {
                resource: crate::ResourceKey {
                    namespace: namespace.into(),
                    key: key.into(),
                },
                natural_width: width,
                natural_height: height,
                provenance: BTreeMap::new(),
            }),
        }
    }

    let snapshot = ProfileComponentSnapshot {
        locale: "en-US".into(),
        region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
        localized_text: BTreeMap::from([
            (
                "custom_profile.general.challenge_live.title".into(),
                "Challenge Show".into(),
            ),
            (
                "custom_profile.general.challenge_live.solo".into(),
                "Solo".into(),
            ),
            (
                "custom_profile.general.player_level.label".into(),
                "Rank".into(),
            ),
        ]),
        user_rank: 321,
        challenge_score: 987_654,
        challenge_character_id: 7,
        challenge_avatar: Some(image(
            "userProfile.challengeLiveSoloResult.characterId",
            "7",
            "static",
            "chara_avatar/chara07_02",
            76.0,
            76.0,
        )),
        player_avatar: Some(image(
            "userProfile.leaderCard",
            "1007",
            "assets",
            "thumbnail/chara/demo_after_training",
            940.0,
            530.0,
        )),
        ..ProfileComponentSnapshot::default()
    };

    for (general_type, expected_roles) in [
        (
            10,
            vec![
                "divider",
                "title",
                "solo-background",
                "solo-label",
                "challenge-avatar",
                "challenge-score",
            ],
        ),
        (17, vec!["background", "rank-icon", "label", "rank"]),
        (18, vec!["avatar-background", "avatar", "avatar-border"]),
    ] {
        let recipe = super::build_general_recipe(
            general_type,
            StableId(general_type as u64),
            &format!("general:{general_type}"),
            &snapshot,
        )
        .unwrap()
        .expect("shared recipe must own this General type");
        assert_eq!(
            recipe
                .nodes
                .iter()
                .map(|node| node.role.as_str())
                .collect::<Vec<_>>(),
            expected_roles
        );
    }

    let challenge = super::build_general_recipe(10, StableId(10), "general:10", &snapshot)
        .unwrap()
        .unwrap();
    assert_eq!(
        challenge.nodes[4].bounds,
        crate::Rect {
            x: -268.0,
            y: -14.0,
            width: 92.0,
            height: 92.0,
        }
    );
    assert!(matches!(
        &challenge.nodes[4].payload,
        super::GeneralRecipePayload::Image {
            source: super::GeneralImageSource::WholeImageImplicit,
            sampling: super::GeneralImageSampling::Nearest,
            source_constraint: super::GeneralImageSourceConstraint::Implicit,
            ..
        }
    ));
    assert!(matches!(
        challenge.nodes[4].clips.as_slice(),
        [super::GeneralClip::Ellipse { bounds, .. }] if *bounds == challenge.nodes[4].bounds
    ));
    assert_eq!(challenge.interaction_regions.len(), 1);
    assert_eq!(
        challenge.interaction_regions[0]
            .resolved_data
            .get("character_id"),
        Some(&crate::ParameterValue::I64(7))
    );

    let rank = super::build_general_recipe(17, StableId(17), "general:17", &snapshot)
        .unwrap()
        .unwrap();
    assert_eq!(rank.nodes[2].bounds.x, -54.0);
    assert_eq!(rank.nodes[2].bounds.width, 90.0);
    let lowered_rank =
        crate::profile_scene::lower_identity_general(17, StableId(17), "general:17", &snapshot)
            .unwrap()
            .unwrap();
    assert!(matches!(
        &lowered_rank.commands[3].payload,
        crate::SemanticCommandPayload::Text {
            size,
            alignment: 4,
            ..
        } if (*size - 13.0).abs() < 1e-6
    ));
    let placement = lowered_rank.commands[3].render_placement.unwrap();
    assert_eq!(placement.anchor_x, 45.0);
    assert!((placement.baseline.unwrap() - 4.55).abs() < 1e-5);

    let avatar = super::build_general_recipe(18, StableId(18), "general:18", &snapshot)
        .unwrap()
        .unwrap();
    assert!(matches!(
        &avatar.nodes[1].payload,
        super::GeneralRecipePayload::Image {
            source: super::GeneralImageSource::Pixels(crate::Rect {
                x: 205.0,
                y: 0.0,
                width: 530.0,
                height: 530.0,
            }),
            source_constraint: super::GeneralImageSourceConstraint::Fast,
            ..
        }
    ));
    assert_eq!(
        avatar.nodes[2].id,
        super::recipe_node_id("general:18", "avatar-border", 2)
    );
}

#[test]
fn leader_member_recipe_preserves_native_cover_stack_and_availability_branch() {
    let card = crate::profile_scene::CardVisualSnapshot {
        card_id: 2001,
        after_training: false,
        master_rank: 3,
        level: 50,
        rarity: "rarity_4".into(),
        attribute: "cool".into(),
        image: crate::profile_scene::ComponentImageSnapshot {
            source_field: "userProfile.leaderCard".into(),
            source_id: "2001".into(),
            descriptor: Some(crate::profile_scene::ResourceDescriptor {
                resource: crate::ResourceKey {
                    namespace: "assets".into(),
                    key: "character/member_small/demo/card_normal".into(),
                },
                natural_width: 1200.0,
                natural_height: 600.0,
                provenance: BTreeMap::new(),
            }),
        },
    };
    let recipe = super::build_general_recipe(
        5,
        StableId(5),
        "general:5",
        &ProfileComponentSnapshot {
            leader_card: Some(card.clone()),
            ..ProfileComponentSnapshot::default()
        },
    )
    .unwrap()
    .expect("type 5 must not require a font");
    assert_eq!(
        recipe
            .nodes
            .iter()
            .map(|node| node.role.as_str())
            .collect::<Vec<_>>(),
        vec![
            "leader-artwork",
            "leader-frame",
            "leader-attribute",
            "leader-star-0",
            "leader-star-1",
            "leader-star-2",
            "leader-star-3",
            "leader-master-rank",
        ]
    );
    assert!(matches!(
        &recipe.nodes[0].payload,
        super::GeneralRecipePayload::Image {
            source: super::GeneralImageSource::Pixels(source),
            source_constraint: super::GeneralImageSourceConstraint::Fast,
            ..
        } if source.x == 67.924_5
            && source.y == 0.0
            && source.width == 1064.151
            && source.height == 600.0
    ));
    assert_eq!(recipe.nodes[3].bounds.y, 39.0);
    assert_eq!(recipe.nodes[6].bounds.y, 183.0);
    assert_eq!(recipe.interaction_regions.len(), 1);
    assert_eq!(
        recipe.interaction_regions[0].resolved_data.get("level"),
        Some(&crate::ParameterValue::I64(50))
    );

    let mut unavailable = card;
    unavailable.image.descriptor = None;
    let fallback = super::build_general_recipe(
        5,
        StableId(5),
        "general:5",
        &ProfileComponentSnapshot {
            leader_card: Some(unavailable),
            ..ProfileComponentSnapshot::default()
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(fallback.nodes.len(), 1);
    assert_eq!(fallback.nodes[0].role, "leader-artwork");
    assert_eq!(fallback.nodes[0].id, recipe.nodes[0].id);
    assert!(matches!(
        fallback.nodes[0].payload,
        super::GeneralRecipePayload::Shape { .. }
    ));
}

#[test]
fn leader_member_recipe_does_not_clamp_native_rarity_count() {
    let mut card = crate::profile_scene::CardVisualSnapshot {
        card_id: 2001,
        after_training: false,
        master_rank: 0,
        level: 1,
        rarity: "rarity_5".into(),
        attribute: "cool".into(),
        image: crate::profile_scene::ComponentImageSnapshot {
            source_field: "userProfile.leaderCard".into(),
            source_id: "2001".into(),
            descriptor: None,
        },
    };
    card.image.descriptor = Some(crate::profile_scene::ResourceDescriptor {
        resource: crate::ResourceKey {
            namespace: "assets".into(),
            key: "character/member_small/demo/card_normal".into(),
        },
        natural_width: 1200.0,
        natural_height: 600.0,
        provenance: BTreeMap::new(),
    });
    let recipe = super::build_general_recipe(
        5,
        StableId(5),
        "general:5",
        &ProfileComponentSnapshot {
            leader_card: Some(card),
            ..ProfileComponentSnapshot::default()
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        recipe
            .nodes
            .iter()
            .filter(|node| node.role.starts_with("leader-star-"))
            .count(),
        5
    );
    assert_eq!(
        recipe
            .nodes
            .iter()
            .find(|node| node.role == "leader-star-4")
            .unwrap()
            .bounds
            .y,
        231.0
    );
}

#[test]
fn deck_recipe_preserves_five_native_slots_crop_order_and_localized_sdf_level() {
    let card = crate::profile_scene::CardVisualSnapshot {
        card_id: 3001,
        after_training: true,
        master_rank: 2,
        level: 60,
        rarity: "rarity_4".into(),
        attribute: "cute".into(),
        image: crate::profile_scene::ComponentImageSnapshot {
            source_field: "userProfile.deckMembers".into(),
            source_id: "3001".into(),
            descriptor: Some(crate::profile_scene::ResourceDescriptor {
                resource: crate::ResourceKey {
                    namespace: "assets".into(),
                    key: "character/member_cutout/demo/after_training".into(),
                },
                natural_width: 600.0,
                natural_height: 576.0,
                provenance: BTreeMap::new(),
            }),
        },
    };
    let mut missing = card.clone();
    missing.card_id = 3002;
    missing.image.source_id = "3002".into();
    missing.image.descriptor = None;
    let recipe = super::build_general_recipe(
        3,
        StableId(3),
        "general:3",
        &ProfileComponentSnapshot {
            locale: "en-US".into(),
            region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
            localized_text: BTreeMap::from([(
                "custom_profile.general.card_level".into(),
                "Lv.{level}".into(),
            )]),
            deck_members: vec![card, missing],
            ..ProfileComponentSnapshot::default()
        },
    )
    .unwrap()
    .unwrap();

    let slot0 = recipe
        .nodes
        .iter()
        .filter(|node| node.role.starts_with("deck-slot-0-"))
        .collect::<Vec<_>>();
    assert_eq!(
        slot0
            .iter()
            .map(|node| node.role.as_str())
            .collect::<Vec<_>>(),
        vec![
            "deck-slot-0-artwork",
            "deck-slot-0-level-bar",
            "deck-slot-0-level",
            "deck-slot-0-frame",
            "deck-slot-0-attribute",
            "deck-slot-0-star-0",
            "deck-slot-0-star-1",
            "deck-slot-0-star-2",
            "deck-slot-0-star-3",
            "deck-slot-0-master-rank",
        ]
    );
    assert_eq!(slot0[0].bounds.width, 148.078_13);
    assert_eq!(slot0[0].bounds.height, 243.0);
    assert!(matches!(
        slot0[0].payload,
        super::GeneralRecipePayload::Image {
            source: super::GeneralImageSource::Pixels(crate::Rect {
                x: 144.0,
                y: 0.0,
                width: 312.0,
                height: 512.0,
            }),
            source_constraint: super::GeneralImageSourceConstraint::Fast,
            ..
        }
    ));
    assert!(matches!(
        &slot0[1].payload,
        super::GeneralRecipePayload::Image { resource, tint, .. }
            if resource.namespace == "static"
                && resource.key == "card/bg_base_wh"
                && *tint == [68.0 / 255.0, 68.0 / 255.0, 102.0 / 255.0, 1.0]
    ));
    assert!(matches!(
        &slot0[2].payload,
        super::GeneralRecipePayload::Text {
            source: crate::TextSource::Localized { key, value, .. },
            font_role: crate::FontRole::RegionFontId(1),
            ..
        } if key == "custom_profile.general.card_level" && value == "Lv.60"
    ));
    assert!(slot0[2].clips.is_empty());
    assert!(slot0
        .iter()
        .filter(|node| !matches!(node.payload, super::GeneralRecipePayload::Text { .. }))
        .all(|node| matches!(node.clips.as_slice(), [super::GeneralClip::Rect { .. }])));

    for slot in 1..5 {
        assert!(recipe.nodes.iter().any(|node| {
            node.role == format!("deck-slot-{slot}-artwork")
                && matches!(node.payload, super::GeneralRecipePayload::Shape { .. })
        }));
    }
    assert_eq!(recipe.interaction_regions.len(), 2);
}

fn overlay_card(
    master_rank: i32,
    level: i32,
    trained: bool,
) -> crate::profile_scene::CardVisualSnapshot {
    crate::profile_scene::CardVisualSnapshot {
        card_id: 5001,
        after_training: trained,
        master_rank,
        level,
        rarity: "rarity_3".into(),
        attribute: "happy".into(),
        image: crate::profile_scene::ComponentImageSnapshot {
            source_field: "userProfile.deckMembers".into(),
            source_id: "5001".into(),
            descriptor: Some(crate::profile_scene::ResourceDescriptor {
                resource: crate::ResourceKey {
                    namespace: "assets".into(),
                    key: "character/member_cutout/demo/normal".into(),
                },
                natural_width: 600.0,
                natural_height: 576.0,
                provenance: BTreeMap::new(),
            }),
        },
    }
}

fn deck_snapshot(
    deck_members: Vec<crate::profile_scene::CardVisualSnapshot>,
) -> ProfileComponentSnapshot {
    ProfileComponentSnapshot {
        locale: "en-US".into(),
        region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
        localized_text: BTreeMap::from([(
            super::CARD_LEVEL_LOCALIZATION_KEY.into(),
            "Lv.{level}".into(),
        )]),
        deck_members,
        ..ProfileComponentSnapshot::default()
    }
}

fn image_key<'a>(recipe: &'a super::GeneralRecipe, role: &str) -> Option<&'a str> {
    recipe
        .nodes
        .iter()
        .find(|node| node.role == role)
        .and_then(|node| match &node.payload {
            super::GeneralRecipePayload::Image { resource, .. } => Some(resource.key.as_str()),
            _ => None,
        })
}

#[test]
fn card_level_captions_pad_single_digit_levels_with_two_spaces() {
    assert_eq!(super::card_level_text("Lv.{level}", 5), "Lv.  5");
    assert_eq!(super::card_level_text("Lv.{level}", 9), "Lv.  9");
    assert_eq!(super::card_level_text("Lv.{level}", 10), "Lv.10");
    let recipe = super::build_general_recipe(
        3,
        StableId(3),
        "general:3",
        &deck_snapshot(vec![overlay_card(1, 5, false)]),
    )
    .unwrap()
    .unwrap();
    let level = recipe
        .nodes
        .iter()
        .find(|node| node.role == "deck-slot-0-level")
        .unwrap();
    assert!(matches!(
        &level.payload,
        super::GeneralRecipePayload::Text {
            source: crate::TextSource::Localized { value, .. },
            ..
        } if value == "Lv.  5"
    ));
}

#[test]
fn card_overlays_hide_rank_zero_and_draw_large_master_rank_badges() {
    let recipe = super::build_general_recipe(
        3,
        StableId(3),
        "general:3",
        &deck_snapshot(vec![overlay_card(0, 60, true), overlay_card(3, 60, false)]),
    )
    .unwrap()
    .unwrap();
    assert_eq!(image_key(&recipe, "deck-slot-0-master-rank"), None);
    assert_eq!(
        image_key(&recipe, "deck-slot-1-master-rank"),
        Some("card/masterRank_L_3")
    );
    assert_eq!(
        image_key(&recipe, "deck-slot-0-star-0"),
        Some("card/rarity_star_afterTraining")
    );
    assert_eq!(
        image_key(&recipe, "deck-slot-1-star-0"),
        Some("card/rarity_star_normal")
    );

    let leader = super::build_general_recipe(
        5,
        StableId(5),
        "general:5",
        &ProfileComponentSnapshot {
            leader_card: Some(overlay_card(0, 60, false)),
            ..ProfileComponentSnapshot::default()
        },
    )
    .unwrap()
    .unwrap();
    assert!(leader
        .nodes
        .iter()
        .all(|node| node.role != "leader-master-rank"));
}

#[test]
fn challenge_live_without_a_record_hides_the_character_avatar() {
    let snapshot = ProfileComponentSnapshot {
        locale: "en-US".into(),
        region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
        localized_text: BTreeMap::from([
            (
                "custom_profile.general.challenge_live.title".into(),
                "Challenge Show".into(),
            ),
            (
                "custom_profile.general.challenge_live.solo".into(),
                "Solo".into(),
            ),
        ]),
        ..ProfileComponentSnapshot::default()
    };
    let recipe = super::build_general_recipe(10, StableId(10), "general:10", &snapshot)
        .unwrap()
        .unwrap();
    assert!(recipe
        .nodes
        .iter()
        .all(|node| node.role != "challenge-avatar"));
    assert!(recipe.interaction_regions.is_empty());
    let score = recipe
        .nodes
        .iter()
        .find(|node| node.role == "challenge-score")
        .unwrap();
    assert!(matches!(
        &score.payload,
        super::GeneralRecipePayload::Text {
            source: crate::TextSource::ProfileField { value, .. },
            ..
        } if value == "0"
    ));
}

#[test]
fn challenge_stage_without_a_cleared_stage_shows_stage_one() {
    let snapshot = ProfileComponentSnapshot {
        locale: "en-US".into(),
        region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
        localized_text: BTreeMap::from([
            (
                "custom_profile.general.character_rank.title".into(),
                "Character Rank".into(),
            ),
            (
                "custom_profile.general.character_rank.challenge".into(),
                "Challenge".into(),
            ),
        ]),
        character_ranks: vec![crate::profile_scene::CharacterRankSnapshot {
            character_id: 1,
            rank: 40,
            challenge_rank: None,
            avatar: crate::profile_scene::ComponentImageSnapshot {
                source_field: "userProfile.characterRanks".into(),
                source_id: "1".into(),
                descriptor: None,
            },
        }],
        ..ProfileComponentSnapshot::default()
    };
    let recipe = super::build_general_recipe(11, StableId(11), "general:11", &snapshot)
        .unwrap()
        .unwrap();
    let stage = recipe
        .nodes
        .iter()
        .find(|node| node.role == "character-1-challenge_live_rank")
        .unwrap();
    assert!(matches!(
        &stage.payload,
        super::GeneralRecipePayload::Text {
            source: crate::TextSource::ProfileField { value, .. },
            ..
        } if value == "1"
    ));
}

#[test]
fn story_favorite_slots_without_a_story_are_drawn_empty() {
    let story = crate::profile_scene::StoryFavoriteSnapshot {
        story_id: 9,
        story_type: "event_story".into(),
        image: crate::profile_scene::ComponentImageSnapshot {
            source_field: "userProfile.storyFavorites".into(),
            source_id: "event_story:9".into(),
            descriptor: None,
        },
    };
    let empty = crate::profile_scene::StoryFavoriteSnapshot {
        story_id: 0,
        story_type: String::new(),
        image: crate::profile_scene::ComponentImageSnapshot {
            source_field: "userProfile.storyFavorites".into(),
            source_id: String::new(),
            descriptor: None,
        },
    };
    let snapshot = ProfileComponentSnapshot {
        locale: "en-US".into(),
        region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
        localized_text: BTreeMap::from([(
            "custom_profile.general.story_favorite.title".into(),
            "Favorite stories".into(),
        )]),
        story_favorites: vec![empty.clone(), empty, story],
        ..ProfileComponentSnapshot::default()
    };
    let recipe = super::build_general_recipe(14, StableId(14), "general:14", &snapshot)
        .unwrap()
        .unwrap();
    let slots = recipe
        .nodes
        .iter()
        .filter(|node| node.role.starts_with("story-") && !node.role.contains("scroll"))
        .map(|node| node.role.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        slots,
        vec![
            "story-0-empty",
            "story-1-empty",
            "story-2-placeholder",
            "story-3-empty",
            "story-4-empty",
            "story-5-empty",
            "story-6-empty",
            "story-7-empty",
        ]
    );
    assert_eq!(
        recipe
            .interaction_regions
            .iter()
            .filter(|region| region.role == "story-2")
            .count(),
        1
    );
}

#[test]
fn character_rank_grid_pill_shares_the_avatar_left_and_bottom_edges() {
    let elements = crate::profile_layout::CHAR_RANK.elements;
    let avatar = super::layout_rect(elements[2]);
    let pill = super::layout_rect(elements[3]);
    assert_eq!(pill.x, avatar.x);
    assert!((pill.y + pill.height - (avatar.y + avatar.height)).abs() < 1e-3);
    assert_eq!(elements[4].w, avatar.width);
    assert_eq!(elements[5].w, avatar.width);
}

#[test]
fn full_deck_exposes_five_independent_card_interaction_regions() {
    let deck_members = (0..5)
        .map(|slot| crate::profile_scene::CardVisualSnapshot {
            card_id: 4001 + slot,
            after_training: slot % 2 == 0,
            master_rank: slot,
            level: 50 + slot,
            rarity: "rarity_4".into(),
            attribute: "cool".into(),
            image: crate::profile_scene::ComponentImageSnapshot {
                source_field: "userProfile.deckMembers".into(),
                source_id: (4001 + slot).to_string(),
                descriptor: None,
            },
        })
        .collect();
    let recipe = super::build_general_recipe(
        3,
        StableId(3),
        "general:3",
        &ProfileComponentSnapshot {
            locale: "en-US".into(),
            region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
            localized_text: BTreeMap::from([(
                "custom_profile.general.card_level".into(),
                "Lv.{level}".into(),
            )]),
            deck_members,
            ..ProfileComponentSnapshot::default()
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(recipe.interaction_regions.len(), 5);
    for slot in 0..5 {
        let region = recipe
            .interaction_regions
            .iter()
            .find(|region| region.role == format!("deck-slot-{slot}-card"))
            .unwrap();
        assert_eq!(
            region.resolved_data.get("card_id"),
            Some(&crate::ParameterValue::I64(4001 + slot))
        );
        assert_eq!(region.bounds.width, 783.0 / 5.0);
        assert!(region.capabilities.contains(&"select_item".into()));
    }
}

#[test]
fn bonds_honor_recipe_uses_one_isolation_and_group_level_dst_in() {
    let descriptor = |namespace: &str, key: &str, width: f32, height: f32| {
        crate::profile_scene::ResourceDescriptor {
            resource: crate::ResourceKey {
                namespace: namespace.into(),
                key: key.into(),
            },
            natural_width: width,
            natural_height: height,
            provenance: BTreeMap::new(),
        }
    };
    let honor = crate::profile_scene::HonorVisualSnapshot {
        source_field: "userProfile.honorSlots".into(),
        source_id: "100".into(),
        honor_id: 100,
        honor_level: 27,
        full_size: true,
        visual: crate::profile_scene::HonorVisualKind::Bonds {
            character_ids: [1, 2],
            backgrounds: [
                Some(descriptor("static", "honor/bonds/1", 380.0, 80.0)),
                Some(descriptor("static", "honor/bonds/2", 380.0, 80.0)),
            ],
            characters: [
                Some(descriptor(
                    "assets",
                    "bonds_honor/chr_sd_01_01",
                    160.0,
                    160.0,
                )),
                Some(descriptor(
                    "assets",
                    "bonds_honor/chr_sd_02_01",
                    160.0,
                    160.0,
                )),
            ],
            mask: Some(descriptor("static", "honor/mask_degree_main", 380.0, 80.0)),
            frame: Some(descriptor("static", "honor/frame_degree_m_3", 380.0, 80.0)),
            word: Some(descriptor(
                "assets",
                "bonds_honor/word/demo_01",
                180.0,
                40.0,
            )),
            star: Some(descriptor("static", "honor/icon_degreeLv", 16.0, 16.0)),
            star_high: Some(descriptor("static", "honor/icon_degreeLv6", 16.0, 16.0)),
        },
    };
    let recipe = super::build_general_recipe(
        6,
        StableId(6),
        "general:6",
        &ProfileComponentSnapshot {
            honor_slots: vec![honor.clone()],
            ..ProfileComponentSnapshot::default()
        },
    )
    .unwrap()
    .unwrap();
    let roles = recipe
        .nodes
        .iter()
        .map(|node| node.role.as_str())
        .collect::<Vec<_>>();
    let begin = roles
        .iter()
        .position(|role| role.ends_with("isolation-begin"))
        .unwrap();
    let mask = roles
        .iter()
        .position(|role| role.ends_with("-mask"))
        .unwrap();
    let end = roles
        .iter()
        .position(|role| role.ends_with("isolation-end"))
        .unwrap();
    let frame = roles
        .iter()
        .position(|role| role.ends_with("-frame"))
        .unwrap();
    assert!(begin < mask && mask < end && end < frame);
    assert!(matches!(
        recipe.nodes[mask].payload,
        super::GeneralRecipePayload::Image {
            blend_mode: crate::BlendMode::DstIn,
            ..
        }
    ));
    assert_eq!(
        roles
            .iter()
            .filter(|role| role.contains("-star-") && !role.contains("star-high"))
            .count(),
        5
    );
    // Level 27 wraps to seven stars: five slots, two of them upgraded.
    assert_eq!(
        roles
            .iter()
            .filter(|role| role.contains("star-high"))
            .count(),
        2
    );
    assert_eq!(
        recipe.nodes[begin].bounds,
        crate::Rect {
            x: -378.0,
            y: -40.0,
            width: 380.0,
            height: 80.0
        }
    );
    let lowered = crate::profile_scene::lower_identity_general(
        6,
        StableId(6),
        "general:6",
        &ProfileComponentSnapshot {
            honor_slots: vec![honor.clone()],
            ..ProfileComponentSnapshot::default()
        },
    )
    .unwrap()
    .unwrap();
    assert!(lowered.commands.iter().any(|command| matches!(
        command.payload,
        crate::SemanticCommandPayload::Composite {
            operation: crate::CompositeOperation::BeginIsolation,
            ..
        }
    )));
    assert!(lowered.commands.iter().any(|command| matches!(
        command.payload,
        crate::SemanticCommandPayload::Composite {
            operation: crate::CompositeOperation::EndIsolation,
            ..
        }
    )));
    let mut without_mask = honor;
    if let crate::profile_scene::HonorVisualKind::Bonds { mask, .. } = &mut without_mask.visual {
        *mask = None;
    }
    let unmasked = super::build_general_recipe(
        6,
        StableId(6),
        "general:6",
        &ProfileComponentSnapshot {
            honor_slots: vec![without_mask],
            ..ProfileComponentSnapshot::default()
        },
    )
    .unwrap()
    .unwrap();
    assert!(!unmasked.nodes.iter().any(|node| matches!(
        node.payload,
        super::GeneralRecipePayload::Image {
            blend_mode: crate::BlendMode::DstIn,
            ..
        }
    )));
    assert_eq!(
        unmasked
            .nodes
            .iter()
            .filter(|node| matches!(node.payload, super::GeneralRecipePayload::Group { .. }))
            .count(),
        2
    );
}

#[test]
fn standard_honor_selects_first_available_frame_candidate_without_font() {
    let descriptor = |namespace: &str, key: &str| crate::profile_scene::ResourceDescriptor {
        resource: crate::ResourceKey {
            namespace: namespace.into(),
            key: key.into(),
        },
        natural_width: 180.0,
        natural_height: 80.0,
        provenance: BTreeMap::new(),
    };
    let honor = crate::profile_scene::HonorVisualSnapshot {
        source_field: "userProfile.honorSlots".into(),
        source_id: "7".into(),
        honor_id: 7,
        honor_level: 1,
        full_size: false,
        visual: crate::profile_scene::HonorVisualKind::Standard {
            honor_type: "achievement".into(),
            has_star: false,
            is_live_master: false,
            progress: 0,
            background: None,
            frame_candidates: vec![None, Some(descriptor("static", "honor/frame_degree_s_1"))],
            overlay: None,
            star: None,
            star_high: None,
            live_star_on: None,
            live_star_off: None,
        },
    };
    let recipe = super::build_general_recipe(
        6,
        StableId(6),
        "general:6",
        &ProfileComponentSnapshot {
            honor_slots: vec![honor],
            ..ProfileComponentSnapshot::default()
        },
    )
    .unwrap()
    .unwrap();
    assert!(recipe.nodes.iter().any(|node| matches!(&node.payload, super::GeneralRecipePayload::Image { resource, .. } if resource.key == "honor/frame_degree_s_1")));
}

#[test]
fn live_master_honor_keeps_only_the_progress_overlay() {
    let descriptor = crate::profile_scene::ResourceDescriptor {
        resource: crate::ResourceKey {
            namespace: "static".into(),
            key: "honor/live_master_honor_star_1".into(),
        },
        natural_width: 16.0,
        natural_height: 16.0,
        provenance: BTreeMap::new(),
    };
    let honor = crate::profile_scene::HonorVisualSnapshot {
        source_field: "userProfile.honorSlots".into(),
        source_id: "42".into(),
        honor_id: 42,
        honor_level: 1,
        full_size: true,
        visual: crate::profile_scene::HonorVisualKind::Standard {
            honor_type: "achievement".into(),
            has_star: false,
            is_live_master: true,
            progress: 73,
            background: None,
            frame_candidates: Vec::new(),
            overlay: None,
            star: None,
            star_high: None,
            live_star_on: Some(descriptor.clone()),
            live_star_off: Some(descriptor),
        },
    };
    let recipe = super::build_general_recipe(
        6,
        StableId(6),
        "general:6",
        &ProfileComponentSnapshot {
            region_fonts: BTreeMap::from([(1, "Noto Sans CJK SC".into())]),
            honor_slots: vec![honor],
            ..ProfileComponentSnapshot::default()
        },
    )
    .unwrap()
    .unwrap();

    let progress = recipe
        .nodes
        .iter()
        .find(|node| node.role == "honor-42-progress")
        .expect("Live Master progress command");
    assert_eq!(
        progress.bounds,
        crate::Rect {
            x: -148.0,
            y: 13.0,
            width: 80.0,
            height: 20.0,
        }
    );
    assert!(matches!(
        &progress.payload,
        super::GeneralRecipePayload::Text {
            source: TextSource::ProfileField { value, .. },
            font_size: 20.0,
            align: super::GeneralTextAlign::Center,
            ..
        } if value == "73"
    ));
    assert!(recipe
        .nodes
        .iter()
        .all(|node| !node.role.contains("live-star")));
}

#[test]
fn story_favorite_recipe_preserves_measured_slots_and_scrolls_only_the_content_group() {
    let favorites = (0..10)
        .map(|index| crate::profile_scene::StoryFavoriteSnapshot {
            story_id: index + 1,
            story_type: "event".into(),
            image: crate::profile_scene::ComponentImageSnapshot {
                source_field: "userProfile.storyFavorites".into(),
                source_id: format!("event:{}", index + 1),
                descriptor: Some(crate::profile_scene::ResourceDescriptor {
                    resource: crate::ResourceKey {
                        namespace: "assets".into(),
                        key: format!("story/{index}"),
                    },
                    natural_width: 400.0,
                    natural_height: 170.0,
                    provenance: BTreeMap::new(),
                }),
            },
        })
        .collect();
    let snapshot = ProfileComponentSnapshot {
        locale: "en-US".into(),
        region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
        localized_text: BTreeMap::from([(
            "custom_profile.general.story_favorite.title".into(),
            "Favorite stories".into(),
        )]),
        story_favorites: favorites,
        ..ProfileComponentSnapshot::default()
    };
    let recipe = super::build_general_recipe(14, StableId(14), "general:14", &snapshot)
        .unwrap()
        .unwrap();
    assert_eq!(
        recipe
            .nodes
            .iter()
            .filter(|node| node.role.starts_with("story-") && !node.role.contains("scroll"))
            .count(),
        10
    );
    for index in 0..8 {
        let node = recipe
            .nodes
            .iter()
            .find(|node| node.role == format!("story-{index}"))
            .unwrap();
        assert_eq!(
            node.bounds,
            super::layout_rect(crate::profile_layout::STORY_FAVORITE.elements[index + 2])
        );
    }
    let track = recipe
        .nodes
        .iter()
        .find(|node| node.role == "story-scroll-track")
        .unwrap();
    assert_eq!((track.bounds.x, track.bounds.width), (475.5, 4.0));
    assert!(recipe
        .interaction_regions
        .iter()
        .find(|region| region.role == "story-favorite-scroll")
        .unwrap()
        .control_bindings
        .iter()
        .any(|binding| matches!(binding, crate::CommandControlBinding::ScrollViewport { .. })));
    let thumb_region = recipe
        .interaction_regions
        .iter()
        .find(|region| region.role == "story-scroll-thumb")
        .unwrap();
    assert_eq!(
        (thumb_region.bounds.x, thumb_region.bounds.width),
        (475.5, 4.0)
    );
    assert!(thumb_region
        .capabilities
        .iter()
        .any(|value| value == "drag"));
    assert!(thumb_region
        .control_bindings
        .iter()
        .any(|binding| matches!(binding, crate::CommandControlBinding::ScrollThumb { .. })));
    let content = recipe
        .nodes
        .iter()
        .find(|node| node.role == "story-0")
        .unwrap();
    assert!(matches!(
        content.control_bindings[0],
        crate::CommandControlBinding::ScrollContent { .. }
    ));
    assert!(matches!(
        content.clips[0],
        super::GeneralClip::Rect {
            bounds: crate::Rect {
                x: -424.0,
                y: -320.0,
                width: 848.0,
                height: 756.0
            },
            ..
        }
    ));
    assert_eq!(recipe.controls.len(), 1);
    assert_eq!(
        recipe
            .interaction_regions
            .iter()
            .filter(|region| {
                region.role.starts_with("story-") && !region.role.contains("scroll")
            })
            .count(),
        10
    );
}

#[test]
fn compact_character_rank_recipe_uses_true_circle_tabs_and_patch_only_scroll_bindings() {
    let ranks = (1..=26)
        .map(|character_id| crate::profile_scene::CharacterRankSnapshot {
            character_id,
            rank: 10 + character_id,
            challenge_rank: Some(20 + character_id),
            avatar: crate::profile_scene::ComponentImageSnapshot {
                source_field: "userProfile.characterRanks".into(),
                source_id: character_id.to_string(),
                descriptor: Some(crate::profile_scene::ResourceDescriptor {
                    resource: crate::ResourceKey {
                        namespace: "static".into(),
                        key: format!("chara_avatar/chara{character_id:02}_02"),
                    },
                    natural_width: 128.0,
                    natural_height: 128.0,
                    provenance: BTreeMap::new(),
                }),
            },
        })
        .collect();
    let snapshot = ProfileComponentSnapshot {
        locale: "en-US".into(),
        region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
        localized_text: BTreeMap::from([
            (
                "custom_profile.general.character_rank.title".into(),
                "Character rank".into(),
            ),
            (
                "custom_profile.general.character_rank.challenge".into(),
                "Challenge rank".into(),
            ),
        ]),
        character_ranks: ranks,
        ..ProfileComponentSnapshot::default()
    };
    let recipe = super::build_general_recipe(15, StableId(15), "general:15", &snapshot)
        .unwrap()
        .unwrap();
    assert_eq!(recipe.controls.len(), 2);
    assert_eq!(
        recipe
            .nodes
            .iter()
            .filter(|node| node.role.contains("label-"))
            .count(),
        4
    );
    let avatar = recipe
        .nodes
        .iter()
        .find(|node| node.role == "character-21-avatar")
        .unwrap();
    assert!(matches!(
        avatar.clips[0],
        super::GeneralClip::Ellipse {
            anti_alias: true,
            ..
        }
    ));
    assert_eq!((avatar.bounds.width, avatar.bounds.height), (76.0, 76.0));
    for mode in ["character_rank", "challenge_live_rank"] {
        let number = recipe
            .nodes
            .iter()
            .find(|node| node.role == format!("character-21-{mode}"))
            .unwrap();
        assert!(number.control_bindings.iter().any(|binding| matches!(binding, crate::CommandControlBinding::TabOption { value, .. } if value == mode)));
        assert!(number
            .control_bindings
            .iter()
            .any(|binding| matches!(binding, crate::CommandControlBinding::ScrollContent { .. })));
        assert!(matches!(
            number.clips[0],
            super::GeneralClip::Rect {
                bounds: crate::Rect {
                    x: -483.5,
                    y: -175.5,
                    width: 967.0,
                    height: 461.5
                },
                ..
            }
        ));
    }
    let track = recipe
        .nodes
        .iter()
        .find(|node| node.role == "character-rank-scroll-track")
        .unwrap();
    assert_eq!((track.bounds.x, track.bounds.width), (475.5, 4.0));
    assert!(recipe
        .interaction_regions
        .iter()
        .find(|region| region.role == "character-rank-scroll")
        .unwrap()
        .control_bindings
        .iter()
        .any(|binding| matches!(binding, crate::CommandControlBinding::ScrollViewport { .. })));
    let thumb_region = recipe
        .interaction_regions
        .iter()
        .find(|region| region.role == "character-rank-scroll-thumb")
        .unwrap();
    assert_eq!(
        (thumb_region.bounds.x, thumb_region.bounds.width),
        (475.5, 4.0)
    );
    assert!(thumb_region
        .capabilities
        .iter()
        .any(|value| value == "drag"));
    assert!(thumb_region
        .control_bindings
        .iter()
        .any(|binding| matches!(binding, crate::CommandControlBinding::ScrollThumb { .. })));
    assert_eq!(
        recipe
            .interaction_regions
            .iter()
            .filter(|region| region.resolved_data.contains_key("character_id"))
            .count(),
        26
    );
    assert_eq!(
        recipe
            .nodes
            .iter()
            .filter(|node| node.role.ends_with("-character_rank")
                || node.role.ends_with("-challenge_live_rank"))
            .count(),
        52
    );
    let final_character = recipe
        .nodes
        .iter()
        .find(|node| node.role == "character-20-character_rank")
        .unwrap();
    assert!((final_character.bounds.y - 517.6).abs() < 0.001);
    let full = super::build_general_recipe(11, StableId(11), "general:11", &snapshot)
        .unwrap()
        .unwrap();
    assert_eq!(full.controls.len(), 1);
    assert!(!full.nodes.iter().any(|node| node.role.contains("scroll")));
    assert_eq!(
        full.interaction_regions
            .iter()
            .filter(|region| region.resolved_data.contains_key("character_id"))
            .count(),
        26
    );
    let full_avatar = full
        .nodes
        .iter()
        .find(|node| node.role == "character-21-avatar")
        .unwrap();
    assert_eq!(full_avatar.bounds.y, -297.0);
    assert_eq!(
        full.nodes
            .iter()
            .find(|node| node.role == "tab-selected-character-rank")
            .unwrap()
            .bounds
            .y,
        -384.0
    );
}
