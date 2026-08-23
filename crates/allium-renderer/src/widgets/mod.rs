//! Card element primitive layer.
//!
//! Element coordinates come from the authored `ObjectData.position` in the
//! profile JSON; `adapters` wraps each authored element kind as a `Widget`.

pub mod adapters;
pub mod card_util;
pub mod theme;

use crate::context::RenderContext;

/// Card element primitive trait.
///
/// The scene-graph interpreter drives every element through `measure()` + `draw()`.
pub trait Widget: Send + Sync {
    /// 返回组件类型名。
    fn name(&self) -> &'static str;

    /// 测量图元所需的宽高（不执行绘制）。
    fn measure(&self, ctx: &RenderContext<'_>) -> (f32, f32);

    /// 枚举该节点依赖的素材 key。
    fn asset_keys(&self, _ctx: &RenderContext<'_>) -> Vec<String> {
        Vec::new()
    }
}
