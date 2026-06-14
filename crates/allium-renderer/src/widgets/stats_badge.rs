//! 指标标签组件。

use crate::context::RenderContext;

#[cfg(feature = "skia-core")]
use super::text::TextAlign;
use super::theme::Color;
use super::Widget;

/// 指标标签图元。
pub struct StatsBadge {
    /// 指标名称。
    pub label: String,
    /// 指标值。
    pub value: String,
    /// 指标值颜色。
    pub color: Color,
    /// 是否使用霓虹高亮。
    pub is_highlight: bool,
}

impl StatsBadge {
    const LABEL_SIZE: f32 = 12.0;
    const VALUE_SIZE: f32 = 20.0;
    const WIDTH_PADDING: f32 = 12.0;
    const HEIGHT: f32 = 44.0;

    /// 创建指标标签组件。
    pub fn new(label: impl Into<String>, value: impl Into<String>, color: Color) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            color,
            is_highlight: false,
        }
    }

    fn text_width(text: &str, size: f32) -> f32 {
        #[cfg(feature = "skia-core")]
        {
            return crate::widgets::text::measure_text_width(text, size);
        }
        #[cfg(not(feature = "skia-core"))]
        {
            crate::widgets::text::estimate_text_width(text, size)
        }
    }
}

impl Widget for StatsBadge {
    /// 返回组件名称。
    fn name(&self) -> &'static str {
        "stats_badge"
    }

    /// 测量指标标签宽高。
    fn measure(&self, _ctx: &RenderContext<'_>) -> (f32, f32) {
        let label_w = Self::text_width(&self.label, Self::LABEL_SIZE);
        let value_w = Self::text_width(&self.value, Self::VALUE_SIZE);
        (
            label_w.max(value_w) + Self::WIDTH_PADDING * 2.0,
            Self::HEIGHT,
        )
    }

    /// 在指定位置绘制指标标签。
    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, ctx: &RenderContext<'_>) {
        let (width, height) = self.measure(ctx);

        crate::widgets::text::draw_label(
            canvas,
            &self.label,
            x,
            y + 11.5,
            Self::LABEL_SIZE,
            ctx.theme.colors.text_gray,
            TextAlign::Left,
        );

        if self.is_highlight {
            crate::widgets::text::draw_neon_text(
                canvas,
                &self.value,
                x + width / 2.0,
                y + 31.0,
                Self::VALUE_SIZE,
                self.color,
            );
        } else {
            crate::widgets::text::draw_label(
                canvas,
                &self.value,
                x,
                y + 31.0,
                Self::VALUE_SIZE,
                self.color,
                TextAlign::Left,
            );
        }

        let mut line = skia_safe::Paint::default();
        line.set_anti_alias(true);
        line.set_stroke_width(1.0);
        line.set_color4f(ctx.theme.colors.glass_edge.to_skia(), None);
        canvas.draw_line((x, y + height - 1.0), (x + width, y + height - 1.0), &line);
    }
}
