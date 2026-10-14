//! Shape element SDF capture.
//!
//! Shapes are rasterized through their sprite's distance field so they stay
//! sharp under the card's transforms. Capture resolves a shape's sprite
//! identity, device quad and material into a command the shape atlas can
//! place.

use crate::assets::{AssetStore, ShapeSdfSourceIdentity};
use crate::masterdata::{MasterData, ResolvedColor};
use crate::sdf::shape::ShapeSdfMaterial;
use crate::sdf::tile::{Affine2, Point2, SdfCommandBuildError, SdfDrawCommand};
use crate::types::ShapeElement;

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
    #[error("shape asset {asset_key} is unavailable")]
    MissingAsset { asset_key: String },
    #[error("shape asset {asset_key} has invalid dimensions {width}x{height}")]
    InvalidDimensions {
        asset_key: String,
        width: i32,
        height: i32,
    },
    #[error("shape asset {asset_key} pixels could not be read")]
    ReadPixels { asset_key: String },
}

/// Builds the SDF command of a shape whose sprite `identity` is drawn over the
/// layout-space rectangle `dst_ltrb` and mapped to device space by `affine`.
fn resolve_shape_sdf_command(
    affine: [f32; 6],
    shape: &ShapeElement,
    asset_key: &str,
    identity: ShapeSdfSourceIdentity,
    face_color: ResolvedColor,
    outline_color: ResolvedColor,
    dst_ltrb: [f32; 4],
) -> Result<ResolvedShapeSdfCommand, ShapeSdfCaptureError> {
    let ShapeSdfSourceIdentity {
        width,
        height,
        rg8_sha256: source_rg8_sha256,
    } = identity;
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
    Ok(ResolvedShapeSdfCommand {
        shape_id: shape.id,
        asset_key: asset_key.to_string(),
        source_size,
        source_rg8_sha256,
        quad,
        material: ShapeSdfMaterial::from_profile_values(
            rgb(face_color),
            shape.alpha,
            rgb(outline_color),
            shape.outline_alpha,
            shape.outline_size,
        ),
    })
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
    // Without any colour table the face is white and the outline black, the
    // same defaults the shared scene uses.
    let color = md
        .resolve_color_or_default(shape.color_id)
        .unwrap_or(ResolvedColor {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        });
    let outline_color = md
        .resolve_color_or_default(shape.outline_color_id)
        .unwrap_or(ResolvedColor {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        });
    // The sprite comes from the shape's master resource; without one the
    // game loads no sprite, so there is nothing to resolve.
    let Some(asset_key) = md
        .resolve_resource("shape", shape.id)
        .map(|resource| resource.asset_key())
    else {
        observer(Err(ShapeSdfCaptureError::MissingResource {
            shape_id: shape.id,
        }));
        return;
    };
    let identity = assets
        .ok_or(crate::assets::ShapeSourceIdentityError::Missing)
        .and_then(|store| store.shape_sdf_source_identity_for_key(&asset_key));
    let captured = match identity {
        Ok(identity) => {
            let sprite_w = identity.width as f32;
            let sprite_h = identity.height as f32;
            // The sprite is drawn at its native size, centred on the element.
            let left = -sprite_w / 2.0;
            let top = -sprite_h / 2.0;
            resolve_shape_sdf_command(
                affine,
                shape,
                &asset_key,
                identity,
                color,
                outline_color,
                [left, top, left + sprite_w, top + sprite_h],
            )
        }
        Err(crate::assets::ShapeSourceIdentityError::Unreadable) => {
            Err(ShapeSdfCaptureError::ReadPixels { asset_key })
        }
        Err(crate::assets::ShapeSourceIdentityError::Missing) => {
            Err(ShapeSdfCaptureError::MissingAsset { asset_key })
        }
    };
    observer(captured);
}

#[cfg(test)]
mod tests;
