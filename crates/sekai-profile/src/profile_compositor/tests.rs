//! Scalar and packet executor tests over images, shapes, masks, clips, blend modes,
//! isolation groups and component controls.

use std::collections::BTreeMap;

use sekai_profile_core::profile_scene::{
    ComponentControlSource, ComponentControlState, ResolvedProfileScene,
};
use sekai_profile_core::{
    AuthoredElementKind, BlendMode, CommandControlBinding, CompositeOperation, FontRole, LayerKind,
    LayerSource, LinearGradient, Rect, ResourceKey, SemanticCommandPayload, SemanticCommandSource,
    ShapePrimitive, StableId, TextSource,
};
use sha2::{Digest, Sha256};

use super::*;
use crate::assets::AssetStore;
use crate::render_object::{
    MappedRenderObjectStore, RenderObjectKind, RenderObjectStoreWriter, RenderObjectWrite,
};

fn packet_simd_available() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

fn test_layer(id: StableId, matrix: Matrix2d) -> LayerSource {
    LayerSource {
        id,
        parent_id: None,
        kind: LayerKind::Image,
        authored_kind: AuthoredElementKind::Other,
        authored_index: 0,
        game_layer: 0,
        z: 0,
        authored_visible: true,
        source_content: String::new(),
        resolved_parameters: BTreeMap::new(),
        bounds: Rect::default(),
        quad: [[0.0; 2]; 4],
        matrix,
        hit_geometry: [[0.0; 2]; 4],
        line_indent: None,
    }
}

fn image_command(
    id: &str,
    layer_id: StableId,
    resource_key: &str,
    bounds: Rect,
    uv: Rect,
    blend_mode: BlendMode,
) -> SemanticCommandSource {
    let mut command = SemanticCommandSource::image(
        StableId::derive("command", id.as_bytes()),
        layer_id,
        id,
        ResourceKey {
            namespace: "assets".into(),
            key: resource_key.into(),
        },
        bounds,
    );
    command.blend_mode = blend_mode;
    if let SemanticCommandPayload::Image { uv: target, .. } = &mut command.payload {
        *target = uv;
    }
    command
}

fn composite_command(
    id: &str,
    layer_id: StableId,
    operation: CompositeOperation,
) -> SemanticCommandSource {
    let mut command = SemanticCommandSource::composite(
        StableId::derive("command", id.as_bytes()),
        layer_id,
        id,
        Rect::default(),
    );
    if let SemanticCommandPayload::Composite {
        operation: target, ..
    } = &mut command.payload
    {
        *target = operation;
    }
    command
}

fn shape_command(
    id: &str,
    layer_id: StableId,
    bounds: Rect,
    primitive: ShapePrimitive,
    fill: [f32; 4],
    gradient: Option<LinearGradient>,
    stroke: [f32; 4],
    stroke_width: f32,
) -> SemanticCommandSource {
    let mut command = SemanticCommandSource::shape(
        StableId::derive("command", id.as_bytes()),
        layer_id,
        id,
        bounds,
        primitive,
    );
    if let SemanticCommandPayload::Shape {
        fill: target_fill,
        gradient: target_gradient,
        stroke: target_stroke,
        stroke_width: target_stroke_width,
        ..
    } = &mut command.payload
    {
        *target_fill = fill;
        *target_gradient = gradient;
        *target_stroke = stroke;
        *target_stroke_width = stroke_width;
    }
    command
}

fn scene(layer: LayerSource, commands: Vec<SemanticCommandSource>) -> ResolvedProfileScene {
    ResolvedProfileScene {
        layers: vec![layer],
        commands,
        interaction_regions: Vec::new(),
        controls: Vec::new(),
    }
}

/// SHA-256 of the face the Live Master progress tests pin their pixels to:
/// `SourceHanSansSC-Medium.otf` from Source Han Sans 2.005.
const PROGRESS_FACE_SHA256: &str =
    "1df61d31687d04fd2f928a3bb6ca6cd61f0e988cc267cf317f32406edbb49f70";

/// Whether the configured progress face is the one the pinned pixels were
/// rendered with. Without it, or with another face, the pixel checks skip.
fn progress_face_is_pinned() -> bool {
    let Some(bytes) = crate::sdf::outline::load_font_bytes_for_family(
        crate::widgets::theme::fonts::LIVE_MASTER_PROGRESS,
    ) else {
        eprintln!("skipping: FONT_DIR does not provide the progress face");
        return false;
    };
    if hex::encode(Sha256::digest(bytes.as_slice())) != PROGRESS_FACE_SHA256 {
        eprintln!("skipping: the progress face is not SourceHanSansSC-Medium 2.005");
        return false;
    }
    true
}

#[test]
fn native_compositor_filters_inactive_tab_commands() {
    let layer_id = StableId(10);
    let control_id = StableId(20);
    let mut active = image_command(
        "active",
        layer_id,
        "active",
        Rect::default(),
        Rect::default(),
        BlendMode::SrcOver,
    );
    active
        .control_bindings
        .push(CommandControlBinding::TabOption {
            control_id,
            value: "character_rank".into(),
        });
    let mut inactive = active.clone();
    inactive.role = "inactive".into();
    inactive.control_bindings = vec![CommandControlBinding::TabOption {
        control_id,
        value: "challenge_live_rank".into(),
    }];
    let mut scene = scene(test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]), vec![]);
    scene.controls.push(ComponentControlSource {
        id: control_id,
        layer_id,
        role: "rank-mode".into(),
        state: ComponentControlState::Tabs {
            options: vec!["character_rank".into(), "challenge_live_rank".into()],
            active: "character_rank".into(),
        },
    });

    assert!(
        evaluate_command_control_state(&scene, &active)
            .expect("active state")
            .visible
    );
    assert!(
        !evaluate_command_control_state(&scene, &inactive)
            .expect("inactive state")
            .visible
    );
}

#[test]
fn native_compositor_resolves_scroll_content_translation() {
    let layer_id = StableId(11);
    let control_id = StableId(21);
    let mut command = image_command(
        "scroll-content",
        layer_id,
        "scroll-content",
        Rect {
            width: 10.0,
            height: 10.0,
            ..Rect::default()
        },
        Rect::default(),
        BlendMode::SrcOver,
    );
    command
        .control_bindings
        .push(CommandControlBinding::ScrollContent { control_id });
    let mut scene = scene(test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]), vec![]);
    scene.controls.push(ComponentControlSource {
        id: control_id,
        layer_id,
        role: "scroll".into(),
        state: ComponentControlState::Scroll {
            offset: 10.0,
            min: 0.0,
            max: 20.0,
            viewport_extent: 10.0,
            content_extent: 30.0,
            step: 5.0,
        },
    });

    let state = evaluate_command_control_state(&scene, &command).expect("scroll state");
    assert!(state.visible);
    assert_eq!(state.translate_y, -10.0);
}

#[test]
fn missing_mmap_texture_uses_asset_store_without_leaving_software_compositor() {
    let layer_id = StableId(301);
    let scene = scene(
        test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        vec![image_command(
            "cos-fallback",
            layer_id,
            "missing/source",
            Rect {
                width: 2.0,
                height: 2.0,
                ..Rect::default()
            },
            Rect {
                width: 1.0,
                height: 1.0,
                ..Rect::default()
            },
            BlendMode::SrcOver,
        )],
    );
    let (_temp, store) = store(vec![("dummy", 1, 1, vec![0; 4])]);
    let assets = AssetStore::new(8);
    let encoded = crate::codec::png::encode_rgba(2, 2, &[255, 0, 0, 255].repeat(4))
        .expect("encode the source PNG");
    assets.put("missing/source".into(), encoded);
    let mut pixels = vec![0; 16];
    let stats = render_image_commands_into(
        &scene,
        &store,
        2,
        2,
        ImageExecutor::Scalar,
        &mut pixels,
        None,
        None,
        None,
        Some(&assets),
    )
    .unwrap();

    assert_eq!(pixels, [255, 0, 0, 255].repeat(4));
    assert_eq!(stats.source_fallback_object_count, 1);
    assert_eq!(stats.source_fallback_bytes, 16);
    assert_eq!(stats.image_command_count, 1);
}

/// The progress count draws as a plain anti-aliased white fill through the
/// FreeType raster, not through the SDF atlas: "73" at 20 px, centred on
/// x = 80 with its baseline at y = 30.
#[test]
fn live_master_progress_draws_simple_text_pixels_without_sdf() {
    if !progress_face_is_pinned() {
        return;
    }

    let layer_id = StableId(302);
    let mut command = SemanticCommandSource::profile_text(
        StableId(303),
        layer_id,
        "honor-4242-progress",
        "userHonorMissions.4242",
        "73",
        FontRole::RegionFontId(1),
    );
    command.matrix = [1.0, 0.0, 0.0, 1.0, 80.0, 20.0];
    if let SemanticCommandPayload::Text {
        source,
        size,
        color,
        alignment,
        ..
    } = &mut command.payload
    {
        *source = TextSource::ProfileField {
            field: "userHonorMissions.4242".into(),
            value: "73".into(),
        };
        *size = 20.0;
        *color = [1.0; 4];
        *alignment = 2;
    }
    let scene = scene(
        test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        vec![command],
    );
    let (_temp, store) = store(vec![("dummy", 1, 1, vec![0; 4])]);
    let mut actual = vec![0; 200 * 80 * 4];
    let stats = render_image_commands_into(
        &scene,
        &store,
        200,
        80,
        ImageExecutor::Scalar,
        &mut actual,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    // White over transparency stays premultiplied white: every channel
    // equals the coverage.
    assert!(actual
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| pixel[0] == pixel[3] && pixel[1] == pixel[3] && pixel[2] == pixel[3]));
    assert!(actual.iter().any(|&value| value != 0), "drew nothing");
    assert_eq!(
        hex::encode(Sha256::digest(&actual)),
        "be9bb8841d6ce8748fe6b1c8e24924dd88f2aec57805b1e2082462e2e1a9c4e6"
    );
    assert_eq!(stats.text_command_count, 1);
    assert_eq!(stats.skipped_text_command_count, 0);
}

/// The digit raster blends over content that is already on the page, not
/// just over transparency: opaque white source-over each destination
/// channel by the remapped coverage, at a fractional placement.
#[test]
fn live_master_progress_blends_exactly_over_existing_content() {
    if !progress_face_is_pinned() {
        return;
    }

    // Deterministic premultiplied noise: alpha >= every channel.
    let background: Vec<u8> = {
        let mut state = 0x0123_4567_89AB_CDEFu64;
        (0..200 * 80)
            .flat_map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let alpha = (state >> 32) as u8;
                [
                    ((state >> 8) as u8).min(alpha),
                    ((state >> 16) as u8).min(alpha),
                    ((state >> 24) as u8).min(alpha),
                    alpha,
                ]
            })
            .collect()
    };

    let layer_id = StableId(312);
    let mut command = SemanticCommandSource::profile_text(
        StableId(313),
        layer_id,
        "honor-4243-progress",
        "userHonorMissions.4243",
        "2470",
        FontRole::RegionFontId(1),
    );
    command.matrix = [1.0, 0.0, 0.0, 1.0, 100.7, 50.3];
    if let SemanticCommandPayload::Text {
        source,
        size,
        color,
        alignment,
        ..
    } = &mut command.payload
    {
        *source = TextSource::ProfileField {
            field: "userHonorMissions.4243".into(),
            value: "2470".into(),
        };
        *size = 20.0;
        *color = [1.0; 4];
        *alignment = 2;
    }
    let scene = scene(
        test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        vec![command],
    );
    let (_temp, store) = store(vec![("dummy", 1, 1, vec![0; 4])]);
    let mut actual = background.clone();
    render_image_commands_into(
        &scene,
        &store,
        200,
        80,
        ImageExecutor::Scalar,
        &mut actual,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    assert_ne!(actual, background, "drew nothing");
    assert_eq!(
        hex::encode(Sha256::digest(&actual)),
        "2043606f53726ff5064d0e02ae3e02cdc08e21dd33489bf4db5143a03e60de22"
    );
}

/// A placed honor may be scaled or rotated, and its digits follow the
/// honor's transform: a fractional down-scale, whole-pixel and anisotropic
/// up-scales, a 30 degree rotation and a quarter turn each ink a pinned
/// pixel box with pinned pixels.
#[test]
fn live_master_progress_follows_scaled_and_rotated_honors() {
    if !progress_face_is_pinned() {
        return;
    }

    let (sin, cos) = 30f32.to_radians().sin_cos();
    for (matrix, bounds, digest) in [
        (
            [0.8, 0.0, 0.0, 0.8, 120.3, 60.6],
            (103, 57, 137, 68),
            "82280760f6d67309701d18835534180563b11865cdbf3b4ba89ac4c3f5e67258",
        ),
        (
            [2.0, 0.0, 0.0, 2.0, 150.0, 40.0],
            (107, 30, 192, 60),
            "b6e0ba809c3a812d9dc26d07683d6a7fdfe8093078095888408bed33cbe9e1e1",
        ),
        (
            [1.5, 0.0, 0.0, 0.75, 110.0, 70.0],
            (78, 67, 142, 77),
            "933cc09291d5dec6bea60fc42248c18e419db78461c16b44a2804c4e7132b4bc",
        ),
        (
            [cos, sin, -sin, cos, 140.0, 60.0],
            (116, 46, 159, 77),
            "57d1236a0e40dd17b422f5e6608f664907550399d6a610cc6170e3599730bf98",
        ),
        (
            [0.0, 1.0, -1.0, 0.0, 150.0, 60.0],
            (140, 38, 154, 81),
            "e9a9e2e25777c02b7f69825b392e131e3c4a4b039b17fb0bcedad3ad8966fe09",
        ),
    ] {
        let layer_id = StableId(322);
        let mut command = SemanticCommandSource::profile_text(
            StableId(323),
            layer_id,
            "honor-4244-progress",
            "userHonorMissions.4244",
            "2470",
            FontRole::RegionFontId(1),
        );
        command.matrix = matrix;
        if let SemanticCommandPayload::Text {
            source,
            size,
            color,
            alignment,
            ..
        } = &mut command.payload
        {
            *source = TextSource::ProfileField {
                field: "userHonorMissions.4244".into(),
                value: "2470".into(),
            };
            *size = 20.0;
            *color = [1.0; 4];
            *alignment = 2;
        }
        let scene = scene(
            test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            vec![command],
        );
        let (_temp, store) = store(vec![("dummy", 1, 1, vec![0; 4])]);
        let mut actual = vec![0; 300 * 120 * 4];
        render_image_commands_into(
            &scene,
            &store,
            300,
            120,
            ImageExecutor::Scalar,
            &mut actual,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        assert!(
            actual.iter().any(|&value| value != 0),
            "{matrix:?} drew nothing"
        );
        let inked = |pixels: &[u8]| {
            let mut bounds = (usize::MAX, usize::MAX, 0, 0);
            for (index, pixel) in pixels.chunks_exact(4).enumerate() {
                if pixel[3] != 0 {
                    let (x, y) = (index % 300, index / 300);
                    bounds = (
                        bounds.0.min(x),
                        bounds.1.min(y),
                        bounds.2.max(x),
                        bounds.3.max(y),
                    );
                }
            }
            bounds
        };
        assert_eq!(inked(&actual), bounds, "{matrix:?}: inked bounds");
        assert_eq!(
            hex::encode(Sha256::digest(&actual)),
            digest,
            "{matrix:?}: pixels"
        );
    }
}

fn store(objects: Vec<(&str, u32, u32, Vec<u8>)>) -> (tempfile::TempDir, MappedRenderObjectStore) {
    let temp = tempfile::tempdir().expect("tempdir");
    let output = temp.path().join("store");
    let mut writer =
        RenderObjectStoreWriter::create(&output, "profile-compositor-test", 4096).expect("writer");
    for (key, width, height, pixels) in objects {
        let source_sha = hex::encode(Sha256::digest(key.as_bytes()));
        writer
            .add(RenderObjectWrite {
                key,
                kind: RenderObjectKind::Texture,
                source_sha256: &source_sha,
                width,
                height,
                row_bytes: width * 4,
                pixels: &pixels,
            })
            .expect("add object");
    }
    let manifest = writer.finish().expect("manifest");
    let mapped = MappedRenderObjectStore::open(manifest).expect("mapped store");
    (temp, mapped)
}

#[test]
fn integer_premultiplied_multiply_matches_float_quantization() {
    for left in u8::MIN..=u8::MAX {
        for right in u8::MIN..=u8::MAX {
            let reference = quantize(f32::from(left) / 255.0 * f32::from(right) / 255.0);
            assert_eq!(
                mul_div_255_round(left, right),
                reference,
                "{left} * {right}"
            );
            let product = u32::from(left) * u32::from(right);
            let biased = product + 128;
            let packet_formula = ((biased + (biased >> 8)) >> 8) as u8;
            assert_eq!(packet_formula, reference, "packet {left} * {right}");
        }
    }
}

#[test]
fn semantic_shape_simd_matches_scalar_for_gradient_stroke_and_affine_geometry() {
    let layer_id = StableId(91);
    let bounds = Rect {
        x: 3.25,
        y: 2.5,
        width: 31.5,
        height: 19.25,
    };
    let mut rounded = shape_command(
        "rounded-gradient",
        layer_id,
        bounds,
        ShapePrimitive::RoundedRect { radius: [6.5, 4.0] },
        [0.2, 0.7, 0.3, 0.8],
        Some(LinearGradient {
            start: [0.1, 0.2],
            end: [0.9, 0.8],
            start_color: [0.9, 0.1, 0.4, 0.7],
            end_color: [0.1, 0.8, 0.6, 0.9],
        }),
        [0.8, 0.2, 0.1, 0.65],
        2.25,
    );
    rounded.matrix = [0.97, 0.13, -0.09, 1.02, 4.0, 3.0];
    let mut ellipse = shape_command(
        "ellipse-solid",
        layer_id,
        Rect {
            x: 20.0,
            y: 13.0,
            width: 27.0,
            height: 21.0,
        },
        ShapePrimitive::Ellipse,
        [0.15, 0.35, 0.95, 0.72],
        None,
        [0.9, 0.8, 0.1, 0.55],
        1.5,
    );
    ellipse.matrix = [1.0, -0.07, 0.11, 0.96, -1.0, 2.0];
    let scene = scene(
        test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        vec![rounded, ellipse],
    );
    let (_temp, store) = store(vec![("dummy", 1, 1, vec![0; 4])]);
    let background = [17, 31, 47, 191].repeat(73 * 47);
    let mut scalar = background.clone();
    render_image_commands_into(
        &scene,
        &store,
        73,
        47,
        ImageExecutor::Scalar,
        &mut scalar,
        None,
        None,
        None,
        None,
    )
    .expect("scalar semantic Shape oracle");
    let mut simd = background;

    if packet_simd_available() && std::arch::is_x86_feature_detected!("fma") {
        let stats = render_image_commands_into(
            &scene,
            &store,
            73,
            47,
            ImageExecutor::Simd,
            &mut simd,
            None,
            None,
            None,
            None,
        )
        .expect("SIMD semantic Shape candidate");
        assert_eq!(simd, scalar);
        assert!(stats.simd_packet_count > 0);
        assert_eq!(stats.scalar_fragment_count, 0);
    } else {
        assert_eq!(simd, [17, 31, 47, 191].repeat(73 * 47));
    }
}

#[test]
fn partial_uv_and_layer_translation_sample_nearest_texels() {
    let (_temp, store) = store(vec![(
        "texture:assets/strip",
        4,
        1,
        vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ],
    )]);
    let layer_id = StableId::derive("layer", b"uv");
    let command = image_command(
        "right-half",
        layer_id,
        "strip",
        Rect {
            x: 0.0,
            y: 0.0,
            width: 2.0,
            height: 1.0,
        },
        Rect {
            x: 0.5,
            y: 0.0,
            width: 0.5,
            height: 1.0,
        },
        BlendMode::SrcOver,
    );
    let output = render_image_scene_scalar(
        &scene(
            test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 1.0, 1.0]),
            vec![command],
        ),
        &store,
        4,
        3,
    )
    .expect("render");
    // The right half of the strip (blue, white) lands one pixel right and
    // one row down; nothing else is drawn.
    let mut expected = vec![0u8; 4 * 3 * 4];
    expected[20..28].copy_from_slice(&[0, 0, 255, 255, 255, 255, 255, 255]);
    assert_eq!(output.pixels, expected);
    assert_eq!(output.stats.sampled_fragment_count, 2);
}

#[test]
fn visible_scene_bounds_include_layer_and_scroll_translation() {
    let layer_id = StableId(401);
    let control_id = StableId(402);
    let mut command = image_command(
        "scroll-bounds",
        layer_id,
        "dummy",
        Rect {
            x: 2.0,
            y: 3.0,
            width: 10.0,
            height: 6.0,
        },
        Rect::default(),
        BlendMode::SrcOver,
    );
    command
        .control_bindings
        .push(CommandControlBinding::ScrollContent { control_id });
    let mut scene = scene(
        test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 20.0, 30.0]),
        vec![command],
    );
    scene.controls.push(ComponentControlSource {
        id: control_id,
        layer_id,
        role: "scroll".into(),
        state: ComponentControlState::Scroll {
            offset: 4.0,
            min: 0.0,
            max: 10.0,
            viewport_extent: 10.0,
            content_extent: 20.0,
            step: 1.0,
        },
    });

    assert_eq!(
        visible_scene_device_bounds(&scene, 2.0).unwrap(),
        Some((20, 27, 14, 10))
    );
}

#[test]
fn tight_translated_scene_matches_full_surface_crop_exactly() {
    let (_temp, store) = store(vec![(
        "texture:assets/tight",
        2,
        2,
        vec![
            255, 0, 0, 255, 0, 255, 0, 192, 0, 0, 255, 128, 255, 255, 255, 64,
        ],
    )]);
    let layer_id = StableId(403);
    let scene = scene(
        test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 7.0, 5.0]),
        vec![image_command(
            "tight",
            layer_id,
            "tight",
            Rect {
                width: 2.0,
                height: 2.0,
                ..Rect::default()
            },
            Rect {
                width: 1.0,
                height: 1.0,
                ..Rect::default()
            },
            BlendMode::SrcOver,
        )],
    );
    let (left, top, width, height) = visible_scene_device_bounds(&scene, 0.0)
        .unwrap()
        .expect("visible bounds");
    assert_eq!((left, top, width, height), (7, 5, 2, 2));

    let mut full = vec![0; 16 * 12 * 4];
    render_image_commands_into(
        &scene,
        &store,
        16,
        12,
        ImageExecutor::Scalar,
        &mut full,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let mut translated = scene.clone();
    translated.layers[0].matrix[4] -= left as f32;
    translated.layers[0].matrix[5] -= top as f32;
    let mut tight = vec![0; width as usize * height as usize * 4];
    render_image_commands_into(
        &translated,
        &store,
        width,
        height,
        ImageExecutor::Scalar,
        &mut tight,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let mut cropped = Vec::with_capacity(tight.len());
    for y in top as usize..(top as usize + height as usize) {
        let start = (y * 16 + left as usize) * 4;
        cropped.extend_from_slice(&full[start..start + width as usize * 4]);
    }
    assert_eq!(tight, cropped);
}

#[test]
fn isolation_dst_in_masks_only_the_isolated_group() {
    let (_temp, store) = store(vec![
        (
            "texture:assets/background",
            2,
            1,
            vec![255, 0, 0, 255, 255, 0, 0, 255],
        ),
        (
            "texture:assets/mask",
            2,
            1,
            vec![255, 255, 255, 255, 0, 0, 0, 0],
        ),
    ]);
    let layer_id = StableId::derive("layer", b"mask");
    let bounds = Rect {
        x: 0.0,
        y: 0.0,
        width: 2.0,
        height: 1.0,
    };
    let commands = vec![
        composite_command("begin", layer_id, CompositeOperation::BeginIsolation),
        image_command(
            "background",
            layer_id,
            "background",
            bounds,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            BlendMode::SrcOver,
        ),
        image_command(
            "mask",
            layer_id,
            "mask",
            bounds,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            BlendMode::DstIn,
        ),
        composite_command("end", layer_id, CompositeOperation::EndIsolation),
    ];
    let output = render_image_scene_scalar(
        &scene(
            test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            commands,
        ),
        &store,
        2,
        1,
    )
    .expect("render");
    assert_eq!(output.pixels, vec![255, 0, 0, 255, 0, 0, 0, 0]);
    assert_eq!(output.stats.isolation_count, 1);
    assert_eq!(output.stats.maximum_isolation_depth, 1);
}

#[test]
fn containing_axis_aligned_clip_is_a_noop() {
    let (_temp, store) = store(vec![(
        "texture:assets/pixel",
        1,
        1,
        vec![255, 255, 255, 255],
    )]);
    let layer_id = StableId::derive("layer", b"containing-clip");
    let bounds = Rect {
        x: 0.25,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
    let uv = Rect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
    let mut clipped = image_command("clipped", layer_id, "pixel", bounds, uv, BlendMode::SrcOver);
    clipped.clip = Some([[-0.5, 0.0], [1.5, 0.0], [1.5, 1.0], [-0.5, 1.0]]);
    let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.5, 0.0]);
    let output = render_image_scene_scalar(&scene(layer.clone(), vec![clipped]), &store, 2, 1)
        .expect("containing clip");
    let unclipped = render_image_scene_scalar(
        &scene(
            layer,
            vec![image_command(
                "unclipped",
                layer_id,
                "pixel",
                bounds,
                uv,
                BlendMode::SrcOver,
            )],
        ),
        &store,
        2,
        1,
    )
    .expect("unclipped");
    assert_eq!(output.pixels, unclipped.pixels);
    // The image spans x 0.75..1.75, which holds only the second pixel's
    // centre.
    assert_eq!(output.pixels, vec![0, 0, 0, 0, 255, 255, 255, 255]);
}

/// An alpha mask scales the source's coverage, which is what a `DstIn` draw
/// of the mask over the same bounds produces. The mask covers the left half
/// fully, the right half not at all.
#[test]
fn an_alpha_mask_scales_source_coverage_over_the_command_bounds() {
    let (_temp, store) = store(vec![
        ("texture:assets/mask", 2, 1, vec![0, 0, 0, 255, 0, 0, 0, 0]),
        ("texture:assets/white", 2, 1, vec![255; 8]),
    ]);
    let layer_id = StableId::derive("layer", b"alpha-mask");
    let bounds = Rect {
        x: 0.0,
        y: 0.0,
        width: 2.0,
        height: 1.0,
    };
    let unit = Rect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
    let mut command = image_command(
        "masked",
        layer_id,
        "white",
        bounds,
        unit,
        BlendMode::SrcOver,
    );
    let SemanticCommandPayload::Image { alpha_mask, .. } = &mut command.payload else {
        unreachable!("image command helper must create an image payload");
    };
    *alpha_mask = Some(ResourceKey {
        namespace: "assets".into(),
        key: "mask".into(),
    });
    let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    for executor in [ImageExecutor::Scalar, ImageExecutor::Simd] {
        if executor == ImageExecutor::Simd && !packet_simd_available() {
            continue;
        }
        let mut pixels = vec![0u8; 2 * 4];
        render_image_commands_into(
            &scene(layer.clone(), vec![command.clone()]),
            &store,
            2,
            1,
            executor,
            &mut pixels,
            None,
            None,
            None,
            None,
        )
        .unwrap_or_else(|error| panic!("{executor:?}: {error}"));
        assert_eq!(pixels, vec![255, 255, 255, 255, 0, 0, 0, 0], "{executor:?}");
    }
}

/// A partially covering mask scales every premultiplied channel, keeping the
/// result premultiplied rather than only dimming alpha.
#[test]
fn a_partial_alpha_mask_keeps_the_source_premultiplied() {
    let (_temp, store) = store(vec![
        ("texture:assets/half", 1, 1, vec![0, 0, 0, 128]),
        ("texture:assets/white", 1, 1, vec![255; 4]),
    ]);
    let layer_id = StableId::derive("layer", b"partial-mask");
    let unit = Rect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
    let mut command = image_command("masked", layer_id, "white", unit, unit, BlendMode::SrcOver);
    let SemanticCommandPayload::Image { alpha_mask, .. } = &mut command.payload else {
        unreachable!("image command helper must create an image payload");
    };
    *alpha_mask = Some(ResourceKey {
        namespace: "assets".into(),
        key: "half".into(),
    });
    let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let output = render_image_scene_scalar(&scene(layer, vec![command]), &store, 1, 1)
        .expect("partial mask");
    // round(255 * 128 / 255) on every channel, alpha included.
    assert_eq!(output.pixels, vec![128, 128, 128, 128]);
}

/// A shape command's blend mode is honoured rather than assumed to be
/// `SrcOver`, and the packet and scalar rasterizers agree on it.
#[test]
fn shape_blend_mode_is_honoured_and_simd_matches_scalar() {
    let layer_id = StableId::derive("layer", b"shape-blend");
    let bounds = Rect {
        x: 0.0,
        y: 0.0,
        width: 6.0,
        height: 2.0,
    };
    for blend_mode in [BlendMode::Multiply, BlendMode::Screen, BlendMode::Add] {
        let mut command = shape_command(
            "shape",
            layer_id,
            bounds,
            ShapePrimitive::Rect,
            [0.75, 0.5, 0.25, 1.0],
            None,
            [0.0; 4],
            0.0,
        );
        command.blend_mode = blend_mode;
        let commands = vec![command];
        let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let (_temp, store) = store(vec![("texture:assets/unused", 1, 1, vec![0; 4])]);
        let mut scalar = vec![64u8; 6 * 2 * 4];
        render_image_commands_into(
            &scene(layer.clone(), commands.clone()),
            &store,
            6,
            2,
            ImageExecutor::Scalar,
            &mut scalar,
            None,
            None,
            None,
            None,
        )
        .unwrap_or_else(|error| panic!("{blend_mode:?} scalar: {error}"));
        assert_ne!(scalar, vec![64u8; 6 * 2 * 4], "{blend_mode:?} drew nothing");
        if packet_simd_available() {
            let mut simd = vec![64u8; 6 * 2 * 4];
            render_image_commands_into(
                &scene(layer, commands),
                &store,
                6,
                2,
                ImageExecutor::Simd,
                &mut simd,
                None,
                None,
                None,
                None,
            )
            .unwrap_or_else(|error| panic!("{blend_mode:?} SIMD: {error}"));
            assert_eq!(simd, scalar, "{blend_mode:?}");
        }
    }
}

/// The packet blender and the scalar blender must agree bit for bit on the
/// separable modes too, over a run wide enough to cover a full packet plus a
/// masked tail.
#[test]
fn separable_blend_modes_simd_matches_scalar() {
    if !packet_simd_available() {
        eprintln!("skipping: this host has no AVX-512F+BW packet executor");
        return;
    }
    const WIDTH: u32 = 21;
    let under: Vec<u8> = (0..WIDTH)
        .flat_map(|x| {
            let v = (x * 11 % 256) as u8;
            [v, 255 - v, v / 2, (64 + x * 7 % 192) as u8]
        })
        .collect();
    let over: Vec<u8> = (0..WIDTH)
        .flat_map(|x| {
            let v = (x * 13 % 256) as u8;
            [255 - v, v, v / 3, (32 + x * 9 % 224) as u8]
        })
        .collect();
    let (_temp, store) = store(vec![
        ("texture:assets/over", WIDTH, 1, over),
        ("texture:assets/under", WIDTH, 1, under),
    ]);
    for blend_mode in [
        BlendMode::SrcOver,
        BlendMode::SrcIn,
        BlendMode::DstIn,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Add,
    ] {
        let layer_id = StableId::derive("layer", b"separable-simd");
        let bounds = Rect {
            x: 0.0,
            y: 0.0,
            width: WIDTH as f32,
            height: 1.0,
        };
        let unit = Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        let commands = vec![
            image_command("under", layer_id, "under", bounds, unit, BlendMode::SrcOver),
            image_command("over", layer_id, "over", bounds, unit, blend_mode),
        ];
        let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let scalar =
            render_image_scene_scalar(&scene(layer.clone(), commands.clone()), &store, WIDTH, 1)
                .unwrap_or_else(|error| panic!("{blend_mode:?} scalar: {error}"));
        let simd = render_image_scene_simd(&scene(layer, commands), &store, WIDTH, 1)
            .unwrap_or_else(|error| panic!("{blend_mode:?} SIMD: {error}"));
        assert_eq!(simd.pixels, scalar.pixels, "{blend_mode:?}");
    }
}

/// A group's opacity applies to the merged result, not to each command
/// inside it: two overlapping opaque commands in a half-opacity group must
/// not show through one another.
#[test]
fn group_opacity_applies_to_the_merged_group_not_each_command() {
    let (_temp, store) = store(vec![("texture:assets/white", 1, 1, vec![255; 4])]);
    let layer_id = StableId::derive("layer", b"opacity");
    let unit = Rect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
    let mut end = composite_command("end", layer_id, CompositeOperation::EndIsolation);
    if let SemanticCommandPayload::Composite { opacity, .. } = &mut end.payload {
        *opacity = 0.5;
    }
    let commands = vec![
        composite_command("begin", layer_id, CompositeOperation::BeginIsolation),
        image_command("first", layer_id, "white", unit, unit, BlendMode::SrcOver),
        image_command("second", layer_id, "white", unit, unit, BlendMode::SrcOver),
        end,
    ];
    let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let output = render_image_scene_scalar(&scene(layer, commands), &store, 1, 1)
        .expect("half-opacity group");
    // Two opaque whites inside the group still merge as one opaque white,
    // scaled once: 128, not 191 (which is what scaling each would give).
    assert_eq!(output.pixels, vec![128, 128, 128, 128]);
}

/// A group clip restricts where the merge lands. The clip covers the left
/// column only, so the right column keeps the background.
#[test]
fn group_clip_restricts_the_merge_to_its_rectangle() {
    let (_temp, store) = store(vec![("texture:assets/white", 2, 1, vec![255; 8])]);
    let layer_id = StableId::derive("layer", b"group-clip");
    let bounds = Rect {
        x: 0.0,
        y: 0.0,
        width: 2.0,
        height: 1.0,
    };
    let unit = Rect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
    let mut end = composite_command("end", layer_id, CompositeOperation::EndIsolation);
    if let SemanticCommandPayload::Composite { clip, .. } = &mut end.payload {
        *clip = Some([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
    }
    let commands = vec![
        composite_command("begin", layer_id, CompositeOperation::BeginIsolation),
        image_command("fill", layer_id, "white", bounds, unit, BlendMode::SrcOver),
        end,
    ];
    let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let output =
        render_image_scene_scalar(&scene(layer, commands), &store, 2, 1).expect("clipped group");
    assert_eq!(output.pixels, vec![255, 255, 255, 255, 0, 0, 0, 0]);
}

/// A group inside scrolled content translates as a unit: its recorded
/// extent and its merge clip follow the scroll offset together with the
/// member commands, so the merged pixels land at the scrolled position.
#[test]
fn a_scrolled_group_translates_its_extent_and_clip_with_the_content() {
    let (_temp, store) = store(vec![("texture:assets/white", 1, 1, vec![255; 4])]);
    let layer_id = StableId::derive("layer", b"scrolled-group");
    let control_id = StableId::derive("control", b"scrolled-group");
    let unit = Rect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
    let row1 = Rect {
        x: 0.0,
        y: 1.0,
        width: 1.0,
        height: 1.0,
    };
    let mut begin = composite_command("begin", layer_id, CompositeOperation::BeginIsolation);
    begin.bounds = row1;
    let mut end = composite_command("end", layer_id, CompositeOperation::EndIsolation);
    if let SemanticCommandPayload::Composite { clip, .. } = &mut end.payload {
        // The clip covers the authored row; after the scroll it must cover
        // the translated row instead.
        *clip = Some([[0.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]]);
    }
    let mut fill = image_command("fill", layer_id, "white", row1, unit, BlendMode::SrcOver);
    for command in [&mut begin, &mut fill, &mut end] {
        command
            .control_bindings
            .push(CommandControlBinding::ScrollContent { control_id });
    }
    let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let mut scene = scene(layer, vec![begin, fill, end]);
    scene.controls.push(ComponentControlSource {
        id: control_id,
        layer_id,
        role: "scroll".into(),
        state: ComponentControlState::Scroll {
            offset: 1.0,
            min: 0.0,
            max: 2.0,
            viewport_extent: 1.0,
            content_extent: 3.0,
            step: 1.0,
        },
    });

    let scalar = render_image_scene_scalar(&scene, &store, 1, 3).expect("scrolled group scalar");
    assert_eq!(
        scalar.pixels,
        vec![255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0],
        "the merged pixel must follow the scroll to row 0"
    );
    if packet_simd_available() {
        let simd = render_image_scene_simd(&scene, &store, 1, 3).expect("scrolled group simd");
        assert_eq!(simd.pixels, scalar.pixels, "simd must match scalar");
    }
}

/// Tint multiplies each premultiplied channel by its factor and the whole
/// pixel by the tint alpha, quantized per channel. The expected bytes here
/// are literal, so any drift in the formula fails loudly, and the SIMD
/// executor must agree with the scalar one byte for byte.
#[test]
fn image_tint_scales_premultiplied_channels_and_matches_across_executors() {
    // Premultiplied source pixels chosen to exercise rounding in every
    // channel: full white, a mid gray, an already-translucent pixel, and
    // a near-black one.
    let source: Vec<u8> = vec![
        255, 255, 255, 255, //
        128, 128, 128, 255, //
        100, 60, 20, 128, //
        3, 2, 1, 9,
    ];
    let tint = [0.5f32, 0.25, 1.0, 0.8];
    let expected: Vec<u8> = source
        .chunks_exact(4)
        .flat_map(|pixel| {
            let scale = |value: u8, factor: f32| {
                (f32::from(value) / 255.0 * factor * tint[3])
                    .clamp(0.0, 1.0)
                    .mul_add(255.0, 0.0)
                    .round() as u8
            };
            [
                scale(pixel[0], tint[0]),
                scale(pixel[1], tint[1]),
                scale(pixel[2], tint[2]),
                scale(pixel[3], 1.0),
            ]
        })
        .collect();

    let (_temp, store) = store(vec![("texture:assets/tinted", 4, 1, source)]);
    let layer_id = StableId::derive("layer", b"tint");
    let bounds = Rect {
        x: 0.0,
        y: 0.0,
        width: 4.0,
        height: 1.0,
    };
    let mut command = image_command(
        "tinted",
        layer_id,
        "tinted",
        bounds,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        BlendMode::SrcOver,
    );
    if let SemanticCommandPayload::Image { tint: target, .. } = &mut command.payload {
        *target = tint;
    }
    let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let scene = scene(layer, vec![command]);

    let scalar = render_image_scene_scalar(&scene, &store, 4, 1).expect("tinted scalar");
    assert_eq!(scalar.pixels, expected, "tint semantics are pinned");
    if packet_simd_available() {
        let simd = render_image_scene_simd(&scene, &store, 4, 1).expect("tinted simd");
        assert_eq!(simd.pixels, scalar.pixels, "simd must match scalar");
    }
}

/// A fully transparent group contributes nothing, and a fully opaque one is
/// identical to having declared no opacity at all.
#[test]
fn group_opacity_endpoints_are_exact() {
    let (_temp, store) = store(vec![("texture:assets/white", 1, 1, vec![255; 4])]);
    let unit = Rect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
    for (opacity, expected) in [(0.0f32, vec![0, 0, 0, 0]), (1.0, vec![255; 4])] {
        let layer_id = StableId::derive("layer", b"endpoints");
        let mut end = composite_command("end", layer_id, CompositeOperation::EndIsolation);
        if let SemanticCommandPayload::Composite {
            opacity: declared, ..
        } = &mut end.payload
        {
            *declared = opacity;
        }
        let commands = vec![
            composite_command("begin", layer_id, CompositeOperation::BeginIsolation),
            image_command("fill", layer_id, "white", unit, unit, BlendMode::SrcOver),
            end,
        ];
        let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let output = render_image_scene_scalar(&scene(layer, commands), &store, 1, 1)
            .unwrap_or_else(|error| panic!("opacity {opacity}: {error}"));
        assert_eq!(output.pixels, expected, "opacity {opacity}");
    }
}

/// Multiply, Screen and Add are the separable modes the command contract
/// declares. On premultiplied channels Multiply is
/// `s·(1 − da) + d·(1 − sa) + s·d`, Screen is `s + d − s·d` and Add is a
/// saturating sum. The semi-transparent input is where a formula written
/// for non-premultiplied colour would differ.
#[test]
fn separable_blend_modes_over_semi_transparent_pixels() {
    // The store requires strictly increasing keys.
    let (_temp, store) = store(vec![
        ("texture:assets/over", 1, 1, vec![64, 200, 128, 144]),
        ("texture:assets/under", 1, 1, vec![160, 96, 32, 200]),
    ]);
    for (blend_mode, expected) in [
        (BlendMode::Multiply, [124, 160, 58, 231]),
        (BlendMode::Screen, [184, 221, 144, 231]),
        (BlendMode::Add, [224, 255, 160, 255]),
    ] {
        let layer_id = StableId::derive("layer", b"separable");
        let unit = Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        let commands = vec![
            image_command("under", layer_id, "under", unit, unit, BlendMode::SrcOver),
            image_command("over", layer_id, "over", unit, unit, blend_mode),
        ];
        let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let scalar = render_image_scene_scalar(&scene(layer, commands), &store, 1, 1)
            .unwrap_or_else(|error| panic!("{blend_mode:?} scalar: {error}"));
        assert_eq!(scalar.pixels, expected, "{blend_mode:?}");
    }
}

/// Blending is defined on premultiplied input, so a fully transparent source
/// must leave the destination untouched for every mode except `Add`, which
/// is a saturating sum and therefore also leaves it unchanged.
#[test]
fn a_transparent_source_leaves_the_destination_unchanged() {
    for blend_mode in [
        BlendMode::SrcOver,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Add,
    ] {
        let mut destination = [17u8, 34, 51, 68];
        blend_pixel(&mut destination, [0, 0, 0, 0], blend_mode);
        assert_eq!(destination, [17, 34, 51, 68], "{blend_mode:?}");
    }
}

/// An axis-aligned clip is hard-edged: a pixel draws only when its centre
/// lies inside the clip, so a clip over 49% of the pixel leaves it
/// untouched.
#[test]
fn partial_axis_aligned_clip_keeps_only_covered_pixel_centres() {
    let (_temp, store) = store(vec![(
        "texture:assets/pixel",
        1,
        1,
        vec![255, 255, 255, 255],
    )]);
    let layer_id = StableId::derive("layer", b"clip");
    let mut command = image_command(
        "clipped",
        layer_id,
        "pixel",
        Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        BlendMode::SrcOver,
    );
    command.clip = Some([[0.0, 0.0], [0.49, 0.0], [0.49, 1.0], [0.0, 1.0]]);
    let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let output = render_image_scene_scalar(&scene(layer, vec![command]), &store, 1, 1)
        .expect("partial axis-aligned clip");
    assert_eq!(output.pixels, vec![0, 0, 0, 0]);
}

/// An ellipse image clip is hard-edged: a pixel draws in full when its
/// centre lies inside the ellipse and not at all otherwise, with no
/// fractional edge coverage.
#[test]
fn ellipse_image_clip_has_a_hard_edge() {
    let (_temp, store) = store(vec![(
        "texture:assets/pixel",
        1,
        1,
        vec![255, 255, 255, 255],
    )]);
    let layer_id = StableId::derive("layer", b"ellipse-aa-clip");
    let mut command = image_command(
        "ellipse",
        layer_id,
        "pixel",
        Rect {
            x: 0.0,
            y: 0.0,
            width: 8.0,
            height: 8.0,
        },
        Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        BlendMode::SrcOver,
    );
    let SemanticCommandPayload::Image { clip, .. } = &mut command.payload else {
        unreachable!("image command helper must create an image payload");
    };
    *clip = Some(sekai_profile_core::ImageClip::Ellipse);
    let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let output = render_image_scene_scalar(&scene(layer, vec![command]), &store, 8, 8)
        .expect("ellipse clip");

    let mask = [
        "..####..", ".######.", "########", "########", "########", "########", ".######.",
        "..####..",
    ];
    for (y, row) in mask.iter().enumerate() {
        for (x, cell) in row.bytes().enumerate() {
            let (dx, dy) = ((x as f32 + 0.5 - 4.0) / 4.0, (y as f32 + 0.5 - 4.0) / 4.0);
            assert_eq!(
                cell == b'#',
                dx * dx + dy * dy < 1.0,
                "pixel ({x}, {y}) centre"
            );
        }
    }
    assert_eq!(output.pixels, mask_pixels(&mask));
}

/// Expands rows of `#` (opaque white) and `.` (transparent) into
/// premultiplied RGBA8.
fn mask_pixels(rows: &[&str]) -> Vec<u8> {
    rows.iter()
        .flat_map(|row| row.bytes())
        .flat_map(|cell| if cell == b'#' { [255u8; 4] } else { [0; 4] })
        .collect()
}

/// On a 4x4 ellipse the four corner pixels have their centres outside the
/// ellipse and are dropped, symmetrically, by both executors.
#[test]
fn ellipse_image_clip_drops_the_corner_pixels_on_both_executors() {
    let (_temp, store) = store(vec![(
        "texture:assets/pixel",
        1,
        1,
        vec![255, 255, 255, 255],
    )]);
    let layer_id = StableId::derive("layer", b"ellipse-image-clip");
    let mut command = image_command(
        "ellipse",
        layer_id,
        "pixel",
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
        BlendMode::SrcOver,
    );
    let SemanticCommandPayload::Image { clip, .. } = &mut command.payload else {
        unreachable!("image command helper must create an image payload");
    };
    *clip = Some(sekai_profile_core::ImageClip::Ellipse);
    let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let output =
        render_image_scene_scalar(&scene(layer.clone(), vec![command.clone()]), &store, 4, 4)
            .expect("ellipse clip");
    assert_eq!(
        output.pixels,
        mask_pixels(&[".##.", "####", "####", ".##."])
    );
    if !packet_simd_available() {
        eprintln!("skipping the packet executor: this host has no AVX-512F+BW");
        return;
    }
    let simd = render_image_scene_simd(&scene(layer, vec![command]), &store, 4, 4)
        .expect("SIMD ellipse clip");
    assert_eq!(simd.pixels, output.pixels);
}

#[test]
fn rounded_image_clip_simd_matches_scalar() {
    if !packet_simd_available() {
        eprintln!("skipping: this host has no AVX-512F+BW packet executor");
        return;
    }
    let (_temp, store) = store(vec![(
        "texture:assets/pixel",
        1,
        1,
        vec![255, 255, 255, 255],
    )]);
    let layer_id = StableId::derive("layer", b"rounded-image-clip");
    let mut command = image_command(
        "rounded",
        layer_id,
        "pixel",
        Rect {
            x: 0.0,
            y: 0.0,
            width: 8.0,
            height: 6.0,
        },
        Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        BlendMode::SrcOver,
    );
    let SemanticCommandPayload::Image { clip, .. } = &mut command.payload else {
        unreachable!("image command helper must create an image payload");
    };
    *clip = Some(sekai_profile_core::ImageClip::RoundedRect { radius: [2.0, 1.5] });
    let layer = test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let scalar =
        render_image_scene_scalar(&scene(layer.clone(), vec![command.clone()]), &store, 8, 6)
            .expect("scalar rounded clip");
    let simd = render_image_scene_simd(&scene(layer, vec![command]), &store, 8, 6)
        .expect("SIMD rounded clip");
    assert_eq!(simd.pixels, scalar.pixels);
}

#[test]
fn scaled_translated_rounded_image_clip_simd_matches_scalar() {
    if !packet_simd_available() {
        eprintln!("skipping: this host has no AVX-512F+BW packet executor");
        return;
    }
    let (_temp, store) = store(vec![(
        "texture:assets/pattern",
        3,
        2,
        vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255, 0, 255, 255, 255,
            255, 0, 255, 255,
        ],
    )]);
    let layer_id = StableId::derive("layer", b"scaled-rounded-image-clip");
    let mut command = image_command(
        "scaled-rounded",
        layer_id,
        "pattern",
        Rect {
            x: 1.0,
            y: 1.0,
            width: 7.0,
            height: 5.0,
        },
        Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        BlendMode::SrcOver,
    );
    let SemanticCommandPayload::Image { clip, .. } = &mut command.payload else {
        unreachable!("image command helper must create an image payload");
    };
    *clip = Some(sekai_profile_core::ImageClip::RoundedRect { radius: [1.5, 1.0] });
    let layer = test_layer(layer_id, [1.75, 0.0, 0.0, 1.4, 0.75, 0.5]);
    let scalar =
        render_image_scene_scalar(&scene(layer.clone(), vec![command.clone()]), &store, 19, 11)
            .expect("scalar scaled rounded clip");
    let simd = render_image_scene_simd(&scene(layer, vec![command]), &store, 19, 11)
        .expect("SIMD scaled rounded clip");
    assert_eq!(simd.pixels, scalar.pixels);
}

#[test]
fn unsupported_non_axis_aligned_clip_fails_closed_without_pixels() {
    let (_temp, store) = store(vec![(
        "texture:assets/pixel",
        1,
        1,
        vec![255, 255, 255, 255],
    )]);
    let layer_id = StableId::derive("layer", b"non-axis-clip");
    let mut command = image_command(
        "clipped",
        layer_id,
        "pixel",
        Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        BlendMode::SrcOver,
    );
    command.clip = Some([[0.0, 0.0], [1.0, 0.0], [0.75, 1.0], [0.0, 1.0]]);
    let error = render_image_scene_scalar(
        &scene(
            test_layer(layer_id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            vec![command],
        ),
        &store,
        1,
        1,
    )
    .expect_err("clip must fail closed");
    assert!(matches!(
        error,
        ProfileCompositorError::UnsupportedFeature { .. }
    ));
}
