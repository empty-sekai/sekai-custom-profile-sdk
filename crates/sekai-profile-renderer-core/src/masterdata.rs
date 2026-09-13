//! Backend-neutral master-data queries used by profile resolution.

use std::collections::BTreeMap;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const PROFILE_MASTERDATA_TABLES: &[&str] = &[
    "cards",
    "stamps",
    "honors",
    "honorGroups",
    "bondsHonors",
    "bondsHonorWords",
    "gameCharacterUnits",
    "customProfileTextColors",
    "customProfileTextFonts",
    "customProfileShapeResources",
    "customProfileEtcResources",
    "customProfileCollectionResources",
    "customProfileGeneralBackgroundResources",
    "customProfileMemberStandingPictureResources",
    "customProfileStoryBackgroundResources",
    "eventStories",
    "unitStoryEpisodeGroups",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl ResolvedColor {
    pub fn from_hex(value: &str) -> Option<Self> {
        let value = value.strip_prefix('#').unwrap_or(value);
        // Byte-based: the length is measured in bytes, and a string slice at
        // a non-char-boundary index would panic on multi-byte input. Invalid
        // digits yield None rather than a panic.
        let bytes = value.as_bytes();
        if bytes.len() != 6 && bytes.len() != 8 {
            return None;
        }
        let hex2 = |range: std::ops::Range<usize>| {
            std::str::from_utf8(&bytes[range])
                .ok()
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        };
        Some(Self {
            r: hex2(0..2)?,
            g: hex2(2..4)?,
            b: hex2(4..6)?,
            a: if bytes.len() == 8 { hex2(6..8)? } else { 255 },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceInfo {
    pub file_name: String,
    pub load_value: String,
    pub resource_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardEntry {
    pub id: i32,
    #[serde(rename = "assetbundleName")]
    pub asset_bundle_name: String,
    pub card_rarity_type: String,
    pub attr: String,
    #[serde(default)]
    pub character_id: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HonorLevelEntry {
    pub level: i32,
    pub assetbundle_name: Option<String>,
    pub honor_rarity: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HonorEntry {
    pub id: i32,
    pub assetbundle_name: Option<String>,
    pub honor_rarity: Option<String>,
    pub group_id: Option<i32>,
    #[serde(default)]
    pub levels: Vec<HonorLevelEntry>,
    pub honor_mission_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedHonor {
    pub asset_bundle_name: String,
    pub honor_rarity: String,
    pub honor_type: String,
    pub background_asset_bundle_name: Option<String>,
    pub frame_name: Option<String>,
    pub is_live_master: bool,
    pub has_star: bool,
    pub honor_mission_type: Option<String>,
}

/// Resolves the bundle that owns a standard honor's degree layer.
///
/// This is a backend-neutral game-data rule. Native and browser renderers
/// must call it instead of maintaining independent bundle fallbacks.
pub fn effective_honor_background_asset_bundle_name<'a>(
    honor_type: &str,
    asset_bundle_name: &'a str,
    background_asset_bundle_name: Option<&'a str>,
) -> &'a str {
    if let Some(background) = background_asset_bundle_name.filter(|value| !value.is_empty()) {
        return background;
    }
    if honor_type == "limitevent" && asset_bundle_name.starts_with("honor_top_") {
        return "honor_bg_event_cheerteam";
    }
    asset_bundle_name
}

/// Returns whether a standard honor owns a rank/progress overlay layer.
pub fn honor_has_rank_overlay(
    honor_type: &str,
    asset_bundle_name: &str,
    is_live_master: bool,
) -> bool {
    is_live_master
        || matches!(honor_type, "rank_match" | "sekai_echo")
        || [
            "honor_top_",
            "honor_shining",
            "honor_memorial",
            "honor_memory",
        ]
        .iter()
        .any(|prefix| asset_bundle_name.starts_with(prefix))
}

/// Resource identities for one standard honor. Frame candidates are ordered
/// alternatives, not independent requirements: custom first, then default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardHonorAssetPlan {
    pub background: crate::ResourceKey,
    pub overlay: Option<crate::ResourceKey>,
    pub frame_candidates: [Option<crate::ResourceKey>; 2],
    pub star: Option<crate::ResourceKey>,
    pub star_high: Option<crate::ResourceKey>,
}

impl StandardHonorAssetPlan {
    /// All possible source keys, including both sides of the frame fallback.
    pub fn resources(&self) -> impl Iterator<Item = &crate::ResourceKey> {
        std::iter::once(&self.background)
            .chain(self.overlay.iter())
            .chain(self.frame_candidates.iter().flatten())
            .chain(self.star.iter())
            .chain(self.star_high.iter())
    }
}

impl ResolvedHonor {
    pub fn asset_plan(&self, level: i32, full_size: bool) -> StandardHonorAssetPlan {
        let resource = |namespace: &str, key: String| crate::ResourceKey {
            namespace: namespace.into(),
            key,
        };
        let (suffix, size_char) = if full_size {
            ("main", "m")
        } else {
            ("sub", "s")
        };
        let directory = if self.honor_type == "rank_match" {
            "rank_live/honor"
        } else {
            "honor"
        };
        let rarity = match self.honor_rarity.as_str() {
            "low" => 1,
            "middle" => 2,
            "high" => 3,
            _ => 4,
        };
        let frame_candidates = [
            self.frame_name.as_ref().map(|name| {
                resource(
                    "assets",
                    format!("honor_frame/{name}/frame_degree_{size_char}_{rarity}"),
                )
            }),
            Some(resource(
                "static",
                format!("honor/frame_degree_{size_char}_{rarity}"),
            )),
        ];
        let overlay = self.has_rank_overlay().then(|| {
            let name = if self.honor_type == "rank_match" {
                suffix.into()
            } else if self.is_live_master {
                "scroll".into()
            } else {
                format!("rank_{suffix}")
            };
            resource(
                "assets",
                format!("{directory}/{}/{name}", self.asset_bundle_name),
            )
        });
        let mut star_level = level % 10;
        if star_level == 0 && level > 0 {
            star_level = 10;
        }
        let has_stars = self.has_star
            && !self.is_live_master
            && matches!(self.honor_type.as_str(), "character" | "achievement");
        StandardHonorAssetPlan {
            background: resource(
                "assets",
                format!(
                    "{directory}/{}/degree_{suffix}",
                    self.effective_background_asset_bundle_name(),
                ),
            ),
            overlay,
            frame_candidates,
            star: (has_stars && star_level > 0)
                .then(|| resource("static", "honor/icon_degreeLv".into())),
            star_high: (has_stars && star_level > 5)
                .then(|| resource("static", "honor/icon_degreeLv6".into())),
        }
    }

    pub fn effective_background_asset_bundle_name(&self) -> &str {
        effective_honor_background_asset_bundle_name(
            &self.honor_type,
            &self.asset_bundle_name,
            self.background_asset_bundle_name.as_deref(),
        )
    }

    pub fn has_rank_overlay(&self) -> bool {
        honor_has_rank_overlay(
            &self.honor_type,
            &self.asset_bundle_name,
            self.is_live_master,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BondsHonorEntry {
    pub id: i32,
    pub game_character_unit_id1: i32,
    pub game_character_unit_id2: i32,
    pub honor_rarity: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BondsHonorWordEntry {
    pub id: i64,
    pub assetbundle_name: String,
}

pub trait ProfileMasterData {
    fn resolve_story_banner(&self, story_type: &str, story_id: i32) -> Option<String>;
    fn get_card(&self, card_id: i32) -> Option<CardEntry>;
    fn resolve_color(&self, color_id: i32) -> Option<ResolvedColor>;
    fn resolve_font(&self, font_id: i32) -> Option<String>;
    fn resolve_stamp(&self, stamp_id: i32) -> Option<String>;
    fn resolve_resource(&self, resource_type: &str, id: i32) -> Option<ResourceInfo>;
    fn resolve_honor(&self, honor_id: i32, honor_level: i32) -> Option<ResolvedHonor>;
    fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry>;
    fn get_bonds_honor_word(&self, id: i64) -> Option<BondsHonorWordEntry>;
    fn resolve_unit_virtual_singer(&self, self_id: i32, partner_id: i32) -> i32;
    fn resolve_localized_text(&self, _key: &str) -> Option<String> {
        None
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum MasterDataError {
    #[error("master-data table {table} is not a JSON array or object")]
    InvalidTable { table: String },
}

#[derive(Clone, Debug)]
struct JsonTable {
    rows: Vec<Value>,
    index: BTreeMap<i64, usize>,
}

impl JsonTable {
    fn new(name: &str, value: Value) -> Result<Self, MasterDataError> {
        let rows = match value {
            Value::Array(rows) => rows,
            Value::Object(_) => vec![value],
            _ => return Err(MasterDataError::InvalidTable { table: name.into() }),
        };
        let index = rows
            .iter()
            .enumerate()
            .filter_map(|(position, row)| {
                row.get("id")
                    .and_then(Value::as_i64)
                    .map(|id| (id, position))
            })
            .collect();
        Ok(Self { rows, index })
    }
    fn get(&self, id: i64) -> Option<&Value> {
        self.index.get(&id).map(|index| &self.rows[*index])
    }
    fn typed<T: DeserializeOwned>(&self, id: i64) -> Option<T> {
        serde_json::from_value(self.get(id)?.clone()).ok()
    }
}

/// Canonicalizes a region code so the CN font-name mapping and `locale`'s
/// alias table agree on the same inputs regardless of caller casing.
pub fn normalize_region(region: &str) -> String {
    match region.trim().to_ascii_lowercase().as_str() {
        "cn" | "sc" | "zh-cn" | "zh-hans" => "cn".into(),
        "jp" | "ja" | "ja-jp" => "jp".into(),
        "tw" | "tc" | "zh-tw" | "zh-hant" => "tw".into(),
        "en" | "world" | "en-us" | "en-gb" => "en".into(),
        "kr" | "ko" | "ko-kr" => "kr".into(),
        other => other.into(),
    }
}

/// Parsed table collection. It contains no network or filesystem policy.
#[derive(Clone, Debug)]
pub struct JsonMasterData {
    region: String,
    tables: BTreeMap<String, JsonTable>,
}

impl JsonMasterData {
    pub fn new(region: impl Into<String>) -> Self {
        Self {
            region: normalize_region(&region.into()),
            tables: BTreeMap::new(),
        }
    }
    pub fn insert_value(&mut self, name: &str, value: Value) -> Result<(), MasterDataError> {
        self.tables
            .insert(name.into(), JsonTable::new(name, value)?);
        Ok(())
    }
    pub fn insert_json(&mut self, name: &str, json: &str) -> Result<(), serde_json::Error> {
        let value = serde_json::from_str(json)?;
        self.insert_value(name, value).map_err(|error| {
            serde_json::Error::io(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        })
    }
    pub fn loaded_tables(&self) -> impl Iterator<Item = &str> {
        self.tables.keys().map(String::as_str)
    }
    fn table(&self, name: &str) -> Option<&JsonTable> {
        self.tables.get(name)
    }
}

impl ProfileMasterData for JsonMasterData {
    fn resolve_story_banner(&self, story_type: &str, story_id: i32) -> Option<String> {
        let (table, suffix) = match story_type {
            "event_story" => ("eventStories", "banner_event_story"),
            "unit_story" => ("unitStoryEpisodeGroups", "banner_unit_story"),
            _ => return None,
        };
        let row = self.table(table)?.get(story_id.into())?;
        let bundle = row.get("assetbundleName")?.as_str()?;
        Some(format!("{story_type}/{bundle}/screen_image/{suffix}"))
    }
    fn get_card(&self, card_id: i32) -> Option<CardEntry> {
        self.table("cards")?.typed(card_id.into())
    }
    fn resolve_color(&self, color_id: i32) -> Option<ResolvedColor> {
        ResolvedColor::from_hex(
            self.table("customProfileTextColors")?
                .get(color_id.into())?
                .get("colorCode")?
                .as_str()?,
        )
    }
    fn resolve_font(&self, font_id: i32) -> Option<String> {
        let name = self
            .table("customProfileTextFonts")?
            .get(font_id.into())?
            .get("fontName")?
            .as_str()?;
        Some(
            if self.region == "cn" {
                match name {
                    "FOT-RodinNTLGPro-DB" => "FZLanTingHei-DB-GBK",
                    "FOT-SkipProN-B" => "FZZhengHei-EB-GBK",
                    "FOT-PopHappinessStd-EB" => "FZShaoEr-M11-JF",
                    other => other,
                }
            } else {
                name
            }
            .into(),
        )
    }
    fn resolve_stamp(&self, stamp_id: i32) -> Option<String> {
        self.table("stamps")?
            .get(stamp_id.into())?
            .get("assetbundleName")?
            .as_str()
            .map(str::to_owned)
    }
    fn resolve_resource(&self, resource_type: &str, id: i32) -> Option<ResourceInfo> {
        let table = match resource_type {
            "shape" => "customProfileShapeResources",
            "etc" => "customProfileEtcResources",
            "collection" => "customProfileCollectionResources",
            "general_bg" => "customProfileGeneralBackgroundResources",
            "standing" => "customProfileMemberStandingPictureResources",
            "story_bg" => "customProfileStoryBackgroundResources",
            _ => return None,
        };
        let row = self.table(table)?.get(id.into())?;
        Some(ResourceInfo {
            file_name: row.get("fileName")?.as_str()?.into(),
            load_value: row.get("resourceLoadVal")?.as_str()?.into(),
            resource_type: row.get("customProfileResourceType")?.as_str()?.into(),
        })
    }
    fn resolve_honor(&self, honor_id: i32, honor_level: i32) -> Option<ResolvedHonor> {
        let honor: HonorEntry = self.table("honors")?.typed(honor_id.into())?;
        let live = honor.honor_mission_type.is_some() && honor.assetbundle_name.is_none();
        // A level the masterdata does not know yet (data lagging behind player
        // progress) falls back to the last known level, matching the game's
        // lookup, rather than producing an empty asset bundle name.
        let level = honor
            .levels
            .iter()
            .find(|entry| entry.level == honor_level)
            .or_else(|| honor.levels.last());
        let group = honor
            .group_id
            .and_then(|id| self.table("honorGroups")?.get(id.into()));
        Some(ResolvedHonor {
            asset_bundle_name: if live {
                level
                    .and_then(|v| v.assetbundle_name.clone())
                    .unwrap_or_default()
            } else {
                honor.assetbundle_name.unwrap_or_default()
            },
            honor_rarity: if live {
                level
                    .and_then(|v| v.honor_rarity.clone())
                    .unwrap_or_else(|| "low".into())
            } else {
                honor.honor_rarity.unwrap_or_else(|| "low".into())
            },
            honor_type: group
                .and_then(|v| v.get("honorType"))
                .and_then(Value::as_str)
                .unwrap_or("normal")
                .into(),
            background_asset_bundle_name: group
                .and_then(|v| v.get("backgroundAssetbundleName"))
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .map(str::to_owned),
            frame_name: group
                .and_then(|v| v.get("frameName"))
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .map(str::to_owned),
            is_live_master: live,
            has_star: honor.levels.len() > 1,
            honor_mission_type: honor.honor_mission_type,
        })
    }
    fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry> {
        self.table("bondsHonors")?.typed(id.into())
    }
    fn get_bonds_honor_word(&self, id: i64) -> Option<BondsHonorWordEntry> {
        self.table("bondsHonorWords")?.typed(id)
    }
    fn resolve_unit_virtual_singer(&self, self_id: i32, partner_id: i32) -> i32 {
        let Some(table) = self.table("gameCharacterUnits") else {
            return self_id;
        };
        let Some(character) = table
            .get(self_id.into())
            .and_then(|v| v.get("gameCharacterId"))
            .and_then(Value::as_i64)
        else {
            return self_id;
        };
        if character < 21 {
            return self_id;
        }
        let Some(unit) = table
            .get(partner_id.into())
            .and_then(|v| v.get("unit"))
            .and_then(Value::as_str)
        else {
            return self_id;
        };
        table
            .rows
            .iter()
            .find(|row| {
                row.get("gameCharacterId").and_then(Value::as_i64) == Some(character)
                    && row.get("unit").and_then(Value::as_str) == Some(unit)
            })
            .and_then(|row| row.get("id"))
            .and_then(Value::as_i64)
            .unwrap_or(self_id.into()) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planned_honor() -> ResolvedHonor {
        ResolvedHonor {
            asset_bundle_name: "honor_sample".into(),
            honor_rarity: "low".into(),
            honor_type: "character".into(),
            background_asset_bundle_name: None,
            frame_name: Some("custom_frame".into()),
            is_live_master: false,
            has_star: true,
            honor_mission_type: None,
        }
    }

    #[test]
    fn standard_honor_plan_keeps_ordered_frames_and_namespaces_for_all_rarities() {
        for (rarity, number) in [("low", 1), ("middle", 2), ("high", 3), ("highest", 4)] {
            for (full_size, size) in [(true, "m"), (false, "s")] {
                let mut honor = planned_honor();
                honor.honor_rarity = rarity.into();
                let plan = honor.asset_plan(10, full_size);
                let frames = plan
                    .frame_candidates
                    .iter()
                    .flatten()
                    .map(|key| (key.namespace.as_str(), key.key.clone()))
                    .collect::<Vec<_>>();
                assert_eq!(
                    frames,
                    vec![
                        (
                            "assets",
                            format!("honor_frame/custom_frame/frame_degree_{size}_{number}")
                        ),
                        ("static", format!("honor/frame_degree_{size}_{number}")),
                    ]
                );
                assert_eq!(plan.star.as_ref().unwrap().namespace, "static");
                assert_eq!(plan.star_high.as_ref().unwrap().namespace, "static");
                assert_eq!(plan.resources().count(), 5);
            }
        }
    }

    #[test]
    fn standard_honor_plan_requests_stars_only_when_they_are_drawn() {
        let mut honor = planned_honor();
        for (level, normal, high) in [
            (-1, false, false),
            (0, false, false),
            (1, true, false),
            (5, true, false),
            (6, true, true),
            (10, true, true),
            (11, true, false),
            (20, true, true),
        ] {
            let plan = honor.asset_plan(level, false);
            assert_eq!(
                (plan.star.is_some(), plan.star_high.is_some()),
                (normal, high),
                "level={level}"
            );
        }
        for honor_type in ["normal", "rank_match", "event"] {
            honor.honor_type = honor_type.into();
            let plan = honor.asset_plan(10, true);
            assert!(plan.star.is_none() && plan.star_high.is_none());
        }
        honor.honor_type = "achievement".into();
        assert!(honor.asset_plan(10, true).star_high.is_some());
        honor.has_star = false;
        assert!(honor.asset_plan(10, true).star.is_none());
        honor.has_star = true;
        honor.is_live_master = true;
        let plan = honor.asset_plan(10, true);
        assert!(plan.star.is_none() && plan.star_high.is_none());
        assert_eq!(plan.overlay.unwrap().key, "honor/honor_sample/scroll");
    }
    #[test]
    fn json_provider_maps_fonts_by_region_without_mutating_source_tables() {
        let table = serde_json::json!([{ "id": 1, "fontName": "FOT-RodinNTLGPro-DB" }]);
        let mut cn = JsonMasterData::new("cn");
        cn.insert_value("customProfileTextFonts", table.clone())
            .unwrap();
        let mut jp = JsonMasterData::new("jp");
        jp.insert_value("customProfileTextFonts", table).unwrap();
        assert_eq!(cn.resolve_font(1).as_deref(), Some("FZLanTingHei-DB-GBK"));
        assert_eq!(jp.resolve_font(1).as_deref(), Some("FOT-RodinNTLGPro-DB"));
    }
    #[test]
    fn missing_honor_group_fields_remain_absent() {
        let mut data = JsonMasterData::new("cn");
        data.insert_value("honors", serde_json::json!([{ "id": 3, "assetbundleName": "honor_sample", "honorRarity": "high", "groupId": 4, "levels": [], "honorMissionType": null }])).unwrap();
        data.insert_value(
            "honorGroups",
            serde_json::json!([{ "id": 4, "honorType": "character" }]),
        )
        .unwrap();
        let honor = data.resolve_honor(3, 1).unwrap();
        assert_eq!(honor.background_asset_bundle_name, None);
        assert_eq!(honor.frame_name, None);
    }

    #[test]
    fn cn_limited_event_top_honor_resolves_shared_background_and_rank_overlay() {
        let mut data = JsonMasterData::new("cn");
        data.insert_value("honors", serde_json::json!([{ "id": 10140, "assetbundleName": "honor_top_000020", "honorRarity": "low", "groupId": 10034, "levels": [{"level": 1}], "honorMissionType": null }])).unwrap();
        data.insert_value(
            "honorGroups",
            serde_json::json!([{ "id": 10034, "honorType": "limitevent" }]),
        )
        .unwrap();
        let honor = data.resolve_honor(10140, 1).unwrap();
        assert_eq!(
            honor.effective_background_asset_bundle_name(),
            "honor_bg_event_cheerteam"
        );
        assert!(honor.has_rank_overlay());
    }

    #[test]
    fn from_hex_rejects_multi_byte_input_without_panicking() {
        // The length check counts bytes; a string slice at a non-char-boundary
        // index would panic on multi-byte input.
        assert_eq!(ResolvedColor::from_hex("日日"), None);
        assert_eq!(ResolvedColor::from_hex("#日日日"), None);
        assert_eq!(ResolvedColor::from_hex("#11223g"), None);
        assert_eq!(
            ResolvedColor::from_hex("#112233"),
            Some(ResolvedColor {
                r: 0x11,
                g: 0x22,
                b: 0x33,
                a: 255
            })
        );
    }

    #[test]
    fn live_master_honor_falls_back_to_the_last_known_level() {
        // The game resolves a level the masterdata does not list yet to the
        // last known level instead of dropping the honor or clearing its
        // asset bundle name.
        let mut data = JsonMasterData::new("cn");
        data.insert_value(
            "honors",
            serde_json::json!([{
                "id": 7,
                "assetbundleName": null,
                "honorRarity": null,
                "groupId": null,
                "honorMissionType": "master_full_perfect",
                "levels": [
                    { "level": 1, "assetbundleName": "honor_live_1", "honorRarity": "low" },
                    { "level": 5, "assetbundleName": "honor_live_5", "honorRarity": "high" },
                ],
            }]),
        )
        .unwrap();
        let honor = data.resolve_honor(7, 9).unwrap();
        assert!(honor.is_live_master);
        assert_eq!(honor.asset_bundle_name, "honor_live_5");
        assert_eq!(honor.honor_rarity, "high");
        // A listed level still resolves exactly.
        let honor = data.resolve_honor(7, 1).unwrap();
        assert_eq!(honor.asset_bundle_name, "honor_live_1");
        assert_eq!(honor.honor_rarity, "low");
    }

    #[test]
    fn region_aliases_canonicalize_to_the_font_mapping_region() {
        let table = serde_json::json!([{ "id": 1, "fontName": "FOT-RodinNTLGPro-DB" }]);
        for region in ["CN", "zh-cn", "sc"] {
            let mut data = JsonMasterData::new(region);
            data.insert_value("customProfileTextFonts", table.clone())
                .unwrap();
            assert_eq!(
                data.resolve_font(1).as_deref(),
                Some("FZLanTingHei-DB-GBK"),
                "region {region} must map the CN font"
            );
        }
    }
}
