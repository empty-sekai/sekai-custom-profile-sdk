//! MasterData 查询抽象。

use std::sync::Arc;

use crate::types::{BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry};

/// 解析后的颜色值（RGBA）。
#[derive(Debug, Clone, Copy)]
pub struct ResolvedColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl ResolvedColor {
    /// 从 `#RRGGBB` 或 `#RRGGBBAA` 格式解析颜色。
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim_start_matches('#');
        let len = hex.len();
        if len != 6 && len != 8 {
            return None;
        }
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        let a = if len == 8 {
            u8::from_str_radix(&hex[6..8], 16).ok()?
        } else {
            255
        };
        Some(Self { r, g, b, a })
    }
}

/// 资源信息。
#[derive(Debug, Clone)]
pub struct ResourceInfo {
    pub file_name: String,
    pub load_val: String,
    pub resource_type: String,
}

/// 解析后的 Honor 渲染信息。
#[derive(Debug, Clone)]
pub struct ResolvedHonor {
    pub asset_bundle_name: String,
    pub honor_rarity: String,
    pub honor_type: String,
    pub background_asset_bundle_name: Option<String>,
    pub frame_name: Option<String>,
    pub is_live_master: bool,
    pub has_star: bool,
    pub honor_level: i32,
    pub honor_mission_type: Option<String>,
}

impl ResolvedHonor {
    /// Returns the bundle that owns the degree layer. CN limited-event fan
    /// honors ship `honor_top_*` as rank overlays only and share the cheer-team
    /// degree layer; their honorGroups rows omit backgroundAssetbundleName.
    pub fn effective_background_asset_bundle_name(&self) -> &str {
        allium_renderer_core::masterdata::effective_honor_background_asset_bundle_name(
            &self.honor_type,
            &self.asset_bundle_name,
            self.background_asset_bundle_name.as_deref(),
        )
    }

    pub fn has_rank_overlay(&self) -> bool {
        allium_renderer_core::masterdata::honor_has_rank_overlay(
            &self.honor_type,
            &self.asset_bundle_name,
            self.is_live_master,
        )
    }
}

#[cfg(test)]
mod resolved_honor_tests {
    use super::ResolvedHonor;

    fn honor(asset: &str, honor_type: &str, background: Option<&str>) -> ResolvedHonor {
        ResolvedHonor {
            asset_bundle_name: asset.into(),
            honor_rarity: "high".into(),
            honor_type: honor_type.into(),
            background_asset_bundle_name: background.map(str::to_owned),
            frame_name: None,
            is_live_master: false,
            has_star: false,
            honor_level: 1,
            honor_mission_type: None,
        }
    }

    #[test]
    fn cn_limited_event_top_honor_uses_shared_cheer_team_degree_layer() {
        let resolved = honor("honor_top_000020", "limitevent", None);
        assert_eq!(
            resolved.effective_background_asset_bundle_name(),
            "honor_bg_event_cheerteam"
        );
        assert!(resolved.has_rank_overlay());
    }

    #[test]
    fn explicit_background_remains_authoritative() {
        let resolved = honor("honor_top_000020", "limitevent", Some("honor_bg_explicit"));
        assert_eq!(
            resolved.effective_background_asset_bundle_name(),
            "honor_bg_explicit"
        );
    }
}

/// 渲染引擎所需的最小 MasterData 查询契约。
pub trait MasterDataProvider: Send + Sync {
    fn resolve_story_banner(&self, story_type: &str, story_id: i32) -> Option<String>;
    fn get_card(&self, card_id: i32) -> Option<CardEntry>;
    fn resolve_color(&self, color_id: i32) -> Option<ResolvedColor>;
    fn resolve_font(&self, font_id: i32) -> Option<String>;
    fn resolve_stamp(&self, stamp_id: i32) -> Option<String>;
    fn resolve_resource(&self, res_type: &str, id: i32) -> Option<ResourceInfo>;
    fn resolve_honor(&self, honor_id: i32, honor_level: i32) -> Option<ResolvedHonor>;
    fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry>;
    fn get_bonds_honor_word(&self, word_id: i64) -> Option<BondsHonorWordEntry>;
    fn get_honor(&self, honor_id: i32) -> Option<HonorEntry>;
    fn resolve_unit_vs_sd(&self, self_id: i32, partner_id: i32) -> i32;
    fn font_count(&self) -> usize;
    fn color_count(&self) -> usize;

    /// 当前 region（默认国服，保留内网历史行为）。
    ///
    /// 驱动：
    /// - `map_font_name` 的 FOT→FZ 映射仅在 CN 服生效；
    /// - `draw_general_text` 的 CJK fallback 字体族按 region 切换；
    /// - `RegionLabels` 的表外兜底标签按 region 取。
    fn region(&self) -> crate::region::Region {
        crate::region::Region::Cn
    }

    /// 查 `customProfilePlayerInfoResources[id].name` 取面板标题本地化文本。
    ///
    /// 用于 general 面板标题（综合力 / 个性签名 / 挑战演出 / 多人演出 /
    /// 最喜欢的剧情 / 玩家名称）。默认实现返回 `None`，由 host 侧
    /// provider 覆盖（直接查原始 JSON 表的 `name` 字段）；表缺失或字段
    /// 缺失时调用方走 `RegionLabels` 兜底。
    fn resolve_player_info_label(&self, _id: i32) -> Option<String> {
        None
    }

    fn resolve_asset_path(&self, element_type: &str, id: i32) -> String {
        match element_type {
            "etc" | "collection" | "general_bg" | "standing" | "player_info" | "story_bg" => {
                if let Some(info) = self.resolve_resource(element_type, id) {
                    return format!("{}/{}.png", info.load_val, info.file_name);
                }
                format!("{}/{}.png", element_type, id)
            }
            "card_member" => format!("card_member/{}.png", id),
            "stamp" => format!("stamp/{}.png", id),
            "honor" => format!("honor/{}.png", id),
            "bonds_honor" => format!("bonds_honor/{}.png", id),
            _ => format!("{}/{}.png", element_type, id),
        }
    }
}

/// 渲染时使用的 MasterData 快照包装。
#[derive(Clone)]
pub struct MasterData {
    provider: Arc<dyn MasterDataProvider>,
}

impl MasterData {
    /// 从 provider 构建快照。
    pub fn new(provider: Arc<dyn MasterDataProvider>) -> Self {
        Self { provider }
    }

    pub fn resolve_story_banner(&self, story_type: &str, story_id: i32) -> Option<String> {
        self.provider.resolve_story_banner(story_type, story_id)
    }

    pub fn get_card(&self, card_id: i32) -> Option<CardEntry> {
        self.provider.get_card(card_id)
    }

    pub fn resolve_color(&self, color_id: i32) -> Option<ResolvedColor> {
        self.provider.resolve_color(color_id)
    }

    pub fn resolve_font(&self, font_id: i32) -> Option<String> {
        self.provider.resolve_font(font_id)
    }

    pub fn resolve_stamp(&self, stamp_id: i32) -> Option<String> {
        self.provider.resolve_stamp(stamp_id)
    }

    pub fn resolve_resource(&self, res_type: &str, id: i32) -> Option<ResourceInfo> {
        self.provider.resolve_resource(res_type, id)
    }

    pub fn resolve_honor(&self, honor_id: i32, honor_level: i32) -> Option<ResolvedHonor> {
        self.provider.resolve_honor(honor_id, honor_level)
    }

    pub fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry> {
        self.provider.get_bonds_honor(id)
    }

    pub fn get_bonds_honor_word(&self, word_id: i64) -> Option<BondsHonorWordEntry> {
        self.provider.get_bonds_honor_word(word_id)
    }

    pub fn get_honor(&self, honor_id: i32) -> Option<HonorEntry> {
        self.provider.get_honor(honor_id)
    }

    pub fn resolve_unit_vs_sd(&self, self_id: i32, partner_id: i32) -> i32 {
        self.provider.resolve_unit_vs_sd(self_id, partner_id)
    }

    pub fn font_count(&self) -> usize {
        self.provider.font_count()
    }

    pub fn color_count(&self) -> usize {
        self.provider.color_count()
    }

    pub fn resolve_asset_path(&self, element_type: &str, id: i32) -> String {
        self.provider.resolve_asset_path(element_type, id)
    }

    /// 当前 region。见 [`MasterDataProvider::region`]。
    pub fn region(&self) -> crate::region::Region {
        self.provider.region()
    }

    /// 面板标题本地化文本。见 [`MasterDataProvider::resolve_player_info_label`]。
    pub fn resolve_player_info_label(&self, id: i32) -> Option<String> {
        self.provider.resolve_player_info_label(id)
    }

    /// 表外兜底标签集（语法糖）。
    pub fn labels(&self) -> crate::region::RegionLabels {
        self.region().labels()
    }
}
