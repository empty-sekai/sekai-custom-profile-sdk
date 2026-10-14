//! 服务器 region 枚举。
//!
//! 供 CLI `--region` 参数与 host 侧 `MasterDataProvider::region()` 契约使用；
//! 面板标签本地化由 `sekai_profile_core::locale` 提供。

/// sekai 5 个服务器 region。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Region {
    /// 国服（简体中文，方正字体），默认值
    #[default]
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

    /// 是否国服。
    pub fn is_cn(self) -> bool {
        matches!(self, Region::Cn)
    }
}

/// A region string that is neither a region code nor one of its aliases.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown region {0}; expected cn, jp, tw, kr or en")]
pub struct UnknownRegion(pub String);

impl std::str::FromStr for Region {
    type Err = UnknownRegion;

    /// 解析 region 编码，大小写不敏感；另接受 `sc`（国服）、`tc`（繁中服）与
    /// `world`（国际服）。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "cn" | "sc" => Ok(Region::Cn),
            "jp" => Ok(Region::Jp),
            "tw" | "tc" => Ok(Region::Tw),
            "kr" => Ok(Region::Kr),
            "en" | "world" => Ok(Region::En),
            _ => Err(UnknownRegion(s.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_roundtrip() {
        for r in [Region::Cn, Region::Jp, Region::Tw, Region::Kr, Region::En] {
            assert_eq!(r.as_str().parse(), Ok(r));
        }
    }

    #[test]
    fn region_aliases() {
        assert_eq!("SC".parse(), Ok(Region::Cn));
        assert_eq!("TC".parse(), Ok(Region::Tw));
        assert_eq!("world".parse(), Ok(Region::En));
        assert_eq!(
            "unknown".parse::<Region>(),
            Err(UnknownRegion("unknown".into()))
        );
    }

    #[test]
    fn default_is_cn() {
        assert_eq!(Region::default(), Region::Cn);
    }
}
