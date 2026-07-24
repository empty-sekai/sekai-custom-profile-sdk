//! 称号渲染公共模块。

mod bonds;
mod common;
mod standard;

pub use bonds::render_bonds_honor;
pub(crate) use standard::draw_live_master_progress_text;
pub use standard::{render_honor, render_static_honor};
