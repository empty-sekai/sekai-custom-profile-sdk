//! General 面板布局常量。

mod cards;
mod extras;
mod header;
mod music;
mod stats;

/// 面板内元素的测绘数据。
#[derive(Clone, Copy)]
pub struct ElementLayout {
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
}

/// 面板测绘数据。
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
