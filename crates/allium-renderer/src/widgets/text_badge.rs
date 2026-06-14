//! 圆角文本标签组件。

use crate::context::RenderContext;

#[cfg(feature = "skia-core")]
use super::text::TextAlign;
use super::theme::Color;
use super::Widget;

/// 圆角文本标签图元。
pub struct TextBadge {
    /// 标签文本。
    pub text: String,
    /// 背景颜色。
    pub bg_color: Color,
    /// 文本颜色。
    pub text_color: Color,
}

impl TextBadge {
    const H_PADDING: f32 = 8.0;
    const HEIGHT: f32 = 24.0;
    #[cfg(feature = "skia-core")]
    const RADIUS: f32 = 4.0;
    const FONT_SIZE: f32 = 14.0;

    /// 创建圆角文本标签组件。
    pub fn new(text: impl Into<String>, bg_color: Color, text_color: Color) -> Self {
        Self {
            text: text.into(),
            bg_color,
            text_color,
        }
    }

    fn text_width(text: &str) -> f32 {
        #[cfg(feature = "skia-core")]
        {
            return crate::widgets::text::measure_text_width(text, Self::FONT_SIZE);
        }
        #[cfg(not(feature = "skia-core"))]
        {
            crate::widgets::text::estimate_text_width(text, Self::FONT_SIZE)
        }
    }
}

impl Widget for TextBadge {
    /// 返回组件名称。
    fn name(&self) -> &'static str {
        "text_badge"
    }

    /// 测量标签宽高。
    fn measure(&self, _ctx: &RenderContext<'_>) -> (f32, f32) {
        (
            Self::text_width(&self.text) + Self::H_PADDING * 2.0,
            Self::HEIGHT,
        )
    }

    /// 在指定位置绘制圆角标签。
    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, ctx: &RenderContext<'_>) {
        let (width, height) = self.measure(ctx);
        let rect = skia_safe::Rect::from_xywh(x, y, width, height);

        let mut bg = skia_safe::Paint::default();
        bg.set_anti_alias(true);
        bg.set_style(skia_safe::PaintStyle::Fill);
        bg.set_color4f(self.bg_color.to_skia(), None);
        canvas.draw_round_rect(rect, Self::RADIUS, Self::RADIUS, &bg);

        crate::widgets::text::draw_label(
            canvas,
            &self.text,
            x + width / 2.0,
            y + 16.5,
            Self::FONT_SIZE,
            self.text_color,
            TextAlign::Center,
        );
    }
}
