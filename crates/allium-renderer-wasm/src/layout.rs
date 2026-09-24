use sekai_profile_renderer_core::sdf_material::{tmp_uv2_y, TmpGlyphMaterial};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use sekai_profile_renderer_core::tmp_text::{
    face::line_spacing_offset,
    glyph,
    layout::{self as tmp_layout, offset_draw_units},
    markup::{CaretCommand, InlineAlign, TextSegment},
    parse_segments, split_lines, DEFAULT_LINE_SPACING_FACTOR, PROFILE_FACE,
};

const TEXT_SCALE: f32 = PROFILE_FACE.scale;
const PAD_ORIGINAL: f32 = tmp_layout::CONTAINER_PADDING / TEXT_SCALE;

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
    let face = PROFILE_FACE;
    let segments = parse_segments(&layer.text, layer.font_size);
    let dynamic_percent = tmp_layout::uniform_line_indent_percent(&segments);
    let lines = split_lines(&segments);
    // Font size of each line feed: a line without characters takes its
    // metrics from the line feed that ends it.
    let line_feed_sizes: Vec<f32> = segments
        .iter()
        .flat_map(|segment| {
            segment
                .text
                .chars()
                .filter(|ch| *ch == '\n')
                .map(|_| segment.font_size)
        })
        .collect();
    let first_size = segments
        .first()
        .map_or(layer.font_size, |segment| segment.font_size);

    let mut line_widths = Vec::with_capacity(lines.len());
    let mut rect_widths = Vec::with_capacity(lines.len());
    let mut line_max_sizes = Vec::with_capacity(lines.len());
    let mut vbounds_max_top = f32::NEG_INFINITY;
    let mut vbounds_min_bottom = f32::INFINITY;
    let mut line_advances_tmp = Vec::with_capacity(lines.len());

    for (line_index, line) in lines.iter().enumerate() {
        let mut current_line_advances_tmp = Vec::new();
        let mut pending_advance_tmp = 0.0f32;
        let mut w_scaled = 0.0;
        let mut max_seg_size: f32 = 0.0;
        let mut prev_cspace = None;
        let mut cpv_xadv_tmp = 0.0;
        let mut max_cpv_width_tmp = 0.0;
        let mut has_chars = false;
        let mut has_visible = false;

        for piece in line {
            let seg = piece.segment;
            match seg.caret {
                Some(CaretCommand::Advance(advance)) => {
                    w_scaled += advance / TEXT_SCALE;
                    cpv_xadv_tmp += advance;
                    pending_advance_tmp += advance;
                    if advance >= 0.0 {
                        max_cpv_width_tmp = tmp_layout::extend_preferred_width(
                            max_cpv_width_tmp,
                            cpv_xadv_tmp,
                            0.0,
                        );
                        has_chars = true;
                        max_seg_size = max_seg_size.max(seg.font_size);
                    } else {
                        // `</cspace>` already took the trailing spacing back.
                        prev_cspace = None;
                    }
                    continue;
                }
                Some(CaretCommand::MoveTo(offset)) => {
                    cpv_xadv_tmp = offset_draw_units(offset, 0.0) * TEXT_SCALE;
                    max_cpv_width_tmp = 0.0;
                    continue;
                }
                None => {}
            }

            let measure_size = seg.render_size();
            let seg_scale = seg.scale.unwrap_or(1.0);
            let cspace_raw_tmp = seg.character_spacing;
            let voffset_tmp = seg.baseline_offset;
            let glyph_asc_tmp = face.font_scale(measure_size) * face.ascent_line;
            let glyph_des_tmp = -face.font_scale(measure_size) * face.descent_line;
            let mut measured = 0.0;
            let mut rendered_count = 0usize;
            for raw_ch in piece.text.chars() {
                let (display, char_scale) = seg.transform_char(raw_ch);
                let glyph_advance_tmp = layer_glyph(layer, glyphs, raw_ch, display).advance(
                    measure_size * char_scale,
                    atlas.base_size,
                    &layer.font_family,
                ) * TEXT_SCALE;
                measured += (glyph_advance_tmp * seg_scale) / TEXT_SCALE;
                rendered_count += 1;
                has_visible |= !raw_ch.is_whitespace();
                max_cpv_width_tmp = tmp_layout::extend_preferred_width_for_char(
                    max_cpv_width_tmp,
                    cpv_xadv_tmp,
                    glyph_advance_tmp,
                    raw_ch,
                );
                // TMP's preferred width ignores `<scale>`, so the line-indent
                // program reads the unscaled advances.
                current_line_advances_tmp
                    .push(glyph_advance_tmp + cspace_raw_tmp + pending_advance_tmp);
                pending_advance_tmp = 0.0;
                vbounds_max_top = vbounds_max_top.max(voffset_tmp + glyph_asc_tmp);
                vbounds_min_bottom = vbounds_min_bottom.min(voffset_tmp - glyph_des_tmp);
                cpv_xadv_tmp += glyph_advance_tmp + cspace_raw_tmp;
            }
            let cspace = cspace_raw_tmp / TEXT_SCALE;
            w_scaled += measured + cspace * rendered_count as f32;
            has_chars = true;
            prev_cspace = Some(cspace);
            max_seg_size = max_seg_size.max(seg.font_size);
        }

        if max_seg_size < 0.001 {
            max_seg_size = line_feed_sizes
                .get(line_index)
                .or_else(|| {
                    line_index
                        .checked_sub(1)
                        .and_then(|prev| line_feed_sizes.get(prev))
                })
                .copied()
                .unwrap_or(first_size);
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
        line_advances_tmp.push(if has_visible {
            current_line_advances_tmp
        } else {
            Vec::new()
        });
    }

    let mut line_asc = Vec::new();
    let mut line_des = Vec::new();
    for (i, max_size) in line_max_sizes.iter().copied().enumerate() {
        let scale = face.font_scale(max_size);
        let asc = scale * face.ascent_line;
        let des = scale * face.descent_line;
        if i == 0 || max_size > 0.001 {
            line_asc.push(asc);
            line_des.push(des);
        } else {
            line_asc.push(*line_asc.last().unwrap_or(&asc));
            line_des.push(*line_des.last().unwrap_or(&des));
        }
    }

    // lineGap = m_LineHeight - (ascentLine - descentLine) + 0.625; the 0.625
    // is an empirical correction matching the game's line pitch.
    let line_gap = face.line_height - (face.ascent_line - face.descent_line) + 0.625;
    let mut line_offsets = vec![0.0; line_max_sizes.len()];
    let lh_override = segments.iter().find_map(|seg| seg.line_height);
    let ls_tmp = line_spacing_offset(
        layer.line_spacing,
        layer.font_size,
        &face,
        DEFAULT_LINE_SPACING_FACTOR,
    );
    for i in 1..line_offsets.len() {
        let delta = if let Some(lh) = lh_override {
            lh + ls_tmp
        } else {
            line_asc[i]
                + line_des[i - 1].abs()
                + line_gap * face.font_scale(layer.font_size)
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

    let default_align = segments.first().and_then(|segment| segment.align);
    let mut instances = Vec::new();
    let mut plain_text_index = 0usize;
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            plain_text_index += 1;
        }
        let sw = *layout_metrics.line_widths.get(i).unwrap_or(&0.0);
        let first_text = line
            .iter()
            .find(|piece| piece.segment.caret.is_none())
            .map(|piece| piece.segment);
        // Justified and flush lines are not stretched; they start at the left.
        let effective_align = match first_text.and_then(|seg| seg.align).or(default_align) {
            Some(InlineAlign::Left | InlineAlign::Justified | InlineAlign::Flush) => 1,
            Some(InlineAlign::Center) => 2,
            Some(InlineAlign::Right) => 4,
            None => layer.text_type & 0x07,
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
        let mut cursor_x = tmp_layout::line_start(lx, sw, max_rw, effective_align, first_text);

        for piece in line {
            let seg = piece.segment;
            match seg.caret {
                Some(CaretCommand::Advance(advance)) => {
                    cursor_x += advance / TEXT_SCALE;
                    continue;
                }
                Some(CaretCommand::MoveTo(offset)) => {
                    cursor_x = lx + offset_draw_units(offset, layout_metrics.box_w);
                    continue;
                }
                None => {}
            }

            let render_size = seg.render_size();
            let seg_scale = seg.scale.unwrap_or(1.0);
            // TMP's baseline offset points up; draw space points down.
            let baseline_shift = -seg.baseline_offset / TEXT_SCALE;
            let cspace_px = seg.character_spacing / TEXT_SCALE;
            let fill = resolve_fill(layer, seg);
            let outline = layer.outline_color;
            let italic_slope = seg.italic.map(|angle| angle as f32 * 0.01);
            let mono_cell = seg.monospace.map(|cell| cell / TEXT_SCALE);

            for raw_ch in piece.text.chars() {
                let (display, char_scale) = seg.transform_char(raw_ch);
                // <smallcaps> scales the whole glyph, not just its width.
                let glyph_size = render_size * char_scale;
                let resolved = layer_glyph(layer, glyphs, raw_ch, display);
                let glyph = resolved.atlas_glyph();
                let advance = resolved.advance(glyph_size, atlas.base_size, &layer.font_family);
                let ft_scale = glyph_size / atlas.base_size;
                let pivot_x = glyph
                    .map(|glyph| {
                        glyph.plane_bearing_x * ft_scale + glyph.plane_width * ft_scale / 2.0
                    })
                    .unwrap_or(advance / 2.0);
                let pivot_y = glyph
                    .map(|glyph| {
                        -(glyph.plane_bearing_y * ft_scale - glyph.plane_height * ft_scale / 2.0)
                    })
                    .unwrap_or(0.0);
                let shear_cx = match (italic_slope, glyph) {
                    (Some(slope), Some(glyph)) => {
                        slope
                            * (glyph.plane_bearing_y - glyph.plane_height - atlas.spread)
                            * ft_scale
                    }
                    _ => 0.0,
                };
                let draw_x = match mono_cell {
                    Some(cell) => cursor_x + cell / 2.0 - pivot_x,
                    None => cursor_x,
                };
                let op = GlyphOp {
                    x: draw_x,
                    y: ly + baseline_shift,
                    pivot_x,
                    pivot_y,
                    shear_cx,
                    scale_x: seg_scale,
                    skew_x: italic_slope.map_or(0.0, |slope| -slope),
                    rotate_deg: seg.rotate.unwrap_or(0.0),
                };
                instances.push(make_instance(
                    layer,
                    plain_text_index,
                    atlas,
                    glyph,
                    &resolved.drawn_char().to_string(),
                    op,
                    fill,
                    outline,
                    layout_metrics.clone(),
                    glyph_size,
                    compute_sdf_shader_params(
                        glyph_size,
                        seg.bold,
                        layer.outline_width,
                        effective_vertex_alpha(seg.alpha, layer.color[3]),
                    ),
                ));
                cursor_x += match mono_cell {
                    Some(cell) => cell,
                    None => advance * seg_scale,
                } + cspace_px;
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
        alignment: (layer.text_type & 0x07) as u8,
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
        // Card text always draws the underlay; a zero width puts it on the
        // face edge rather than removing it.
        outline,
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
    let material = TmpGlyphMaterial::new(tmp_uv2_y(point_size, is_bold), outline_size);
    SdfShaderParams {
        face_scale: material.face_scale,
        face_bias: material.face_bias,
        underlay_scale: material.underlay_scale,
        underlay_bias: material.underlay_bias,
        vertex_alpha,
    }
}

#[cfg(test)]
mod material_tests {
    use super::{build_layout_json, compute_sdf_shader_params};
    use sekai_profile_renderer_core::sdf_material::{tmp_uv2_y, TmpGlyphMaterial};

    #[test]
    fn zero_width_outline_keeps_the_underlay_on_the_face_edge() {
        let input = serde_json::json!({
            "layers": [{
                "id": "text-layer", "z": 0, "text": "A",
                "region": "en", "fontFamily": "SyntheticSans", "fontSourceHash": "a".repeat(64),
                "x": 0.0, "y": 0.0, "fontSize": 24.0, "color": [1.0, 1.0, 1.0, 1.0],
                "outlineColor": [0.25, 0.5, 0.75, 1.0], "colorRgb": [255.0, 255.0, 255.0],
                "outlineWidth": 0.0, "lineSpacing": 0.0, "textType": 0, "dynamic": null
            }],
            "atlas": { "baseSize": 75.0, "spread": 6.0, "glyphs": [] },
            "tick": 0, "frameMode": "animate"
        });
        let output: serde_json::Value =
            serde_json::from_str(&build_layout_json(&input.to_string()).unwrap()).unwrap();
        let instance = &output["instances"][0];
        assert_eq!(
            instance["outline"],
            serde_json::json!([0.25, 0.5, 0.75, 1.0])
        );
        assert_eq!(instance["shaderUnderlayBias"], instance["shaderFaceBias"]);
        assert_eq!(instance["shaderUnderlayScale"], instance["shaderFaceScale"]);
    }

    #[test]
    fn shader_params_are_the_shared_glyph_material() {
        for size in [6.0f32, 8.0, 13.0, 18.0, 24.0, 31.5, 48.0, 72.0, 96.0, 250.0] {
            for bold in [false, true] {
                for dilate in [0.0f32, 0.3, 1.0] {
                    let params = compute_sdf_shader_params(size, bold, dilate, 1.0);
                    let material = TmpGlyphMaterial::new(tmp_uv2_y(size, bold), dilate);
                    let context = format!("size {size} bold {bold} dilate {dilate}");
                    assert_eq!(
                        params.face_scale.to_bits(),
                        material.face_scale.to_bits(),
                        "{context}"
                    );
                    assert_eq!(
                        params.face_bias.to_bits(),
                        material.face_bias.to_bits(),
                        "{context}"
                    );
                    assert_eq!(
                        params.underlay_scale.to_bits(),
                        material.underlay_scale.to_bits(),
                        "{context}"
                    );
                    assert_eq!(
                        params.underlay_bias.to_bits(),
                        material.underlay_bias.to_bits(),
                        "{context}"
                    );
                }
            }
        }
    }
}

fn effective_vertex_alpha(markup_alpha: u8, base_alpha: f32) -> f32 {
    let base_u8 = (base_alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    markup_alpha.min(base_u8) as f32 / 255.0
}

fn glyph_for<'a>(layer: &TextLayer, glyphs: &'a [GlyphInfo], text: &str) -> Option<&'a GlyphInfo> {
    let key = format!(
        "{}\u{0}{}\u{0}{}\u{0}{}",
        layer.region, layer.font_source_hash, layer.font_family, text
    );
    glyphs.iter().find(|glyph| glyph.key == key)
}

/// What one character takes from the layer's atlas.
enum LayerGlyph<'a> {
    /// A glyph the atlas carries, drawn in place of the character.
    Atlas { glyph: &'a GlyphInfo, ch: char },
    /// White space the atlas does not carry; its advance is estimated.
    Blank { ch: char },
    /// Nothing is drawn and the caret does not move.
    Missing { ch: char },
}

impl<'a> LayerGlyph<'a> {
    fn atlas_glyph(&self) -> Option<&'a GlyphInfo> {
        match self {
            Self::Atlas { glyph, .. } => Some(glyph),
            Self::Blank { .. } | Self::Missing { .. } => None,
        }
    }

    fn drawn_char(&self) -> char {
        match *self {
            Self::Atlas { ch, .. } | Self::Blank { ch } | Self::Missing { ch } => ch,
        }
    }

    fn advance(&self, font_size: f32, base_size: f32, family: &str) -> f32 {
        match self {
            Self::Atlas { glyph, .. } => glyph.advance * (font_size / base_size),
            Self::Blank { ch } => blank_advance(*ch, font_size, family),
            Self::Missing { .. } => 0.0,
        }
    }
}

/// Resolves `display`, the case-mapped form of `source`, against the layer's
/// atlas with TextMesh Pro's missing-glyph chain.
fn layer_glyph<'a>(
    layer: &TextLayer,
    glyphs: &'a [GlyphInfo],
    source: char,
    display: char,
) -> LayerGlyph<'a> {
    let lookup = |ch: char| glyph_for(layer, glyphs, &ch.to_string());
    if !glyph::advances_caret(source) {
        return LayerGlyph::Missing { ch: display };
    }
    if let Some(found) = lookup(display) {
        return LayerGlyph::Atlas {
            glyph: found,
            ch: display,
        };
    }
    if display.is_control() {
        return LayerGlyph::Missing { ch: display };
    }
    if display.is_whitespace() {
        // Glyph demand leaves white space out, so an atlas rarely carries it.
        let ch = glyph::same_face_alternate(display).unwrap_or(display);
        return lookup(ch).map_or(LayerGlyph::Blank { ch }, |found| LayerGlyph::Atlas {
            glyph: found,
            ch,
        });
    }
    match glyph::resolve_glyph(display, 1, |_, ch| ch == ' ' || lookup(ch).is_some()) {
        Some(choice) => {
            lookup(choice.glyph).map_or(LayerGlyph::Blank { ch: choice.glyph }, |found| {
                LayerGlyph::Atlas {
                    glyph: found,
                    ch: choice.glyph,
                }
            })
        }
        None => LayerGlyph::Missing { ch: display },
    }
}

/// Estimated advance of white space the atlas does not carry.
fn blank_advance(ch: char, font_size: f32, family: &str) -> f32 {
    if ch == ' ' {
        return (font_size * space_advance_ratio(family)).round();
    }
    if is_fullwidth(ch) {
        font_size
    } else {
        font_size * 0.5
    }
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
    matches!(cp, 0x2000..=0x206f | 0x3000..=0x30ff)
}

/// Characters the layout may draw: the visible, case-mapped characters that
/// are not white space, their same-face stand-ins, and the missing-glyph
/// square that replaces any the font lacks.
fn glyph_demand_chars(raw: &str) -> Vec<char> {
    let mut chars = Vec::new();
    for segment in parse_segments(raw, 0.0) {
        for source in segment.text.chars() {
            if source.is_whitespace() || source.is_control() {
                continue;
            }
            let (display, _) = segment.transform_char(source);
            chars.push(display);
            if let Some(alternate) =
                glyph::same_face_alternate(display).filter(|ch| !ch.is_whitespace())
            {
                chars.push(alternate);
            }
        }
    }
    if !chars.is_empty() {
        chars.push(glyph::MISSING_GLYPH_CHARACTER);
    }
    chars
}

fn resolve_fill(layer: &TextLayer, seg: &TextSegment) -> [f32; 4] {
    let color = seg.color.map_or(layer.color_rgb, |[r, g, b]| {
        [f32::from(r), f32::from(g), f32::from(b)]
    });
    [color[0] / 255.0, color[1] / 255.0, color[2] / 255.0, 1.0]
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
    use super::{build_glyph_demand_json, build_layout_json, glyph_demand_chars};
    use serde_json::{json, Value};

    const FAMILY: &str = "SyntheticSans";

    fn source_hash() -> String {
        "a".repeat(64)
    }

    fn atlas_glyph(ch: char, advance: f32) -> Value {
        json!({
            "key": format!("en\u{0}{}\u{0}{FAMILY}\u{0}{ch}", source_hash()),
            "page": 0, "u0": 0.0, "v0": 0.0, "u1": 0.1, "v1": 0.1,
            "advance": advance,
            "planeBearingX": 2.0, "planeBearingY": 50.0,
            "planeWidth": 30.0, "planeHeight": 50.0,
            "drawable": true
        })
    }

    fn layout(
        text: &str,
        font_size: f32,
        text_type: i32,
        line_spacing: f32,
        glyphs: &[(char, f32)],
    ) -> Value {
        let input = json!({
            "layers": [{
                "id": "text-layer", "z": 0, "text": text,
                "region": "en", "fontFamily": FAMILY, "fontSourceHash": source_hash(),
                "x": 0.0, "y": 0.0, "rotationDeg": 0.0, "scaleX": 1.0, "scaleY": 1.0,
                "fontSize": font_size, "color": [1.0, 1.0, 1.0, 1.0],
                "outlineColor": [0.0, 0.0, 0.0, 0.0], "colorRgb": [10.0, 20.0, 30.0],
                "outlineWidth": 0.0, "lineSpacing": line_spacing, "textType": text_type
            }],
            "atlas": {
                "baseSize": 75.0, "spread": 6.0,
                "glyphs": glyphs.iter().map(|(ch, advance)| atlas_glyph(*ch, *advance)).collect::<Vec<_>>()
            },
            "tick": 0, "frameMode": "animate"
        });
        serde_json::from_str(&build_layout_json(&input.to_string()).unwrap()).unwrap()
    }

    /// Latin capitals 60 units wide at the 75-unit atlas size: 19.2 at size 24.
    const LETTERS: &[(char, f32)] = &[('A', 60.0), ('B', 60.0), ('C', 60.0), ('\u{25A1}', 75.0)];
    const LETTER_ADVANCE: f64 = 19.2;

    fn instances(output: &Value) -> &Vec<Value> {
        output["instances"].as_array().unwrap()
    }

    fn op(instance: &Value, index: usize) -> f64 {
        instance["charOp"][index].as_f64().unwrap()
    }

    fn assert_close(actual: f64, expected: f64, what: &str) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "{what}: {actual} != {expected}"
        );
    }

    #[test]
    fn glyph_demand_uses_tmp_visible_transformed_scalars() {
        assert_eq!(
            glyph_demand_chars("<uppercase>aß</uppercase> <noparse><b></noparse>"),
            vec!['A', 'ß', '<', 'b', '>', '\u{25A1}'],
        );
    }

    #[test]
    fn glyph_demand_json_is_deduplicated_and_font_scoped() {
        let output: Value = serde_json::from_str(
            &build_glyph_demand_json(r#"{"layers":[{"text":"<b>12</b>","region":"en","fontFamily":"Inter","fontSourceHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},{"text":"2A","region":"en","fontFamily":"Inter","fontSourceHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#).unwrap(),
        ).unwrap();
        assert_eq!(output["source"], "wasm-tmp-glyph-demand");
        let chars: Vec<&str> = output["requests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|request| request["char"].as_str().unwrap())
            .collect();
        assert_eq!(chars, ["1", "2", "\u{25A1}", "A"]);
    }

    #[test]
    fn case_changes_map_one_character_to_one() {
        let output = layout("<uppercase>ß</uppercase>", 24.0, 1, 0.0, &[('ß', 50.0)]);
        let chars: Vec<&str> = instances(&output)
            .iter()
            .map(|instance| instance["char"].as_str().unwrap())
            .collect();
        assert_eq!(chars, ["ß"]);
    }

    #[test]
    fn multi_byte_colour_values_stay_literal_instead_of_panicking() {
        for text in ["<alpha=#日>A", "<color=#日日>A", "<#日日>A"] {
            let output = layout(text, 24.0, 1, 0.0, LETTERS);
            let count = instances(&output).len();
            assert_eq!(count, text.chars().count(), "{text:?}");
        }
    }

    #[test]
    fn named_colours_and_closing_tags_follow_tmp() {
        let output = layout("<color=red>A", 24.0, 1, 0.0, LETTERS);
        assert_eq!(instances(&output)[0]["fill"], json!([1.0, 0.0, 0.0, 1.0]));

        let output = layout("<color=#ff0000>A</color>B", 24.0, 1, 0.0, LETTERS);
        let fill = instances(&output)[1]["fill"].as_array().unwrap().clone();
        assert_close(fill[0].as_f64().unwrap(), 10.0 / 255.0, "element red");
        assert_close(fill[2].as_f64().unwrap(), 30.0 / 255.0, "element blue");

        // A colour tag replaces the alpha an earlier <alpha> set.
        let output = layout("<alpha=#40>AB<color=#ff0000>C", 24.0, 1, 0.0, LETTERS);
        let alphas: Vec<f64> = instances(&output)
            .iter()
            .map(|instance| instance["shaderVertexAlpha"].as_f64().unwrap())
            .collect();
        assert_close(alphas[0], 64.0 / 255.0, "alpha tag");
        assert_close(alphas[2], 1.0, "colour tag alpha");
    }

    #[test]
    fn unrecognised_tags_are_laid_out_as_text() {
        let glyphs: Vec<(char, f32)> = "<love>".chars().map(|ch| (ch, 40.0)).collect();
        let output = layout("<love>", 24.0, 1, 0.0, &glyphs);
        let text: String = instances(&output)
            .iter()
            .map(|instance| instance["char"].as_str().unwrap())
            .collect();
        assert_eq!(text, "<love>");
    }

    #[test]
    fn spacing_tags_resolve_units_and_move_the_caret() {
        let plain = layout("A", 24.0, 1, 0.0, LETTERS);
        let start = op(&instances(&plain)[0], 1);

        let spaced = layout("A<space=50>B", 24.0, 1, 0.0, LETTERS);
        let [a, b] = [0, 1].map(|index| op(&instances(&spaced)[index], 1));
        assert_close(b - a, LETTER_ADVANCE + 25.0, "space advance");

        // The second line holds no space and centres on its own glyph.
        let centred = layout("<space=50>A\nA", 24.0, 2, 0.0, LETTERS);
        assert_close(
            op(&instances(&centred)[1], 1),
            -LETTER_ADVANCE / 2.0,
            "second line start",
        );

        let cspace = layout("<cspace=0.1em>AB", 24.0, 1, 0.0, LETTERS);
        let [a, b] = [0, 1].map(|index| op(&instances(&cspace)[index], 1));
        assert_close(
            b - a,
            LETTER_ADVANCE + 0.1 * 24.0 / 2.0,
            "em character spacing",
        );

        let indent = layout("<indent=1em>A", 24.0, 1, 0.0, LETTERS);
        assert_close(op(&instances(&indent)[0], 1) - start, 12.0, "em indent");

        let line_indent = layout("<line-indent=20>A", 24.0, 1, 0.0, LETTERS);
        assert_close(
            op(&instances(&line_indent)[0], 1) - start,
            10.0,
            "pixel line indent",
        );
    }

    #[test]
    fn static_percent_line_indent_follows_the_unscaled_preferred_width() {
        let at = |scale: f32| {
            let output = layout(
                &format!("<line-indent=96%><size=1250><scale={scale}>A"),
                24.0,
                1,
                0.0,
                LETTERS,
            );
            op(&instances(&output)[0], 1)
        };
        assert_close(at(60.0), at(1.0), "static start under <scale>");
    }

    #[test]
    fn superscript_rises_by_the_face_offset() {
        let output = layout("A<sup>B</sup>", 24.0, 1, 0.0, LETTERS);
        let [a, b] = [0, 1].map(|index| op(&instances(&output)[index], 2));
        assert_close(a - b, 66.0 * 0.64 * 0.5 / 2.0, "superscript rise");
        assert_close(
            instances(&output)[1]["shaderFontSize"].as_f64().unwrap(),
            12.0,
            "superscript size",
        );
    }

    #[test]
    fn line_height_percent_and_line_spacing_use_tmp_units() {
        let output = layout("A\n<line-height=50%>A", 24.0, 1, 0.0, LETTERS);
        // 50% of the 150-unit face line height at 24 / 75 * 2 per unit.
        assert_close(
            output["instances"][0]["layoutMetrics"]["lineOffsets"][1]
                .as_f64()
                .unwrap(),
            48.0,
            "line height",
        );
        let gap = |line_spacing: f32| {
            let output = layout("A\nA", 300.0, 1, line_spacing, LETTERS);
            output["instances"][0]["layoutMetrics"]["lineOffsets"][1]
                .as_f64()
                .unwrap()
        };
        assert_close(gap(1.0) - gap(0.0), 2.0 * 1.325 * 3.0, "line spacing");
    }

    #[test]
    fn italic_and_small_caps_follow_tmp_geometry() {
        let output = layout("<i>A</i>", 24.0, 1, 0.0, LETTERS);
        assert_close(
            instances(&output)[0]["charPosition"][4].as_f64().unwrap(),
            -0.35,
            "italic slant",
        );
        let output = layout("<smallcaps>a</smallcaps>", 24.0, 1, 0.0, LETTERS);
        let instance = &instances(&output)[0];
        assert_eq!(instance["char"], "A");
        assert_close(
            instance["shaderFontSize"].as_f64().unwrap(),
            19.2,
            "small caps size",
        );
        assert_close(op(instance, 3), 1.0, "small caps horizontal scale");
    }

    #[test]
    fn a_character_the_atlas_lacks_is_drawn_as_the_missing_glyph_square() {
        let output = layout("A\u{6F22}B", 24.0, 1, 0.0, LETTERS);
        let square = &instances(&output)[1];
        assert_eq!(square["char"], "\u{25A1}");
        assert_eq!(square["drawable"], true);
        let [a, b] = [0, 2].map(|index| op(&instances(&output)[index], 1));
        assert_close(b - a, LETTER_ADVANCE + 24.0, "square advance");
    }

    #[test]
    fn plain_text_indices_match_the_numeric_runs() {
        let glyphs: Vec<(char, f32)> = "HP10".chars().map(|ch| (ch, 40.0)).collect();
        let text = "HP<br>100";
        let output = layout(text, 24.0, 1, 0.0, &glyphs);
        let digit_indices: Vec<u64> = instances(&output)
            .iter()
            .filter(|instance| {
                instance["char"]
                    .as_str()
                    .unwrap()
                    .chars()
                    .all(|ch| ch.is_ascii_digit())
            })
            .map(|instance| instance["plainTextIndex"].as_u64().unwrap())
            .collect();
        let runs = sekai_profile_renderer_core::tmp_text::numeric_text_runs(text);
        assert_eq!(runs.len(), 1);
        let run_indices: Vec<u64> = (runs[0].plain_start..runs[0].plain_end)
            .map(u64::from)
            .collect();
        assert_eq!(digit_indices, run_indices);
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
    fn line_indent_program_uses_preferred_width_advance_without_scale() {
        let request = |scale: f32| {
            serde_json::json!({
                "layers": [{
                    "id": "curtain", "z": 0,
                    "text": format!("<line-indent=96%><size=1250><scale={scale}>A"),
                    "region": "cn", "fontFamily": "SyntheticSans",
                    "fontSourceHash": "a".repeat(64),
                    "x": 0.0, "y": 0.0, "rotationDeg": 0.0,
                    "scaleX": 1.0, "scaleY": 1.0,
                    "fontSize": 24.0, "color": [1.0,1.0,1.0,1.0],
                    "outlineColor": [0.0,0.0,0.0,0.0],
                    "colorRgb": [255.0,255.0,255.0], "outlineWidth": 0.0,
                    "lineSpacing": 0.0, "textType": 0, "dynamic": null
                }],
                "atlas": { "baseSize": 75.0, "spread": 6.0, "glyphs": [] },
                "tick": 0, "frameMode": "animate"
            })
        };
        let normal: serde_json::Value =
            serde_json::from_str(&build_layout_json(&request(1.0).to_string()).unwrap()).unwrap();
        let scaled: serde_json::Value =
            serde_json::from_str(&build_layout_json(&request(60.0).to_string()).unwrap()).unwrap();

        assert_eq!(
            normal["dynamicPrograms"][0]["lineAdvancesTmp"],
            scaled["dynamicPrograms"][0]["lineAdvancesTmp"]
        );
        assert_ne!(
            normal["instances"][0]["layoutMetrics"]["lineWidths"],
            scaled["instances"][0]["layoutMetrics"]["lineWidths"]
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
    alignment: u8,
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
