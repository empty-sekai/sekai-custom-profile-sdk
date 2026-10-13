//! 渲染引擎初始化（启动时一次性操作）
//!
//! 与渲染逻辑分离的基础设施关注点。

use std::path::Path;

/// 游戏字体白名单（fontId → 文件名）
///
/// exported/ 下同时存在 .otf（Source Han Sans SC）和 .ttf（FZLanTingHei），
/// 两者共存会导致 fontconfig 冲突。游戏实际使用 .ttf。
const FONT_WHITELIST: [&str; 3] = [
    "FOT-RodinNTLGPro-DB.ttf",    // fontId=1 → FZLanTingHei-DB-GBK
    "FOT-SkipProN-B.otf",         // fontId=2 → FZZhengHei-EB-GBK
    "FOT-PopHappinessStd-EB.otf", // fontId=3 → FZShaoEr-M11-JF
];

/// 安装游戏字体到系统目录（生产环境启动时调用一次）
///
/// 白名单模式：每次清空重建，只安装 3 个正确字体文件，避免冲突。
pub fn install_fonts(font_dir: &Path) -> Result<u32, String> {
    let target = Path::new("/usr/share/fonts/custom");
    let _ = std::fs::remove_dir_all(target);
    std::fs::create_dir_all(target).map_err(|e| format!("创建字体目录失败: {e}"))?;

    let mut installed = 0u32;
    for name in &FONT_WHITELIST {
        let src = font_dir.join(name);
        let dest = target.join(name);
        if src.exists() {
            std::fs::copy(&src, &dest).map_err(|e| format!("拷贝字体失败 {name}: {e}"))?;
            installed += 1;
        } else {
            tracing::warn!(file = %src.display(), "字体文件不存在");
        }
    }

    if installed > 0 {
        let _ = std::process::Command::new("fc-cache").arg("-f").status();
    }
    tracing::info!(installed, total = FONT_WHITELIST.len(), "游戏字体安装完成");
    Ok(installed)
}
