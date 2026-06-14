//! SDF 生成精度 + 性能对比：解析法（真值）vs EDT 法。
//!
//! 用法:
//!   compare-sdf <font_family> <chars> [supersample]
//!
//! 例:
//!   compare-sdf FOT-RodinNTLGPro-DB "永和あ国A1" 2
//!
//! 环境变量: SCAPUS_FONT_DIR 字体目录
//!
//! 解析法是当前生产用的、视觉正确的输出，作为真值。EDT 法误差越小越好。

use allium_renderer::sdf::outline::benchmark_methods;

/// 把 0-255 gray 映射成 ASCII 浓淡，便于在终端目视 SDF 结构。
fn gray_ch(g: u8) -> char {
    let ramp = [' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];
    ramp[(g as usize * (ramp.len() - 1)) / 255]
}

fn dump_ascii(label: &str, glyph: &allium_renderer::sdf::outline::OutlineSdfGlyph) {
    println!("--- {label} {}x{} ---", glyph.width(), glyph.height());
    let px = glyph.pixels();
    let (w, h) = (glyph.width(), glyph.height());
    // 行列各抽样到 ≤40 宽 / ≤24 高，避免刷屏
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
    let family = args.next().unwrap_or_else(|| "FOT-RodinNTLGPro-DB".to_string());
    let chars = args.next().unwrap_or_else(|| "永和国A1".to_string());
    let supersample: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(2);
    let dump = std::env::var("DUMP").is_ok();

    if dump {
        // dump 模式：可视化第一个字符的两种 SDF
        if let Some(ch) = chars.chars().next() {
            if let Some((analytic, _, edt, _)) = benchmark_methods(&family, ch, supersample) {
                println!("字符 '{ch}' 超采样={supersample}x");
                dump_ascii("解析法(真值)", &analytic);
                dump_ascii("EDT法", &edt);
            }
        }
        return;
    }

    println!(
        "字体={family} 超采样={supersample}x\n{:<6} {:>9} {:>9} {:>8} {:>8} {:>9} {:>9} {:>7}",
        "char", "解析us", "EDTus", "加速", "MAE", "最大误差", "尺寸匹配", "px数"
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
        let Some((analytic, a_dur, edt, e_dur)) = benchmark_methods(&family, ch, supersample) else {
            println!("{ch:<6} 生成失败");
            continue;
        };

        let size_match = analytic.width() == edt.width() && analytic.height() == edt.height();
        if !size_match {
            println!(
                "{ch:<6} 尺寸不匹配! 解析={}x{} EDT={}x{}",
                analytic.width(), analytic.height(), edt.width(), edt.height()
            );
            continue;
        }

        let ap = analytic.pixels();
        let ep = edt.pixels();
        let n = ap.len();
        let mut sum_abs = 0.0f64;
        let mut max_abs = 0.0f32;
        // 误差归类：边缘过渡带（解析法 gray 在 [0.15,0.85]，喂给 shader 后是
        // 抗锯齿软边，误差无害）vs 平坦区（纯内/纯外，误差会变成可见瑕疵）。
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
            "平均: MAE={:.2}/255 ({:.2}%)  最大误差={:.0}/255  平均加速={:.1}x  样本={}",
            total_mae / count as f64,
            total_mae / count as f64 / 255.0 * 100.0,
            total_max,
            sum_speedup / count as f64,
            count
        );
        let total_err = total_err_flat + total_err_edge;
        println!(
            "误差分布: 边缘过渡带={:.1}%  平坦区={:.1}%  平坦区最大误差={:.0}/255",
            total_err_edge / total_err.max(1e-9) * 100.0,
            total_err_flat / total_err.max(1e-9) * 100.0,
            total_flat_max
        );
        println!("（边缘带误差=抗锯齿软边，视觉无害；平坦区误差才会成为可见瑕疵）");
    }
}
