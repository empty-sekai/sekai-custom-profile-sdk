use std::io::Write;

use super::*;

fn generation_report(glyph_count: u32) -> SdfAtlasGenerationReport {
    SdfAtlasGenerationReport {
        cmap_codepoint_count: glyph_count,
        requested_codepoint_count: glyph_count,
        generated_glyph_count: glyph_count,
        failed_glyph_count: 0,
        analytic_fallback_count: 0,
        page_width: 8,
        page_height: 8,
        gutter: 0,
        failures: Vec::new(),
        analytic_fallback_codepoints: Vec::new(),
    }
}

fn write_test_page(root: &Path) -> SdfAtlasPageManifest {
    let mut bytes = vec![0u8; SWIZZLED_PAGE_HEADER_BYTES + 64];
    bytes[..SWIZZLED_PAGE_MAGIC.len()].copy_from_slice(SWIZZLED_PAGE_MAGIC);
    bytes[12..16].copy_from_slice(&SWIZZLED_PAGE_VERSION.to_le_bytes());
    bytes[16..20].copy_from_slice(&8u32.to_le_bytes());
    bytes[20..24].copy_from_slice(&8u32.to_le_bytes());
    bytes[24..28].copy_from_slice(&8u32.to_le_bytes());
    bytes[28..32].copy_from_slice(&8u32.to_le_bytes());
    for (index, value) in bytes[SWIZZLED_PAGE_HEADER_BYTES..].iter_mut().enumerate() {
        *value = index as u8;
    }
    let path = root.join("page-000.r8swz");
    let mut file = File::create(&path).expect("create test page");
    file.write_all(&bytes).expect("write test page");
    SdfAtlasPageManifest {
        file: "page-000.r8swz".into(),
        width: 8,
        height: 8,
        file_sha256: hex::encode(Sha256::digest(&bytes)),
    }
}

fn write_test_atlas_at_sampling(
    root: &Path,
    family: &str,
    codepoint: Option<u32>,
    point_size: f32,
    spread: f32,
) -> Arc<MappedSdfAtlas> {
    std::fs::create_dir_all(root).expect("create atlas directory");
    let page = write_test_page(root);
    let glyphs = codepoint
        .map(|codepoint| SdfAtlasGlyphManifest {
            codepoint,
            page: 0,
            rect: [0, 0, 8, 8],
            plane_bearing: [0.0, 1.0],
            plane_size: [8.0, 8.0],
            plane_advance_x: 8.0,
        })
        .into_iter()
        .collect::<Vec<_>>();
    let manifest = SdfAtlasManifest {
        schema: ATLAS_MANIFEST_SCHEMA.into(),
        generator_contract: "outline-edt-v1".into(),
        font_family: family.into(),
        font_sha256: "00".repeat(32),
        point_size,
        spread,
        pages: vec![page],
        generation: generation_report(u32::try_from(glyphs.len()).expect("glyph count")),
        glyphs,
    };
    let manifest_path = root.join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");
    Arc::new(MappedSdfAtlas::open(manifest_path).expect("open test atlas"))
}

fn write_test_atlas(root: &Path, family: &str, codepoint: Option<u32>) -> Arc<MappedSdfAtlas> {
    write_test_atlas_at_sampling(root, family, codepoint, 75.0, 6.0)
}

#[test]
fn atlas_set_keeps_three_x_tier_lazy_and_selectable() {
    let root = tempfile::tempdir().expect("temporary atlas directory");
    let base = write_test_atlas_at_sampling(
        &root.path().join("base"),
        "test-family",
        Some(u32::from('(')),
        75.0,
        6.0,
    );
    let three_x = write_test_atlas_at_sampling(
        &root.path().join("three-x"),
        "test-family",
        Some(u32::from('(')),
        225.0,
        18.0,
    );
    let mut set = MappedSdfAtlasSet::new();
    let base_id = set.insert(base).expect("insert base tier");
    let three_x_id = set.insert(three_x).expect("insert 3x tier");

    assert_eq!(
        set.atlas_for_font_family("test-family").map(|v| v.0),
        Some(base_id)
    );
    assert_eq!(
        set.atlas_for_font_family_at_least("test-family", 225.0)
            .map(|v| v.0),
        Some(three_x_id)
    );
    let report = set.prewarm_primary_pages();
    assert_eq!(report.atlas_count, 1);
    assert_eq!(report.page_count, 1);
}

#[test]
fn sparse_sampling_tiers_select_the_smallest_adequate_glyph() {
    let root = tempfile::tempdir().expect("temporary atlas directory");
    let base = write_test_atlas_at_sampling(
        &root.path().join("base"),
        "test-family",
        Some(u32::from('(')),
        75.0,
        6.0,
    );
    let three_x = write_test_atlas_at_sampling(
        &root.path().join("three-x"),
        "test-family",
        Some(u32::from('(')),
        225.0,
        18.0,
    );
    let six_x_missing = write_test_atlas_at_sampling(
        &root.path().join("six-x-missing"),
        "test-family",
        Some(u32::from('A')),
        450.0,
        36.0,
    );
    let twelve_x = write_test_atlas_at_sampling(
        &root.path().join("twelve-x"),
        "test-family",
        Some(u32::from('(')),
        900.0,
        72.0,
    );
    let mut set = MappedSdfAtlasSet::new();
    set.insert(base).expect("insert base tier");
    let three_x_id = set.insert(three_x).expect("insert 3x tier");
    set.insert(six_x_missing).expect("insert sparse 6x tier");
    let twelve_x_id = set.insert(twelve_x).expect("insert sparse 12x tier");

    assert_eq!(
        set.glyph_for_font_family_at_least("test-family", u32::from('('), 300.0)
            .map(|value| value.0),
        Some(twelve_x_id)
    );
    assert_eq!(
        set.highest_glyph_for_font_family_above("test-family", u32::from('('), 75.0)
            .map(|value| value.0),
        Some(twelve_x_id)
    );
    assert!(set
        .glyph_for_font_family_at_least("test-family", u32::from('('), 1000.0)
        .is_none());
    assert_eq!(
        set.highest_glyph_for_font_family_above("test-family", u32::from('A'), 75.0)
            .map(|value| value.0),
        set.atlas_for_font_family_at_least("test-family", 450.0)
            .map(|value| value.0)
    );
    assert_ne!(three_x_id, twelve_x_id);
}

#[test]
fn opens_and_addresses_swizzled_page() {
    let root = tempfile::tempdir().expect("temporary atlas directory");
    let page = write_test_page(root.path());
    let manifest = SdfAtlasManifest {
        schema: ATLAS_MANIFEST_SCHEMA.into(),
        generator_contract: "outline-edt-v1".into(),
        font_family: "test".into(),
        font_sha256: "00".repeat(32),
        point_size: 75.0,
        spread: 6.0,
        pages: vec![page],
        glyphs: vec![SdfAtlasGlyphManifest {
            codepoint: u32::from('A'),
            page: 0,
            rect: [0, 0, 8, 8],
            plane_bearing: [0.0, 1.0],
            plane_size: [8.0, 8.0],
            plane_advance_x: 8.0,
        }],
        generation: generation_report(1),
    };
    let manifest_path = root.path().join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");

    let atlas = MappedSdfAtlas::open(&manifest_path).expect("open mapped atlas");
    assert_eq!(
        atlas.manifest_sha256(),
        hex::encode(Sha256::digest(
            std::fs::read(&manifest_path).expect("read manifest identity")
        ))
    );
    assert_eq!(atlas.pages()[0].texel(3, 5), Some(43));
    assert_eq!(atlas.pages()[0].texel(8, 0), None);
    assert_eq!(atlas.glyph(u32::from('A')).map(|glyph| glyph.page), Some(0));
}

#[test]
fn trusted_immutable_open_skips_only_the_redundant_payload_digest() {
    let root = tempfile::tempdir().expect("temporary atlas directory");
    let mut page = write_test_page(root.path());
    page.file_sha256 = "ff".repeat(32);
    let manifest = SdfAtlasManifest {
        schema: ATLAS_MANIFEST_SCHEMA.into(),
        generator_contract: "outline-edt-v1".into(),
        font_family: "trusted-image-font".into(),
        font_sha256: "00".repeat(32),
        point_size: 225.0,
        spread: 18.0,
        pages: vec![page],
        glyphs: Vec::new(),
        generation: generation_report(0),
    };
    let manifest_path = root.path().join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");

    assert!(matches!(
        MappedSdfAtlas::open(&manifest_path),
        Err(SdfAtlasError::HashMismatch { .. })
    ));
    let trusted = MappedSdfAtlas::open_trusted_immutable_artifact(&manifest_path)
        .expect("trusted immutable image atlas");
    assert_eq!(trusted.pages()[0].texel(3, 5), Some(43));
}

#[test]
fn rejects_manifest_path_traversal() {
    let manifest = SdfAtlasManifest {
        schema: ATLAS_MANIFEST_SCHEMA.into(),
        generator_contract: "outline-edt-v1".into(),
        font_family: "test".into(),
        font_sha256: "00".repeat(32),
        point_size: 75.0,
        spread: 6.0,
        pages: vec![SdfAtlasPageManifest {
            file: "../outside.r8swz".into(),
            width: 8,
            height: 8,
            file_sha256: "00".repeat(32),
        }],
        glyphs: Vec::new(),
        generation: generation_report(0),
    };
    assert!(validate_manifest(&manifest).is_err());
}

#[test]
fn atlas_set_assigns_stable_ids_and_deduplicates_identity() {
    let root = tempfile::tempdir().expect("temporary atlas directory");
    let page = write_test_page(root.path());
    let manifest = SdfAtlasManifest {
        schema: ATLAS_MANIFEST_SCHEMA.into(),
        generator_contract: "outline-edt-v1".into(),
        font_family: "test-family".into(),
        font_sha256: "00".repeat(32),
        point_size: 75.0,
        spread: 6.0,
        pages: vec![page],
        glyphs: Vec::new(),
        generation: generation_report(0),
    };
    let manifest_path = root.path().join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");
    let atlas = Arc::new(MappedSdfAtlas::open(&manifest_path).expect("open mapped atlas"));
    let mut set = MappedSdfAtlasSet::new();
    assert_eq!(set.insert(Arc::clone(&atlas)).expect("first insert"), 0);
    assert_eq!(set.insert(atlas).expect("duplicate identity"), 0);
    assert_eq!(set.len(), 1);
    assert_eq!(
        set.atlas_for_font_family("test-family").map(|(id, _)| id),
        Some(0)
    );
    assert_eq!(set.mapped_bytes(), (SWIZZLED_PAGE_HEADER_BYTES + 64) as u64);
    let prewarm = set.prewarm_pages();
    assert_eq!(prewarm.atlas_count, 1);
    assert_eq!(prewarm.page_count, 1);
    assert_eq!(prewarm.mapped_bytes, 128);
    assert_eq!(prewarm.page_touch_count, 1);
    assert_eq!(prewarm.checksum, 189);
}

#[test]
fn profile_glyph_uses_declared_fallback_after_primary_miss() {
    let root = tempfile::tempdir().expect("temporary atlas directory");
    let primary = write_test_atlas(&root.path().join("primary"), "game-font", None);
    let fallback = write_test_atlas(
        &root.path().join("fallback"),
        PROFILE_TEXT_FALLBACK_FONT_FAMILY,
        Some(u32::from('♥')),
    );
    let mut set = MappedSdfAtlasSet::new();
    assert_eq!(set.insert(primary).expect("primary atlas"), 0);
    assert_eq!(set.insert(fallback).expect("fallback atlas"), 1);
    let (atlas_set, atlas, glyph) = set
        .profile_glyph_for_font_family("game-font", u32::from('♥'))
        .expect("fallback glyph");
    assert_eq!(atlas_set, 1);
    assert_eq!(
        atlas.manifest().font_family,
        PROFILE_TEXT_FALLBACK_FONT_FAMILY
    );
    assert_eq!(glyph.codepoint, u32::from('♥'));
}
