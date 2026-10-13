//! Executor, cache-variant, clip and lit-badge coverage over the scene entry
//! points.

use std::collections::BTreeMap;
use std::sync::Arc;

use sekai_profile_core::profile_scene::ResolvedProfileScene;
use sekai_profile_core::{
    AuthoredElementKind, FontRole, ImageMaterial, LayerKind, LayerSource, ParameterValue, Rect,
    ResourceKey, SemanticCommandPayload, SemanticCommandSource, ShapePrimitive, StableId,
};
use sha2::{Digest, Sha256};

use super::*;
use crate::masterdata::{MasterDataProvider, ResolvedColor, ResolvedHonor, ResourceInfo};
use crate::render_object::{
    MappedRenderObjectStore, RenderObjectKind, RenderObjectStoreWriter, RenderObjectWrite,
};
use crate::sdf::atlas::{
    MappedSdfAtlas, MappedSdfAtlasSet, SdfAtlasGenerationReport, SdfAtlasManifest,
    SdfAtlasPageManifest,
};
use crate::types::{BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry};

/// A family no font directory provides, so text capture resolves the
/// installed atlas but lays out no glyphs.
const TEXT_FAMILY: &str = "compositor-test-face";

struct FontOnlyProvider;

impl MasterDataProvider for FontOnlyProvider {
    fn resolve_story_banner(&self, _story_type: &str, _story_id: i32) -> Option<String> {
        None
    }
    fn get_card(&self, _card_id: i32) -> Option<CardEntry> {
        None
    }
    fn resolve_color(&self, _color_id: i32) -> Option<ResolvedColor> {
        None
    }
    fn resolve_font(&self, _font_id: i32) -> Option<String> {
        Some(TEXT_FAMILY.into())
    }
    fn resolve_stamp(&self, _stamp_id: i32) -> Option<String> {
        None
    }
    fn resolve_resource(&self, _res_type: &str, _id: i32) -> Option<ResourceInfo> {
        None
    }
    fn resolve_honor(&self, _honor_id: i32, _honor_level: i32) -> Option<ResolvedHonor> {
        None
    }
    fn get_bonds_honor(&self, _id: i32) -> Option<BondsHonorEntry> {
        None
    }
    fn get_bonds_honor_word(&self, _word_id: i64) -> Option<BondsHonorWordEntry> {
        None
    }
    fn get_honor(&self, _honor_id: i32) -> Option<HonorEntry> {
        None
    }
    fn resolve_unit_vs_sd(&self, self_id: i32, _partner_id: i32) -> i32 {
        self_id
    }
    fn font_count(&self) -> usize {
        1
    }
    fn color_count(&self) -> usize {
        0
    }
}

fn packet_simd_available() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
            && std::arch::is_x86_feature_detected!("fma")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

fn layer(id: StableId, authored_kind: AuthoredElementKind, general_type: i64) -> LayerSource {
    LayerSource {
        id,
        parent_id: None,
        kind: LayerKind::Image,
        authored_kind,
        authored_index: 0,
        game_layer: 0,
        z: 0,
        authored_visible: true,
        source_content: String::new(),
        resolved_parameters: BTreeMap::from([(
            "general_type".to_string(),
            ParameterValue::I64(general_type),
        )]),
        bounds: Rect::default(),
        quad: [[0.0; 2]; 4],
        matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        hit_geometry: [[0.0; 2]; 4],
        line_indent: None,
    }
}

fn scene(layer: LayerSource, commands: Vec<SemanticCommandSource>) -> ResolvedProfileScene {
    ResolvedProfileScene {
        layers: vec![layer],
        commands,
        interaction_regions: Vec::new(),
        controls: Vec::new(),
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn full_uv() -> Rect {
    rect(0.0, 0.0, 1.0, 1.0)
}

fn filled_shape(
    id: u64,
    layer_id: StableId,
    bounds: Rect,
    fill: [f32; 4],
) -> SemanticCommandSource {
    let mut command = SemanticCommandSource::shape(
        StableId(id),
        layer_id,
        "shape",
        bounds,
        ShapePrimitive::Rect,
    );
    if let SemanticCommandPayload::Shape { fill: target, .. } = &mut command.payload {
        *target = fill;
    }
    command
}

fn assets_image(
    id: u64,
    layer_id: StableId,
    role: &str,
    key: &str,
    bounds: Rect,
    uv: Rect,
) -> SemanticCommandSource {
    let mut command = SemanticCommandSource::image(
        StableId(id),
        layer_id,
        role,
        ResourceKey {
            namespace: "assets".into(),
            key: key.into(),
        },
        bounds,
    );
    if let SemanticCommandPayload::Image { uv: target, .. } = &mut command.payload {
        *target = uv;
    }
    command
}

struct StoredObject {
    key: String,
    kind: RenderObjectKind,
    source_sha256: String,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

fn texture(key: &str, width: u32, height: u32, pixels: Vec<u8>) -> StoredObject {
    StoredObject {
        key: key.into(),
        kind: RenderObjectKind::Texture,
        source_sha256: hex::encode(Sha256::digest(key.as_bytes())),
        width,
        height,
        pixels,
    }
}

fn store(mut objects: Vec<StoredObject>) -> (tempfile::TempDir, MappedRenderObjectStore) {
    objects.sort_by(|left, right| left.key.cmp(&right.key));
    let temp = tempfile::tempdir().expect("tempdir");
    let output = temp.path().join("store");
    let mut writer =
        RenderObjectStoreWriter::create(&output, "profile-compositor-software-test", 1 << 20)
            .expect("writer");
    for object in &objects {
        writer
            .add(RenderObjectWrite {
                key: &object.key,
                kind: object.kind,
                source_sha256: &object.source_sha256,
                width: object.width,
                height: object.height,
                row_bytes: object.width * 4,
                pixels: &object.pixels,
            })
            .expect("add object");
    }
    let manifest = writer.finish().expect("manifest");
    (
        temp,
        MappedRenderObjectStore::open(manifest).expect("mapped store"),
    )
}

fn text_atlases(root: &std::path::Path) -> MappedSdfAtlasSet {
    let mut page = vec![0u8; crate::sdf::atlas::SWIZZLED_PAGE_HEADER_BYTES + 8 * 8];
    page[..crate::sdf::atlas::SWIZZLED_PAGE_MAGIC.len()]
        .copy_from_slice(crate::sdf::atlas::SWIZZLED_PAGE_MAGIC);
    page[12..16].copy_from_slice(&crate::sdf::atlas::SWIZZLED_PAGE_VERSION.to_le_bytes());
    for offset in [16, 20, 24, 28] {
        page[offset..offset + 4].copy_from_slice(&8u32.to_le_bytes());
    }
    std::fs::write(root.join("page-000.r8swz"), &page).expect("write atlas page");
    let manifest = SdfAtlasManifest {
        schema: crate::sdf::atlas::ATLAS_MANIFEST_SCHEMA.into(),
        generator_contract: "compositor-software-test".into(),
        font_family: TEXT_FAMILY.into(),
        font_sha256: "00".repeat(32),
        point_size: 8.0,
        spread: 1.0,
        pages: vec![SdfAtlasPageManifest {
            file: "page-000.r8swz".into(),
            width: 8,
            height: 8,
            file_sha256: hex::encode(Sha256::digest(&page)),
        }],
        glyphs: Vec::new(),
        generation: SdfAtlasGenerationReport {
            cmap_codepoint_count: 0,
            requested_codepoint_count: 0,
            generated_glyph_count: 0,
            failed_glyph_count: 0,
            analytic_fallback_count: 0,
            page_width: 8,
            page_height: 8,
            gutter: 0,
            failures: Vec::new(),
            analytic_fallback_codepoints: Vec::new(),
        },
    };
    let manifest_path = root.join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec(&manifest).expect("manifest json"),
    )
    .expect("write atlas manifest");
    let mut atlases = MappedSdfAtlasSet::new();
    atlases
        .insert(Arc::new(
            MappedSdfAtlas::open(manifest_path).expect("open atlas"),
        ))
        .expect("install atlas");
    atlases
}

#[test]
fn scalar_entry_composites_a_general_base_hit_without_the_packet_executor() {
    let atlas_root = tempfile::tempdir().expect("atlas tempdir");
    let atlases = text_atlases(atlas_root.path());
    let md = MasterData::new(Arc::new(FontOnlyProvider));
    let layer_id = StableId(100);
    let mut overlay = SemanticCommandSource::profile_text(
        StableId(102),
        layer_id,
        "general-word",
        "userProfile.word",
        "",
        FontRole::RegionFontId(1),
    );
    overlay.bounds = rect(2.0, 2.0, 8.0, 6.0);
    if let SemanticCommandPayload::Text { size, .. } = &mut overlay.payload {
        *size = 20.0;
    }
    let scene = scene(
        layer(layer_id, AuthoredElementKind::General, 11),
        vec![
            filled_shape(
                101,
                layer_id,
                rect(2.0, 2.0, 8.0, 6.0),
                [1.0, 0.0, 0.0, 1.0],
            ),
            overlay,
        ],
    );
    let (_seed_temp, seed_store) = store(vec![texture("texture:assets/seed", 1, 1, vec![0; 4])]);
    let semantic = SemanticSdfContext {
        text_atlases: &atlases,
        md: &md,
    };
    let plan = general_base_plan(&scene, &seed_store, semantic, 0)
        .expect("plan")
        .expect("type 11 with a profile field is eligible");
    let tile = &plan.tiles[0];
    assert_eq!(tile.bounds, rect(2.0, 2.0, 8.0, 6.0));
    let (_temp, store) = store(vec![
        texture("texture:assets/seed", 1, 1, vec![0; 4]),
        StoredObject {
            key: tile.object_key.clone(),
            kind: RenderObjectKind::Component,
            source_sha256: tile.source_sha256.clone(),
            width: 8,
            height: 6,
            pixels: [0, 0, 255, 255].repeat(8 * 6),
        },
    ]);

    let mut pixels = vec![0; 16 * 12 * 4];
    let stats = render_authored_profile_into_scalar(
        &scene,
        &store,
        Some(&atlases),
        &md,
        None,
        AuthoredElementKind::General,
        0,
        &mut pixels,
        16,
        12,
    )
    .expect("scalar render");

    assert_eq!(stats.general_base_hit_count, 1);
    assert_eq!(stats.simd_packet_count, 0);
    assert_eq!(stats.scalar_fragment_count, 8 * 6);
    for y in 0..12 {
        for x in 0..16 {
            let offset = (y * 16 + x) * 4;
            let expected = if (2..10).contains(&x) && (2..8).contains(&y) {
                [0, 0, 255, 255]
            } else {
                [0; 4]
            };
            assert_eq!(pixels[offset..offset + 4], expected, "pixel ({x}, {y})");
        }
    }
}

#[test]
fn deck_art_variant_hit_matches_the_source_draw() {
    if !packet_simd_available() {
        eprintln!("skipping: the deck art variant builder needs the packet executor");
        return;
    }
    const SOURCE_WIDTH: u32 = 600;
    const SOURCE_HEIGHT: u32 = 576;
    let source_key = "texture:assets/character/member_cutout/test/normal";
    let source_pixels = (0..SOURCE_HEIGHT)
        .flat_map(|y| {
            (0..SOURCE_WIDTH)
                .flat_map(move |x| [x as u8, y as u8, ((x >> 8) | ((y >> 8) << 4)) as u8, 255])
        })
        .collect::<Vec<_>>();
    let (_source_temp, source_store) = store(vec![texture(
        source_key,
        SOURCE_WIDTH,
        SOURCE_HEIGHT,
        source_pixels.clone(),
    )]);
    let built =
        build_deck_art_variant_simd(source_store.object(source_key).expect("mapped source"))
            .expect("build variant");
    let (_hit_temp, hit_store) = store(vec![
        texture(source_key, SOURCE_WIDTH, SOURCE_HEIGHT, source_pixels),
        StoredObject {
            key: built.object_key.clone(),
            kind: RenderObjectKind::Component,
            source_sha256: built.source_sha256.clone(),
            width: built.width,
            height: built.height,
            pixels: built.pixels.clone(),
        },
    ]);

    let layer_id = StableId(200);
    let scene = scene(
        layer(layer_id, AuthoredElementKind::General, 3),
        vec![assets_image(
            201,
            layer_id,
            "deck-slot-0-artwork",
            "character/member_cutout/test/normal",
            rect(10.0, 5.0, 148.078_13, 243.0),
            deck_art_source_uv(SOURCE_WIDTH, SOURCE_HEIGHT),
        )],
    );
    for executor in [ImageExecutor::Scalar, ImageExecutor::Simd] {
        let render = |store: &MappedRenderObjectStore| {
            let mut pixels = vec![0; 170 * 260 * 4];
            let stats = render_image_commands_into(
                &scene,
                store,
                170,
                260,
                executor,
                &mut pixels,
                None,
                None,
                None,
                None,
            )
            .expect("render deck art");
            (pixels, stats)
        };
        let (miss, miss_stats) = render(&source_store);
        let (hit, hit_stats) = render(&hit_store);
        assert_eq!(miss_stats.deck_art_variant_miss_count, 1);
        assert_eq!(hit_stats.deck_art_variant_hit_count, 1);
        let differing = (0..miss.len() / 4)
            .filter(|pixel| miss[pixel * 4..pixel * 4 + 4] != hit[pixel * 4..pixel * 4 + 4])
            .count();
        assert_eq!(
            differing, 0,
            "{executor:?}: variant hit differs from the source draw"
        );
    }
}

#[test]
fn simd_shape_quantisation_matches_scalar_just_below_a_half_step() {
    if !packet_simd_available() {
        eprintln!("skipping: packet executor unavailable");
        return;
    }
    // 0.0019607842 * 255 rounds to 0.49999997 in f32.
    let layer_id = StableId(300);
    let scene = scene(
        layer(layer_id, AuthoredElementKind::Other, 0),
        vec![filled_shape(
            301,
            layer_id,
            rect(0.0, 0.0, 4.0, 4.0),
            [1.0, 1.0, 1.0, 0.001_960_784_2],
        )],
    );
    let (_temp, store) = store(vec![texture("texture:assets/seed", 1, 1, vec![0; 4])]);
    let render = |executor| {
        let mut pixels = vec![0; 4 * 4 * 4];
        render_image_commands_into(
            &scene,
            &store,
            4,
            4,
            executor,
            &mut pixels,
            None,
            None,
            None,
            None,
        )
        .expect("render shape");
        pixels
    };
    let scalar = render(ImageExecutor::Scalar);
    assert!(scalar.iter().all(|&value| value == 0));
    assert_eq!(render(ImageExecutor::Simd), scalar);
}

#[test]
fn a_clip_beside_the_image_draws_nothing_on_either_executor() {
    let (_temp, store) = store(vec![texture(
        "texture:assets/pixel",
        1,
        1,
        vec![255, 255, 255, 255],
    )]);
    let layer_id = StableId(400);
    let mut command = assets_image(
        401,
        layer_id,
        "clipped-away",
        "pixel",
        rect(0.0, 0.0, 4.0, 4.0),
        full_uv(),
    );
    command.clip = Some([[8.0, 0.0], [12.0, 0.0], [12.0, 4.0], [8.0, 4.0]]);
    let scene = scene(
        layer(layer_id, AuthoredElementKind::Other, 0),
        vec![command],
    );
    let mut executors = vec![ImageExecutor::Scalar];
    if packet_simd_available() {
        executors.push(ImageExecutor::Simd);
    }
    for executor in executors {
        let mut pixels = vec![0; 16 * 4 * 4];
        let stats = render_image_commands_into(
            &scene,
            &store,
            16,
            4,
            executor,
            &mut pixels,
            None,
            None,
            None,
            None,
        )
        .expect("render clipped image");
        assert_eq!(stats.sampled_fragment_count, 0, "{executor:?}");
        assert!(pixels.iter().all(|&value| value == 0), "{executor:?}");
    }
}

const BADGE_ALBEDO: &str = "texture:assets/badge";
const BADGE_NORMAL_MAP: &str = "texture:static/ui/sekai_badge_normal";
const RED: [u8; 4] = [200, 40, 90, 255];
const TILTED: [u8; 4] = [200, 90, 220, 255];
const FLAT: [u8; 4] = [128, 128, 255, 255];

fn badge_image(
    id: u64,
    layer_id: StableId,
    bounds: Rect,
    element_alpha: f32,
) -> SemanticCommandSource {
    let mut command = assets_image(id, layer_id, "collection", "badge", bounds, full_uv());
    if let SemanticCommandPayload::Image { tint, material, .. } = &mut command.payload {
        tint[3] = element_alpha;
        *material = ImageMaterial::LitBadge {
            normal_map: ResourceKey {
                namespace: "static".into(),
                key: "ui/sekai_badge_normal".into(),
            },
        };
    }
    command
}

fn pixels_of(texels: &[[u8; 4]]) -> Vec<u8> {
    texels.concat()
}

/// Renders `scene` with the scalar executor and, where available, the
/// packet executor, and returns the pixels both agree on.
fn render_both(
    scene: &ResolvedProfileScene,
    store: &MappedRenderObjectStore,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let scalar = render_image_scene_scalar(scene, store, width, height).expect("scalar badge");
    if packet_simd_available() {
        let simd = render_image_scene_simd(scene, store, width, height).expect("simd badge");
        assert_eq!(simd.pixels, scalar.pixels, "executors agree");
    }
    scalar.pixels
}

// Expected pixels follow a double-precision evaluation of the badge
// material over the same texels, rounded to 8 bits.

#[test]
fn lit_badges_shade_each_pixel_from_its_image_texel_and_the_bilinear_normal_map() {
    let layer_id = StableId::derive("layer", b"badge");
    let (_temp, store) = store(vec![
        texture(
            BADGE_ALBEDO,
            4,
            1,
            pixels_of(&[
                [50, 75, 113, 255],
                RED,
                [10, 20, 30, 255],
                [120, 200, 60, 255],
            ]),
        ),
        texture(BADGE_NORMAL_MAP, 2, 1, pixels_of(&[FLAT, TILTED])),
    ]);
    let scene = scene(
        layer(layer_id, AuthoredElementKind::Collection, 0),
        vec![badge_image(1, layer_id, rect(0.0, 0.0, 4.0, 1.0), 1.0)],
    );
    // The normal map is half as wide as the image: the outer pixels clamp
    // to its edge texels and the inner ones blend a quarter and three
    // quarters of the way between them.
    assert_eq!(
        render_both(&scene, &store, 4, 1),
        pixels_of(&[
            [88, 112, 149, 255],
            [209, 56, 104, 255],
            [35, 44, 54, 255],
            [126, 203, 68, 255],
        ])
    );
}

#[test]
fn the_badge_tangent_frame_follows_the_element_rotation_and_flip() {
    let layer_id = StableId::derive("layer", b"turned-badge");
    let (_temp, store) = store(vec![
        texture(BADGE_ALBEDO, 2, 2, pixels_of(&[RED; 4])),
        texture(BADGE_NORMAL_MAP, 2, 2, pixels_of(&[TILTED; 4])),
    ]);
    let (sin, cos) = 30f32.to_radians().sin_cos();
    for (name, matrix, expected) in [
        (
            "upright",
            [1.0, 0.0, 0.0, 1.0, 8.0, 8.0],
            [203, 49, 97, 255],
        ),
        // Turned 30 degrees counter-clockwise on screen.
        (
            "turned",
            [cos, -sin, sin, cos, 8.0, 8.0],
            [196, 42, 90, 255],
        ),
        (
            "mirrored",
            [-1.0, 0.0, 0.0, 1.0, 8.0, 8.0],
            [194, 40, 88, 255],
        ),
    ] {
        let mut badge_layer = layer(layer_id, AuthoredElementKind::Collection, 0);
        badge_layer.matrix = matrix;
        let scene = scene(
            badge_layer,
            vec![badge_image(1, layer_id, rect(-4.0, -4.0, 8.0, 8.0), 1.0)],
        );
        let pixels = render_both(&scene, &store, 16, 16);
        let centre = (8 * 16 + 8) * 4;
        assert_eq!(pixels[centre..centre + 4], expected, "{name}");
    }
}

#[test]
fn badge_alpha_is_cut_at_one_half_and_scaled_by_the_element_alpha() {
    let layer_id = StableId::derive("layer", b"badge-alpha");
    // Magenta at alpha 127 and 128, stored premultiplied.
    let (_cut_temp, cut_store) = store(vec![
        texture(
            BADGE_ALBEDO,
            2,
            1,
            pixels_of(&[[127, 0, 127, 127], [128, 0, 128, 128]]),
        ),
        texture(BADGE_NORMAL_MAP, 1, 1, pixels_of(&[FLAT])),
    ]);
    let scene_for = |element_alpha| {
        scene(
            layer(layer_id, AuthoredElementKind::Collection, 0),
            vec![badge_image(
                1,
                layer_id,
                rect(0.0, 0.0, 2.0, 1.0),
                element_alpha,
            )],
        )
    };
    assert_eq!(
        render_both(&scene_for(1.0), &cut_store, 2, 1),
        pixels_of(&[[0; 4], [255, 40, 255, 255]])
    );

    let (_half_temp, half_store) = store(vec![
        texture(BADGE_ALBEDO, 1, 1, pixels_of(&[RED])),
        texture(BADGE_NORMAL_MAP, 1, 1, pixels_of(&[TILTED])),
    ]);
    let half = scene(
        layer(layer_id, AuthoredElementKind::Collection, 0),
        vec![badge_image(1, layer_id, rect(0.0, 0.0, 1.0, 1.0), 0.5)],
    );
    assert_eq!(render_both(&half, &half_store, 1, 1), [101, 24, 48, 128]);
}

#[test]
fn a_badge_without_its_normal_map_fails_like_any_missing_resource() {
    let layer_id = StableId::derive("layer", b"badge-missing");
    let (_temp, store) = store(vec![texture(BADGE_ALBEDO, 1, 1, pixels_of(&[RED]))]);
    let scene = scene(
        layer(layer_id, AuthoredElementKind::Collection, 0),
        vec![badge_image(1, layer_id, rect(0.0, 0.0, 1.0, 1.0), 1.0)],
    );
    match render_image_scene_scalar(&scene, &store, 1, 1) {
        Err(ProfileCompositorError::MissingObject(key)) => assert_eq!(key, BADGE_NORMAL_MAP),
        other => panic!("a missing normal map must fail the command: {other:?}"),
    }
}
