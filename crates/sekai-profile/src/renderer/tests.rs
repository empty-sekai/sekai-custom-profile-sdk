#[cfg(all(feature = "animation-export", target_arch = "x86_64"))]
use super::animation_layer::find_opaque_bounds_avx512;
#[cfg(feature = "animation-export")]
use super::animation_layer::{
    animation_sdf_retained_surface_bytes, crop_pixels_lossless, find_opaque_bounds,
    find_opaque_bounds_scalar, LayerSurface,
};
use super::authored::resolve_native_profile_scene;
use super::capture::{
    align_substituted_text_sdf_command, captured_text_sdf_glyph_is_invisible,
    sdf_command_device_magnification, TextSdfSamplingGrid,
};
use super::fallback::extend_profile_fallback_codepoints;
use super::realtime_edt::{realtime_edt_request_key, realtime_edt_sampling_spread};
use super::*;
use crate::masterdata::{
    CollectionResourceType, MasterDataProvider, OmikujiRow, ResolvedColor, ResolvedHonor,
    ResourceInfo,
};
use crate::output::RenderedPage;
#[cfg(feature = "animation-export")]
use crate::render_config::Executor;
use crate::types::{BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry};

#[test]
fn fallback_requests_follow_the_text_mesh_pro_glyph_chain() {
    let request = |text: &str, primary_has: &dyn Fn(u32) -> bool| {
        let mut requested = std::collections::BTreeSet::new();
        extend_profile_fallback_codepoints(&mut requested, text, primary_has);
        requested
            .into_iter()
            .filter_map(char::from_u32)
            .collect::<String>()
    };
    let ascii = |codepoint: u32| (0x21..=0x7e).contains(&codepoint);
    let ascii_and_square = |codepoint: u32| ascii(codepoint) || codepoint == 0x25a1;

    // Hyphen stand-ins come from the primary face; characters that draw
    // nothing need no glyph at all.
    assert_eq!(
        request(
            "A\u{2011}B\u{00AD}C\u{200B}D\u{00A0}E\u{3000}F",
            &ascii_and_square
        ),
        ""
    );
    // Markup is not drawn, and case tags change the character drawn.
    assert_eq!(
        request(
            "<color=#ff0000>\u{6F22}</color><lowercase>\u{00C4}</lowercase>",
            &|codepoint| ascii_and_square(codepoint) || codepoint == 0xe4
        ),
        "\u{6F22}"
    );
    // A primary face without the stand-in or the missing-glyph square
    // takes both from the fallback font.
    assert_eq!(
        request("A\u{2011}\u{6F22}", &|codepoint| ascii(codepoint)
            && codepoint != u32::from('-')),
        "-\u{2011}\u{25A1}\u{6F22}"
    );
}

fn magnification_command(
    atlas_width: u32,
    atlas_height: u32,
    quad: [crate::sdf::tile::Point2; 4],
) -> crate::sdf::tile::SdfDrawCommand {
    crate::sdf::tile::SdfDrawCommand {
        kind: crate::sdf::tile::SdfPrimitiveKind::Text,
        atlas_set: 0,
        atlas_page: 0,
        atlas_rect: [0, 0, atlas_width, atlas_height],
        quad,
        device_clip: None,
        material: crate::sdf::tile::SdfCommandMaterial::Text(
            crate::sdf::tile::SdfMaterial::default(),
        ),
    }
}

#[test]
fn realtime_edt_threshold_uses_final_command_geometry() {
    use crate::sdf::tile::Point2;

    let exactly_three = magnification_command(
        10,
        20,
        [
            Point2::new(0.0, 0.0),
            Point2::new(30.0, 0.0),
            Point2::new(30.0, 60.0),
            Point2::new(0.0, 60.0),
        ],
    );
    let above_three = magnification_command(
        10,
        20,
        [
            Point2::new(0.0, 0.0),
            Point2::new(30.1, 0.0),
            Point2::new(30.1, 60.0),
            Point2::new(0.0, 60.0),
        ],
    );
    assert_eq!(sdf_command_device_magnification(&exactly_three), Some(3.0));
    assert!(!(sdf_command_device_magnification(&exactly_three).unwrap() > 3.0));
    assert!(sdf_command_device_magnification(&above_three).unwrap() > 3.0);
}

#[test]
fn final_command_magnification_handles_rotation_and_non_uniform_scale() {
    use crate::sdf::tile::Point2;

    let command = magnification_command(
        10,
        20,
        [
            Point2::new(5.0, 7.0),
            Point2::new(5.0, 37.0),
            Point2::new(-75.0, 37.0),
            Point2::new(-75.0, 7.0),
        ],
    );
    assert_eq!(sdf_command_device_magnification(&command), Some(4.0));
}

#[test]
fn realtime_edt_spread_preserves_the_atlas_logical_distance_range() {
    assert_eq!(realtime_edt_sampling_spread(75.0, 6.0, 75.0), Some(6.0));
    assert_eq!(realtime_edt_sampling_spread(75.0, 6.0, 300.0), Some(24.0));
    assert_eq!(
        realtime_edt_sampling_spread(75.0, 6.0, 4096.0),
        Some(327.68)
    );
    assert_eq!(realtime_edt_sampling_spread(0.0, 6.0, 300.0), None);
    assert_eq!(realtime_edt_sampling_spread(75.0, 0.0, 300.0), None);
}

#[test]
fn non_positive_resolved_text_size_is_an_invisible_sdf_operation() {
    fn glyph(font_size: f32) -> crate::text::ResolvedTextSdfGlyph {
        crate::text::ResolvedTextSdfGlyph {
            text: "(".into(),
            font_family: Some("test-family".into()),
            baseline_origin: crate::sdf::tile::Point2::new(0.0, 0.0),
            font_size,
            local_to_device: crate::sdf::tile::Affine2::IDENTITY,
            material: crate::sdf::tile::SdfMaterial::default(),
        }
    }

    assert!(captured_text_sdf_glyph_is_invisible(&glyph(0.0)));
    assert!(captured_text_sdf_glyph_is_invisible(&glyph(-147.636_35)));
    assert!(!captured_text_sdf_glyph_is_invisible(&glyph(0.001)));
    assert!(!captured_text_sdf_glyph_is_invisible(&glyph(f32::NAN)));
}

#[test]
fn realtime_edt_request_identity_deduplicates_only_exact_sampling_requests() {
    use std::collections::BTreeSet;

    let duplicate_a = realtime_edt_request_key("test-family", '(', 300.0, 24.0);
    let duplicate_b = realtime_edt_request_key("test-family", '(', 300.0, 24.0);
    let different_size = realtime_edt_request_key("test-family", '(', 300.001, 24.0);
    let different_spread = realtime_edt_request_key("test-family", '(', 300.0, 24.001);
    let different_font = realtime_edt_request_key("other-test-family", '(', 300.0, 24.0);

    let unique = [
        duplicate_a,
        duplicate_b,
        different_size,
        different_spread,
        different_font,
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    assert_eq!(unique.len(), 4);
}

#[test]
fn substituted_text_tier_preserves_primary_sample_grid() {
    use crate::sdf::atlas::SdfAtlasGlyphManifest;
    use crate::sdf::tile::{Point2, SdfCommandMaterial, SdfDrawCommand, SdfMaterial};

    let primary_glyph = SdfAtlasGlyphManifest {
        codepoint: 0x25a0,
        page: 0,
        rect: [0, 0, 73, 73],
        plane_bearing: [7.5, 55.203_125],
        plane_size: [60.0, 60.0],
        plane_advance_x: 75.0,
    };
    let target_glyph = SdfAtlasGlyphManifest {
        codepoint: 0x25a0,
        page: 1,
        rect: [0, 0, 217, 217],
        plane_bearing: [22.5, 165.593_75],
        plane_size: [180.0, 180.0],
        plane_advance_x: 225.0,
    };
    let command = |atlas_set, atlas_page, atlas_rect| SdfDrawCommand {
        kind: crate::sdf::tile::SdfPrimitiveKind::Text,
        atlas_set,
        atlas_page,
        atlas_rect,
        quad: [
            Point2::new(0.0, 0.0),
            Point2::new(144_000.0, 0.0),
            Point2::new(144_000.0, 2_400.0),
            Point2::new(0.0, 2_400.0),
        ],
        device_clip: None,
        material: SdfCommandMaterial::Text(SdfMaterial::default()),
    };
    let primary_command = command(0, 0, primary_glyph.rect);
    let target_command = command(1, 1, target_glyph.rect);
    let primary_grid = TextSdfSamplingGrid::new(primary_command, &primary_glyph, 75.0, 6.0)
        .expect("valid primary grid");
    let aligned = align_substituted_text_sdf_command(
        primary_grid,
        target_command,
        &target_glyph,
        225.0,
        18.0,
    )
    .expect("valid substituted grid");

    let close = |actual: f32, expected: f32| {
        assert!((actual - expected).abs() < 0.01, "{actual} != {expected}");
    };
    close(aligned.quad[0].x, 144_000.0 / 219.0);
    close(aligned.quad[1].x, 144_000.0 * 218.0 / 219.0);
    close(aligned.quad[0].y, 2_400.0 * 2.0 / 219.0);
    close(aligned.quad[3].y, 2_400.0);
    assert_eq!(aligned.atlas_set, 1);
    assert_eq!(aligned.atlas_page, 1);
    assert_eq!(aligned.atlas_rect, target_glyph.rect);
}

#[cfg(feature = "animation-export")]
#[test]
fn lossless_crop_rejects_short_row_stride() {
    let surface = LayerSurface {
        pixels: &[0; 16],
        row_bytes: 12,
        width: 4,
        height: 1,
    };
    let result = crop_pixels_lossless(surface, 0, 0, 1, 1);
    assert!(matches!(result, Err(_)));
}

#[cfg(feature = "animation-export")]
#[test]
fn lossless_crop_rejects_out_of_bounds_rect() {
    let surface = LayerSurface {
        pixels: &[0; 16],
        row_bytes: 16,
        width: 4,
        height: 1,
    };
    let result = crop_pixels_lossless(surface, 4, 0, 1, 1);
    assert!(matches!(result, Err(_)));
}

#[cfg(feature = "animation-export")]
#[test]
fn lossless_crop_rejects_zero_sized_rect() {
    let surface = LayerSurface {
        pixels: &[0; 16],
        row_bytes: 16,
        width: 4,
        height: 1,
    };
    let result = crop_pixels_lossless(surface, 0, 0, 0, 1);
    assert!(matches!(result, Err(_)));
}

#[cfg(feature = "animation-export")]
#[test]
fn direct_opaque_bounds_match_scalar_and_vector_paths() {
    let surface_width = 4u32;
    let surface_height = 3u32;
    let row_bytes = 20usize;
    let mut source = vec![0u8; row_bytes * surface_height as usize];
    let samples = [
        (1u32, 1u32, [10u8, 20, 30, 40]),
        (2, 1, [50, 60, 70, 80]),
        (1, 2, [90, 100, 110, 120]),
        (2, 2, [130, 140, 150, 160]),
    ];
    for (x, y, rgba) in samples {
        let offset = y as usize * row_bytes + x as usize * 4;
        source[offset..offset + 4].copy_from_slice(&rgba);
    }
    assert_eq!(
        find_opaque_bounds(&source, surface_width, surface_height, row_bytes),
        (1, 1, 2, 2)
    );
    assert_eq!(
        find_opaque_bounds_scalar(&source, surface_width, surface_height, row_bytes),
        (1, 1, 2, 2)
    );
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
        && std::arch::is_x86_feature_detected!("bmi2")
    {
        assert_eq!(
            // SAFETY: the AVX-512F/BW/BMI2 detection above satisfies the
            // kernel's target-feature contract, and `source` is
            // `row_bytes * surface_height` bytes with
            // `row_bytes >= surface_width * 4`.
            unsafe { find_opaque_bounds_avx512(&source, surface_width, surface_height, row_bytes) },
            (1, 1, 2, 2)
        );
    }
}

#[test]
fn cache_identity_is_stable_and_config_sensitive() {
    use std::sync::Arc;

    let renderer = CustomProfileRenderer::new(Arc::new(NullProvider));
    let default = RenderConfig::default();
    let first = renderer.cache_identity(&default).expect("default identity");
    let repeated = renderer
        .cache_identity(&default)
        .expect("repeated identity");
    let changed = renderer
        .cache_identity(&RenderConfig {
            tile_width: 64,
            ..default
        })
        .expect("changed identity");

    assert_eq!(first, repeated);
    assert_ne!(first, changed);
    assert!(first.starts_with("profile-render-"));
}

#[cfg(feature = "animation-export")]
#[test]
fn oversized_animation_sdf_surface_is_detected_before_plan_or_surface_build() {
    let (surface_bytes, retained_peak_bytes) =
        animation_sdf_retained_surface_bytes(46_290, 29_984).unwrap();
    assert_eq!(surface_bytes, 5_551_837_440);
    assert_eq!(retained_peak_bytes, 11_103_674_880);
    assert!(retained_peak_bytes > 64 * 1024 * 1024);

    let (_, ordinary_peak_bytes) = animation_sdf_retained_surface_bytes(1_830, 812).unwrap();
    assert!(ordinary_peak_bytes < 64 * 1024 * 1024);
}

#[cfg(feature = "animation-export")]
#[test]
fn deferred_animation_window_keeps_complete_source_across_viewport_changes() {
    use sha2::Digest as _;

    let root = tempfile::tempdir().unwrap();
    let page_path = root.path().join("page-000.r8swz");
    let mut page = vec![0u8; crate::sdf::atlas::SWIZZLED_PAGE_HEADER_BYTES + 8 * 8];
    page[..crate::sdf::atlas::SWIZZLED_PAGE_MAGIC.len()]
        .copy_from_slice(crate::sdf::atlas::SWIZZLED_PAGE_MAGIC);
    page[12..16].copy_from_slice(&crate::sdf::atlas::SWIZZLED_PAGE_VERSION.to_le_bytes());
    page[16..20].copy_from_slice(&8u32.to_le_bytes());
    page[20..24].copy_from_slice(&8u32.to_le_bytes());
    page[24..28].copy_from_slice(&8u32.to_le_bytes());
    page[28..32].copy_from_slice(&8u32.to_le_bytes());
    page[crate::sdf::atlas::SWIZZLED_PAGE_HEADER_BYTES..].fill(128);
    std::fs::write(&page_path, &page).unwrap();
    let manifest = crate::sdf::atlas::SdfAtlasManifest {
        schema: crate::sdf::atlas::ATLAS_MANIFEST_SCHEMA.into(),
        generator_contract: "deferred-window-test".into(),
        font_family: "deferred-window-test".into(),
        font_sha256: "00".repeat(32),
        point_size: 8.0,
        spread: 1.0,
        pages: vec![crate::sdf::atlas::SdfAtlasPageManifest {
            file: "page-000.r8swz".into(),
            width: 8,
            height: 8,
            file_sha256: hex::encode(sha2::Sha256::digest(&page)),
        }],
        glyphs: Vec::new(),
        generation: crate::sdf::atlas::SdfAtlasGenerationReport {
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
    let manifest_path = root.path().join("manifest.json");
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let atlas =
        std::sync::Arc::new(crate::sdf::atlas::MappedSdfAtlas::open(manifest_path).unwrap());
    let mut atlases = crate::sdf::atlas::MappedSdfAtlasSet::new();
    let atlas_set = atlases.insert(atlas).unwrap();
    let command = crate::sdf::tile::SdfDrawCommand {
        kind: crate::sdf::tile::SdfPrimitiveKind::Text,
        atlas_set,
        atlas_page: 0,
        atlas_rect: [0, 0, 8, 8],
        quad: [
            crate::sdf::tile::Point2 { x: 0.0, y: 0.0 },
            crate::sdf::tile::Point2 {
                x: 10_000.0,
                y: 0.0,
            },
            crate::sdf::tile::Point2 {
                x: 10_000.0,
                y: 10_000.0,
            },
            crate::sdf::tile::Point2 {
                x: 0.0,
                y: 10_000.0,
            },
        ],
        device_clip: None,
        material: crate::sdf::tile::SdfCommandMaterial::Text(
            crate::sdf::tile::SdfMaterial::default(),
        ),
    };
    let deferred = DeferredAnimationSdfLayer {
        commands: vec![command],
        text_atlases: std::sync::Arc::new(atlases),
        shape_atlas: None,
        runtime_text: Vec::new(),
        executor: Executor::Scalar,
        tile_width: 32,
        tile_height: 32,
        x: 0,
        y: 0,
        width: 10_000,
        height: 10_000,
    };
    let renderer = CustomProfileRenderer::new(std::sync::Arc::new(NullProvider));
    let first = renderer
        .render_deferred_animation_sdf_window(&deferred, 0, 0, 64, 64)
        .unwrap();
    let shifted = renderer
        .render_deferred_animation_sdf_window(&deferred, 9_000, 9_000, 64, 64)
        .unwrap();
    let restored = renderer
        .render_deferred_animation_sdf_window(&deferred, 0, 0, 64, 64)
        .unwrap();

    let first_rgba = first.pixels.clone();
    assert!(first_rgba.chunks_exact(4).any(|pixel| pixel[3] != 0));
    assert_eq!(shifted.pixels, first_rgba);
    assert_eq!(restored.pixels, first_rgba);
    assert_eq!(first.scratch_peak_bytes, 64 * 64 * 8);
}

/// A provider with no optional master-data entries keeps renderer setup independent of assets.
struct NullProvider;

impl MasterDataProvider for NullProvider {
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
        None
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
    fn resolve_unit_vs_sd(&self, _self_id: i32, _partner_id: i32) -> i32 {
        0
    }
    fn font_count(&self) -> usize {
        0
    }
    fn color_count(&self) -> usize {
        0
    }
}

/// Reports one server region and otherwise carries no master data.
struct RegionalProvider(crate::region::Region);

impl MasterDataProvider for RegionalProvider {
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
        Some("FZLanTingHei-DB-GBK".into())
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
    fn region(&self) -> crate::region::Region {
        self.0
    }
}

#[test]
fn native_scene_captions_follow_the_masterdata_region() {
    use crate::region::Region;
    use sekai_profile_core::{SemanticCommandPayload, TextSource};

    let card: crate::types::CustomProfileCard = serde_json::from_value(serde_json::json!({
        "generals": [{
            "objectData": {
                "layer": 1, "lock": false,
                "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                "scale": { "x": 1.0, "y": 1.0, "z": 1.0 },
                "visible": true
            },
            "type": 4
        }]
    }))
    .expect("card");
    let profile = crate::profile::ProfileData {
        word: "Bio".into(),
        ..crate::profile::ProfileData::default()
    };
    for (region, expected) in [(Region::Cn, "个性签名"), (Region::Jp, "ひとこと")] {
        let md = MasterData::new(Arc::new(RegionalProvider(region)));
        let scene = resolve_native_profile_scene(&card, &md, Some(&profile), None, None)
            .expect("resolve scene");
        let title = scene
            .commands
            .iter()
            .find_map(|command| match &command.payload {
                SemanticCommandPayload::Text {
                    source: TextSource::Localized { key, value, .. },
                    ..
                } if key == "custom_profile.general.comment.title" => Some(value.as_str()),
                _ => None,
            });
        assert_eq!(title, Some(expected), "{region:?}");
    }
}

const CHARACTER_ICON_KEY: &str = "custom_profile/character_icon/profile_chr_icon_ichika";
const MATERIAL_KEY: &str = "custom_profile/material/profile_icon_item_0003";
const USER_INTERFACE_ICON_KEY: &str = "custom_profile/user_interface_icon/profile_icon_0002";
const ETC_KEY: &str = "custom_profile/etc/etc_001";

const BADGE_KEY: &str = "custom_profile/collection/crash/crash_fixture_canbadge";
const OMIKUJI_KEY: &str = "lottery_game/new_year_2022/Prefabs/Omikuji";
const OMIKUJI_COVER_KEY: &str = "lottery_game/new_year_2022_material/bg_omikuji_street";
const OMIKUJI_FORTUNE_KEY: &str = "lottery_game/new_year_2022_material/unsei_kichi";
const STAND_KEY: &str = "custom_profile/collection/acrylic/acrylic_fixture";
const BADGE_NORMAL_MAP_KEY: &str = "ui/sekai_badge_normal";

/// Colours, `customProfileEtcResources` 1, collections 1 (can badge),
/// 2 (omikuji prefab) and 3 (acrylic stand), `omikujis` row 7, and, when
/// `icon_tables` is set, character icon 1, material 3 and user-interface
/// icon 2. A region without the icon tables has only the others.
struct IconRowsProvider {
    icon_tables: bool,
}

impl MasterDataProvider for IconRowsProvider {
    fn resolve_story_banner(&self, _story_type: &str, _story_id: i32) -> Option<String> {
        None
    }
    fn get_card(&self, _card_id: i32) -> Option<CardEntry> {
        None
    }
    fn resolve_color(&self, color_id: i32) -> Option<ResolvedColor> {
        match color_id {
            7 => ResolvedColor::from_hex("#444466"),
            2 => ResolvedColor::from_hex("#ff8000"),
            _ => None,
        }
    }
    fn default_color(&self) -> Option<ResolvedColor> {
        self.resolve_color(7)
    }
    fn resolve_font(&self, _font_id: i32) -> Option<String> {
        None
    }
    fn resolve_stamp(&self, _stamp_id: i32) -> Option<String> {
        None
    }
    fn resolve_resource(&self, res_type: &str, id: i32) -> Option<ResourceInfo> {
        let plain = CollectionResourceType::None;
        let (key, collection_type) = match (res_type, id) {
            ("etc", 1) => (ETC_KEY, plain),
            ("collection", 1) => (BADGE_KEY, CollectionResourceType::CanBadge),
            ("collection", 2) => (OMIKUJI_KEY, CollectionResourceType::Omikuji),
            ("collection", 3) => (STAND_KEY, CollectionResourceType::AcrylicStand),
            ("character_icon", 1) if self.icon_tables => (CHARACTER_ICON_KEY, plain),
            ("material", 3) if self.icon_tables => (MATERIAL_KEY, plain),
            ("user_interface_icon", 2) if self.icon_tables => (USER_INTERFACE_ICON_KEY, plain),
            _ => return None,
        };
        // The slip prefab sits in a folder of its bundle.
        let (load_val, file_name) = match key.split_once("/Prefabs/") {
            Some((load_val, _)) => (load_val, sekai_profile_core::omikuji::PREFAB_FILE),
            None => key.rsplit_once('/')?,
        };
        Some(ResourceInfo {
            file_name: file_name.into(),
            load_val: load_val.into(),
            resource_type: res_type.into(),
            collection_type,
        })
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
        0
    }
    fn color_count(&self) -> usize {
        2
    }
    fn resolve_omikuji(&self, id: i32) -> Option<OmikujiRow> {
        (id == 7).then(|| OmikujiRow {
            id,
            unit: "street".into(),
            summary: "夢の実現に\n近づく年".into(),
            title1: "願望".into(),
            description1: "必ず叶う".into(),
            title2: "健康".into(),
            description2: "大変良好".into(),
            title3: "待人".into(),
            description3: "必ず来る".into(),
            fortune_assetbundle_name: "lottery_game/new_year_2022_material".into(),
            fortune_file_path: "unsei_kichi".into(),
            omikuji_cover_assetbundle_name: "lottery_game/new_year_2022_material".into(),
            omikuji_cover_file_path: "omikuji_street".into(),
        })
    }
}

/// A version-4 page with one visible element of each icon kind in a row,
/// a decoration above the material, a hidden character icon and a
/// material whose row does not exist.
fn icon_page() -> CustomProfileCard {
    let object = |layer: i32, x: f32, y: f32, visible: bool| {
        serde_json::json!({
            "layer": layer, "lock": false, "visible": visible,
            "position": { "x": x, "y": y, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
        })
    };
    serde_json::from_value(serde_json::json!({
        "characterIcons": [
            { "objectData": object(1, -600.0, 0.0, true), "id": 1 },
            { "objectData": object(5, -600.0, 200.0, false), "id": 1 }
        ],
        "materials": [
            { "objectData": object(2, 0.0, 0.0, true), "id": 3 },
            { "objectData": object(6, 600.0, 200.0, true), "id": 99 }
        ],
        "userInterfaceIcons": [
            { "objectData": object(3, 600.0, 0.0, true), "id": 2, "colorId": 2, "alpha": 0.5 }
        ],
        "others": [{ "objectData": object(4, 0.0, 200.0, true), "id": 1 }]
    }))
    .expect("version-4 page")
}

fn icon_page_renderer(icon_tables: bool) -> (tempfile::TempDir, CustomProfileRenderer) {
    page_renderer(
        icon_tables,
        &[
            (CHARACTER_ICON_KEY, [255, 0, 0, 255]),
            (MATERIAL_KEY, [0, 255, 0, 255]),
            (USER_INTERFACE_ICON_KEY, [255, 255, 255, 255]),
            (ETC_KEY, [0, 0, 255, 255]),
        ],
    )
}

/// A renderer over [`IconRowsProvider`] whose asset store holds 8x8
/// images of one colour each.
fn page_renderer(
    icon_tables: bool,
    images: &[(&str, [u8; 4])],
) -> (tempfile::TempDir, CustomProfileRenderer) {
    let assets = Arc::new(AssetStore::new(8));
    for &(key, pixel) in images {
        assets.put(
            key.into(),
            crate::codec::png::encode_rgba(8, 8, &pixel.repeat(64)).expect("png"),
        );
    }
    // The store holds an unrelated object only, so every image draws from
    // the asset store's source bytes.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut writer = crate::render_object::RenderObjectStoreWriter::create(
        temp.path().join("store"),
        "icon-fixture",
        4096,
    )
    .expect("store writer");
    writer
        .add(crate::render_object::RenderObjectWrite {
            key: "unrelated",
            kind: crate::render_object::RenderObjectKind::Texture,
            source_sha256: &"0".repeat(64),
            width: 1,
            height: 1,
            row_bytes: 4,
            pixels: &[0; 4],
        })
        .expect("store object");
    let store = crate::render_object::MappedRenderObjectStore::open(
        writer.finish().expect("store manifest"),
    )
    .expect("store");
    let renderer = CustomProfileRenderer::new(Arc::new(IconRowsProvider { icon_tables }))
        .with_assets(assets)
        .with_render_object_store(Arc::new(store));
    (temp, renderer)
}

/// Premultiplied pixel at a Unity position of a full page.
fn pixel_at(page: &RenderedPage, x: f32, y: f32) -> [u8; 4] {
    let (x, y) = sekai_profile_core::profile_transform::unity_to_canvas(x, y);
    let offset = (y as usize * page.width() as usize + x as usize) * 4;
    page.premultiplied_rgba()[offset..offset + 4]
        .try_into()
        .expect("pixel inside the surface")
}

/// Renders `card` over a transparent background on the scalar executor.
fn render_transparent(
    renderer: &CustomProfileRenderer,
    card: &CustomProfileCard,
) -> Result<RenderedPage, String> {
    renderer
        .render(
            card,
            None,
            &RenderConfig {
                executor: crate::SdfExecutor::Scalar,
                background: crate::Background::Transparent,
                ..RenderConfig::default()
            },
        )
        .map_err(|error| error.to_string())
}

#[test]
fn icon_elements_request_their_assets_and_draw_on_the_native_surface() {
    let card = icon_page();
    let (_temp, renderer) = icon_page_renderer(true);
    let md = renderer.snapshot();
    let mut keys = crate::asset_keys::collect_card_asset_keys(&card, &md);
    keys.sort();
    assert_eq!(
        keys,
        [
            CHARACTER_ICON_KEY,
            ETC_KEY,
            MATERIAL_KEY,
            USER_INTERFACE_ICON_KEY
        ]
    );
    let output = render_transparent(&renderer, &card).expect("rendered page");
    assert_eq!(pixel_at(&output, -600.0, 0.0), [255, 0, 0, 255]);
    assert_eq!(pixel_at(&output, 0.0, 0.0), [0, 255, 0, 255]);
    // A white icon tinted #ff8000 at alpha 128/255, premultiplied.
    assert_eq!(pixel_at(&output, 600.0, 0.0), [128, 64, 0, 128]);
    assert_eq!(pixel_at(&output, 0.0, 200.0), [0, 0, 255, 255]);
    // Hidden elements and elements without a row are not drawn.
    assert_eq!(pixel_at(&output, -600.0, 200.0), [0; 4]);
    assert_eq!(pixel_at(&output, 600.0, 200.0), [0; 4]);
}

/// A can badge, an omikuji collection and an acrylic stand in a row, and
/// a hidden can badge below the omikuji.
fn collection_page() -> CustomProfileCard {
    let object = |layer: i32, x: f32, y: f32, visible: bool| {
        serde_json::json!({
            "layer": layer, "lock": false, "visible": visible,
            "position": { "x": x, "y": y, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
        })
    };
    serde_json::from_value(serde_json::json!({
        "collections": [
            { "objectData": object(1, -600.0, 0.0, true), "id": 1, "targetId": null },
            { "objectData": object(2, 0.0, 0.0, true), "id": 2, "targetId": null },
            { "objectData": object(3, 600.0, 0.0, true), "id": 3, "targetId": null },
            { "objectData": object(4, 0.0, 200.0, false), "id": 1, "targetId": null }
        ]
    }))
    .expect("collection page")
}

const COLLECTION_IMAGES: [(&str, [u8; 4]); 3] = [
    (BADGE_KEY, [200, 40, 90, 255]),
    (STAND_KEY, [0, 255, 0, 255]),
    // A normal tilted away from the flat surface.
    (BADGE_NORMAL_MAP_KEY, [200, 90, 220, 255]),
];

#[test]
fn collections_follow_their_collection_type_on_the_native_surface() {
    let card = collection_page();
    let (_temp, renderer) = page_renderer(false, &COLLECTION_IMAGES);
    let md = renderer.snapshot();
    let mut keys = crate::asset_keys::collect_card_asset_keys(&card, &md);
    keys.sort();
    // The omikuji has no `omikujis` row to show: it requests nothing.
    assert_eq!(keys, [STAND_KEY, BADGE_KEY, BADGE_NORMAL_MAP_KEY]);
    let output = render_transparent(&renderer, &card).expect("rendered page");
    // The badge image lit through the normal map, from a double-precision
    // evaluation of the badge material.
    assert_eq!(pixel_at(&output, -600.0, 0.0), [203, 49, 97, 255]);
    assert_eq!(pixel_at(&output, 0.0, 0.0), [0; 4]);
    assert_eq!(pixel_at(&output, 600.0, 0.0), [0, 255, 0, 255]);
    assert_eq!(pixel_at(&output, 0.0, 200.0), [0; 4]);
}

/// An omikuji collection showing `omikujis` row 7, and one whose row does
/// not exist below it.
fn omikuji_page() -> CustomProfileCard {
    let object = |layer: i32, y: f32| {
        serde_json::json!({
            "layer": layer, "lock": false, "visible": true,
            "position": { "x": 0.0, "y": y, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
        })
    };
    serde_json::from_value(serde_json::json!({
        "collections": [
            { "objectData": object(1, 0.0), "id": 2, "targetId": 7 },
            { "objectData": object(2, -300.0), "id": 2, "targetId": 8 }
        ]
    }))
    .expect("omikuji page")
}

#[test]
fn an_omikuji_collection_requests_its_slip_images_and_draws_with_its_font() {
    let card = omikuji_page();
    let cover = [30, 60, 90, 255];
    let fortune = [200, 0, 0, 255];
    let (_temp, renderer) = page_renderer(
        false,
        &[(OMIKUJI_COVER_KEY, cover), (OMIKUJI_FORTUNE_KEY, fortune)],
    );
    let md = renderer.snapshot();
    let mut keys = crate::asset_keys::collect_card_asset_keys(&card, &md);
    keys.sort();
    assert_eq!(keys, [OMIKUJI_COVER_KEY, OMIKUJI_FORTUNE_KEY]);
    let rendered = render_transparent(&renderer, &card);
    // The slip's texts need the font asset's file and both fallback
    // faces; without one of them the page fails rather than drawing the
    // slip without its texts.
    let client = sekai_profile_core::omikuji::OmikujiClient::for_masterdata(&md, "");
    let missing = [
        "FOT-Omikuji",
        sekai_profile_core::omikuji::LATIN_FALLBACK_FAMILY,
        client.cjk_fallback.family(),
    ]
    .into_iter()
    .find(|family| crate::sdf::outline::resolve_font_path(family).is_none());
    match (missing, rendered) {
        (Some(family), Err(error)) => assert!(
            error.contains(&format!("omikuji-title needs font {family}")),
            "{error}"
        ),
        (None, Ok(output)) => {
            assert_eq!(pixel_at(&output, -600.0, 0.0), cover);
            assert_eq!(pixel_at(&output, 624.0, 1.0), fortune);
            // The collection without a row draws nothing.
            assert_eq!(pixel_at(&output, -600.0, -300.0 - 200.0), [0; 4]);
        }
        (missing, rendered) => panic!(
            "missing font {missing:?}: {:?}",
            rendered.map(|output| output.width())
        ),
    }
}

#[test]
fn a_missing_badge_normal_map_is_reported_and_fails_the_page() {
    let card = collection_page();
    let (_temp, renderer) = page_renderer(false, &COLLECTION_IMAGES[..2]);
    let md = renderer.snapshot();
    let assets = renderer.assets().expect("asset store");
    assert_eq!(
        crate::asset_keys::missing_card_asset_keys(&card, &md, |key| assets.contains(key)),
        [BADGE_NORMAL_MAP_KEY]
    );
    match render_transparent(&renderer, &card) {
        Err(error) => assert!(
            error.contains("missing render object texture:assets/ui/sekai_badge_normal"),
            "{error}"
        ),
        Ok(_) => panic!("a missing required resource fails the page"),
    }
}

#[test]
fn a_region_without_the_icon_tables_still_renders_the_rest_of_the_page() {
    let card = icon_page();
    let (_temp, renderer) = icon_page_renderer(false);
    let md = renderer.snapshot();
    assert_eq!(
        crate::asset_keys::collect_card_asset_keys(&card, &md),
        [ETC_KEY]
    );
    let output = render_transparent(&renderer, &card).expect("rendered page");
    assert_eq!(pixel_at(&output, 0.0, 200.0), [0, 0, 255, 255]);
    for (x, y) in [(-600.0, 0.0), (0.0, 0.0), (600.0, 0.0)] {
        assert_eq!(pixel_at(&output, x, y), [0; 4], "({x}, {y})");
    }
}

/// The scalar, transparent config the pixel checks above render with.
fn scalar_transparent() -> RenderConfig {
    RenderConfig {
        executor: crate::SdfExecutor::Scalar,
        background: crate::Background::Transparent,
        ..RenderConfig::default()
    }
}

#[test]
fn a_batch_renders_each_page_as_a_single_render_does() {
    let card = icon_page();
    let (_temp, renderer) = icon_page_renderer(true);
    let config = scalar_transparent();
    let single = renderer.render(&card, None, &config).expect("single page");
    let batch = renderer
        .render_batch("batch", "cn", [(2, &card), (1, &card)], None, &config)
        .expect("batch");
    let seqs: Vec<i32> = batch.pages.iter().map(|(seq, _)| *seq).collect();
    assert_eq!(seqs, [1, 2], "pages come back in ascending seq order");
    assert_eq!(batch.compiled.pages.len(), 2);
    for (seq, page) in &batch.pages {
        assert_eq!(
            (page.width(), page.height()),
            (single.width(), single.height()),
            "seq {seq}"
        );
        assert_eq!(
            page.premultiplied_rgba(),
            single.premultiplied_rgba(),
            "seq {seq}"
        );
    }
    // The store holds none of the page's images, so the batch reports each
    // one missing and decodes it from the asset store instead.
    let prepared = batch
        .prepared_render_objects
        .expect("a store is installed, so the batch reports its coverage");
    assert_eq!(prepared.mapped_object_count, 0);
    assert!(!prepared.missing_object_keys.is_empty());
    assert_eq!(
        prepared
            .missing_page_object_keys
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [1, 2]
    );
}

#[test]
fn a_batch_refuses_a_config_the_cpu_cannot_run_before_compiling() {
    let card = icon_page();
    let (_temp, renderer) = icon_page_renderer(true);
    let invalid = RenderConfig {
        tile_width: 24,
        ..scalar_transparent()
    };
    match renderer.render_batch("batch", "cn", [(1, &card)], None, &invalid) {
        Err(crate::RenderError::InvalidTileSize { width, height }) => {
            assert_eq!((width, height), (24, 32));
        }
        Err(other) => panic!("unexpected error: {other}"),
        Ok(_) => panic!("an invalid tile size renders"),
    }
}

#[test]
fn a_white_page_differs_from_a_transparent_one_only_where_nothing_is_drawn() {
    let card = icon_page();
    let (_temp, renderer) = icon_page_renderer(true);
    let transparent = renderer
        .render(&card, None, &scalar_transparent())
        .expect("transparent");
    let white = renderer
        .render(
            &card,
            None,
            &RenderConfig {
                background: crate::Background::White,
                ..scalar_transparent()
            },
        )
        .expect("white");
    assert_eq!(transparent.background(), crate::Background::Transparent);
    assert_eq!(white.background(), crate::Background::White);
    // An empty spot is cleared to each background.
    assert_eq!(pixel_at(&transparent, 600.0, 200.0), [0; 4]);
    assert_eq!(pixel_at(&white, 600.0, 200.0), [255; 4]);
    // An opaque icon covers the background either way.
    assert_eq!(pixel_at(&white, -600.0, 0.0), [255, 0, 0, 255]);
    assert_eq!(pixel_at(&transparent, -600.0, 0.0), [255, 0, 0, 255]);
}

#[test]
fn capabilities_report_what_the_renderer_has_installed() {
    let bare = CustomProfileRenderer::new(Arc::new(NullProvider));
    let capabilities = bare.capabilities();
    assert_eq!(capabilities.simd, crate::sdf_simd_available());
    assert_eq!(capabilities.text_atlases, 0);
    assert!(!capabilities.shape_atlas);
    assert!(!capabilities.render_objects);
    assert!(!capabilities.fallback_glyphs);

    let (_temp, with_store) = icon_page_renderer(true);
    assert!(with_store.capabilities().render_objects);
}

#[test]
fn an_empty_atlas_directory_installs_nothing_and_still_renders() {
    let root = tempfile::tempdir().expect("root");
    let card = icon_page();
    let (_temp, renderer) = icon_page_renderer(true);
    let renderer = renderer
        .with_atlas_dir(root.path().join("absent"))
        .expect("an absent directory installs no atlas");
    let capabilities = renderer.capabilities();
    assert_eq!(capabilities.text_atlases, 0);
    assert!(!capabilities.shape_atlas);
    let page = render_transparent(&renderer, &card).expect("rendered page");
    assert_eq!(pixel_at(&page, -600.0, 0.0), [255, 0, 0, 255]);
}

#[test]
fn the_cache_identity_follows_the_installed_store() {
    let without = CustomProfileRenderer::new(Arc::new(NullProvider));
    let (_temp, with_store) = icon_page_renderer(true);
    let config = RenderConfig::default();
    assert_ne!(
        without.cache_identity(&config).expect("identity"),
        with_store.cache_identity(&config).expect("identity"),
    );
    let generation = with_store.pin_render_object_generation();
    assert_eq!(
        with_store.cache_identity(&config).expect("identity"),
        with_store
            .cache_identity_pinned(&config, &generation)
            .expect("pinned identity"),
    );
}

#[cfg(feature = "jpeg-turbo")]
#[test]
fn an_encoded_page_carries_the_page_size_and_its_telemetry() {
    let card = icon_page();
    let (_temp, renderer) = icon_page_renderer(true);
    let page = renderer
        .render(
            &card,
            None,
            &RenderConfig {
                background: crate::Background::White,
                ..scalar_transparent()
            },
        )
        .expect("page");
    let (width, height) = (page.width(), page.height());
    let encoded = renderer
        .encode_jpeg(page, crate::JpegOptions::default())
        .expect("jpeg");
    assert_eq!((encoded.width, encoded.height), (width, height));
    assert_eq!(&encoded.bytes[..2], &[0xff, 0xd8], "a JPEG starts with SOI");
    assert_eq!(
        encoded.telemetry.bytes.encoded_output_bytes,
        encoded.bytes.len() as u64
    );
    assert_eq!(encoded.telemetry.jpeg, Some(crate::JpegOptions::default()));
}

#[cfg(feature = "jpeg-turbo")]
#[test]
fn jpeg_quality_outside_1_to_100_is_refused() {
    let card = icon_page();
    let (_temp, renderer) = icon_page_renderer(true);
    let white = RenderConfig {
        background: crate::Background::White,
        ..scalar_transparent()
    };
    for quality in [0, 101] {
        let page = renderer.render(&card, None, &white).expect("page");
        let error = renderer
            .encode_jpeg(
                page,
                crate::JpegOptions {
                    quality,
                    ..crate::JpegOptions::default()
                },
            )
            .err()
            .expect("an out-of-range quality is refused");
        assert!(error.to_string().contains("quality"), "{error}");
    }
}
