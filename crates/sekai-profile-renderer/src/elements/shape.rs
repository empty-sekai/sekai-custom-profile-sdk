//! Shape element SDF capture.
//!
//! Shapes are rasterized through their sprite's distance field so they stay
//! sharp under the card's transforms. Capture resolves a shape's sprite
//! identity, device quad and material into a command the shape atlas can
//! place.

use crate::assets::AssetStore;
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

    fn element(alpha: f32, outline_alpha: f32, outline_size: f32) -> ShapeElement {
        use crate::types::{ObjectData, Quaternion, Vec3};
        ShapeElement {
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
            alpha,
            color_id: 1,
            id: 7,
            outline_alpha,
            outline_color_id: 1,
            outline_size,
        }
    }

    struct ShapeProvider;

    impl crate::masterdata::MasterDataProvider for ShapeProvider {
        fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> {
            None
        }
        fn get_card(&self, _: i32) -> Option<crate::types::CardEntry> {
            None
        }
        fn resolve_color(&self, color_id: i32) -> Option<ResolvedColor> {
            (color_id == 1).then_some(ResolvedColor {
                r: 68,
                g: 68,
                b: 102,
                a: 255,
            })
        }
        fn resolve_font(&self, _: i32) -> Option<String> {
            None
        }
        fn resolve_stamp(&self, _: i32) -> Option<String> {
            None
        }
        fn resolve_resource(
            &self,
            resource_type: &str,
            id: i32,
        ) -> Option<crate::masterdata::ResourceInfo> {
            (resource_type == "shape" && id == 7).then(|| crate::masterdata::ResourceInfo {
                file_name: "star".into(),
                load_val: "custom_profile/shape_hd".into(),
                resource_type: "shape".into(),
            })
        }
        fn resolve_honor(&self, _: i32, _: i32) -> Option<crate::masterdata::ResolvedHonor> {
            None
        }
        fn get_bonds_honor(&self, _: i32) -> Option<crate::types::BondsHonorEntry> {
            None
        }
        fn get_bonds_honor_word(&self, _: i64) -> Option<crate::types::BondsHonorWordEntry> {
            None
        }
        fn get_honor(&self, _: i32) -> Option<crate::types::HonorEntry> {
            None
        }
        fn resolve_unit_vs_sd(&self, _: i32, _: i32) -> i32 {
            0
        }
        fn font_count(&self) -> usize {
            0
        }
        fn color_count(&self) -> usize {
            1
        }
    }

    fn capture(shape: &ShapeElement) -> Result<ResolvedShapeSdfCommand, ShapeSdfCaptureError> {
        let store = AssetStore::new(4);
        let pixels = [200u8, 0, 0, 255].repeat(4);
        store.put(
            "custom_profile/shape_hd/star".into(),
            crate::codec::png::encode_rgba(2, 2, &pixels).expect("encode shape"),
        );
        let md = MasterData::new(std::sync::Arc::new(ShapeProvider));
        let mut captured = None;
        capture_shape_sdf_from_affine(
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            shape,
            &md,
            Some(&store),
            &mut |result| captured = Some(result),
        );
        captured.expect("capture reports a result")
    }

    #[test]
    fn shape_asset_key_follows_the_master_resource_load_value() {
        let command = capture(&element(1.0, 1.0, 0.2)).expect("captured shape");
        assert_eq!(command.asset_key, "custom_profile/shape_hd/star");
        assert_eq!(command.source_size, [2, 2]);
        let mut missing = element(1.0, 1.0, 0.2);
        missing.id = 8;
        assert_eq!(
            capture(&missing),
            Err(ShapeSdfCaptureError::MissingResource { shape_id: 8 })
        );
    }

    #[test]
    fn thin_outline_keeps_its_alpha() {
        let command = capture(&element(0.5, 0.5, 0.01)).expect("captured shape");
        assert_eq!(command.material.face[3], 128.0 / 255.0);
        assert_eq!(command.material.outline[3], 0.5);
        let none = capture(&element(1.0, 0.8, 0.0)).expect("captured shape");
        assert_eq!(none.material.outline[3], 0.8);
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
                identity.width,
                identity.height,
                color,
                outline_color,
                [left, top, left + sprite_w, top + sprite_h],
                identity.rg8_sha256,
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
