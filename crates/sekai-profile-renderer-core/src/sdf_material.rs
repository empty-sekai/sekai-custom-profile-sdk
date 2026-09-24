//! Material parameters of the two SDF primitives, shared by the native tile
//! executor and the WebGL2 shaders.
//!
//! Text follows the TextMesh Pro distance-field material with `UNDERLAY_ON`,
//! which the game enables on every card font: an element's outline is the
//! underlay pass, dilated by the outline size and tinted with the outline
//! colour. It is drawn even when the outline size is zero, where it shares the
//! face edge.
//!
//! Shapes follow the distance-field image material: a face edge and an outer
//! outline edge over the sprite's red distance channel, both gated by the
//! sprite's alpha.

/// `_GradientScale` of the card font atlases.
pub const TMP_GRADIENT_SCALE: f32 = 6.0;
/// `ShaderUtilities.m_clamp`.
const TMP_SHADER_CLAMP: f32 = 1.0;
const TMP_FACE_DILATE: f32 = 0.0;
const TMP_OUTLINE_WIDTH: f32 = 0.0;
const TMP_OUTLINE_SOFTNESS: f32 = 0.0;
const TMP_UNDERLAY_SOFTNESS: f32 = 0.0;
const TMP_WEIGHT_NORMAL: f32 = 0.0;
const TMP_WEIGHT_BOLD: f32 = 0.75;
const TMP_SHARPNESS: f32 = 0.0;
/// Per-point `uv2.y` TextMesh Pro writes for a card glyph.
const TMP_UV2_PER_POINT: f32 = 1.0 / 20250.0;
/// Smallest `uv2.y` magnitude; keeps the bold sign meaningful at size zero.
const TMP_MIN_UV2: f32 = 1e-8;
/// Floor of the shader scale, as in the TextMesh Pro vertex program.
const TMP_MIN_SHADER_SCALE: f32 = 0.0001;

/// Screen-space pixel scale of the reference 1920x1080 canvas: the vertex
/// program's `_ScreenParams`-derived term, `1 / length(pixelSize)` with a
/// 1/1080 pixel on both axes.
pub const TMP_PIXEL_SCALE: f32 = 763.675_35;

/// `max(WeightNormal, WeightBold) / 4`, the weight term of the scale ratios.
const TMP_WEIGHT_DILATE: f32 = (if TMP_WEIGHT_BOLD > TMP_WEIGHT_NORMAL {
    TMP_WEIGHT_BOLD
} else {
    TMP_WEIGHT_NORMAL
}) / 4.0;

/// `_ScaleRatioA` from `ShaderUtilities.UpdateShaderRatios`.
const TMP_SCALE_RATIO_A: f32 = {
    let range = TMP_OUTLINE_SOFTNESS + TMP_OUTLINE_WIDTH + (TMP_FACE_DILATE + TMP_WEIGHT_DILATE);
    let t = if range > 1.0 { range } else { 1.0 };
    let ratio = (TMP_GRADIENT_SCALE - TMP_SHADER_CLAMP) / (TMP_GRADIENT_SCALE * t);
    if ratio > 0.0 {
        ratio
    } else {
        0.0
    }
};

/// `_ScaleRatioC` from `ShaderUtilities.UpdateShaderRatios`, evaluated with no
/// underlay offset or softness and a dilate within the editor's 0..1 range,
/// where the range divisor is 1.
pub const TMP_SCALE_RATIO_C: f32 = {
    let range = (TMP_WEIGHT_DILATE + TMP_FACE_DILATE) * (TMP_GRADIENT_SCALE - TMP_SHADER_CLAMP);
    let numerator = TMP_GRADIENT_SCALE - TMP_SHADER_CLAMP - range;
    (if numerator > 0.0 { numerator } else { 0.0 }) / TMP_GRADIENT_SCALE
};

/// `uv2.y` of a glyph: proportional to the point size, negated for bold.
pub fn tmp_uv2_y(point_size: f32, bold: bool) -> f32 {
    let size = point_size.abs();
    let magnitude = if !size.is_finite() || size <= 0.0 {
        TMP_MIN_UV2
    } else {
        (size * TMP_UV2_PER_POINT).max(TMP_MIN_UV2)
    };
    if bold {
        -magnitude
    } else {
        magnitude
    }
}

/// Face and underlay scale/bias of one glyph, in the form the fragment stage
/// evaluates: `clamp(sdf * scale - bias, 0, 1)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TmpGlyphMaterial {
    pub face_scale: f32,
    pub face_bias: f32,
    pub underlay_scale: f32,
    pub underlay_bias: f32,
}

impl TmpGlyphMaterial {
    /// Resolves the material for a glyph with the given `uv2.y` (see
    /// [`tmp_uv2_y`]) and underlay dilate, the element's outline size.
    pub fn new(uv2_y: f32, underlay_dilate: f32) -> Self {
        let shader_scale = shader_scale(uv2_y);
        let selected_weight = if uv2_y <= 0.0 {
            TMP_WEIGHT_BOLD
        } else {
            TMP_WEIGHT_NORMAL
        } * 0.25;
        let face_dilate = TMP_FACE_DILATE + selected_weight;
        let face_softness = TMP_OUTLINE_SOFTNESS * TMP_SCALE_RATIO_A;
        let face_scale = shader_scale / (1.0 + face_softness * shader_scale);
        let face_base = 0.5 - face_dilate * TMP_SCALE_RATIO_A * 0.5;
        let face_bias = face_base * face_scale - 0.5;

        let underlay_softness = TMP_UNDERLAY_SOFTNESS * TMP_SCALE_RATIO_C;
        let underlay_scale = shader_scale / (1.0 + underlay_softness * shader_scale);
        let underlay_bias = face_base * underlay_scale
            - 0.5
            - (underlay_dilate.max(0.0) * TMP_SCALE_RATIO_C) * underlay_scale * 0.5;
        Self {
            face_scale,
            face_bias,
            underlay_scale,
            underlay_bias,
        }
    }
}

fn shader_scale(uv2_y: f32) -> f32 {
    let scale = uv2_y.abs() * TMP_PIXEL_SCALE * TMP_GRADIENT_SCALE * (TMP_SHARPNESS + 1.0);
    if scale.is_finite() && scale > TMP_MIN_SHADER_SCALE {
        scale
    } else {
        TMP_MIN_SHADER_SCALE
    }
}

/// Face edge of a shape without outline, on the normalized distance channel.
pub const SHAPE_FACE_THRESHOLD: f32 = 0.5;
/// Face edge shift per unit of outline size.
pub const SHAPE_FACE_THRESHOLD_PER_OUTLINE: f32 = 0.2375;
/// `_OuterFillRatio` per unit of outline size.
pub const SHAPE_OUTER_FILL_RATIO: f32 = 0.95;
/// Outer outline edge shift per unit of `_OuterFillRatio`.
pub const SHAPE_OUTLINE_THRESHOLD_PER_FILL: f32 = 0.75;
/// Half width of the coverage ramp, in 8-bit distance steps.
pub const SHAPE_SDF_SHARPNESS: f32 = 1.5;

/// The outline size the shape material receives: negative values disable
/// the outline and values above one saturate.
pub fn shape_outline_size(value: f32) -> f32 {
    if value < 0.0 {
        0.0
    } else {
        value.min(1.0)
    }
}

/// Face and outer outline edges of a shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeSdfThresholds {
    pub face: f32,
    pub outline: f32,
}

impl ShapeSdfThresholds {
    /// Edges for an element's outline size, clamped by [`shape_outline_size`].
    pub fn new(outline_size: f32) -> Self {
        let outline_size = shape_outline_size(outline_size);
        let outer_fill_ratio = outline_size * SHAPE_OUTER_FILL_RATIO;
        Self {
            face: SHAPE_FACE_THRESHOLD + outline_size * SHAPE_FACE_THRESHOLD_PER_OUTLINE,
            outline: (1.0 - outer_fill_ratio * SHAPE_OUTLINE_THRESHOLD_PER_FILL)
                .min(SHAPE_FACE_THRESHOLD),
        }
    }
}

/// One channel after Unity's `Color` to `Color32` conversion, back in unit
/// range: clamped, scaled to 255 and rounded half to even. Vertex colours,
/// and so a shape's face alpha, go through it; material colours do not.
pub fn color32_unit(value: f32) -> f32 {
    let clamped = if value < 0.0 { 0.0 } else { value.min(1.0) };
    (clamped * 255.0).round_ties_even() / 255.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uv2_is_linear_in_point_size_and_signed_by_weight() {
        for size in [8.0, 10.0, 18.0, 24.0, 48.0, 72.0, 96.0] {
            let expected = size * TMP_UV2_PER_POINT;
            assert!((tmp_uv2_y(size, false) - expected).abs() < 1e-8);
            assert_eq!(tmp_uv2_y(size, true), -tmp_uv2_y(size, false));
        }
        assert_eq!(tmp_uv2_y(0.0, false), TMP_MIN_UV2);
        assert_eq!(tmp_uv2_y(f32::NAN, true), -TMP_MIN_UV2);
    }

    #[test]
    fn pixel_scale_is_the_reference_canvas_term() {
        let pixel = 1.0f32 / 1080.0;
        let derived = 1.0 / (pixel * pixel + pixel * pixel).sqrt();
        assert_eq!(TMP_PIXEL_SCALE.to_bits(), derived.to_bits());
    }

    #[test]
    fn scale_ratios_follow_the_material_weights() {
        assert!((TMP_SCALE_RATIO_A - 5.0 / 6.0).abs() < 1e-7);
        assert_eq!(TMP_SCALE_RATIO_C, 0.677_083_3);
    }

    #[test]
    fn shader_scale_matches_the_reference_domain() {
        assert!((shader_scale(tmp_uv2_y(96.0, false)) - 21.722_32).abs() < 1e-4);
    }

    #[test]
    fn zero_dilate_underlay_shares_the_face_edge() {
        for bold in [false, true] {
            let material = TmpGlyphMaterial::new(tmp_uv2_y(24.0, bold), 0.0);
            assert_eq!(material.underlay_scale, material.face_scale);
            assert_eq!(material.underlay_bias, material.face_bias);
        }
    }

    #[test]
    fn underlay_dilate_lowers_the_underlay_edge() {
        let uv2_y = tmp_uv2_y(24.0, true);
        let plain = TmpGlyphMaterial::new(uv2_y, 0.0);
        let dilated = TmpGlyphMaterial::new(uv2_y, 1.0);
        let expected = plain.underlay_bias - TMP_SCALE_RATIO_C * dilated.underlay_scale * 0.5;
        assert!((dilated.underlay_bias - expected).abs() < 1e-6);
        assert_eq!(TmpGlyphMaterial::new(uv2_y, -1.0), plain);
    }

    #[test]
    fn shape_outline_size_saturates_like_the_image_component() {
        assert_eq!(shape_outline_size(-0.5), 0.0);
        assert_eq!(shape_outline_size(0.4), 0.4);
        assert_eq!(shape_outline_size(1.5), 1.0);
        assert_eq!(ShapeSdfThresholds::new(1.5), ShapeSdfThresholds::new(1.0));
        let full = ShapeSdfThresholds::new(1.0);
        assert!((full.face - 0.7375).abs() < 1e-6);
        assert!((full.outline - 0.2875).abs() < 1e-6);
    }

    #[test]
    fn thin_outline_edges_start_at_the_face_edge() {
        let none = ShapeSdfThresholds::new(0.0);
        assert_eq!(none.face, SHAPE_FACE_THRESHOLD);
        assert_eq!(none.outline, SHAPE_FACE_THRESHOLD);
        let thin = ShapeSdfThresholds::new(0.4);
        assert!((thin.face - 0.595).abs() < 1e-6);
        assert_eq!(thin.outline, SHAPE_FACE_THRESHOLD);
    }

    #[test]
    fn color32_rounds_half_to_even() {
        assert_eq!(color32_unit(0.5), 128.0 / 255.0);
        assert_eq!(color32_unit(0.01), 3.0 / 255.0);
        let mut even_ties = 0;
        for step in (0..255u8).step_by(2) {
            let value = (f32::from(step) + 0.5) / 255.0;
            if value * 255.0 == f32::from(step) + 0.5 {
                assert_eq!(color32_unit(value), f32::from(step) / 255.0);
                even_ties += 1;
            }
        }
        assert!(even_ties > 0);
        assert_eq!(color32_unit(-1.0), 0.0);
        assert_eq!(color32_unit(2.0), 1.0);
    }
}
