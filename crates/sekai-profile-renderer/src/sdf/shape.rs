//! Shape-SDF coverage and material contract.
//!
//! Production shape PNGs contain two independent signals: distance in red and
//! an alpha gate. Thresholding is performed per source texel before sampling,
//! so an exact scalar oracle must retain both channels. The current legacy
//! four-argument `Canvas::draw_image_rect` uses Skia's default nearest
//! sampling; linear filtering remains a separately measured candidate.

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
    pub fn from_profile_values(
        face_rgb: [f32; 3],
        face_alpha: f32,
        outline_rgb: [f32; 3],
        outline_alpha: f32,
        outline_size: f32,
    ) -> Self {
        let face_alpha = face_alpha.clamp(0.0, 1.0);
        let outline_alpha = outline_alpha.clamp(0.0, 1.0);
        let outline_size = outline_size.max(0.0);
        let outer_fill_ratio = outline_size * 0.95;
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
            face_threshold: 0.5 + outline_size * 0.2375,
            outline_threshold: (1.0 - outer_fill_ratio * 0.75).min(0.5),
            sharpness: 1.5,
        }
    }
}

fn premultiply_layer_channel(channel: f32, alpha: f32) -> f32 {
    channel.clamp(0.0, 1.0) * alpha.clamp(0.0, 1.0)
}

/// Matches `draw_shape::sdf_mask_alpha` before the generated mask is scaled.
pub fn texel_coverage(texel: ShapeSdfTexel, threshold: f32, sharpness: f32) -> f32 {
    let threshold = threshold * 255.0;
    let sharpness = sharpness.max(f32::EPSILON);
    let coverage =
        ((f32::from(texel.distance) - threshold + sharpness) / (2.0 * sharpness)).clamp(0.0, 1.0);
    coverage * (f32::from(texel.gate) / 255.0)
}

/// Candidate bilinear filtering after per-texel threshold/gate evaluation.
/// Texel order is `[p00, p10, p01, p11]`.
pub fn bilinear_coverage(
    texels: [ShapeSdfTexel; 4],
    fx: f32,
    fy: f32,
    threshold: f32,
    sharpness: f32,
) -> f32 {
    let [p00, p10, p01, p11] = texels.map(|texel| texel_coverage(texel, threshold, sharpness));
    let top = (p10 - p00).mul_add(fx, p00);
    let bottom = (p11 - p01).mul_add(fx, p01);
    (bottom - top).mul_add(fy, top)
}

/// Produces one premultiplied source equivalent to drawing the outline mask,
/// subtracting the face mask from it, then drawing the face above it.
pub fn shade_shape(
    texels: [ShapeSdfTexel; 4],
    fx: f32,
    fy: f32,
    material: ShapeSdfMaterial,
) -> [f32; 4] {
    let face_coverage =
        bilinear_coverage(texels, fx, fy, material.face_threshold, material.sharpness);
    let outline_coverage = bilinear_coverage(
        texels,
        fx,
        fy,
        material.outline_threshold,
        material.sharpness,
    ) * (1.0 - face_coverage);
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
    fn threshold_is_applied_before_linear_candidate_filtering() {
        let texels = [
            ShapeSdfTexel {
                distance: 0,
                gate: 255,
            },
            ShapeSdfTexel {
                distance: 255,
                gate: 255,
            },
            ShapeSdfTexel {
                distance: 255,
                gate: 0,
            },
            ShapeSdfTexel {
                distance: 255,
                gate: 255,
            },
        ];
        assert!((bilinear_coverage(texels, 0.5, 0.5, 0.5, 1.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn face_is_composited_above_outline() {
        let texels = [ShapeSdfTexel {
            distance: 255,
            gate: 255,
        }; 4];
        let material = ShapeSdfMaterial {
            face: [0.5, 0.0, 0.0, 0.5],
            outline: [0.0, 0.0, 1.0, 1.0],
            face_threshold: 0.5,
            outline_threshold: 0.5,
            sharpness: 1.5,
        };
        assert_eq!(
            shade_shape(texels, 0.0, 0.0, material),
            [0.5, 0.0, 0.0, 0.5]
        );
    }

    #[test]
    fn partial_face_alpha_attenuates_outline_below_face() {
        let texels = [ShapeSdfTexel {
            distance: 128,
            gate: 255,
        }; 4];
        let material = ShapeSdfMaterial {
            face: [0.25, 0.0, 0.0, 0.5],
            outline: [0.0, 0.0, 1.0, 1.0],
            face_threshold: 0.5,
            outline_threshold: 0.5,
            sharpness: 1.5,
        };
        let coverage = texel_coverage(texels[0], 0.5, 1.5);
        let outline_above = coverage * (1.0 - coverage) * (1.0 - 0.5 * coverage);
        let shaded = shade_shape(texels, 0.0, 0.0, material);

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
