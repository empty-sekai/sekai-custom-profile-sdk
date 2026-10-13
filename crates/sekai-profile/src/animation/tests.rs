use super::*;

fn deterministic_rgba(width: usize, height: usize) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            rgba.extend_from_slice(&[
                ((x * 17 + y * 29 + 3) & 255) as u8,
                ((x * 71 + y * 11 + 19) & 255) as u8,
                ((x * 7 + y * 97 + 113) & 255) as u8,
                ((x * 43 + y * 31 + 251) & 255) as u8,
            ]);
        }
    }
    rgba
}

#[cfg(target_arch = "x86_64")]
fn assert_avx512_yuv420_matches_scalar(width: usize, height: usize) {
    if !std::arch::is_x86_feature_detected!("avx512f")
        || !std::arch::is_x86_feature_detected!("avx512bw")
    {
        return;
    }
    let rgba = deterministic_rgba(width, height);
    let mut scalar = vec![0u8; width * height * 3 / 2];
    let mut simd = vec![0u8; scalar.len()];
    rgba_to_yuv420p_scalar(&rgba, width, height, &mut scalar);
    // SAFETY: feature detection above satisfies the target-feature contract.
    unsafe { rgba_to_yuv420p_avx512(&rgba, width, height, &mut simd) };
    assert_eq!(simd, scalar);
}

#[test]
#[cfg(target_arch = "x86_64")]
fn avx512_yuv420_matches_scalar_across_a_non_zmm_tail() {
    assert_avx512_yuv420_matches_scalar(34, 6);
}

#[test]
#[cfg(target_arch = "x86_64")]
fn avx512_yuv420_matches_scalar_at_profile_export_resolution() {
    assert_avx512_yuv420_matches_scalar(1832, 816);
}

#[test]
fn yuv420_scratch_reuse_overwrites_dirty_storage() {
    let width = 4;
    let height = 2;
    let mut config = H264EncoderConfig::default();
    config.yuv_converter = H264YuvConverter::Scalar;
    let first = [17, 91, 203, 255].repeat((width * height) as usize);
    let second = [241, 37, 11, 255].repeat((width * height) as usize);
    let expected_first = rgba_to_yuv420p(&first, width, height, &config).unwrap();
    let expected_second = rgba_to_yuv420p(&second, width, height, &config).unwrap();
    let mut scratch = vec![0xa5; expected_first.len()];

    rgba_to_yuv420p_into(&first, width, height, &config, &mut scratch).unwrap();
    assert_eq!(scratch, expected_first);
    scratch.fill(0x5a);
    rgba_to_yuv420p_into(&second, width, height, &config, &mut scratch).unwrap();
    assert_eq!(scratch, expected_second);
}

#[test]
fn exact_dirty_scan_marks_only_rgba_changed_macroblocks() {
    let width = 34u32;
    let height = 18u32;
    let mut before = vec![17u8; width as usize * height as usize * 4];
    let mut after = before.clone();
    let pixel = |x: usize, y: usize| (y * width as usize + x) * 4;
    after[pixel(17, 0)] = 99;
    after[pixel(33, 17) + 2] = 201;
    let mut flags = vec![0u8; width.div_ceil(16) as usize * height.div_ceil(16) as usize];
    mark_changed_macroblocks_for_region(
        &before,
        &after,
        width,
        PixelRect {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        },
        &mut flags,
    )
    .unwrap();
    assert_eq!(flags, [0, 1, 0, 0, 0, 1]);

    before.copy_from_slice(&after);
    flags.fill(0);
    mark_changed_macroblocks_for_region(
        &before,
        &after,
        width,
        PixelRect {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        },
        &mut flags,
    )
    .unwrap();
    assert!(flags.iter().all(|&value| value == 0));
}

#[test]
fn macroblock_yuv_updates_match_full_frame_conversion() {
    let width = 34u32;
    let height = 18u32;
    let config = H264EncoderConfig::default();
    let first = deterministic_rgba(width as usize, height as usize);
    let mut second = first.clone();
    for (x, y, color) in [
        (17usize, 0usize, [9, 81, 173, 255]),
        (33, 17, [233, 7, 61, 255]),
    ] {
        let offset = (y * width as usize + x) * 4;
        second[offset..offset + 4].copy_from_slice(&color);
    }
    let mut partial = rgba_to_yuv420p(&first, width, height, &config).unwrap();
    let expected = rgba_to_yuv420p(&second, width, height, &config).unwrap();
    rgba_to_yuv420p_macroblocks_into(&second, width, height, &[1, 5], &config, &mut partial)
        .unwrap();
    assert_eq!(partial, expected);
}

#[test]
fn h264_cache_identity_covers_every_output_affecting_setting() {
    let base = H264EncoderConfig::default();
    let base_identity = base.cache_identity().unwrap();
    let mut changed = base.clone();
    changed.qp = 19;
    assert_ne!(base_identity, changed.cache_identity().unwrap());
    changed = base.clone();
    changed.yuv_converter = H264YuvConverter::Avx512;
    assert_ne!(base_identity, changed.cache_identity().unwrap());
    changed = base;
    changed.encoder_revision = "cache-identity-test-revision".into();
    assert_ne!(base_identity, changed.cache_identity().unwrap());
}

#[test]
fn x264_mb_info_marks_only_macroblocks_outside_dirty_regions_constant() {
    let mut mb_info = Vec::new();
    assert_eq!(
        fill_x264_mb_info(&FrameCompositeUpdate::Full, 50, 33, &mut mb_info).unwrap(),
        (0, 12)
    );
    assert!(mb_info.iter().all(|&flags| flags == 0));

    assert_eq!(
        fill_x264_mb_info(&FrameCompositeUpdate::Reused, 50, 33, &mut mb_info).unwrap(),
        (12, 12)
    );
    assert!(mb_info.iter().all(|&flags| flags == 1));

    assert_eq!(
        fill_x264_mb_info(
            &FrameCompositeUpdate::Dirty(vec![1, 5]),
            50,
            33,
            &mut mb_info,
        )
        .unwrap(),
        (10, 12)
    );
    assert_eq!(&mb_info[..4], &[1, 0, 1, 1]);
    assert_eq!(&mb_info[4..8], &[1, 0, 1, 1]);
    assert_eq!(&mb_info[8..], &[1, 1, 1, 1]);
}

#[test]
fn non_mp4_encoder_suffix_preserves_existing_animation_cache_contract() {
    let gif = resolve_preset("qq", Some(AnimationFormat::Gif)).unwrap();
    assert_eq!(animation_encoder_cache_suffix(&gif).unwrap(), "");
    let mp4 = resolve_preset("qq", Some(AnimationFormat::Mp4)).unwrap();
    assert!(animation_encoder_cache_suffix(&mp4)
        .unwrap()
        .starts_with("-h264-v2-"));
}

#[test]
fn animation_memory_gate_preserves_retained_buffer_contract() {
    assert_eq!(animation_peak_export_bytes(100, 300), 700);
    assert_eq!(animation_peak_export_bytes(usize::MAX, 1), usize::MAX);

    let gif = resolve_preset("qq", Some(AnimationFormat::Gif)).unwrap();
    assert_eq!(gif.export_memory_budget_bytes, 64 * 1024 * 1024);
    let mp4 = resolve_preset("qq", Some(AnimationFormat::Mp4)).unwrap();
    assert_eq!(mp4.export_memory_budget_bytes, 64 * 1024 * 1024);
    assert!(animation_peak_export_bytes(1_115_785_640, 0) > mp4.export_memory_budget_bytes);
}

#[test]
fn dynamic_animation_groups_keep_motion_bounds_during_rasterization() {
    assert!(!animation_group_uses_dynamic_bounds(None));
    assert!(animation_group_uses_dynamic_bounds(Some(
        sekai_profile_core::StableId(1)
    )));
}

#[test]
fn deferred_sdf_tile_cache_evicts_lru_entries_within_budget() {
    fn rgba(color: [u8; 4]) -> Vec<u8> {
        color.to_vec().repeat(4 * 4)
    }

    let key = |layer_index| DeferredSdfTileKey {
        layer_index,
        x: 0,
        y: 0,
        width: 4,
        height: 4,
    };
    let mut cache = DeferredSdfTileCache::new(4 * 4 * 4 * 2);
    cache.insert(key(1), rgba([255, 0, 0, 255]));
    cache.insert(key(2), rgba([0, 255, 0, 255]));
    assert!(cache.get(key(1)).is_some());
    cache.insert(key(3), rgba([0, 0, 255, 255]));

    assert!(cache.entries.contains_key(&key(1)));
    assert!(!cache.entries.contains_key(&key(2)));
    assert!(cache.entries.contains_key(&key(3)));
    assert!(cache.resident_bytes <= cache.budget_bytes);
    assert_eq!(cache.peak_bytes, cache.budget_bytes);
}

#[test]
fn deferred_source_window_tracks_translation_and_guard_pixels() {
    let layer = RasterLayer {
        dynamic_layer_id: None,
        content: RasterLayerContent::Pixels(Vec::new()),
        x: 100.0,
        y: 200.0,
        width: 100,
        height: 50,
    };
    let state = LayerFrameState {
        visible: true,
        destination: FrameRect::from_xywh(120.0, 190.0, 100.0, 50.0),
        pixel_bounds: PixelRect::default(),
    };

    assert_eq!(
        deferred_source_window(
            &layer,
            &state,
            PixelRect {
                left: 130,
                top: 210,
                right: 160,
                bottom: 230,
            },
            1.0,
        ),
        Some(PixelRect {
            left: 108,
            top: 218,
            right: 142,
            bottom: 242,
        })
    );
}

#[test]
fn deferred_source_window_clamps_fractional_scaled_viewport_to_layer() {
    let layer = RasterLayer {
        dynamic_layer_id: None,
        content: RasterLayerContent::Pixels(Vec::new()),
        x: 10.0,
        y: 20.0,
        width: 100,
        height: 50,
    };
    let state = LayerFrameState {
        visible: true,
        destination: FrameRect::from_xywh(13.25, 17.5, 50.0, 25.0),
        pixel_bounds: PixelRect::default(),
    };

    assert_eq!(
        deferred_source_window(
            &layer,
            &state,
            PixelRect {
                left: 5,
                top: 10,
                right: 30,
                bottom: 25,
            },
            0.5,
        ),
        Some(PixelRect {
            left: 10,
            top: 20,
            right: 46,
            bottom: 37,
        })
    );
    assert_eq!(
        deferred_source_window(
            &layer,
            &state,
            PixelRect {
                left: 100,
                top: 100,
                right: 110,
                bottom: 110,
            },
            0.5,
        ),
        None
    );
}

#[test]
fn profile_animation_frame_starts_with_opaque_white_canvas() {
    let scene = sekai_profile_core::Scene::new(sekai_profile_core::SceneSource {
        scene_id: sekai_profile_core::StableId(1),
        region: "cn".into(),
        font_engine_fingerprint: "test".into(),
        raster_contract: "test".into(),
        layers: Vec::new(),
        glyphs: Vec::new(),
        semantic_commands: Vec::new(),
        interaction_regions: Vec::new(),
        component_controls: Vec::new(),
    })
    .unwrap();

    let rgba = composite_frame(&scene, &[], 2, 2, 1.0).unwrap();
    assert_eq!(rgba, [255, 255, 255, 255].repeat(4));
}

#[test]
fn compositor_into_matches_owned_frame_output() {
    let scene = sekai_profile_core::Scene::new(sekai_profile_core::SceneSource {
        scene_id: sekai_profile_core::StableId(1),
        region: "cn".into(),
        font_engine_fingerprint: "test".into(),
        raster_contract: "test".into(),
        layers: Vec::new(),
        glyphs: Vec::new(),
        semantic_commands: Vec::new(),
        interaction_regions: Vec::new(),
        component_controls: Vec::new(),
    })
    .unwrap();
    let mut owned = AnimationFrameCompositor::new(2, 2, 1.0).unwrap();
    let expected = owned.composite(0, &scene, &[]).unwrap();
    let mut direct = AnimationFrameCompositor::new(2, 2, 1.0).unwrap();
    let mut actual = Vec::new();
    direct.composite_into(0, &scene, &[], &mut actual).unwrap();
    assert_eq!(actual, expected);
}

fn solid_layer(
    rgba: [u8; 4],
    x: f32,
    y: f32,
    width: u32,
    height: u32,
    dynamic_layer_id: Option<sekai_profile_core::LayerId>,
) -> RasterLayer {
    RasterLayer {
        dynamic_layer_id,
        content: RasterLayerContent::Pixels(rgba.repeat(width as usize * height as usize)),
        x,
        y,
        width,
        height,
    }
}

fn layer_state(x: f32, y: f32, width: f32, height: f32) -> LayerFrameState {
    let destination = FrameRect::from_xywh(x, y, width, height);
    LayerFrameState {
        visible: true,
        destination,
        pixel_bounds: pixel_bounds_for_destination(destination, 8, 4),
    }
}

fn read_frame(canvas: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    read_canvas_rgba_into(canvas, width, height, &mut rgba).unwrap();
    rgba
}

#[test]
fn dirty_compositor_matches_full_redraw_with_interleaved_static_layer() {
    let layers = vec![
        solid_layer([0, 0, 255, 255], 0.0, 0.0, 8, 4, None),
        solid_layer(
            [255, 0, 0, 255],
            1.0,
            1.0,
            2,
            2,
            Some(sekai_profile_core::StableId(1)),
        ),
        solid_layer([0, 128, 0, 255], 3.0, 0.0, 2, 4, None),
    ];
    let before = vec![
        layer_state(0.0, 0.0, 8.0, 4.0),
        layer_state(1.0, 1.0, 2.0, 2.0),
        layer_state(3.0, 0.0, 2.0, 4.0),
    ];
    let after = vec![
        layer_state(0.0, 0.0, 8.0, 4.0),
        layer_state(5.0, 1.0, 2.0, 2.0),
        layer_state(3.0, 0.0, 2.0, 4.0),
    ];
    let dirty = before[1].pixel_bounds.union(after[1].pixel_bounds);

    let mut incremental = vec![0u8; 8 * 4 * 4];
    redraw_full_frame(
        None,
        &mut None,
        &mut incremental,
        8,
        4,
        &layers,
        &before,
        1.0,
        &mut DeferredSdfTileCache::new(0),
    )
    .unwrap();
    redraw_dirty_frame(
        None,
        &mut None,
        &mut incremental,
        8,
        4,
        &layers,
        &after,
        dirty,
        1.0,
        &mut DeferredSdfTileCache::new(0),
    )
    .unwrap();

    let mut oracle = vec![0u8; 8 * 4 * 4];
    redraw_full_frame(
        None,
        &mut None,
        &mut oracle,
        8,
        4,
        &layers,
        &after,
        1.0,
        &mut DeferredSdfTileCache::new(0),
    )
    .unwrap();

    assert_eq!(read_frame(&incremental, 8, 4), read_frame(&oracle, 8, 4));
}

/// A provider that names exactly one shape resource, so a shape element
/// resolves to a stable asset key without any other master-data table.
struct SingleShapeProvider;

impl crate::masterdata::MasterDataProvider for SingleShapeProvider {
    fn resolve_story_banner(&self, _story_type: &str, _story_id: i32) -> Option<String> {
        None
    }
    fn get_card(&self, _card_id: i32) -> Option<crate::types::CardEntry> {
        None
    }
    fn resolve_color(&self, _color_id: i32) -> Option<crate::masterdata::ResolvedColor> {
        None
    }
    fn resolve_font(&self, _font_id: i32) -> Option<String> {
        None
    }
    fn resolve_stamp(&self, _stamp_id: i32) -> Option<String> {
        None
    }
    fn resolve_resource(&self, res_type: &str, id: i32) -> Option<crate::masterdata::ResourceInfo> {
        (res_type == "shape" && id == TELEMETRY_SHAPE_ID).then(|| crate::masterdata::ResourceInfo {
            file_name: TELEMETRY_SHAPE_FILE.into(),
            load_val: "custom_profile/shape".into(),
            resource_type: "shape".into(),
            collection_type: crate::masterdata::CollectionResourceType::None,
        })
    }
    fn resolve_honor(
        &self,
        _honor_id: i32,
        _honor_level: i32,
    ) -> Option<crate::masterdata::ResolvedHonor> {
        None
    }
    fn get_bonds_honor(&self, _id: i32) -> Option<crate::types::BondsHonorEntry> {
        None
    }
    fn get_bonds_honor_word(&self, _word_id: i64) -> Option<crate::types::BondsHonorWordEntry> {
        None
    }
    fn get_honor(&self, _honor_id: i32) -> Option<crate::types::HonorEntry> {
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

const TELEMETRY_SHAPE_ID: i32 = 1;
const TELEMETRY_SHAPE_FILE: &str = "telemetry_square";
const TELEMETRY_SHAPE_EXTENT: u32 = 8;

/// Writes a shape SDF atlas whose single entry matches the identity the
/// asset store derives for `asset_key`, and returns it opened.
fn telemetry_shape_atlas(
    root: &std::path::Path,
    asset_key: &str,
    identity_width: i32,
    identity_height: i32,
    identity_rg8_sha256: String,
) -> std::sync::Arc<crate::sdf::shape_atlas::MappedShapeSdfAtlas> {
    use sha2::Digest;

    let extent = TELEMETRY_SHAPE_EXTENT as usize;
    let mut page =
        vec![0u8; crate::sdf::shape_atlas::SHAPE_PAGE_HEADER_BYTES + extent * extent * 2];
    let magic = crate::sdf::shape_atlas::SHAPE_PAGE_MAGIC;
    page[..magic.len()].copy_from_slice(magic);
    page[12..16].copy_from_slice(&crate::sdf::shape_atlas::SHAPE_PAGE_VERSION.to_le_bytes());
    page[16..20].copy_from_slice(&TELEMETRY_SHAPE_EXTENT.to_le_bytes());
    page[20..24].copy_from_slice(&TELEMETRY_SHAPE_EXTENT.to_le_bytes());
    page[24..28].copy_from_slice(&crate::sdf::shape_atlas::SHAPE_BLOCK_WIDTH.to_le_bytes());
    page[28..32].copy_from_slice(&crate::sdf::shape_atlas::SHAPE_BLOCK_HEIGHT.to_le_bytes());
    page[32..36].copy_from_slice(&crate::sdf::shape_atlas::SHAPE_CHANNELS.to_le_bytes());
    // Fully inside the shape everywhere, so the executor writes pixels.
    for texel in 0..extent * extent {
        let offset = crate::sdf::shape_atlas::SHAPE_PAGE_HEADER_BYTES + texel * 2;
        page[offset] = 255;
        page[offset + 1] = 255;
    }
    let page_name = "shape-page-000.rg8swz";
    std::fs::write(root.join(page_name), &page).expect("write shape page");
    let manifest = crate::sdf::shape_atlas::ShapeSdfAtlasManifest {
        schema: crate::sdf::shape_atlas::SHAPE_ATLAS_MANIFEST_SCHEMA.into(),
        generator_contract: crate::sdf::shape_atlas::SHAPE_ATLAS_GENERATOR_CONTRACT.into(),
        pixel_format: crate::sdf::shape_atlas::SHAPE_ATLAS_PIXEL_FORMAT.into(),
        pages: vec![crate::sdf::shape_atlas::ShapeSdfAtlasPageManifest {
            file: page_name.into(),
            width: TELEMETRY_SHAPE_EXTENT,
            height: TELEMETRY_SHAPE_EXTENT,
            file_sha256: hex::encode(sha2::Sha256::digest(&page)),
        }],
        shapes: vec![crate::sdf::shape_atlas::ShapeSdfAtlasEntry {
            shape_id: TELEMETRY_SHAPE_ID,
            asset_key: asset_key.into(),
            source_sha256: "00".repeat(32),
            source_rg8_sha256: identity_rg8_sha256,
            page: 0,
            rect: [0, 0, TELEMETRY_SHAPE_EXTENT, TELEMETRY_SHAPE_EXTENT],
            source_size: [identity_width as u32, identity_height as u32],
        }],
        generation: crate::sdf::shape_atlas::ShapeSdfAtlasGenerationReport {
            requested_shape_count: 1,
            packed_shape_count: 1,
            failed_shape_count: 0,
            page_width: TELEMETRY_SHAPE_EXTENT,
            page_height: TELEMETRY_SHAPE_EXTENT,
            gutter: 0,
            failures: Vec::new(),
        },
    };
    let manifest_path = root.join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec(&manifest).expect("serialize shape manifest"),
    )
    .expect("write shape manifest");
    std::sync::Arc::new(
        crate::sdf::shape_atlas::MappedShapeSdfAtlas::open(&manifest_path)
            .expect("open shape atlas"),
    )
}

/// The ordered layer raster must report what it did, not just what it drew:
/// the atlases it was allowed to sample, the runs it executed, and a
/// realtime-EDT account whose totals agree with the per-glyph records they
/// were summed from.
///
/// The corpus is shape-only, so no oversized glyph is generated and the
/// three realtime-EDT totals are legitimately zero; the assertion is the
/// invariant that each total equals the sum over `realtime_edt_glyphs`,
/// which holds whether or not any glyph was generated.
#[test]
fn ordered_layer_raster_reports_atlas_identity_runs_and_realtime_edt_totals() {
    let root = tempfile::tempdir().expect("shape telemetry tempdir");
    let extent = TELEMETRY_SHAPE_EXTENT;
    let pixels = vec![255u8; (extent * extent * 4) as usize];
    let encoded =
        crate::codec::png::encode_rgba(extent, extent, &pixels).expect("encode shape source");
    let asset_key = format!("custom_profile/shape/{TELEMETRY_SHAPE_FILE}");
    let assets = std::sync::Arc::new(crate::assets::AssetStore::new(4));
    assets.put(asset_key.clone(), encoded);
    let identity = assets
        .shape_sdf_source_identity_for_key(&asset_key)
        .expect("shape source identity");

    let atlas = telemetry_shape_atlas(
        root.path(),
        &asset_key,
        identity.width,
        identity.height,
        identity.rg8_sha256,
    );
    let renderer =
        crate::renderer::CustomProfileRenderer::new(std::sync::Arc::new(SingleShapeProvider))
            .with_assets(std::sync::Arc::clone(&assets))
            .with_shape_sdf_atlas(atlas);

    let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
        "shapes": [{
            "objectData": {
                "layer": 0,
                "lock": false,
                "position": {"x": 0.0, "y": 0.0, "z": 0.0},
                "rotation": {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0},
                "scale": {"x": 1.0, "y": 1.0, "z": 1.0},
                "visible": true
            },
            "alpha": 1.0,
            "colorId": 1,
            "id": TELEMETRY_SHAPE_ID,
            "outlineAlpha": 0.0,
            "outlineColorId": 1,
            "outlineSize": 0.0
        }]
    }))
    .expect("build shape-only card");

    let groups: Vec<AnimationRasterGroup> = vec![(
        None,
        vec![(sekai_profile_core::AuthoredElementKind::Shape, 0)],
    )];
    let config = crate::render_config::RenderConfig {
        executor: crate::render_config::SdfExecutor::Scalar,
        ..crate::render_config::RenderConfig::default()
    };
    let md = renderer.masterdata();
    let (layers, _bytes, _scratch, telemetry) = rasterize_animation_groups(
        &renderer,
        &card,
        None,
        &md,
        Some(assets.as_ref()),
        None,
        &std::collections::BTreeMap::new(),
        &groups,
        &config,
        crate::render_object::MappedRenderObjectStore::empty(),
        usize::MAX,
    )
    .expect("rasterize the shape layer");
    assert_eq!(layers.len(), 1, "the shape group produced one raster layer");

    let telemetry = telemetry.expect("rasterizing layers reports telemetry");

    assert!(
        !telemetry.atlas_identities.is_empty(),
        "the layer raster recorded no atlas identity",
    );
    assert!(
        telemetry
            .atlas_identities
            .iter()
            .all(|identity| identity.usage == ANIMATION_LAYER_RASTER_USAGE),
        "atlas identities carry the animation layer raster usage",
    );
    assert!(
        telemetry
            .atlas_identities
            .iter()
            .any(|identity| identity.family == "shape"),
        "the enabled shape atlas is among the recorded identities",
    );

    assert!(
        telemetry.work.element_run_count > 0,
        "the layer raster executed no element run",
    );

    let glyph_substitutions: u64 = telemetry
        .realtime_edt_glyphs
        .iter()
        .map(|glyph| glyph.substitution_count)
        .sum();
    let glyph_page_bytes: u64 = telemetry
        .realtime_edt_glyphs
        .iter()
        .map(|glyph| glyph.page_bytes)
        .sum();
    let glyph_generation_ns: u64 = telemetry
        .realtime_edt_glyphs
        .iter()
        .map(|glyph| glyph.generation_ns)
        .sum();
    assert_eq!(
        telemetry.work.realtime_edt_glyph_count, glyph_substitutions,
        "the glyph total disagrees with the per-glyph records",
    );
    assert_eq!(
        telemetry.bytes.realtime_edt_page_bytes, glyph_page_bytes,
        "the page-byte total disagrees with the per-glyph records",
    );
    assert_eq!(
        telemetry.timings.realtime_edt_generation_ns, glyph_generation_ns,
        "the generation-time total disagrees with the per-glyph records",
    );
}

#[test]
fn every_encoder_is_byte_deterministic_for_one_frame_sequence() {
    // Animation artifacts are compared byte-for-byte across engine
    // revisions, so no encoder may embed a wall-clock stamp, an address,
    // or any other per-run value.
    let spec = AnimationEncodeSpec {
        width: 16,
        height: 8,
        fps: 12,
        frame_count: 3,
        looped: true,
        output_budget_bytes: 1_000_000,
        gif_quality: 80,
    };
    let frame_at = |index: u32| {
        let shade = (index * 40 + 15) as u8;
        Ok(vec![shade, 255 - shade, shade / 2, 255].repeat(16 * 8))
    };
    for format in [
        AnimationFormat::Gif,
        AnimationFormat::Webp,
        AnimationFormat::Apng,
        AnimationFormat::Mp4,
    ] {
        let first = encode_rgba_frames(format, &spec, frame_at).unwrap();
        let second = encode_rgba_frames(format, &spec, frame_at).unwrap();
        validate_magic(format, &first.data).unwrap();
        assert_eq!(
            first.data, second.data,
            "{format:?} encoded two different byte streams for one frame sequence",
        );
    }
}

#[test]
fn all_animation_encoders_stream_valid_two_frame_artifacts() {
    let spec = AnimationEncodeSpec {
        width: 2,
        height: 2,
        fps: 20,
        frame_count: 2,
        looped: true,
        output_budget_bytes: 1_000_000,
        gif_quality: 80,
    };
    for format in [
        AnimationFormat::Gif,
        AnimationFormat::Webp,
        AnimationFormat::Apng,
    ] {
        let encoded = encode_rgba_frames(format, &spec, |index| {
            Ok(if index == 0 {
                [255, 0, 0, 255].repeat(4)
            } else {
                [0, 0, 255, 255].repeat(4)
            })
        })
        .unwrap();
        validate_magic(format, &encoded.data).unwrap();
        assert_eq!(encoded.frame_count, 2);
        assert_eq!(encoded.duration_ms, 100);
        assert!(encoded.data.len() < spec.output_budget_bytes);
        match format {
            AnimationFormat::Gif => {
                let mut options = gif::DecodeOptions::new();
                options.set_color_output(gif::ColorOutput::RGBA);
                let mut decoder = options.read_info(Cursor::new(&encoded.data)).unwrap();
                let mut frames = 0;
                while decoder.read_next_frame().unwrap().is_some() {
                    frames += 1;
                }
                assert_eq!(frames, 2);
            }
            AnimationFormat::Webp => {
                let decoder = webp_animation::Decoder::new(&encoded.data).unwrap();
                assert_eq!(decoder.into_iter().count(), 2);
            }
            AnimationFormat::Apng => {
                let marker = encoded
                    .data
                    .windows(4)
                    .position(|chunk| chunk == b"acTL")
                    .unwrap();
                assert_eq!(
                    u32::from_be_bytes(encoded.data[marker + 4..marker + 8].try_into().unwrap()),
                    2
                );
            }
            AnimationFormat::Mp4 => unreachable!("MP4 is tested with the external encoder"),
        }
    }
}

#[test]
fn direct_x264_streams_a_valid_mp4() {
    let spec = AnimationEncodeSpec {
        width: 16,
        height: 8,
        fps: 12,
        frame_count: 2,
        looped: false,
        output_budget_bytes: 1_000_000,
        gif_quality: 80,
    };
    let frames = [
        [255, 0, 0, 255].repeat(16 * 8),
        [0, 255, 0, 255].repeat(16 * 8),
    ];
    let encoded = encode_rgba_frames(AnimationFormat::Mp4, &spec, |index| {
        Ok(frames[index as usize].clone())
    })
    .unwrap();
    validate_mp4_container(&encoded.data).unwrap();
    assert_eq!((encoded.width, encoded.height, encoded.fps), (16, 8, 12));
    if std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
    {
        use std::io::Write;
        let mut decoder = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i", "pipe:0", "-f", "null", "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        decoder
            .stdin
            .take()
            .unwrap()
            .write_all(&encoded.data)
            .unwrap();
        let output = decoder.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn direct_x264_accepts_constant_macroblock_hints() {
    let spec = AnimationEncodeSpec {
        width: 16,
        height: 16,
        fps: 12,
        frame_count: 2,
        looped: false,
        output_budget_bytes: 1_000_000,
        gif_quality: 80,
    };
    let frame = [17, 91, 203, 255].repeat(16 * 16);
    let mut telemetry = AnimationEncodeTelemetry::default();
    let data = encode_mp4_into(
        &spec,
        frame.len(),
        &H264EncoderConfig::default(),
        &mut telemetry,
        |index, rgba| {
            *rgba = frame.clone();
            Ok(if index == 0 {
                FrameCompositeUpdate::Full
            } else {
                FrameCompositeUpdate::Reused
            })
        },
    )
    .unwrap();
    validate_mp4_container(&data).unwrap();
    assert_eq!(telemetry.h264_mbinfo_total_blocks, 2);
    assert_eq!(telemetry.h264_mbinfo_constant_blocks, 1);
}

#[test]
fn gif_retained_deltas_remove_the_second_frame_pass_without_changing_bytes() {
    let spec = AnimationEncodeSpec {
        width: 4,
        height: 2,
        fps: 20,
        frame_count: 4,
        looped: true,
        output_budget_bytes: 1_000_000,
        gif_quality: 80,
    };
    let mut frames = vec![[255, 0, 0, 255].repeat(8)];
    let mut second = frames[0].clone();
    second[4..8].copy_from_slice(&[0, 255, 0, 255]);
    frames.push(second.clone());
    frames.push(second.clone());
    let mut fourth = second;
    fourth[24..28].copy_from_slice(&[0, 0, 255, 255]);
    frames.push(fourth);

    let (two_pass, two_pass_telemetry) = encode_rgba_frames_with_h264_and_telemetry(
        AnimationFormat::Gif,
        &spec,
        &H264EncoderConfig::default(),
        0,
        |index| Ok(frames[index as usize].clone()),
    )
    .unwrap();
    let (retained, retained_telemetry) = encode_rgba_frames_with_h264_and_telemetry(
        AnimationFormat::Gif,
        &spec,
        &H264EncoderConfig::default(),
        1_000_000,
        |index| Ok(frames[index as usize].clone()),
    )
    .unwrap();

    assert_eq!(retained.data, two_pass.data);
    assert!(two_pass_telemetry.gif_second_frame_pass);
    assert_eq!(two_pass_telemetry.frame_callback_calls, 8);
    assert!(!retained_telemetry.gif_second_frame_pass);
    assert_eq!(retained_telemetry.frame_callback_calls, 4);
    assert!(retained_telemetry.gif_retained_frame_bytes > 0);
}

#[test]
fn gif_preserves_source_palette_without_outline_color_bleed() {
    let source = vec![
        239, 207, 211, 255, 255, 238, 240, 255, 202, 105, 145, 255, 255, 95, 145, 255, 207, 198,
        209, 255, 67, 87, 190, 255,
    ];
    let spec = AnimationEncodeSpec {
        width: 6,
        height: 1,
        fps: 20,
        frame_count: 1,
        looped: false,
        output_budget_bytes: 1_000_000,
        gif_quality: 80,
    };
    let encoded = encode_rgba_frames(AnimationFormat::Gif, &spec, |_| Ok(source.clone())).unwrap();
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(Cursor::new(&encoded.data)).unwrap();
    let decoded = decoder.read_next_frame().unwrap().unwrap().buffer.to_vec();
    let max_channel_error = source
        .iter()
        .zip(decoded.iter())
        .enumerate()
        .filter(|(index, _)| index % 4 != 3)
        .map(|(_, (&expected, &actual))| expected.abs_diff(actual))
        .max()
        .unwrap();
    assert!(
        max_channel_error <= 2,
        "GIF color error was {max_channel_error}"
    );
}

#[test]
fn gif_delta_writes_opaque_white_over_a_moving_layers_old_position() {
    let white = [255, 255, 255, 255];
    let red = [255, 0, 0, 255];
    let frames = [[red, white, white].concat(), [white, white, red].concat()];
    let spec = AnimationEncodeSpec {
        width: 3,
        height: 1,
        fps: 20,
        frame_count: 2,
        looped: true,
        output_budget_bytes: 1_000_000,
        gif_quality: 80,
    };
    let encoded = encode_rgba_frames(AnimationFormat::Gif, &spec, |index| {
        Ok(frames[index as usize].clone())
    })
    .unwrap();

    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(Cursor::new(&encoded.data)).unwrap();
    decoder.read_next_frame().unwrap().unwrap();
    let delta = decoder.read_next_frame().unwrap().unwrap();

    assert_eq!((delta.left, delta.width), (0, 3));
    assert_eq!(&delta.buffer[0..4], &white);
    assert_eq!(
        delta.buffer[7], 0,
        "unchanged middle pixel stays transparent"
    );
    assert_eq!(&delta.buffer[8..12], &red);
}

#[test]
fn gif_artifact_keeps_an_initially_off_canvas_dynamic_layer_and_erases_its_old_position() {
    // A dynamic layer that starts fully outside the canvas, travels across
    // it, and leaves on the other side. Frame 0 must stay blank, frame 1
    // must show the layer, and frame 2 must erase the position it left —
    // the dirty-region composite has to repaint vacated pixels, not just
    // the ones the layer now covers.
    let layers = vec![RasterLayer {
        dynamic_layer_id: Some(sekai_profile_core::StableId(7)),
        content: RasterLayerContent::Pixels([255, 0, 0, 255].repeat(2)),
        x: 0.0,
        y: 0.0,
        width: 2,
        height: 1,
    }];
    let state = |x: f32| {
        let destination = FrameRect::from_xywh(x, 0.0, 2.0, 1.0);
        LayerFrameState {
            visible: true,
            destination,
            pixel_bounds: pixel_bounds_for_destination(destination, 4, 1),
        }
    };
    let states = [state(-4.0), state(1.0), state(5.0)];
    assert!(states[0].pixel_bounds.is_empty());
    assert!(!states[1].pixel_bounds.is_empty());
    assert!(states[2].pixel_bounds.is_empty());

    let spec = AnimationEncodeSpec {
        width: 4,
        height: 1,
        fps: 20,
        frame_count: states.len() as u32,
        looped: false,
        output_budget_bytes: 1_000_000,
        gif_quality: 80,
    };
    let mut compositor = AnimationFrameCompositor::new(4, 1, 1.0).unwrap();
    let mut rgba = vec![0; 4 * 4];
    let encoded = encode_rgba_frames(AnimationFormat::Gif, &spec, |index| {
        compositor
            .composite_states(index, &layers, vec![states[index as usize]], &mut rgba)
            .unwrap();
        Ok(rgba.clone())
    })
    .unwrap();

    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(Cursor::new(&encoded.data)).unwrap();
    let white = [255, 255, 255, 255];
    let red = [255, 0, 0, 255];
    let mut canvas = white.repeat(4);
    let mut decoded = Vec::new();
    while let Some(frame) = decoder.read_next_frame().unwrap() {
        for y in 0..usize::from(frame.height) {
            for x in 0..usize::from(frame.width) {
                let source = (y * usize::from(frame.width) + x) * 4;
                if frame.buffer[source + 3] == 0 {
                    continue;
                }
                let destination =
                    ((usize::from(frame.top) + y) * 4 + usize::from(frame.left) + x) * 4;
                canvas[destination..destination + 4]
                    .copy_from_slice(&frame.buffer[source..source + 4]);
            }
        }
        decoded.push(canvas.clone());
    }

    assert_eq!(decoded.len(), 3);
    assert_eq!(decoded[0], white.repeat(4));
    assert_eq!(&decoded[1][4..8], &red);
    assert_eq!(&decoded[1][8..12], &red);
    assert_eq!(&decoded[2][4..8], &white);
    assert_eq!(&decoded[2][8..12], &white);
}

#[test]
fn gif_palette_covers_colors_introduced_after_the_first_frame() {
    let frames = [
        [255, 0, 0, 255].repeat(4),
        [0, 255, 0, 255].repeat(4),
        [0, 0, 255, 255].repeat(4),
    ];
    let spec = AnimationEncodeSpec {
        width: 2,
        height: 2,
        fps: 20,
        frame_count: frames.len() as u32,
        looped: false,
        output_budget_bytes: 1_000_000,
        gif_quality: 80,
    };
    let encoded = encode_rgba_frames(AnimationFormat::Gif, &spec, |index| {
        Ok(frames[index as usize].clone())
    })
    .unwrap();
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(Cursor::new(&encoded.data)).unwrap();
    for expected in [[255, 0, 0], [0, 255, 0], [0, 0, 255]] {
        let decoded = decoder.read_next_frame().unwrap().unwrap();
        assert!(decoded.buffer.chunks_exact(4).all(|pixel| {
            pixel[..3]
                .iter()
                .zip(expected)
                .all(|(&actual, expected)| actual.abs_diff(expected) <= 2)
        }));
    }
}

#[test]
fn gif_delay_uses_rational_centisecond_boundaries() {
    assert_eq!(
        (0..8)
            .map(|index| gif_delay_centiseconds(index, 15))
            .collect::<Vec<_>>(),
        vec![7, 6, 7, 7, 6, 7, 7, 6]
    );
    assert_eq!(
        (0..6)
            .map(|index| gif_delay_centiseconds(index, 60))
            .collect::<Vec<_>>(),
        vec![2, 1, 2, 2, 1, 2]
    );
}

#[test]
fn gif_delta_encodes_an_unchanged_frame_as_one_transparent_index() {
    let rgba = vec![12, 34, 56, 255];
    let palette = vec![0; 256 * 3];
    let mut lookup = GifPaletteLookup::new(0, 255);
    let mut indexed = vec![0];
    assert_eq!(
        gif_delta_indexed(&rgba, &rgba, 1, 1, &palette, &mut lookup, &mut indexed),
        (0, 0, 1, 1, vec![255])
    );
}

#[test]
fn gif_changed_bounds_matches_scalar_across_zmm_tail() {
    let width = 19u32;
    let height = 2u32;
    let previous = vec![255u8; width as usize * height as usize * 4];
    let mut current = previous.clone();
    for (x, y, rgba) in [
        (17u32, 0u32, [1u8, 2, 3, 255]),
        (2u32, 1u32, [9u8, 8, 7, 255]),
    ] {
        let offset = (y * width + x) as usize * 4;
        current[offset..offset + 4].copy_from_slice(&rgba);
    }
    let expected = Some((2, 0, 17, 1));
    assert_eq!(
        gif_changed_bounds_scalar(&previous, &current, width, height),
        expected
    );
    assert_eq!(
        gif_changed_bounds(&previous, &current, width, height),
        expected
    );
    assert_eq!(
        gif_changed_bounds_scalar(&previous, &previous, width, height),
        None
    );

    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx512f") {
        assert_eq!(
            unsafe { gif_changed_bounds_avx512(&previous, &current, width, height) },
            expected
        );
    }
}

#[test]
#[cfg(target_arch = "x86_64")]
fn avx512_gif_palette_distance_matches_scalar() {
    if !std::arch::is_x86_feature_detected!("avx512f") {
        return;
    }
    let mut palette = Vec::with_capacity(255 * 3);
    for index in 0..255u32 {
        palette.extend_from_slice(&[
            index.wrapping_mul(73) as u8,
            index.wrapping_mul(151).wrapping_add(19) as u8,
            index.wrapping_mul(211).wrapping_add(37) as u8,
        ]);
    }
    let mut lookup = GifPaletteLookup::new(0, 255);
    lookup.set_palette(&palette);
    for index in 0..4096u32 {
        let pixel = [
            index.wrapping_mul(17) as u8,
            index.wrapping_mul(43).wrapping_add(7) as u8,
            index.wrapping_mul(97).wrapping_add(31) as u8,
            255,
        ];
        let scalar = nearest_gif_palette_index_scalar(&pixel, &palette, 255);
        let avx512 = unsafe { nearest_gif_palette_index_avx512(&pixel, &lookup) };
        assert_eq!(avx512, scalar, "pixel={pixel:?}");
    }
}

#[test]
fn fast_gif_lzw_roundtrips_dictionary_resets() {
    let width = 320u16;
    let height = 200u16;
    let pixel_count = width as usize * height as usize;
    let mut pixels = Vec::with_capacity(pixel_count);
    let mut value = 0x1234_5678u32;
    for _ in 0..pixel_count {
        value = value.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        pixels.push((value >> 24) as u8);
    }
    let mut palette = Vec::with_capacity(256 * 3);
    for index in 0..=255u8 {
        palette.extend_from_slice(&[index, index, index]);
    }
    let mut lzw = FastGifLzwEncoder::new();
    for _ in 0..2 {
        let encoded_lzw = lzw.encode(&pixels);
        let mut output = Vec::new();
        {
            let mut encoder = gif::Encoder::new(&mut output, width, height, &palette).unwrap();
            let mut frame =
                gif::Frame::from_indexed_pixels(width, height, pixels.clone(), Some(255));
            frame.buffer = std::borrow::Cow::Owned(encoded_lzw);
            encoder.write_lzw_pre_encoded_frame(&frame).unwrap();
        }
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::Indexed);
        let mut decoder = options.read_info(output.as_slice()).unwrap();
        let decoded = decoder.read_next_frame().unwrap().unwrap();
        assert_eq!(decoded.buffer.as_ref(), pixels.as_slice());
    }
}

#[test]
fn apng_delta_frames_keep_the_previous_canvas() {
    let spec = AnimationEncodeSpec {
        width: 2,
        height: 1,
        fps: 20,
        frame_count: 2,
        looped: false,
        output_budget_bytes: 1_000_000,
        gif_quality: 80,
    };
    let frames = [
        vec![255, 0, 0, 255, 0, 0, 0, 255],
        vec![255, 0, 0, 255, 0, 255, 0, 255],
    ];
    let encoded = encode_rgba_frames(AnimationFormat::Apng, &spec, |index| {
        Ok(frames[index as usize].clone())
    })
    .unwrap();
    let mut controls = Vec::new();
    let mut offset = 8usize;
    while offset + 12 <= encoded.data.len() {
        let length =
            u32::from_be_bytes(encoded.data[offset..offset + 4].try_into().unwrap()) as usize;
        let kind = &encoded.data[offset + 4..offset + 8];
        let payload_start = offset + 8;
        let payload_end = payload_start + length;
        assert!(payload_end + 4 <= encoded.data.len());
        if kind == b"fcTL" {
            controls.push(&encoded.data[payload_start..payload_end]);
        }
        offset = payload_end + 4;
    }
    assert_eq!(controls.len(), 2);
    assert_eq!(
        controls[1][24], 0,
        "delta frame must use APNG_DISPOSE_OP_NONE"
    );
    assert_eq!(
        controls[1][25], 0,
        "delta frame must use APNG_BLEND_OP_SOURCE"
    );
}

#[test]
fn frame_size_and_output_budget_fail_closed() {
    let spec = AnimationEncodeSpec {
        width: 2,
        height: 2,
        fps: 20,
        frame_count: 1,
        looped: false,
        output_budget_bytes: 1,
        gif_quality: 80,
    };
    assert!(
        encode_rgba_frames(AnimationFormat::Apng, &spec, |_| Ok(vec![0; 15]))
            .unwrap_err()
            .contains("expected 16")
    );
    assert!(
        encode_rgba_frames(AnimationFormat::Apng, &spec, |_| Ok(vec![0; 16]))
            .unwrap_err()
            .contains("OUTPUT_BUDGET_EXCEEDED")
    );
}

#[test]
fn ready_artifact_validation_rejects_path_mime_size_and_magic_mismatches() {
    let spec = AnimationEncodeSpec {
        width: 1,
        height: 1,
        fps: 20,
        frame_count: 1,
        looped: false,
        output_budget_bytes: 4096,
        gif_quality: 80,
    };
    let gif = encode_rgba_frames(AnimationFormat::Gif, &spec, |_| Ok(vec![255, 0, 0, 255]))
        .unwrap()
        .data;
    validate_encoded_artifact(
        AnimationFormat::Gif,
        "page.gif",
        "image/gif",
        gif.len(),
        gif.len(),
        &gif,
    )
    .unwrap();
    let truncated = &gif[..gif.len() - 1];
    for result in [
        validate_encoded_artifact(
            AnimationFormat::Gif,
            "page.apng",
            "image/gif",
            gif.len(),
            gif.len(),
            &gif,
        ),
        validate_encoded_artifact(
            AnimationFormat::Gif,
            "page.gif",
            "image/png",
            gif.len(),
            gif.len(),
            &gif,
        ),
        validate_encoded_artifact(
            AnimationFormat::Gif,
            "page.gif",
            "image/gif",
            gif.len() - 1,
            gif.len(),
            &gif,
        ),
        validate_encoded_artifact(
            AnimationFormat::Gif,
            "page.gif",
            "image/gif",
            10,
            10,
            b"\x89PNG\r\n\x1a\n00",
        ),
        validate_encoded_artifact(
            AnimationFormat::Gif,
            "page.gif",
            "image/gif",
            truncated.len(),
            truncated.len(),
            truncated,
        ),
    ] {
        assert!(result.is_err());
    }
}

#[test]
fn immutable_presets_resolve_auto_without_format_guessing() {
    let qq = resolve_preset("qq", None).unwrap();
    assert_eq!(qq.preset, AnimationPreset::QqV2);
    assert_eq!(qq.format, AnimationFormat::Gif);
    assert_eq!(qq.maximum_ticks, 480);
    assert_eq!(qq.maximum_long_edge, 1830);
    assert_eq!(qq.fps, 12);
    assert_eq!((align_to_8(1830), align_to_8(812)), (1832, 816));
    let qq_v1 = resolve_preset("qq-v1", None).unwrap();
    assert_eq!(qq_v1.preset, AnimationPreset::QqV1);
    assert_eq!(qq_v1.maximum_long_edge, 1280);
    let archive = resolve_preset("archive", None).unwrap();
    assert_eq!(archive.format, AnimationFormat::Apng);
    assert_eq!(
        resolve_preset("qq", Some(AnimationFormat::Apng))
            .unwrap()
            .format,
        AnimationFormat::Apng
    );
    assert!(resolve_preset("future", None)
        .unwrap_err()
        .contains("ANIMATION_PRESET_UNSUPPORTED"));
}

#[test]
fn qq_v1_budget_steps_are_immutable_and_ordered() {
    let preset = resolve_preset("qq-v1", None).unwrap();
    let steps = animation_budget_steps(&preset);
    assert_eq!(
        steps
            .iter()
            .map(|step| (step.maximum_long_edge, step.fps, step.gif_quality))
            .collect::<Vec<_>>(),
        vec![
            (1280, 20, 80),
            (1280, 20, 65),
            (1280, 15, 65),
            (960, 15, 65)
        ]
    );
    assert!(steps
        .iter()
        .all(|step| step.output_budget_bytes == 10 * 1024 * 1024));

    let web = resolve_preset("web", None).unwrap();
    assert_eq!(animation_budget_steps(&web).len(), 1);
}

#[test]
fn budget_ladder_retries_only_output_budget_failures_in_order() {
    let preset = resolve_preset("qq-v1", None).unwrap();
    let mut attempted = Vec::new();
    let (value, step, attempts) = run_animation_budget_ladder(&preset, |step| {
        attempted.push(step.index);
        if step.index < 3 {
            Err(format!("OUTPUT_BUDGET_EXCEEDED: step {}", step.index))
        } else {
            Ok("encoded")
        }
    })
    .unwrap();
    assert_eq!(value, "encoded");
    assert_eq!(step.index, 3);
    assert_eq!(attempts, 4);
    assert_eq!(attempted, vec![0, 1, 2, 3]);

    let error = run_animation_budget_ladder::<(), _>(&preset, |_| {
        Err("ANIMATION_ENCODE_FAILED: corrupt input".into())
    })
    .unwrap_err();
    assert!(error.contains("ANIMATION_ENCODE_FAILED"));
}

#[test]
fn static_passthrough_never_becomes_animation_work() {
    assert_eq!(
        plan_page_execution(false, "passthrough").unwrap(),
        PageExecution::StaticPassthrough
    );
    assert_eq!(
        plan_page_execution(false, "wrap").unwrap(),
        PageExecution::StaticWrap
    );
    assert_eq!(
        plan_page_execution(true, "passthrough").unwrap(),
        PageExecution::Animation
    );
}

#[test]
fn loop_policy_only_repeats_periodic_programs() {
    use sekai_profile_core::TimelineDescriptor;
    let at_zero = |period_ticks| TimelineDescriptor {
        loop_start_tick: 0,
        period_ticks,
    };

    assert_eq!(
        animation_loop_period_ticks([at_zero(2), at_zero(3)], 6),
        Some(6)
    );
    assert_eq!(
        animation_loop_period_ticks([at_zero(2), at_zero(3)], 5),
        None
    );
    assert_eq!(
        animation_loop_period_ticks(
            [
                at_zero(2),
                TimelineDescriptor {
                    loop_start_tick: 1,
                    period_ticks: 2,
                },
            ],
            8,
        ),
        None
    );
    assert_eq!(animation_loop_period_ticks([], 8), None);
}

#[test]
fn running_tick_window_never_exceeds_the_preset_duration() {
    assert_eq!(plan_running_ticks(6, 20), vec![0, 3]);
    assert_eq!(plan_running_ticks(10, 24), vec![0, 2, 5, 7]);
    assert!(plan_running_ticks(6, 20).iter().all(|tick| *tick < 6));
}

#[test]
fn gif_manifest_duration_uses_encoded_centisecond_delays() {
    let spec = AnimationEncodeSpec {
        width: 1,
        height: 1,
        fps: 15,
        frame_count: 8,
        looped: false,
        output_budget_bytes: 4096,
        gif_quality: 80,
    };
    let encoded =
        encode_rgba_frames(AnimationFormat::Gif, &spec, |_| Ok(vec![255, 0, 0, 255])).unwrap();
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::Indexed);
    let mut decoder = options.read_info(encoded.data.as_slice()).unwrap();
    let mut duration_ms = 0u64;
    while let Some(frame) = decoder.read_next_frame().unwrap() {
        duration_ms += frame.delay as u64 * 10;
    }
    assert_eq!(encoded.duration_ms, duration_ms);
    assert_eq!(duration_ms, 530);
}

#[test]
fn webp_container_loop_count_matches_manifest() {
    for looped in [false, true] {
        let spec = AnimationEncodeSpec {
            width: 1,
            height: 1,
            fps: 20,
            frame_count: 2,
            looped,
            output_budget_bytes: 4096,
            gif_quality: 80,
        };
        let encoded = encode_rgba_frames(AnimationFormat::Webp, &spec, |index| {
            Ok(if index == 0 {
                vec![255, 0, 0, 255]
            } else {
                vec![0, 0, 255, 255]
            })
        })
        .unwrap();
        assert_eq!(
            webp_loop_count(&encoded.data),
            Some(if looped { 0 } else { 1 })
        );
        assert_eq!(encoded.looped, looped);
    }
}

fn webp_loop_count(data: &[u8]) -> Option<u16> {
    let mut cursor = 12usize;
    while cursor.checked_add(8)? <= data.len() {
        let name = &data[cursor..cursor + 4];
        let size = u32::from_le_bytes(data[cursor + 4..cursor + 8].try_into().ok()?) as usize;
        let payload = cursor.checked_add(8)?;
        let end = payload.checked_add(size)?;
        if end > data.len() {
            return None;
        }
        if name == b"ANIM" && size >= 6 {
            return Some(u16::from_le_bytes(
                data[payload + 4..payload + 6].try_into().ok()?,
            ));
        }
        cursor = end + (size & 1);
    }
    None
}

#[test]
fn gif_rejects_frames_wider_or_taller_than_its_16_bit_screen() {
    for (width, height) in [(65_536u32, 1u32), (1, 65_536)] {
        let spec = AnimationEncodeSpec {
            width,
            height,
            fps: 20,
            frame_count: 1,
            looped: false,
            output_budget_bytes: usize::MAX,
            gif_quality: 80,
        };
        let error = encode_rgba_frames(AnimationFormat::Gif, &spec, |_| {
            Ok([255, 0, 0, 255].repeat(65_536))
        })
        .expect_err("a GIF extent beyond 16 bits is rejected");
        assert!(error.contains("65535"), "{width}x{height}: {error}");
    }

    let widest = AnimationEncodeSpec {
        width: 65_535,
        height: 1,
        fps: 20,
        frame_count: 1,
        looped: false,
        output_budget_bytes: usize::MAX,
        gif_quality: 80,
    };
    let encoded = encode_rgba_frames(AnimationFormat::Gif, &widest, |_| {
        Ok([255, 0, 0, 255].repeat(65_535))
    })
    .expect("the widest GIF encodes");
    let mut decoder = gif::DecodeOptions::new()
        .read_info(Cursor::new(&encoded.data))
        .expect("the widest GIF decodes");
    assert_eq!(decoder.width(), 65_535);
    let frame = decoder
        .read_next_frame()
        .expect("the widest GIF frame decodes")
        .expect("the widest GIF has a frame");
    assert_eq!(frame.width, 65_535);
}

#[test]
fn frame_byte_size_overflow_is_reported_instead_of_wrapping() {
    let spec = AnimationEncodeSpec {
        width: u32::MAX,
        height: u32::MAX,
        fps: 20,
        frame_count: 1,
        looped: false,
        output_budget_bytes: usize::MAX,
        gif_quality: 80,
    };
    for format in [
        AnimationFormat::Gif,
        AnimationFormat::Webp,
        AnimationFormat::Apng,
        AnimationFormat::Mp4,
    ] {
        let mut requested = false;
        let error = encode_rgba_frames(format, &spec, |_| {
            requested = true;
            Ok(Vec::new())
        })
        .expect_err("an unrepresentable frame size is rejected");
        assert!(error.contains("overflow"), "{format:?}: {error}");
        assert!(!requested, "{format:?} asked for a frame it cannot hold");
    }
}

fn assert_gif_paths_agree(spec: &AnimationEncodeSpec, frames: &[Vec<u8>]) {
    let (streamed, streamed_telemetry) = encode_rgba_frames_with_h264_and_telemetry(
        AnimationFormat::Gif,
        spec,
        &H264EncoderConfig::default(),
        0,
        |index| Ok(frames[index as usize].clone()),
    )
    .expect("the streamed pass encodes");
    let (retained, retained_telemetry) = encode_rgba_frames_with_h264_and_telemetry(
        AnimationFormat::Gif,
        spec,
        &H264EncoderConfig::default(),
        usize::MAX,
        |index| Ok(frames[index as usize].clone()),
    )
    .expect("the retained pass encodes");
    assert!(streamed_telemetry.gif_second_frame_pass);
    assert!(!retained_telemetry.gif_second_frame_pass);
    assert_eq!(
        retained.data, streamed.data,
        "the GIF bytes depend on which memory path encoded them"
    );
}

#[test]
fn gif_retained_and_streamed_deltas_agree_when_only_alpha_changes() {
    // An alpha-only change keeps the RGB key, so the palette index stays.
    let spec = AnimationEncodeSpec {
        width: 2,
        height: 1,
        fps: 20,
        frame_count: 2,
        looped: true,
        output_budget_bytes: usize::MAX,
        gif_quality: 80,
    };
    assert_gif_paths_agree(
        &spec,
        &[
            vec![255, 0, 0, 255, 0, 0, 255, 255],
            vec![255, 0, 0, 255, 0, 0, 255, 200],
        ],
    );
}

#[test]
fn gif_retained_and_streamed_deltas_agree_when_quantized_colors_share_an_entry() {
    // More than 255 opaque colors go through the quantizer, which maps
    // neighbouring colors to one palette entry.
    let (width, height) = (32u32, 16u32);
    let first = (0..width * height)
        .flat_map(|index| {
            [
                (index % width * 8) as u8,
                (index / width * 16) as u8,
                128,
                255,
            ]
        })
        .collect::<Vec<_>>();
    let mut second = first.clone();
    second[..4].copy_from_slice(&[1, 0, 128, 255]);
    let spec = AnimationEncodeSpec {
        width,
        height,
        fps: 20,
        frame_count: 2,
        looped: true,
        output_budget_bytes: usize::MAX,
        gif_quality: 80,
    };
    assert_gif_paths_agree(&spec, &[first, second]);
}

#[test]
fn unsupported_static_policy_is_rejected_for_animated_pages_too() {
    for animated in [false, true] {
        assert!(
            plan_page_execution(animated, "passthru")
                .expect_err("an unknown static policy is rejected")
                .contains("static_policy=passthru"),
            "animated={animated}"
        );
    }
    assert_eq!(
        plan_page_execution(true, "wrap"),
        Ok(PageExecution::Animation)
    );
}

#[test]
fn encoders_stop_pulling_frames_once_the_output_budget_is_exceeded() {
    let (width, height, frame_count) = (64u32, 64u32, 12u32);
    let frame = |index: u32| {
        let mut rgba = deterministic_rgba(width as usize, height as usize);
        for (offset, byte) in rgba.iter_mut().enumerate() {
            *byte = byte
                .wrapping_mul(index as u8 * 2 + 3)
                .wrapping_add(offset as u8);
        }
        for alpha in rgba.iter_mut().skip(3).step_by(4) {
            *alpha = 255;
        }
        rgba
    };
    let spec = AnimationEncodeSpec {
        width,
        height,
        fps: 20,
        frame_count,
        looped: true,
        output_budget_bytes: 8 * 1024,
        gif_quality: 80,
    };

    let (full, _) = encode_rgba_frames_with_h264_and_telemetry(
        AnimationFormat::Gif,
        &AnimationEncodeSpec {
            output_budget_bytes: usize::MAX,
            ..spec.clone()
        },
        &H264EncoderConfig::default(),
        0,
        |index| Ok(frame(index)),
    )
    .expect("the unbounded GIF encodes");
    assert!(full.data.len() > 4 * spec.output_budget_bytes);

    let mut gif_calls = 0u32;
    let error = encode_rgba_frames_with_h264_and_telemetry(
        AnimationFormat::Gif,
        &spec,
        &H264EncoderConfig::default(),
        0,
        |index| {
            gif_calls += 1;
            Ok(frame(index))
        },
    )
    .expect_err("the GIF crosses its budget");
    assert!(error.contains("OUTPUT_BUDGET_EXCEEDED"), "{error}");
    // One analysis pass over every frame, then the encode pass stops at
    // the first frame that crosses the budget.
    assert!(gif_calls < 2 * frame_count, "GIF pulled {gif_calls} frames");

    let mut apng_calls = 0u32;
    let error = encode_rgba_frames(AnimationFormat::Apng, &spec, |index| {
        apng_calls += 1;
        Ok(frame(index))
    })
    .expect_err("the APNG crosses its budget");
    assert!(error.contains("OUTPUT_BUDGET_EXCEEDED"), "{error}");
    assert!(apng_calls < frame_count, "APNG pulled {apng_calls} frames");

    let mut mp4_calls = 0u32;
    let error = encode_rgba_frames(AnimationFormat::Mp4, &spec, |index| {
        mp4_calls += 1;
        Ok(frame(index))
    })
    .expect_err("the MP4 crosses its budget");
    assert!(error.contains("OUTPUT_BUDGET_EXCEEDED"), "{error}");
    assert!(mp4_calls < frame_count, "MP4 pulled {mp4_calls} frames");
}
