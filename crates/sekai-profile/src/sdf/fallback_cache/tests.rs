use super::*;

const REGULAR: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";
const BOLD: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf";

fn fonts_available() -> bool {
    let available = Path::new(REGULAR).is_file() && Path::new(BOLD).is_file();
    if !available {
        eprintln!("skipping: DejaVu Sans is not installed");
    }
    available
}

fn codepoints(text: &str) -> BTreeSet<u32> {
    text.chars().map(u32::from).collect()
}

/// Every regular file below `root`, relative to it.
fn files_below(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("read cache directory") {
            let path = entry.expect("cache directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(
                    path.strip_prefix(root)
                        .expect("path below root")
                        .to_path_buf(),
                );
            }
        }
    }
    files.sort();
    files
}

#[test]
fn extending_the_cache_generates_only_new_glyphs_and_keeps_one_copy_of_each() {
    if !fonts_available() {
        return;
    }
    let root = tempfile::tempdir().expect("fallback cache tempdir");
    let cache = PersistentFallbackSdfCache::new(root.path(), REGULAR, "test-fallback")
        .expect("create fallback cache");
    for (step, text) in ["A", "AB", "ABC"].into_iter().enumerate() {
        let (atlas, report) = cache
            .ensure_codepoints(&codepoints(text))
            .expect("extend fallback cache");
        let atlas = atlas.expect("fallback atlas");
        assert_eq!(atlas.manifest().glyphs.len(), step + 1);
        assert_eq!(report.generated_codepoint_count, 1);
        assert_eq!(report.cache_hit_count, step as u64);
    }
    let (_, report) = cache
        .ensure_codepoints(&codepoints("CA"))
        .expect("hit fallback cache");
    assert_eq!(report.generated_codepoint_count, 0);
    assert_eq!(report.cache_hit_count, 2);

    // One page per cached glyph and one manifest; superseded
    // generations are not left behind.
    let files = files_below(root.path());
    let pages = files
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "r8swz"))
        .count();
    let manifests = files
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter(|path| !path.ends_with("current.json"))
        .count();
    assert_eq!((pages, manifests), (3, 1), "cache files: {files:?}");

    let reopened = PersistentFallbackSdfCache::new(root.path(), REGULAR, "test-fallback")
        .expect("reopen fallback cache");
    let current = reopened
        .load_current()
        .expect("cache state")
        .expect("persisted atlas");
    assert_eq!(current.manifest().glyphs.len(), 3);
}

#[test]
fn codepoints_the_font_lacks_are_recorded_instead_of_failing_the_generation() {
    if !fonts_available() {
        return;
    }
    let root = tempfile::tempdir().expect("fallback cache tempdir");
    let cache = PersistentFallbackSdfCache::new(root.path(), REGULAR, "test-fallback")
        .expect("create fallback cache");
    // DejaVu Sans has no CJK ideographs.
    let requested = codepoints("A\u{6F22}");
    let (atlas, report) = cache
        .ensure_codepoints(&requested)
        .expect("a codepoint the font lacks must not fail the generation");
    let atlas = atlas.expect("fallback atlas");
    assert!(atlas.glyph(u32::from('A')).is_some());
    assert!(atlas.glyph(0x6F22).is_none());
    let failures = atlas
        .manifest()
        .generation
        .failures
        .iter()
        .map(|failure| failure.codepoint)
        .collect::<Vec<_>>();
    assert_eq!(failures, vec![0x6F22]);
    assert_eq!(report.generated_codepoint_count, 1);

    // The recorded codepoint counts as resolved: asking again, in this
    // process or after a restart, generates nothing.
    let (again, report) = cache
        .ensure_codepoints(&requested)
        .expect("repeated request");
    assert_eq!(
        (report.cache_hit_count, report.generated_codepoint_count),
        (2, 0)
    );
    assert_eq!(
        again.expect("fallback atlas").manifest_sha256(),
        atlas.manifest_sha256()
    );
    let reopened = PersistentFallbackSdfCache::new(root.path(), REGULAR, "test-fallback")
        .expect("reopen fallback cache");
    let (_, report) = reopened
        .ensure_codepoints(&requested)
        .expect("request after reopening");
    assert_eq!(
        (report.cache_hit_count, report.generated_codepoint_count),
        (2, 0)
    );

    // With no glyph at all there is no atlas to publish, and the codepoint
    // is still not generated again.
    let root = tempfile::tempdir().expect("second fallback cache tempdir");
    let cache = PersistentFallbackSdfCache::new(root.path(), REGULAR, "test-fallback")
        .expect("create second fallback cache");
    let unavailable = codepoints("\u{6F22}");
    let (atlas, _) = cache
        .ensure_codepoints(&unavailable)
        .expect("a request of unavailable codepoints only");
    assert!(atlas.is_none());
    let (_, report) = cache
        .ensure_codepoints(&unavailable)
        .expect("repeated unavailable request");
    assert_eq!(
        (report.cache_hit_count, report.generated_codepoint_count),
        (1, 0)
    );
}

#[test]
fn replacing_the_font_file_invalidates_its_cached_glyphs() {
    if !fonts_available() {
        return;
    }
    let root = tempfile::tempdir().expect("fallback cache tempdir");
    let font_dir = tempfile::tempdir().expect("font tempdir");
    let font = font_dir.path().join("fallback.ttf");
    fs::copy(REGULAR, &font).expect("install regular font");
    let regular = PersistentFallbackSdfCache::new(root.path(), &font, "test-fallback")
        .expect("create fallback cache");
    let (regular_atlas, _) = regular
        .ensure_codepoints(&codepoints("A"))
        .expect("regular glyph");
    let regular_atlas = regular_atlas.expect("regular atlas");

    fs::copy(BOLD, &font).expect("replace font file");
    let bold_sha256 = hex::encode(Sha256::digest(
        fs::read(&font).expect("replaced font bytes"),
    ));
    let bold = PersistentFallbackSdfCache::new(root.path(), &font, "test-fallback")
        .expect("reopen fallback cache for the replaced font");
    assert!(
        bold.load_current().expect("cache state").is_none(),
        "glyphs of the previous font must not be served"
    );
    let (bold_atlas, report) = bold
        .ensure_codepoints(&codepoints("A"))
        .expect("bold glyph");
    let bold_atlas = bold_atlas.expect("bold atlas");
    assert_eq!(report.generated_codepoint_count, 1);
    assert_eq!(bold_atlas.manifest().font_sha256, bold_sha256);
    assert_ne!(
        bold_atlas.manifest().glyphs[0].plane_advance_x,
        regular_atlas.manifest().glyphs[0].plane_advance_x
    );
    let identity_dirs = fs::read_dir(root.path())
        .expect("cache root listing")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .count();
    assert_eq!(identity_dirs, 1, "the previous font's glyphs are reclaimed");
}

#[test]
fn a_pointer_from_another_generator_contract_is_a_cache_miss() {
    if !fonts_available() {
        return;
    }
    let root = tempfile::tempdir().expect("fallback cache tempdir");
    fs::write(
        root.path().join("current.json"),
        serde_json::json!({
            "schema": "allium.sdf-fallback-cache-pointer.v1",
            "font_family": "test-fallback",
            "font_sha256": "00".repeat(32),
            "generator_contract": "outline-edt-v0",
            "manifest": "generations/old/manifest.json",
        })
        .to_string(),
    )
    .expect("foreign pointer");
    let cache = PersistentFallbackSdfCache::new(root.path(), REGULAR, "test-fallback")
        .expect("a foreign pointer must not prevent opening the cache");
    assert!(cache.load_current().expect("cache state").is_none());
    let (atlas, _) = cache.ensure_codepoints(&codepoints("A")).expect("glyph");
    assert!(atlas.is_some());
}

#[test]
fn a_lock_file_left_by_a_crashed_process_does_not_block() {
    let root = tempfile::tempdir().expect("lock tempdir");
    fs::write(root.path().join("cache.lock"), "pid=1\n").expect("stale lock file");
    let started = Instant::now();
    let guard = CacheFileLock::acquire(root.path()).expect("acquire lock");
    assert!(started.elapsed() < Duration::from_secs(5));
    drop(guard);
    assert!(CacheFileLock::acquire(root.path()).is_ok());
}

#[test]
fn the_lock_admits_one_holder_at_a_time() {
    let root = tempfile::tempdir().expect("lock tempdir");
    let first = CacheFileLock::acquire(root.path()).expect("first holder");
    assert!(CacheFileLock::acquire_within(root.path(), Duration::from_millis(50)).is_err());
    drop(first);
    let second = CacheFileLock::acquire_within(root.path(), Duration::from_millis(50))
        .expect("lock is free once the first holder is gone");
    assert!(CacheFileLock::acquire_within(root.path(), Duration::from_millis(50)).is_err());
    drop(second);
}
