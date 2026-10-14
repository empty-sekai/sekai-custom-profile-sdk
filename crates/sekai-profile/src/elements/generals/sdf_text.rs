//! General panel text, built and captured through the SDF text pipeline.

use super::layout;
use crate::masterdata::MasterData;
use crate::text::{
    capture_text_sdf_with_placement, wrap_rich_text_to_width_with_atlases, TextRenderPlacement,
    TextSdfCaptureError, TextSdfCaptureTimings, TEXT_SCALE,
};
use crate::types::{ObjectData, Quaternion, TextElement, Vec3};

#[derive(Clone, Copy)]
pub(super) enum SdfTextAlign {
    Left,
    Center,
    Right,
}

pub(crate) struct GeneralSdfTextSpec {
    pub element: TextElement,
    /// Element-centre translation `(x, y)` in canvas coordinates.
    #[allow(dead_code)]
    pub(super) origin: [f32; 2],
    pub(super) render_placement: TextRenderPlacement,
}

/// A lowered General Text command: its string, layout widths and TMP style
/// in the units of the lowered scene.
#[derive(Clone, Copy)]
pub(crate) struct LoweredGeneralText<'a> {
    pub text: &'a str,
    /// Width of the command bounds.
    pub bounds_width: f32,
    /// Wrapping width. When set, the text is laid out and wrapped to it
    /// instead of `bounds_width`.
    pub max_width: Option<f32>,
    pub color: [f32; 4],
    /// TMP horizontal alignment: `1` left, `2` center, `4` right.
    pub alignment: u8,
    pub font_size: f32,
    pub line_spacing: f32,
    pub font_id: i32,
    pub render_placement: TextRenderPlacement,
    /// Glyph outline, overriding the element's own.
    pub outline: Option<crate::text::TextOutlineOverride>,
}

impl LoweredGeneralText<'_> {
    /// The width the text is laid out in.
    fn layout_width(&self) -> f32 {
        self.max_width.unwrap_or(self.bounds_width)
    }
}

pub(crate) fn build_general_sdf_text_from_lowered(
    lowered: &LoweredGeneralText<'_>,
) -> Result<GeneralSdfTextSpec, String> {
    let LoweredGeneralText {
        text,
        color,
        alignment,
        font_size: lowered_font_size,
        line_spacing,
        font_id,
        render_placement,
        ..
    } = *lowered;
    let width = lowered.layout_width();
    let align = match alignment {
        1 => SdfTextAlign::Left,
        2 => SdfTextAlign::Center,
        4 => SdfTextAlign::Right,
        value => return Err(format!("unsupported General text alignment={value}")),
    };
    if !width.is_finite()
        || width <= 0.0
        || !lowered_font_size.is_finite()
        || lowered_font_size <= 0.0
        || !line_spacing.is_finite()
        || color.iter().any(|value| !value.is_finite())
    {
        return Err("invalid lowered General text geometry".into());
    }
    let mut spec = build_general_sdf_text_with_font(
        text,
        &layout::ElementLayout {
            cx: 0.0,
            cy: 0.0,
            w: width,
            h: 0.0,
        },
        color,
        align,
        lowered_font_size * TEXT_SCALE,
        line_spacing,
        font_id,
    );
    spec.render_placement = render_placement;
    Ok(spec)
}

/// Converts a lowered General Text command back into the exact production
/// General `TextElement`, then captures the shared TMP glyph stream. The
/// semantic compositor supplies only the command transform and clip; it does
/// not own advances, wrapping, baselines, or font fallback.
pub(crate) fn capture_general_sdf_text_from_lowered(
    base_affine: [f32; 6],
    lowered: &LoweredGeneralText<'_>,
    md: &MasterData,
    atlases: &crate::sdf::atlas::MappedSdfAtlasSet,
    observer: &mut dyn FnMut(Result<crate::text::ResolvedTextSdfGlyph, TextSdfCaptureError>),
) -> Result<TextSdfCaptureTimings, String> {
    let layout_width = lowered.layout_width();
    let mut spec = build_general_sdf_text_from_lowered(lowered)?;
    if lowered.max_width.is_some() {
        if let Some(wrapped) =
            wrap_rich_text_to_width_with_atlases(&spec.element, md, layout_width, atlases)
        {
            spec.element.text = wrapped;
        }
    }
    // TEXT_SCALE applies after the content transform.
    let scaled_base = [
        base_affine[0] * TEXT_SCALE,
        base_affine[1] * TEXT_SCALE,
        base_affine[2] * TEXT_SCALE,
        base_affine[3] * TEXT_SCALE,
        base_affine[4],
        base_affine[5],
    ];
    let timings = capture_text_sdf_with_placement(
        scaled_base,
        &spec.element,
        md,
        Some(atlases),
        spec.render_placement,
        lowered.outline,
        observer,
    );
    Ok(timings)
}

#[cfg(test)]
pub(super) fn build_general_sdf_text(
    text: &str,
    layout: &layout::ElementLayout,
    color: [f32; 4],
    align: SdfTextAlign,
    font_size: f32,
    line_spacing: f32,
) -> GeneralSdfTextSpec {
    build_general_sdf_text_with_font(text, layout, color, align, font_size, line_spacing, 1)
}

pub(super) fn build_general_sdf_text_with_font(
    text: &str,
    layout: &layout::ElementLayout,
    color: [f32; 4],
    align: SdfTextAlign,
    font_size: f32,
    line_spacing: f32,
    font_id: i32,
) -> GeneralSdfTextSpec {
    let rgba = color.map(|channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
    let text = format!(
        "<color=#{:02X}{:02X}{:02X}{:02X}>{text}</color>",
        rgba[0], rgba[1], rgba[2], rgba[3]
    );
    let (anchor_x, text_type) = match align {
        SdfTextAlign::Left => (-layout.w / (2.0 * TEXT_SCALE), 1),
        SdfTextAlign::Center => (0.0, 2),
        SdfTextAlign::Right => (layout.w / (2.0 * TEXT_SCALE), 4),
    };
    GeneralSdfTextSpec {
        element: TextElement {
            object_data: ObjectData {
                layer: 0,
                lock: false,
                position: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                rotation: Quaternion {
                    w: 1.0,
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                scale: Vec3 {
                    x: 1.0,
                    y: 1.0,
                    z: 1.0,
                },
                visible: true,
            },
            color_id: 1,
            font_id,
            line_spacing: line_spacing / TEXT_SCALE,
            outline_color_id: 1,
            outline_size: 0.0,
            size: font_size / TEXT_SCALE,
            text,
            text_type,
        },
        origin: [layout.cx, -layout.cy],
        render_placement: TextRenderPlacement {
            anchor_x,
            baseline: Some(font_size * 0.35 / TEXT_SCALE),
        },
    }
}
