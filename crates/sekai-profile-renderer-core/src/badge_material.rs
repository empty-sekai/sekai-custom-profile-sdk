//! Lit material of can-badge collection images, shared by the native
//! compositor and the WebGL2 shader.
//!
//! A can badge is an ordinary collection image drawn through a lit material:
//! the static normal map [`NORMAL_MAP_KEY`], sampled at the image's UV, bends a
//! flat surface facing the viewer, and three fixed directional lights add a
//! specular highlight on top of the image colour. The card renders in gamma
//! space, so every value is an 8-bit texel value divided by 255; nothing is
//! linearised.
//!
//! The lighting frame has +X right, +Y up and +Z away from the viewer. The
//! image faces the viewer with normal `(0, 0, -1)`; its tangent is the image's
//! +u axis after the element's rotation and flip ([`tangent`]), and the
//! bitangent is `cross(normal, tangent) * -1`, which is +Y for an upright
//! image.

use crate::{Matrix2d, ResourceKey};

/// Namespace of the normal map: it ships with the renderer's static assets
/// rather than with a game version.
pub const NORMAL_MAP_NAMESPACE: &str = "static";
/// Static asset key of the normal map.
pub const NORMAL_MAP_KEY: &str = "ui/sekai_badge_normal";
/// Pixel width and height of the normal map, used as its metric before it is
/// loaded.
pub const NORMAL_MAP_SIZE: f32 = 256.0;

/// Surface smoothness.
pub const SMOOTHNESS: f32 = 0.6;
/// Metallic share of the surface; a badge is not metallic.
pub const METALLIC: f32 = 0.0;
/// Diffuse share of a non-metallic surface, one minus its 4% reflectance.
pub const DIELECTRIC_DIFFUSE: f32 = 0.96;
/// Lower bound of the roughness.
pub const MIN_ROUGHNESS: f32 = 0.007_812_5;
/// Roughness factor of the specular normalization term.
pub const NORMALIZATION_SCALE: f32 = 4.0;
/// Constant of the specular normalization term.
pub const NORMALIZATION_BIAS: f32 = 30.0;
/// Constant of the specular distribution denominator.
pub const DISTRIBUTION_BIAS: f32 = 1.000_01;
/// Lower bound of the squared cosine between light and half vector.
pub const MIN_LIGHT_HALF_SQUARED: f32 = 0.1;
/// Offset subtracted from each light's specular term before clamping.
pub const SPECULAR_FLOOR: f32 = 6.103_515_6e-5;
/// Upper bound of each light's specular term.
pub const SPECULAR_MAX: f32 = 1000.0;
/// Lower bound of the unpacked normal's z component.
pub const MIN_NORMAL_Z: f32 = 1e-16;
/// Image alpha at and above which a fragment is opaque; below it the fragment
/// is transparent.
pub const ALPHA_CUTOFF: f32 = 0.5;
/// View direction the half vectors are taken with.
pub const VIEW_DIRECTION: [f32; 3] = [0.0, 0.0, -1.0];
/// Unit direction towards each light, and its intensity in the fourth
/// component.
pub const LIGHTS: [[f32; 4]; 3] = [
    [-0.25, 0.258_819_04, -0.933_012_7, 0.5],
    [0.433_012_7, 0.5, -0.75, 0.5],
    [0.5, -std::f32::consts::FRAC_1_SQRT_2, -0.5, 0.5],
];

/// The normal map as a resource key.
pub fn normal_map_resource() -> ResourceKey {
    ResourceKey {
        namespace: NORMAL_MAP_NAMESPACE.into(),
        key: NORMAL_MAP_KEY.into(),
    }
}

/// Unit tangent of an image drawn with `matrix`, a canvas-space matrix (+x
/// right, +y down) that is invertible: the image's +u axis in the lighting
/// frame, where +Y is up.
pub fn tangent(matrix: Matrix2d) -> [f32; 2] {
    let (x, y) = (matrix[0], -matrix[1]);
    let length = (x * x + y * y).sqrt();
    [x / length, y / length]
}

/// Premultiplied colour of one fragment, each channel in `0..=1`.
///
/// `albedo` is the image texel and `normal_texel` the normal-map texel at the
/// same UV, both straight (not premultiplied) RGBA in `0..=1`. `tangent` is
/// the unit tangent from [`tangent`], and `vertex_color` the element colour
/// (straight RGBA, its alpha the element alpha). The lit colour is multiplied
/// by the vertex colour and the coverage, and only that final value is
/// clamped, as an 8-bit target stores it; a highlight may therefore exceed
/// the fragment alpha and add light to what lies below.
pub fn shade(
    albedo: [f32; 4],
    normal_texel: [f32; 4],
    tangent: [f32; 2],
    vertex_color: [f32; 4],
) -> [f32; 4] {
    let tangent = [tangent[0], tangent[1], 0.0];
    let normal = [0.0, 0.0, -1.0];
    let bitangent = scale(cross(normal, tangent), -1.0);
    let nx = normal_texel[0] * normal_texel[3] * 2.0 - 1.0;
    let ny = normal_texel[1] * 2.0 - 1.0;
    let nz = (1.0 - (nx * nx + ny * ny).min(1.0))
        .sqrt()
        .max(MIN_NORMAL_Z);
    // The perturbed normal is used as is, without renormalising.
    let surface = add(
        add(scale(tangent, nx), scale(bitangent, ny)),
        scale(normal, nz),
    );
    let roughness = ((1.0 - SMOOTHNESS) * (1.0 - SMOOTHNESS)).max(MIN_ROUGHNESS);
    let roughness2 = roughness * roughness;
    let normalization = roughness * NORMALIZATION_SCALE + NORMALIZATION_BIAS;
    let mut specular = 0.0;
    for light in LIGHTS {
        let direction = [light[0], light[1], light[2]];
        let half = normalize(add(direction, VIEW_DIRECTION));
        let light_half = dot(direction, half).clamp(0.0, 1.0);
        let light_half2 = (light_half * light_half).max(MIN_LIGHT_HALF_SQUARED);
        let normal_half = dot(surface, half).clamp(0.0, 1.0);
        let d = normal_half * normal_half * (roughness2 - 1.0) + DISTRIBUTION_BIAS;
        let term = roughness2 / (d * d * light_half2 * normalization);
        specular += (term - SPECULAR_FLOOR).clamp(0.0, SPECULAR_MAX) * light[3];
    }
    let diffuse = DIELECTRIC_DIFFUSE * (1.0 - METALLIC);
    let coverage = if albedo[3] >= ALPHA_CUTOFF { 1.0 } else { 0.0 };
    let alpha = coverage * vertex_color[3].clamp(0.0, 1.0);
    let channel = |channel: usize| {
        ((albedo[channel] * diffuse + specular) * vertex_color[channel] * alpha).clamp(0.0, 1.0)
    };
    [channel(0), channel(1), channel(2), alpha]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f32; 3], factor: f32) -> [f32; 3] {
    [a[0] * factor, a[1] * factor, a[2] * factor]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    scale(a, 1.0 / dot(a, a).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(texel: [f32; 4]) -> [f32; 4] {
        texel.map(|value| value / 255.0)
    }

    /// Premultiplied output in 8-bit units.
    fn shaded(albedo: [f32; 4], normal: [f32; 4], tangent: [f32; 2], alpha: f32) -> [f32; 4] {
        shade(unit(albedo), unit(normal), tangent, [1.0, 1.0, 1.0, alpha])
            .map(|value| value * 255.0)
    }

    fn assert_close(actual: [f32; 4], expected: [f32; 4]) {
        for (actual, expected) in actual.iter().zip(expected) {
            assert!((actual - expected).abs() < 0.01, "{actual} != {expected}");
        }
    }

    const UPRIGHT: [f32; 2] = [1.0, 0.0];
    const TILTED: [f32; 4] = [200.0, 90.0, 220.0, 255.0];
    const RED: [f32; 4] = [200.0, 40.0, 90.0, 255.0];

    // Expected values follow a double-precision evaluation of the same
    // material, in 8-bit units.

    #[test]
    fn a_flat_normal_adds_the_same_highlight_to_every_channel() {
        assert_close(
            shaded(
                [50.0, 75.0, 113.0, 255.0],
                [128.0, 128.0, 255.0, 255.0],
                UPRIGHT,
                1.0,
            ),
            [88.2999, 112.2999, 148.7799, 255.0],
        );
    }

    #[test]
    fn a_normal_facing_a_light_saturates_the_highlight() {
        assert_close(
            shaded(
                [50.0, 75.0, 113.0, 255.0],
                [114.217, 140.783, 254.0, 255.0],
                UPRIGHT,
                1.0,
            ),
            [206.0653, 230.0653, 255.0, 255.0],
        );
    }

    #[test]
    fn the_tangent_frame_turns_with_the_element() {
        assert_close(
            shaded(RED, TILTED, UPRIGHT, 1.0),
            [202.5983, 48.9983, 96.9983, 255.0],
        );
        let radians = 30f32.to_radians();
        assert_close(
            shaded(RED, TILTED, [radians.cos(), radians.sin()], 1.0),
            [195.6354, 42.0354, 90.0354, 255.0],
        );
        assert_close(
            shaded(RED, TILTED, [-1.0, 0.0], 1.0),
            [194.0715, 40.4715, 88.4715, 255.0],
        );
    }

    #[test]
    fn the_normal_x_is_the_red_channel_times_alpha() {
        assert_close(
            shaded(
                [10.0, 20.0, 30.0, 255.0],
                [60.0, 200.0, 180.0, 128.0],
                UPRIGHT,
                1.0,
            ),
            [10.0383, 19.6383, 29.2383, 255.0],
        );
    }

    #[test]
    fn image_alpha_is_cut_at_one_half_and_scaled_by_the_element_alpha() {
        let flat = [128.0, 128.0, 255.0, 255.0];
        assert_eq!(
            shaded([10.0, 20.0, 30.0, 127.0], flat, UPRIGHT, 1.0),
            [0.0; 4]
        );
        assert_close(
            shaded([10.0, 20.0, 30.0, 128.0], flat, UPRIGHT, 1.0),
            [49.8999, 59.4999, 69.0999, 255.0],
        );
        assert_close(
            shaded(RED, TILTED, UPRIGHT, 0.5),
            [101.2992, 24.4992, 48.4992, 127.5],
        );
    }

    #[test]
    fn the_vertex_colour_tints_the_lit_colour_and_only_the_result_is_clamped() {
        // The blue channel is lit above one; at half alpha it stays above the
        // fragment alpha instead of being clamped before the alpha applies.
        let lit = shade(
            unit([50.0, 75.0, 113.0, 255.0]),
            unit([114.217, 140.783, 254.0, 255.0]),
            UPRIGHT,
            [1.0, 0.5, 1.0, 0.5],
        )
        .map(|value| value * 255.0);
        assert_close(lit, [103.0327, 57.5163, 133.2727, 127.5]);
    }

    #[test]
    fn the_tangent_is_the_image_u_axis_with_y_up() {
        assert_eq!(tangent([1.0, 0.0, 0.0, 1.0, 400.0, 200.0]), [1.0, 0.0]);
        assert_eq!(tangent([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]), [1.0, 0.0]);
        // A counter-clockwise turn on screen is a negative angle in the
        // y-down canvas.
        let radians = 30f32.to_radians();
        let [x, y] = tangent([
            radians.cos(),
            -radians.sin(),
            radians.sin(),
            radians.cos(),
            0.0,
            0.0,
        ]);
        assert!((x - radians.cos()).abs() < 1e-6 && (y - radians.sin()).abs() < 1e-6);
        assert_eq!(tangent([-1.0, 0.0, 0.0, 1.0, 0.0, 0.0]), [-1.0, 0.0]);
        assert_eq!(tangent([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]), [1.0, 0.0]);
    }

    #[test]
    fn lights_are_unit_directions() {
        for light in LIGHTS {
            let length = (light[0] * light[0] + light[1] * light[1] + light[2] * light[2]).sqrt();
            assert!((length - 1.0).abs() < 1e-6, "{light:?}");
        }
    }
}
