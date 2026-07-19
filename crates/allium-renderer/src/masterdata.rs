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
