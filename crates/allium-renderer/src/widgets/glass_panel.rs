//! 碎片玻璃容器组件。

use crate::context::RenderContext;

#[cfg(feature = "skia-core")]
use super::theme::glass_panel as glass_theme;
#[cfg(feature = "skia-core")]
use super::theme::{Color, Theme};
use super::Widget;

/// 碎片玻璃容器图元。
pub struct GlassPanel {
    /// 面板宽度。
    pub width: f32,
    /// 面板高度。
    pub height: f32,
    /// 裁切扰动强度。
    pub clip_variance: f32,
}

impl GlassPanel {
    /// 创建玻璃面板组件。
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            clip_variance: 0.0,
        }
    }
}

#[cfg(feature = "skia-core")]
pub fn prewarm_glass_panel(width: f32, height: f32, clip_variance: f32, theme: &Theme) {
    let _ = cached_panel_image(width, height, clip_variance, theme);
}

#[cfg(not(feature = "skia-core"))]
pub fn prewarm_glass_panel(
    _width: f32,
    _height: f32,
    _clip_variance: f32,
    _theme: &super::theme::Theme,
) {
}

#[cfg(feature = "skia-core")]
fn jagged_offset(seed: f32, variance: f32, limit: f32) -> f32 {
    let wave = (seed.sin() * 0.5 + (seed * 1.73).cos() * 0.35 + (seed * 2.41).sin() * 0.15)
        .clamp(-1.0, 1.0);
    (wave * variance * limit).clamp(-limit, limit)
}

#[cfg(feature = "skia-core")]
fn build_panel_path(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    clip_variance: f32,
) -> skia_safe::Path {
    let mut b = skia_safe::PathBuilder::new();
    if clip_variance <= 0.0 {
        let rrect = skia_safe::RRect::new_rect_xy(
            skia_safe::Rect::from_xywh(x, y, width, height),
            12.0,
            12.0,
        );
        b.add_rrect(rrect, None, None);
        return b.detach();
    }

    let variance = clip_variance.clamp(0.0, 1.0);
    let max_x = width * 0.12;
    let max_y = height * 0.12;
    let points = [
        (
            x + jagged_offset(0.7, variance, max_x),
            y + jagged_offset(1.1, variance, max_y),
        ),
        (x + width * 0.34, y + jagged_offset(2.3, variance, max_y)),
        (
            x + width + jagged_offset(3.7, variance, max_x),
            y + jagged_offset(4.2, variance, max_y),
        ),
        (
            x + width + jagged_offset(5.6, variance, max_x),
            y + height * 0.31,
        ),
        (
            x + width + jagged_offset(6.8, variance, max_x),
            y + height + jagged_offset(7.3, variance, max_y),
        ),
        (
            x + width * 0.61,
            y + height + jagged_offset(8.9, variance, max_y),
        ),
        (
            x + jagged_offset(9.7, variance, max_x),
            y + height + jagged_offset(10.4, variance, max_y),
        ),
        (x + jagged_offset(11.2, variance, max_x), y + height * 0.54),
    ];

    if let Some((first_x, first_y)) = points.first().copied() {
        b.move_to((first_x, first_y));
        for (px, py) in points.iter().copied().skip(1) {
            b.line_to((px, py));
        }
        b.close();
    }
    b.detach()
}

#[cfg(feature = "skia-core")]
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct GlassCacheKey {
    width_px: i32,
    height_px: i32,
    variance_bucket: i32,
    glass_bg: [u16; 4],
    glass_edge: [u16; 4],
}

#[cfg(feature = "skia-core")]
thread_local! {
    static GLASS_CACHE: std::cell::RefCell<lru::LruCache<GlassCacheKey, skia_safe::Image>> =
        std::cell::RefCell::new(lru::LruCache::new(std::num::NonZeroUsize::new(96).unwrap_or(std::num::NonZeroUsize::MIN)));
}

#[cfg(feature = "skia-core")]
fn color_key(color: Color) -> [u16; 4] {
    [
        (color.r.clamp(0.0, 1.0) * 4095.0).round() as u16,
        (color.g.clamp(0.0, 1.0) * 4095.0).round() as u16,
        (color.b.clamp(0.0, 1.0) * 4095.0).round() as u16,
        (color.a.clamp(0.0, 1.0) * 4095.0).round() as u16,
    ]
}

#[cfg(feature = "skia-core")]
fn glass_cache_key(width: f32, height: f32, clip_variance: f32, theme: &Theme) -> GlassCacheKey {
    GlassCacheKey {
        width_px: width.round() as i32,
        height_px: height.round() as i32,
        variance_bucket: (clip_variance.clamp(0.0, 1.0) * 100.0).round() as i32,
        glass_bg: color_key(theme.colors.glass_bg),
        glass_edge: color_key(theme.colors.glass_edge),
    }
}

#[cfg(feature = "skia-core")]
fn cached_panel_image(
    width: f32,
    height: f32,
    clip_variance: f32,
    theme: &Theme,
) -> Option<skia_safe::Image> {
    if width <= 0.0 || height <= 0.0 {
        return None;
    }

    let key = glass_cache_key(width, height, clip_variance, theme);
    GLASS_CACHE.with(|cache| {
        if let Some(image) = cache.borrow_mut().get(&key).cloned() {
            return Some(image);
        }

        let pad = cache_padding();
        let surface_width = (width + pad * 2.0).ceil() as i32;
        let surface_height = (height + pad * 2.0).ceil() as i32;
        let mut surface = skia_safe::surfaces::raster_n32_premul((surface_width, surface_height))?;
        let canvas = surface.canvas();
        canvas.clear(skia_safe::Color::from_argb(0, 0, 0, 0));
        draw_panel_uncached(canvas, pad, pad, width, height, clip_variance, theme);
        let image = surface.image_snapshot();
        cache.borrow_mut().put(key, image.clone());
        Some(image)
    })
}

#[cfg(feature = "skia-core")]
fn cache_padding() -> f32 {
    (glass_theme::BLUR_SIGMA * 2.0).ceil()
}

#[cfg(feature = "skia-core")]
fn draw_panel_uncached(
    canvas: &skia_safe::Canvas,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    clip_variance: f32,
    theme: &Theme,
) {
    let path = build_panel_path(x, y, width, height, clip_variance);
    let bounds = skia_safe::Rect::from_xywh(x, y, width, height);

    let mut shadow = skia_safe::Paint::default();
    shadow.set_anti_alias(true);
    shadow.set_style(skia_safe::PaintStyle::Fill);
    shadow.set_color4f(
        super::theme::Color::new(
            theme.colors.glass_bg.r,
            theme.colors.glass_bg.g,
            theme.colors.glass_bg.b,
            glass_theme::SHADOW_ALPHA * 0.18,
        )
        .to_skia(),
        None,
    );
    shadow.set_mask_filter(skia_safe::MaskFilter::blur(
        skia_safe::BlurStyle::Normal,
        glass_theme::BLUR_SIGMA,
        false,
    ));
    canvas.draw_path(&path, &shadow);

    canvas.save();
    canvas.clip_path(&path, skia_safe::ClipOp::Intersect, true);

    let mut fill = skia_safe::Paint::default();
    fill.set_anti_alias(true);
    fill.set_style(skia_safe::PaintStyle::Fill);
    fill.set_color4f(theme.colors.glass_bg.to_skia(), None);
    canvas.draw_rect(bounds, &fill);

    let gradient_colors = [
        skia_safe::Color4f::new(1.0, 1.0, 1.0, 0.18),
        skia_safe::Color4f::new(1.0, 1.0, 1.0, 0.02),
    ];
    if let Some(shader) = skia_safe::Shader::linear_gradient(
        (
            skia_safe::Point::new(x, y),
            skia_safe::Point::new(x + width, y + height),
        ),
        gradient_colors.as_slice(),
        None,
        skia_safe::TileMode::Clamp,
        None,
        None,
    ) {
        let mut sheen = skia_safe::Paint::default();
        sheen.set_anti_alias(true);
        sheen.set_style(skia_safe::PaintStyle::Fill);
        sheen.set_shader(shader);
        canvas.draw_rect(bounds, &sheen);
    }

    canvas.restore();

    let mut edge = skia_safe::Paint::default();
    edge.set_anti_alias(true);
    edge.set_style(skia_safe::PaintStyle::Stroke);
    edge.set_stroke_width(1.0);
    edge.set_color4f(theme.colors.glass_edge.to_skia(), None);
    canvas.draw_path(&path, &edge);
}

impl Widget for GlassPanel {
    /// 返回组件名称。
    fn name(&self) -> &'static str {
        "glass_panel"
    }

    /// 测量玻璃面板宽高。
    fn measure(&self, _ctx: &RenderContext<'_>) -> (f32, f32) {
        (self.width, self.height)
    }

    /// 在指定位置绘制玻璃面板。
    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, ctx: &RenderContext<'_>) {
        if let Some(image) =
            cached_panel_image(self.width, self.height, self.clip_variance, ctx.theme)
        {
            let pad = cache_padding();
            canvas.draw_image(image, (x - pad, y - pad), None);
            return;
        }

        draw_panel_uncached(
            canvas,
            x,
            y,
            self.width,
            self.height,
            self.clip_variance,
            ctx.theme,
        );
    }
}
