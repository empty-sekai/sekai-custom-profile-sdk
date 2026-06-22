//! 服务器 region 枚举与本地化兜底标签。
//!
//! masterdata 表（`customProfilePlayerInfoResources.name` 等）已对各服本地化，
//! 引擎首选查表；本模块提供**表外**硬编码标签的 per-region 兜底，例如：
//! - type=17 玩家等级面板的 "等级" 标签（masterdata 无对应资源条目）
//! - `char_rank` 的两个 tab 子标签（id=11 的 name 字段是连体串，分隔符各服不同）
//! - `mvp_superstar` 的 "{}次" 后缀
//! - `music_clear` / `music_clear_tab` 的三段标题（完成 / Full Combo / AP）
//!
//! 此外提供 CJK fallback 字体名（`Noto Sans CJK SC/JP/TC/KR`），
//! 供 `draw_general_text` 在主字体不支持某字符时按 region 切换 fallback。

/// sekai 5 个服务器 region。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Region {
    /// 国服（简体中文，方正字体）
    Cn,
    /// 日服
    Jp,
    /// 繁中服（台服）
    Tw,
    /// 韩服
    Kr,
    /// 国际服（英文）
    En,
}

impl Region {
    /// 字符串编码（用于 CLI `--region` / 日志 / 配置序列化）。
    pub fn as_str(self) -> &'static str {
        match self {
            Region::Cn => "cn",
            Region::Jp => "jp",
            Region::Tw => "tw",
            Region::Kr => "kr",
            Region::En => "en",
        }
    }

    /// 从字符串解析 region。未知值返回 `None`。
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "cn" | "sc" => Some(Region::Cn),
            "jp" => Some(Region::Jp),
            "tw" | "tc" => Some(Region::Tw),
            "kr" => Some(Region::Kr),
            "en" | "world" => Some(Region::En),
            _ => None,
        }
    }

    /// 是否国服。国服独有 FOT→FZ 字体名映射。
    pub fn is_cn(self) -> bool {
        matches!(self, Region::Cn)
    }

    /// CJK fallback 字体族名（主字体不支持某字符时使用）。
    pub fn cjk_fallback_font(self) -> &'static str {
        match self {
            Region::Cn => "Noto Sans CJK SC",
            Region::Jp => "Noto Sans CJK JP",
            Region::Tw => "Noto Sans CJK TC",
            Region::Kr => "Noto Sans CJK KR",
            Region::En => "Noto Sans CJK JP",
        }
    }

    /// 表外本地化标签集（masterdata 无对应字段的硬编码标签）。
    pub fn labels(self) -> RegionLabels {
        RegionLabels::for_region(self)
    }
}

impl Default for Region {
    /// 默认国服（保留内网历史行为）。
    fn default() -> Self {
        Region::Cn
    }
}

/// per-region 的表外兜底标签。
///
/// 这些标签在 masterdata 中没有干净的对应字段（要么无条目，要么是连体串），
/// 故引擎内置 5 服本地化字符串。
#[derive(Debug, Clone, Copy)]
pub struct RegionLabels {
    region: Region,
}

impl RegionLabels {
    pub fn for_region(region: Region) -> Self {
        Self { region }
    }

    /// type=17 玩家等级面板左侧 "等级" 标签。
    pub fn player_level_label(self) -> &'static str {
        match self.region {
            Region::Cn => "等级",
            Region::Jp => "レベル",
            Region::Tw => "等級",
            Region::Kr => "레벨",
            Region::En => "Lv",
        }
    }

    /// `char_rank` 左 tab：角色收藏等级（选中态）。
    pub fn char_rank_tab_active(self) -> &'static str {
        match self.region {
            Region::Cn => "角色收藏等级",
            Region::Jp => "キャラクターランク",
            Region::Tw => "角色收藏等級",
            Region::Kr => "캐릭터 랭크",
            Region::En => "Character Rank",
        }
    }

    /// `char_rank` 右 tab：挑战舞台（非选中态）。
    pub fn char_rank_tab_inactive(self) -> &'static str {
        match self.region {
            Region::Cn => "挑战舞台",
            Region::Jp => "チャレンジステージ",
            Region::Tw => "挑戰舞臺",
            Region::Kr => "챌린지 스테이지",
            Region::En => "Challenge Level",
        }
    }

    /// `mvp_superstar` 的 "{}次" 后缀（`{}` 为数字占位）。
    pub fn mvp_count_suffix(self) -> &'static str {
        match self.region {
            Region::Cn => "次",
            Region::Jp => "回",
            Region::Tw => "次",
            Region::Kr => "회",
            Region::En => "x",
        }
    }

    /// `challenge_live` 的 "独奏" 图标标签（表外硬编码）。
    pub fn challenge_solo_label(self) -> &'static str {
        match self.region {
            Region::Cn => "独奏",
            Region::Jp => "ソロ",
            Region::Tw => "獨奏",
            Region::Kr => "솔로",
            Region::En => "Solo",
        }
    }

    /// `music_clear`（type=12 详细版）三段标题。
    ///
    /// 与 [`Self::music_clear_labels`] 的区别：type=12 用全大写 "FULL COMBO"
    /// （1:1 复原游戏内显示），type=16 tab 用首字母大写 "Full Combo"。
    pub fn music_clear_detail_labels(self) -> [&'static str; 3] {
        match self.region {
            Region::Cn => ["完成", "FULL COMBO", "AP"],
            Region::Jp => ["クリア", "FULL COMBO", "AP"],
            Region::Tw => ["過關", "FULL COMBO", "AP"],
            Region::Kr => ["클리어", "FULL COMBO", "AP"],
            Region::En => ["Clear", "FULL COMBO", "AP"],
        }
    }

    /// `music_clear_tab`（type=16 tab 版）底部三段标题。
    ///
    /// 返回 `[clear_label, full_combo_label, all_perfect_label]`。
    pub fn music_clear_labels(self) -> [&'static str; 3] {
        match self.region {
            Region::Cn => ["完成", "Full Combo", "AP"],
            Region::Jp => ["クリア", "Full Combo", "AP"],
            Region::Tw => ["過關", "Full Combo", "AP"],
            Region::Kr => ["클리어", "Full Combo", "AP"],
            Region::En => ["Clear", "Full Combo", "AP"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_roundtrip() {
        for r in [
            Region::Cn,
            Region::Jp,
            Region::Tw,
            Region::Kr,
            Region::En,
        ] {
            assert_eq!(Region::from_str(r.as_str()), Some(r));
        }
    }

    #[test]
    fn region_aliases() {
        assert_eq!(Region::from_str("SC"), Some(Region::Cn));
        assert_eq!(Region::from_str("TC"), Some(Region::Tw));
        assert_eq!(Region::from_str("world"), Some(Region::En));
        assert_eq!(Region::from_str("unknown"), None);
    }

    #[test]
    fn default_is_cn() {
        assert_eq!(Region::default(), Region::Cn);
    }

    #[test]
    fn cjk_fallback_matches_region() {
        assert_eq!(Region::Cn.cjk_fallback_font(), "Noto Sans CJK SC");
        assert_eq!(Region::Jp.cjk_fallback_font(), "Noto Sans CJK JP");
        assert_eq!(Region::Tw.cjk_fallback_font(), "Noto Sans CJK TC");
        assert_eq!(Region::Kr.cjk_fallback_font(), "Noto Sans CJK KR");
    }

    #[test]
    fn cn_labels_match_legacy_hardcoded() {
        let l = Region::Cn.labels();
        assert_eq!(l.player_level_label(), "等级");
        assert_eq!(l.char_rank_tab_active(), "角色收藏等级");
        assert_eq!(l.char_rank_tab_inactive(), "挑战舞台");
        assert_eq!(l.mvp_count_suffix(), "次");
        assert_eq!(l.challenge_solo_label(), "独奏");
        // type=12 详细版用全大写 FULL COMBO（1:1 复原游戏内显示）
        assert_eq!(l.music_clear_detail_labels(), ["完成", "FULL COMBO", "AP"]);
        // type=16 tab 版用首字母大写 Full Combo
        assert_eq!(l.music_clear_labels(), ["完成", "Full Combo", "AP"]);
    }

    #[test]
    fn en_labels_differ_from_cn() {
        let l = Region::En.labels();
        assert_ne!(l.player_level_label(), "等级");
        assert_ne!(l.char_rank_tab_active(), "角色收藏等级");
        assert_eq!(l.music_clear_labels(), ["Clear", "Full Combo", "AP"]);
    }
}
