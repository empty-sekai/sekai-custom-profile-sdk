//! 渲染侧使用的最小 MasterData 条目类型。

use serde::Deserialize;

/// 颜色映射条目。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorEntry {
    pub id: i32,
    pub color_code: String,
    #[serde(default)]
    pub seq: i32,
}

/// 字体映射条目。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FontEntry {
    pub id: i32,
    pub font_name: String,
    pub name: String,
}

/// 通用资源条目。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceEntry {
    pub id: i32,
    pub name: String,
    pub file_name: String,
    pub resource_load_type: String,
    pub resource_load_val: String,
    pub custom_profile_resource_type: String,
    pub custom_profile_resource_collection_type: Option<String>,
    pub character_id: Option<i32>,
    pub group_id: Option<i32>,
    #[serde(default)]
    pub seq: i32,
    pub pronunciation: Option<String>,
}

/// 称号等级条目。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HonorLevelEntry {
    pub level: i32,
    pub assetbundle_name: Option<String>,
    pub honor_rarity: Option<String>,
    pub description: Option<String>,
}

/// 贴纸条目。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StampEntry {
    pub id: i32,
    pub assetbundle_name: String,
}

/// 称号条目。
#[derive(Debug, Clone, Deserialize)]
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

/// 称号分组条目。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HonorGroupEntry {
    pub id: i32,
    pub honor_type: String,
    pub background_assetbundle_name: Option<String>,
    pub frame_name: Option<String>,
}

/// 羁绊称号条目。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BondsHonorEntry {
    pub id: i32,
    pub game_character_unit_id1: i32,
    pub game_character_unit_id2: i32,
    pub honor_rarity: String,
    #[serde(default)]
    pub configurable_unit_virtual_singer: bool,
}

/// 羁绊称号文字条目。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BondsHonorWordEntry {
    pub id: i32,
    #[serde(rename = "assetbundleName")]
    pub assetbundle_name: String,
    pub bonds_group_id: i32,
    pub seq: i32,
}

/// 活动剧情条目。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventStoryEntry {
    pub id: i32,
    pub event_id: i32,
    pub assetbundle_name: String,
}

/// 组合剧情组条目。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitStoryGroupEntry {
    pub id: i32,
    pub assetbundle_name: String,
    pub unit: String,
}

/// 卡牌条目。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardEntry {
    pub id: i32,
    #[serde(rename = "assetbundleName")]
    pub asset_bundle_name: String,
    pub card_rarity_type: String,
    pub attr: String,
    pub character_id: i32,
}
