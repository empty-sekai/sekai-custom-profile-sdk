use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use allium_renderer::assets::AssetStore;
use allium_renderer::init::install_fonts;
use allium_renderer::masterdata::{MasterDataProvider, ResolvedColor, ResolvedHonor, ResourceInfo};
use allium_renderer::renderer::CustomProfileRenderer;
use allium_renderer::text::resolve_custom_profile_typeface;
use allium_renderer::text::richtext::parse_rich_segments;
use allium_renderer::text::richtext::{Indent, SizeSpec};
use allium_renderer::transform::quaternion_to_degrees;
use allium_renderer::types::{
    BondsHonorEntry, BondsHonorWordEntry, CardEntry, CustomProfileCard, HonorEntry, TextElement,
    UserCustomProfileCard,
};
use serde::Deserialize;
use skia_safe::{Font, FontMgr};

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
struct ProfileEnvelope {
    user_custom_profile_cards: Vec<UserCustomProfileCard>,
}

struct Task043Provider {
    fonts: HashMap<i32, String>,
    colors: HashMap<i32, ResolvedColor>,
}

impl Task043Provider {
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
            .map(|entry| (entry.id, Self::map_font_name(&entry.font_name).to_string()))
            .collect();
        let color_map = colors
            .into_iter()
            .filter_map(|entry| ResolvedColor::from_hex(&entry.color_code).map(|c| (entry.id, c)))
            .collect();
        Ok(Self {
            fonts: font_map,
            colors: color_map,
        })
    }

    fn map_font_name(name: &str) -> &str {
        match name {
            "FOT-RodinNTLGPro-DB" => "FZLanTingHei-DB-GBK",
            "FOT-SkipProN-B" => "FZZhengHei-EB-GBK",
            "FOT-PopHappinessStd-EB" => "FZShaoEr-M11-JF",
            other => other,
        }
    }
}

impl MasterDataProvider for Task043Provider {
    fn resolve_story_banner(&self, _story_type: &str, _story_id: i32) -> Option<String> {
        None
    }

    fn get_card(&self, _card_id: i32) -> Option<CardEntry> {
        None
    }

    fn resolve_color(&self, color_id: i32) -> Option<ResolvedColor> {
        self.colors.get(&color_id).copied()
    }

    fn resolve_font(&self, font_id: i32) -> Option<String> {
        self.fonts.get(&font_id).cloned()
    }

    fn resolve_stamp(&self, _stamp_id: i32) -> Option<String> {
        None
    }

    fn resolve_resource(&self, _res_type: &str, _id: i32) -> Option<ResourceInfo> {
        None
    }

    fn resolve_honor(&self, _honor_id: i32, _honor_level: i32) -> Option<ResolvedHonor> {
        None
    }

    fn get_bonds_honor(&self, _id: i32) -> Option<BondsHonorEntry> {
        None
    }

    fn get_bonds_honor_word(&self, _word_id: i64) -> Option<BondsHonorWordEntry> {
        None
    }

    fn get_honor(&self, _honor_id: i32) -> Option<HonorEntry> {
        None
    }

    fn resolve_unit_vs_sd(&self, self_id: i32, _partner_id: i32) -> i32 {
        self_id
    }

    fn font_count(&self) -> usize {
        self.fonts.len()
    }

    fn color_count(&self) -> usize {
        self.colors.len()
    }
}

fn isolate_layer(card: &CustomProfileCard, layer: i32) -> CustomProfileCard {
    let mut isolated = card.clone();
    isolated
        .texts
        .retain(|text| text.object_data.visible && text.object_data.layer == layer);
    isolated.shapes.clear();
    isolated.card_members.clear();
    isolated.stamps.clear();
    isolated.others.clear();
    isolated.bonds_honors.clear();
    isolated.honors.clear();
    isolated.collections.clear();
    isolated.generals.clear();
    isolated.general_backgrounds.clear();
    isolated.stand_members.clear();
    isolated.story_backgrounds.clear();
    isolated
}

fn is_fullwidth_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{2000}'..='\u{206F}'
            | '\u{2190}'..='\u{21FF}'
            | '\u{2200}'..='\u{22FF}'
            | '\u{2300}'..='\u{23FF}'
            | '\u{2460}'..='\u{24FF}'
            | '\u{2500}'..='\u{259F}'
            | '\u{25A0}'..='\u{25FF}'
            | '\u{2600}'..='\u{26FF}'
            | '\u{2700}'..='\u{27BF}'
            | '\u{3000}'..='\u{30FF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FE30}'..='\u{FE4F}'
            | '\u{FF01}'..='\u{FF60}'
    )
}

fn resolve_segment_font_size(size: Option<SizeSpec>, base_font_size: f32) -> f32 {
    match size {
        Some(SizeSpec::Absolute(v)) => v,
        Some(SizeSpec::Delta(v)) => base_font_size + v,
        Some(SizeSpec::Percent(v)) => base_font_size * v / 100.0,
        Some(SizeSpec::Em(v)) => base_font_size * v,
        None => base_font_size,
    }
}

fn resolve_indent_pixels(spec: Option<Indent>) -> Option<f32> {
    match spec {
        Some(Indent::Pixels(v)) => Some(v / allium_renderer::text::TEXT_SCALE),
        Some(Indent::Em(_)) | Some(Indent::Percent(_)) | None => None,
    }
}

fn tmp_measure_advance_local(text: &str, font: &Font, font_size: f32) -> f32 {
    let mut total = 0.0f32;
    for ch in text.chars() {
        if is_fullwidth_char(ch) {
            total += font_size;
        } else {
            total += font.measure_str(ch.to_string(), None).0;
        }
    }
    total
}

fn update_cpv_width(max_width_tmp: &mut f32, cpv_xadv_tmp: f32, glyph_hadv_tmp: f32) {
    *max_width_tmp = (*max_width_tmp).max(cpv_xadv_tmp.abs() + glyph_hadv_tmp);
}

fn transform_char_for_segment_local(ch: char) -> (String, f32) {
    (ch.to_string(), 1.0)
}

fn dump_layout_metrics(provider: &Task043Provider, text: &TextElement) -> Result<(), String> {
    let font_mgr = FontMgr::default();
    let family = provider.resolve_font(text.font_id);
    let typeface = resolve_custom_profile_typeface(&font_mgr, family.as_deref())
        .or_else(|| font_mgr.legacy_make_typeface(None, skia_safe::FontStyle::default()))
        .ok_or_else(|| "无法解析调试字体".to_string())?;
    let segments = parse_rich_segments(&text.text);
    let base_size = text.size;
    let base_font = Font::new(typeface.clone(), Some(base_size));
    let clean_owned: String = segments.iter().map(|seg| seg.text.as_str()).collect();
    let clean_trimmed = clean_owned.strip_suffix('\n').unwrap_or(&clean_owned);
    let line_texts: Vec<&str> = clean_trimmed.split('\n').collect();
    let seg_cleans: Vec<String> = segments
        .iter()
        .map(|seg| seg.text.chars().filter(|c| *c != '\n').collect())
        .collect();
    let align = text.text_type & 0x07;
    let mut seg_consumed: Vec<usize> = vec![0; segments.len()];
    let mut line_widths: Vec<f32> = Vec::new();
    let mut rect_widths: Vec<f32> = Vec::new();

    for line_str in &line_texts {
        let mut w_scaled = 0.0f32;
        let mut prev_cspace: Option<f32> = None;
        let mut cpv_xadv_tmp = 0.0f32;
        let mut max_cpv_width_tmp = 0.0f32;
        let mut has_chars = false;
        let mut remaining = *line_str;
        let mut current_position: Option<Indent> = None;

        for (si, seg) in segments.iter().enumerate() {
            if remaining.is_empty() {
                break;
            }
            let sc = &seg_cleans[si];
            if sc.is_empty() || seg_consumed[si] >= sc.len() {
                continue;
            }
            let seg_rest = &sc[seg_consumed[si]..];
            let part = if remaining.starts_with(seg_rest) {
                remaining = &remaining[seg_rest.len()..];
                seg_consumed[si] = sc.len();
                seg_rest.to_string()
            } else if seg_rest.starts_with(remaining) {
                let p = remaining.to_string();
                seg_consumed[si] += remaining.len();
                remaining = "";
                p
            } else {
                continue;
            };
            if part.is_empty() {
                continue;
            }

            let seg_size = resolve_segment_font_size(seg.size, text.size);
            if seg.position != current_position {
                if let Some(pos_shift) = resolve_indent_pixels(seg.position) {
                    cpv_xadv_tmp = pos_shift * allium_renderer::text::TEXT_SCALE;
                }
                current_position = seg.position;
            }
            let measure_size = if seg.subscript || seg.superscript {
                seg_size * 0.5
            } else {
                seg_size
            };
            let seg_font = Font::new(typeface.clone(), Some(measure_size));
            let part_chars: Vec<char> = part.chars().collect();
            let cspace_raw_tmp = seg.cspace.unwrap_or(0.0);
            let mut measured = 0.0f32;
            for ch in &part_chars {
                let (display, char_scale) = transform_char_for_segment_local(*ch);
                let glyph_hadv_tmp = tmp_measure_advance_local(&display, &seg_font, measure_size)
                    * char_scale
                    * allium_renderer::text::TEXT_SCALE;
                measured += glyph_hadv_tmp / allium_renderer::text::TEXT_SCALE;
                update_cpv_width(&mut max_cpv_width_tmp, cpv_xadv_tmp, glyph_hadv_tmp);
                cpv_xadv_tmp += glyph_hadv_tmp + cspace_raw_tmp;
            }
            let cspace = seg.cspace.unwrap_or(0.0) / allium_renderer::text::TEXT_SCALE;
            let cspace_total = if part_chars.len() > 1 {
                cspace * (part_chars.len() - 1) as f32
            } else {
                0.0
            };
            let inter_seg_cspace = prev_cspace.unwrap_or(0.0);
            w_scaled += measured * seg.scale.unwrap_or(1.0) + cspace_total + inter_seg_cspace;
            has_chars = true;
            prev_cspace = Some(cspace);
        }

        if !remaining.is_empty() {
            let measured = tmp_measure_advance_local(remaining, &base_font, base_size);
            w_scaled += measured;
            for ch in remaining.chars() {
                let glyph_hadv_tmp = tmp_measure_advance_local(&ch.to_string(), &base_font, base_size)
                    * allium_renderer::text::TEXT_SCALE;
                update_cpv_width(&mut max_cpv_width_tmp, cpv_xadv_tmp, glyph_hadv_tmp);
                cpv_xadv_tmp += glyph_hadv_tmp;
            }
            has_chars = true;
        }

        line_widths.push(w_scaled);
        rect_widths.push(if has_chars {
            max_cpv_width_tmp / allium_renderer::text::TEXT_SCALE
        } else {
            0.0
        });
    }

    const PAD_ORIGINAL: f32 = 64.0 / allium_renderer::text::TEXT_SCALE;
    let box_w = rect_widths.iter().cloned().fold(0.0f32, f32::max) + PAD_ORIGINAL;
    println!(
        "  layout align={} line_widths={:?} rect_widths={:?} box_w={:.3}",
        align, line_widths, rect_widths, box_w
    );

    let mut render_consumed: Vec<usize> = vec![0; segments.len()];
    for (line_idx, line_str) in line_texts.iter().enumerate() {
        let sw = line_widths[line_idx];
        let lx = match align {
            2 => -sw / 2.0,
            4 => box_w / 2.0 - sw,
            _ => -box_w / 2.0,
        };
        println!("  line[{line_idx}] sw={sw:.3} lx={lx:.3}");
        let mut cursor_x = lx;
        let mut current_position: Option<Indent> = None;
        let mut remaining = *line_str;

        for (si, seg) in segments.iter().enumerate() {
            if remaining.is_empty() {
                break;
            }
            let sc = &seg_cleans[si];
            if sc.is_empty() || render_consumed[si] >= sc.len() {
                continue;
            }
            let seg_rest = &sc[render_consumed[si]..];
            let part = if remaining.starts_with(seg_rest) {
                remaining = &remaining[seg_rest.len()..];
                render_consumed[si] = sc.len();
                seg_rest.to_string()
            } else if seg_rest.starts_with(remaining) {
                let p = remaining.to_string();
                render_consumed[si] += remaining.len();
                remaining = "";
                p
            } else {
                continue;
            };
            if part.is_empty() {
                continue;
            }

            let seg_size = resolve_segment_font_size(seg.size, text.size);
            let seg_scale = seg.scale.unwrap_or(1.0);
            if seg.position != current_position {
                if let Some(pos_shift) = resolve_indent_pixels(seg.position) {
                    cursor_x = lx + pos_shift;
                }
                current_position = seg.position;
            }
            let render_size = if seg.subscript || seg.superscript {
                seg_size * 0.5
            } else {
                seg_size
            };
            let seg_font = Font::new(typeface.clone(), Some(render_size));
            for ch in part.chars().filter(|ch| *ch != '\n') {
                let (display, char_scale) = transform_char_for_segment_local(ch);
                let effective_scale = seg_scale * char_scale;
                let ch_advance = tmp_measure_advance_local(&display, &seg_font, render_size);
                let (_, glyph_bounds) = seg_font.measure_str(&display, None);
                let pivot_x = if (effective_scale - 1.0).abs() > 0.001 {
                    (glyph_bounds.left + glyph_bounds.right) / 2.0
                } else {
                    ch_advance / 2.0
                };
                println!(
                    "    draw ch=U+{:04X} '{}' cursor_x={:.3} advance={:.3} eff_scale={:.3} pivot_x={:.3} visual_center_x={:.3}",
                    ch as u32,
                    ch,
                    cursor_x,
                    ch_advance,
                    effective_scale,
                    pivot_x,
                    cursor_x + pivot_x
                );
                cursor_x += ch_advance * effective_scale + seg.cspace.unwrap_or(0.0) / allium_renderer::text::TEXT_SCALE;
            }
        }
    }

    Ok(())
}

fn dump_geometry(provider: &Task043Provider, text: &TextElement) -> Result<(), String> {
    let font_mgr = FontMgr::default();
    let family = provider.resolve_font(text.font_id);
    let typeface = resolve_custom_profile_typeface(&font_mgr, family.as_deref())
        .or_else(|| font_mgr.legacy_make_typeface(None, skia_safe::FontStyle::default()))
        .ok_or_else(|| "无法解析调试字体".to_string())?;
    let angle = quaternion_to_degrees(&text.object_data.rotation);
    println!(
        "layer={} angle={:.4} text={}",
        text.object_data.layer, angle, text.text
    );
    let mut cursor_x = 0.0f32;
    let mut current_position: Option<Indent> = None;
    for (seg_idx, seg) in parse_rich_segments(&text.text).iter().enumerate() {
        let seg_size = resolve_segment_font_size(seg.size, text.size);
        let render_size = if seg.subscript || seg.superscript {
            seg_size * 0.5
        } else {
            seg_size
        };
        let seg_scale = seg.scale.unwrap_or(1.0);
        let baseline_shift = seg
            .voffset
            .map(|value| -value / allium_renderer::text::TEXT_SCALE)
            .unwrap_or(0.0);
        if seg.position != current_position {
            if let Some(pos_shift) = resolve_indent_pixels(seg.position) {
                cursor_x = pos_shift;
            }
            current_position = seg.position;
        }
        let font = Font::new(typeface.clone(), Some(render_size));
        for ch in seg.text.chars().filter(|ch| *ch != '\n') {
            let advance = tmp_measure_advance_local(&ch.to_string(), &font, render_size);
            let (_, bounds) = font.measure_str(ch.to_string(), None);
            let bounds_center_x = (bounds.left + bounds.right) / 2.0;
            let bounds_center_y = (bounds.top + bounds.bottom) / 2.0;
            let mut extra = String::new();
            if let Some(glyph) = allium_renderer::sdf::outline::lookup_or_generate(family.as_deref(), ch) {
                let scale_local = render_size / 75.0;
                let plane_center_x = (glyph.plane_bearing_x() + glyph.plane_width() / 2.0) * scale_local;
                let plane_center_y = -(glyph.plane_bearing_y() - glyph.plane_height() / 2.0) * scale_local;
                let origin_scaled_center_x = plane_center_x * seg_scale;
                let center_scaled_center_x = plane_center_x;
                let italic_mid = ((59.0 - (0.0 + seg.voffset.unwrap_or(0.0))) / 2.0) * scale_local;
                let top_term = glyph.plane_bearing_y() * scale_local - italic_mid;
                let bottom_term =
                    (glyph.plane_bearing_y() - glyph.plane_height()) * scale_local - italic_mid;
                let top_shear = -0.35 * top_term;
                let bottom_shear = -0.35 * bottom_term;
                extra = format!(
                    " plane=({:.3},{:.3}) origin_scale_x={:.3} center_scale_x={:.3} delta_x={:.3} plane_bearing_x={:.3} plane_w={:.3} plane_bearing_y={:.3} plane_h={:.3} italic_mid={:.3} top_shear={:.3} bottom_shear={:.3} shear_avg={:.3}",
                    plane_center_x,
                    plane_center_y,
                    origin_scaled_center_x,
                    center_scaled_center_x,
                    origin_scaled_center_x - center_scaled_center_x,
                    glyph.plane_bearing_x() * scale_local,
                    glyph.plane_width() * scale_local,
                    glyph.plane_bearing_y() * scale_local,
                    glyph.plane_height() * scale_local,
                    italic_mid,
                    top_shear,
                    bottom_shear,
                    (top_shear + bottom_shear) * 0.5
                );
            }
            println!(
                "  seg[{seg_idx}] ch=U+{:04X} '{}' cursor_x={:.3} size={:.3} scale={:.3} italic={} voffset={:?} baseline_shift={:.3} advance={:.3} bounds_center=({:.3},{:.3}){}",
                ch as u32,
                ch,
                cursor_x,
                render_size,
                seg_scale,
                seg.italic,
                seg.voffset,
                baseline_shift,
                advance,
                bounds_center_x,
                bounds_center_y,
                extra
            );
            cursor_x += advance * seg_scale;
        }
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(
        args.next()
            .ok_or_else(|| "缺少输入 JSON 路径".to_string())?,
    );
    let output = PathBuf::from(args.next().ok_or_else(|| "缺少输出 PNG 路径".to_string())?);
    let seq: i32 = args
        .next()
        .ok_or_else(|| "缺少 seq".to_string())?
        .parse()
        .map_err(|e| format!("解析 seq 失败: {e}"))?;
    let layer: i32 = args
        .next()
        .ok_or_else(|| "缺少 layer".to_string())?
        .parse()
        .map_err(|e| format!("解析 layer 失败: {e}"))?;

    let font_dir = Path::new("assets/fonts");
    let _ = install_fonts(font_dir)?;

    let provider = Arc::new(Task043Provider::load(Path::new(
        "tmp/render_cache/masterdata",
    ))?);
    let renderer_provider: Arc<dyn MasterDataProvider> = provider.clone();
    let renderer =
        CustomProfileRenderer::new(renderer_provider).with_assets(Arc::new(AssetStore::new(1)));

    let body: ProfileEnvelope = serde_json::from_str(
        &std::fs::read_to_string(&input)
            .map_err(|e| format!("读取 {} 失败: {e}", input.display()))?,
    )
    .map_err(|e| format!("解析 {} 失败: {e}", input.display()))?;

    let card = body
        .user_custom_profile_cards
        .iter()
        .find(|card| card.seq == seq)
        .ok_or_else(|| format!("未找到 seq={seq} 的名片"))?;
    let isolated = isolate_layer(&card.custom_profile_card, layer);
    if isolated.texts.is_empty() {
        return Err(format!("seq={seq} 中未找到 layer={layer} 的可见文本"));
    }
    if std::env::var("TASK043_DUMP_GEOMETRY").ok().as_deref() == Some("1") {
        for text in &isolated.texts {
            dump_geometry(provider.as_ref(), text)?;
        }
    }
    if std::env::var("TASK043_DUMP_LAYOUT").ok().as_deref() == Some("1") {
        for text in &isolated.texts {
            dump_layout_metrics(provider.as_ref(), text)?;
        }
    }
    if std::env::var("TASK043_DUMP_TEXT_SEGMENTS").ok().as_deref() == Some("1") {
        for text in &isolated.texts {
            println!("raw={}", text.text);
            for (idx, seg) in parse_rich_segments(&text.text).iter().enumerate() {
                println!(
                    "seg[{idx}] text={:?} size={:?} scale={:?} italic={} voffset={:?} pos={:?}",
                    seg.text, seg.size, seg.scale, seg.italic, seg.voffset, seg.position
                );
            }
        }
    }

    let png = renderer
        .render_page_png_transparent_with_profile(&isolated, None)
        .map_err(|e| format!("渲染失败: {e}"))?;

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录 {} 失败: {e}", parent.display()))?;
    }
    std::fs::write(&output, png).map_err(|e| format!("写入 {} 失败: {e}", output.display()))?;
    Ok(())
}
