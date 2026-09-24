//! Shape-SDF coverage and material contract.
//!
//! Shape sprites carry two independent signals: a distance field in red and
//! the sprite's alpha, which gates it. The texture is sampled with nearest
//! filtering, so each fragment evaluates one source texel: its coverage is
//! thresholded from the distance and multiplied by the gate. The thresholds
//! and alpha conversions come from
//! [`sekai_profile_renderer_core::sdf_material`], which the WebGL2 mask shader
//! shares.

use sekai_profile_renderer_core::sdf_material::{
    color32_unit, ShapeSdfThresholds, SHAPE_SDF_SHARPNESS,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShapeSdfTexel {
    pub distance: u8,
    pub gate: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShapeSdfMaterial {
    /// Premultiplied face RGBA after the element alpha is applied.
    pub face: [f32; 4],
    /// Premultiplied outline RGBA after outline alpha is applied.
    pub outline: [f32; 4],
    pub face_threshold: f32,
    pub outline_threshold: f32,
    pub sharpness: f32,
}

impl ShapeSdfMaterial {
    /// Builds the material from a shape element's authored values. The face
    /// alpha reaches the shader as a vertex colour and is quantized like one;
    /// the outline colour is a material colour and keeps its float alpha. The
    /// outline size saturates at one.
    pub fn from_profile_values(
        face_rgb: [f32; 3],
        face_alpha: f32,
        outline_rgb: [f32; 3],
        outline_alpha: f32,
        outline_size: f32,
    ) -> Self {
        let face_alpha = color32_unit(face_alpha);
        let outline_alpha = outline_alpha.clamp(0.0, 1.0);
        let thresholds = ShapeSdfThresholds::new(outline_size);
        Self {
            face: [
                premultiply_layer_channel(face_rgb[0], face_alpha),
                premultiply_layer_channel(face_rgb[1], face_alpha),
                premultiply_layer_channel(face_rgb[2], face_alpha),
                face_alpha,
            ],
            outline: [
                premultiply_layer_channel(outline_rgb[0], outline_alpha),
                premultiply_layer_channel(outline_rgb[1], outline_alpha),
                premultiply_layer_channel(outline_rgb[2], outline_alpha),
                outline_alpha,
            ],
            face_threshold: thresholds.face,
            outline_threshold: thresholds.outline,
            sharpness: SHAPE_SDF_SHARPNESS,
        }
    }
}

fn premultiply_layer_channel(channel: f32, alpha: f32) -> f32 {
    channel.clamp(0.0, 1.0) * alpha.clamp(0.0, 1.0)
}

/// Offset and scale mapping a raw distance byte to coverage for one edge:
/// `clamp((distance + offset) * scale, 0, 1)`, a ramp of `2 * sharpness`
/// distance steps centred on `threshold * 255`.
pub(crate) fn coverage_terms(threshold: f32, sharpness: f32) -> (f32, f32) {
    let sharpness = sharpness.max(f32::EPSILON);
    (sharpness - threshold * 255.0, (2.0 * sharpness).recip())
}

/// Coverage of one texel for the edge at `threshold`, gated by its alpha.
pub fn texel_coverage(texel: ShapeSdfTexel, threshold: f32, sharpness: f32) -> f32 {
    let (offset, scale) = coverage_terms(threshold, sharpness);
    ((f32::from(texel.distance) + offset) * scale).clamp(0.0, 1.0) * (f32::from(texel.gate) / 255.0)
}

/// Produces one premultiplied source equivalent to drawing the outline mask,
/// subtracting the face mask from it, then drawing the face above it.
pub fn shade_shape(texel: ShapeSdfTexel, material: ShapeSdfMaterial) -> [f32; 4] {
    let face_coverage = texel_coverage(texel, material.face_threshold, material.sharpness);
    let outline_coverage = texel_coverage(texel, material.outline_threshold, material.sharpness)
        * (1.0 - face_coverage);
    shade_shape_coverages(face_coverage, outline_coverage, material)
}

pub(crate) fn shade_shape_coverages(
    face_coverage: f32,
    outline_coverage: f32,
    material: ShapeSdfMaterial,
) -> [f32; 4] {
    let face_alpha = material.face[3] * face_coverage;
    let outline_above_weight = outline_coverage * (1.0 - face_alpha);
    std::array::from_fn(|channel| {
        material.outline[channel]
            .mul_add(outline_above_weight, material.face[channel] * face_coverage)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_alpha_gate_proves_r8_is_insufficient() {
        let opaque = texel_coverage(
            ShapeSdfTexel {
                distance: 255,
                gate: 255,
            },
            0.5,
            1.5,
        );
        let transparent = texel_coverage(
            ShapeSdfTexel {
                distance: 255,
                gate: 0,
            },
            0.5,
            1.5,
        );
        assert_eq!(opaque, 1.0);
        assert_eq!(transparent, 0.0);
    }

    #[test]
    fn face_is_composited_above_outline() {
        let texel = ShapeSdfTexel {
            distance: 255,
            gate: 255,
        };
        let material = ShapeSdfMaterial {
            face: [0.5, 0.0, 0.0, 0.5],
            outline: [0.0, 0.0, 1.0, 1.0],
            face_threshold: 0.5,
            outline_threshold: 0.5,
            sharpness: 1.5,
        };
        assert_eq!(shade_shape(texel, material), [0.5, 0.0, 0.0, 0.5]);
    }

    #[test]
    fn partial_face_alpha_attenuates_outline_below_face() {
        let texel = ShapeSdfTexel {
            distance: 128,
            gate: 255,
        };
        let material = ShapeSdfMaterial {
            face: [0.25, 0.0, 0.0, 0.5],
            outline: [0.0, 0.0, 1.0, 1.0],
            face_threshold: 0.5,
            outline_threshold: 0.5,
            sharpness: 1.5,
        };
        let coverage = texel_coverage(texel, 0.5, 1.5);
        let outline_above = coverage * (1.0 - coverage) * (1.0 - 0.5 * coverage);
        let shaded = shade_shape(texel, material);

        assert!((shaded[0] - 0.25 * coverage).abs() < 1.0e-7);
        assert!((shaded[2] - outline_above).abs() < 1.0e-7);
        assert!((shaded[3] - (0.5 * coverage + outline_above)).abs() < 1.0e-7);
    }

    #[test]
    fn profile_threshold_formula_matches_existing_shape_path() {
        let material =
            ShapeSdfMaterial::from_profile_values([1.0, 0.0, 0.0], 0.8, [0.0, 0.0, 1.0], 0.6, 0.4);
        assert!((material.face_threshold - 0.595).abs() < 1e-6);
        assert!((material.outline_threshold - 0.5).abs() < 1e-6);
        assert_eq!(material.face, [0.8, 0.0, 0.0, 0.8]);
        assert_eq!(material.outline, [0.0, 0.0, 0.6, 0.6]);
    }

    #[test]
    fn outline_size_saturates_at_one() {
        let full =
            ShapeSdfMaterial::from_profile_values([1.0, 0.0, 0.0], 1.0, [0.0, 0.0, 1.0], 1.0, 1.0);
        let over =
            ShapeSdfMaterial::from_profile_values([1.0, 0.0, 0.0], 1.0, [0.0, 0.0, 1.0], 1.0, 1.5);
        assert_eq!(over, full);
        assert!((full.face_threshold - 0.7375).abs() < 1e-6);
        assert!((full.outline_threshold - 0.2875).abs() < 1e-6);
        let negative =
            ShapeSdfMaterial::from_profile_values([1.0, 0.0, 0.0], 1.0, [0.0, 0.0, 1.0], 1.0, -0.5);
        assert_eq!(negative.face_threshold, 0.5);
        assert_eq!(negative.outline_threshold, 0.5);
    }

    #[test]
    fn face_alpha_is_a_vertex_color_and_outline_alpha_a_material_color() {
        let material =
            ShapeSdfMaterial::from_profile_values([1.0, 1.0, 1.0], 0.5, [1.0, 1.0, 1.0], 0.5, 0.3);
        assert_eq!(material.face[3], 128.0 / 255.0);
        assert_eq!(material.face[0], 128.0 / 255.0);
        assert_eq!(material.outline[3], 0.5);
        assert_eq!(material.outline[0], 0.5);
        let faint = ShapeSdfMaterial::from_profile_values(
            [1.0, 1.0, 1.0],
            0.01,
            [1.0, 1.0, 1.0],
            0.01,
            0.3,
        );
        assert_eq!(faint.face[3], 3.0 / 255.0);
        assert_eq!(faint.outline[3], 0.01);
    }

    #[test]
    fn material_matches_game_shader_float_premultiplication() {
        let material = ShapeSdfMaterial::from_profile_values(
            [68.0 / 255.0, 68.0 / 255.0, 102.0 / 255.0],
            134.0 / 255.0,
            [0.0; 3],
            0.0,
            0.0,
        );
        let alpha = 134.0 / 255.0;
        let expected = [
            68.0 / 255.0 * alpha,
            68.0 / 255.0 * alpha,
            102.0 / 255.0 * alpha,
            alpha,
        ];
        for (actual, expected) in material.face.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1.0e-7);
        }
    }

    #[test]
    fn coverage_matches_game_shader_without_intermediate_rgba8_rounding() {
        let coverage = texel_coverage(
            ShapeSdfTexel {
                distance: 127,
                gate: 1,
            },
            0.5,
            1.5,
        );
        assert!((coverage - (1.0 / 3.0 / 255.0)).abs() < 1.0e-8);
    }
}
