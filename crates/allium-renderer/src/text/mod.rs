//! 文本渲染相关模块。

#[cfg(feature = "skia-core")]
mod font;
#[cfg(feature = "skia-core")]
mod measure;
pub mod richtext;

#[cfg(feature = "skia-core")]
use crate::masterdata::{MasterData, ResolvedColor};
#[cfg(feature = "skia-core")]
use crate::sdf::outline::{self as sdf_outline, lookup_or_generate};
#[cfg(feature = "skia-core")]
use crate::text::font::{resolve_tmp_face_info_constants, resolve_typeface};
#[cfg(feature = "skia-core")]
use crate::text::measure::{
    resolve_indent_value, resolve_segment_font_size, segments_to_global, tmp_measure_advance,
    transform_char_for_segment,
};
#[cfg(feature = "skia-core")]
use crate::text::richtext::{parse_rich_segments, Indent, InlineAlign, LineIndent, TextSegment};
#[cfg(feature = "skia-core")]
use crate::types::TextElement;
#[cfg(feature = "skia-core")]
use skia_safe::{
    Canvas, Color4f, Font, FontMgr, FontStyle, Matrix, Paint, PaintStyle, Point, Rect,
};

/// TMP FontAsset 全局缩放因子 (m_FaceInfo.m_Scale)。
pub const TEXT_SCALE: f32 = 2.0;

/// 解析自定义名片渲染同源字体。
///
/// 这个入口复用 `src/text/font.rs` 的字体字节缓存和 fallback 规则，供非名片场景
/// 绘制少量 UI 文本时避免退回到 Skia 默认字体。
#[cfg(feature = "skia-core")]
pub fn resolve_custom_profile_typeface(
    font_mgr: &FontMgr,
    family: Option<&str>,
) -> Option<skia_safe::Typeface> {
    resolve_typeface(font_mgr, family)
}

fn effective_vertex_alpha_u8(alpha_override: Option<f32>, base_alpha_u8: u8) -> u8 {
    let override_u8 =
        alpha_override.map(|alpha| (alpha.clamp(0.0, 1.0) * 255.0).round().clamp(0.0, 255.0) as u8);
    override_u8
        .map(|alpha| alpha.min(base_alpha_u8))
        .unwrap_or(base_alpha_u8)
}

#[cfg_attr(not(test), allow(dead_code))]
fn effective_vertex_alpha(alpha_override: Option<f32>, base_alpha_u8: u8) -> f32 {
    effective_vertex_alpha_u8(alpha_override, base_alpha_u8) as f32 / 255.0
}

#[cfg(feature = "skia-core")]
fn debug_text_probe_enabled() -> bool {
    std::env::var("SCAPUS_DEBUG_TMP_PROBE")
        .ok()
        .map(|v| {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
}

#[cfg(feature = "skia-core")]
fn update_cpv_width(max_width_tmp: &mut f32, cpv_xadv_tmp: f32, glyph_hadv_tmp: f32) {
    *max_width_tmp = (*max_width_tmp).max(cpv_xadv_tmp.abs() + glyph_hadv_tmp);
}

#[cfg(feature = "skia-core")]
#[allow(dead_code)]
#[derive(Debug)]
struct TmpDebugCharProbe {
    line_index: usize,
    ch: String,
    seg_size_tmp: f32,
    seg_scale: f32,
    char_scale: f32,
    baseline_offset_tmp: f32,
    pos_tmp: Option<f32>,
    x_advance_before_tmp: f32,
    glyph_advance_tmp_for_layout: f32,
    glyph_advance_tmp_for_caret: f32,
    x_advance_after_tmp: f32,
    preferred_width_candidate_tmp: f32,
}

#[cfg(feature = "skia-core")]
#[allow(dead_code)]
#[derive(Debug)]
struct TmpDebugLineProbe {
    line_index: usize,
    text: String,
    line_width_tmp_like: f32,
    preferred_width_tmp: f32,
    max_seg_size_tmp: f32,
    line_offset_tmp: f32,
    line_height_tmp: f32,
}

#[cfg(feature = "skia-core")]
#[allow(dead_code)]
#[derive(Debug)]
struct TmpDebugFinalMetrics {
    current_font_size_tmp: f32,
    baseline_offset_tmp: f32,
    x_advance_tmp: f32,
    preferred_width_tmp: f32,
    preferred_height_tmp: f32,
    margin_width_tmp: f32,
    margin_height_tmp: f32,
    text_alignment_hex: String,
    font_style_hex: String,
    font_style_internal_hex: String,
    padding_tmp: f32,
    outline_width_tmp: f32,
}

#[cfg(feature = "skia-core")]
struct DrawCharOp {
    ch: String,
    x: f32,
    y: f32,
    pivot_x: f32,
    pivot_y: f32,
    shear_cx: f32,
    scale_x: f32,
    skew_x: f32,
    rotate_deg: f32,
    font: Font,
    face: Paint,
    sdf_params: Option<crate::sdf::rasterize::SdfOutlineParams>,
    mesh_carrier: crate::sdf::rasterize::RuntimeLikeGlyphMeshCarrier,
}

/// 构造单字形的局部变换矩阵（相对画布当前 CTM 的增量），与渲染循环逐字绘制时
/// 对 canvas 施加的链式调用保持**逐字节同源**。debug 顶点输出与渲染都只走这一处，
/// 保证 debug 数值 == 实际渲染。
///
/// 复合顺序对齐游戏真机（il2cpp FX 块 `v' = C + M·(v−C)`，`M = Rotate·Scale`）：
/// 绕 glyph center（= anchor）施加 **R 外层、S 内层**，italic skew 最内层（真机
/// 在 FX 前先改顶点）。即：
///   T(anchor) · R(-rotate_deg) · S(scale_x,1) · Skew(skew_x)
/// 字形随后画在 (-pivot_x, -pivot_y)，使 glyph center 落在 anchor 上。
///
/// 退化等价：skew_x=0 时为 `T·R·S`；当 scale_x=1 或 rotate_deg=0 其一为平凡，
/// `R·S = S·R`，与旧 canvas 链 `T·S·R` 逐字节一致——剪切偏差仅在 scale 与 rotate
/// 同时非平凡时出现，正是 #4 要修的复合。
#[cfg(feature = "skia-core")]
fn glyph_local_matrix(op: &DrawCharOp) -> Matrix {
    let mut m = Matrix::new_identity();
    m.pre_translate((op.x + op.pivot_x + op.shear_cx, op.y + op.pivot_y));
    if op.rotate_deg.abs() > 0.001 {
        // TMP <rotate> 是 Unity Y-up/CCW，Skia Y-down/CW，取负翻转（与元素级
        // transform::quaternion_to_degrees 负号同源）。R 外层。
        m.pre_rotate(-op.rotate_deg, None);
    }
    m.pre_scale((op.scale_x, 1.0), None); // S 内层（先把字形横向拉成矩形）
    if op.skew_x != 0.0 {
        // italic skew 最内层：真机在 FX 块前先改顶点。
        m.pre_concat(&Matrix::from_affine(&[1.0, 0.0, op.skew_x, 1.0, 0.0, 0.0]));
    }
    m
}

/// 计算字形 footprint 四角经 `glyph_local_matrix` 变换后的设备前坐标（TMP 等效坐标系，
/// 乘 TEXT_SCALE）。footprint 取绕 glyph center 的 ±pivot 盒；刚性旋转下保持矩形，
/// 复合产生剪切时退化为平行四边形——四角即可直接量化剪切。
/// 返回 [TL, TR, BR, BL] 各 (x, y)。
#[cfg(feature = "skia-core")]
fn glyph_quad_corners(op: &DrawCharOp) -> [(f32, f32); 4] {
    let m = glyph_local_matrix(op);
    // 字形相对其 center（绘制原点在 -pivot）的局部盒。center 在原点，半展为 pivot。
    let (hx, hy) = (op.pivot_x.abs().max(1.0), op.pivot_y.abs().max(1.0));
    let local = [(-hx, -hy), (hx, -hy), (hx, hy), (-hx, hy)];
    let mut out = [(0.0f32, 0.0f32); 4];
    for (i, (lx, ly)) in local.iter().enumerate() {
        let p = m.map_point(Point::new(*lx, *ly));
        out[i] = (p.x * TEXT_SCALE, -p.y * TEXT_SCALE);
    }
    out
}

/// 绘制文本（逐段排版 + 描边 + 富文本标签支持）。
#[cfg(feature = "skia-core")]
pub fn draw_text(canvas: &Canvas, text: &TextElement, md: &MasterData) {
    if std::env::var("SCAPUS_DEBUG_TEXT_CODEPOINTS")
        .ok()
        .map(|v| {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
    {
        let cps: Vec<String> = text
            .text
            .chars()
            .map(|ch| format!("U+{:04X}", ch as u32))
            .collect();
        tracing::debug!(
            font_id = text.font_id,
            size = text.size,
            outline = text.outline_size,
            text = %text.text,
            cps = %cps.join(","),
            "TEXT_CODEPOINTS"
        );
    }

    let segments = parse_rich_segments(&text.text);
    let global = segments_to_global(&segments);
    let debug_probe = debug_text_probe_enabled();
    tracing::debug!(
        font_id = text.font_id,
        color_id = text.color_id,
        size = text.size,
        seg_count = segments.len(),
        raw_len = text.text.len(),
        raw_text = %text.text.chars().take(80).collect::<String>(),
        clean_text = %global.clean.chars().take(80).collect::<String>(),
        "draw_text 入口"
    );

    let font_mgr = FontMgr::default();
    let resolved_name = md.resolve_font(text.font_id);
    let resolved_name_ref = resolved_name.as_deref();
    let typeface = resolve_typeface(&font_mgr, resolved_name_ref)
        .or_else(|| font_mgr.match_family_style("Noto Sans CJK SC", FontStyle::default()))
        .or_else(|| font_mgr.match_family_style("Noto Sans CJK", FontStyle::default()))
        .or_else(|| font_mgr.legacy_make_typeface(None, FontStyle::default()));
    let Some(typeface) = typeface else {
        tracing::warn!(font_id = text.font_id, "无法获取默认字体，跳过文本元素");
        return;
    };

    let base_size = text.size;
    let base_font = Font::new(typeface.clone(), Some(base_size));

    const TMP_POINT_SIZE: f32 = 75.0;
    const TMP_ASCENT_RATIO: f32 = 66.0 / 75.0;
    const TMP_DESCENT_RATIO: f32 = 9.0 / 75.0;
    const SDF_DILATE_SCALE: f32 = 4.5;
    const TMP_POINT_SIZE_OUTLINE: f32 = 75.0;

    let tmp_ascent = -(TMP_ASCENT_RATIO * base_size);
    let tmp_descent = TMP_DESCENT_RATIO * base_size;
    let _base_font_h = -tmp_ascent + tmp_descent;
    let align = text.text_type & 0x07;

    let def_color = md.resolve_color(text.color_id).unwrap_or(ResolvedColor {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    });
    tracing::debug!(
        color_id = text.color_id,
        r = def_color.r,
        g = def_color.g,
        b = def_color.b,
        a = def_color.a,
        "draw_text 颜色解析"
    );

    let clean_owned: String;
    let clean: &str = match global.clean.strip_suffix('\n') {
        Some(s) => {
            clean_owned = s.to_string();
            &clean_owned
        }
        None => &global.clean,
    };
    let line_texts: Vec<&str> = clean.split('\n').collect();
    tracing::debug!(lines=%line_texts.len(), clean_bytes=%clean.len(), clean_escaped=%clean.escape_debug().to_string().chars().take(200).collect::<String>(), "text_lines");

    let mut line_segs: Vec<Vec<&TextSegment>> = vec![Vec::new()];
    for seg in &segments {
        for (j, part) in seg.text.split('\n').enumerate() {
            if j > 0 {
                line_segs.push(Vec::new());
            }
            if !part.is_empty() {
                line_segs
                    .last_mut()
                    .expect("line_segs 应至少有一行")
                    .push(seg);
            }
        }
    }

    let mut line_widths: Vec<f32> = Vec::new();
    let mut rect_widths: Vec<f32> = Vec::new();
    let mut line_max_sizes: Vec<f32> = Vec::new();
    let mut tmp_line_probes: Vec<TmpDebugLineProbe> = Vec::new();
    let mut tmp_char_probes: Vec<TmpDebugCharProbe> = Vec::new();
    let seg_cleans: Vec<String> = segments
        .iter()
        .map(|s| s.text.chars().filter(|c| *c != '\n').collect())
        .collect();
    let mut seg_consumed: Vec<usize> = vec![0; segments.len()];

    // 独立 caret 链：追踪 TMP 真实 xAdvance（乘 scale），与 CPV preferredWidth 链分离。
    let mut final_caret_xadv_tmp = 0.0f32;
    // vertical bounds 追踪：voffset 偏移后每个字形的上下极值（TMP 单位）。
    let mut vbounds_max_top_tmp = f32::NEG_INFINITY;
    let mut vbounds_min_bottom_tmp = f32::INFINITY;

    for (line_idx, line_str) in line_texts.iter().enumerate() {
        let mut w_scaled = 0.0f32;
        let mut max_seg_size = 0.0f32;
        let mut remaining = *line_str;
        let mut prev_cspace: Option<f32> = None;
        let mut cpv_xadv_tmp = 0.0f32;
        let mut max_cpv_width_tmp = 0.0f32;
        let mut has_chars = false;
        // TMP CPV 在每个可见字符前用当前 xAdvance 计算宽度；<pos> 只改 caret。
        let mut current_position: Option<Indent> = None;
        // caret 链：<scale> 影响字符前进，与 CPV width 链独立。
        let mut caret_xadv_tmp = 0.0f32;
        let mut caret_position: Option<Indent> = None;

        for (si, seg) in segments.iter().enumerate() {
            if remaining.is_empty() {
                break;
            }
            if let Some(fixed_advance) = seg.fixed_advance {
                let seg_font_size = resolve_segment_font_size(seg.size, text.size);
                if seg.position != current_position {
                    if let Some(pos_shift) = resolve_indent_value(seg.position, seg_font_size, 0.0)
                    {
                        cpv_xadv_tmp = pos_shift * TEXT_SCALE;
                        caret_xadv_tmp = pos_shift * TEXT_SCALE;
                    }
                    current_position = seg.position;
                    caret_position = seg.position;
                }
                let adv = fixed_advance / TEXT_SCALE;
                w_scaled += adv;
                cpv_xadv_tmp += fixed_advance;
                caret_xadv_tmp += fixed_advance;
                update_cpv_width(&mut max_cpv_width_tmp, cpv_xadv_tmp, 0.0);
                has_chars = true;
                if seg_font_size > max_seg_size {
                    max_seg_size = seg_font_size;
                }
                continue;
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
                if let Some(pos_shift) = resolve_indent_value(seg.position, seg_size, 0.0) {
                    cpv_xadv_tmp = pos_shift * TEXT_SCALE;
                    // TMP 在 <pos> 处重置 preferredWidth 追踪
                    max_cpv_width_tmp = 0.0;
                }
                current_position = seg.position;
            }
            if seg.position != caret_position {
                if let Some(pos_shift) = resolve_indent_value(seg.position, seg_size, 0.0) {
                    caret_xadv_tmp = pos_shift * TEXT_SCALE;
                }
                caret_position = seg.position;
            }
            let measure_size = if seg.subscript || seg.superscript {
                seg_size * 0.5
            } else {
                seg_size
            };
            let seg_font = Font::new(typeface.clone(), Some(measure_size));
            let part_chars: Vec<char> = part.chars().collect();
            let cspace_raw_tmp = seg.cspace.unwrap_or(0.0);
            let seg_scale = seg.scale.unwrap_or(1.0);
            // voffset 用于 vertical bounds 追踪（TMP 单位，Y-up）。
            let voffset_tmp = seg.voffset.unwrap_or(0.0);
            let mut measured = 0.0f32;
            for ch in &part_chars {
                let (display, char_scale) = transform_char_for_segment(*ch, seg);
                // advance 优先用 FreeType（与 TMP FontEngine 同源，真机 truth 已验证），
                // Skia measure_str 对半角符号（如 `)`）advance 偏小约 10%，导致 cspace 画弧层
                // 字符间距偏小、弧形变形。回退 Skia 仅用于 SDF 未覆盖字符。
                let ft_hadv = lookup_or_generate(resolved_name_ref, *ch)
                    .as_ref()
                    .map(|g| {
                        g.plane_advance_x() * (measure_size / sdf_outline::sampling_point_size())
                    })
                    .filter(|v| *v > 0.0);
                let glyph_hadv_tmp_layout = (ft_hadv
                    .unwrap_or_else(|| tmp_measure_advance(&display, &seg_font, measure_size)))
                    * char_scale
                    * TEXT_SCALE;
                measured += glyph_hadv_tmp_layout * seg_scale / TEXT_SCALE;
                update_cpv_width(&mut max_cpv_width_tmp, cpv_xadv_tmp, glyph_hadv_tmp_layout);
                // caret 链：字符前进乘以 scale。
                let glyph_hadv_tmp_caret = glyph_hadv_tmp_layout * seg_scale;
                // vertical bounds：字形在 voffset 偏移后的上下极值。
                // TMP 中 ascent = seg_size * (ASCENT_LINE / POINT_SIZE) * TEXT_SCALE，
                // descent 同理。voffset 向上为正（Y-up）。
                let glyph_asc_tmp = measure_size * (66.0 / 75.0) * TEXT_SCALE;
                let glyph_des_tmp = measure_size * (9.0 / 75.0) * TEXT_SCALE;
                let glyph_top = voffset_tmp + glyph_asc_tmp;
                let glyph_bottom = voffset_tmp - glyph_des_tmp;
                if glyph_top > vbounds_max_top_tmp {
                    vbounds_max_top_tmp = glyph_top;
                }
                if glyph_bottom < vbounds_min_bottom_tmp {
                    vbounds_min_bottom_tmp = glyph_bottom;
                }
                if debug_probe {
                    let before = cpv_xadv_tmp;
                    let after = cpv_xadv_tmp + glyph_hadv_tmp_layout + cspace_raw_tmp;
                    tmp_char_probes.push(TmpDebugCharProbe {
                        line_index: line_idx,
                        ch: display.clone(),
                        seg_size_tmp: measure_size,
                        seg_scale,
                        char_scale,
                        baseline_offset_tmp: voffset_tmp,
                        pos_tmp: seg.position.and_then(|pos| match pos {
                            Indent::Pixels(v) => Some(v),
                            Indent::Em(v) => Some(v * text.size),
                            Indent::Percent(_) => None,
                        }),
                        x_advance_before_tmp: before,
                        glyph_advance_tmp_for_layout: glyph_hadv_tmp_layout,
                        glyph_advance_tmp_for_caret: glyph_hadv_tmp_caret,
                        x_advance_after_tmp: after,
                        preferred_width_candidate_tmp: before.abs() + glyph_hadv_tmp_layout,
                    });
                }
                cpv_xadv_tmp = cpv_xadv_tmp + glyph_hadv_tmp_layout + cspace_raw_tmp;
                caret_xadv_tmp = caret_xadv_tmp + glyph_hadv_tmp_caret + cspace_raw_tmp;
            }
            let cspace = seg.cspace.unwrap_or(0.0) / TEXT_SCALE;
            let n_chars = part_chars.len();
            let cspace_total = cspace * n_chars as f32;
            w_scaled += measured + cspace_total;
            has_chars = true;
            prev_cspace = Some(cspace);
            if seg_size > max_seg_size {
                max_seg_size = seg_size;
            }
        }

        if !remaining.is_empty() {
            let measured = tmp_measure_advance(remaining, &base_font, base_size);
            w_scaled += measured * global.scale;
            for ch in remaining.chars() {
                let ch_text = ch.to_string();
                let glyph_hadv_tmp =
                    tmp_measure_advance(&ch_text, &base_font, base_size) * TEXT_SCALE;
                update_cpv_width(&mut max_cpv_width_tmp, cpv_xadv_tmp, glyph_hadv_tmp);
                cpv_xadv_tmp += glyph_hadv_tmp;
                caret_xadv_tmp += glyph_hadv_tmp;
                // vertical bounds：无 voffset 的 fallback 字符。
                let glyph_asc_tmp = base_size * (66.0 / 75.0) * TEXT_SCALE;
                let glyph_des_tmp = base_size * (9.0 / 75.0) * TEXT_SCALE;
                if glyph_asc_tmp > vbounds_max_top_tmp {
                    vbounds_max_top_tmp = glyph_asc_tmp;
                }
                if -glyph_des_tmp < vbounds_min_bottom_tmp {
                    vbounds_min_bottom_tmp = -glyph_des_tmp;
                }
            }
            has_chars = true;
            if base_size > max_seg_size {
                max_seg_size = base_size;
            }
        }

        if max_seg_size < 0.001 {
            // TMP 在空行时用当前 active style 的 metrics（\n 字符继承 active size）。
            // 优先取最后一个已消费 segment 的 size；若无（首行空），取首个 segment 的 size
            // （即 \n 发生时的 active style）。
            let active_size = segments
                .iter()
                .enumerate()
                .rev()
                .find(|(si, _)| seg_consumed[*si] > 0)
                .or_else(|| segments.iter().enumerate().next())
                .map(|(_, seg)| resolve_segment_font_size(seg.size, text.size))
                .unwrap_or(base_size);
            max_seg_size = active_size;
        }

        // TMP CENTER 对齐的 lineWidth = caret_xAdvance + trailing_cspace。
        // 行末多算一个 cspace 使对齐基准正确。
        if let Some(trailing_cspace) = prev_cspace {
            w_scaled += trailing_cspace;
        }
        line_widths.push(w_scaled);
        let rect_w = if has_chars {
            max_cpv_width_tmp / TEXT_SCALE
        } else {
            0.0
        };
        rect_widths.push(rect_w);
        line_max_sizes.push(max_seg_size);
        // 每行结束时记录 caret 链最终值（多行时取最后一行）。
        final_caret_xadv_tmp = caret_xadv_tmp;
        if debug_probe {
            tmp_line_probes.push(TmpDebugLineProbe {
                line_index: line_idx,
                text: (*line_str).to_string(),
                line_width_tmp_like: w_scaled * TEXT_SCALE,
                preferred_width_tmp: max_cpv_width_tmp,
                max_seg_size_tmp: max_seg_size,
                line_offset_tmp: 0.0,
                line_height_tmp: max_seg_size * TEXT_SCALE,
            });
        }
    }

    let base_line_h = text.size;
    let lh_override: Option<f32> = segments.iter().find_map(|s| s.line_height);
    let n_lines = line_max_sizes.len();

    // TMP lineGap 实测为 75.625（= m_LineHeight - (ascentLine - descentLine) + 0.625）
    // 0.625 是 TMP 内部的行间距修正项（通过 Frida 多数据点拟合确认）
    const ASCENT_LINE: f32 = 66.0;
    const DESCENT_LINE: f32 = -9.0;
    const LINE_GAP: f32 = 150.0 - (66.0 + 9.0) + 0.625;

    let mut line_asc: Vec<f32> = Vec::with_capacity(n_lines);
    let mut line_des: Vec<f32> = Vec::with_capacity(n_lines);
    for i in 0..n_lines {
        let ms = line_max_sizes[i];
        let es = (ms / TMP_POINT_SIZE) * TEXT_SCALE;
        let asc = es * ASCENT_LINE;
        let des = es * DESCENT_LINE;
        if i == 0 || ms > 0.001 {
            line_asc.push(asc);
            line_des.push(des);
        } else {
            line_asc.push(line_asc[i - 1]);
            line_des.push(line_des[i - 1]);
        }
    }

    let mut line_offsets = vec![0.0f32; n_lines];
    let ls_tmp = text.line_spacing * base_line_h * TEXT_SCALE / TMP_POINT_SIZE;
    for i in 1..n_lines {
        let delta = if let Some(lh) = lh_override {
            lh + ls_tmp
        } else {
            let asc_new = line_asc[i];
            let des_prev = line_des[i - 1];
            let base_scale = (base_line_h / TMP_POINT_SIZE) * TEXT_SCALE;
            asc_new + des_prev.abs() + LINE_GAP * base_scale + ls_tmp
        };
        line_offsets[i] = line_offsets[i - 1] + delta;
    }

    if debug_probe {
        for probe in &mut tmp_line_probes {
            if let Some(offset) = line_offsets.get(probe.line_index) {
                probe.line_offset_tmp = *offset;
            }
        }
    }

    // TMP m_maxTextAscender: 首行 ascender（lineCount==0 时设置，后续不更新）。
    // TMP m_ElementDescender: 末行 descender（每行覆盖，overflow 后停止更新）。
    let logical_max_asc = line_asc[0];
    let logical_min_des = line_des[n_lines - 1] - line_offsets[n_lines - 1];

    // vbounds 扩展视觉范围（用于 preferredHeight/margin 报告），不影响 anchor。
    // 当 line-height 标签存在时，vbounds 不应扩展 preferredHeight，因为 TMP 在
    // line-height 压缩行距时使用 logical box（基于 line_offsets）而非 glyph bounds。
    let effective_max_asc = if lh_override.is_none() && vbounds_max_top_tmp > f32::NEG_INFINITY {
        logical_max_asc.max(vbounds_max_top_tmp)
    } else {
        logical_max_asc
    };
    let effective_min_des = if lh_override.is_none() && vbounds_min_bottom_tmp < f32::INFINITY {
        logical_min_des.min(vbounds_min_bottom_tmp)
    } else {
        logical_min_des
    };

    let total_h_tmp = effective_max_asc - effective_min_des;
    let _total_h = total_h_tmp / TEXT_SCALE;
    let anchor_base = (effective_max_asc + effective_min_des) / (2.0 * TEXT_SCALE);
    let has_outline = text.outline_size > 0.0;
    let max_rw = rect_widths.iter().cloned().fold(0.0f32, f32::max);
    const PAD_ORIGINAL: f32 = 64.0 / TEXT_SCALE;
    let box_w = max_rw + PAD_ORIGINAL;

    let any_italic = segments.iter().any(|seg| seg.italic);
    let any_bold = segments.iter().any(|seg| seg.bold);
    let debug_align_hex = match align {
        2 => "0x1000202".to_string(),
        4 => "0x1000404".to_string(),
        _ => "0x10000ffff".to_string(),
    };
    let debug_font_style_hex = if any_italic {
        "0x200000000".to_string()
    } else if any_bold {
        "0x1".to_string()
    } else {
        "0x0".to_string()
    };
    let debug_font_style_internal_hex = if any_italic {
        "0x10000000002".to_string()
    } else if any_bold {
        "0x1".to_string()
    } else {
        "0x0".to_string()
    };
    let debug_current_font_size_tmp = line_max_sizes.iter().cloned().fold(0.0f32, f32::max);
    let debug_baseline_offset_tmp = tmp_char_probes
        .iter()
        .map(|probe| probe.baseline_offset_tmp)
        .rev()
        .find(|offset| offset.abs() > 0.0001)
        .unwrap_or(0.0);
    // xAdvance 使用测量循环中的独立 caret 链（乘 scale），不再依赖渲染循环 cursor。
    let debug_final_x_advance_tmp = final_caret_xadv_tmp;

    if debug_probe {
        let raw_text_json =
            serde_json::to_string(&text.text).unwrap_or_else(|_| "\"<encode-error>\"".to_string());
        tracing::debug!(
            layer = text.object_data.layer,
            raw_text = %text.text,
            raw_text_json = %raw_text_json,
            font_id = text.font_id,
            base_size = text.size,
            outline_size = text.outline_size,
            line_spacing = text.line_spacing,
            line_widths = ?line_widths,
            rect_widths = ?rect_widths,
            box_w,
            preferred_height_tmp = total_h_tmp,
            margin_width_tmp = max_rw * TEXT_SCALE + 64.0,
            margin_height_tmp = total_h_tmp + 64.0,
            align,
            any_italic = any_italic,
            any_bold = any_bold,
            line_max_sizes = ?line_max_sizes,
            line_offsets = ?line_offsets,
            anchor_base,
            tmp_line_probes = ?tmp_line_probes,
            tmp_char_probes = ?tmp_char_probes,
            "TMP_DEBUG_LAYOUT"
        );
    }

    let mut render_consumed: Vec<usize> = vec![0; segments.len()];
    let mut draw_ops = Vec::new();

    for (i, line_str) in line_texts.iter().enumerate() {
        let sw = line_widths[i];
        let line_align = line_segs
            .get(i)
            .and_then(|ls| ls.first())
            .and_then(|seg| seg.align)
            .or(global.align);
        let effective_align = match line_align {
            Some(InlineAlign::Left) => 1,
            Some(InlineAlign::Center) => 2,
            Some(InlineAlign::Right) => 4,
            None => align,
        };
        let lx = match effective_align {
            2 => -sw / 2.0,
            4 => box_w / 2.0 - sw,
            _ => -box_w / 2.0,
        };
        let ly = anchor_base + line_offsets[i] / TEXT_SCALE;
        let mut cursor_x = lx;
        // 解析后的 position 是状态；同一个 <pos> 跨颜色/voffset 分段时只应跳转一次。
        let mut current_position: Option<Indent> = None;

        if let Some(li_seg) = line_segs.get(i).and_then(|ls| ls.first()) {
            if let Some(ref indent) = li_seg.indent {
                match indent {
                    Indent::Percent(p) => {
                        let pct = *p / 100.0;
                        if pct < 1.0 {
                            const TMP_PAD: f32 = 64.0;
                            let sw_canvas = sw * TEXT_SCALE;
                            let rect = (sw_canvas + TMP_PAD) / (1.0 - pct);
                            let indent_skia = rect * pct / TEXT_SCALE;
                            cursor_x = match effective_align {
                                2 => (indent_skia - sw) / 2.0,
                                4 => rect / (2.0 * TEXT_SCALE) - sw,
                                _ => rect * (pct - 0.5) / TEXT_SCALE,
                            };
                        }
                    }
                    Indent::Pixels(px) => {
                        cursor_x += px / TEXT_SCALE;
                    }
                    Indent::Em(em) => {
                        let em_px = em * resolve_segment_font_size(li_seg.size, text.size);
                        cursor_x += em_px / TEXT_SCALE;
                    }
                }
            }
            if let Some(ref li) = li_seg.line_indent {
                match li {
                    LineIndent::Percent(p) => {
                        let pct = *p / 100.0;
                        if pct < 1.0 {
                            const TMP_PAD: f32 = 64.0;
                            let sw_canvas = sw * TEXT_SCALE;
                            let rect = (sw_canvas + TMP_PAD) / (1.0 - pct);
                            let indent_skia = rect * pct / TEXT_SCALE;
                            cursor_x = match effective_align {
                                2 => (indent_skia - sw) / 2.0,
                                4 => rect / (2.0 * TEXT_SCALE) - sw,
                                _ => rect * (pct - 0.5) / TEXT_SCALE,
                            };
                        }
                    }
                    LineIndent::Pixels(px) => {
                        cursor_x = lx + px;
                    }
                }
            }
        }

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
                if let Some(pos_shift) = resolve_indent_value(seg.position, seg_size, box_w) {
                    cursor_x = lx + pos_shift;
                }
                current_position = seg.position;
            }
            let face_info = resolve_tmp_face_info_constants(resolved_name_ref);
            let point_size = face_info.point_size.max(1.0);
            let (render_size, mut baseline_shift) = if seg.subscript {
                (
                    seg_size * face_info.subscript_size,
                    (face_info.subscript_offset * seg_size / point_size) / TEXT_SCALE,
                )
            } else if seg.superscript {
                (
                    seg_size * face_info.superscript_size,
                    (face_info.superscript_offset * seg_size / point_size) / TEXT_SCALE,
                )
            } else {
                (seg_size, 0.0)
            };
            if let Some(vo) = seg.voffset {
                baseline_shift = -vo / TEXT_SCALE;
            }
            let cspace_px = seg.cspace.unwrap_or(0.0) / TEXT_SCALE;
            let seg_font = Font::new(typeface.clone(), Some(render_size));

            let (sr, sg, sb) = seg.color.unwrap_or((def_color.r, def_color.g, def_color.b));
            let sa_u8 = effective_vertex_alpha_u8(seg.alpha, def_color.a);
            let sa = sa_u8 as f32 / 255.0;

            let mut fp = Paint::default();
            fp.set_style(PaintStyle::Fill);
            fp.set_color4f(
                Color4f::new(sr as f32 / 255.0, sg as f32 / 255.0, sb as f32 / 255.0, 1.0),
                None,
            );
            fp.set_anti_alias(true);

            let part_chars: Vec<char> = part.chars().collect();
            let mut measured = 0.0f32;
            for ch in &part_chars {
                let (display, char_scale) = transform_char_for_segment(*ch, seg);
                measured +=
                    tmp_measure_advance(&display, &seg_font, render_size) * seg_scale * char_scale;
            }

            if let Some((mr, mg, mb, ma)) = seg.mark_color {
                let mut mp = Paint::default();
                mp.set_style(PaintStyle::Fill);
                mp.set_color4f(
                    Color4f::new(
                        mr as f32 / 255.0,
                        mg as f32 / 255.0,
                        mb as f32 / 255.0,
                        ma as f32 / 255.0,
                    ),
                    None,
                );
                mp.set_anti_alias(true);
                let rect = Rect::from_xywh(
                    cursor_x,
                    ly - render_size * 0.85,
                    measured,
                    render_size * 1.1,
                );
                canvas.draw_rect(rect, &mp);
            }

            let seg_chars: Vec<char> = part_chars;
            for ch in &seg_chars {
                let (ch_str, char_scale) = transform_char_for_segment(*ch, seg);
                let effective_scale = seg_scale * char_scale;
                let ch_advance = tmp_measure_advance(&ch_str, &seg_font, render_size);
                let (_, glyph_bounds) = seg_font.measure_str(&ch_str, None);
                let mono_cell = resolve_indent_value(seg.monospace, seg_size, box_w)
                    .map(|width| {
                        if seg.duospace && matches!(*ch, '.' | ':' | ',') {
                            width / 2.0
                        } else {
                            width
                        }
                    })
                    .unwrap_or(0.0);
                let glyph_center_x = (glyph_bounds.left + glyph_bounds.right) / 2.0;
                // 查询 SDF glyph，获取 FreeType 度量（与 TMP FontEngine 同源，NO_HINTING）
                let sdf_glyph = lookup_or_generate(resolved_name_ref, *ch);
                let ft_scale = render_size / sdf_outline::sampling_point_size();
                let ft_advance_x = sdf_glyph.as_ref().map(|g| g.plane_advance_x() * ft_scale);
                let ft_pivot_x = sdf_glyph
                    .as_ref()
                    .map(|g| (g.plane_bearing_x() + g.plane_width() / 2.0) * ft_scale);
                // 优先使用 FreeType 度量计算 pivot，回退到 Skia
                let pivot_x = ft_pivot_x.unwrap_or_else(|| {
                    if (effective_scale - 1.0).abs() > 0.001 {
                        glyph_center_x
                    } else {
                        ch_advance / 2.0
                    }
                });
                let pivot_y = (glyph_bounds.top + glyph_bounds.bottom) / 2.0;
                // FreeType Y 中心：TMP 使用 FontEngine 的 bearingY - height/2
                // Skia Y-down 对应: -(bearing_y_75 - height_75/2) * ft_scale
                let ft_pivot_y = sdf_glyph
                    .as_ref()
                    .map(|g| -(g.plane_bearing_y() - g.plane_height() / 2.0) * ft_scale);
                let pivot_y = ft_pivot_y.unwrap_or(pivot_y);
                // TMP italic shear 公式（从源码 + Frida 5 字符验证推导）：
                // midPoint = height/2 + TMP_SPREAD; center_shift = 0.35 * (bY - h - spread) * base_eS
                // 等价于：shear_cx = 0.35 * (bearingY - height - spread) * ft_scale
                // base_eS 不含 scale 标签（center 在 scale 变换下不变，已验证）
                let shear_cx = if seg.italic {
                    if let Some(g) = sdf_glyph.as_ref() {
                        let bearing_y = g.plane_bearing_y();
                        let height = g.plane_height();
                        let spread = sdf_outline::sampling_spread();
                        0.35 * (bearing_y - height - spread) * ft_scale
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };
                let draw_x = if mono_cell > 0.0 {
                    cursor_x + mono_cell / 2.0 - glyph_center_x
                } else {
                    cursor_x
                };

                draw_ops.push(DrawCharOp {
                    ch: ch_str,
                    x: draw_x,
                    y: ly + baseline_shift,
                    pivot_x,
                    pivot_y,
                    shear_cx,
                    scale_x: effective_scale,
                    skew_x: if seg.italic { -0.21 } else { 0.0 },
                    rotate_deg: seg.rotate.unwrap_or(0.0),
                    font: seg_font.clone(),
                    face: fp.clone(),
                    sdf_params: if has_outline {
                        md.resolve_color(text.outline_color_id).map(|oc| {
                            crate::sdf::rasterize::SdfOutlineParams {
                                outline_r: oc.r as f32 / 255.0,
                                outline_g: oc.g as f32 / 255.0,
                                outline_b: oc.b as f32 / 255.0,
                                outline_a: oc.a as f32 / 255.0,
                                outline_size: text.outline_size,
                                font_size: render_size,
                            }
                        })
                    } else {
                        None
                    },
                    mesh_carrier: crate::sdf::rasterize::runtime_like_mesh_carrier(
                        render_size,
                        seg.bold,
                        sa_u8,
                    ),
                });

                if mono_cell > 0.0 {
                    cursor_x += mono_cell + cspace_px;
                } else {
                    // cursor 推进用 FreeType advance（与真机 TMP FontEngine 同源），
                    // Skia measure_str 对半角符号 advance 偏小，会导致 cspace 画弧层间距偏小。
                    let adv = ft_advance_x.unwrap_or(ch_advance);
                    cursor_x += adv * effective_scale + cspace_px;
                }
            }

            if seg.underline && !seg_chars.is_empty() {
                let ux = cursor_x - measured;
                let uy = ly + baseline_shift + render_size * 0.15;
                let mut up = Paint::default();
                up.set_style(PaintStyle::Stroke);
                up.set_stroke_width((render_size * 0.05).max(1.0));
                up.set_color4f(
                    Color4f::new(sr as f32 / 255.0, sg as f32 / 255.0, sb as f32 / 255.0, sa),
                    None,
                );
                up.set_anti_alias(true);
                canvas.draw_line(Point::new(ux, uy), Point::new(cursor_x, uy), &up);
            }

            if seg.strikethrough && !seg_chars.is_empty() {
                let sx = cursor_x - measured;
                let sy = ly + baseline_shift - render_size * 0.3;
                let mut sp = Paint::default();
                sp.set_style(PaintStyle::Stroke);
                sp.set_stroke_width((render_size * 0.05).max(1.0));
                sp.set_color4f(
                    Color4f::new(sr as f32 / 255.0, sg as f32 / 255.0, sb as f32 / 255.0, sa),
                    None,
                );
                sp.set_anti_alias(true);
                canvas.draw_line(Point::new(sx, sy), Point::new(cursor_x, sy), &sp);
            }
        }

        if !remaining.is_empty() {
            let (fr, fg, fb) = global
                .color
                .unwrap_or((def_color.r, def_color.g, def_color.b));
            let fa_u8 = effective_vertex_alpha_u8(global.alpha, def_color.a);
            let mut fp = Paint::default();
            fp.set_style(PaintStyle::Fill);
            fp.set_color4f(
                Color4f::new(fr as f32 / 255.0, fg as f32 / 255.0, fb as f32 / 255.0, 1.0),
                None,
            );
            fp.set_anti_alias(true);

            draw_ops.push(DrawCharOp {
                ch: remaining.to_string(),
                x: cursor_x,
                y: ly,
                pivot_x: 0.0,
                pivot_y: 0.0,
                shear_cx: 0.0,
                scale_x: global.scale,
                skew_x: 0.0,
                rotate_deg: 0.0,
                font: base_font.clone(),
                face: fp,
                sdf_params: if has_outline {
                    md.resolve_color(text.outline_color_id).map(|oc| {
                        crate::sdf::rasterize::SdfOutlineParams {
                            outline_r: oc.r as f32 / 255.0,
                            outline_g: oc.g as f32 / 255.0,
                            outline_b: oc.b as f32 / 255.0,
                            outline_a: oc.a as f32 / 255.0,
                            outline_size: text.outline_size,
                            font_size: base_size,
                        }
                    })
                } else {
                    None
                },
                mesh_carrier: crate::sdf::rasterize::runtime_like_mesh_carrier(
                    base_size, false, fa_u8,
                ),
            });
        }

        if debug_probe {
            // xAdvance 现在由测量循环的独立 caret 链提供，不再从渲染循环 cursor 计算。
        }
    }

    let _ = (SDF_DILATE_SCALE, TMP_POINT_SIZE_OUTLINE);
    let mut sdf_face_fallback_count: u32 = 0;
    for op in &draw_ops {
        if op.ch.chars().all(char::is_whitespace) {
            continue;
        }
        canvas.save();
        // 逐字复合矩阵走 glyph_local_matrix（与 debug char_quads 同源）：绕 glyph center
        // R 外层·S 内层·italic skew 最内层，对齐 il2cpp FX 块的 Rotate·Scale，消除 #4 剪切。
        canvas.concat(&glyph_local_matrix(op));
        if let Some(ref sdf_p) = op.sdf_params {
            let fc = op.face.color4f();
            crate::sdf::rasterize::render_char_sdf(
                canvas,
                &op.ch,
                Point::new(-op.pivot_x, -op.pivot_y),
                &op.font,
                resolved_name_ref,
                op.mesh_carrier,
                op.scale_x,
                fc,
                sdf_p,
            );
        } else {
            let mut fp = op.face.clone();
            let fc = fp.color4f();
            let rendered = crate::sdf::rasterize::render_char_face_from_atlas(
                canvas,
                &op.ch,
                Point::new(-op.pivot_x, -op.pivot_y),
                &op.font,
                resolved_name_ref,
                op.mesh_carrier,
                op.scale_x,
                fc,
            );
            if !rendered {
                sdf_face_fallback_count += 1;
                tracing::debug!(
                    text = %op.ch,
                    font_family = resolved_name_ref.unwrap_or("<none>"),
                    "outline SDF face glyph generation failed; falling back to plain text draw"
                );
                fp.set_color4f(
                    Color4f::new(fc.r, fc.g, fc.b, fc.a * op.mesh_carrier.vertex_alpha()),
                    None,
                );
                canvas.draw_str(&op.ch, Point::new(-op.pivot_x, -op.pivot_y), &op.font, &fp);
            }
        }
        canvas.restore();
    }

    // SDF 字形回退汇总：每文本元素最多一条 WARN（而非每字形一条），避免大量缺字形时刷屏。
    if sdf_face_fallback_count > 0 {
        tracing::warn!(
            count = sdf_face_fallback_count,
            font_family = resolved_name_ref.unwrap_or("<none>"),
            "SDF 字形回退到纯文本绘制"
        );
    }

    if debug_probe {
        let final_metrics = TmpDebugFinalMetrics {
            current_font_size_tmp: debug_current_font_size_tmp,
            baseline_offset_tmp: debug_baseline_offset_tmp,
            x_advance_tmp: debug_final_x_advance_tmp,
            preferred_width_tmp: max_rw * TEXT_SCALE,
            preferred_height_tmp: total_h_tmp,
            margin_width_tmp: max_rw * TEXT_SCALE + 64.0,
            margin_height_tmp: total_h_tmp + 64.0,
            text_alignment_hex: debug_align_hex,
            font_style_hex: debug_font_style_hex,
            font_style_internal_hex: debug_font_style_internal_hex,
            padding_tmp: 64.0 / 8.0,
            outline_width_tmp: text.outline_size,
        };
        // 输出每个字符的最终绘制中心坐标（TMP 等效坐标系：乘以 TEXT_SCALE）。
        // 与 Frida 采集的 characterInfo vertex center 同语义，用于全量对比。
        // Frida 报告所有字符（含 \n），\n 的 center=(0,0)。
        // 我们按原始 clean 文本顺序输出，\n 插入占位符。
        let char_positions: Vec<(String, f32, f32, f32, f32, f32)> = {
            let mut positions = Vec::new();
            let mut op_idx = 0;
            for ch in clean.chars() {
                if ch == '\n' {
                    positions.push(("\\n".to_string(), 0.0, 0.0, 1.0, 0.0, 0.0));
                } else if op_idx < draw_ops.len() {
                    let op = &draw_ops[op_idx];
                    let cx = (op.x + op.pivot_x + op.shear_cx) * TEXT_SCALE;
                    let cy = -(op.y + op.pivot_y) * TEXT_SCALE;
                    positions.push((op.ch.clone(), cx, cy, op.scale_x, op.skew_x, op.pivot_x));
                    op_idx += 1;
                }
            }
            positions
        };
        let char_ops: Vec<(String, f32, f32, f32, f32, f32, f32)> = draw_ops
            .iter()
            .map(|op| {
                (
                    op.ch.clone(),
                    op.x,
                    op.y,
                    op.scale_x,
                    op.pivot_x,
                    op.pivot_y,
                    op.rotate_deg,
                )
            })
            .collect();
        // 变换后字形 footprint 四角（[TL,TR,BR,BL]），用于 #4 剪切/尺寸的顶点级回归。
        // 刚性旋转下为矩形；S·R 复合剪切时为平行四边形。与 glyph_local_matrix 同源。
        let char_quads: Vec<(String, [(f32, f32); 4])> = draw_ops
            .iter()
            .map(|op| (op.ch.clone(), glyph_quad_corners(op)))
            .collect();
        let raw_text_json =
            serde_json::to_string(&text.text).unwrap_or_else(|_| "\"<encode-error>\"".to_string());
        let raw_text_escaped = text.text.replace('\n', "\\n").replace('\r', "\\r");
        tracing::debug!(
            layer = text.object_data.layer,
            raw_text = %raw_text_escaped,
            raw_text_json = %raw_text_json,
            final_metrics = ?final_metrics,
            char_positions = ?char_positions,
            char_ops = ?char_ops,
            char_quads = ?char_quads,
            "TMP_DEBUG_DRAW"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::effective_vertex_alpha;

    #[test]
    fn effective_vertex_alpha_caps_override_by_base_alpha() {
        let alpha = effective_vertex_alpha(Some(0.8), 128);
        assert!((alpha - (128.0 / 255.0)).abs() < 1e-6);
    }

    #[test]
    fn effective_vertex_alpha_uses_override_when_lower_than_base() {
        let alpha = effective_vertex_alpha(Some(0.25), 255);
        assert!((alpha - (64.0 / 255.0)).abs() < 1e-6);
    }

    #[test]
    fn effective_vertex_alpha_falls_back_to_base_alpha() {
        let alpha = effective_vertex_alpha(None, 64);
        assert!((alpha - (64.0 / 255.0)).abs() < 1e-6);
    }

    #[cfg(feature = "skia-core")]
    #[test]
    fn cpv_width_uses_pos_reset_instead_of_natural_sum() {
        let mut width = 0.0;
        let mut xadv = 0.0;
        let glyph = 36.0;

        super::update_cpv_width(&mut width, xadv, glyph);
        xadv = 0.0;
        super::update_cpv_width(&mut width, xadv, glyph);

        assert!((width - glyph).abs() < 1e-6);
    }

    #[cfg(feature = "skia-core")]
    #[test]
    fn cpv_width_keeps_negative_pos_extent() {
        let mut width = 0.0;

        super::update_cpv_width(&mut width, -221.0, 31.0);

        assert!((width - 252.0).abs() < 1e-6);
    }
}
