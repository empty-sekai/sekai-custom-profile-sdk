//! SDF draw commands: primitive kinds, device-space geometry, materials and the
//! glyph and shape command builders.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::sdf::atlas::SdfAtlasGlyphManifest;
use crate::sdf::shape::ShapeSdfMaterial;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SdfPrimitiveKind {
    Text,
    Shape,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point2 {
    pub x: f32,
    pub y: f32,
}

impl Point2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine2 {
    pub scale_x: f32,
    pub skew_x: f32,
    pub translate_x: f32,
    pub skew_y: f32,
    pub scale_y: f32,
    pub translate_y: f32,
}

impl Affine2 {
    pub const IDENTITY: Self = Self {
        scale_x: 1.0,
        skew_x: 0.0,
        translate_x: 0.0,
        skew_y: 0.0,
        scale_y: 1.0,
        translate_y: 0.0,
    };

    pub fn map_point(self, point: Point2) -> Point2 {
        Point2::new(
            self.skew_x
                .mul_add(point.y, self.scale_x.mul_add(point.x, self.translate_x)),
            self.scale_y
                .mul_add(point.y, self.skew_y.mul_add(point.x, self.translate_y)),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SdfMaterial {
    /// Premultiplied RGBA face color.
    pub face: [f32; 4],
    /// Premultiplied RGBA outline color.
    pub outline: [f32; 4],
    pub face_scale: f32,
    pub face_bias: f32,
    pub outline_scale: f32,
    pub outline_bias: f32,
    pub vertex_alpha: f32,
}

/// Axis-aligned half-open clip in final device coordinates. Geometry and
/// inverse atlas mapping remain unchanged; the planner only emits fragments
/// whose pixel centers lie inside this rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SdfDeviceClip {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SdfCommandMaterial {
    Text(SdfMaterial),
    Shape(ShapeSdfMaterial),
}

impl Default for SdfMaterial {
    fn default() -> Self {
        Self {
            face: [1.0; 4],
            outline: [0.0; 4],
            face_scale: 1.0,
            face_bias: 0.0,
            outline_scale: 1.0,
            outline_bias: 0.0,
            vertex_alpha: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SdfDrawCommand {
    pub kind: SdfPrimitiveKind,
    pub atlas_set: u16,
    pub atlas_page: u16,
    /// Linear atlas texel coordinates `[x, y, width, height]`.
    pub atlas_rect: [u32; 4],
    /// Device-space `[top-left, top-right, bottom-right, bottom-left]`.
    pub quad: [Point2; 4],
    #[serde(default)]
    pub device_clip: Option<SdfDeviceClip>,
    pub material: SdfCommandMaterial,
}

impl SdfDrawCommand {
    /// Builds the padded plane of an atlas glyph, then maps it to device space
    /// through the already-resolved layout matrix.
    pub fn from_atlas_glyph(
        kind: SdfPrimitiveKind,
        atlas_set: u16,
        glyph: &SdfAtlasGlyphManifest,
        atlas_point_size: f32,
        atlas_spread: f32,
        baseline_origin: Point2,
        font_size: f32,
        local_to_device: Affine2,
        material: SdfMaterial,
    ) -> Result<Self, SdfCommandBuildError> {
        let finite = [
            atlas_point_size,
            atlas_spread,
            baseline_origin.x,
            baseline_origin.y,
            font_size,
            local_to_device.scale_x,
            local_to_device.skew_x,
            local_to_device.translate_x,
            local_to_device.skew_y,
            local_to_device.scale_y,
            local_to_device.translate_y,
        ]
        .into_iter()
        .chain(glyph.plane_bearing)
        .chain(glyph.plane_size)
        .all(f32::is_finite);
        if !finite {
            return Err(SdfCommandBuildError::NonFinite);
        }
        if atlas_point_size <= 0.0
            || atlas_spread < 0.0
            || font_size <= 0.0
            || glyph.rect[2] == 0
            || glyph.rect[3] == 0
            || glyph.plane_size[0] <= 0.0
            || glyph.plane_size[1] <= 0.0
        {
            return Err(SdfCommandBuildError::InvalidMetrics);
        }
        let logical_scale = font_size / atlas_point_size;
        let left =
            (glyph.plane_bearing[0] - atlas_spread).mul_add(logical_scale, baseline_origin.x);
        let top =
            (-(glyph.plane_bearing[1] + atlas_spread)).mul_add(logical_scale, baseline_origin.y);
        let width = (glyph.plane_size[0] + atlas_spread * 2.0) * logical_scale;
        let height = (glyph.plane_size[1] + atlas_spread * 2.0) * logical_scale;
        let local_quad = [
            Point2::new(left, top),
            Point2::new(left + width, top),
            Point2::new(left + width, top + height),
            Point2::new(left, top + height),
        ];
        Ok(Self {
            kind,
            atlas_set,
            atlas_page: glyph.page,
            atlas_rect: glyph.rect,
            quad: local_quad.map(|point| local_to_device.map_point(point)),
            device_clip: None,
            material: SdfCommandMaterial::Text(material),
        })
    }

    pub fn from_shape_atlas(
        atlas_set: u16,
        shape: &crate::sdf::shape_atlas::ShapeSdfAtlasEntry,
        quad: [Point2; 4],
        material: ShapeSdfMaterial,
    ) -> Result<Self, SdfCommandBuildError> {
        if shape.rect[2] == 0 || shape.rect[3] == 0 {
            return Err(SdfCommandBuildError::InvalidMetrics);
        }
        if quad
            .iter()
            .flat_map(|point| [point.x, point.y])
            .chain(material.face)
            .chain(material.outline)
            .chain([
                material.face_threshold,
                material.outline_threshold,
                material.sharpness,
            ])
            .any(|value| !value.is_finite())
        {
            return Err(SdfCommandBuildError::NonFinite);
        }
        if material.sharpness <= 0.0 {
            return Err(SdfCommandBuildError::InvalidMetrics);
        }
        Ok(Self {
            kind: SdfPrimitiveKind::Shape,
            atlas_set,
            atlas_page: shape.page,
            atlas_rect: shape.rect,
            quad,
            device_clip: None,
            material: SdfCommandMaterial::Shape(material),
        })
    }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum SdfCommandBuildError {
    #[error("glyph placement contains a non-finite value")]
    NonFinite,
    #[error("glyph placement has invalid atlas or font metrics")]
    InvalidMetrics,
}
