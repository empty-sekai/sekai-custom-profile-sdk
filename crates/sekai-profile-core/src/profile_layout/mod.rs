//! Layout constants for General panels.

mod cards;
mod extras;
mod header;
mod music;
mod stats;

/// Measured placement of an element inside a panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElementLayout {
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
}

/// Measured panel dimensions and child placements.
#[derive(Debug, PartialEq)]
pub struct PanelLayout {
    pub w: f32,
    pub h: f32,
    pub elements: &'static [ElementLayout],
}

pub use cards::{DECK, HONORS, LEADER_MEMBER};
pub use extras::{CHAR_RANK, STORY_FAVORITE};
pub use header::{COMMENT, PLAYER_NAME, TOTAL_POWER};
pub use music::{MUSIC_CLEAR, MUSIC_CLEAR_TAB};
pub use stats::{CHALLENGE_LIVE, MVP_SUPERSTAR};
