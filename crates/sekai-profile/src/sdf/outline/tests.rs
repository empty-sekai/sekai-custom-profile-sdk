use super::*;

fn assert_glyph_exact(actual: &OutlineSdfGlyph, expected: &OutlineSdfGlyph) {
    assert_eq!(
        (actual.width, actual.height),
        (expected.width, expected.height)
    );
    assert_eq!(actual.bearing_x.to_bits(), expected.bearing_x.to_bits());
    assert_eq!(actual.bearing_y.to_bits(), expected.bearing_y.to_bits());
    assert_eq!(
        actual.plane_bearing_x.to_bits(),
        expected.plane_bearing_x.to_bits()
    );
    assert_eq!(
        actual.plane_bearing_y.to_bits(),
        expected.plane_bearing_y.to_bits()
    );
    assert_eq!(actual.plane_width.to_bits(), expected.plane_width.to_bits());
    assert_eq!(
        actual.plane_height.to_bits(),
        expected.plane_height.to_bits()
    );
    assert_eq!(
        actual.plane_advance_x.to_bits(),
        expected.plane_advance_x.to_bits()
    );
    assert_eq!(actual.pixels, expected.pixels);
}

#[test]
fn every_omikuji_font_asset_names_its_font_file() {
    for prefab in sekai_profile_core::omikuji::PREFABS {
        let family = prefab.font_family;
        let files = FONT_FILE_MAP
            .iter()
            .find_map(|(key, files)| (*key == family).then_some(*files));
        assert_eq!(
            files,
            Some(&[format!("{family}.otf").as_str()][..]),
            "{}",
            prefab.bundle
        );
    }
}

#[test]
fn every_omikuji_fallback_face_names_its_font_file() {
    use sekai_profile_core::omikuji::{CjkFallback, LATIN_FALLBACK_FAMILY};
    let files = |family: &str| {
        FONT_FILE_MAP
            .iter()
            .find_map(|(key, files)| (*key == family).then_some(*files))
    };
    assert_eq!(
        files(LATIN_FALLBACK_FAMILY),
        Some(&["Roboto-Regular.ttf"][..])
    );
    for cjk in CjkFallback::ALL {
        assert_eq!(
            files(cjk.family()),
            Some(&["NotoSansCJK-Regular.ttc"][..]),
            "{cjk:?}"
        );
    }
}

#[test]
fn persistent_offline_face_matches_one_shot_generation() {
    let Some((family, path)) = ["FZLanTingHei-DB-GBK", "DejaVu Sans"]
        .into_iter()
        .find_map(|family| resolve_font_path(family).map(|path| (family, path)))
    else {
        eprintln!("skipping: no test family is installed");
        return;
    };
    let generator = OfflineAtlasGlyphGenerator::new(family).expect("offline generator");
    for ch in ['A', 'g'] {
        let analytic_one_shot = generate_outline_sdf(&path, ch).expect("analytic one-shot");
        let (analytic_persistent, analytic_fallback) = generator
            .generate(ch, OfflineGenerationMethod::Analytic)
            .expect("analytic persistent");
        assert!(!analytic_fallback);
        assert_glyph_exact(&analytic_persistent, &analytic_one_shot);

        for supersample in [1, 2, 4] {
            let edt_one_shot =
                generate_outline_sdf_edt(&path, ch, supersample).expect("EDT one-shot");
            let (edt_persistent, edt_fallback) = generator
                .generate(ch, OfflineGenerationMethod::Edt { supersample })
                .expect("EDT persistent");
            assert!(!edt_fallback);
            assert_glyph_exact(&edt_persistent, &edt_one_shot);
        }
    }
}

#[test]
fn realtime_edt_parentheses_scale_the_source_grid_without_cache_reuse() {
    if resolve_font_path("FZLanTingHei-DB-GBK").is_none() {
        eprintln!("skipping: FONT_DIR does not provide the test family");
        return;
    }

    let family = "FZLanTingHei-DB-GBK";
    for ch in ['(', ')'] {
        let normal = generate_realtime_edt(family, ch, 75.0, 6.0, 2).expect("normal EDT");
        let huge = generate_realtime_edt(family, ch, 300.0, 24.0, 2).expect("huge EDT");
        assert!(huge.width() > normal.width() * 2);
        assert!(huge.height() > normal.height() * 2);
        assert!(huge.pixels().iter().any(|&value| value != 0));
    }
}

/// Mean and maximum absolute gray difference over the texels where the
/// exact field is not saturated, i.e. inside the encoded distance band.
fn band_error(actual: &OutlineSdfGlyph, exact: &OutlineSdfGlyph) -> (f32, f32) {
    assert_eq!((actual.width, actual.height), (exact.width, exact.height));
    let mut sum = 0.0f32;
    let mut max = 0.0f32;
    let mut count = 0usize;
    for (&value, &reference) in actual.pixels.iter().zip(&exact.pixels) {
        if reference == 0 || reference == 255 {
            continue;
        }
        let error = (f32::from(value) - f32::from(reference)).abs();
        sum += error;
        max = max.max(error);
        count += 1;
    }
    assert!(
        count > 0,
        "the glyph must have texels inside the distance band"
    );
    (sum / count as f32, max)
}

#[test]
fn edt_supersampling_converges_on_the_analytic_field() {
    let family = "DejaVu Sans";
    let Some(path) = resolve_font_path(family) else {
        eprintln!("skipping: DejaVu Sans is not installed");
        return;
    };
    for ch in ['O', 'a', 'g'] {
        let exact = generate_outline_sdf(&path, ch).expect("analytic glyph");
        let error = |supersample| {
            let glyph = generate_outline_sdf_edt(&path, ch, supersample).expect("EDT glyph");
            band_error(&glyph, &exact)
        };
        let (mean_2, _) = error(2);
        let (mean_4, max_4) = error(4);
        // Real supersampling resolves the edge more finely each time the
        // factor doubles; replicating a 1x mask would leave it unchanged.
        assert!(
            mean_4 < mean_2 * 0.7,
            "{ch}: ss4 mean {mean_4} is not clearly below ss2 mean {mean_2}"
        );
        assert!(mean_4 < 2.5, "{ch}: ss4 mean gray error {mean_4}");
        assert!(max_4 < 10.0, "{ch}: ss4 max gray error {max_4}");
    }
}

#[test]
fn realtime_edt_rejects_non_positive_distance_spread() {
    assert!(generate_realtime_edt("unused", '(', 300.0, 0.0, 2).is_err());
}
