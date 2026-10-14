//! Canvas drawing, PNG round-trip and input validation tests.

use super::*;
#[test]
fn image_and_shape_share_source_over() {
    let mut canvas = Canvas::new(4, 4, [0, 0, 0, 0]).unwrap();
    canvas
        .shape(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 4.0,
                height: 4.0,
            },
            ShapePrimitive::Rect,
            [255, 0, 0, 255],
            [0; 4],
            0.0,
        )
        .unwrap();
    let image = Image::from_rgba(1, 1, vec![0, 0, 255, 128]).unwrap();
    canvas
        .image(
            &image,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 4.0,
                height: 4.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            1.0,
        )
        .unwrap();
    assert_eq!(&canvas.premultiplied_rgba()[20..24], &[127, 0, 128, 255]);
}
#[test]
fn png_preserves_alpha() {
    let canvas = Canvas::new(1, 1, [255, 0, 0, 128]).unwrap();
    let decoded = codec::png::decode(&canvas.encode_png().unwrap()).unwrap();
    assert_eq!(decoded.pixels, vec![255, 0, 0, 128]);
}

#[test]
fn clip_bounds_shapes_and_curves() {
    let mut canvas = Canvas::new(8, 8, [0; 4]).unwrap();
    canvas
        .set_clip(Some(Rect {
            x: 2.0,
            y: 2.0,
            width: 4.0,
            height: 4.0,
        }))
        .unwrap();
    canvas
        .shape(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 8.0,
                height: 8.0,
            },
            ShapePrimitive::Ellipse,
            [0, 255, 0, 255],
            [0; 4],
            0.0,
        )
        .unwrap();
    canvas
        .quadratic(
            [[0.0, 4.0], [4.0, 0.0], [8.0, 4.0]],
            2.0,
            [255, 255, 255, 255],
        )
        .unwrap();
    for y in 0..8 {
        for x in 0..8 {
            if !(2..6).contains(&x) || !(2..6).contains(&y) {
                assert_eq!(canvas.premultiplied_rgba()[(y * 8 + x) * 4 + 3], 0);
            }
        }
    }
    assert!(canvas
        .premultiplied_rgba()
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| p[3] > 0));
}

#[test]
fn asset_mask_shapes_are_rejected_instead_of_drawing_nothing() {
    let mut canvas = Canvas::new(4, 4, [0; 4]).expect("canvas");
    let result = canvas.shape(
        Rect {
            x: 0.0,
            y: 0.0,
            width: 4.0,
            height: 4.0,
        },
        ShapePrimitive::AssetMask {
            resource: sekai_profile_core::ResourceKey {
                namespace: "assets".into(),
                key: "custom_profile/shape/heart".into(),
            },
        },
        [255, 0, 0, 255],
        [0; 4],
        0.0,
    );
    assert!(result.is_err());
    assert!(canvas.premultiplied_rgba().iter().all(|&value| value == 0));
}

#[test]
fn huge_png_is_rejected_before_decode() {
    let mut data = vec![137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82];
    data.extend_from_slice(&u32::MAX.to_be_bytes());
    data.extend_from_slice(&u32::MAX.to_be_bytes());
    assert!(Image::from_png(&data).is_err());
}

#[test]
fn invalid_geometry_is_rejected() {
    assert!(Canvas::new(u32::MAX, u32::MAX, [0; 4]).is_err());
    let mut canvas = Canvas::new(2, 2, [0; 4]).unwrap();
    assert!(canvas
        .set_clip(Some(Rect {
            x: f32::NAN,
            y: 0.0,
            width: 1.0,
            height: 1.0
        }))
        .is_err());
}
