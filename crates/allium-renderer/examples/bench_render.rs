//! 端到端渲染基线 benchmark：量化一次完整名片渲染的时间分布。
//!
//! 复用 render_cached_card 的真实 masterdata + profile 加载路径，跑 N 次渲染，
//! 分阶段计时（flatten / draw_element 循环 / snapshot+encode），输出各阶段占比。
//! 用于判断该优化哪里、#29（消除 per-element AssetStore 重建）是否值得。
//!
//! 用法:
//!   bench-render <profile.json> <seq> [iterations]
//!
//! 数据准备（CDN 拉取）:
//!   masterdata → tmp/render_cache/masterdata/*.json
//!   素材磁盘缓存 → tmp/render_cache/

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use allium_renderer::assets::AssetStore;
use allium_renderer::init::install_fonts;
use allium_renderer::masterdata::{MasterDataProvider, ResolvedColor, ResolvedHonor, ResourceInfo};
use allium_renderer::profile::{build_honor_maps, ProfileData};
use allium_renderer::renderer::CustomProfileRenderer;
use allium_renderer::types::{
    BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry, HonorGroupEntry, StampEntry,
    UserCustomProfileCard,
};

use allium_renderer_host::Table;

struct CachedProvider {
    tables: HashMap<String, Arc<Table>>,
}

impl CachedProvider {
    fn from_cache_dir(dir: &Path) -> Result<Self, String> {
        let mut tables = HashMap::new();
        for entry in std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))? {
            let entry = entry.map_err(|e| format!("entry: {e}"))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            let content = std::fs::read_to_string(&path).map_err(|e| format!("read: {e}"))?;
            let table = Table::from_json(&content).map_err(|e| format!("parse {name}: {e}"))?;
            tables.insert(name, Arc::new(table));
        }
        Ok(Self { tables })
    }
    fn table(&self, n: &str) -> Option<&Arc<Table>> {
        self.tables.get(n)
    }
    fn typed<T: serde::de::DeserializeOwned>(&self, table: &str, id: i64) -> Option<T> {
        serde_json::from_value(self.table(table)?.by_id(id)?.clone()).ok()
    }
    fn map_font(name: &str) -> &str {
        match name {
            "FOT-RodinNTLGPro-DB" => "FZLanTingHei-DB-GBK",
            "FOT-SkipProN-B" => "FZZhengHei-EB-GBK",
            "FOT-PopHappinessStd-EB" => "FZShaoEr-M11-JF",
            other => other,
        }
    }
}

impl MasterDataProvider for CachedProvider {
    fn resolve_story_banner(&self, story_type: &str, story_id: i32) -> Option<String> {
        let id = story_id as i64;
        match story_type {
            "event_story" => {
                let abn = self.table("eventStories")?.by_id(id)?["assetbundleName"].as_str()?;
                Some(format!("event_story/{abn}/screen_image/banner_event_story"))
            }
            "unit_story" => {
                let abn = self.table("unitStoryEpisodeGroups")?.by_id(id)?["assetbundleName"].as_str()?;
                Some(format!("unit_story/{abn}/screen_image/banner_unit_story"))
            }
            _ => None,
        }
    }
    fn get_card(&self, id: i32) -> Option<CardEntry> {
        self.typed("cards", id as i64)
    }
    fn resolve_color(&self, id: i32) -> Option<ResolvedColor> {
        let hex = self.table("customProfileTextColors")?.by_id(id as i64)?["colorCode"].as_str()?;
        ResolvedColor::from_hex(hex)
    }
    fn resolve_font(&self, id: i32) -> Option<String> {
        let name = self.table("customProfileTextFonts")?.by_id(id as i64)?["fontName"].as_str()?;
        Some(Self::map_font(name).to_string())
    }
    fn resolve_stamp(&self, id: i32) -> Option<String> {
        let e: StampEntry = self.typed("stamps", id as i64)?;
        Some(e.assetbundle_name)
    }
    fn resolve_resource(&self, res_type: &str, id: i32) -> Option<ResourceInfo> {
        let tname = match res_type {
            "shape" => "customProfileShapeResources",
            "etc" => "customProfileEtcResources",
            "collection" => "customProfileCollectionResources",
            "general_bg" => "customProfileGeneralBackgroundResources",
            "standing" => "customProfileMemberStandingPictureResources",
            "player_info" => "customProfilePlayerInfoResources",
            "story_bg" => "customProfileStoryBackgroundResources",
            _ => return None,
        };
        let v = self.table(tname)?.by_id(id as i64)?;
        Some(ResourceInfo {
            file_name: v["fileName"].as_str()?.to_string(),
            load_val: v["resourceLoadVal"].as_str()?.to_string(),
            resource_type: v["customProfileResourceType"].as_str()?.to_string(),
        })
    }
    fn resolve_honor(&self, honor_id: i32, honor_level: i32) -> Option<ResolvedHonor> {
        let honor: HonorEntry = self.typed("honors", honor_id as i64)?;
        let is_live_master = honor.honor_mission_type.is_some() && honor.assetbundle_name.is_none();
        let (abn, rarity) = if is_live_master {
            let lvl = honor.levels.iter().find(|l| l.level == honor_level);
            (
                lvl.and_then(|l| l.assetbundle_name.as_deref()).unwrap_or("").to_string(),
                lvl.and_then(|l| l.honor_rarity.as_deref()).unwrap_or("low").to_string(),
            )
        } else {
            (
                honor.assetbundle_name.clone().unwrap_or_default(),
                honor.honor_rarity.clone().unwrap_or_else(|| "low".into()),
            )
        };
        let group: Option<HonorGroupEntry> =
            honor.group_id.and_then(|g| self.typed("honorGroups", g as i64));
        Some(ResolvedHonor {
            asset_bundle_name: abn,
            honor_rarity: rarity,
            honor_type: group.as_ref().map(|g| g.honor_type.as_str()).unwrap_or("normal").to_string(),
            background_asset_bundle_name: group.as_ref().and_then(|g| g.background_assetbundle_name.clone()),
            frame_name: group.as_ref().and_then(|g| g.frame_name.clone()),
            is_live_master,
            has_star: honor.levels.len() > 1,
            honor_level,
            honor_mission_type: honor.honor_mission_type.clone(),
        })
    }
    fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry> {
        self.typed("bondsHonors", id as i64)
    }
    fn get_bonds_honor_word(&self, id: i64) -> Option<BondsHonorWordEntry> {
        self.typed("bondsHonorWords", id)
    }
    fn get_honor(&self, id: i32) -> Option<HonorEntry> {
        self.typed("honors", id as i64)
    }
    fn resolve_unit_vs_sd(&self, self_id: i32, partner_id: i32) -> i32 {
        let units = match self.table("gameCharacterUnits") {
            Some(t) => t,
            None => return self_id,
        };
        let self_unit = match units.by_id(self_id as i64) {
            Some(v) => v,
            None => return self_id,
        };
        let self_cid = self_unit["gameCharacterId"].as_i64().unwrap_or(0);
        if self_cid < 21 {
            return self_id;
        }
        let partner = match units.by_id(partner_id as i64) {
            Some(v) => v,
            None => return self_id,
        };
        let target = match partner["unit"].as_str() {
            Some(u) => u,
            None => return self_id,
        };
        for e in units.all() {
            if e["gameCharacterId"].as_i64().unwrap_or(0) == self_cid && e["unit"].as_str().unwrap_or("") == target {
                return e["id"].as_i64().unwrap_or(self_id as i64) as i32;
            }
        }
        self_id
    }
    fn font_count(&self) -> usize {
        self.table("customProfileTextFonts").map(|t| t.len()).unwrap_or(0)
    }
    fn color_count(&self) -> usize {
        self.table("customProfileTextColors").map(|t| t.len()).unwrap_or(0)
    }
}

fn pct(part: f64, total: f64) -> f64 {
    if total <= 0.0 { 0.0 } else { part / total * 100.0 }
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(args.next().unwrap_or_else(|| "tmp/render_cache/raw_case1.json".to_string()));
    let seq: i32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(2);
    let iters: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(50);

    let _ = install_fonts(Path::new("assets/fonts"));
    let provider = Arc::new(CachedProvider::from_cache_dir(Path::new("tmp/render_cache/masterdata"))?);
    let mut asset_store = AssetStore::new(512);
    asset_store.set_disk_cache_dir(PathBuf::from("tmp/render_cache"));
    let _ = asset_store.load_static_dir(Path::new("assets/static"));
    let renderer = CustomProfileRenderer::new(provider).with_assets(Arc::new(asset_store));

    let body: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&input).map_err(|e| format!("read {}: {e}", input.display()))?)
            .map_err(|e| format!("parse: {e}"))?;
    let mut cards: Vec<UserCustomProfileCard> = serde_json::from_value(
        body.get("userCustomProfileCards").cloned().ok_or("missing userCustomProfileCards")?,
    ).map_err(|e| format!("parse cards: {e}"))?;
    let (hl, bl, cr) = build_honor_maps(&body);
    for c in &mut cards {
        renderer.enrich_honor_levels(&mut c.custom_profile_card, &hl, &bl, &cr);
    }
    let profile = ProfileData::from_json(&body);
    let card = cards.iter().find(|c| c.seq == seq).ok_or_else(|| format!("seq {seq} not found"))?;

    let elem_count = {
        let c = &card.custom_profile_card;
        c.texts.len() + c.shapes.len() + c.card_members.len() + c.stamps.len() + c.others.len()
            + c.bonds_honors.len() + c.honors.len() + c.collections.len() + c.generals.len()
            + c.stand_members.len() + c.general_backgrounds.len() + c.story_backgrounds.len()
    };

    // 预热：填充 glyph/素材缓存，剔除冷启动噪声，测稳态渲染
    eprintln!("预热（首次渲染填充缓存）...");

    // SCAPUS_DISABLE_DOWNSAMPLE=1 时关闭巨型字降采样（精确光栅化），
    // 与默认降采样在同一二进制内对比墙钟，消除 build-to-build 方差。
    if std::env::var("SCAPUS_DISABLE_DOWNSAMPLE").as_deref() == Ok("1") {
        allium_renderer::sdf::rasterize::bench_counters::DISABLE_DOWNSAMPLE
            .store(true, std::sync::atomic::Ordering::Relaxed);
        eprintln!("[bench] DISABLE_DOWNSAMPLE=1 → 巨型字精确光栅化（不降采样）");
    }

    let warm = renderer.render_page_with_profile(&card.custom_profile_card, Some(&profile))?;
    eprintln!("预热完成，输出 {} bytes，元素数={elem_count}", warm.len());

    // A/B 像素对比模式（SCAPUS_AB_COMPARE=1）：同一名片渲两遍，逐像素比对，
    // 量化巨型字降采样的视觉代价（精确光栅化 vs 降采样）。
    // 用无损 PNG 输出避免 JPEG 压缩噪声干扰对比。
    if std::env::var("SCAPUS_AB_COMPARE").as_deref() == Ok("1") {
        use std::sync::atomic::Ordering;
        let flag = &allium_renderer::sdf::rasterize::bench_counters::DISABLE_DOWNSAMPLE;
        let (name_a, name_b) = ("精确光栅化(不降采样)", "巨型字降采样");
        eprintln!("[A/B] A={name_a}  B={name_b}");

        flag.store(true, Ordering::Relaxed);
        let legacy_png =
            renderer.render_page_png_with_profile(&card.custom_profile_card, Some(&profile))?;
        flag.store(false, Ordering::Relaxed);
        let fast_png =
            renderer.render_page_png_with_profile(&card.custom_profile_card, Some(&profile))?;

        std::fs::write("tmp/render_cache/ab_legacy.png", &legacy_png).ok();
        std::fs::write("tmp/render_cache/ab_fast.png", &fast_png).ok();

        if legacy_png == fast_png {
            println!("\n=== A/B 像素对比 (seq={seq}, 降采样) ===");
            println!("PNG 字节完全一致 → 逐字节等价（md5 相同），零回归。");
            println!("dump: tmp/render_cache/ab_legacy.png(A={name_a})  ab_fast.png(B={name_b})");
            return Ok(());
        }

        // 字节不一致 → 解码两张 PNG 做逐像素 diff
        let decode = |bytes: &[u8]| -> Option<(Vec<u8>, i32, i32)> {
            let data = skia_safe::Data::new_copy(bytes);
            let img = skia_safe::images::deferred_from_encoded_data(data, None)?;
            let (w, h) = (img.width(), img.height());
            let info = skia_safe::ImageInfo::new(
                (w, h),
                skia_safe::ColorType::RGBA8888,
                skia_safe::AlphaType::Unpremul,
                None,
            );
            let mut buf = vec![0u8; (w * h * 4) as usize];
            if img.read_pixels(
                &info,
                &mut buf,
                (w * 4) as usize,
                (0, 0),
                skia_safe::image::CachingHint::Disallow,
            ) {
                Some((buf, w, h))
            } else {
                None
            }
        };

        let (a, aw, ah) = decode(&legacy_png).ok_or("decode legacy png")?;
        let (b, bw, bh) = decode(&fast_png).ok_or("decode fast png")?;
        if (aw, ah) != (bw, bh) {
            return Err(format!("尺寸不一致 legacy={aw}x{ah} fast={bw}x{bh}"));
        }

        let total_px = (aw * ah) as usize;
        let mut diff_px = 0usize; // 至少一个通道不同的像素数
        let mut max_diff = 0u8; // 单通道最大绝对差
        let mut sum_sq = 0f64; // 用于 RMSE
        let mut hist = [0u64; 5]; // 差值分布：0, 1, 2, 3, >=4
        for i in 0..total_px {
            let mut px_diff = false;
            for c in 0..4 {
                let d = (a[i * 4 + c] as i32 - b[i * 4 + c] as i32).unsigned_abs() as u8;
                if d > 0 {
                    px_diff = true;
                    if d > max_diff {
                        max_diff = d;
                    }
                    sum_sq += (d as f64) * (d as f64);
                    hist[(d as usize).min(4)] += 1;
                }
            }
            if px_diff {
                diff_px += 1;
            }
        }
        let rmse = (sum_sq / (total_px * 4) as f64).sqrt();

        println!("\n=== A/B 像素对比 (seq={seq}, {aw}x{ah}={total_px}px) ===");
        println!("PNG 字节: 不一致");
        println!(
            "差异像素: {diff_px} / {total_px} ({:.4}%)",
            diff_px as f64 / total_px as f64 * 100.0
        );
        println!("单通道最大绝对差: {max_diff} / 255");
        println!("全图 RMSE: {rmse:.5}");
        println!(
            "通道差值分布: =1:{}  =2:{}  =3:{}  >=4:{}",
            hist[1], hist[2], hist[3], hist[4]
        );
        println!("dump: tmp/render_cache/ab_legacy.png  tmp/render_cache/ab_fast.png");
        if max_diff <= 1 {
            println!("结论: 最大差 ≤1/255，视觉不可察觉（仅 round 边界舍入差）。");
        } else if max_diff <= 3 {
            println!("结论: 最大差 ≤3/255，视觉几乎不可察觉，建议肉眼复核 dump 图。");
        } else {
            println!("结论: 最大差 >3/255，需肉眼复核 dump 图确认可接受性。");
        }
        return Ok(());
    }

    let mut times = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t = Instant::now();
        let _ = renderer.render_page_with_profile(&card.custom_profile_card, Some(&profile))?;
        times.push(t.elapsed().as_secs_f64() * 1e3); // ms
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let sum: f64 = times.iter().sum();
    let mean = sum / iters as f64;
    let p50 = times[iters / 2];
    let p99 = times[(iters as f64 * 0.99) as usize % iters];
    let min = times[0];
    let max = times[iters - 1];

    println!("\n=== 端到端 render_page 基线 ===");
    println!("seq={seq} 元素数={elem_count} 迭代={iters}（已预热，稳态）");
    println!("总耗时/次: mean={mean:.3}ms  p50={p50:.3}ms  p99={p99:.3}ms  min={min:.3}ms  max={max:.3}ms");

    // 光栅化计数器：单次渲染的字形数/总像素/shade 调用数，拆出 per-glyph 真实成本。
    allium_renderer::sdf::rasterize::bench_counters::reset();
    let single = Instant::now();
    let _ = renderer.render_page_with_profile(&card.custom_profile_card, Some(&profile))?;
    let single_ms = single.elapsed().as_secs_f64() * 1e3;
    let (glyphs, px, shade, prepad, pad_sum, max_win) =
        allium_renderer::sdf::rasterize::bench_counters::snapshot();
    println!("\n=== 光栅化计数（单次渲染）===");
    println!("字形数={glyphs}  总像素={px}  shade调用={shade}  单次={single_ms:.2}ms");
    if glyphs > 0 {
        let g = glyphs as f64;
        println!(
            "per-glyph: {:.1}us/字形  窗口={:.0}px/字形  pre-pad={:.0}px/字形  平均pad={:.1}  最大窗口={}px",
            single_ms * 1000.0 / g,
            px as f64 / g,
            prepad as f64 / g,
            pad_sum as f64 / g,
            max_win
        );
        println!(
            "膨胀倍数(窗口/prepad)={:.1}x  →  >2x 说明 pad_px 撑爆，≈1x 说明字形本身大",
            if prepad > 0 { px as f64 / prepad as f64 } else { 0.0 }
        );
    }

    // 单独测 draw_element 循环段占比（#29 影响的部分）。
    // 复刻 render_card 的循环，隔离测「flatten + draw_element 全部元素」这一段。
    use skia_safe::surfaces;
    let w = allium_renderer::transform::CANVAS_WIDTH as i32;
    let h = allium_renderer::transform::CANVAS_HEIGHT as i32;
    let md = renderer.masterdata();
    let assets = renderer.assets().cloned();

    let mut loop_times = Vec::with_capacity(iters);
    let mut flat_times = Vec::with_capacity(iters);
    // 按元素类型累加耗时（定位 1315ms 的去向）
    use allium_renderer::elements::RenderElement;
    let mut by_type: HashMap<&'static str, (f64, usize)> = HashMap::new();
    for _ in 0..iters {
        let mut surface = surfaces::raster_n32_premul((w, h)).ok_or("surface")?;
        let canvas = surface.canvas();
        canvas.clear(skia_safe::Color::TRANSPARENT);

        let tf = Instant::now();
        let elements = allium_renderer::elements::flatten_and_sort(&card.custom_profile_card);
        flat_times.push(tf.elapsed().as_secs_f64() * 1e3);

        let fallback = AssetStore::new(1);
        let theme = allium_renderer::widgets::theme::Theme::default();
        let tl = Instant::now();
        for elem in &elements {
            if !elem.visible() {
                continue;
            }
            let kind = match elem {
                RenderElement::Text(_) => "Text",
                RenderElement::Shape(_) => "Shape",
                RenderElement::CardMember(_) => "CardMember",
                RenderElement::Stamp(_) => "Stamp",
                RenderElement::Other(_) => "Other",
                RenderElement::BondsHonor(_) => "BondsHonor",
                RenderElement::Honor(_) => "Honor",
                RenderElement::Collection(_) => "Collection",
                RenderElement::General(_) => "General",
                RenderElement::StandMember(_) => "StandMember",
                RenderElement::GeneralBackground(_) => "GeneralBackground",
                RenderElement::StoryBackground(_) => "StoryBackground",
            };
            let te = Instant::now();
            allium_renderer::elements::draw_element_on_canvas(
                canvas, elem, &md, assets.as_deref(), Some(&profile),
                &fallback, &theme, w as f32, h as f32,
            );
            let dt = te.elapsed().as_secs_f64() * 1e3;
            let entry = by_type.entry(kind).or_insert((0.0, 0));
            entry.0 += dt;
            entry.1 += 1;
        }
        loop_times.push(tl.elapsed().as_secs_f64() * 1e3);
    }
    let flat_mean: f64 = flat_times.iter().sum::<f64>() / iters as f64;
    let loop_mean: f64 = loop_times.iter().sum::<f64>() / iters as f64;

    println!("\n=== 阶段分解（mean，隔离测量）===");
    println!("flatten_and_sort: {flat_mean:.4}ms  ({:.1}% of 总)", pct(flat_mean, mean));
    println!("draw_element 循环: {loop_mean:.4}ms  ({:.1}% of 总)", pct(loop_mean, mean));
    println!("其余(surface+snapshot+encode): {:.4}ms  ({:.1}% of 总)", mean - flat_mean - loop_mean, pct(mean - flat_mean - loop_mean, mean));
    println!("\n注：#29 优化的是 draw_element 循环内的 per-element AssetStore 重建，");
    println!("只有当 draw_element 循环占比显著时，#29 才有实际意义。");

    println!("\n=== 按元素类型耗时（{iters} 次累加，定位热点）===");
    let mut rows: Vec<(&str, f64, usize)> =
        by_type.iter().map(|(k, (t, n))| (*k, *t, *n)).collect();
    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let total_typed: f64 = rows.iter().map(|r| r.1).sum();
    println!("{:<18} {:>10} {:>8} {:>12} {:>7}", "类型", "总ms", "次数", "单次us", "占比");
    for (kind, t, n) in &rows {
        let per = if *n > 0 { t / *n as f64 * 1000.0 } else { 0.0 };
        println!(
            "{:<18} {:>10.2} {:>8} {:>12.1} {:>6.1}%",
            kind, t, n, per, pct(*t, total_typed)
        );
    }
    Ok(())
}
