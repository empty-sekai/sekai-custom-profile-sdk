// 检查字体加载情况的最小脚本
// cargo run --example check_fonts --features skia
use std::path::Path;

fn main() {
    // 安装字体
    let font_dir = Path::new("tmp/font_extract/exported");
    let font_mgr = skia_safe::FontMgr::default();

    println!("--- 安装前可用字体族 ---");
    for i in 0..font_mgr.count_families() {
        let name = font_mgr.family_name(i);
        println!("  {i}: {name}");
    }

    // 安装字体文件
    if font_dir.exists() {
        for entry in std::fs::read_dir(font_dir).unwrap().flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ext == "otf" || ext == "ttf" {
                    let data = std::fs::read(&path).unwrap();
                    let sk_data = skia_safe::Data::new_copy(&data);
                    match font_mgr.new_from_data(&sk_data, None) {
                        Some(tf) => println!(
                            "✅ {} → family=[{}]",
                            path.file_name().unwrap().to_string_lossy(),
                            tf.family_name()
                        ),
                        None => println!("❌ {}", path.file_name().unwrap().to_string_lossy()),
                    }
                }
            }
        }
    }

    // 测试 match_family_style
    println!("\n--- 测试 match_family_style ---");
    for name in &[
        "FOT-RodinNTLGPro-DB",
        "FOT-SkipProN-B",
        "FOT-PopHappinessStd-EB",
        "Noto Sans CJK SC",
        "Rodin",
    ] {
        let result = font_mgr.match_family_style(name, skia_safe::FontStyle::default());
        println!(
            "  {name}: {}",
            if result.is_some() {
                "FOUND"
            } else {
                "NOT FOUND"
            }
        );
    }
}
