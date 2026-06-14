//! 从 profile API JSON 渲染自定义名片全页，用于验证文本排版修复。
//!
//! 用法:
//!   render-profile <profile.json> <output.png> [seq] [--fonts=DIR] [--masterdata=DIR]
//!
//! 环境变量:
//!   FONT_DIR       字体目录（默认 assets/fonts）
//!   MASTERDATA_DIR masterdata 目录（默认 masterdata）

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing_subscriber::EnvFilter;

use allium_renderer::assets::AssetStore;
use allium_renderer::init::install_fonts;
use allium_renderer::masterdata::{MasterDataProvider, ResolvedColor, ResolvedHonor, ResourceInfo};
use allium_renderer::renderer::CustomProfileRenderer;
use allium_renderer::types::{
    BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry, UserCustomProfileCard,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FontEntry {
    id: i32,
    font_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ColorEntry {
    id: i32,
    color_code: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileInput {
    user_custom_profile_cards: Vec<UserCustomProfileCard>,
}

struct SimpleProvider {
    fonts: HashMap<i32, String>,
    colors: HashMap<i32, ResolvedColor>,
}

impl SimpleProvider {
    fn load(dir: &Path) -> Result<Self, String> {
        let fonts_path = dir.join("customProfileTextFonts.json");
        let colors_path = dir.join("customProfileTextColors.json");
        let fonts: Vec<FontEntry> = serde_json::from_str(
            &std::fs::read_to_string(&fonts_path)
                .map_err(|e| format!("读取 {} 失败: {e}", fonts_path.display()))?,
        )
        .map_err(|e| format!("解析 {} 失败: {e}", fonts_path.display()))?;
        let colors: Vec<ColorEntry> = serde_json::from_str(
            &std::fs::read_to_string(&colors_path)
                .map_err(|e| format!("读取 {} 失败: {e}", colors_path.display()))?,
        )
        .map_err(|e| format!("解析 {} 失败: {e}", colors_path.display()))?;
        let font_map = fonts
            .into_iter()
            .map(|e| (e.id, Self::map_font(&e.font_name)))
            .collect();
        let color_map = colors
            .into_iter()
            .filter_map(|e| ResolvedColor::from_hex(&e.color_code).map(|c| (e.id, c)))
            .collect();
        Ok(Self { fonts: font_map, colors: color_map })
    }

    fn map_font(name: &str) -> String {
        match name {
            "FOT-RodinNTLGPro-DB" => "FZLanTingHei-DB-GBK",
            "FOT-SkipProN-B" => "FZZhengHei-EB-GBK",
            "FOT-PopHappinessStd-EB" => "FZShaoEr-M11-JF",
            other => other,
        }
        .to_string()
    }
}

impl MasterDataProvider for SimpleProvider {
    fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> { None }
    fn get_card(&self, _: i32) -> Option<CardEntry> { None }
    fn resolve_color(&self, id: i32) -> Option<ResolvedColor> { self.colors.get(&id).copied() }
    fn resolve_font(&self, id: i32) -> Option<String> { self.fonts.get(&id).cloned() }
    fn resolve_stamp(&self, _: i32) -> Option<String> { None }
    fn resolve_resource(&self, _: &str, _: i32) -> Option<ResourceInfo> { None }
    fn resolve_honor(&self, _: i32, _: i32) -> Option<ResolvedHonor> { None }
    fn get_bonds_honor(&self, _: i32) -> Option<BondsHonorEntry> { None }
    fn get_bonds_honor_word(&self, _: i64) -> Option<BondsHonorWordEntry> { None }
    fn get_honor(&self, _: i32) -> Option<HonorEntry> { None }
    fn resolve_unit_vs_sd(&self, self_id: i32, _: i32) -> i32 { self_id }
    fn font_count(&self) -> usize { self.fonts.len() }
    fn color_count(&self) -> usize { self.colors.len() }
}

fn main() -> Result<(), String> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("allium_renderer::text=info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("用法: render-profile <profile.json> <output.png> [seq]");
        eprintln!("环境变量: FONT_DIR, MASTERDATA_DIR");
        std::process::exit(1);
    }

    let input = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[2]);
    let seq: i32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);

    let font_dir = std::env::var("FONT_DIR").unwrap_or_else(|_| "assets/fonts".to_string());
    let md_dir = std::env::var("MASTERDATA_DIR").unwrap_or_else(|_| "masterdata".to_string());

    install_fonts(Path::new(&font_dir))?;

    let provider = Arc::new(SimpleProvider::load(Path::new(&md_dir))?);
    let renderer_provider: Arc<dyn MasterDataProvider> = provider.clone();
    let renderer =
        CustomProfileRenderer::new(renderer_provider).with_assets(Arc::new(AssetStore::new(1)));

    let body: ProfileInput = serde_json::from_str(
        &std::fs::read_to_string(&input)
            .map_err(|e| format!("读取 {} 失败: {e}", input.display()))?,
    )
    .map_err(|e| format!("解析 {} 失败: {e}", input.display()))?;

    let card = body
        .user_custom_profile_cards
        .iter()
        .find(|c| c.seq == seq)
        .ok_or_else(|| format!("未找到 seq={seq}"))?;

    eprintln!("渲染 seq={} texts={}", seq, card.custom_profile_card.texts.len());

    let png = renderer
        .render_page_png_transparent_with_profile(&card.custom_profile_card, None)
        .map_err(|e| format!("渲染失败: {e}"))?;

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败: {e}"))?;
    }
    std::fs::write(&output, &png)
        .map_err(|e| format!("写入失败: {e}"))?;
    eprintln!("完成: {} ({} bytes)", output.display(), png.len());
    Ok(())
}
