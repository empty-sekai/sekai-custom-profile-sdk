//! Card render theme tokens.

/// Font families declared by the game's custom profile assets.
pub mod fonts {
    /// 主字体（游戏内标准 UI 字体）。
    pub const PRIMARY: &str = "FZLanTingHei-DB-GBK";
    /// 粗体强调字体。
    pub const EMPHASIS: &str = "FZZhengHei-EB-GBK";
    /// Family for the Live Master progress number.
    ///
    /// That number is plain UI text in the game rather than TMP text, so it is
    /// drawn with an open-licensed CJK sans instead of one of the game's own
    /// display faces.
    pub const LIVE_MASTER_PROGRESS: &str = "Source Han Sans SC";
}
