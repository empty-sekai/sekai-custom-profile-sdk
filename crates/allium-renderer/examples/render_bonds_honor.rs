//! 羁绊称号隔离渲染探针。
//!
//! 直接驱动 `elements::honor::render_bonds_honor`，用于验证 SD 角色缩放/锚点/
//! 边框对齐的几何修改，无需起完整服务或拉取真实 profile。
//!
//! 用法:
//!   render-bonds-honor <out_dir>
//!
//! 资源来源:
//!   - 静态素材: assets/static（bg / frame / mask / icon，启动时 pin）
//!   - SD 角色:  tmp/bonds_probe/sd/chr_sd_NN_01.png（从 CDN 预拉）
//!
//! 对照组（角色尺寸差异大，能暴露统一高度缩放的问题）:
//!   - 穗波(03, 99×101) × 奏(17, 160×136)
//!   - 一歌(01, 108×105) × 真冬(18, 160×136)

use std::path::{Path, PathBuf};
use std::sync::Arc;

use allium_renderer::assets::AssetStore;
use allium_renderer::elements::honor::{render_bonds_honor, render_honor};
use allium_renderer::masterdata::{
    MasterData, MasterDataProvider, ResolvedColor, ResolvedHonor, ResourceInfo,
};
use allium_renderer::types::{
    BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry,
};
use skia_safe::{surfaces, Color, EncodedImageFormat, Point};

/// 测试用 provider：按构造时给定的角色对返回羁绊条目，
/// 以及（用于 standard 边框诊断的）可配置普通称号。
struct ProbeProvider {
    cid1: i32,
    cid2: i32,
    rarity: String,
    /// 普通称号配置：(honor_type, honor_rarity, frame_name)
    std_honor: Option<(String, String, Option<String>)>,
}

impl MasterDataProvider for ProbeProvider {
    fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> {
        None
    }
    fn get_card(&self, _: i32) -> Option<CardEntry> {
        None
    }
    fn resolve_color(&self, _: i32) -> Option<ResolvedColor> {
        None
    }
    fn resolve_font(&self, _: i32) -> Option<String> {
        None
    }
    fn resolve_stamp(&self, _: i32) -> Option<String> {
        None
    }
    fn resolve_resource(&self, _: &str, _: i32) -> Option<ResourceInfo> {
        None
    }
    fn resolve_honor(&self, _: i32, honor_level: i32) -> Option<ResolvedHonor> {
        let (htype, rarity, frame_name) = self.std_honor.clone()?;
        Some(ResolvedHonor {
            asset_bundle_name: "probe_honor".to_string(),
            honor_rarity: rarity,
            honor_type: htype,
            background_asset_bundle_name: None,
            frame_name,
            is_live_master: false,
            has_star: false,
            honor_level,
            honor_mission_type: None,
        })
    }
    fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry> {
        Some(BondsHonorEntry {
            id,
            game_character_unit_id1: self.cid1,
            game_character_unit_id2: self.cid2,
            honor_rarity: self.rarity.clone(),
            configurable_unit_virtual_singer: false,
        })
    }
    fn get_bonds_honor_word(&self, _: i64) -> Option<BondsHonorWordEntry> {
        None
    }
    fn get_honor(&self, _: i32) -> Option<HonorEntry> {
        None
    }
    fn resolve_unit_vs_sd(&self, self_id: i32, _: i32) -> i32 {
        self_id
    }
    fn font_count(&self) -> usize {
        0
    }
    fn color_count(&self) -> usize {
        0
    }
}

fn put_png(store: &AssetStore, key: &str, path: &Path) {
    match std::fs::read(path) {
        Ok(bytes) => store.put(key.to_string(), bytes),
        Err(e) => eprintln!("WARN 读取 {} 失败: {e}（key={key} 将缺失）", path.display()),
    }
}

fn render_one(out_dir: &Path, label: &str, cid1: i32, cid2: i32, rarity: &str, full_size: bool) {
    let store = AssetStore::new(512);
    // 静态素材（bg / frame / mask / icon）→ pin
    if let Err(e) = store.load_static_dir(Path::new("assets/static")) {
        eprintln!("WARN load_static_dir 失败: {e}");
    }
    // SD 角色素材（CDN 预拉）→ put
    let sd_dir = Path::new("tmp/bonds_probe/sd");
    for cid in [cid1, cid2] {
        let key = format!("bonds_honor/chr_sd_{:02}_01", cid);
        let path = sd_dir.join(format!("chr_sd_{:02}_01.png", cid));
        put_png(&store, &key, &path);
    }

    let provider = Arc::new(ProbeProvider {
        cid1,
        cid2,
        rarity: rarity.to_string(),
        std_honor: None,
    });
    let md = MasterData::new(provider);

    let (w, h) = if full_size { (380.0_f32, 80.0_f32) } else { (180.0_f32, 80.0_f32) };
    // 留 padding 以观察是否有越界/错位
    let pad = 20.0_f32;
    let cw = (w + pad * 2.0) as i32;
    let ch = (h + pad * 2.0) as i32;
    let mut surface = surfaces::raster_n32_premul((cw, ch)).expect("创建 surface 失败");
    let canvas = surface.canvas();
    // 浅灰底，方便看透明边界
    canvas.clear(Color::from_argb(255, 40, 40, 48));
    canvas.save();
    canvas.translate(Point::new(w / 2.0 + pad, h / 2.0 + pad));
    render_bonds_honor(
        canvas, 1, /*honor_level*/ 5, full_size, 0, /*word_id*/
        false, /*inverse*/ false, /*use_unit_vs*/ &md, &store,
    );
    canvas.restore();

    let image = surface.image_snapshot();
    let data = image
        .encode(None, EncodedImageFormat::PNG, 100)
        .expect("编码 PNG 失败");
    let size = if full_size { "main" } else { "sub" };
    let out = out_dir.join(format!("{label}_{size}.png"));
    std::fs::write(&out, data.as_bytes()).expect("写出 PNG 失败");
    println!("saved {} ({} bytes)", out.display(), data.as_bytes().len());
}

fn render_one_std(
    out_dir: &Path,
    label: &str,
    honor_type: &str,
    rarity: &str,
    frame_name: Option<&str>,
    full_size: bool,
) {
    let store = AssetStore::new(512);
    if let Err(e) = store.load_static_dir(Path::new("assets/static")) {
        eprintln!("WARN load_static_dir 失败: {e}");
    }
    // frame_name 自定义边框（CDN 预拉到 tmp/bonds_probe/frame/<name>/<file>.png）
    if let Some(fname) = frame_name {
        let sc = if full_size { "m" } else { "s" };
        let rn = match rarity {
            "low" => 1,
            "middle" => 2,
            "high" => 3,
            _ => 4,
        };
        let key = format!("honor_frame/{fname}/frame_degree_{sc}_{rn}");
        let path = Path::new("tmp/bonds_probe/frame")
            .join(fname)
            .join(format!("frame_degree_{sc}_{rn}.png"));
        put_png(&store, &key, &path);
    }

    let provider = Arc::new(ProbeProvider {
        cid1: 0,
        cid2: 0,
        rarity: rarity.to_string(),
        std_honor: Some((
            honor_type.to_string(),
            rarity.to_string(),
            frame_name.map(str::to_string),
        )),
    });
    let md = MasterData::new(provider);

    let (w, h) = if full_size { (380.0_f32, 80.0_f32) } else { (180.0_f32, 80.0_f32) };
    let pad = 20.0_f32;
    let mut surface =
        surfaces::raster_n32_premul(((w + pad * 2.0) as i32, (h + pad * 2.0) as i32))
            .expect("创建 surface 失败");
    let canvas = surface.canvas();
    canvas.clear(Color::from_argb(255, 40, 40, 48));
    canvas.save();
    canvas.translate(Point::new(w / 2.0 + pad, h / 2.0 + pad));
    render_honor(canvas, 1, 1, full_size, &md, &store, None);
    canvas.restore();

    let image = surface.image_snapshot();
    let data = image
        .encode(None, EncodedImageFormat::PNG, 100)
        .expect("编码 PNG 失败");
    let size = if full_size { "main" } else { "sub" };
    let out = out_dir.join(format!("std_{label}_{size}.png"));
    std::fs::write(&out, data.as_bytes()).expect("写出 PNG 失败");
    println!("saved {} ({} bytes)", out.display(), data.as_bytes().len());
}

fn main() {
    // debug 级别订阅器：打印插桩的几何量到 stderr
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();

    let args: Vec<String> = std::env::args().collect();
    let out_dir = PathBuf::from(args.get(1).map(String::as_str).unwrap_or("tmp/bonds_probe/out"));
    std::fs::create_dir_all(&out_dir).expect("创建输出目录失败");

    // 羁绊对照组：角色尺寸差异大
    let cases: &[(&str, i32, i32, &str)] = &[
        ("hotaru_kanade", 3, 17, "middle"),  // 穗波 99×101 × 奏 160×136
        ("ichika_mafuyu", 1, 18, "high"),    // 一歌 108×105 × 真冬 160×136
        ("ichika_hotaru", 1, 3, "low"),      // 一歌 108×105 × 穗波 99×101
    ];
    for (label, c1, c2, rarity) in cases {
        for full_size in [true, false] {
            let size = if full_size { "main" } else { "sub" };
            eprintln!("===== bonds case={label} size={size} cid1={c1} cid2={c2} rarity={rarity} =====");
            render_one(&out_dir, label, *c1, *c2, rarity, full_size);
        }
    }

    // standard 边框诊断：默认边框（与羁绊同素材）各稀有度 + 一个 frame_name 自定义边框
    let std_cases: &[(&str, &str, &str, Option<&str>)] = &[
        ("event_low", "event", "low", None),
        ("event_mid", "event", "middle", None),
        ("event_high", "event", "high", None),
        ("achievement_low", "achievement", "low", None),
        ("character_low", "character", "low", None),
    ];
    for (label, htype, rarity, frame) in std_cases {
        for full_size in [true, false] {
            let size = if full_size { "main" } else { "sub" };
            eprintln!("===== std case={label} size={size} type={htype} rarity={rarity} frame={frame:?} =====");
            render_one_std(&out_dir, label, htype, rarity, *frame, full_size);
        }
    }
    println!("完成，输出目录: {}", out_dir.display());
}
