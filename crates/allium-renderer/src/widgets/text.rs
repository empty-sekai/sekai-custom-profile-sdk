//! 文本绘制工具与 text-in-box 文本盒子组件。
//!
//! 两条 API：
//! 1. 底层 baseline API：`draw_label(canvas, text, x, y_baseline, size, color, align)`
//!    `y_baseline` 直接传 Skia baseline 坐标。deck_result.rs / TextBadge / StatsBadge
//!    内部布局使用。
//! 2. `SimpleText` widget：text-in-box 模型，与前端 WidgetPreview 像素级对齐。
//!    使用显式 `width × height` 盒子 + `padding` + `h_align/v_align` + `line_height`
//!    定位文字，前后端共享 `ASCENT_RATIO = 0.80` 几何模型计算 baseline。

use crate::context::RenderContext;

#[cfg(feature = "skia-core")]
use super::theme::fonts;
use super::theme::Color;
use super::Widget;

/// CSS line-box 几何模型中 ascent 占字号的比例。
///
/// 前后端共用此常量推算 baseline 位置。不查询真实字体度量——这是有意的，
/// 避免因字体不同导致基线漂移。
pub const ASCENT_RATIO: f32 = 0.80;

/// 水平对齐方式（底层 baseline API 使用）。
#[derive(Clone, Copy, Debug)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// SimpleText 盒内水平对齐。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

/// SimpleText 盒内垂直对齐。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VAlign {
    Top,
    Middle,
    Bottom,
}

#[cfg(feature = "skia-core")]
fn rgba_to_color4f(color: Color) -> skia_safe::Color4f {
    color.to_skia()
}

/// text-in-box 文本盒子组件。
///
/// 与前端 WidgetPreview 的 <div padding flex><span lineHeight></span></div> 模型对齐：
/// 文字位置由 (width, height, padding, h_align, v_align, line_height, font_size)
/// 共同决定，与字体真实度量解耦。
pub struct SimpleText {
    pub text: String,
    pub size: f32,
    pub color: Color,
    pub width: f32,
    pub height: f32,
    pub h_align: HAlign,
    pub v_align: VAlign,
    pub padding: f32,
    pub line_height: f32,
    pub glow: bool,
}

impl SimpleText {
    /// 创建默认 SimpleText（盒子 260×72，左上对齐，padding=4，line-height=1.2）。
    pub fn new(text: impl Into<String>, size: f32, color: Color) -> Self {
        Self {
            text: text.into(),
            size,
            color,
            width: 260.0,
            height: 72.0,
            h_align: HAlign::Left,
            v_align: VAlign::Top,
            padding: 4.0,
            line_height: 1.2,
            glow: false,
        }
    }
}

impl Widget for SimpleText {
    fn name(&self) -> &'static str {
        "simple_text"
    }

    /// 返回盒子外接矩形（render_document::measure_node 使用）。
    ///
    /// 当 width 为 9999（text_align 扁平盒模式）时，measure 返回估算文字实际宽度，
    /// 以免 Horizontal 布局将 9999 误累加导致兄弟节点被推飞。
    /// draw() 仍使用 self.width（9999）作为 clip_rect，不会裁切文字。
    fn measure(&self, _ctx: &RenderContext<'_>) -> (f32, f32) {
        if self.width >= 9999.0 {
            // 扁平盒模式：布局应使用估算文字真实宽度
            (estimate_text_width(&self.text, self.size), self.height)
        } else {
            (self.width, self.height)
        }
    }

    /// 在 (x, y) 为左上角的盒子内绘制文字。
    ///
    /// 盒子裁切（clip_rect）模拟 CSS overflow: hidden；baseline 通过共享几何模型推算，
    /// 与前端 WidgetPreview 像素位置一致。
    ///
    /// 退化场景：
    /// - 空字符串：Skia draw_str 无副作用，clip+save 开销可忽略。
    /// - font_size ≤ 0 或 line_height ≤ 0：debug 构建会 panic 报错；release 下绘制为空。
    /// - inner_w/inner_h ≤ 0（padding 大于 width/2 等）：max(0) 兜底，文字仍可能溢出 inner 区
    ///   但被外层 clip_rect 裁掉。
    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, _ctx: &RenderContext<'_>) {
        debug_assert!(self.size > 0.0, "SimpleText::draw: font_size 必须 > 0");
        debug_assert!(
            self.line_height > 0.0,
            "SimpleText::draw: line_height 必须 > 0"
        );
        if self.text.is_empty() || self.size <= 0.0 || self.line_height <= 0.0 {
            return;
        }
        let p = self.padding;
        let inner_x = x + p;
        let inner_y = y + p;
        let inner_w = (self.width - 2.0 * p).max(0.0);
        let inner_h = (self.height - 2.0 * p).max(0.0);

        let fs = self.size;
        let line_box_h = fs * self.line_height;
        // CSS half-leading：行盒余量在字号上下平分。
        let text_top_in_line = (line_box_h - fs) / 2.0;
        let baseline_in_line = text_top_in_line + fs * ASCENT_RATIO;

        let baseline_y_in_box = match self.v_align {
            VAlign::Top => baseline_in_line,
            VAlign::Middle => (inner_h - line_box_h) / 2.0 + baseline_in_line,
            VAlign::Bottom => inner_h - line_box_h + baseline_in_line,
        };
        let draw_y = inner_y + baseline_y_in_box;

        let typeface = match resolve_typeface() {
            Some(tf) => tf,
            None => return,
        };
        let font = skia_safe::Font::new(typeface, Some(fs));
        let mut paint = skia_safe::Paint::default();
        paint.set_anti_alias(true);
        paint.set_color4f(rgba_to_color4f(self.color), None);
        let text_w = font.measure_str(&self.text, Some(&paint)).1.width();
        let draw_x = match self.h_align {
            HAlign::Left => inner_x,
            HAlign::Center => inner_x + (inner_w - text_w) / 2.0,
            HAlign::Right => inner_x + inner_w - text_w,
        };

        // overflow: hidden — clip 到盒子边界。
        canvas.save();
        let clip_rect = skia_safe::Rect::from_xywh(x, y, self.width, self.height);
        canvas.clip_rect(clip_rect, None, None);

        if self.glow {
            draw_neon_text_at(canvas, &self.text, draw_x, draw_y, fs, self.color);
        } else {
            canvas.draw_str(&self.text, (draw_x, draw_y), &font, &paint);
        }

        canvas.restore();
    }
}

#[cfg(feature = "skia-core")]
fn resolve_typeface() -> Option<skia_safe::Typeface> {
    static TYPEFACE: std::sync::OnceLock<Option<skia_safe::Typeface>> = std::sync::OnceLock::new();

    TYPEFACE
        .get_or_init(|| {
            let font_mgr = skia_safe::FontMgr::default();
            font_mgr
                .match_family_style(fonts::EMPHASIS, skia_safe::FontStyle::normal())
                .or_else(|| {
                    font_mgr.match_family_style(fonts::PRIMARY, skia_safe::FontStyle::normal())
                })
                // 无系统字体环境（wasm）：从注册表/同源字体字节构造 Typeface。
                .or_else(|| {
                    crate::text::resolve_custom_profile_typeface(&font_mgr, Some(fonts::EMPHASIS))
                })
                .or_else(|| {
                    crate::text::resolve_custom_profile_typeface(&font_mgr, Some(fonts::PRIMARY))
                })
                .or_else(|| {
                    font_mgr.match_family_style(fonts::FALLBACK, skia_safe::FontStyle::normal())
                })
                .or_else(|| font_mgr.legacy_make_typeface(None, skia_safe::FontStyle::normal()))
        })
        .clone()
}

/// 底层 baseline 文本绘制（deck_result.rs / TextBadge / StatsBadge 使用）。
///
/// `y` 直接传 Skia baseline 坐标。
#[cfg(feature = "skia-core")]
pub fn draw_label(
    canvas: &skia_safe::Canvas,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    color: Color,
    align: TextAlign,
) {
    let mut paint = skia_safe::Paint::default();
    paint.set_anti_alias(true);
    paint.set_color4f(rgba_to_color4f(color), None);
    draw_label_with_paint(canvas, text, x, y, size, &paint, align);
}

#[cfg(feature = "skia-core")]
fn draw_label_with_paint(
    canvas: &skia_safe::Canvas,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    paint: &skia_safe::Paint,
    align: TextAlign,
) {
    let Some(typeface) = resolve_typeface() else {
        return;
    };
    let font = skia_safe::Font::new(typeface, Some(size));
    let width = font.measure_str(text, Some(paint)).1.width();
    let adjusted_x = match align {
        TextAlign::Left => x,
        TextAlign::Center => x - width / 2.0,
        TextAlign::Right => x - width,
    };
    canvas.draw_str(text, (adjusted_x, y), &font, paint);
}

/// 霓虹灯发光文本（baseline API，居中绘制）。
#[cfg(feature = "skia-core")]
pub fn draw_neon_text(
    canvas: &skia_safe::Canvas,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    glow_color: Color,
) {
    let layers = [(20.0_f32, 0.10_f32), (8.0, 0.30), (3.0, 0.60)];
    for (sigma, alpha) in layers {
        let mut paint = skia_safe::Paint::default();
        paint.set_anti_alias(true);
        paint.set_color4f(
            rgba_to_color4f(Color::new(
                glow_color.r,
                glow_color.g,
                glow_color.b,
                glow_color.a * alpha,
            )),
            None,
        );
        paint.set_mask_filter(skia_safe::MaskFilter::blur(
            skia_safe::BlurStyle::Normal,
            sigma,
            false,
        ));
        draw_label_with_paint(canvas, text, x, y, size, &paint, TextAlign::Center);
    }

    let mut core_paint = skia_safe::Paint::default();
    core_paint.set_anti_alias(true);
    core_paint.set_color4f(rgba_to_color4f(Color::new(1.0, 1.0, 1.0, 1.0)), None);
    draw_label_with_paint(canvas, text, x, y, size, &core_paint, TextAlign::Center);
}

/// SimpleText 盒子内霓虹文本（draw_x 已经是文字起点，按左对齐绘制）。
#[cfg(feature = "skia-core")]
fn draw_neon_text_at(
    canvas: &skia_safe::Canvas,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    glow_color: Color,
) {
    let layers = [(20.0_f32, 0.10_f32), (8.0, 0.30), (3.0, 0.60)];
    for (sigma, alpha) in layers {
        let mut paint = skia_safe::Paint::default();
        paint.set_anti_alias(true);
        paint.set_color4f(
            rgba_to_color4f(Color::new(
                glow_color.r,
                glow_color.g,
                glow_color.b,
                glow_color.a * alpha,
            )),
            None,
        );
        paint.set_mask_filter(skia_safe::MaskFilter::blur(
            skia_safe::BlurStyle::Normal,
            sigma,
            false,
        ));
        draw_label_with_paint(canvas, text, x, y, size, &paint, TextAlign::Left);
    }

    let mut core_paint = skia_safe::Paint::default();
    core_paint.set_anti_alias(true);
    core_paint.set_color4f(rgba_to_color4f(Color::new(1.0, 1.0, 1.0, 1.0)), None);
    draw_label_with_paint(canvas, text, x, y, size, &core_paint, TextAlign::Left);
}

/// 测量文本宽度（不绘制）。
#[cfg(feature = "skia-core")]
pub fn measure_text_width(text: &str, size: f32) -> f32 {
    let Some(typeface) = resolve_typeface() else {
        return estimate_text_width(text, size);
    };
    let font = skia_safe::Font::new(typeface, Some(size));
    font.measure_str(text, None).1.width()
}

/// 估算文本宽度（非 Skia 环境使用 / deck 对齐换算）。
///
/// 区分 ASCII 与 CJK 字符：ASCII 字符宽约 0.56 × size，CJK 全角字符宽约 1.0 × size。
pub fn estimate_text_width(text: &str, size: f32) -> f32 {
    let mut total: f32 = 0.0;
    for ch in text.chars() {
        // ASCII / 拉丁补充 / 控制字符按窄字符
        let factor = if (ch as u32) < 0x0080 {
            0.56
        } else {
            // CJK / 假名 / 韩文 / 全角符号都按全角处理
            1.0
        };
        total += size * factor;
    }
    total
}
