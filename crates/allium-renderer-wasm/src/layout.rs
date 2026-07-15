use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const TEXT_SCALE: f32 = 2.0;
const TMP_POINT_SIZE: f32 = 75.0;
const ASCENT_LINE: f32 = 66.0;
const DESCENT_LINE: f32 = -9.0;
const LINE_GAP: f32 = 150.0 - (66.0 + 9.0) + 0.625;
const PAD_ORIGINAL: f32 = 64.0 / TEXT_SCALE;
const TMP_SHADER_CLAMP: f32 = 1.0;
const GRADIENT_SCALE: f32 = 6.0;
const FACE_DILATE: f32 = 0.0;
const OUTLINE_WIDTH: f32 = 0.0;
const OUTLINE_SOFTNESS: f32 = 0.0;
const UNDERLAY_SOFTNESS: f32 = 0.0;
const WEIGHT_NORMAL: f32 = 0.0;
const WEIGHT_BOLD: f32 = 0.75;
const SHARPNESS: f32 = 0.0;
const RUNTIME_SCALE_RATIO_C: f32 = 0.6770833;
const RUNTIME_PIXEL_SCALE: f32 = 763.6753237;

pub fn build_layout_json(input: &str) -> Result<String, String> {
    let request: LayoutRequest =
        serde_json::from_str(input).map_err(|err| format!("parse layout json failed: {err}"))?;
    let batch = build_layout(request);
    serde_json::to_string(&batch).map_err(|err| format!("serialize layout json failed: {err}"))
}

pub fn build_glyph_demand_json(input: &str) -> Result<String, String> {
    let request: GlyphDemandRequest = serde_json::from_str(input)
        .map_err(|err| format!("parse glyph-demand json failed: {err}"))?;
    let mut seen = BTreeSet::new();
    let mut requests = Vec::new();
    for layer in request.layers {
        for ch in glyph_demand_chars(&layer.text) {
            let identity = (
                layer.region.clone(),
                layer.font_family.clone(),
                layer.font_source_hash.clone(),
                ch,
            );
            if seen.insert(identity) {
                requests.push(GlyphDemandEntry {
                    region: layer.region.clone(),
                    family: layer.font_family.clone(),
                    font_source_hash: layer.font_source_hash.clone(),
                    ch: ch.to_string(),
                });
            }
        }
    }
    serde_json::to_string(&GlyphDemandBatch {
        version: 1,
        source: "wasm-tmp-glyph-demand",
        requests,
    })
    .map_err(|err| format!("serialize glyph-demand json failed: {err}"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlyphDemandRequest {
    layers: Vec<GlyphDemandLayer>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlyphDemandLayer {
    text: String,
    region: String,
    font_family: String,
    font_source_hash: String,
}

#[derive(Serialize)]
struct GlyphDemandBatch {
    version: u32,
    source: &'static str,
    requests: Vec<GlyphDemandEntry>,
}

#[derive(Serialize)]
struct GlyphDemandEntry {
    region: String,
    family: String,
    font_source_hash: String,
    #[serde(rename = "char")]
    ch: String,
}

fn build_layout(request: LayoutRequest) -> LayoutBatch {
    let mut instances = Vec::new();
    let mut dynamic_programs = Vec::new();
    for layer in &request.layers {
        let (layer_instances, dynamic_program) =
            layout_layer(layer, &request.atlas, &request.atlas.glyphs);
        instances.extend(layer_instances);
        if let Some(program) = dynamic_program {
            dynamic_programs.push(program);
        }
    }
    LayoutBatch {
        version: 1,
        source: "wasm-sdf-freetype-layout".to_string(),
        instances,
        dynamic_programs,
    }
}

fn layout_layer(
    layer: &TextLayer,
    atlas: &AtlasInput,
    glyphs: &[GlyphInfo],
) -> (Vec<GlyphInstance>, Option<DynamicProgramDescriptor>) {
    let segments = parse_rich_segments(&layer.text);
    let dynamic_percent = line_indent_dynamic_percent(&layer.text, &segments);
    let global = segments_to_global(&segments);
    let mut clean = global.clean.clone();
    if clean.ends_with('\n') {
        clean.pop();
    }
    let line_texts = clean
        .split('\n')
        .map(|part| part.to_string())
        .collect::<Vec<_>>();
    let mut line_segs: Vec<Vec<TextSegment>> = vec![Vec::new()];
    for seg in &segments {
        for (idx, part) in seg.text.split('\n').enumerate() {
            if idx > 0 {
                line_segs.push(Vec::new());
            }
            if !part.is_empty() {
                line_segs.last_mut().unwrap().push(seg.clone());
            }
        }
    }

    let seg_cleans = segments
        .iter()
        .map(|seg| {
            seg.text
                .chars()
                .filter(|ch| *ch != '\n')
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut measure_consumed = vec![0usize; segments.len()];
    let mut line_widths = Vec::new();
    let mut rect_widths = Vec::new();
    let mut line_max_sizes = Vec::new();
    let mut vbounds_max_top = f32::NEG_INFINITY;
    let mut vbounds_min_bottom = f32::INFINITY;
    let mut line_advances_tmp = Vec::new();

    for line_text in &line_texts {
        let mut current_line_advances_tmp = Vec::new();
        let mut remaining = line_text.chars().collect::<Vec<_>>();
        let mut w_scaled = 0.0;
        let mut max_seg_size: f32 = 0.0;
        let mut prev_cspace = None;
        let mut cpv_xadv_tmp = 0.0;
        let mut max_cpv_width_tmp = 0.0;
        let mut has_chars = false;
        let mut current_position: Option<Indent> = None;
        let mut caret_position: Option<Indent> = None;
        let mut caret_xadv_tmp = 0.0;

        for (si, seg) in segments.iter().enumerate() {
            if remaining.is_empty() {
                break;
            }
            if let Some(fixed) = seg.fixed_advance {
                let seg_font_size = resolve_segment_font_size(&seg.size, layer.font_size);
                if seg.position != current_position {
                    if let Some(pos_shift) = resolve_indent_value(&seg.position, seg_font_size, 0.0)
                    {
                        cpv_xadv_tmp = pos_shift * TEXT_SCALE;
                        caret_xadv_tmp = pos_shift * TEXT_SCALE;
                    }
                    current_position = seg.position.clone();
                    caret_position = seg.position.clone();
                }
                let adv = fixed / TEXT_SCALE;
                w_scaled += adv;
                cpv_xadv_tmp += fixed;
                caret_xadv_tmp += fixed;
                current_line_advances_tmp.push(fixed);
                max_cpv_width_tmp = update_cpv_width(max_cpv_width_tmp, cpv_xadv_tmp, 0.0);
                has_chars = true;
                max_seg_size = max_seg_size.max(seg_font_size);
                continue;
            }

            let sc = &seg_cleans[si];
            if sc.is_empty() || measure_consumed[si] >= sc.len() {
                continue;
            }
            let seg_rest = &sc[measure_consumed[si]..];
            let part: Vec<char>;
            if starts_with_chars(&remaining, seg_rest) {
                part = seg_rest.to_vec();
                remaining.drain(..seg_rest.len());
                measure_consumed[si] = sc.len();
            } else if starts_with_chars(seg_rest, &remaining) {
                part = remaining.clone();
                measure_consumed[si] += remaining.len();
                remaining.clear();
            } else {
                continue;
            }
            if part.is_empty() {
                continue;
            }

            let seg_size = resolve_segment_font_size(&seg.size, layer.font_size);
            if seg.position != current_position {
                if let Some(pos_shift) = resolve_indent_value(&seg.position, seg_size, 0.0) {
                    cpv_xadv_tmp = pos_shift * TEXT_SCALE;
                    max_cpv_width_tmp = 0.0;
                }
                current_position = seg.position.clone();
            }
            if seg.position != caret_position {
                if let Some(pos_shift) = resolve_indent_value(&seg.position, seg_size, 0.0) {
                    caret_xadv_tmp = pos_shift * TEXT_SCALE;
                }
                caret_position = seg.position.clone();
            }

            let measure_size = if seg.subscript || seg.superscript {
                seg_size * 0.5
            } else {
                seg_size
            };
            let seg_scale = seg.scale.unwrap_or(1.0);
            let cspace_raw_tmp = seg.cspace.unwrap_or(0.0);
            let voffset_tmp = seg.voffset.unwrap_or(0.0);
            let mut measured = 0.0;
            let mut rendered_count = 0usize;
            for raw_ch in part {
                for (rendered_ch, char_scale) in transformed_glyphs(raw_ch, seg) {
                    let text = rendered_ch.to_string();
                    let glyph = if force_fallback_glyph(rendered_ch) {
                        None
                    } else {
                        glyph_for(layer, glyphs, &text)
                    };
                    let glyph_advance_tmp = glyph_advance(
                        glyph,
                        rendered_ch,
                        measure_size,
                        atlas.base_size,
                        &layer.font_family,
                    ) * char_scale
                        * TEXT_SCALE;
                    measured += (glyph_advance_tmp * seg_scale) / TEXT_SCALE;
                    rendered_count += 1;
                    max_cpv_width_tmp =
                        update_cpv_width(max_cpv_width_tmp, cpv_xadv_tmp, glyph_advance_tmp);
                    let glyph_advance_caret = glyph_advance_tmp * seg_scale;
                    current_line_advances_tmp.push(glyph_advance_caret + cspace_raw_tmp);
                    let glyph_asc_tmp = measure_size * (66.0 / 75.0) * TEXT_SCALE;
                    let glyph_des_tmp = measure_size * (9.0 / 75.0) * TEXT_SCALE;
                    vbounds_max_top = vbounds_max_top.max(voffset_tmp + glyph_asc_tmp);
                    vbounds_min_bottom = vbounds_min_bottom.min(voffset_tmp - glyph_des_tmp);
                    cpv_xadv_tmp += glyph_advance_tmp + cspace_raw_tmp;
                    caret_xadv_tmp += glyph_advance_caret + cspace_raw_tmp;
                }
            }
            let cspace = seg.cspace.unwrap_or(0.0) / TEXT_SCALE;
            w_scaled += measured + cspace * rendered_count as f32;
            has_chars = true;
            prev_cspace = Some(cspace);
            max_seg_size = max_seg_size.max(seg_size);
        }

        if !remaining.is_empty() {
            for raw_ch in remaining {
                let glyph = if force_fallback_glyph(raw_ch) {
                    None
                } else {
                    glyph_for(layer, glyphs, &raw_ch.to_string())
                };
                let glyph_advance_tmp = glyph_advance(
                    glyph,
                    raw_ch,
                    layer.font_size,
                    atlas.base_size,
                    &layer.font_family,
                ) * TEXT_SCALE;
                w_scaled += glyph_advance_tmp / TEXT_SCALE;
                max_cpv_width_tmp =
                    update_cpv_width(max_cpv_width_tmp, cpv_xadv_tmp, glyph_advance_tmp);
                cpv_xadv_tmp += glyph_advance_tmp;
                caret_xadv_tmp += glyph_advance_tmp;
                current_line_advances_tmp.push(glyph_advance_tmp);
                vbounds_max_top = vbounds_max_top.max(layer.font_size * (66.0 / 75.0) * TEXT_SCALE);
                vbounds_min_bottom =
                    vbounds_min_bottom.min(-layer.font_size * (9.0 / 75.0) * TEXT_SCALE);
            }
            has_chars = true;
            max_seg_size = max_seg_size.max(layer.font_size);
        }

        if max_seg_size < 0.001 {
            let consumed_idx = measure_consumed.iter().rposition(|value| *value > 0);
            let active = consumed_idx
                .and_then(|idx| segments.get(idx))
                .or_else(|| segments.first());
            max_seg_size = active
                .map(|seg| resolve_segment_font_size(&seg.size, layer.font_size))
                .unwrap_or(layer.font_size);
        }
        if let Some(cspace) = prev_cspace {
            w_scaled += cspace;
        }
        line_widths.push(w_scaled);
        rect_widths.push(if has_chars {
            max_cpv_width_tmp / TEXT_SCALE
        } else {
            0.0
        });
        line_max_sizes.push(max_seg_size);
        line_advances_tmp.push(if line_text.chars().any(|ch| !ch.is_whitespace()) {
            current_line_advances_tmp
        } else {
            Vec::new()
        });
        let _ = caret_xadv_tmp;
    }

    let mut line_asc = Vec::new();
    let mut line_des = Vec::new();
    for (i, max_size) in line_max_sizes.iter().copied().enumerate() {
        let scale = (max_size / TMP_POINT_SIZE) * TEXT_SCALE;
        let asc = scale * ASCENT_LINE;
        let des = scale * DESCENT_LINE;
        if i == 0 || max_size > 0.001 {
            line_asc.push(asc);
            line_des.push(des);
        } else {
            line_asc.push(*line_asc.last().unwrap_or(&asc));
            line_des.push(*line_des.last().unwrap_or(&des));
        }
    }

    let mut line_offsets = vec![0.0; line_max_sizes.len()];
    let lh_override = segments.iter().find_map(|seg| seg.line_height);
    let ls_tmp = layer.line_spacing * layer.font_size * TEXT_SCALE / TMP_POINT_SIZE;
    for i in 1..line_offsets.len() {
        let delta = if let Some(lh) = lh_override {
            lh + ls_tmp
        } else {
            line_asc[i]
                + line_des[i - 1].abs()
                + LINE_GAP * ((layer.font_size / TMP_POINT_SIZE) * TEXT_SCALE)
                + ls_tmp
        };
        line_offsets[i] = line_offsets[i - 1] + delta;
    }

    let logical_max_asc = line_asc.first().copied().unwrap_or(0.0);
    let logical_min_des =
        line_des.last().copied().unwrap_or(0.0) - line_offsets.last().copied().unwrap_or(0.0);
    let effective_max_asc = if lh_override.is_none() && vbounds_max_top.is_finite() {
        logical_max_asc.max(vbounds_max_top)
    } else {
        logical_max_asc
    };
    let effective_min_des = if lh_override.is_none() && vbounds_min_bottom.is_finite() {
        logical_min_des.min(vbounds_min_bottom)
    } else {
        logical_min_des
    };
    let anchor_base = (effective_max_asc + effective_min_des) / (2.0 * TEXT_SCALE);
    let max_rw = rect_widths.iter().copied().fold(0.0, f32::max);
    let box_w = max_rw + PAD_ORIGINAL;
    let layout_metrics = LayoutMetrics {
        line_widths,
        rect_widths,
        box_w,
        anchor_base,
        line_offsets,
    };

    let mut render_consumed = vec![0usize; segments.len()];
    let mut instances = Vec::new();
    let mut plain_text_index = 0usize;
    for (i, line_text) in line_texts.iter().enumerate() {
        if i > 0 {
            plain_text_index += 1;
        }
        let sw = *layout_metrics.line_widths.get(i).unwrap_or(&0.0);
        let line_align = line_segs
            .get(i)
            .and_then(|list| list.first())
            .and_then(|seg| seg.align.clone())
            .or_else(|| global.align.clone());
        let effective_align = match line_align.as_deref() {
            Some("center") => 2,
            Some("right") => 4,
            _ => layer.text_type & 0x07,
        };
        let lx = if effective_align == 2 {
            -sw / 2.0
        } else if effective_align == 4 {
            layout_metrics.box_w / 2.0 - sw
        } else {
            -layout_metrics.box_w / 2.0
        };
        let ly = layout_metrics.anchor_base
            + layout_metrics.line_offsets.get(i).copied().unwrap_or(0.0) / TEXT_SCALE;
        let mut cursor_x = apply_line_indent(
            lx,
            sw,
            layout_metrics.box_w,
            effective_align,
            line_segs.get(i).and_then(|list| list.first()),
        );
        let mut remaining = line_text.chars().collect::<Vec<_>>();
        let mut current_position: Option<Indent> = None;

        for (si, seg) in segments.iter().enumerate() {
            if remaining.is_empty() {
                break;
            }
            let sc = &seg_cleans[si];
            if sc.is_empty() || render_consumed[si] >= sc.len() {
                continue;
            }
            let seg_rest = &sc[render_consumed[si]..];
            let part: Vec<char>;
            if starts_with_chars(&remaining, seg_rest) {
                part = seg_rest.to_vec();
                remaining.drain(..seg_rest.len());
                render_consumed[si] = sc.len();
            } else if starts_with_chars(seg_rest, &remaining) {
                part = remaining.clone();
                render_consumed[si] += remaining.len();
                remaining.clear();
            } else {
                continue;
            }
            if part.is_empty() {
                continue;
            }

            let seg_size = resolve_segment_font_size(&seg.size, layer.font_size);
            let seg_scale = seg.scale.unwrap_or(1.0);
            if seg.position != current_position {
                if let Some(pos_shift) =
                    resolve_indent_value(&seg.position, seg_size, layout_metrics.box_w)
                {
                    cursor_x = lx + pos_shift;
                }
                current_position = seg.position.clone();
            }
            let render_size = if seg.subscript || seg.superscript {
                seg_size * 0.5
            } else {
                seg_size
            };
            let mut baseline_shift = if seg.subscript {
                0.15 * seg_size / TEXT_SCALE
            } else if seg.superscript {
                -0.35 * seg_size / TEXT_SCALE
            } else {
                0.0
            };
            if let Some(voffset) = seg.voffset {
                baseline_shift = -voffset / TEXT_SCALE;
            }
            let cspace_px = seg.cspace.unwrap_or(0.0) / TEXT_SCALE;
            let fill = resolve_fill(layer, seg, &global);
            let outline = layer.outline_color;

            for raw_ch in part {
                for (rendered_ch, char_scale) in transformed_glyphs(raw_ch, seg) {
                    let text = rendered_ch.to_string();
                    let glyph = if force_fallback_glyph(rendered_ch) {
                        None
                    } else {
                        glyph_for(layer, glyphs, &text)
                    };
                    let effective_scale = seg_scale * char_scale;
                    let ft_scale = render_size / atlas.base_size;
                    let fallback_adv =
                        fallback_advance(rendered_ch, render_size, &layer.font_family);
                    let pivot_x = glyph
                        .map(|glyph| {
                            glyph.plane_bearing_x * ft_scale + glyph.plane_width * ft_scale / 2.0
                        })
                        .unwrap_or(fallback_adv / 2.0);
                    let pivot_y = glyph
                        .map(|glyph| {
                            -(glyph.plane_bearing_y * ft_scale
                                - glyph.plane_height * ft_scale / 2.0)
                        })
                        .unwrap_or(0.0);
                    let shear_cx = if let (true, Some(glyph)) = (seg.italic, glyph) {
                        0.35 * (glyph.plane_bearing_y - glyph.plane_height - atlas.spread)
                            * ft_scale
                    } else {
                        0.0
                    };
                    let mono_cell =
                        resolve_indent_value(&seg.monospace, seg_size, layout_metrics.box_w);
                    let draw_x = if let Some(cell) = mono_cell {
                        if cell > 0.0 {
                            cursor_x
                                + if seg.duospace && matches_duo(rendered_ch) {
                                    cell / 4.0
                                } else {
                                    cell / 2.0
                                }
                                - pivot_x
                        } else {
                            cursor_x
                        }
                    } else {
                        cursor_x
                    };
                    let op = GlyphOp {
                        x: draw_x,
                        y: ly + baseline_shift,
                        pivot_x,
                        pivot_y,
                        shear_cx,
                        scale_x: effective_scale,
                        skew_x: if seg.italic { -0.21 } else { 0.0 },
                        rotate_deg: seg.rotate.unwrap_or(0.0),
                    };
                    instances.push(make_instance(
                        layer,
                        plain_text_index,
                        atlas,
                        glyph,
                        &text,
                        op,
                        fill,
                        outline,
                        layout_metrics.clone(),
                        render_size,
                        compute_sdf_shader_params(
                            render_size,
                            seg.bold,
                            layer.outline_width,
                            effective_vertex_alpha(seg.alpha.or(global.alpha), layer.color[3]),
                        ),
                    ));
                    if let Some(cell) = mono_cell {
                        if cell > 0.0 {
                            cursor_x += if seg.duospace && matches_duo(rendered_ch) {
                                cell / 2.0
                            } else {
                                cell
                            } + cspace_px;
                        } else {
                            cursor_x +=
                                (glyph.map(|g| g.advance * ft_scale).unwrap_or(fallback_adv))
                                    * effective_scale
                                    + cspace_px;
                        }
                    } else {
                        cursor_x += (glyph.map(|g| g.advance * ft_scale).unwrap_or(fallback_adv))
                            * effective_scale
                            + cspace_px;
                    }
                }
                plain_text_index += 1;
            }
        }
    }

    let base_matrix = layer_base_matrix(layer);
    let (dynamic_rotation_deg, dynamic_scale_x) = if layer.transform_matrix.is_some() {
        (
            base_matrix[1].atan2(base_matrix[0]).to_degrees(),
            base_matrix[0].hypot(base_matrix[1]),
        )
    } else {
        (layer.rotation_deg, layer.scale_x)
    };
    let dynamic_program = dynamic_percent.map(|percent| DynamicProgramDescriptor {
        layer_id: layer
            .dynamic_layer_id
            .clone()
            .unwrap_or_else(|| layer.id.clone()),
        percent,
        line_advances_tmp,
        rotation_deg: dynamic_rotation_deg,
        scale_x: dynamic_scale_x,
    });
    (instances, dynamic_program)
}

fn make_instance(
    layer: &TextLayer,
    plain_text_index: usize,
    atlas: &AtlasInput,
    glyph: Option<&GlyphInfo>,
    ch: &str,
    op: GlyphOp,
    fill: [f32; 4],
    outline: [f32; 4],
    layout_metrics: LayoutMetrics,
    render_size: f32,
    shader_params: SdfShaderParams,
) -> GlyphInstance {
    let glyph_scale = render_size / atlas.base_size;
    let mut local = Vec::<[f32; 4]>::new();
    if let Some(glyph) = glyph {
        if glyph.drawable {
            let local_left = (glyph.plane_bearing_x - atlas.spread) * glyph_scale - op.pivot_x;
            let local_top = -(glyph.plane_bearing_y + atlas.spread) * glyph_scale - op.pivot_y;
            let local_right = (glyph.plane_bearing_x + glyph.plane_width + atlas.spread)
                * glyph_scale
                - op.pivot_x;
            let local_bottom = (-glyph.plane_bearing_y + glyph.plane_height + atlas.spread)
                * glyph_scale
                - op.pivot_y;
            local.push([local_left, local_top, glyph.u0, glyph.v0]);
            local.push([local_right, local_top, glyph.u1, glyph.v0]);
            local.push([local_right, local_bottom, glyph.u1, glyph.v1]);
            local.push([local_left, local_bottom, glyph.u0, glyph.v1]);
        }
    }

    let glyph_matrix = multiply(
        translate(op.x + op.pivot_x + op.shear_cx, op.y + op.pivot_y),
        multiply(
            rotate(-op.rotate_deg),
            multiply(scale(op.scale_x, 1.0), skew(op.skew_x)),
        ),
    );
    let text_scale = scale(TEXT_SCALE, TEXT_SCALE);
    let outer = layer_base_matrix(layer);
    let matrix = multiply(outer, multiply(text_scale, glyph_matrix));
    let quad = local
        .iter()
        .map(|[x, y, u, v]| {
            let [px, py] = apply(matrix, *x, *y);
            [px, py, *u, *v, atlas.spread * glyph_scale]
        })
        .collect::<Vec<_>>();

    let hx = op.pivot_x.abs().max(1.0);
    let hy = op.pivot_y.abs().max(1.0);
    let footprint_local = [[-hx, -hy], [hx, -hy], [hx, hy], [-hx, hy]];
    let char_quad = footprint_local
        .iter()
        .map(|[x, y]| {
            let [px, py] = apply(glyph_matrix, *x, *y);
            [px * TEXT_SCALE, -py * TEXT_SCALE]
        })
        .collect::<Vec<_>>();
    let device_char_quad = footprint_local
        .iter()
        .map(|[x, y]| apply(matrix, *x, *y))
        .collect::<Vec<_>>();
    let [device_cx, device_cy] = apply(
        outer,
        (op.x + op.pivot_x + op.shear_cx) * TEXT_SCALE,
        (op.y + op.pivot_y) * TEXT_SCALE,
    );

    GlyphInstance {
        layer_id: layer.id.clone(),
        plain_text_index,
        char_value: ch.to_string(),
        drawable: glyph.is_some(),
        glyph_key: glyph.map(|glyph| glyph.key.clone()).unwrap_or_default(),
        atlas_page: glyph.map(|glyph| glyph.page).unwrap_or(0),
        z: layer.z,
        quad: quad.clone(),
        char_position: (
            ch.to_string(),
            (op.x + op.pivot_x + op.shear_cx) * TEXT_SCALE,
            -(op.y + op.pivot_y) * TEXT_SCALE,
            op.scale_x,
            op.skew_x,
            op.pivot_x,
        ),
        char_op: (
            ch.to_string(),
            op.x,
            op.y,
            op.scale_x,
            op.pivot_x,
            op.pivot_y,
            op.rotate_deg,
        ),
        char_quad: (ch.to_string(), char_quad),
        device_char_position: (ch.to_string(), device_cx, device_cy),
        device_char_quad: (ch.to_string(), device_char_quad),
        device_glyph_quad: (
            ch.to_string(),
            quad.iter().map(|row| [row[0], row[1]]).collect(),
        ),
        layout_metrics,
        fill,
        outline: if layer.outline_width > 0.0 {
            outline
        } else {
            [0.0, 0.0, 0.0, 0.0]
        },
        outline_width: layer.outline_width,
        shader_font_size: render_size,
        shader_face_scale: shader_params.face_scale,
        shader_face_bias: shader_params.face_bias,
        shader_underlay_scale: shader_params.underlay_scale,
        shader_underlay_bias: shader_params.underlay_bias,
        shader_vertex_alpha: shader_params.vertex_alpha,
    }
}

fn compute_sdf_shader_params(
    point_size: f32,
    is_bold: bool,
    outline_size: f32,
    vertex_alpha: f32,
) -> SdfShaderParams {
    let uv2_y = runtime_uv2_y(point_size, is_bold);
    let shader_scale = compute_shader_scale(uv2_y);
    let ratio_weight_dilate = WEIGHT_NORMAL.max(WEIGHT_BOLD) * 0.25;
    let selected_weight_dilate = if uv2_y <= 0.0 {
        WEIGHT_BOLD
    } else {
        WEIGHT_NORMAL
    } * 0.25;
    let ratio_face_dilate = FACE_DILATE + ratio_weight_dilate;
    let selected_face_dilate = FACE_DILATE + selected_weight_dilate;
    let face_denom = (OUTLINE_SOFTNESS + OUTLINE_WIDTH + ratio_face_dilate).max(1.0);
    let scale_ratio_a =
        ((GRADIENT_SCALE - TMP_SHADER_CLAMP) / (GRADIENT_SCALE * face_denom)).max(0.0);
    let face_softness = OUTLINE_SOFTNESS * scale_ratio_a;
    let face_scale = shader_scale / (1.0 + face_softness * shader_scale);
    let face_base = 0.5 - selected_face_dilate * scale_ratio_a * 0.5;
    let face_bias = face_base * face_scale - 0.5;

    let underlay_softness = UNDERLAY_SOFTNESS * RUNTIME_SCALE_RATIO_C;
    let underlay_scale = shader_scale / (1.0 + underlay_softness * shader_scale);
    let underlay_bias = face_base * underlay_scale
        - 0.5
        - (outline_size.max(0.0) * RUNTIME_SCALE_RATIO_C) * underlay_scale * 0.5;

    SdfShaderParams {
        face_scale,
        face_bias,
        underlay_scale,
        underlay_bias,
        vertex_alpha,
    }
}

fn runtime_uv2_y(point_size: f32, is_bold: bool) -> f32 {
    let mag = (point_size.abs() / 20250.0).max(1e-8);
    if is_bold {
        -mag
    } else {
        mag
    }
}

fn compute_shader_scale(uv2_y: f32) -> f32 {
    let shader_scale = uv2_y.abs() * RUNTIME_PIXEL_SCALE * GRADIENT_SCALE * (SHARPNESS + 1.0);
    if shader_scale.is_finite() && shader_scale > 0.0001 {
        shader_scale
    } else {
        0.0001
    }
}

fn effective_vertex_alpha(alpha_override: Option<f32>, base_alpha: f32) -> f32 {
    let base_u8 = (base_alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    let alpha_u8 = alpha_override
        .map(|alpha| (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
        .map(|alpha| alpha.min(base_u8))
        .unwrap_or(base_u8);
    alpha_u8 as f32 / 255.0
}

fn glyph_for<'a>(layer: &TextLayer, glyphs: &'a [GlyphInfo], text: &str) -> Option<&'a GlyphInfo> {
    let key = format!(
        "{}\u{0}{}\u{0}{}\u{0}{}",
        layer.region, layer.font_source_hash, layer.font_family, text
    );
    glyphs.iter().find(|glyph| glyph.key == key)
}

fn glyph_advance(
    glyph: Option<&GlyphInfo>,
    ch: char,
    font_size: f32,
    base_size: f32,
    family: &str,
) -> f32 {
    glyph
        .map(|glyph| glyph.advance * (font_size / base_size))
        .unwrap_or_else(|| fallback_advance(ch, font_size, family))
}

fn fallback_advance(ch: char, font_size: f32, family: &str) -> f32 {
    if ch == '\u{00a0}' {
        return font_size;
    }
    if ch == ' ' {
        return (font_size * space_advance_ratio(family)).round();
    }
    if is_fullwidth(ch) {
        font_size
    } else {
        font_size * 0.5
    }
}

fn force_fallback_glyph(ch: char) -> bool {
    ch == ' ' || ch == '\u{00a0}'
}

fn space_advance_ratio(family: &str) -> f32 {
    if family.contains("FZShaoEr") {
        return 0.25;
    }
    if family.contains("FZZhengHei") || family.contains("SkipPro") {
        4.0 / 15.0
    } else {
        5.0 / 24.0
    }
}

fn is_fullwidth(ch: char) -> bool {
    let cp = ch as u32;
    matches!(
        cp,
        0x2000..=0x206f
            | 0x2190..=0x21ff
            | 0x2200..=0x22ff
            | 0x2300..=0x23ff
            | 0x2460..=0x24ff
            | 0x2500..=0x259f
            | 0x25a0..=0x25ff
            | 0x2600..=0x26ff
            | 0x2700..=0x27bf
            | 0x3000..=0x30ff
            | 0x3400..=0x4dbf
            | 0x4e00..=0x9fff
            | 0xf900..=0xfaff
            | 0xfe30..=0xfe4f
            | 0xff01..=0xff60
    )
}

fn parse_rich_segments(raw: &str) -> Vec<TextSegment> {
    let mut state = ParseState::default();
    let chars = raw.chars().collect::<Vec<_>>();
    let mut i = 0usize;
    while i < chars.len() {
        if state.noparse_depth > 0 {
            if chars[i] == '<' {
                if let Some(end) = find_gt(&chars, i) {
                    let tag = chars[i + 1..end].iter().collect::<String>().to_lowercase();
                    if tag == "/noparse" && handle_tag(&mut state, &tag) {
                        i = end + 1;
                        continue;
                    }
                }
            }
            append_char(&mut state, chars[i]);
            i += 1;
            continue;
        }
        if chars[i] == '<' {
            if let Some(end) = find_gt(&chars, i) {
                let tag = chars[i + 1..end].iter().collect::<String>().to_lowercase();
                if handle_tag(&mut state, &tag) {
                    i = end + 1;
                    continue;
                }
            }
        }
        append_char(&mut state, chars[i]);
        i += 1;
    }
    state.segs
}

fn find_gt(chars: &[char], start: usize) -> Option<usize> {
    chars
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(idx, ch)| (*ch == '>').then_some(idx))
}

#[derive(Default)]
struct ParseState {
    segs: Vec<TextSegment>,
    color_stack: Vec<ColorFrame>,
    current_color: Option<[f32; 3]>,
    size_stack: Vec<SizeSpec>,
    scale_stack: Vec<f32>,
    alpha_override: Option<f32>,
    bold_depth: i32,
    italic_depth: i32,
    underline_depth: i32,
    strikethrough_depth: i32,
    subscript_depth: i32,
    superscript_depth: i32,
    mark_stack: Vec<[f32; 4]>,
    case_stack: Vec<CaseTransform>,
    smallcaps_depth: i32,
    noparse_depth: i32,
    voffset_stack: Vec<f32>,
    rotate_stack: Vec<f32>,
    cspace_override: Option<f32>,
    line_height_override: Option<f32>,
    line_indent_override: Option<LineIndent>,
    indent_stack: Vec<Indent>,
    position_stack: Vec<Indent>,
    monospace_stack: Vec<Indent>,
    duospace_stack: Vec<bool>,
    align_stack: Vec<String>,
}

#[derive(Clone)]
struct ColorFrame {
    color: Option<[f32; 3]>,
    alpha: Option<f32>,
}

fn build_segment(state: &ParseState, text: String, fixed_advance: Option<f32>) -> TextSegment {
    TextSegment {
        text,
        fixed_advance,
        color: state.current_color,
        size: state.size_stack.last().cloned(),
        scale: state.scale_stack.last().copied(),
        alpha: state.alpha_override,
        bold: state.bold_depth > 0,
        italic: state.italic_depth > 0,
        underline: state.underline_depth > 0,
        strikethrough: state.strikethrough_depth > 0,
        mark_color: state.mark_stack.last().copied(),
        superscript: state.superscript_depth > 0,
        subscript: state.subscript_depth > 0,
        case_transform: state
            .case_stack
            .last()
            .cloned()
            .unwrap_or(CaseTransform::None),
        smallcaps: state.smallcaps_depth > 0,
        voffset: state.voffset_stack.last().copied(),
        rotate: state.rotate_stack.last().copied(),
        cspace: state.cspace_override,
        line_height: state.line_height_override,
        line_indent: state.line_indent_override.clone(),
        indent: state.indent_stack.last().cloned(),
        position: state.position_stack.last().cloned(),
        monospace: state.monospace_stack.last().cloned(),
        duospace: state.duospace_stack.last().copied().unwrap_or(false),
        align: state.align_stack.last().cloned(),
    }
}

fn append_char(state: &mut ParseState, ch: char) {
    let seg = build_segment(state, ch.to_string(), None);
    if let Some(last) = state.segs.last_mut() {
        if can_merge(last, &seg) {
            last.text.push(ch);
            return;
        }
    }
    state.segs.push(seg);
}

fn push_text(state: &mut ParseState, text: &str) {
    state
        .segs
        .push(build_segment(state, text.to_string(), None));
}

fn handle_tag(state: &mut ParseState, tag: &str) -> bool {
    if let Some(rest) = tag.strip_prefix("color=#") {
        return push_color(state, rest);
    }
    if let Some(rest) = tag.strip_prefix('#') {
        return push_color(state, rest);
    }
    if tag == "/color" {
        if state.color_stack.len() > 1 {
            state.color_stack.pop();
        }
        let prev = state.color_stack.last().cloned().unwrap_or(ColorFrame {
            color: None,
            alpha: None,
        });
        state.current_color = prev.color;
        state.alpha_override = prev.alpha;
        return true;
    }
    if let Some(rest) = tag.strip_prefix("size=") {
        if let Some(parsed) = parse_size(rest) {
            state.size_stack.push(parsed);
        }
        return true;
    }
    if tag == "/size" {
        state.size_stack.pop();
        return true;
    }
    if let Some(rest) = tag.strip_prefix("scale=") {
        if let Some(value) = parse_loose(&rest.replace('#', "")) {
            state.scale_stack.push(value);
        }
        return true;
    }
    if tag == "/scale" {
        state.scale_stack.pop();
        return true;
    }
    if tag == "br" || tag == "cr" {
        push_text(state, "\n");
        return true;
    }
    if tag == "nbsp" {
        append_char(state, '\u{00a0}');
        return true;
    }
    if let Some(rest) = tag.strip_prefix("space=") {
        if let Some(value) = parse_loose(&rest.replace('#', "")) {
            state
                .segs
                .push(build_segment(state, String::new(), Some(value)));
        }
        return true;
    }
    if let Some(rest) = tag.strip_prefix("alpha=") {
        let hex = rest.replace('#', "");
        let slice = &hex[..hex.len().min(2)];
        state.alpha_override = u8::from_str_radix(if slice.is_empty() { "ff" } else { slice }, 16)
            .ok()
            .map(|value| value as f32 / 255.0);
        return true;
    }
    if let Some(rest) = tag.strip_prefix("voffset=") {
        return push_number(&mut state.voffset_stack, rest);
    }
    if tag == "/voffset" {
        state.voffset_stack.pop();
        return true;
    }
    if let Some(rest) = tag.strip_prefix("rotate=") {
        return push_number(&mut state.rotate_stack, rest);
    }
    if tag == "/rotate" {
        state.rotate_stack.pop();
        return true;
    }
    if let Some(rest) = tag.strip_prefix("cspace=") {
        state.cspace_override = parse_loose(&rest.replace('#', ""));
        return true;
    }
    if tag == "/cspace" {
        state.cspace_override = None;
        return true;
    }
    if let Some(rest) = tag.strip_prefix("line-height=") {
        state.line_height_override = parse_loose(&rest.replace('#', ""));
        return true;
    }
    if tag == "/line-height" {
        state.line_height_override = None;
        return true;
    }
    if let Some(rest) = tag.strip_prefix("line-indent=") {
        state.line_indent_override = parse_line_indent(rest);
        return true;
    }
    if tag == "/line-indent" {
        state.line_indent_override = None;
        return true;
    }
    if let Some(rest) = tag.strip_prefix("indent=") {
        return push_indent(&mut state.indent_stack, rest);
    }
    if tag == "/indent" {
        state.indent_stack.pop();
        return true;
    }
    if let Some(rest) = tag.strip_prefix("pos=") {
        return push_indent(&mut state.position_stack, rest);
    }
    if tag == "/pos" {
        state.position_stack.pop();
        return true;
    }
    if let Some(rest) = tag.strip_prefix("mspace=") {
        let mut parts = rest.split_whitespace();
        if let Some(first) = parts.next() {
            if let Some(parsed) = parse_indent(first) {
                state.monospace_stack.push(parsed);
                state
                    .duospace_stack
                    .push(parts.any(|part| part.replace('#', "") == "duospace=1"));
            }
        }
        return true;
    }
    if tag == "/mspace" {
        state.monospace_stack.pop();
        state.duospace_stack.pop();
        return true;
    }
    if let Some(rest) = tag.strip_prefix("align=") {
        if rest == "left" || rest == "center" || rest == "right" {
            state.align_stack.push(rest.to_string());
        }
        return true;
    }
    if tag == "/align" {
        state.align_stack.pop();
        return true;
    }
    match tag {
        "b" => state.bold_depth += 1,
        "/b" => state.bold_depth = (state.bold_depth - 1).max(0),
        "i" => state.italic_depth += 1,
        "/i" => state.italic_depth = (state.italic_depth - 1).max(0),
        "u" => state.underline_depth += 1,
        "/u" => state.underline_depth = (state.underline_depth - 1).max(0),
        "s" => state.strikethrough_depth += 1,
        "/s" => state.strikethrough_depth = (state.strikethrough_depth - 1).max(0),
        "sub" => state.subscript_depth += 1,
        "/sub" => state.subscript_depth = (state.subscript_depth - 1).max(0),
        "sup" => state.superscript_depth += 1,
        "/sup" => state.superscript_depth = (state.superscript_depth - 1).max(0),
        "uppercase" | "allcaps" => state.case_stack.push(CaseTransform::Upper),
        "/uppercase" | "/allcaps" => {
            state.case_stack.pop();
        }
        "lowercase" => state.case_stack.push(CaseTransform::Lower),
        "/lowercase" => {
            state.case_stack.pop();
        }
        "smallcaps" => state.smallcaps_depth += 1,
        "/smallcaps" => state.smallcaps_depth = (state.smallcaps_depth - 1).max(0),
        "noparse" => state.noparse_depth += 1,
        "/noparse" => state.noparse_depth = (state.noparse_depth - 1).max(0),
        _ => {}
    }
    true
}

fn can_merge(a: &TextSegment, b: &TextSegment) -> bool {
    let mut aa = a.clone();
    let mut bb = b.clone();
    aa.text.clear();
    bb.text.clear();
    aa == bb
}

fn segments_to_global(segs: &[TextSegment]) -> GlobalStyle {
    let first = segs.first();
    GlobalStyle {
        color: first.and_then(|seg| seg.color),
        alpha: first.and_then(|seg| seg.alpha),
        align: first.and_then(|seg| seg.align.clone()),
        clean: segs.iter().map(|seg| seg.text.as_str()).collect::<String>(),
    }
}

fn resolve_segment_font_size(size: &Option<SizeSpec>, base: f32) -> f32 {
    match size {
        None => base,
        Some(SizeSpec::Absolute(value)) => *value,
        Some(SizeSpec::Delta(value)) => base + *value,
        Some(SizeSpec::Percent(value)) => base * *value / 100.0,
        Some(SizeSpec::Em(value)) => base * *value,
    }
}

fn resolve_indent_value(spec: &Option<Indent>, base_font_size: f32, box_w: f32) -> Option<f32> {
    match spec {
        None => None,
        Some(Indent::Pixels(value)) => Some(*value / TEXT_SCALE),
        Some(Indent::Em(value)) => Some(*value * base_font_size / TEXT_SCALE),
        Some(Indent::Percent(value)) => Some(box_w * *value / 100.0),
    }
}

fn update_cpv_width(current: f32, before: f32, glyph_advance: f32) -> f32 {
    current.max(before.abs() + glyph_advance)
}

fn transform_char(ch: char, seg: &TextSegment) -> (String, f32) {
    if seg.smallcaps
        && ch.to_lowercase().to_string() == ch.to_string()
        && ch.to_uppercase().to_string() != ch.to_string()
    {
        return (ch.to_uppercase().collect::<String>(), 0.8);
    }
    match seg.case_transform {
        CaseTransform::Upper => (ch.to_uppercase().collect::<String>(), 1.0),
        CaseTransform::Lower => (ch.to_lowercase().collect::<String>(), 1.0),
        CaseTransform::None => (ch.to_string(), 1.0),
    }
}

fn transformed_glyphs(ch: char, seg: &TextSegment) -> Vec<(char, f32)> {
    let (text, scale) = transform_char(ch, seg);
    text.chars().map(|value| (value, scale)).collect()
}

fn glyph_demand_chars(raw: &str) -> Vec<char> {
    parse_rich_segments(raw)
        .iter()
        .flat_map(|segment| {
            segment.text.chars().flat_map(|source| {
                if source == '\n' || source == '\r' || force_fallback_glyph(source) {
                    Vec::new()
                } else {
                    transformed_glyphs(source, segment)
                        .into_iter()
                        .map(|(value, _)| value)
                        .collect()
                }
            })
        })
        .collect()
}

fn resolve_fill(layer: &TextLayer, seg: &TextSegment, global: &GlobalStyle) -> [f32; 4] {
    let color = seg.color.or(global.color).unwrap_or(layer.color_rgb);
    [color[0] / 255.0, color[1] / 255.0, color[2] / 255.0, 1.0]
}

fn apply_line_indent(
    lx: f32,
    sw: f32,
    _box_w: f32,
    effective_align: i32,
    seg: Option<&TextSegment>,
) -> f32 {
    let mut cursor_x = lx;
    if let Some(seg) = seg {
        if let Some(indent) = &seg.indent {
            match indent {
                Indent::Percent(value) if *value < 100.0 => {
                    cursor_x = percent_indent_cursor(sw, effective_align, *value);
                }
                Indent::Pixels(value) => cursor_x += *value / TEXT_SCALE,
                Indent::Em(value) => {
                    cursor_x += *value * resolve_segment_font_size(&seg.size, 0.0) / TEXT_SCALE;
                }
                _ => {}
            }
        }
        if let Some(line_indent) = &seg.line_indent {
            match line_indent {
                LineIndent::Percent(value) if *value < 100.0 => {
                    cursor_x = percent_indent_cursor(sw, effective_align, *value);
                }
                LineIndent::Pixels(value) => cursor_x = lx + *value,
                _ => {}
            }
        }
    }
    cursor_x
}

fn percent_indent_cursor(sw: f32, effective_align: i32, percent: f32) -> f32 {
    let pct = percent / 100.0;
    let rect = (sw * TEXT_SCALE + 64.0) / (1.0 - pct);
    let resolved_indent = rect * pct / TEXT_SCALE;
    if effective_align == 2 {
        (resolved_indent - sw) / 2.0
    } else if effective_align == 4 {
        rect / (2.0 * TEXT_SCALE) - sw
    } else {
        rect * (pct - 0.5) / TEXT_SCALE
    }
}

fn layer_base_matrix(layer: &TextLayer) -> Mat {
    layer.transform_matrix.unwrap_or_else(|| {
        multiply(
            translate(layer.x, layer.y),
            multiply(
                rotate(layer.rotation_deg),
                scale(layer.scale_x, layer.scale_y),
            ),
        )
    })
}

fn line_indent_dynamic_percent(raw: &str, segments: &[TextSegment]) -> Option<f32> {
    let _ = raw;
    let mut dynamic_percent = None;
    for segment in segments
        .iter()
        .filter(|segment| segment.text.chars().any(|ch| !ch.is_whitespace()))
    {
        let LineIndent::Percent(value) = segment.line_indent.as_ref()? else {
            return None;
        };
        if !value.is_finite()
            || dynamic_percent.is_some_and(|current: f32| (current - *value).abs() > f32::EPSILON)
        {
            return None;
        }
        dynamic_percent = Some(*value);
    }
    dynamic_percent
}

type Mat = [f32; 6];

fn translate(x: f32, y: f32) -> Mat {
    [1.0, 0.0, 0.0, 1.0, x, y]
}

fn scale(x: f32, y: f32) -> Mat {
    [x, 0.0, 0.0, y, 0.0, 0.0]
}

fn skew(x: f32) -> Mat {
    [1.0, 0.0, x, 1.0, 0.0, 0.0]
}

fn rotate(deg: f32) -> Mat {
    let t = deg.to_radians();
    let c = t.cos();
    let s = t.sin();
    [c, s, -s, c, 0.0, 0.0]
}

fn multiply(a: Mat, b: Mat) -> Mat {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

fn apply(m: Mat, x: f32, y: f32) -> [f32; 2] {
    [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]
}

fn parse_size(raw: &str) -> Option<SizeSpec> {
    let value = raw.replace('#', "");
    if let Some(rest) = value.strip_suffix("em") {
        parse_as(rest, SizeSpec::Em)
    } else if let Some(rest) = value.strip_suffix('%') {
        parse_as(rest, SizeSpec::Percent)
    } else if let Some(rest) = value.strip_prefix('+') {
        parse_as(rest, SizeSpec::Delta)
    } else if let Some(rest) = value.strip_prefix('-') {
        parse_as(rest, |n| SizeSpec::Delta(-n))
    } else {
        parse_as(&value, SizeSpec::Absolute)
    }
}

fn parse_indent(raw: &str) -> Option<Indent> {
    let value = raw.replace('#', "");
    if let Some(rest) = value.strip_suffix('%') {
        parse_as(rest, Indent::Percent)
    } else if let Some(rest) = value.strip_suffix("em") {
        parse_as(rest, Indent::Em)
    } else if let Some(rest) = value.strip_suffix("px") {
        parse_as(rest, Indent::Pixels)
    } else {
        parse_as(&value, Indent::Pixels)
    }
}

fn parse_line_indent(raw: &str) -> Option<LineIndent> {
    if let Some(rest) = raw.strip_suffix('%') {
        parse_as(rest, LineIndent::Percent)
    } else {
        parse_as(raw, LineIndent::Pixels)
    }
}

fn parse_as<T>(raw: &str, map: impl FnOnce(f32) -> T) -> Option<T> {
    parse_loose(raw).map(map)
}

fn parse_loose(raw: &str) -> Option<f32> {
    let trimmed = raw.trim();
    let mut end = 0usize;
    for (idx, ch) in trimmed.char_indices() {
        let ok =
            ch.is_ascii_digit() || ch == '+' || ch == '-' || ch == '.' || ch == 'e' || ch == 'E';
        if ok {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    trimmed[..end]
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
}

fn push_color(state: &mut ParseState, hex: &str) -> bool {
    if let Some(parsed) = parse_hex_color(hex) {
        state.current_color = Some([parsed[0], parsed[1], parsed[2]]);
        state.alpha_override = if parsed[3].is_nan() {
            None
        } else {
            Some(parsed[3] / 255.0)
        };
        state.color_stack.push(ColorFrame {
            color: state.current_color,
            alpha: state.alpha_override,
        });
    }
    true
}

fn parse_hex_color(hex: &str) -> Option<[f32; 4]> {
    let value = hex.replace('#', "");
    if value.len() == 3 || value.len() == 4 {
        let mut nums = value
            .chars()
            .filter_map(|ch| ch.to_digit(16).map(|value| value as f32 * 17.0))
            .collect::<Vec<_>>();
        if nums.len() < 3 {
            return None;
        }
        while nums.len() < 4 {
            nums.push(f32::NAN);
        }
        return nums.try_into().ok();
    }
    if value.len() == 6 || value.len() == 8 {
        let r = u8::from_str_radix(&value[0..2], 16).ok()? as f32;
        let g = u8::from_str_radix(&value[2..4], 16).ok()? as f32;
        let b = u8::from_str_radix(&value[4..6], 16).ok()? as f32;
        let a = if value.len() == 8 {
            u8::from_str_radix(&value[6..8], 16).ok()? as f32
        } else {
            f32::NAN
        };
        return Some([r, g, b, a]);
    }
    None
}

fn push_number(stack: &mut Vec<f32>, raw: &str) -> bool {
    if let Some(value) = parse_loose(&raw.replace('#', "")) {
        stack.push(value);
    }
    true
}

fn push_indent(stack: &mut Vec<Indent>, raw: &str) -> bool {
    if let Some(value) = parse_indent(raw) {
        stack.push(value);
    }
    true
}

fn matches_duo(ch: char) -> bool {
    ch == '.' || ch == ':' || ch == ','
}

fn starts_with_chars(left: &[char], prefix: &[char]) -> bool {
    left.len() >= prefix.len() && left.iter().zip(prefix.iter()).all(|(a, b)| a == b)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LayoutRequest {
    layers: Vec<TextLayer>,
    atlas: AtlasInput,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtlasInput {
    base_size: f32,
    spread: f32,
    glyphs: Vec<GlyphInfo>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlyphInfo {
    key: String,
    #[serde(default)]
    page: u32,
    u0: f32,
    v0: f32,
    u1: f32,
    v1: f32,
    advance: f32,
    plane_bearing_x: f32,
    plane_bearing_y: f32,
    plane_width: f32,
    plane_height: f32,
    drawable: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TextLayer {
    id: String,
    #[serde(default)]
    dynamic_layer_id: Option<String>,
    z: f32,
    text: String,
    region: String,
    font_family: String,
    font_source_hash: String,
    #[serde(default)]
    transform_matrix: Option<Mat>,
    #[serde(default)]
    x: f32,
    #[serde(default)]
    y: f32,
    #[serde(default)]
    rotation_deg: f32,
    #[serde(default = "one")]
    scale_x: f32,
    #[serde(default = "one")]
    scale_y: f32,
    font_size: f32,
    color: [f32; 4],
    outline_color: [f32; 4],
    color_rgb: [f32; 3],
    outline_width: f32,
    line_spacing: f32,
    text_type: i32,
}

fn one() -> f32 {
    1.0
}

#[cfg(test)]
mod tests {
    use super::{
        build_glyph_demand_json, build_layout_json, glyph_demand_chars, parse_rich_segments,
        transformed_glyphs,
    };

    #[test]
    fn glyph_demand_uses_tmp_visible_transformed_scalars() {
        assert_eq!(
            glyph_demand_chars("<uppercase>aß</uppercase> <noparse><b></noparse>"),
            vec!['A', 'S', 'S', '<', 'b', '>'],
        );
    }

    #[test]
    fn glyph_demand_json_is_deduplicated_and_font_scoped() {
        let output: serde_json::Value = serde_json::from_str(
            &build_glyph_demand_json(r#"{"layers":[{"text":"<b>12</b>","region":"en","fontFamily":"Inter","fontSourceHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},{"text":"2A","region":"en","fontFamily":"Inter","fontSourceHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#).unwrap(),
        ).unwrap();
        assert_eq!(output["source"], "wasm-tmp-glyph-demand");
        assert_eq!(output["requests"].as_array().unwrap().len(), 3);
        assert_eq!(output["requests"][0]["char"], "1");
        assert_eq!(output["requests"][1]["char"], "2");
        assert_eq!(output["requests"][2]["char"], "A");
    }

    #[test]
    fn case_expansion_is_laid_out_as_scalar_glyphs() {
        let segments = parse_rich_segments("<uppercase>ß</uppercase>");
        let glyphs = transformed_glyphs('ß', &segments[0]);
        assert_eq!(glyphs, vec![('S', 1.0), ('S', 1.0)]);
    }

    #[test]
    fn layout_compiles_line_indent_program_from_tmp_without_ts_metadata() {
        let input = serde_json::json!({
            "layers": [{
                "id": "text-layer", "z": 0, "text": "<line-indent=50%>12</line-indent>",
                "region": "en", "fontFamily": "SyntheticSans", "fontSourceHash": "a".repeat(64),
                "x": 0.0, "y": 0.0, "rotationDeg": 15.0, "scaleX": 2.0, "scaleY": 1.0,
                "fontSize": 24.0, "color": [1.0,1.0,1.0,1.0], "outlineColor": [0.0,0.0,0.0,0.0],
                "colorRgb": [255.0,255.0,255.0], "outlineWidth": 0.0, "lineSpacing": 0.0,
                "textType": 0, "dynamic": null
            }],
            "atlas": { "baseSize": 75.0, "spread": 6.0, "glyphs": [] },
            "tick": 0, "frameMode": "animate"
        });
        let output: serde_json::Value =
            serde_json::from_str(&build_layout_json(&input.to_string()).unwrap()).unwrap();
        assert_eq!(output["dynamicPrograms"][0]["percent"], 50.0);
        assert_eq!(output["dynamicPrograms"][0]["rotationDeg"], 15.0);
        assert_eq!(output["dynamicPrograms"][0]["scaleX"], 2.0);
        let layout_metrics = &output["instances"][0]["layoutMetrics"];
        assert!(layout_metrics["lineWidths"].is_array());
        assert!(layout_metrics["rectWidths"].is_array());
        assert!(layout_metrics["boxW"].is_number());
        assert!(layout_metrics["anchorBase"].is_number());
        assert!(layout_metrics["lineOffsets"].is_array());
        assert!(layout_metrics.get("line_widths").is_none());
        assert_eq!(
            output["dynamicPrograms"][0]["lineAdvancesTmp"][0]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn layout_compiles_one_global_line_indent_program_across_hard_breaks() {
        let input = serde_json::json!({
            "layers": [{
                "id": "text-layer", "z": 0,
                "text": " <line-indent=50%>A\nBB",
                "region": "en", "fontFamily": "SyntheticSans", "fontSourceHash": "a".repeat(64),
                "x": 0.0, "y": 0.0, "rotationDeg": 0.0, "scaleX": 1.0, "scaleY": 1.0,
                "fontSize": 24.0, "color": [1.0,1.0,1.0,1.0], "outlineColor": [0.0,0.0,0.0,0.0],
                "colorRgb": [255.0,255.0,255.0], "outlineWidth": 0.0, "lineSpacing": 0.0,
                "textType": 0, "dynamic": null
            }],
            "atlas": { "baseSize": 75.0, "spread": 6.0, "glyphs": [] },
            "tick": 0, "frameMode": "animate"
        });
        let output: serde_json::Value =
            serde_json::from_str(&build_layout_json(&input.to_string()).unwrap()).unwrap();

        let lines = output["dynamicPrograms"][0]["lineAdvancesTmp"]
            .as_array()
            .unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].as_array().unwrap().len(), 2);
        assert_eq!(lines[1].as_array().unwrap().len(), 2);
    }

    #[test]
    fn local_layout_is_independent_of_frame_mode() {
        let request = |frame_mode: &str| {
            serde_json::json!({
                "layers": [{
                    "id": "text-layer", "z": 0, "text": "<line-indent=96%>12</line-indent>",
                    "region": "en", "fontFamily": "SyntheticSans", "fontSourceHash": "a".repeat(64),
                    "x": 200.0, "y": 100.0, "rotationDeg": 0.0, "scaleX": 1.0, "scaleY": 1.0,
                    "fontSize": 24.0, "color": [1.0,1.0,1.0,1.0], "outlineColor": [0.0,0.0,0.0,0.0],
                    "colorRgb": [255.0,255.0,255.0], "outlineWidth": 0.0, "lineSpacing": 0.0,
                    "textType": 0
                }],
                "atlas": { "baseSize": 75.0, "spread": 6.0, "glyphs": [] },
                "tick": 0, "frameMode": frame_mode
            })
        };
        let animate: serde_json::Value =
            serde_json::from_str(&build_layout_json(&request("animate").to_string()).unwrap())
                .unwrap();
        let final_layout: serde_json::Value =
            serde_json::from_str(&build_layout_json(&request("final").to_string()).unwrap())
                .unwrap();

        assert_eq!(animate["instances"], final_layout["instances"]);
        assert_eq!(animate["dynamicPrograms"], final_layout["dynamicPrograms"]);
    }

    #[test]
    fn layout_preserves_a_renderer_owned_reflection_matrix() {
        let input = serde_json::json!({
            "layers": [{
                "id": "reflected-text", "z": 0, "text": "A",
                "region": "en", "fontFamily": "SyntheticSans", "fontSourceHash": "a".repeat(64),
                "transformMatrix": [-1.0, 0.0, 0.0, 1.0, 100.0, 20.0],
                "x": 0.0, "y": 0.0, "rotationDeg": 0.0, "scaleX": 1.0, "scaleY": 1.0,
                "fontSize": 24.0, "color": [1.0,1.0,1.0,1.0], "outlineColor": [0.0,0.0,0.0,0.0],
                "colorRgb": [255.0,255.0,255.0], "outlineWidth": 0.0, "lineSpacing": 0.0,
                "textType": 0
            }],
            "atlas": { "baseSize": 75.0, "spread": 6.0, "glyphs": [] },
            "tick": 0, "frameMode": "animate"
        });
        let output: serde_json::Value =
            serde_json::from_str(&build_layout_json(&input.to_string()).unwrap()).unwrap();
        let quad = output["instances"][0]["deviceCharQuad"][1]
            .as_array()
            .unwrap();
        let point = |index: usize| {
            let value = quad[index].as_array().unwrap();
            [value[0].as_f64().unwrap(), value[1].as_f64().unwrap()]
        };
        let p0 = point(0);
        let p1 = point(1);
        let p2 = point(2);
        let winding = (p1[0] - p0[0]) * (p2[1] - p1[1]) - (p1[1] - p0[1]) * (p2[0] - p1[0]);
        assert!(
            winding < 0.0,
            "reflection must preserve negative winding: {quad:?}"
        );
    }
}

#[derive(Clone, PartialEq)]
enum SizeSpec {
    Absolute(f32),
    Delta(f32),
    Percent(f32),
    Em(f32),
}

#[derive(Clone, PartialEq)]
enum Indent {
    Percent(f32),
    Pixels(f32),
    Em(f32),
}

#[derive(Clone, PartialEq)]
enum LineIndent {
    Percent(f32),
    Pixels(f32),
}

#[derive(Clone, PartialEq)]
enum CaseTransform {
    None,
    Upper,
    Lower,
}

#[derive(Clone, PartialEq)]
struct TextSegment {
    text: String,
    fixed_advance: Option<f32>,
    color: Option<[f32; 3]>,
    size: Option<SizeSpec>,
    scale: Option<f32>,
    alpha: Option<f32>,
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    mark_color: Option<[f32; 4]>,
    superscript: bool,
    subscript: bool,
    case_transform: CaseTransform,
    smallcaps: bool,
    voffset: Option<f32>,
    rotate: Option<f32>,
    cspace: Option<f32>,
    line_height: Option<f32>,
    line_indent: Option<LineIndent>,
    indent: Option<Indent>,
    position: Option<Indent>,
    monospace: Option<Indent>,
    duospace: bool,
    align: Option<String>,
}

struct GlobalStyle {
    color: Option<[f32; 3]>,
    alpha: Option<f32>,
    align: Option<String>,
    clean: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutMetrics {
    line_widths: Vec<f32>,
    rect_widths: Vec<f32>,
    box_w: f32,
    anchor_base: f32,
    line_offsets: Vec<f32>,
}

struct GlyphOp {
    x: f32,
    y: f32,
    pivot_x: f32,
    pivot_y: f32,
    shear_cx: f32,
    scale_x: f32,
    skew_x: f32,
    rotate_deg: f32,
}

#[derive(Clone, Copy)]
struct SdfShaderParams {
    face_scale: f32,
    face_bias: f32,
    underlay_scale: f32,
    underlay_bias: f32,
    vertex_alpha: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LayoutBatch {
    version: i32,
    source: String,
    instances: Vec<GlyphInstance>,
    dynamic_programs: Vec<DynamicProgramDescriptor>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DynamicProgramDescriptor {
    layer_id: String,
    percent: f32,
    line_advances_tmp: Vec<Vec<f32>>,
    rotation_deg: f32,
    scale_x: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GlyphInstance {
    layer_id: String,
    plain_text_index: usize,
    #[serde(rename = "char")]
    char_value: String,
    drawable: bool,
    glyph_key: String,
    atlas_page: u32,
    z: f32,
    quad: Vec<[f32; 5]>,
    char_position: (String, f32, f32, f32, f32, f32),
    char_op: (String, f32, f32, f32, f32, f32, f32),
    char_quad: (String, Vec<[f32; 2]>),
    device_char_position: (String, f32, f32),
    device_char_quad: (String, Vec<[f32; 2]>),
    device_glyph_quad: (String, Vec<[f32; 2]>),
    layout_metrics: LayoutMetrics,
    fill: [f32; 4],
    outline: [f32; 4],
    outline_width: f32,
    shader_font_size: f32,
    shader_face_scale: f32,
    shader_face_bias: f32,
    shader_underlay_scale: f32,
    shader_underlay_bias: f32,
    shader_vertex_alpha: f32,
}
