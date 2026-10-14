use super::*;

fn provider_with(name: &str, json: &str) -> JsonMasterDataProvider {
    let mut p = JsonMasterDataProvider::empty();
    p.insert_table(name, json).expect("insert table");
    p
}

#[test]
fn resolves_color_from_hex_table() {
    let p = provider_with(
        "customProfileTextColors",
        r##"[{"id": 1, "colorCode": "#ff8800"}]"##,
    );
    let c = p.resolve_color(1).expect("color");
    assert_eq!(p.default_color().map(|value| value.r), Some(c.r));
    assert_eq!((c.r, c.g, c.b, c.a), (0xff, 0x88, 0x00, 0xff));
    assert!(p.resolve_color(2).is_none());
    assert_eq!(p.color_count(), 1);
}

#[test]
fn maps_fot_font_names_to_cn() {
    let p = provider_with(
        "customProfileTextFonts",
        r#"[{"id": 1, "fontName": "FOT-RodinNTLGPro-DB"}, {"id": 9, "fontName": "Custom"}]"#,
    );
    // The default region is Cn: FOT names map to FZ names, other names pass
    // through.
    assert_eq!(p.resolve_font(1).as_deref(), Some("FZLanTingHei-DB-GBK"));
    assert_eq!(p.resolve_font(9).as_deref(), Some("Custom"));
}

#[test]
fn non_cn_region_does_not_map_font_names() {
    let p = provider_with(
        "customProfileTextFonts",
        r#"[{"id": 1, "fontName": "FOT-RodinNTLGPro-DB"}, {"id": 4, "fontName": "FOT-HummingPro-B"}]"#,
    )
    .with_region(Region::Jp);
    // The JP font files are the FOT faces, so FOT names pass through.
    assert_eq!(p.resolve_font(1).as_deref(), Some("FOT-RodinNTLGPro-DB"));
    assert_eq!(p.resolve_font(4).as_deref(), Some("FOT-HummingPro-B"));
    assert_eq!(p.region(), Region::Jp);
}

#[test]
fn resolve_player_info_label_reads_name_field() {
    let p = provider_with(
        "customProfilePlayerInfoResources",
        r#"[{"id": 2, "name": "Total", "fileName": "TotalPower", "resourceLoadVal": "x", "customProfileResourceType": "player_info"}]"#,
    );
    assert_eq!(p.resolve_player_info_label(2).as_deref(), Some("Total"));
    assert!(p.resolve_player_info_label(99).is_none());
}

#[test]
fn collection_rows_carry_their_collection_type() {
    let p = provider_with(
        "customProfileCollectionResources",
        r#"[
                {"id": 1, "customProfileResourceType": "collection", "customProfileResourceCollectionType": "can_badge", "resourceLoadVal": "custom_profile/collection/crash", "fileName": "fixture_canbadge"},
                {"id": 2, "customProfileResourceType": "collection", "customProfileResourceCollectionType": "omikuji", "resourceLoadVal": "lottery_game/new_year_2022", "fileName": "Prefabs/Omikuji"},
                {"id": 3, "customProfileResourceType": "collection", "customProfileResourceCollectionType": null, "resourceLoadVal": "custom_profile/collection/misc", "fileName": "plain"},
                {"id": 4, "customProfileResourceType": "collection", "resourceLoadVal": "custom_profile/collection/misc", "fileName": "absent"}
            ]"#,
    );
    let kind = |id| {
        p.resolve_resource("collection", id)
            .unwrap()
            .collection_type
    };
    assert_eq!(kind(1), CollectionResourceType::CanBadge);
    assert_eq!(kind(2), CollectionResourceType::Omikuji);
    assert_eq!(kind(3), CollectionResourceType::None);
    assert_eq!(kind(4), CollectionResourceType::None);
}

#[test]
fn omikuji_rows_resolve_from_the_optional_omikujis_table() {
    let p = JsonMasterDataProvider::empty();
    assert!(p.resolve_omikuji(1).is_none());
    assert!(OPTIONAL_TABLES.contains(&"omikujis"));
    assert!(!p.missing_tables().contains(&"omikujis"));
    let p = provider_with(
        "omikujis",
        r#"[{"id": 1, "omikujiGroupId": 1, "unit": "idol", "fortuneType": "grate_fortune",
                 "summary": "s", "title1": "t1", "description1": "d1", "title2": "t2",
                 "description2": "d2", "title3": "t3", "description3": "d3",
                 "unitAssetbundleName": "u", "unitFilePath": "uf",
                 "fortuneAssetbundleName": "lottery_game/new_year_2022_material",
                 "fortuneFilePath": "unsei_daikichi",
                 "omikujiCoverAssetbundleName": "lottery_game/new_year_2022_material",
                 "omikujiCoverFilePath": "omikuji_idol"}]"#,
    );
    let row = p.resolve_omikuji(1).expect("row");
    assert_eq!((row.unit.as_str(), row.title3.as_str()), ("idol", "t3"));
    assert_eq!(row.fortune_file_path, "unsei_daikichi");
    assert_eq!(row.omikuji_cover_file_path, "omikuji_idol");
    assert!(p.resolve_omikuji(2).is_none());
}

#[test]
fn missing_tables_reports_unloaded() {
    let p = provider_with("cards", "[]");
    assert!(p.missing_tables().contains(&"stamps"));
    assert!(!p.missing_tables().contains(&"cards"));
}

const ICON_TABLES: [(&str, &str, &str); 3] = [
    (
        "customProfileCharacterIconResources",
        "character_icon",
        "profile_chr_icon_ichika",
    ),
    (
        "customProfileMaterialResources",
        "material",
        "profile_icon_item_0001",
    ),
    (
        "customProfileUserInterfaceIconResources",
        "user_interface_icon",
        "profile_icon_0001",
    ),
];

fn icon_table_json(resource_type: &str, file_name: &str) -> String {
    format!(
        r#"[{{"id": 1, "seq": 1, "customProfileResourceType": "{resource_type}", "resourceLoadType": "assetbundle", "resourceLoadVal": "custom_profile/{resource_type}", "fileName": "{file_name}"}}]"#
    )
}

#[test]
fn icon_resource_tables_resolve_their_rows() {
    let mut p = JsonMasterDataProvider::empty();
    for (table, resource_type, file_name) in ICON_TABLES {
        p.insert_table(table, &icon_table_json(resource_type, file_name))
            .expect("insert table");
    }
    for (_, resource_type, file_name) in ICON_TABLES {
        let info = p.resolve_resource(resource_type, 1).expect(resource_type);
        assert_eq!(
            info.asset_key(),
            format!("custom_profile/{resource_type}/{file_name}")
        );
        assert_eq!(info.resource_type, resource_type);
        assert!(p.resolve_resource(resource_type, 2).is_none());
    }
}

#[test]
fn a_directory_without_the_icon_tables_is_not_missing_required_tables() {
    let dir = std::env::temp_dir().join(format!(
        "sekai-profile-host-optional-tables-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create dir");
    std::fs::write(dir.join("cards.json"), "[]").expect("write cards");
    let (table, resource_type, file_name) = ICON_TABLES[1];
    std::fs::write(
        dir.join(format!("{table}.json")),
        icon_table_json(resource_type, file_name),
    )
    .expect("write icon table");
    let loaded = JsonMasterDataProvider::from_dir(&dir);
    std::fs::remove_dir_all(&dir).expect("remove dir");
    let p = loaded.expect("provider");
    assert!(p.resolve_resource(resource_type, 1).is_some());
    let missing = p.missing_tables();
    assert!(missing.contains(&"stamps"));
    for (table, _, _) in ICON_TABLES {
        assert!(OPTIONAL_TABLES.contains(&table), "{table}");
        assert!(!REQUIRED_TABLES.contains(&table), "{table}");
        assert!(!missing.contains(&table), "{table}");
    }
}

#[test]
fn live_master_honor_falls_back_to_the_last_known_level() {
    // The game resolves a level the masterdata does not list yet to the
    // last known level instead of clearing the asset bundle name.
    let p = provider_with(
        "honors",
        r##"[{"id": 7, "assetbundleName": null, "honorRarity": null, "groupId": null,
                  "honorMissionType": "master_full_perfect",
                  "levels": [
                    {"level": 1, "assetbundleName": "honor_live_1", "honorRarity": "low"},
                    {"level": 5, "assetbundleName": "honor_live_5", "honorRarity": "high"}
                  ]}]"##,
    );
    let honor = p.resolve_honor(7, 9).expect("honor");
    assert!(honor.is_live_master);
    assert_eq!(honor.asset_bundle_name, "honor_live_5");
    assert_eq!(honor.honor_rarity, "high");
    let honor = p.resolve_honor(7, 1).expect("honor");
    assert_eq!(honor.asset_bundle_name, "honor_live_1");
    assert_eq!(honor.honor_level, 1);
}

#[test]
fn honor_groups_treat_empty_fields_as_absent() {
    let mut p = provider_with(
        "honors",
        r#"[{"id": 3, "assetbundleName": "honor_0003", "honorRarity": "high", "groupId": 4,
                 "levels": [{"level": 1}]}]"#,
    );
    p.insert_table(
        "honorGroups",
        r#"[{"id": 4, "honorType": "event", "backgroundAssetbundleName": "", "frameName": ""}]"#,
    )
    .unwrap();
    let honor = p.resolve_honor(3, 1).expect("honor");
    assert_eq!(honor.honor_type, "event");
    assert_eq!(honor.background_asset_bundle_name, None);
    assert_eq!(honor.frame_name, None);
}
