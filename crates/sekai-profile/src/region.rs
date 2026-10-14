//! Game server regions.
//!
//! Used by the CLI `--region` option and by
//! [`MasterDataProvider::region`](crate::masterdata::MasterDataProvider::region).
//! Panel label text comes from `sekai_profile_core::locale`.

/// The five game server regions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Region {
    /// Simplified Chinese server, whose text fonts are FZ faces. The default.
    #[default]
    Cn,
    /// Japanese server.
    Jp,
    /// Traditional Chinese server.
    Tw,
    /// Korean server.
    Kr,
    /// Global (English) server.
    En,
}

impl Region {
    /// The region code: `cn`, `jp`, `tw`, `kr` or `en`, as the CLI
    /// `--region` option, logs and configuration use it.
    pub fn as_str(self) -> &'static str {
        match self {
            Region::Cn => "cn",
            Region::Jp => "jp",
            Region::Tw => "tw",
            Region::Kr => "kr",
            Region::En => "en",
        }
    }

    /// Whether this is [`Region::Cn`].
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

    /// Parses a region code, ignoring case. Also accepts `sc` for
    /// [`Region::Cn`], `tc` for [`Region::Tw`] and `world` for [`Region::En`].
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
