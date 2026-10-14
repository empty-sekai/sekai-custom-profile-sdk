//! A [`MasterDataProvider`] backed by the game's JSON tables.
//!
//! It reads `cards`, `stamps`, `honors`, `honorGroups`, `bondsHonors`,
//! `bondsHonorWords`, `gameCharacterUnits`, the `customProfile*` color, font
//! and resource tables, `eventStories` and `unitStoryEpisodeGroups`
//! ([`REQUIRED_TABLES`]), plus the tables a set may lack
//! ([`OPTIONAL_TABLES`]): three icon resource tables that only some regions
//! ship, and `omikujis`, read by omikuji collection items.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use super::{
    CollectionResourceType, MasterDataProvider, OmikujiRow, ResolvedColor, ResolvedHonor,
    ResourceInfo,
};
use crate::region::Region;
use crate::types::{BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry, StampEntry};

use super::table::Table;

/// Names of the tables card rendering reads.
pub const REQUIRED_TABLES: &[&str] = &[
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
    "customProfilePlayerInfoResources",
    "customProfileStoryBackgroundResources",
    "eventStories",
    "unitStoryEpisodeGroups",
];

/// Tables a set may lack; the same list as
/// `sekai_profile_core::masterdata::PROFILE_OPTIONAL_MASTERDATA_TABLES`. When
/// one is missing, elements that reference it are treated as referencing a
/// missing row and are not drawn, and the table is not reported by
/// [`JsonMasterDataProvider::missing_tables`].
pub const OPTIONAL_TABLES: &[&str] = crate::core::masterdata::PROFILE_OPTIONAL_MASTERDATA_TABLES;

/// A [`MasterDataProvider`] built from a set of JSON tables.
///
/// Load a directory of `<table>.json` files with
/// [`JsonMasterDataProvider::from_dir`], or start from
/// [`JsonMasterDataProvider::empty`] and add tables with
/// [`JsonMasterDataProvider::insert_table`]. Set the game region with
/// [`JsonMasterDataProvider::with_region`]; it defaults to
/// [`Region::Cn`], and decides how font names map to font families.
pub struct JsonMasterDataProvider {
    tables: HashMap<String, Arc<Table>>,
    region: Region,
}

impl JsonMasterDataProvider {
    /// Builds a provider from parsed tables keyed by table name. The region
    /// is [`Region::Cn`].
    pub fn new(tables: HashMap<String, Arc<Table>>) -> Self {
        Self {
            tables,
            region: Region::Cn,
        }
    }

    /// Sets the region of the game client the tables come from.
    ///
    /// The renderer reads it through [`MasterDataProvider::region`]. It also
    /// selects the font family names [`MasterDataProvider::resolve_font`]
    /// returns. Panel label text comes from `sekai_profile_core::locale`.
    pub fn with_region(mut self, region: Region) -> Self {
        self.region = region;
        self
    }

    /// An empty provider, the starting point for adding tables one by one with
    /// [`Self::insert_table`]. The region is [`Region::Cn`].
    pub fn empty() -> Self {
        Self {
            tables: HashMap::new(),
            region: Region::Cn,
        }
    }

    /// Adds or replaces the table `name` from its JSON text: an array of rows;
    /// any other JSON value is stored as a single row.
    ///
    /// # Errors
    ///
    /// Returns an error when `json` is not valid JSON.
    pub fn insert_table(&mut self, name: &str, json: &str) -> Result<(), String> {
        let table =
            Table::from_json(json).map_err(|e| format!("failed to parse table {name}: {e}"))?;
        self.tables.insert(name.to_string(), Arc::new(table));
        Ok(())
    }

    /// Loads `<dir>/<table>.json` for every name in [`REQUIRED_TABLES`] and
    /// [`OPTIONAL_TABLES`]. A missing required table is skipped with a
    /// warning, and elements that need it are treated as unmapped; a missing
    /// optional table is skipped without a warning. [`Self::missing_tables`]
    /// lists the required tables that were skipped.
    ///
    /// # Errors
    ///
    /// Returns an error when a table file cannot be read or parsed, or when
    /// `dir` contains none of the known tables.
    pub fn from_dir(dir: &Path) -> Result<Self, String> {
        let mut provider = Self::empty();
        for name in REQUIRED_TABLES.iter().chain(OPTIONAL_TABLES) {
            let path = dir.join(format!("{name}.json"));
            if !path.exists() {
                if OPTIONAL_TABLES.contains(name) {
                    tracing::debug!(table = name, path = %path.display(), "optional masterdata table not found");
                } else {
                    tracing::warn!(table = name, path = %path.display(), "masterdata table missing");
                }
                continue;
            }
            let json = std::fs::read_to_string(&path)
                .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
            provider.insert_table(name, &json)?;
        }
        if provider.tables.is_empty() {
            return Err(format!(
                "{} contains no known masterdata table",
                dir.display()
            ));
        }
        Ok(provider)
    }

    /// Names of the loaded tables, in no particular order.
    pub fn loaded_tables(&self) -> Vec<&str> {
        self.tables.keys().map(|s| s.as_str()).collect()
    }

    /// Names in [`REQUIRED_TABLES`] that are not loaded.
    pub fn missing_tables(&self) -> Vec<&'static str> {
        REQUIRED_TABLES
            .iter()
            .filter(|name| !self.tables.contains_key(**name))
            .copied()
            .collect()
    }

    /// The loaded table `name`, if any.
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.get(name).map(Arc::as_ref)
    }

    /// Font families the `customProfileTextFonts` table references, named as
    /// the renderer resolves them for the current region, sorted and
    /// deduplicated.
    pub fn text_font_families(&self) -> Vec<String> {
        let Some(table) = self.table("customProfileTextFonts") else {
            return Vec::new();
        };
        let mut families: Vec<String> = table
            .all()
            .iter()
            .filter_map(|row| row.get("fontName")?.as_str())
            .map(|name| self.map_font_name(name).to_string())
            .collect();
        families.sort();
        families.dedup();
        families
    }

    fn typed<T: serde::de::DeserializeOwned>(&self, table: &str, id: i64) -> Option<T> {
        self.table(table)?.typed(id)
    }

    /// Maps an FOT font name to the FZ family the CN client uses.
    ///
    /// Only [`Region::Cn`] maps: its font files are FZ faces, while the
    /// masterdata `fontName` field keeps the FOT names. Other regions return
    /// the FOT name unchanged, since their font files are the FOT faces.
    fn map_font_name<'a>(&self, name: &'a str) -> &'a str {
        if !self.region.is_cn() {
            return name;
        }
        match name {
            "FOT-RodinNTLGPro-DB" => "FZLanTingHei-DB-GBK",
            "FOT-SkipProN-B" => "FZZhengHei-EB-GBK",
            "FOT-PopHappinessStd-EB" => "FZShaoEr-M11-JF",
            other => other,
        }
    }
}

impl MasterDataProvider for JsonMasterDataProvider {
    fn resolve_story_banner(&self, story_type: &str, story_id: i32) -> Option<String> {
        let id = story_id as i64;
        match story_type {
            "event_story" => {
                let t = self.table("eventStories")?;
                let abn = t.by_id(id)?["assetbundleName"].as_str()?;
                Some(format!("event_story/{abn}/screen_image/banner_event_story"))
            }
            "unit_story" => {
                let t = self.table("unitStoryEpisodeGroups")?;
                let abn = t.by_id(id)?["assetbundleName"].as_str()?;
                Some(format!("unit_story/{abn}/screen_image/banner_unit_story"))
            }
            _ => None,
        }
    }

    fn get_card(&self, card_id: i32) -> Option<CardEntry> {
        self.typed("cards", card_id as i64)
    }

    fn resolve_color(&self, color_id: i32) -> Option<ResolvedColor> {
        let t = self.table("customProfileTextColors")?;
        let v = t.by_id(color_id as i64)?;
        ResolvedColor::from_hex(v["colorCode"].as_str()?)
    }

    fn default_color(&self) -> Option<ResolvedColor> {
        let t = self.table("customProfileTextColors")?;
        ResolvedColor::from_hex(t.all().first()?["colorCode"].as_str()?)
    }

    fn resolve_font(&self, font_id: i32) -> Option<String> {
        let t = self.table("customProfileTextFonts")?;
        let name = t.by_id(font_id as i64)?["fontName"].as_str()?;
        Some(self.map_font_name(name).to_string())
    }

    fn resolve_stamp(&self, stamp_id: i32) -> Option<String> {
        let entry: StampEntry = self.typed("stamps", stamp_id as i64)?;
        Some(entry.assetbundle_name)
    }

    fn resolve_resource(&self, res_type: &str, id: i32) -> Option<ResourceInfo> {
        let table_name = match res_type {
            "shape" => "customProfileShapeResources",
            "etc" => "customProfileEtcResources",
            "collection" => "customProfileCollectionResources",
            "general_bg" => "customProfileGeneralBackgroundResources",
            "standing" => "customProfileMemberStandingPictureResources",
            "player_info" => "customProfilePlayerInfoResources",
            "story_bg" => "customProfileStoryBackgroundResources",
            "character_icon" => "customProfileCharacterIconResources",
            "material" => "customProfileMaterialResources",
            "user_interface_icon" => "customProfileUserInterfaceIconResources",
            _ => return None,
        };
        let t = self.table(table_name)?;
        let v = t.by_id(id as i64)?;
        Some(ResourceInfo {
            file_name: v["fileName"].as_str()?.to_string(),
            load_val: v["resourceLoadVal"].as_str()?.to_string(),
            resource_type: v["customProfileResourceType"].as_str()?.to_string(),
            collection_type: CollectionResourceType::parse(
                v["customProfileResourceCollectionType"].as_str(),
            ),
        })
    }

    fn region(&self) -> Region {
        self.region
    }

    fn resolve_player_info_label(&self, id: i32) -> Option<String> {
        let t = self.table("customProfilePlayerInfoResources")?;
        let v = t.by_id(id as i64)?;
        v["name"].as_str().map(|s| s.to_string())
    }

    fn resolve_omikuji(&self, id: i32) -> Option<OmikujiRow> {
        self.typed("omikujis", i64::from(id))
    }

    fn resolve_honor(&self, honor_id: i32, honor_level: i32) -> Option<ResolvedHonor> {
        let honor = self.table("honors")?.by_id(honor_id as i64)?;
        let group = honor
            .get("groupId")
            .and_then(serde_json::Value::as_i64)
            .and_then(|id| self.table("honorGroups")?.by_id(id));
        crate::core::masterdata::resolve_honor_rows(honor, group, honor_level)
            .map(|value| ResolvedHonor::from_core(value, honor_level))
    }

    fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry> {
        self.typed("bondsHonors", id as i64)
    }

    fn get_bonds_honor_word(&self, word_id: i64) -> Option<BondsHonorWordEntry> {
        self.typed("bondsHonorWords", word_id)
    }

    fn get_honor(&self, honor_id: i32) -> Option<HonorEntry> {
        self.typed("honors", honor_id as i64)
    }

    fn resolve_unit_vs_sd(&self, self_id: i32, partner_id: i32) -> i32 {
        let Some(units_table) = self.table("gameCharacterUnits") else {
            return self_id;
        };
        let Some(self_unit) = units_table.by_id(self_id as i64) else {
            return self_id;
        };
        let self_char_id = self_unit["gameCharacterId"].as_i64().unwrap_or(0);
        if self_char_id < 21 {
            return self_id;
        }
        let Some(partner_unit) = units_table.by_id(partner_id as i64) else {
            return self_id;
        };
        let Some(target_unit) = partner_unit["unit"].as_str() else {
            return self_id;
        };
        for entry in units_table.all() {
            let cid = entry["gameCharacterId"].as_i64().unwrap_or(0);
            let u = entry["unit"].as_str().unwrap_or("");
            if cid == self_char_id && u == target_unit {
                return entry["id"].as_i64().unwrap_or(self_id as i64) as i32;
            }
        }
        self_id
    }

    fn font_count(&self) -> usize {
        self.table("customProfileTextFonts")
            .map(|t| t.len())
            .unwrap_or(0)
    }

    fn color_count(&self) -> usize {
        self.table("customProfileTextColors")
            .map(|t| t.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests;
