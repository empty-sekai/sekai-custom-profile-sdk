//! TMP material resolution: the face and underlay scale-bias the SDF tile
//! executor consumes for one glyph.
//!
//! The formulas live in [`sekai_profile_renderer_core::sdf_material`], which
//! the WebGL2 backend shares; this module only adapts them to the executor's
//! premultiplied material. The tile executor is the only consumer, so a
//! glyph's material is resolved once and shared by every command it produces.

use sekai_profile_renderer_core::sdf_material::{tmp_uv2_y, TmpGlyphMaterial};

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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SdfOutlineParams {
    pub outline_r: f32,
    pub outline_g: f32,
    pub outline_b: f32,
    pub outline_a: f32,
    pub outline_size: f32,
}

pub fn runtime_like_mesh_carrier(
    point_size: f32,
    is_bold: bool,
    vertex_alpha_u8: u8,
) -> RuntimeLikeGlyphMeshCarrier {
    RuntimeLikeGlyphMeshCarrier {
        point_size,
        uv2_y: tmp_uv2_y(point_size, is_bold),
        vertex_alpha_u8,
    }
}

/// Resolves the SDF tile material for one glyph from its mesh carrier,
/// straight (non-premultiplied) face RGBA in unit range, and underlay.
pub(crate) fn resolve_tile_material_direct(
    carrier: RuntimeLikeGlyphMeshCarrier,
    face_color: [f32; 4],
    outline: Option<&SdfOutlineParams>,
) -> crate::sdf::tile::SdfMaterial {
    let underlay_dilate = outline.map_or(0.0, |params| params.outline_size);
    let shader = TmpGlyphMaterial::new(carrier.uv2_y, underlay_dilate);
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
    use super::*;

    #[test]
    fn runtime_like_mesh_carrier_marks_bold_with_negative_uv2() {
        let normal = runtime_like_mesh_carrier(48.0, false, 255);
        let bold = runtime_like_mesh_carrier(48.0, true, 255);
        assert!(normal.uv2_y > 0.0);
        assert!(bold.uv2_y < 0.0);
    }

    #[test]
    fn zero_size_underlay_is_drawn_on_the_face_edge() {
        let carrier = runtime_like_mesh_carrier(24.0, false, 255);
        let outline = SdfOutlineParams {
            outline_r: 0.25,
            outline_g: 0.5,
            outline_b: 1.0,
            outline_a: 0.5,
            outline_size: 0.0,
        };
        let material = resolve_tile_material_direct(carrier, [1.0; 4], Some(&outline));
        assert_eq!(material.outline, [0.125, 0.25, 0.5, 0.5]);
        assert_eq!(material.outline_scale, material.face_scale);
        assert_eq!(material.outline_bias, material.face_bias);
    }
}
