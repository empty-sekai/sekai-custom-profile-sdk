//! Font families the compositor draws profile text with.

pub(crate) mod fonts {
    /// Family for the Live Master progress number.
    ///
    /// The game draws that number with digit sprites outside the static asset
    /// set, so the engine draws it as plain text with an open-licensed CJK sans
    /// instead of one of the game's own display faces.
    pub(crate) const LIVE_MASTER_PROGRESS: &str = "Source Han Sans SC";
}
