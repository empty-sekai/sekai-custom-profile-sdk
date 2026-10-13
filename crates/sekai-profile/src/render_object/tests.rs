use super::*;

fn write_generation(root: &Path, identity: &str, value: u8) -> PathBuf {
    let pixels = vec![value; 2 * 2 * 4];
    let source_sha256 = hex::encode(Sha256::digest([value]));
    let mut writer = RenderObjectStoreWriter::create(root, identity, 1024).unwrap();
    writer
        .add(RenderObjectWrite {
            key: "texture:assets/generation-fixture",
            kind: RenderObjectKind::Texture,
            source_sha256: &source_sha256,
            width: 2,
            height: 2,
            row_bytes: 8,
            pixels: &pixels,
        })
        .unwrap();
    writer.finish().unwrap()
}

#[test]
fn generation_publish_keeps_pinned_request_and_previous_alive() {
    let first_root = tempfile::tempdir().unwrap();
    let second_root = tempfile::tempdir().unwrap();
    let first_manifest = write_generation(first_root.path(), "generation-1", 1);
    let second_manifest = write_generation(second_root.path(), "generation-2", 2);
    let manager = RenderObjectGenerationManager::open(first_manifest).unwrap();
    let pinned_request = manager.current();

    let published = manager.publish_manifest(second_manifest).unwrap();
    assert_eq!(published.manifest().source_identity, "generation-2");
    assert_eq!(manager.current().manifest().source_identity, "generation-2");
    assert_eq!(
        manager.previous().unwrap().manifest().source_identity,
        "generation-1"
    );
    assert_eq!(pinned_request.manifest().source_identity, "generation-1");
    assert_eq!(
        pinned_request
            .object("texture:assets/generation-fixture")
            .unwrap()
            .pixels[0],
        1
    );

    let rolled_back = manager.rollback().expect("previous generation");
    assert_eq!(rolled_back.manifest().source_identity, "generation-1");
    assert_eq!(manager.current().manifest().source_identity, "generation-1");
    assert_eq!(
        manager.previous().unwrap().manifest().source_identity,
        "generation-2"
    );
}

fn write_fixture(root: &Path) -> PathBuf {
    let pixels = [
        1u8, 2, 3, 4, 5, 6, 7, 8, // row 0
        9, 10, 11, 12, 13, 14, 15, 16, // row 1
    ];
    let mut page = vec![0u8; RENDER_OBJECT_PAGE_HEADER_BYTES + pixels.len()];
    page[..RENDER_OBJECT_PAGE_MAGIC.len()].copy_from_slice(RENDER_OBJECT_PAGE_MAGIC);
    page[12..16].copy_from_slice(&RENDER_OBJECT_PAGE_VERSION.to_le_bytes());
    page[16..24].copy_from_slice(&(pixels.len() as u64).to_le_bytes());
    page[24..28].copy_from_slice(&1u32.to_le_bytes());
    page[RENDER_OBJECT_PAGE_HEADER_BYTES..].copy_from_slice(&pixels);
    std::fs::write(root.join("objects.rgba"), &page).expect("write page");

    let manifest = RenderObjectManifest {
        schema: RENDER_OBJECT_MANIFEST_SCHEMA.into(),
        generator_contract: RENDER_OBJECT_GENERATOR_CONTRACT.into(),
        pixel_format: RENDER_OBJECT_PIXEL_FORMAT.into(),
        source_identity: "fixture-v1".into(),
        pages: vec![RenderObjectPageManifest {
            file: "objects.rgba".into(),
            payload_bytes: pixels.len() as u64,
            object_count: 1,
            file_sha256: hex::encode(Sha256::digest(&page)),
        }],
        objects: vec![RenderObjectEntry {
            key: "texture:fixture".into(),
            kind: RenderObjectKind::Texture,
            source_sha256: hex::encode(Sha256::digest(b"source")),
            page: 0,
            offset: 0,
            length: pixels.len() as u64,
            width: 2,
            height: 2,
            row_bytes: 8,
            pixel_sha256: hex::encode(Sha256::digest(pixels)),
        }],
    };
    let manifest_path = root.join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");
    manifest_path
}

#[test]
fn opens_and_returns_mapped_rows() {
    let root = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_fixture(root.path());
    let store = MappedRenderObjectStore::open(manifest_path).expect("open store");
    let object = store.object("texture:fixture").expect("mapped object");
    assert_eq!(object.pixels.len(), 16);
    assert_eq!(object.row(0), Some(&[1, 2, 3, 4, 5, 6, 7, 8][..]));
    assert_eq!(object.row(1), Some(&[9, 10, 11, 12, 13, 14, 15, 16][..]));
    assert_eq!(object.row(2), None);
    assert_eq!(store.manifest().objects.len(), 1);
    assert_eq!(store.mapped_bytes(), 80);
}

#[test]
fn metadata_catalog_does_not_open_pixel_pages() {
    let root = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_fixture(root.path());
    std::fs::remove_file(root.path().join("objects.rgba")).expect("remove pixel page");

    let store = MappedRenderObjectStore::open_metadata_catalog(&manifest_path)
        .expect("open metadata catalog");
    let metadata = store.metadata("texture:fixture").expect("object metadata");
    assert_eq!((metadata.width, metadata.height), (2, 2));
    assert_eq!(store.mapped_bytes(), 0);
    assert!(store.object("texture:fixture").is_none());
}

#[test]
fn rejects_path_traversal_and_overlapping_objects() {
    let root = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_fixture(root.path());
    let mut manifest: RenderObjectManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).expect("read manifest"))
            .expect("parse manifest");
    manifest.pages[0].file = "../objects.rgba".into();
    assert!(matches!(
        validate_manifest(&manifest),
        Err(RenderObjectError::Invalid(_))
    ));

    manifest.pages[0].file = "objects.rgba".into();
    manifest.pages[0].object_count = 2;
    let mut duplicate = manifest.objects[0].clone();
    duplicate.key = "texture:overlap".into();
    manifest.objects.push(duplicate);
    assert!(matches!(
        validate_manifest(&manifest),
        Err(RenderObjectError::Invalid(_))
    ));
}

#[test]
fn accepts_unreferenced_physical_objects_for_incremental_hardlinks() {
    let root = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_fixture(root.path());
    let mut manifest: RenderObjectManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).expect("read manifest"))
            .expect("parse manifest");
    manifest.objects.clear();
    validate_manifest(&manifest).expect("orphaned immutable payload is valid");
}

#[test]
fn rejects_tampered_pixels_even_with_valid_page_hash() {
    let root = tempfile::tempdir().expect("tempdir");
    let manifest_path = write_fixture(root.path());
    let mut page = std::fs::read(root.path().join("objects.rgba")).expect("read page");
    page[RENDER_OBJECT_PAGE_HEADER_BYTES] ^= 0xff;
    std::fs::write(root.path().join("objects.rgba"), &page).expect("write page");

    let mut manifest: RenderObjectManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).expect("read manifest"))
            .expect("parse manifest");
    manifest.pages[0].file_sha256 = hex::encode(Sha256::digest(&page));
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");

    assert!(matches!(
        MappedRenderObjectStore::open(manifest_path),
        Err(RenderObjectError::PixelHashMismatch { .. })
    ));
}

#[test]
fn writer_streams_aligned_pages_and_reopens_them() {
    let root = tempfile::tempdir().expect("tempdir");
    let output = root.path().join("store");
    let source = hex::encode(Sha256::digest(b"source"));
    let first = [1u8; 16];
    let second = [2u8; 16];
    let mut writer =
        RenderObjectStoreWriter::create(&output, "fixture-writer-v1", 64).expect("writer");
    writer
        .add(RenderObjectWrite {
            key: "texture:assets/a",
            kind: RenderObjectKind::Texture,
            source_sha256: &source,
            width: 2,
            height: 2,
            row_bytes: 8,
            pixels: &first,
        })
        .expect("add first");
    writer
        .add(RenderObjectWrite {
            key: "texture:assets/b",
            kind: RenderObjectKind::Texture,
            source_sha256: &source,
            width: 2,
            height: 2,
            row_bytes: 8,
            pixels: &second,
        })
        .expect("add second");
    let manifest_path = writer.finish().expect("finish store");
    let store = MappedRenderObjectStore::open(manifest_path).expect("reopen store");
    assert_eq!(store.manifest().pages.len(), 2);
    assert_eq!(store.object("texture:assets/a").expect("a").pixels, first);
    assert_eq!(store.object("texture:assets/b").expect("b").pixels, second);
    let metadata = store
        .resource_metadata("assets", "a")
        .expect("resource metadata");
    assert_eq!(
        (metadata.width, metadata.height, metadata.row_bytes),
        (2, 2, 8)
    );
    assert_eq!(metadata.pixel_format, RENDER_OBJECT_PIXEL_FORMAT);
    assert_eq!(metadata.source_identity, "fixture-writer-v1");
    assert_eq!(store.manifest().objects[0].offset, 0);
    assert_eq!(store.manifest().objects[1].offset, 0);
}

#[test]
fn profile_hotset_prewarm_touches_only_declared_global_prefixes() {
    let root = tempfile::tempdir().expect("tempdir");
    let output = root.path().join("store");
    let source = hex::encode(Sha256::digest(b"source"));
    let hot = [3u8; 16];
    let cold = [7u8; 16];
    let mut writer =
        RenderObjectStoreWriter::create(&output, "prewarm-fixture", 128).expect("writer");
    for key in [
        "texture:assets/bonds_honor/example",
        "texture:assets/chara_avatar/chara01_02",
        "texture:assets/character/member_cutout/res/after_training",
        "texture:assets/event_story/example/screen_image/banner_event_story",
        "texture:assets/honor/example/degree_main",
        "texture:assets/thumbnail/chara/example",
        "texture:assets/unit_story/example/screen_image/banner_unit_story",
        "texture:static/honor/frame_degree_m_1",
    ] {
        writer
            .add(RenderObjectWrite {
                key,
                kind: RenderObjectKind::Texture,
                source_sha256: &source,
                width: 2,
                height: 2,
                row_bytes: 8,
                pixels: &hot,
            })
            .expect("General hotset object");
    }
    writer
        .add(RenderObjectWrite {
            key: "texture:unrelated/example",
            kind: RenderObjectKind::Texture,
            source_sha256: &source,
            width: 2,
            height: 2,
            row_bytes: 8,
            pixels: &cold,
        })
        .expect("cold object");
    let manifest = writer.finish().expect("finish store");
    let store = MappedRenderObjectStore::open(manifest).expect("open store");
    let report = store.prewarm_profile_hotset();
    assert_eq!(report.object_count, 8);
    assert_eq!(report.object_bytes, 128);
    assert_eq!(report.page_touch_count, 8);
    assert_eq!(report.checksum, 65_537);
}

#[test]
fn profile_hotset_prewarm_skips_deck_sources_covered_by_variants() {
    let root = tempfile::tempdir().expect("tempdir");
    let output = root.path().join("store");
    let source_key = "texture:assets/character/member_cutout/res/normal";
    let source_sha256 = hex::encode(Sha256::digest(b"deck source"));
    let (variant_key, variant_sha256) =
        crate::profile_compositor::deck_art_variant_identity_from_source(&source_sha256);
    let mut writer =
        RenderObjectStoreWriter::create(&output, "prewarm-variant-fixture", 128).expect("writer");
    // Keys are added in ascending order: "component:" sorts before "texture:".
    writer
        .add(RenderObjectWrite {
            key: &variant_key,
            kind: RenderObjectKind::Component,
            source_sha256: &variant_sha256,
            width: 2,
            height: 2,
            row_bytes: 8,
            pixels: &[5u8; 16],
        })
        .expect("variant object");
    writer
        .add(RenderObjectWrite {
            key: source_key,
            kind: RenderObjectKind::Texture,
            source_sha256: &source_sha256,
            width: 2,
            height: 2,
            row_bytes: 8,
            pixels: &[9u8; 16],
        })
        .expect("deck source object");
    let manifest = writer.finish().expect("finish store");
    let store = MappedRenderObjectStore::open(manifest).expect("open store");

    let report = store.prewarm_profile_hotset();
    assert_eq!(report.object_count, 1);
    assert_eq!(report.object_bytes, 16);
}
