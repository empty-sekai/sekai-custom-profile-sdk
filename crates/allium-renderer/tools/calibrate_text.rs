//! 文本框模型标定工具：测量字符精确像素宽度，反推 padding 常量
//!
//! cargo run -p allium-renderer --example calibrate_text --features skia

use skia_safe::{Data, Font, FontMgr, FontStyle, Paint};
use std::path::Path;

fn main() {
    // 安装游戏字体
    let font_dir =
        std::env::var("FONT_DIR").unwrap_or_else(|_| "tmp/font_extract/exported".to_string());
    install_fonts(Path::new(&font_dir));

    let font_mgr = FontMgr::default();

    // 游戏默认字体: FOT-RodinNTLGPro-DB → Source Han Sans SC
    let typeface = font_mgr
        .match_family_style("Source Han Sans SC", FontStyle::default())
        .or_else(|| font_mgr.match_family_style("Noto Sans CJK SC", FontStyle::default()))
        .or_else(|| font_mgr.legacy_make_typeface(None, FontStyle::default()))
        .expect("无法获取字体");

    println!("字体: {}", typeface.family_name());
    println!();

    const TEXT_SCALE: f32 = 2.024;

    // 测量不同字号下"人"的像素宽度
    println!("=== 字符像素宽度测量 ===");
    println!(
        "{:<8} {:<12} {:<12} {:<12} {:<12}",
        "size", "人(px)", "t(px)", "人人(px)", "人t人t(px)"
    );

    let paint = Paint::default();

    for size in [12.0, 18.0, 24.0, 36.0, 48.0, 72.0, 96.0_f32] {
        let scaled_size = size * TEXT_SCALE;
        let font = Font::new(typeface.clone(), Some(scaled_size));
        let w_ren = font.measure_str("人", Some(&paint)).0;
        let w_t = font.measure_str("t", Some(&paint)).0;
        let w_2ren = font.measure_str("人人", Some(&paint)).0;
        let w_rtrt = font.measure_str("人t人t", Some(&paint)).0;

        println!(
            "{:<8.1} {:<12.2} {:<12.2} {:<12.2} {:<12.2}",
            size, w_ren, w_t, w_2ren, w_rtrt
        );
    }

    // 用 size=24 的数据标定（报告中最常用的字号）
    println!();
    println!("=== Padding 标定（假设报告的测量字号=24） ===");
    let cal_size = 24.0 * TEXT_SCALE;
    let cal_font = Font::new(typeface.clone(), Some(cal_size));
    let ren_px = cal_font.measure_str("人", Some(&paint)).0;

    // 报告: "人"最小字号 = 0.55cm
    // 注意: 24可能不是最小字号，需要用户确认
    let cm_char = 0.55_f32;
    let cm_to_px = ren_px / cm_char;

    let padding_total_cm = 1.55_f32;
    let m_anchor_cm = 0.4_f32;

    let padding_total_px = padding_total_cm * cm_to_px;
    let m_anchor_px = m_anchor_cm * cm_to_px;

    println!("  '人' @ size=24: {:.2} px", ren_px);
    println!("  cm → px 比例: {:.2} px/cm", cm_to_px);
    println!(
        "  PADDING_TOTAL = {:.1}cm × {:.2} = {:.2} px",
        padding_total_cm, cm_to_px, padding_total_px
    );
    println!(
        "  M_ANCHOR      = {:.1}cm × {:.2} = {:.2} px",
        m_anchor_cm, cm_to_px, m_anchor_px
    );
    println!("  PADDING / base_size = {:.4}", padding_total_px / cal_size);
    println!("  M_ANCHOR / base_size = {:.4}", m_anchor_px / cal_size);

    // 也用 't' 交叉验证
    println!();
    println!("=== 交叉验证 ===");

    // 报告: 't' 最小宽度 0.2cm, 两个 't' 时文本框 1.95cm
    // B = 1.95, W = 0.4, padding = 1.55 ✓
    let t_px = cal_font.measure_str("t", Some(&paint)).0;
    let t_cm = 0.2_f32; // 报告: 't' 最小
    let cm_to_px_t = t_px / t_cm;

    let padding_px_via_t = 1.55 * cm_to_px_t;
    println!("  't' @ size=24: {:.2} px", t_px);
    println!("  cm → px (via 't'): {:.2} px/cm", cm_to_px_t);
    println!("  PADDING via 't': {:.2} px", padding_px_via_t);

    // 比较两种标定
    println!();
    println!(
        "  标定差异: {:.1}%",
        (cm_to_px - cm_to_px_t).abs() / cm_to_px * 100.0
    );

    // 枚举不同字号的 padding 比例
    println!();
    println!("=== 各字号下 padding / scaled_size 比例 ===");
    for size in [12.0, 18.0, 24.0, 36.0, 48.0, 72.0, 96.0_f32] {
        let s = size * TEXT_SCALE;
        let f = Font::new(typeface.clone(), Some(s));
        let w = f.measure_str("人", Some(&paint)).0;
        let ratio = padding_total_px / s;
        println!(
            "  size={:<5.0} scaled={:<7.1} '人'={:<7.2} padding/scaled={:.4}",
            size, s, w, ratio
        );
    }

    println!();
    println!("=== 结论 ===");
    println!("  如果游戏的'最小字号'= size 24:");
    println!("    PADDING_TOTAL = {:.1} px (Skia 像素)", padding_total_px);
    println!("    M_ANCHOR      = {:.1} px", m_anchor_px);
    println!("  请用户确认报告中测量时的实际字号值。");
}

/// 安装字体
fn install_fonts(dir: &Path) {
    let font_mgr = FontMgr::default();
    let mut count = 0u32;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let ext_lower = ext.to_lowercase();
                if ext_lower == "otf" || ext_lower == "ttf" {
                    if let Ok(data) = std::fs::read(&path) {
                        let sk_data = Data::new_copy(&data);
                        if font_mgr.new_from_data(&sk_data, None).is_some() {
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    println!("✅ 安装了 {} 个自定义字体", count);
}
