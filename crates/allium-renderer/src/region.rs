//! 服务器 region 枚举。
//!
//! 供 CLI `--region` 参数与 host 侧 `MasterDataProvider::region()` 契约使用；
//! 面板标签本地化由 `allium_renderer_core::locale` 提供。

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

    /// 是否国服。
    pub fn is_cn(self) -> bool {
        matches!(self, Region::Cn)
    }
}

impl Default for Region {
    /// 默认国服（保留内网历史行为）。
    fn default() -> Self {
        Region::Cn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_roundtrip() {
        for r in [Region::Cn, Region::Jp, Region::Tw, Region::Kr, Region::En] {
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
}
