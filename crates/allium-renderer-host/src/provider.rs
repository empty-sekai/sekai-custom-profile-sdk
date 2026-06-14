//! JSON 表驱动的 MasterDataProvider。
//!
//! 表映射与生产网关适配层一致：cards / stamps / honors / honorGroups /
//! bondsHonors / bondsHonorWords / gameCharacterUnits / 7 张 customProfile*
//! 资源表 / eventStories / unitStoryEpisodeGroups。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use allium_renderer::masterdata::{
    MasterDataProvider, ResolvedColor, ResolvedHonor, ResourceInfo,
};
use allium_renderer::types::{
    BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry, HonorGroupEntry, StampEntry,
};

use crate::table::Table;

/// 名片渲染所需的全部表名。
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

/// 从 JSON 表集合构建的 MasterDataProvider。
pub struct JsonMasterDataProvider {
    tables: HashMap<String, Arc<Table>>,
}

impl JsonMasterDataProvider {
    /// 从已解析的表集合构建。
    pub fn new(tables: HashMap<String, Arc<Table>>) -> Self {
        Self { tables }
    }

    /// 空 provider（逐张 [`Self::insert_table`] 注入时的起点）。
    pub fn empty() -> Self {
        Self {
            tables: HashMap::new(),
        }
    }

    /// 注入或替换一张表（JSON 字符串）。
    pub fn insert_table(&mut self, name: &str, json: &str) -> Result<(), String> {
        let table = Table::from_json(json).map_err(|e| format!("解析表 {name} 失败: {e}"))?;
        self.tables.insert(name.to_string(), Arc::new(table));
        Ok(())
    }

    /// 从目录加载 `<dir>/<table>.json`。缺失的表记 warning 跳过，
    /// 渲染时对应元素按缺映射处理。
    pub fn from_dir(dir: &Path) -> Result<Self, String> {
        let mut provider = Self::empty();
        for name in REQUIRED_TABLES {
            let path = dir.join(format!("{name}.json"));
            if !path.exists() {
                tracing::warn!(table = name, path = %path.display(), "masterdata 表缺失");
                continue;
            }
            let json = std::fs::read_to_string(&path)
                .map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
            provider.insert_table(name, &json)?;
        }
        if provider.tables.is_empty() {
            return Err(format!("目录 {} 内没有任何已知的 masterdata 表", dir.display()));
        }
        Ok(provider)
    }

    /// 已加载的表名（缺表诊断用）。
    pub fn loaded_tables(&self) -> Vec<&str> {
        self.tables.keys().map(|s| s.as_str()).collect()
    }

    /// 缺失的必需表名。
    pub fn missing_tables(&self) -> Vec<&'static str> {
        REQUIRED_TABLES
            .iter()
            .filter(|name| !self.tables.contains_key(**name))
            .copied()
            .collect()
    }

    fn table(&self, name: &str) -> Option<&Arc<Table>> {
        self.tables.get(name)
    }

    fn typed<T: serde::de::DeserializeOwned>(&self, table: &str, id: i64) -> Option<T> {
        self.table(table)?.typed(id)
    }

    /// FOT 日文字体名 → CN 服方正字体名。
    fn map_font_name(name: &str) -> &str {
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

    fn resolve_font(&self, font_id: i32) -> Option<String> {
        let t = self.table("customProfileTextFonts")?;
        let name = t.by_id(font_id as i64)?["fontName"].as_str()?;
        Some(Self::map_font_name(name).to_string())
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
            _ => return None,
        };
        let t = self.table(table_name)?;
        let v = t.by_id(id as i64)?;
        Some(ResourceInfo {
            file_name: v["fileName"].as_str()?.to_string(),
            load_val: v["resourceLoadVal"].as_str()?.to_string(),
            resource_type: v["customProfileResourceType"].as_str()?.to_string(),
        })
    }

    fn resolve_honor(&self, honor_id: i32, honor_level: i32) -> Option<ResolvedHonor> {
        let honor: HonorEntry = self.typed("honors", honor_id as i64)?;
        let is_live_master = honor.honor_mission_type.is_some() && honor.assetbundle_name.is_none();
        let (abn, rarity) = if is_live_master {
            let lvl = honor.levels.iter().find(|l| l.level == honor_level);
            let a = lvl
                .and_then(|l| l.assetbundle_name.as_deref())
                .unwrap_or("")
                .to_string();
            let r = lvl
                .and_then(|l| l.honor_rarity.as_deref())
                .unwrap_or("low")
                .to_string();
            (a, r)
        } else {
            (
                honor.assetbundle_name.clone().unwrap_or_default(),
                honor.honor_rarity.clone().unwrap_or_else(|| "low".into()),
            )
        };
        let group: Option<HonorGroupEntry> = honor
            .group_id
            .and_then(|gid| self.typed("honorGroups", gid as i64));
        Some(ResolvedHonor {
            asset_bundle_name: abn,
            honor_rarity: rarity,
            honor_type: group
                .as_ref()
                .map(|g| g.honor_type.as_str())
                .unwrap_or("normal")
                .to_string(),
            background_asset_bundle_name: group
                .as_ref()
                .and_then(|g| g.background_assetbundle_name.clone()),
            frame_name: group.as_ref().and_then(|g| g.frame_name.clone()),
            is_live_master,
            has_star: honor.levels.len() > 1,
            honor_level,
            honor_mission_type: honor.honor_mission_type.clone(),
        })
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
mod tests {
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
        assert_eq!(p.resolve_font(1).as_deref(), Some("FZLanTingHei-DB-GBK"));
        assert_eq!(p.resolve_font(9).as_deref(), Some("Custom"));
    }

    #[test]
    fn missing_tables_reports_unloaded() {
        let p = provider_with("cards", "[]");
        assert!(p.missing_tables().contains(&"stamps"));
        assert!(!p.missing_tables().contains(&"cards"));
    }
}
