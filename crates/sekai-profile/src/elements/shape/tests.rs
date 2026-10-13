//! Shape SDF capture and atlas command resolution tests.

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
            collection_type: crate::masterdata::CollectionResourceType::None,
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
