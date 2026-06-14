//! render-card：自定义名片渲染 CLI。
//!
//! 两种运行模式：
//! - 单次模式：渲染一张名片后退出。
//! - `--serve` 常驻模式：stdin/stdout NDJSON 协议，请求严格串行，
//!   字体 / masterdata / glyph 缓存 / AssetStore 跨请求常驻。
//!   日志只走 stderr。
//!
//! 用法：
//!   render-card --masterdata <dir> --card <card.json> -o <out.jpg> \
//!       [--profile <profile.json>] [--assets-dir <dir>] [--font-dir <dir>] \
//!       [--format jpeg|png|png-transparent] [--page <seq>]
//!   render-card --serve --masterdata <dir> [--assets-dir <dir>] [--font-dir <dir>]

mod serve;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use allium_renderer::assets::AssetStore;
use allium_renderer::profile::ProfileData;
use allium_renderer::renderer::CustomProfileRenderer;
use allium_renderer::types::{CustomProfileCard, UserCustomProfileCard};
use allium_renderer_host::JsonMasterDataProvider;

const USAGE: &str = "\
render-card：自定义名片渲染 CLI

单次模式：
  render-card --masterdata <dir> --card <card.json> -o <out>
      [--profile <profile.json>] [--assets-dir <dir>] [--font-dir <dir>]
      [--format jpeg|png|png-transparent] [--page <seq>]

常驻模式（stdin/stdout NDJSON）：
  render-card --serve --masterdata <dir> [--assets-dir <dir>] [--font-dir <dir>]

参数：
  --masterdata <dir>   masterdata JSON 表目录（<dir>/<table>.json）
  --card <file>        名片 JSON：CustomProfileCard 或 UserCustomProfileCard 数组
  --page <seq>         --card 为数组时选择的页码（默认第一张）
  --profile <file>     profile API 响应 JSON（注入 generals 数据与称号等级）
  --assets-dir <dir>   素材目录（key = 相对路径去扩展名）
  --font-dir <dir>     字体目录（等效 SCAPUS_FONT_DIR）
  --format <fmt>       输出格式：jpeg（默认）/ png / png-transparent
  -o <file>            输出文件路径（单次模式必填）
  --serve              常驻模式
";

struct Args {
    masterdata: Option<PathBuf>,
    card: Option<PathBuf>,
    page: Option<i32>,
    profile: Option<PathBuf>,
    assets_dir: Option<PathBuf>,
    font_dir: Option<PathBuf>,
    format: String,
    output: Option<PathBuf>,
    serve: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        masterdata: None,
        card: None,
        page: None,
        profile: None,
        assets_dir: None,
        font_dir: None,
        format: "jpeg".into(),
        output: None,
        serve: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut take = |name: &str| -> Result<String, String> {
            it.next().ok_or_else(|| format!("{name} 缺少参数值"))
        };
        match arg.as_str() {
            "--masterdata" => args.masterdata = Some(PathBuf::from(take("--masterdata")?)),
            "--card" => args.card = Some(PathBuf::from(take("--card")?)),
            "--page" => {
                args.page = Some(
                    take("--page")?
                        .parse()
                        .map_err(|e| format!("--page 解析失败: {e}"))?,
                )
            }
            "--profile" => args.profile = Some(PathBuf::from(take("--profile")?)),
            "--assets-dir" => args.assets_dir = Some(PathBuf::from(take("--assets-dir")?)),
            "--font-dir" => args.font_dir = Some(PathBuf::from(take("--font-dir")?)),
            "--format" => args.format = take("--format")?,
            "-o" | "--output" => args.output = Some(PathBuf::from(take("-o")?)),
            "--serve" => args.serve = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("未知参数: {other}")),
        }
    }
    Ok(args)
}

/// 解析 --card 文件：直接的 CustomProfileCard 或 UserCustomProfileCard 数组。
fn load_card(path: &PathBuf, page: Option<i32>) -> Result<CustomProfileCard, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("解析名片 JSON 失败: {e}"))?;
    card_from_value(value, page)
}

/// 从 JSON 值解析名片（serve 模式复用）。
fn card_from_value(value: serde_json::Value, page: Option<i32>) -> Result<CustomProfileCard, String> {
    if value.is_array() {
        let cards: Vec<UserCustomProfileCard> =
            serde_json::from_value(value).map_err(|e| format!("解析名片数组失败: {e}"))?;
        if cards.is_empty() {
            return Err("名片数组为空".into());
        }
        let card = match page {
            Some(seq) => cards
                .into_iter()
                .find(|c| c.seq == seq)
                .ok_or_else(|| format!("未找到 seq={seq} 的名片"))?,
            None => cards.into_iter().next().expect("非空数组"),
        };
        Ok(card.custom_profile_card)
    } else {
        serde_json::from_value(value).map_err(|e| format!("解析 CustomProfileCard 失败: {e}"))
    }
}

/// 加载 profile 并填充称号等级。
fn load_profile(
    path: &PathBuf,
    renderer: &CustomProfileRenderer,
    card: &mut CustomProfileCard,
) -> Result<ProfileData, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    let body: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("解析 profile JSON 失败: {e}"))?;
    Ok(enrich_from_profile_value(&body, renderer, card))
}

/// 从 profile JSON 值构建 ProfileData 并填充名片称号等级（serve 模式复用）。
fn enrich_from_profile_value(
    body: &serde_json::Value,
    renderer: &CustomProfileRenderer,
    card: &mut CustomProfileCard,
) -> ProfileData {
    let profile = ProfileData::from_json(body);
    let (honor_levels, bonds_levels, char_ranks) =
        allium_renderer::profile::build_honor_maps(body);
    renderer.enrich_honor_levels(card, &honor_levels, &bonds_levels, &char_ranks);
    profile
}

/// 按 --assets-dir 注入素材：key = 相对路径去掉 .png/.jpg 扩展名。
fn load_assets_dir(store: &AssetStore, dir: &std::path::Path) -> Result<usize, String> {
    fn walk(
        base: &std::path::Path,
        dir: &std::path::Path,
        store: &AssetStore,
        count: &mut usize,
    ) -> Result<(), String> {
        let entries =
            std::fs::read_dir(dir).map_err(|e| format!("读取目录 {} 失败: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("遍历目录失败: {e}"))?;
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, store, count)?;
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let ext = ext.to_lowercase();
                if ext == "png" || ext == "jpg" || ext == "webp" {
                    let rel = path
                        .strip_prefix(base)
                        .map_err(|e| format!("路径前缀错误: {e}"))?;
                    let key = rel
                        .to_string_lossy()
                        .replace('\\', "/")
                        .trim_end_matches(&format!(".{ext}"))
                        .to_string();
                    let data = std::fs::read(&path)
                        .map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
                    store.put(key, data);
                    *count += 1;
                }
            }
        }
        Ok(())
    }
    let mut count = 0;
    walk(dir, dir, store, &mut count)?;
    Ok(count)
}

/// 渲染一张名片并返回编码字节。
fn render_with_format(
    renderer: &CustomProfileRenderer,
    card: &CustomProfileCard,
    profile: Option<&ProfileData>,
    format: &str,
) -> Result<Vec<u8>, String> {
    match format {
        "jpeg" | "jpg" => renderer.render_page_with_profile(card, profile),
        "png" => renderer.render_page_png_with_profile(card, profile),
        "png-transparent" | "png_transparent" => {
            renderer.render_page_png_transparent_with_profile(card, profile)
        }
        other => Err(format!("不支持的格式: {other}")),
    }
}

/// 收集名片所需但 AssetStore 中缺失的素材 key。
fn missing_asset_keys(
    renderer: &CustomProfileRenderer,
    card: &CustomProfileCard,
    store: &AssetStore,
) -> Vec<String> {
    let md = renderer.snapshot_masterdata();
    allium_renderer::asset_keys::collect_card_asset_keys(card, &md)
        .into_iter()
        .filter(|key| !store.contains(key))
        .collect()
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = match parse_args() {
        Ok(args) => args,
        Err(err) => {
            eprintln!("参数错误: {err}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    if let Some(font_dir) = &args.font_dir {
        // 引擎按 SCAPUS_FONT_DIR 解析字体文件。
        std::env::set_var("SCAPUS_FONT_DIR", font_dir);
    }

    let Some(md_dir) = &args.masterdata else {
        eprintln!("缺少 --masterdata\n\n{USAGE}");
        return ExitCode::from(2);
    };
    let provider = match JsonMasterDataProvider::from_dir(md_dir) {
        Ok(p) => p,
        Err(err) => {
            eprintln!("加载 masterdata 失败: {err}");
            return ExitCode::FAILURE;
        }
    };
    let missing = provider.missing_tables();
    if !missing.is_empty() {
        tracing::warn!(?missing, "部分 masterdata 表缺失，相关元素将按缺映射渲染");
    }

    let assets = Arc::new(AssetStore::new(256));
    if let Some(dir) = &args.assets_dir {
        match load_assets_dir(&assets, dir) {
            Ok(count) => tracing::info!(count, dir = %dir.display(), "素材注入完成"),
            Err(err) => {
                eprintln!("加载素材目录失败: {err}");
                return ExitCode::FAILURE;
            }
        }
    }

    let renderer =
        CustomProfileRenderer::new(Arc::new(provider)).with_assets(Arc::clone(&assets));

    if args.serve {
        return serve::run(renderer, assets);
    }

    // 单次模式
    let Some(card_path) = &args.card else {
        eprintln!("缺少 --card\n\n{USAGE}");
        return ExitCode::from(2);
    };
    let Some(output) = &args.output else {
        eprintln!("缺少 -o\n\n{USAGE}");
        return ExitCode::from(2);
    };

    let mut card = match load_card(card_path, args.page) {
        Ok(card) => card,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };

    let profile = match &args.profile {
        Some(path) => match load_profile(path, &renderer, &mut card) {
            Ok(profile) => Some(profile),
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };

    let warnings = renderer.validate_card(&card);
    for warning in &warnings {
        tracing::warn!("{warning}");
    }
    let missing_assets = missing_asset_keys(&renderer, &card, &assets);
    if !missing_assets.is_empty() {
        tracing::warn!(count = missing_assets.len(), keys = ?missing_assets, "缺失素材");
    }

    match render_with_format(&renderer, &card, profile.as_ref(), &args.format) {
        Ok(data) => {
            if let Err(err) = std::fs::write(output, &data) {
                eprintln!("写出 {} 失败: {err}", output.display());
                return ExitCode::FAILURE;
            }
            tracing::info!(bytes = data.len(), path = %output.display(), "渲染完成");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("渲染失败: {err}");
            ExitCode::FAILURE
        }
    }
}
