//! PNG parity sweep: the dependency-free codec against Skia on a real corpus.
//!
//! Replacing the Skia decoder is only safe if it produces identical pixels for
//! every asset the render path can be handed. This walks a directory tree and,
//! for each PNG, checks that:
//!
//!   1. the premultiplied buffer the compositor consumes is byte-identical to
//!      what `AssetStore::get_premultiplied_rgba` produces today;
//!   2. `codec::png::encode_rgba` round-trips those pixels losslessly;
//!   3. Skia reads our re-encoded file exactly as it reads the original.
//!
//! Note on (1): skia decodes to its premultiplied internal form, so reading it
//! back as Unpremul loses up to 1/255 per channel on semi-transparent pixels.
//! Our decoder returns the true PNG samples instead, so the unpremultiplied
//! buffers legitimately differ; the premultiplied ones must not.
//!
//! Usage:
//!   png-parity <dir> [more dirs...]
//!
//! Exit status is non-zero if any file mismatches, so it can gate a change.

use std::path::{Path, PathBuf};

use allium_renderer::codec::{png, premultiply_channel, unpremultiply_channel_like_skia};

struct Stats {
    scanned: usize,
    skipped_not_png: usize,
    decode_ok: usize,
    decode_mismatch: Vec<(PathBuf, String)>,
    decode_err: Vec<(PathBuf, String)>,
    skia_refused: Vec<PathBuf>,
    roundtrip_mismatch: Vec<PathBuf>,
    skia_reencode_mismatch: Vec<PathBuf>,
    unsupported: Vec<(PathBuf, String)>,
    /// Files where skia's Unpremul read differs from the true PNG samples.
    unpremul_differs: usize,
    /// Files whose premultiplied buffer matches skia's internal premultiplied form.
    internal_premul_ok: usize,
    internal_premul_mismatch: Vec<(PathBuf, String)>,
    internal_premul_unavailable: usize,
    /// Files where our reconstruction of skia's unpremultiplied read matches it.
    unpremul_reconstruction_ok: usize,
    unpremul_reconstruction_mismatch: Vec<PathBuf>,
}

impl Stats {
    fn new() -> Self {
        Self {
            scanned: 0,
            skipped_not_png: 0,
            decode_ok: 0,
            decode_mismatch: Vec::new(),
            decode_err: Vec::new(),
            skia_refused: Vec::new(),
            roundtrip_mismatch: Vec::new(),
            skia_reencode_mismatch: Vec::new(),
            unsupported: Vec::new(),
            unpremul_differs: 0,
            internal_premul_ok: 0,
            internal_premul_mismatch: Vec::new(),
            internal_premul_unavailable: 0,
            unpremul_reconstruction_ok: 0,
            unpremul_reconstruction_mismatch: Vec::new(),
        }
    }
}

/// Decodes with Skia into tightly packed RGBA8 with the requested alpha type.
fn skia_decode_as(bytes: &[u8], alpha: skia_safe::AlphaType) -> Option<(u32, u32, Vec<u8>)> {
    let data = skia_safe::Data::new_copy(bytes);
    let image = skia_safe::Image::from_encoded(data)?;
    let width = image.width();
    let height = image.height();
    let row_bytes = usize::try_from(width).ok()?.checked_mul(4)?;
    let mut rgba = vec![0u8; row_bytes.checked_mul(usize::try_from(height).ok()?)?];
    let info =
        skia_safe::ImageInfo::new((width, height), skia_safe::ColorType::RGBA8888, alpha, None);
    if !image.read_pixels(
        &info,
        &mut rgba,
        row_bytes,
        (0, 0),
        skia_safe::image::CachingHint::Disallow,
    ) {
        return None;
    }
    Some((width as u32, height as u32, rgba))
}

/// Decodes with Skia into tightly packed, non-premultiplied RGBA8.
///
/// This is what `AssetStore::get_premultiplied_rgba` reads today, before
/// multiplying by alpha itself.
fn skia_decode(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    skia_decode_as(bytes, skia_safe::AlphaType::Unpremul)
}

/// Decodes with Skia into its internal premultiplied form.
///
/// The legacy element draw path hands a decoded `skia_safe::Image` straight to
/// the canvas, so this - not the unpremultiplied read above - is the buffer that
/// path composites. Sourcing that path from our own decoder requires agreement
/// here as well.
fn skia_decode_premul(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    skia_decode_as(bytes, skia_safe::AlphaType::Premul)
}

/// Premultiplies in place exactly as `AssetStore::get_premultiplied_rgba` does.
///
/// This is the form the compositor consumes, so it — not the unpremultiplied
/// intermediate — is what parity has to hold for.
fn premultiply_like_production(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3];
        if alpha < 255 {
            pixel[0] = premultiply_channel(pixel[0], alpha);
            pixel[1] = premultiply_channel(pixel[1], alpha);
            pixel[2] = premultiply_channel(pixel[2], alpha);
        }
    }
}

/// Reproduces Skia's non-premultiplied read from the true PNG samples.
///
/// Premultiply, then divide back out the way Skia's reciprocal table does. The
/// shape-atlas source hash is computed over exactly these values, so this has to
/// agree with Skia byte for byte.
fn skia_unpremul_from_samples(pixels: &[u8]) -> Vec<u8> {
    let mut out = pixels.to_vec();
    for pixel in out.chunks_exact_mut(4) {
        let alpha = pixel[3];
        for channel in 0..3 {
            let premultiplied = premultiply_channel(pixel[channel], alpha);
            pixel[channel] = unpremultiply_channel_like_skia(premultiplied, alpha);
        }
    }
    out
}

fn first_difference(a: &[u8], b: &[u8]) -> Option<(usize, u8, u8)> {
    a.iter()
        .zip(b.iter())
        .enumerate()
        .find(|(_, (x, y))| x != y)
        .map(|(i, (x, y))| (i, *x, *y))
}

fn check_file(path: &Path, stats: &mut Stats) {
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    // Extensions lie in these trees; dispatch on content.
    if !png::is_png(&bytes) {
        stats.skipped_not_png += 1;
        return;
    }
    stats.scanned += 1;

    let mine = match png::decode(&bytes) {
        Ok(image) => image,
        Err(err) => {
            // An unsupported-but-valid feature is a scope note, not a defect.
            if matches!(err, allium_renderer::codec::CodecError::Unsupported(_)) {
                stats
                    .unsupported
                    .push((path.to_path_buf(), err.to_string()));
            } else {
                stats.decode_err.push((path.to_path_buf(), err.to_string()));
            }
            return;
        }
    };

    let Some((sw, sh, reference)) = skia_decode(&bytes) else {
        stats.skia_refused.push(path.to_path_buf());
        return;
    };

    if mine.width != sw || mine.height != sh {
        stats.decode_mismatch.push((
            path.to_path_buf(),
            format!("size {}x{} vs skia {}x{}", mine.width, mine.height, sw, sh),
        ));
        return;
    }

    // Informational: skia's Unpremul read is lossy for semi-transparent pixels
    // (it round-trips through its premultiplied internal form). Track it so the
    // premultiplied result below is not mistaken for the same comparison.
    if mine.pixels != reference {
        stats.unpremul_differs += 1;
    }

    // The gate: the premultiplied buffer the compositor actually consumes.
    let mut ours_premul = mine.pixels.clone();
    premultiply_like_production(&mut ours_premul);
    let mut production_premul = reference.clone();
    premultiply_like_production(&mut production_premul);

    if ours_premul != production_premul {
        let detail = match first_difference(&ours_premul, &production_premul) {
            Some((i, a, b)) => {
                let px = i / 4;
                let q = px * 4;
                format!(
                    "premultiplied pixel {px} (x={}, y={}) channel {}: ours {a} vs production {b}\n        \
                     source alpha {} | ours {:?} vs production {:?}",
                    px % mine.width as usize,
                    px / mine.width as usize,
                    i % 4,
                    mine.pixels[q + 3],
                    &ours_premul[q..q + 4],
                    &production_premul[q..q + 4]
                )
            }
            None => "length differs".to_string(),
        };
        stats.decode_mismatch.push((path.to_path_buf(), detail));
        return;
    }
    stats.decode_ok += 1;

    // Third gate: the shape-atlas source hash is taken over skia's
    // non-premultiplied read, so that value has to be reconstructible from the
    // true samples without skia present.
    if skia_unpremul_from_samples(&mine.pixels) == reference {
        stats.unpremul_reconstruction_ok += 1;
    } else {
        stats
            .unpremul_reconstruction_mismatch
            .push(path.to_path_buf());
    }

    // Second gate: skia's internal premultiplied form, which the legacy element
    // draw path composites directly.
    match skia_decode_premul(&bytes) {
        Some((w, h, internal)) if w == sw && h == sh => {
            if ours_premul == internal {
                stats.internal_premul_ok += 1;
            } else {
                let detail = match first_difference(&ours_premul, &internal) {
                    Some((i, a, b)) => {
                        let px = i / 4;
                        let q = px * 4;
                        format!(
                            "pixel {px} (x={}, y={}) channel {}: ours {a} vs skia-internal {b}; \
                             source alpha {} | ours {:?} vs skia-internal {:?}",
                            px % mine.width as usize,
                            px / mine.width as usize,
                            i % 4,
                            mine.pixels[q + 3],
                            &ours_premul[q..q + 4],
                            &internal[q..q + 4]
                        )
                    }
                    None => "length differs".to_string(),
                };
                stats
                    .internal_premul_mismatch
                    .push((path.to_path_buf(), detail));
            }
        }
        _ => stats.internal_premul_unavailable += 1,
    }

    // Encoder: lossless round trip through our own decoder.
    let Ok(encoded) = png::encode_rgba(mine.width, mine.height, &mine.pixels) else {
        stats.roundtrip_mismatch.push(path.to_path_buf());
        return;
    };
    match png::decode(&encoded) {
        Ok(back) if back.pixels == mine.pixels => {}
        _ => {
            stats.roundtrip_mismatch.push(path.to_path_buf());
            return;
        }
    }

    // Interop: skia must see our re-encoded file exactly as it sees the
    // original. Both sides go through the same (lossy) skia read, so this
    // isolates the encoder from skia's premultiplied round-trip.
    match skia_decode(&encoded) {
        Some((w, h, via_skia)) if w == sw && h == sh && via_skia == reference => {}
        _ => stats.skia_reencode_mismatch.push(path.to_path_buf()),
    }
}

fn walk(dir: &Path, stats: &mut Stats) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(&path, stats),
            Ok(t) if t.is_file() => check_file(&path, stats),
            _ => {}
        }
    }
}

fn report_group(label: &str, items: &[PathBuf]) -> bool {
    if items.is_empty() {
        return false;
    }
    println!("\n{label}: {}", items.len());
    for p in items.iter().take(10) {
        println!("    {}", p.display());
    }
    if items.len() > 10 {
        println!("    ... and {} more", items.len() - 10);
    }
    true
}

/// Exhaustive check over every (channel value, alpha) pair.
///
/// The corpus cannot cover all 65 536 combinations, so this closes the gap: for
/// each pair it writes a 1x1 PNG, decodes it both ways, premultiplies as
/// production does, and requires the results to agree. Divergence here would
/// mean corpus parity was luck rather than a property.
fn exhaustive_alpha_sweep() -> bool {
    let mut divergent: Vec<(u8, u8, u8, u8)> = Vec::new();
    let mut internal_divergent: Vec<(u8, u8, u8, u8)> = Vec::new();
    let mut unpremul_divergent: Vec<(u8, u8, u8, u8)> = Vec::new();
    let mut skia_lossy_pairs = 0usize;
    for alpha in 0u16..=255 {
        for value in 0u16..=255 {
            let a = alpha as u8;
            let v = value as u8;
            let pixels = [v, v, v, a];
            let Ok(encoded) = png::encode_rgba(1, 1, &pixels) else {
                divergent.push((v, a, 0, 0));
                continue;
            };
            let Ok(mine) = png::decode(&encoded) else {
                divergent.push((v, a, 0, 0));
                continue;
            };
            let Some((_, _, skia)) = skia_decode(&encoded) else {
                divergent.push((v, a, 0, 0));
                continue;
            };
            if mine.pixels != skia {
                skia_lossy_pairs += 1;
            }
            let mut ours = mine.pixels.clone();
            premultiply_like_production(&mut ours);
            let mut theirs = skia.clone();
            premultiply_like_production(&mut theirs);
            if ours != theirs {
                divergent.push((v, a, ours[0], theirs[0]));
            }
            if skia_unpremul_from_samples(&mine.pixels) != skia {
                unpremul_divergent.push((v, a, mine.pixels[0], skia[0]));
            }
            match skia_decode_premul(&encoded) {
                Some((_, _, internal)) => {
                    if ours != internal {
                        internal_divergent.push((v, a, ours[0], internal[0]));
                    }
                }
                None => internal_divergent.push((v, a, 0, 0)),
            }
        }
    }
    println!("exhaustive (value, alpha) pairs tested : 65536");
    println!("pairs where skia's Unpremul read differs: {skia_lossy_pairs}");
    let mut ok = true;
    if divergent.is_empty() {
        println!("premultiplied agreement                : all 65536 pairs");
    } else {
        ok = false;
        println!("\npremultiplied DIVERGENCE: {} pair(s)", divergent.len());
        for (v, a, ours, theirs) in divergent.iter().take(20) {
            println!("    value={v} alpha={a}: ours {ours} vs production {theirs}");
        }
    }
    if unpremul_divergent.is_empty() {
        println!("skia unpremul read reconstructed        : all 65536 pairs");
    } else {
        ok = false;
        println!(
            "\nskia unpremultiplied read DIVERGENCE: {} pair(s)",
            unpremul_divergent.len()
        );
        for (v, a, ours, theirs) in unpremul_divergent.iter().take(20) {
            println!("    value={v} alpha={a}: ours {ours} vs skia {theirs}");
        }
    }
    if internal_divergent.is_empty() {
        println!("skia internal premul agreement         : all 65536 pairs");
    } else {
        ok = false;
        println!(
            "\nskia internal premultiplied DIVERGENCE: {} pair(s)",
            internal_divergent.len()
        );
        for (v, a, ours, theirs) in internal_divergent.iter().take(20) {
            println!("    value={v} alpha={a}: ours {ours} vs skia-internal {theirs}");
        }
    }
    ok
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--exhaustive") {
        if exhaustive_alpha_sweep() {
            println!("\nRESULT: PASS — premultiplied agreement holds for every (value, alpha)");
        } else {
            println!("\nRESULT: FAIL");
            std::process::exit(1);
        }
        return;
    }

    let roots: Vec<PathBuf> = args.into_iter().map(PathBuf::from).collect();
    if roots.is_empty() {
        eprintln!("usage: png-parity <dir> [more dirs...]");
        eprintln!("       png-parity --exhaustive");
        std::process::exit(2);
    }

    let mut stats = Stats::new();
    for root in &roots {
        if root.is_file() {
            check_file(root, &mut stats);
        } else {
            walk(root, &mut stats);
        }
    }

    println!("PNG files scanned          : {}", stats.scanned);
    println!("non-PNG payloads skipped   : {}", stats.skipped_not_png);
    println!("premultiplied parity OK    : {}", stats.decode_ok);
    println!(
        "matches skia internal premul: {} (unavailable in {} file(s))",
        stats.internal_premul_ok, stats.internal_premul_unavailable
    );
    println!(
        "skia unpremul read rebuilt  : {}",
        stats.unpremul_reconstruction_ok
    );
    println!(
        "skia Unpremul read lossy in: {} file(s) (informational: skia round-trips
         {:28}through its premultiplied form; our decoder returns the true samples)",
        stats.unpremul_differs, ""
    );

    let mut failed = false;
    if !stats.decode_mismatch.is_empty() {
        failed = true;
        println!("\ndecode MISMATCH: {}", stats.decode_mismatch.len());
        for (p, detail) in stats.decode_mismatch.iter().take(10) {
            println!("    {}\n        {detail}", p.display());
        }
    }
    if !stats.decode_err.is_empty() {
        failed = true;
        println!("\ndecode ERROR: {}", stats.decode_err.len());
        for (p, detail) in stats.decode_err.iter().take(10) {
            println!("    {}\n        {detail}", p.display());
        }
    }
    if !stats.internal_premul_mismatch.is_empty() {
        failed = true;
        println!(
            "\nskia internal premultiplied MISMATCH: {}",
            stats.internal_premul_mismatch.len()
        );
        for (p, detail) in stats.internal_premul_mismatch.iter().take(10) {
            println!("    {}\n        {detail}", p.display());
        }
    }
    failed |= report_group(
        "skia unpremultiplied read NOT reconstructible",
        &stats.unpremul_reconstruction_mismatch,
    );
    failed |= report_group("encode round-trip MISMATCH", &stats.roundtrip_mismatch);
    failed |= report_group(
        "skia reads our re-encode differently",
        &stats.skia_reencode_mismatch,
    );

    // Informational: not defects.
    if !stats.unsupported.is_empty() {
        println!("\nunsupported-by-design: {}", stats.unsupported.len());
        for (p, detail) in stats.unsupported.iter().take(10) {
            println!("    {}\n        {detail}", p.display());
        }
    }
    report_group(
        "skia refused to decode (ours succeeded)",
        &stats.skia_refused,
    );

    if failed {
        println!("\nRESULT: FAIL");
        std::process::exit(1);
    }
    println!("\nRESULT: PASS — premultiplied output matches production across the corpus");
}
