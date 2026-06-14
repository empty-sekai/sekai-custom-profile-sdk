//! 通用图元组件层。
//!
//! 提供渲染上下文无关的 UI 组件，可在不同布局模式下复用：
//! - 场景图模式：坐标来自 JSON ObjectData.position（名片渲染）
//! - 代码布局模式：坐标由 Rust 布局引擎计算（组卡/box 结果图）

pub mod adapters;
pub mod card_thumbnail;
pub mod card_util;
pub mod glass_panel;
pub mod image;
pub mod panel;
pub mod stats_badge;
pub mod text;
pub mod text_badge;
pub mod theme;

use crate::context::RenderContext;

/// 通用图元 trait。
///
/// 所有消费者（场景图解释器 / deck 布局引擎 / box 布局引擎）
/// 统一通过 `measure()` + `draw()` 使用图元。
pub trait Widget: Send + Sync {
    /// 返回组件类型名。
    fn name(&self) -> &'static str;

    /// 测量图元所需的宽高（不执行绘制）。
    fn measure(&self, ctx: &RenderContext<'_>) -> (f32, f32);

    /// 在 canvas 的 `(x, y)` 位置绘制图元。
    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, ctx: &RenderContext<'_>);

    /// 枚举该节点依赖的素材 key。
    fn asset_keys(&self, _ctx: &RenderContext<'_>) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::glass_panel::GlassPanel;
    use super::text::SimpleText;
    use super::theme::Color;
    use super::Widget;

    #[test]
    fn widgets_can_be_stored_as_dyn_trait_objects() {
        let widgets: Vec<Box<dyn Widget>> = vec![
            Box::new(GlassPanel::new(120.0, 48.0)),
            Box::new(SimpleText::new(
                "widget",
                16.0,
                Color::new(1.0, 1.0, 1.0, 1.0),
            )),
        ];

        assert_eq!(widgets.len(), 2);
        assert_eq!(widgets[0].name(), "glass_panel");
        assert_eq!(widgets[1].name(), "simple_text");
    }
}
