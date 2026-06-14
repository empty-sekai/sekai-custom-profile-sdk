//! 通用颜色面板组件。

use crate::context::RenderContext;

use super::theme::Color;
use super::Widget;

/// 圆角颜色面板。
pub struct Panel {
    /// 面板宽度。
    pub width: f32,
    /// 面板高度。
    pub height: f32,
    /// 圆角半径。
    pub radius: f32,
    /// 填充色。
    pub fill: Color,
    /// 边框色。
    pub border: Option<Color>,
    /// 边框宽度。
    pub border_width: f32,
}

impl Widget for Panel {
    fn name(&self) -> &'static str {
        "panel"
    }

    fn measure(&self, _ctx: &RenderContext<'_>) -> (f32, f32) {
        (self.width, self.height)
    }

    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, _ctx: &RenderContext<'_>) {
        let rect = skia_safe::Rect::from_xywh(x, y, self.width, self.height);

        let mut fill = skia_safe::Paint::default();
        fill.set_anti_alias(true);
        fill.set_style(skia_safe::PaintStyle::Fill);
        fill.set_color4f(self.fill.to_skia(), None);
        canvas.draw_round_rect(rect, self.radius, self.radius, &fill);

        if let Some(border_color) = self.border {
            if self.border_width > 0.0 {
                let inset = self.border_width * 0.5;
                let border_rect = skia_safe::Rect::from_xywh(
                    x + inset,
                    y + inset,
                    (self.width - self.border_width).max(0.0),
                    (self.height - self.border_width).max(0.0),
                );
                let mut border = skia_safe::Paint::default();
                border.set_anti_alias(true);
                border.set_style(skia_safe::PaintStyle::Stroke);
                border.set_stroke_width(self.border_width);
                border.set_color4f(border_color.to_skia(), None);
                canvas.draw_round_rect(border_rect, self.radius, self.radius, &border);
            }
        }
    }
}
