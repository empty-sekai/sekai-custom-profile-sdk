//! Font metric parity: FreeType against Skia, over a font's real cmap.
//!
//! The layout path already treats FreeType advances as truth — it is the same
//! engine TextMesh Pro uses, and the in-tree notes record Skia `measure_str`
//! running short on halfwidth punctuation. Skia is only consulted where the SDF
//! atlas misses a glyph. This tool quantifies exactly what changes when that
//! fallback is removed, so the switch is made on measured evidence.
//!
//! Usage:
//!   font-metric-parity <family> [size ...]
//!
//! `family` is a name from the renderer's font map, e.g. FZLanTingHei-DB-GBK.
//! Sizes default to a spread covering small UI text through display sizes.

use allium_renderer::sdf::outline::{self, lookup_or_generate, resolve_font_path};

/// Sums per-character Skia advances exactly as `text::measure::tmp_measure_advance`.
fn skia_advance(text: &str, font: &skia_safe::Font) -> f32 {
    text.chars()
        .map(|ch| font.measure_str(ch.to_string(), None).0)
        .sum()
}

fn freetype_advance(family: &str, ch: char, size: f32) -> Option<f32> {
    let glyph = lookup_or_generate(Some(family), ch)?;
    let advance = glyph.plane_advance_x() * (size / outline::sampling_point_size());
    (advance > 0.0).then_some(advance)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(family) = args.next() else {
        eprintln!("usage: font-metric-parity <family> [size ...]");
        std::process::exit(2);
    };
    let sizes: Vec<f32> = {
        let explicit: Vec<f32> = args.filter_map(|a| a.parse().ok()).collect();
        if explicit.is_empty() {
            vec![20.0, 32.0, 40.0, 75.0]
        } else {
            explicit
        }
    };

    let Some(path) = resolve_font_path(&family) else {
        eprintln!("font family {family:?} did not resolve to a file on disk");
        eprintln!("set FONT_DIR (or SCAPUS_FONT_DIR) to the directory holding it");
        std::process::exit(2);
    };
    println!("family : {family}");
    println!("file   : {}", path.display());

    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("could not read {}", path.display());
        std::process::exit(2);
    };
    let Ok(face) = ttf_parser::Face::parse(&bytes, 0) else {
        eprintln!("could not parse the font face");
        std::process::exit(2);
    };

    // Enumerate the face's own cmap rather than guessing ranges, so coverage is
    // exactly what the font actually provides.
    let mut codepoints: Vec<char> = Vec::new();
    for subtable in face.tables().cmap.into_iter().flat_map(|c| c.subtables) {
        if !subtable.is_unicode() {
            continue;
        }
        subtable.codepoints(|cp| {
            if let Some(ch) = char::from_u32(cp) {
                codepoints.push(ch);
            }
        });
    }
    codepoints.sort_unstable();
    codepoints.dedup();
    let total_cmap = codepoints.len();

    // Resolving a FreeType advance goes through SDF glyph generation, which is
    // far too slow to run over a full CJK cmap. Keep every non-ideograph (the
    // punctuation and halfwidth/fullwidth forms are exactly where the two
    // engines are known to disagree) and sample the ideographs evenly.
    const CJK_SAMPLE: usize = 400;
    let is_ideograph = |ch: &char| matches!(*ch, '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}');
    let ideographs: Vec<char> = codepoints.iter().copied().filter(is_ideograph).collect();
    let mut selected: Vec<char> = codepoints
        .iter()
        .copied()
        .filter(|c| !is_ideograph(c))
        .collect();
    let kept_non_ideograph = selected.len();
    if !ideographs.is_empty() {
        let step = (ideographs.len() / CJK_SAMPLE).max(1);
        selected.extend(ideographs.iter().step_by(step).copied());
    }
    selected.sort_unstable();
    let codepoints = selected;
    println!(
        "cmap codepoints: {total_cmap} (testing {}: all {kept_non_ideograph} non-ideographs + {} sampled ideographs)",
        codepoints.len(),
        codepoints.len() - kept_non_ideograph
    );

    let font_mgr = skia_safe::FontMgr::new();
    let Some(typeface) = font_mgr.new_from_data(&bytes, None) else {
        eprintln!("skia could not load the font face");
        std::process::exit(2);
    };

    for size in &sizes {
        let skia_font = skia_safe::Font::new(typeface.clone(), Some(*size));
        let mut compared = 0usize;
        let mut ft_missing = 0usize;
        // Difference histogram in absolute pixels. Comparing floats for exact
        // equality is meaningless at these magnitudes, so bucket by distance.
        let mut within_ulp = 0usize;
        let mut lt_001 = 0usize;
        let mut lt_01 = 0usize;
        let mut lt_05 = 0usize;
        let mut ge_05 = 0usize;
        let mut skia_is_round_of_ft = 0usize;
        let mut skia_is_integral = 0usize;
        let mut max_abs = 0.0f32;
        let mut max_abs_ch = ' ';
        let mut worst: Vec<(f32, f32, char, f32, f32)> = Vec::new();

        for &ch in &codepoints {
            let Some(ft) = freetype_advance(&family, ch, *size) else {
                ft_missing += 1;
                continue;
            };
            let sk = skia_advance(&ch.to_string(), &skia_font);
            compared += 1;
            let abs = (ft - sk).abs();
            let ulp = ft.abs().max(sk.abs()) * f32::EPSILON * 4.0;
            if abs <= ulp {
                within_ulp += 1;
            } else if abs < 0.01 {
                lt_001 += 1;
            } else if abs < 0.1 {
                lt_01 += 1;
            } else if abs < 0.5 {
                lt_05 += 1;
            } else {
                ge_05 += 1;
            }
            if sk == sk.round() {
                skia_is_integral += 1;
                if (sk - ft.round()).abs() <= ulp.max(1e-4) {
                    skia_is_round_of_ft += 1;
                }
            }
            if abs > max_abs {
                max_abs = abs;
                max_abs_ch = ch;
            }
            if abs > ulp {
                let rel = if sk != 0.0 {
                    (ft - sk) / sk * 100.0
                } else {
                    0.0
                };
                worst.push((abs, rel, ch, ft, sk));
            }
        }

        println!("\n=== size {size} ===");
        println!("compared               : {compared}");
        println!("FreeType had no glyph  : {ft_missing}");
        println!("\n  absolute difference |freetype - skia|, in pixels:");
        println!("    within float ULP    : {within_ulp}");
        println!("    < 0.01 px           : {lt_001}");
        println!("    < 0.1  px           : {lt_01}");
        println!("    < 0.5  px           : {lt_05}");
        println!("    >= 0.5 px           : {ge_05}");
        println!(
            "    max                 : {max_abs:.4} px (U+{:04X} {max_abs_ch:?})",
            u32::from(max_abs_ch)
        );
        println!("\n  skia advance is a whole number : {skia_is_integral} / {compared}");
        println!("  ... and equals round(freetype)  : {skia_is_round_of_ft} / {skia_is_integral}");

        worst.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        if !worst.is_empty() {
            println!("\n  largest absolute differences (FreeType vs Skia):");
            for (abs, rel, ch, ft, sk) in worst.iter().take(12) {
                println!(
                    "    U+{:04X} {:?}  freetype {ft:8.4}  skia {sk:8.4}  {abs:6.4} px ({rel:+.2}%)",
                    u32::from(*ch),
                    ch
                );
            }
        }
    }
}
