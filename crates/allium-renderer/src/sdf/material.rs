//! TMP 材质参数解析：从字号 / 粗体 / 顶点透明度与 outline 配方推导 SDF tile
//! 执行器消费的 face / outline scale-bias。
//!
//! 这里只有纯计算与环境变量覆盖，不接触任何光栅后端；`sdf::rasterize` 的
//! 逐字形位图路径与 tile 执行器共用同一份参数，保证两条路径逐字节同源。

use std::sync::OnceLock;

const TMP_SHADER_CLAMP: f32 = 1.0;
const GRADIENT_SCALE: f32 = 6.0;
const FACE_DILATE: f32 = 0.0;
const OUTLINE_WIDTH: f32 = 0.0;
pub(crate) const OUTLINE_SOFTNESS: f32 = 0.0;
pub(crate) const UNDERLAY_SOFTNESS: f32 = 0.0;
const WEIGHT_NORMAL: f32 = 0.0;
const WEIGHT_BOLD: f32 = 0.75;
const SHARPNESS: f32 = 0.0;
const DEFAULT_RUNTIME_SCALE_RATIO_C: f32 = 0.6770833;
const DEFAULT_RUNTIME_SCREEN_X: f32 = 1920.0;
const DEFAULT_RUNTIME_SCREEN_Y: f32 = 1080.0;
const DEFAULT_RUNTIME_PROJ0_X: f32 = 0.5625;
const DEFAULT_RUNTIME_PROJ0_Y: f32 = 0.0;
const DEFAULT_RUNTIME_PROJ1_X: f32 = 0.0;
const DEFAULT_RUNTIME_PROJ1_Y: f32 = 1.0;
const DEFAULT_RUNTIME_GL_POSITION_W: f32 = 1.0;
const DEFAULT_RUNTIME_SCALE_X: f32 = 1.0;
const DEFAULT_RUNTIME_SCALE_Y: f32 = 1.0;

#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "skia-oracle"), allow(dead_code))]
pub(crate) struct TmpShaderParams {
    pub(crate) uv2_y: f32,
    pub(crate) pixel_scale: f32,
    pub(crate) scale_ratio_a: f32,
    pub(crate) scale_ratio_c: f32,
    pub(crate) face_bias: f32,
    pub(crate) face_scale: f32,
    pub(crate) underlay_bias: f32,
    pub(crate) underlay_scale: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct RuntimeLikeGlyphMeshCarrier {
    pub point_size: f32,
    pub uv2_y: f32,
    pub vertex_alpha_u8: u8,
}

impl RuntimeLikeGlyphMeshCarrier {
    pub fn vertex_alpha(self) -> f32 {
        self.vertex_alpha_u8 as f32 / 255.0
    }
}

/// Underlay 参数。
pub struct SdfOutlineParams {
    pub outline_r: f32,
    pub outline_g: f32,
    pub outline_b: f32,
    pub outline_a: f32,
    pub outline_size: f32,
    pub font_size: f32,
}

pub(crate) fn env_f32(name: &str, default: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(default)
}

pub(crate) fn env_f32_any(name: &str, default: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .unwrap_or(default)
}

fn runtime_scale_ratio_c() -> f32 {
    static RATIO: OnceLock<f32> = OnceLock::new();
    *RATIO.get_or_init(|| env_f32("SCAPUS_TMP_SCALE_RATIO_C", DEFAULT_RUNTIME_SCALE_RATIO_C))
}

fn runtime_uv2_y_from_point_size(point_size: f32) -> f32 {
    const K: f32 = 1.0 / 20250.0;
    let ps = point_size.abs();
    if !ps.is_finite() || ps <= 0.0 {
        return 1e-8;
    }
    (ps * K).max(1e-8)
}

pub fn runtime_like_mesh_carrier(
    point_size: f32,
    is_bold: bool,
    vertex_alpha_u8: u8,
) -> RuntimeLikeGlyphMeshCarrier {
    let uv2_y_mag = runtime_uv2_y_from_point_size(point_size);
    RuntimeLikeGlyphMeshCarrier {
        point_size,
        uv2_y: if is_bold { -uv2_y_mag } else { uv2_y_mag },
        vertex_alpha_u8,
    }
}

fn runtime_scale_x() -> f32 {
    static SCALE_X: OnceLock<f32> = OnceLock::new();
    *SCALE_X.get_or_init(|| env_f32("SCAPUS_TMP_RUNTIME_SCALE_X", DEFAULT_RUNTIME_SCALE_X))
}

fn runtime_scale_y() -> f32 {
    static SCALE_Y: OnceLock<f32> = OnceLock::new();
    *SCALE_Y.get_or_init(|| env_f32("SCAPUS_TMP_RUNTIME_SCALE_Y", DEFAULT_RUNTIME_SCALE_Y))
}

fn runtime_screen_x() -> f32 {
    static SCREEN_X: OnceLock<f32> = OnceLock::new();
    *SCREEN_X.get_or_init(|| env_f32("SCAPUS_TMP_RUNTIME_SCREEN_X", DEFAULT_RUNTIME_SCREEN_X))
}

fn runtime_screen_y() -> f32 {
    static SCREEN_Y: OnceLock<f32> = OnceLock::new();
    *SCREEN_Y.get_or_init(|| env_f32("SCAPUS_TMP_RUNTIME_SCREEN_Y", DEFAULT_RUNTIME_SCREEN_Y))
}

fn runtime_proj0_x() -> f32 {
    static VALUE: OnceLock<f32> = OnceLock::new();
    *VALUE.get_or_init(|| env_f32("SCAPUS_TMP_RUNTIME_PROJ0_X", DEFAULT_RUNTIME_PROJ0_X))
}

fn runtime_proj0_y() -> f32 {
    static VALUE: OnceLock<f32> = OnceLock::new();
    *VALUE.get_or_init(|| env_f32_any("SCAPUS_TMP_RUNTIME_PROJ0_Y", DEFAULT_RUNTIME_PROJ0_Y))
}

fn runtime_proj1_x() -> f32 {
    static VALUE: OnceLock<f32> = OnceLock::new();
    *VALUE.get_or_init(|| env_f32_any("SCAPUS_TMP_RUNTIME_PROJ1_X", DEFAULT_RUNTIME_PROJ1_X))
}

fn runtime_proj1_y() -> f32 {
    static VALUE: OnceLock<f32> = OnceLock::new();
    *VALUE.get_or_init(|| env_f32("SCAPUS_TMP_RUNTIME_PROJ1_Y", DEFAULT_RUNTIME_PROJ1_Y))
}

fn runtime_gl_position_w() -> f32 {
    static VALUE: OnceLock<f32> = OnceLock::new();
    *VALUE.get_or_init(|| {
        env_f32(
            "SCAPUS_TMP_RUNTIME_GL_POSITION_W",
            DEFAULT_RUNTIME_GL_POSITION_W,
        )
    })
}

fn compute_orthographic_pixel_scale() -> f32 {
    let proj_xy_x =
        (runtime_proj0_x() * runtime_screen_x() + runtime_proj1_x() * runtime_screen_y()).abs()
            * runtime_scale_x().abs().max(1e-6);
    let proj_xy_y =
        (runtime_proj0_y() * runtime_screen_x() + runtime_proj1_y() * runtime_screen_y()).abs()
            * runtime_scale_y().abs().max(1e-6);
    let pixel_size_x = runtime_gl_position_w() / proj_xy_x.max(1e-6);
    let pixel_size_y = runtime_gl_position_w() / proj_xy_y.max(1e-6);
    let pixel_scale = 1.0
        / (pixel_size_x * pixel_size_x + pixel_size_y * pixel_size_y)
            .sqrt()
            .max(1e-6);
    if pixel_scale.is_finite() && pixel_scale > 0.0001 {
        pixel_scale
    } else {
        0.0001
    }
}

fn compute_pixel_scale_from_terms() -> f32 {
    let pixel_scale = compute_orthographic_pixel_scale();
    if pixel_scale.is_finite() && pixel_scale > 0.0001 {
        pixel_scale
    } else {
        0.0001
    }
}

fn compute_shader_scale_from_terms(uv2_y: f32, pixel_scale: f32) -> f32 {
    let shader_scale = uv2_y.abs() * pixel_scale * GRADIENT_SCALE * (SHARPNESS + 1.0);
    if shader_scale.is_finite() && shader_scale > 0.0001 {
        shader_scale
    } else {
        0.0001
    }
}

pub(crate) fn compute_shader_params_without_canvas(
    carrier: RuntimeLikeGlyphMeshCarrier,
    underlay_dilate: f32,
    _fx_scale_x: f32,
) -> TmpShaderParams {
    let pixel_scale = compute_pixel_scale_from_terms();
    let shader_scale = compute_shader_scale_from_terms(carrier.uv2_y, pixel_scale);
    let ratio_weight_dilate = WEIGHT_NORMAL.max(WEIGHT_BOLD) * 0.25;
    let selected_weight_dilate = if carrier.uv2_y <= 0.0 {
        WEIGHT_BOLD
    } else {
        WEIGHT_NORMAL
    } * 0.25;

    let ratio_face_dilate = FACE_DILATE + ratio_weight_dilate;
    let selected_face_dilate = FACE_DILATE + selected_weight_dilate;
    let face_denom = (OUTLINE_SOFTNESS + OUTLINE_WIDTH + ratio_face_dilate).max(1.0);
    let scale_ratio_a =
        ((GRADIENT_SCALE - TMP_SHADER_CLAMP) / (GRADIENT_SCALE * face_denom)).max(0.0);
    let face_softness = OUTLINE_SOFTNESS * scale_ratio_a;
    let face_scale = shader_scale / (1.0 + face_softness * shader_scale);
    let face_base = 0.5 - selected_face_dilate * scale_ratio_a * 0.5;
    let face_bias = face_base * face_scale - 0.5;

    let scale_ratio_c = runtime_scale_ratio_c();
    let underlay_softness = UNDERLAY_SOFTNESS * scale_ratio_c;
    let underlay_scale = shader_scale / (1.0 + underlay_softness * shader_scale);
    let underlay_bias =
        face_base * underlay_scale - 0.5 - (underlay_dilate * scale_ratio_c) * underlay_scale * 0.5;

    TmpShaderParams {
        uv2_y: carrier.uv2_y,
        pixel_scale,
        scale_ratio_a,
        scale_ratio_c,
        face_bias,
        face_scale,
        underlay_bias,
        underlay_scale,
    }
}

/// Resolves the SDF tile material for one glyph from its mesh carrier, FX
/// horizontal scale, straight (non-premultiplied) face RGBA in unit range, and
/// optional outline recipe.
pub(crate) fn resolve_tile_material_direct(
    carrier: RuntimeLikeGlyphMeshCarrier,
    fx_scale_x: f32,
    face_color: [f32; 4],
    outline: Option<&SdfOutlineParams>,
) -> crate::sdf::tile::SdfMaterial {
    let outline_size = outline.map_or(0.0, |params| params.outline_size.max(0.0));
    let shader = compute_shader_params_without_canvas(carrier, outline_size, fx_scale_x);
    let face_alpha = face_color[3];
    let outline_color = outline.map_or([0.0; 4], |params| {
        let alpha = params.outline_a.clamp(0.0, 1.0);
        [
            params.outline_r * alpha,
            params.outline_g * alpha,
            params.outline_b * alpha,
            alpha,
        ]
    });
    crate::sdf::tile::SdfMaterial {
        face: [
            face_color[0] * face_alpha,
            face_color[1] * face_alpha,
            face_color[2] * face_alpha,
            face_alpha,
        ],
        outline: outline_color,
        face_scale: shader.face_scale,
        face_bias: shader.face_bias,
        outline_scale: shader.underlay_scale,
        outline_bias: shader.underlay_bias,
        vertex_alpha: carrier.vertex_alpha(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        compute_orthographic_pixel_scale, compute_shader_scale_from_terms,
        runtime_like_mesh_carrier, runtime_scale_ratio_c, runtime_uv2_y_from_point_size,
    };

    #[test]
    fn runtime_uv2_y_from_point_size_matches_runtime_samples() {
        const K: f32 = 1.0 / 20250.0;
        for ps in [8.0, 10.0, 18.0, 24.0, 48.0, 72.0, 96.0] {
            let uv2_y = runtime_uv2_y_from_point_size(ps);
            let expected = ps * K;
            assert!(
                (uv2_y - expected).abs() < 1e-8,
                "point_size={ps} uv2_y={uv2_y} expected={expected}"
            );
        }
    }

    #[test]
    fn runtime_like_mesh_carrier_marks_bold_with_negative_uv2() {
        let normal = runtime_like_mesh_carrier(48.0, false, 255);
        let bold = runtime_like_mesh_carrier(48.0, true, 255);
        assert!(normal.uv2_y > 0.0);
        assert!(bold.uv2_y < 0.0);
    }

    #[test]
    fn runtime_scale_ratio_c_matches_live_default() {
        assert!((runtime_scale_ratio_c() - 0.6770833).abs() < 1e-6);
    }

    #[test]
    fn compute_shader_scale_from_terms_reaches_runtime_target_domain() {
        let shader_scale = compute_shader_scale_from_terms(
            runtime_uv2_y_from_point_size(96.0),
            compute_orthographic_pixel_scale(),
        );
        assert!((shader_scale - 21.7223203).abs() < 1e-4);
    }
}
