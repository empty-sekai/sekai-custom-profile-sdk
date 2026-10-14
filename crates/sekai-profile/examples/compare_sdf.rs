//! Compares the accuracy and speed of SDF generation: the analytic method
//! (the reference) against the EDT method.
//!
//! Usage:
//!   cargo run -p sekai-profile --features dev --example compare_sdf -- \
//!     <font_family> <chars> [supersample]
//!
//! Example:
//!   cargo run -p sekai-profile --features dev --example compare_sdf -- \
//!     FOT-RodinNTLGPro-DB "Ag1" 2
//!
//! Without arguments it compares `FOT-RodinNTLGPro-DB` on a few kanji and
//! Latin characters at 2x supersampling. Fonts are looked up in the
//! directories `SEKAI_PROFILE_FONT_DIR` and `FONT_DIR` name. With `DUMP` set,
//! it prints both SDFs of the first character as ASCII art instead.
//!
//! The renderer uses the analytic method by default, so it serves as the
//! reference; a smaller EDT error is better.

use sekai_profile::sdf::outline::benchmark_methods;

/// Maps a 0–255 gray value to an ASCII shade, to inspect an SDF in a terminal.
fn gray_ch(g: u8) -> char {
    let ramp = [' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];
    ramp[(g as usize * (ramp.len() - 1)) / 255]
}

fn dump_ascii(label: &str, glyph: &sekai_profile::sdf::outline::OutlineSdfGlyph) {
    println!("--- {label} {}x{} ---", glyph.width(), glyph.height());
    let px = glyph.pixels();
    let (w, h) = (glyph.width(), glyph.height());
    // Subsample rows and columns to at most 40 wide and 24 high.
    let step_x = (w / 40).max(1);
    let step_y = (h / 24).max(1);
    for y in (0..h).step_by(step_y) {
        let mut line = String::new();
        for x in (0..w).step_by(step_x) {
            line.push(gray_ch(px[y * w + x]));
        }
        println!("{line}");
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();

    let mut args = std::env::args().skip(1);
    let family = args
        .next()
        .unwrap_or_else(|| "FOT-RodinNTLGPro-DB".to_string());
    let chars = args.next().unwrap_or_else(|| "永和国A1".to_string());
    let supersample: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(2);
    let dump = std::env::var("DUMP").is_ok();

    if dump {
        // Dump mode: show both SDFs of the first character.
        if let Some(ch) = chars.chars().next() {
            if let Some((analytic, _, edt, _)) = benchmark_methods(&family, ch, supersample) {
                println!("char '{ch}' supersample={supersample}x");
                dump_ascii("analytic (reference)", &analytic);
                dump_ascii("EDT", &edt);
            }
        }
        return;
    }

    println!(
        "font={family} supersample={supersample}x\n{:<6} {:>9} {:>9} {:>8} {:>8} {:>9} {:>9} {:>7}",
        "char", "ana_us", "edt_us", "speedup", "MAE", "max_err", "size", "px"
    );
    println!("{}", "-".repeat(78));

    let mut total_mae = 0.0f64;
    let mut total_max = 0.0f32;
    let mut count = 0;
    let mut sum_speedup = 0.0f64;
    let mut total_err_flat = 0.0f64;
    let mut total_err_edge = 0.0f64;
    let mut total_flat_max = 0.0f32;

    for ch in chars.chars() {
        let Some((analytic, a_dur, edt, e_dur)) = benchmark_methods(&family, ch, supersample)
        else {
            println!("{ch:<6} generation failed");
            continue;
        };

        let size_match = analytic.width() == edt.width() && analytic.height() == edt.height();
        if !size_match {
            println!(
                "{ch:<6} size mismatch: analytic={}x{} EDT={}x{}",
                analytic.width(),
                analytic.height(),
                edt.width(),
                edt.height()
            );
            continue;
        }

        let ap = analytic.pixels();
        let ep = edt.pixels();
        let n = ap.len();
        let mut sum_abs = 0.0f64;
        let mut max_abs = 0.0f32;
        // Split the error by region: the edge band (analytic gray in
        // [0.15, 0.85], which the shader turns into the antialiased soft edge,
        // so errors there are harmless) and the flat region (fully inside or
        // outside, where errors become visible artifacts).
        let mut err_edge = 0.0f64;
        let mut err_flat = 0.0f64;
        let mut flat_max = 0.0f32;
        for i in 0..n {
            let diff = (ap[i] as f32 - ep[i] as f32).abs();
            sum_abs += diff as f64;
            max_abs = max_abs.max(diff);
            let a_norm = ap[i] as f32 / 255.0;
            if (0.15..=0.85).contains(&a_norm) {
                err_edge += diff as f64;
            } else {
                err_flat += diff as f64;
                flat_max = flat_max.max(diff);
            }
        }
        let mae = sum_abs / n as f64;
        let speedup = a_dur.as_secs_f64() / e_dur.as_secs_f64().max(1e-9);

        total_mae += mae;
        total_max = total_max.max(max_abs);
        total_flat_max = total_flat_max.max(flat_max);
        total_err_flat += err_flat;
        total_err_edge += err_edge;
        sum_speedup += speedup;
        count += 1;

        println!(
            "{ch:<6} {:>9.1} {:>9.1} {:>7.1}x {:>8.2} {:>9.0} {:>9} {:>7}",
            a_dur.as_secs_f64() * 1e6,
            e_dur.as_secs_f64() * 1e6,
            speedup,
            mae,
            max_abs,
            "OK",
            n
        );
    }

    if count > 0 {
        println!("{}", "-".repeat(78));
        println!(
            "mean: MAE={:.2}/255 ({:.2}%)  max error={:.0}/255  mean speedup={:.1}x  samples={}",
            total_mae / count as f64,
            total_mae / count as f64 / 255.0 * 100.0,
            total_max,
            sum_speedup / count as f64,
            count
        );
        let total_err = total_err_flat + total_err_edge;
        println!(
            "error split: edge band={:.1}%  flat region={:.1}%  flat max error={:.0}/255",
            total_err_edge / total_err.max(1e-9) * 100.0,
            total_err_flat / total_err.max(1e-9) * 100.0,
            total_flat_max
        );
        println!("(edge-band error falls in the antialiased soft edge; flat-region error shows as visible artifacts)");
    }
}
