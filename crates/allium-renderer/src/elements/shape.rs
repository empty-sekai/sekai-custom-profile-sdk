use crate::assets::AssetStore;
use crate::masterdata::{MasterData, ResolvedColor};
use crate::sdf::shape::ShapeSdfMaterial;
use crate::sdf::tile::{Affine2, Point2, SdfCommandBuildError, SdfDrawCommand};
use crate::types::ShapeElement;
#[cfg(feature = "skia-oracle")]
use skia_safe::Canvas;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedShapeSdfCommand {
    pub shape_id: i32,
    pub asset_key: String,
    pub source_size: [u32; 2],
    pub source_rg8_sha256: String,
    pub quad: [Point2; 4],
    pub material: ShapeSdfMaterial,
}

impl ResolvedShapeSdfCommand {
    pub(crate) fn to_sdf_command(
        &self,
        atlas: &crate::sdf::shape_atlas::MappedShapeSdfAtlas,
        atlas_set: u16,
    ) -> Result<SdfDrawCommand, ShapeSdfCommandError> {
        let entry = atlas
            .shape(self.shape_id)
            .ok_or(ShapeSdfCommandError::MissingShape {
                shape_id: self.shape_id,
            })?;
        self.to_sdf_command_from_entry(entry, atlas_set)
    }

    fn to_sdf_command_from_entry(
        &self,
        entry: &crate::sdf::shape_atlas::ShapeSdfAtlasEntry,
        atlas_set: u16,
    ) -> Result<SdfDrawCommand, ShapeSdfCommandError> {
        if entry.asset_key != self.asset_key {
            return Err(ShapeSdfCommandError::AssetKeyMismatch {
                shape_id: self.shape_id,
                captured: self.asset_key.clone(),
                atlas: entry.asset_key.clone(),
            });
        }
        if entry.source_size != self.source_size {
            return Err(ShapeSdfCommandError::SourceSizeMismatch {
                shape_id: self.shape_id,
                captured: self.source_size,
                atlas: entry.source_size,
            });
        }
        if entry.source_rg8_sha256 != self.source_rg8_sha256 {
            return Err(ShapeSdfCommandError::SourceContentMismatch {
                shape_id: self.shape_id,
                captured: self.source_rg8_sha256.clone(),
                atlas: entry.source_rg8_sha256.clone(),
            });
        }
        SdfDrawCommand::from_shape_atlas(atlas_set, entry, self.quad, self.material)
            .map_err(ShapeSdfCommandError::Placement)
    }
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub(crate) enum ShapeSdfCommandError {
    #[error("shape atlas does not contain shape id {shape_id}")]
    MissingShape { shape_id: i32 },
    #[error("shape {shape_id} asset key mismatch: captured {captured}, atlas {atlas}")]
    AssetKeyMismatch {
        shape_id: i32,
        captured: String,
        atlas: String,
    },
    #[error("shape {shape_id} source size mismatch: captured {captured:?}, atlas {atlas:?}")]
    SourceSizeMismatch {
        shape_id: i32,
        captured: [u32; 2],
        atlas: [u32; 2],
    },
    #[error("shape {shape_id} decoded RG8 identity mismatch: captured {captured}, atlas {atlas}")]
    SourceContentMismatch {
        shape_id: i32,
        captured: String,
        atlas: String,
    },
    #[error("invalid shape placement: {0}")]
    Placement(#[from] SdfCommandBuildError),
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub(crate) enum ShapeSdfCaptureError {
    #[error("shape id {shape_id} has no MasterData resource identity")]
    MissingResource { shape_id: i32 },
    #[error("shape asset {asset_key} is unavailable; legacy analytic fallback was used")]
    MissingAsset { asset_key: String },
    #[error("shape asset {asset_key} has invalid dimensions {width}x{height}")]
    InvalidDimensions {
        asset_key: String,
        width: i32,
        height: i32,
    },
    #[error("shape transform contains perspective")]
    PerspectiveTransform,
    #[error("shape asset {asset_key} pixels could not be read")]
    ReadPixels { asset_key: String },
}

#[allow(clippy::too_many_arguments)]
fn resolve_shape_sdf_command(
    affine: [f32; 6],
    shape: &ShapeElement,
    asset_key: &str,
    width: i32,
    height: i32,
    face_color: ResolvedColor,
    outline_color: ResolvedColor,
    dst_ltrb: [f32; 4],
    source_rg8_sha256: String,
) -> Result<ResolvedShapeSdfCommand, ShapeSdfCaptureError> {
    let source_size = [
        u32::try_from(width).map_err(|_| ShapeSdfCaptureError::InvalidDimensions {
            asset_key: asset_key.to_string(),
            width,
            height,
        })?,
        u32::try_from(height).map_err(|_| ShapeSdfCaptureError::InvalidDimensions {
            asset_key: asset_key.to_string(),
            width,
            height,
        })?,
    ];
    if source_size.contains(&0) {
        return Err(ShapeSdfCaptureError::InvalidDimensions {
            asset_key: asset_key.to_string(),
            width,
            height,
        });
    }
    let local_to_device = Affine2 {
        scale_x: affine[0],
        skew_y: affine[1],
        skew_x: affine[2],
        scale_y: affine[3],
        translate_x: affine[4],
        translate_y: affine[5],
    };
    let quad = [
        Point2::new(dst_ltrb[0], dst_ltrb[1]),
        Point2::new(dst_ltrb[2], dst_ltrb[1]),
        Point2::new(dst_ltrb[2], dst_ltrb[3]),
        Point2::new(dst_ltrb[0], dst_ltrb[3]),
    ]
    .map(|point| local_to_device.map_point(point));
    let rgb = |color: ResolvedColor| {
        [
            f32::from(color.r) / 255.0,
            f32::from(color.g) / 255.0,
            f32::from(color.b) / 255.0,
        ]
    };
    let layer_alpha = |alpha: f32| ((alpha * 255.0) as u32).min(255) as f32 / 255.0;
    Ok(ResolvedShapeSdfCommand {
        shape_id: shape.id,
        asset_key: asset_key.to_string(),
        source_size,
        source_rg8_sha256,
        quad,
        material: ShapeSdfMaterial::from_profile_values(
            rgb(face_color),
            layer_alpha(shape.alpha),
            rgb(outline_color),
            if shape.outline_size > 0.01 {
                layer_alpha(shape.outline_alpha)
            } else {
                0.0
            },
            shape.outline_size,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn captured_shape() -> ResolvedShapeSdfCommand {
        ResolvedShapeSdfCommand {
            shape_id: 7,
            asset_key: "custom_profile/shape/star".into(),
            source_size: [16, 8],
            source_rg8_sha256: "11".repeat(32),
            quad: [
                Point2::new(1.0, 2.0),
                Point2::new(17.0, 2.0),
                Point2::new(17.0, 10.0),
                Point2::new(1.0, 10.0),
            ],
            material: ShapeSdfMaterial::from_profile_values(
                [1.0, 0.0, 0.0],
                0.8,
                [0.0, 0.0, 1.0],
                0.5,
                0.2,
            ),
        }
    }

    fn atlas_entry() -> crate::sdf::shape_atlas::ShapeSdfAtlasEntry {
        crate::sdf::shape_atlas::ShapeSdfAtlasEntry {
            shape_id: 7,
            asset_key: "custom_profile/shape/star".into(),
            source_sha256: "22".repeat(32),
            source_rg8_sha256: "11".repeat(32),
            page: 3,
            rect: [4, 5, 16, 8],
            source_size: [16, 8],
        }
    }

    #[test]
    fn captured_shape_maps_typed_atlas_command_without_relayout() {
        let command = captured_shape()
            .to_sdf_command_from_entry(&atlas_entry(), 4)
            .expect("valid shape command");
        assert_eq!(command.kind, crate::sdf::tile::SdfPrimitiveKind::Shape);
        assert_eq!(command.atlas_set, 4);
        assert_eq!(command.atlas_page, 3);
        assert_eq!(command.atlas_rect, [4, 5, 16, 8]);
        assert_eq!(command.quad, captured_shape().quad);
    }

    #[test]
    fn captured_shape_rejects_decoded_source_identity_mismatch() {
        let mut entry = atlas_entry();
        entry.source_rg8_sha256 = "33".repeat(32);
        assert!(matches!(
            captured_shape().to_sdf_command_from_entry(&entry, 4),
            Err(ShapeSdfCommandError::SourceContentMismatch { shape_id: 7, .. })
        ));
    }
}

#[cfg(feature = "skia-oracle")]
#[allow(dead_code)]
pub(crate) fn capture_shape_sdf(
    canvas: &Canvas,
    shape: &ShapeElement,
    md: &MasterData,
    assets: Option<&AssetStore>,
    observer: &mut dyn FnMut(Result<ResolvedShapeSdfCommand, ShapeSdfCaptureError>),
) {
    let affine = canvas
        .local_to_device_as_3x3()
        .to_affine()
        .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    capture_shape_sdf_from_affine(affine, shape, md, assets, observer);
}

/// Resolves a shape's SDF command with the device transform supplied directly,
/// so the capture needs no canvas — and no raster backend: the source identity
/// comes from the asset store's decoded-identity cache.
pub(crate) fn capture_shape_sdf_from_affine(
    affine: [f32; 6],
    shape: &ShapeElement,
    md: &MasterData,
    assets: Option<&AssetStore>,
    observer: &mut dyn FnMut(Result<ResolvedShapeSdfCommand, ShapeSdfCaptureError>),
) {
    let color = md.resolve_color(shape.color_id).unwrap_or(ResolvedColor {
        r: 128,
        g: 128,
        b: 128,
        a: 255,
    });
    let outline_color = md
        .resolve_color(shape.outline_color_id)
        .unwrap_or(ResolvedColor {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        });
    let shape_info = md.resolve_resource("shape", shape.id);
    let has_resource_identity = shape_info.is_some();
    let file_name = shape_info
        .as_ref()
        .map(|r| r.file_name.as_str())
        .unwrap_or("square");
    let asset_key = format!("custom_profile/shape/{file_name}");
    let identity = assets
        .ok_or(crate::assets::ShapeSourceIdentityError::Missing)
        .and_then(|store| store.shape_sdf_source_identity_for_key(&asset_key));
    let captured = match identity {
        Ok(identity) => {
            if !has_resource_identity {
                Err(ShapeSdfCaptureError::MissingResource { shape_id: shape.id })
            } else {
                let sprite_w = identity.width as f32;
                let sprite_h = identity.height as f32;
                // The historical draw rect: from_xywh(-w/2, -h/2, w, h).
                let left = -sprite_w / 2.0;
                let top = -sprite_h / 2.0;
                resolve_shape_sdf_command(
                    affine,
                    shape,
                    &asset_key,
                    identity.width,
                    identity.height,
                    color,
                    outline_color,
                    [left, top, left + sprite_w, top + sprite_h],
                    identity.rg8_sha256,
                )
            }
        }
        Err(crate::assets::ShapeSourceIdentityError::Unreadable) => {
            Err(ShapeSdfCaptureError::ReadPixels { asset_key })
        }
        Err(crate::assets::ShapeSourceIdentityError::Missing) => {
            if has_resource_identity {
                Err(ShapeSdfCaptureError::MissingAsset { asset_key })
            } else {
                Err(ShapeSdfCaptureError::MissingResource { shape_id: shape.id })
            }
        }
    };
    observer(captured);
}
