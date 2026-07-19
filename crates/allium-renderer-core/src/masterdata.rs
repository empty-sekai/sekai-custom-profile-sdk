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
        if value.len() != 6 && value.len() != 8 {
            return None;
        }
        Some(Self {
            r: u8::from_str_radix(&value[0..2], 16).ok()?,
            g: u8::from_str_radix(&value[2..4], 16).ok()?,
            b: u8::from_str_radix(&value[4..6], 16).ok()?,
            a: if value.len() == 8 {
                u8::from_str_radix(&value[6..8], 16).ok()?
            } else {
                255
            },
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

impl ResolvedHonor {
    pub fn has_rank_overlay(&self) -> bool {
        self.is_live_master
            || matches!(self.honor_type.as_str(), "rank_match" | "sekai_echo")
            || [
                "honor_top_",
                "honor_shining",
                "honor_memorial",
                "honor_memory",
            ]
            .iter()
            .any(|prefix| self.asset_bundle_name.starts_with(prefix))
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

/// Parsed table collection. It contains no network or filesystem policy.
#[derive(Clone, Debug)]
pub struct JsonMasterData {
    region: String,
    tables: BTreeMap<String, JsonTable>,
}

impl JsonMasterData {
    pub fn new(region: impl Into<String>) -> Self {
        Self {
            region: region.into(),
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
        let level = honor.levels.iter().find(|entry| entry.level == honor_level);
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
}
