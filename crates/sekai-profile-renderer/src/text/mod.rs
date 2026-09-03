//! 文本渲染相关模块。

mod font;
mod measure;
pub mod richtext;
pub(crate) mod simple_raster;

use crate::masterdata::{MasterData, ResolvedColor};
use crate::sdf::outline::{self as sdf_outline, lookup_or_generate};
use crate::text::font::resolve_tmp_face_info_constants;
use crate::text::measure::{
    resolve_indent_value, resolve_segment_font_size, segments_to_global, transform_char_for_segment,
};
use crate::text::richtext::{
    parse_rich_segments, Indent, InlineAlign, LineHeight, LineIndent, TextSegment,
};
use crate::types::TextElement;
#[cfg(feature = "skia-oracle")]
use skia_safe::Matrix;

/// TMP FontAsset 全局缩放因子 (m_FaceInfo.m_Scale)。
pub const TEXT_SCALE: f32 = 2.0;

/// Final draw-space placement for text that has already been laid out by the
/// TMP-compatible path. This never changes parsing, advances, line breaks,
/// alignment, or glyph layout; it only translates the completed glyph ops.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextRenderPlacement {
    /// Target left/center/right anchor in the caller's local draw space.
    pub anchor_x: f32,
    pub baseline: Option<f32>,
}

/// Loads the font bytes and immutable TMP face constants for every installed
/// profile atlas family before a worker announces READY.
///
/// Both are process-lifetime caches, so warming them here keeps the first
/// request from paying a disk read. This does not inspect request text and does
/// not generate glyphs.
pub fn prewarm_profile_font_families<'a>(
    families: impl IntoIterator<Item = &'a str>,
) -> Result<(u64, u64), String> {
    let started = std::time::Instant::now();
    let mut count = 0u64;
    for family in families {
        sdf_outline::load_font_bytes_for_family(family)
            .ok_or_else(|| format!("profile font prewarm could not resolve family {family}"))?;
        let _ = resolve_tmp_face_info_constants(Some(family));
        count = count.saturating_add(1);
    }
    Ok((count, capture_elapsed_ns(Some(started))))
}

/// Resolves one glyph's horizontal advance from FreeType only.
///
/// Order: the prebuilt atlas (its metrics are FreeType-derived), then on-demand
/// SDF generation when no atlas is installed, then FreeType's `hmtx` for glyphs
/// that have no outline to generate from. Skia is never consulted: it rounds
/// every advance to a whole pixel, and that error accumulates along a run.
fn freetype_advance_x(
    atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
    family: Option<&str>,
    ch: char,
    display_char: char,
    measure_size: f32,
) -> Option<f32> {
    atlas_layout_glyph_metrics(atlases, family, display_char)
        .map(|glyph| glyph.advance_x * (measure_size / glyph.point_size))
        .or_else(|| {
            // With atlases installed the atlas is authoritative; generating here
            // would put glyph work on the request path.
            if atlases.is_some() {
                None
            } else {
                lookup_or_generate(family, ch).as_ref().map(|g| {
                    g.plane_advance_x() * (measure_size / sdf_outline::sampling_point_size())
                })
            }
        })
        .filter(|v| *v > 0.0)
        .or_else(|| {
            sdf_outline::glyph_advance_x(family, ch)
                .map(|advance| advance * (measure_size / sdf_outline::sampling_point_size()))
        })
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

fn debug_text_probe_enabled() -> bool {
    std::env::var("SCAPUS_DEBUG_TMP_PROBE")
        .ok()
        .map(|v| {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
pub struct TextLineIndentAnimation {
    pub fps: u32,
    pub looped: bool,
    pub frames: Vec<TextLineIndentFrame>,
}

#[derive(Debug, Clone)]
pub struct TextLineIndentFrame {
    pub frame: u32,
    pub dx_local: f32,
}

fn update_cpv_width(max_width_tmp: &mut f32, cpv_xadv_tmp: f32, glyph_hadv_tmp: f32) {
    *max_width_tmp = (*max_width_tmp).max(cpv_xadv_tmp.abs() + glyph_hadv_tmp);
}

fn update_cpv_width_for_char(
    max_width_tmp: &mut f32,
    cpv_xadv_tmp: f32,
    glyph_hadv_tmp: f32,
    ch: char,
) {
    // TMP advances the caret through whitespace, but preferredWidth stops at
    // the last visible glyph. Internal whitespace is captured when the next
    // visible glyph updates the extent from its post-whitespace xAdvance.
    if !ch.is_whitespace() {
        update_cpv_width(max_width_tmp, cpv_xadv_tmp, glyph_hadv_tmp);
    }
}

pub fn line_indent_x_animation(
    text: &TextElement,
    md: &MasterData,
) -> Option<TextLineIndentAnimation> {
    const MAX_CONVERGENCE_FRAMES: usize = 20_000;
    const NON_CONVERGENT_OUTPUT_FRAMES: usize = 1_800;

    let source = line_indent_program(text, md)?;
    let pct = source.percent / 100.0;
    let converges_to_static = (0.0..1.0).contains(&pct);
    let max_output_frames = if converges_to_static {
        MAX_CONVERGENCE_FRAMES
    } else {
        NON_CONVERGENT_OUTPUT_FRAMES
    };
    let materialized =
        sekai_profile_renderer_core::materialize_line_indent(source, max_output_frames)?;
    Some(TextLineIndentAnimation {
        fps: materialized.fps,
        looped: materialized.looped,
        frames: materialized
            .frames
            .into_iter()
            .map(|frame| TextLineIndentFrame {
                frame: frame.tick,
                dx_local: frame.dx_local,
            })
            .collect(),
    })
}

pub fn line_indent_program(
    text: &TextElement,
    md: &MasterData,
) -> Option<sekai_profile_renderer_core::LineIndentSource> {
    line_indent_program_with_optional_atlases(text, md, None)
}

pub(crate) fn line_indent_program_with_atlases(
    text: &TextElement,
    md: &MasterData,
    atlases: &crate::sdf::atlas::MappedSdfAtlasSet,
) -> Option<sekai_profile_renderer_core::LineIndentSource> {
    line_indent_program_with_optional_atlases(text, md, Some(atlases))
}

fn line_indent_program_with_optional_atlases(
    text: &TextElement,
    md: &MasterData,
    atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
) -> Option<sekai_profile_renderer_core::LineIndentSource> {
    let segments = parse_rich_segments(&text.text);
    let mut percent = None;
    for segment in segments
        .iter()
        .filter(|segment| segment.text.chars().any(|ch| !ch.is_whitespace()))
    {
        let LineIndent::Percent(value) = segment.line_indent? else {
            return None;
        };
        if !value.is_finite()
            || percent.is_some_and(|current: f32| (current - value).abs() > f32::EPSILON)
        {
            return None;
        }
        percent = Some(value);
    }
    Some(sekai_profile_renderer_core::LineIndentSource {
        percent: percent?,
        line_advances_tmp: measure_line_advances_tmp(text, md, &segments, atlases)?,
        rotation_deg: 0.0,
        scale_x: 1.0,
    })
}

fn measure_line_advances_tmp(
    text: &TextElement,
    md: &MasterData,
    segments: &[TextSegment],
    atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
) -> Option<Vec<Vec<f32>>> {
    let units = measure_text_units_tmp(text, md, segments, atlases)?;
    let authored_lines = segments
        .iter()
        .flat_map(|segment| segment.text.chars())
        .collect::<String>()
        .split('\n')
        .map(|line| line.chars().any(|ch| !ch.is_whitespace()))
        .collect::<Vec<_>>();
    group_line_advances_tmp(&units, &authored_lines)
}

fn group_line_advances_tmp(
    units: &[sekai_profile_renderer_core::MeasuredTextUnit],
    authored_lines: &[bool],
) -> Option<Vec<Vec<f32>>> {
    let mut measured_lines = vec![Vec::new()];
    for unit in units {
        if unit.hard_break {
            measured_lines.push(Vec::new());
        } else {
            measured_lines.last_mut()?.push(unit.advance);
        }
    }
    if measured_lines.len() != authored_lines.len() {
        return None;
    }
    let lines = measured_lines
        .into_iter()
        .zip(authored_lines.iter().copied())
        .filter_map(|(line, has_visible_content)| has_visible_content.then_some(line))
        .collect::<Vec<_>>();
    (!lines.is_empty()).then_some(lines)
}

fn measure_text_units_tmp(
    text: &TextElement,
    md: &MasterData,
    segments: &[TextSegment],
    atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
) -> Option<Vec<sekai_profile_renderer_core::MeasuredTextUnit>> {
    let family = md.resolve_font(text.font_id);
    let base_size = text.size;
    let mut units = Vec::new();

    for seg in segments {
        if seg.text.is_empty() {
            continue;
        }
        let seg_size = resolve_segment_font_size(seg.size, text.size);
        let measure_size = if seg.subscript || seg.superscript {
            seg_size * 0.5
        } else {
            seg_size
        };
        let seg_scale = seg.scale.unwrap_or(1.0);
        let cspace_raw_tmp = seg.cspace.unwrap_or(0.0);

        for ch in seg.text.chars() {
            if ch == '\n' {
                units.push(sekai_profile_renderer_core::MeasuredTextUnit {
                    advance: 0.0,
                    hard_break: true,
                });
                continue;
            }
            let (display, char_scale) = transform_char_for_segment(ch, seg);
            let display_char = display.chars().next().unwrap_or(ch);
            let ft_hadv =
                freetype_advance_x(atlases, family.as_deref(), ch, display_char, measure_size);
            // A codepoint absent from the font has no advance and nothing to
            // draw; it is skipped rather than measured by another engine.
            let advance = ft_hadv?;
            let advance_tmp = advance * char_scale * seg_scale * TEXT_SCALE + cspace_raw_tmp;
            units.push(sekai_profile_renderer_core::MeasuredTextUnit {
                advance: advance_tmp,
                hard_break: false,
            });
        }
    }

    if units.is_empty() && !text.text.is_empty() {
        for ch in text.text.chars() {
            if ch == '\n' {
                units.push(sekai_profile_renderer_core::MeasuredTextUnit {
                    advance: 0.0,
                    hard_break: true,
                });
                continue;
            }
            if ch == '<' || ch == '>' {
                return None;
            }
            units.push(sekai_profile_renderer_core::MeasuredTextUnit {
                advance: freetype_advance_x(atlases, family.as_deref(), ch, ch, base_size)?
                    * TEXT_SCALE,
                hard_break: false,
            });
        }
    }

    Some(units)
}

pub fn wrap_rich_text_to_width(
    text: &TextElement,
    md: &MasterData,
    max_width: f32,
) -> Option<String> {
    let segments = parse_rich_segments(&text.text);
    let units = measure_text_units_tmp(text, md, &segments, None)?;
    sekai_profile_renderer_core::wrap_tmp_markup(&text.text, &units, max_width).ok()
}

pub(crate) fn wrap_rich_text_to_width_with_atlases(
    text: &TextElement,
    md: &MasterData,
    max_width: f32,
    atlases: &crate::sdf::atlas::MappedSdfAtlasSet,
) -> Option<String> {
    let segments = parse_rich_segments(&text.text);
    let units = measure_text_units_tmp(text, md, &segments, Some(atlases))?;
    sekai_profile_renderer_core::wrap_tmp_markup(&text.text, &units, max_width).ok()
}

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

struct DrawCharOp {
    ch: String,
    x: f32,
    y: f32,
    pivot_x: f32,
    pivot_y: f32,
    /// SDF footprint 的半展，与 `pivot_*` 同坐标系：墨迹盒各边外扩 atlas spread
    /// 后的一半，也就是光栅器实际采样的 atlas 矩形。footprint 绕墨迹中心对称，
    /// 而 `glyph_local_affine` 的局部原点正是墨迹中心，故可直接作局部盒半展。
    /// 无轮廓字形为 0——它不会被光栅化，footprint 也就不存在。
    half_w: f32,
    half_h: f32,
    shear_cx: f32,
    scale_x: f32,
    skew_x: f32,
    rotate_deg: f32,
    font_size: f32,
    /// Straight (non-premultiplied) RGBA of the glyph face, in unit range.
    face: [f32; 4],
    sdf_params: Option<crate::sdf::material::SdfOutlineParams>,
    mesh_carrier: crate::sdf::material::RuntimeLikeGlyphMeshCarrier,
}

#[derive(Clone, Copy)]
struct AtlasLayoutGlyphMetrics {
    point_size: f32,
    spread: f32,
    bearing_x: f32,
    bearing_y: f32,
    width: f32,
    height: f32,
    advance_x: f32,
}

fn atlas_layout_glyph_metrics(
    atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
    font_family: Option<&str>,
    ch: char,
) -> Option<AtlasLayoutGlyphMetrics> {
    let (_, atlas, glyph) = atlases?.profile_glyph_for_font_family(font_family?, u32::from(ch))?;
    Some(AtlasLayoutGlyphMetrics {
        point_size: atlas.manifest().point_size,
        spread: atlas.manifest().spread,
        bearing_x: glyph.plane_bearing[0],
        bearing_y: glyph.plane_bearing[1],
        width: glyph.plane_size[0],
        height: glyph.plane_size[1],
        advance_x: glyph.plane_advance_x,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedTextSdfGlyph {
    pub text: String,
    pub font_family: Option<String>,
    pub baseline_origin: crate::sdf::tile::Point2,
    pub font_size: f32,
    pub local_to_device: crate::sdf::tile::Affine2,
    pub material: crate::sdf::tile::SdfMaterial,
}

impl ResolvedTextSdfGlyph {
    pub(crate) fn to_sdf_command(
        &self,
        atlases: &crate::sdf::atlas::MappedSdfAtlasSet,
    ) -> Result<crate::sdf::tile::SdfDrawCommand, TextSdfCommandError> {
        let codepoint = self.single_codepoint()?;
        let font_family = self
            .font_family
            .as_deref()
            .ok_or(TextSdfCommandError::MissingFontIdentity)?;
        if atlases.atlas_for_font_family(font_family).is_none() {
            return Err(TextSdfCommandError::AtlasNotInstalled {
                font_family: font_family.to_string(),
            });
        }
        let (atlas_set, atlas, glyph) = atlases
            .profile_glyph_for_font_family(font_family, u32::from(codepoint))
            .ok_or(TextSdfCommandError::MissingGlyph {
                codepoint: u32::from(codepoint),
            })?;
        self.to_sdf_command_from_manifest(
            atlas_set,
            glyph,
            atlas.manifest().point_size,
            atlas.manifest().spread,
        )
    }

    fn single_codepoint(&self) -> Result<char, TextSdfCommandError> {
        let mut chars = self.text.chars();
        match (chars.next(), chars.next()) {
            (Some(codepoint), None) => Ok(codepoint),
            _ => Err(TextSdfCommandError::NotSingleScalar),
        }
    }

    pub(crate) fn to_sdf_command_from_manifest(
        &self,
        atlas_set: u16,
        glyph: &crate::sdf::atlas::SdfAtlasGlyphManifest,
        atlas_point_size: f32,
        atlas_spread: f32,
    ) -> Result<crate::sdf::tile::SdfDrawCommand, TextSdfCommandError> {
        crate::sdf::tile::SdfDrawCommand::from_atlas_glyph(
            crate::sdf::tile::SdfPrimitiveKind::Text,
            atlas_set,
            glyph,
            atlas_point_size,
            atlas_spread,
            self.baseline_origin,
            self.font_size,
            self.local_to_device,
            self.material,
        )
        .map_err(TextSdfCommandError::Placement)
    }
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub(crate) enum TextSdfCommandError {
    #[error("captured text operation is not exactly one Unicode scalar")]
    NotSingleScalar,
    #[error("captured glyph has no resolved font identity")]
    MissingFontIdentity,
    #[error("no atlas is installed for font family {font_family}")]
    AtlasNotInstalled { font_family: String },
    #[error("atlas does not contain U+{codepoint:04X}")]
    MissingGlyph { codepoint: u32 },
    #[error("invalid glyph placement: {0}")]
    Placement(#[from] crate::sdf::tile::SdfCommandBuildError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TextSdfCaptureError {
    PerspectiveTransform,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TextSdfCaptureTimings {
    pub rich_parse_ns: u64,
    pub font_resolve_ns: u64,
    pub layout_setup_ns: u64,
    pub measure_ns: u64,
    pub command_build_ns: u64,
    pub emit_ns: u64,
}

fn capture_elapsed_ns(started: Option<std::time::Instant>) -> u64 {
    started
        .map(|started| started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
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
#[cfg(feature = "skia-oracle")]
#[allow(dead_code)]
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

/// `glyph_local_matrix` 的纯仿射版本，供无 canvas 的捕获路径使用，与 SkMatrix
/// 链式复合**逐位一致**：T·R 的每一项都是精确值；`pre_scale` 是逐项 f32 乘法；
/// italic 的 `pre_concat` 按 SkMatrix::setConcat 的语义在 f64 中乘加后一次舍入。
/// sin/cos 与 Skia 相同：radians = deg·(π/180)，结果绝对值 ≤ 1/65536 时钳到 0。
/// 有 gated 测试逐位对拍两条路径。
fn glyph_local_affine(op: &DrawCharOp) -> [f32; 6] {
    let tx = op.x + op.pivot_x + op.shear_cx;
    let ty = op.y + op.pivot_y;
    let mut m = [1.0f32, 0.0, 0.0, 1.0, tx, ty];
    if op.rotate_deg.abs() > 0.001 {
        const SIN_COS_NEARLY_ZERO: f32 = 1.0 / ((1 << 16) as f32);
        let radians = -op.rotate_deg * (std::f32::consts::PI / 180.0);
        let snap = |v: f32| {
            if v.abs() <= SIN_COS_NEARLY_ZERO {
                0.0
            } else {
                v
            }
        };
        let (sin, cos) = (snap(radians.sin()), snap(radians.cos()));
        m = [cos, sin, -sin, cos, tx, ty];
    }
    m[0] *= op.scale_x;
    m[1] *= op.scale_x;
    if op.skew_x != 0.0 {
        m[2] = (f64::from(m[0]) * f64::from(op.skew_x) + f64::from(m[2])) as f32;
        m[3] = (f64::from(m[1]) * f64::from(op.skew_x) + f64::from(m[3])) as f32;
    }
    m
}

/// 计算字形 footprint 四角经 `glyph_local_matrix` 变换后的设备前坐标（TMP 等效坐标系，
/// 乘 TEXT_SCALE）。footprint 取绕 glyph center 的 ±(size/2 + spread) 盒，即 atlas
/// 矩形——`pivot` 是墨迹中心偏移而非半展，拿它当半展会系统性偏离；刚性旋转下保持矩形，
/// 复合产生剪切时退化为平行四边形——四角即可直接量化剪切。
/// 返回 [TL, TR, BR, BL] 各 (x, y)。
fn glyph_quad_corners(op: &DrawCharOp) -> [(f32, f32); 4] {
    let m = glyph_local_affine(op);
    // 字形相对其 center（绘制原点在 -pivot）的局部盒。center 在原点。
    let (hx, hy) = (op.half_w, op.half_h);
    let local = [(-hx, -hy), (hx, -hy), (hx, hy), (-hx, hy)];
    let mut out = [(0.0f32, 0.0f32); 4];
    for (i, (lx, ly)) in local.iter().enumerate() {
        let x = m[0] * lx + m[2] * ly + m[4];
        let y = m[1] * lx + m[3] * ly + m[5];
        out[i] = (x * TEXT_SCALE, -y * TEXT_SCALE);
    }
    out
}

#[cfg(feature = "skia-oracle")]
#[allow(dead_code)]
fn resolve_text_sdf_glyph_from_matrix(
    base: &skia_safe::M44,
    op: &DrawCharOp,
    resolved_font_family: Option<&str>,
) -> Result<ResolvedTextSdfGlyph, TextSdfCaptureError> {
    let mut local_to_device = base.clone();
    local_to_device.pre_concat(&skia_safe::M44::from(glyph_local_matrix(op)));
    let affine = local_to_device
        .to_m33()
        .to_affine()
        .ok_or(TextSdfCaptureError::PerspectiveTransform)?;
    resolve_text_sdf_glyph_from_affine(affine, op, resolved_font_family)
}

/// `base * local` for affine matrices in `[sx, ky, kx, sy, tx, ty]` layout.
///
/// The translation terms associate as `x + (y + t)` — the fold order of the
/// 4x4 concatenation this replaces — so the result stays bit-identical to the
/// canvas route on every input, rotation included.
fn concat_affine(base: [f32; 6], local: [f32; 6]) -> [f32; 6] {
    [
        base[0] * local[0] + base[2] * local[1],
        base[1] * local[0] + base[3] * local[1],
        base[0] * local[2] + base[2] * local[3],
        base[1] * local[2] + base[3] * local[3],
        base[0] * local[4] + (base[2] * local[5] + base[4]),
        base[1] * local[4] + (base[3] * local[5] + base[5]),
    ]
}

/// Canvas-free capture: the device transform arrives as a plain affine.
fn resolve_text_sdf_glyph_from_base_affine(
    base: [f32; 6],
    op: &DrawCharOp,
    resolved_font_family: Option<&str>,
) -> Result<ResolvedTextSdfGlyph, TextSdfCaptureError> {
    let local = glyph_local_affine(op);
    resolve_text_sdf_glyph_from_affine(concat_affine(base, local), op, resolved_font_family)
}

fn resolve_text_sdf_glyph_from_affine(
    affine: [f32; 6],
    op: &DrawCharOp,
    resolved_font_family: Option<&str>,
) -> Result<ResolvedTextSdfGlyph, TextSdfCaptureError> {
    let face_color = op.face;
    Ok(ResolvedTextSdfGlyph {
        text: op.ch.clone(),
        font_family: resolved_font_family.map(str::to_owned),
        baseline_origin: crate::sdf::tile::Point2::new(-op.pivot_x, -op.pivot_y),
        font_size: op.font_size,
        local_to_device: crate::sdf::tile::Affine2 {
            scale_x: affine[0],
            skew_y: affine[1],
            skew_x: affine[2],
            scale_y: affine[3],
            translate_x: affine[4],
            translate_y: affine[5],
        },
        material: crate::sdf::material::resolve_tile_material_direct(
            op.mesh_carrier,
            op.scale_x,
            face_color,
            op.sdf_params.as_ref(),
        ),
    })
}

/// One decoration draw a layout run produced alongside its glyph stream. The
/// SDF paths do not render decorations yet; the layout still records them so
/// the TMP semantics stay captured for the renderer that will.
#[allow(dead_code)]
struct TextDecorationOp {
    /// Straight (non-premultiplied) RGBA in unit range.
    rgba: [f32; 4],
    kind: TextDecorationKind,
}

#[allow(dead_code)]
enum TextDecorationKind {
    /// `<mark>` background rectangle.
    MarkRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    /// Underline / strikethrough stroke from `(x0, y)` to `(x1, y)`.
    Line {
        x0: f32,
        x1: f32,
        y: f32,
        stroke_width: f32,
    },
}

/// A completed TMP-compatible layout: the glyph operations, the decoration
/// draws, and the timings of the phases that produced them. Rendering and
/// capture both consume this one stream.
struct TextLayoutRun {
    font_family: Option<String>,
    draw_ops: Vec<DrawCharOp>,
    #[allow(dead_code)]
    decorations: Vec<TextDecorationOp>,
    timings: TextSdfCaptureTimings,
}

/// Captures the element-path TMP layout (no post-layout placement) against a
/// device transform supplied directly as an affine. The canvas route resolves
/// the same glyph stream through an M44 local-to-device, and the two
/// resolutions are pinned bit-equal.
pub(crate) fn capture_text_sdf_from_affine(
    base_affine: [f32; 6],
    text: &TextElement,
    md: &MasterData,
    atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
    observer: &mut dyn FnMut(Result<ResolvedTextSdfGlyph, TextSdfCaptureError>),
) -> TextSdfCaptureTimings {
    let mut run = layout_text_ops(text, md, None, atlases, None, true);
    let emit_started = Some(std::time::Instant::now());
    for op in &run.draw_ops {
        if op.ch.chars().all(char::is_whitespace) {
            continue;
        }
        observer(resolve_text_sdf_glyph_from_base_affine(
            base_affine,
            op,
            run.font_family.as_deref(),
        ));
    }
    run.timings.emit_ns = capture_elapsed_ns(emit_started);
    run.timings
}

/// Captures the production region-font-only TMP layout with the same
/// post-layout placement used by General components. Pixel generation stays
/// disabled; the observer receives the completed glyph operations for the SDF
/// tile executor. No raster backend is involved.
pub(crate) fn capture_text_sdf_with_placement(
    base_affine: [f32; 6],
    text: &TextElement,
    md: &MasterData,
    atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
    placement: TextRenderPlacement,
    outline_override: Option<TextOutlineOverride>,
    observer: &mut dyn FnMut(Result<ResolvedTextSdfGlyph, TextSdfCaptureError>),
) -> TextSdfCaptureTimings {
    let mut run = layout_text_ops(text, md, Some(placement), atlases, outline_override, true);
    let emit_started = Some(std::time::Instant::now());
    for op in &run.draw_ops {
        if op.ch.chars().all(char::is_whitespace) {
            continue;
        }
        observer(resolve_text_sdf_glyph_from_base_affine(
            base_affine,
            op,
            run.font_family.as_deref(),
        ));
    }
    run.timings.emit_ns = capture_elapsed_ns(emit_started);
    run.timings
}

/// Outline recipe supplied directly as RGBA for callers that resolved the
/// document color table before reaching the capture. Without it the outline
/// comes from the element's `outline_color_id` through the region color table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TextOutlineOverride {
    /// Straight (non-premultiplied) RGBA in unit range.
    pub rgba: [f32; 4],
    /// TMP outline width fraction, same units as `TextElement::outline_size`.
    pub size: f32,
}

fn resolve_outline_params(
    outline_override: Option<TextOutlineOverride>,
    text: &TextElement,
    md: &MasterData,
    font_size: f32,
) -> Option<crate::sdf::material::SdfOutlineParams> {
    if let Some(outline) = outline_override {
        return Some(crate::sdf::material::SdfOutlineParams {
            outline_r: outline.rgba[0],
            outline_g: outline.rgba[1],
            outline_b: outline.rgba[2],
            outline_a: outline.rgba[3],
            outline_size: outline.size,
            font_size,
        });
    }
    md.resolve_color(text.outline_color_id)
        .map(|oc| crate::sdf::material::SdfOutlineParams {
            outline_r: oc.r as f32 / 255.0,
            outline_g: oc.g as f32 / 255.0,
            outline_b: oc.b as f32 / 255.0,
            outline_a: oc.a as f32 / 255.0,
            outline_size: text.outline_size,
            font_size,
        })
}

/// Runs the full TMP-compatible layout for one text element: rich-text
/// parsing, measurement, line placement and glyph operation construction.
/// Nothing here touches a raster backend.
fn layout_text_ops(
    text: &TextElement,
    md: &MasterData,
    render_placement: Option<TextRenderPlacement>,
    capture_atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
    outline_override: Option<TextOutlineOverride>,
    timing_enabled: bool,
) -> TextLayoutRun {
    let capture_timing_enabled = timing_enabled;
    let rich_parse_started = capture_timing_enabled.then(std::time::Instant::now);
    let mut capture_timings = TextSdfCaptureTimings::default();
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
    capture_timings.rich_parse_ns = capture_elapsed_ns(rich_parse_started);

    let font_resolve_started = capture_timing_enabled.then(std::time::Instant::now);
    let resolved_name = md.resolve_font(text.font_id);
    let resolved_name_ref = resolved_name.as_deref();
    // Fail closed on an unavailable family. Substituting another face would
    // report metrics that disagree with the atlas built for the declared family,
    // so the element is skipped instead.
    if resolved_name_ref
        .and_then(sdf_outline::load_font_bytes_for_family)
        .is_none()
    {
        tracing::warn!(
            font_id = text.font_id,
            font_family = resolved_name_ref.unwrap_or("<none>"),
            "declared font family is unavailable; skipping the text element"
        );
        capture_timings.font_resolve_ns = capture_elapsed_ns(font_resolve_started);
        return TextLayoutRun {
            font_family: resolved_name,
            draw_ops: Vec::new(),
            decorations: Vec::new(),
            timings: capture_timings,
        };
    }

    let base_size = text.size;
    capture_timings.font_resolve_ns = capture_elapsed_ns(font_resolve_started);

    let layout_setup_started = capture_timing_enabled.then(std::time::Instant::now);
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
    capture_timings.layout_setup_ns = capture_elapsed_ns(layout_setup_started);

    let measure_started = capture_timing_enabled.then(std::time::Instant::now);
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
            let part_chars: Vec<char> = part.chars().collect();
            let cspace_raw_tmp = seg.cspace.unwrap_or(0.0);
            let seg_scale = seg.scale.unwrap_or(1.0);
            // voffset 用于 vertical bounds 追踪（TMP 单位，Y-up）。
            let voffset_tmp = seg.voffset.unwrap_or(0.0);
            let mut measured = 0.0f32;
            for ch in &part_chars {
                let (display, char_scale) = transform_char_for_segment(*ch, seg);
                // Advances come from FreeType only: it is the engine TMP itself
                // uses, and it reports the true subpixel advance. Skia rounds
                // every advance to a whole pixel (up to 0.5px per glyph), which
                // accumulates along a run and visibly deforms arc-laid text.
                // Codepoints the declared atlas lacks are pre-warmed into the
                // fallback atlas before rendering, so the atlas is authoritative
                // here and on-demand generation stays off the request path.
                let ft_hadv = freetype_advance_x(
                    capture_atlases,
                    resolved_name_ref,
                    *ch,
                    display.chars().next().unwrap_or(*ch),
                    measure_size,
                );
                let glyph_hadv_tmp_layout = ft_hadv.unwrap_or(0.0) * char_scale * TEXT_SCALE;
                measured += glyph_hadv_tmp_layout * seg_scale / TEXT_SCALE;
                update_cpv_width_for_char(
                    &mut max_cpv_width_tmp,
                    cpv_xadv_tmp,
                    glyph_hadv_tmp_layout,
                    *ch,
                );
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
            let mut measured = 0.0f32;
            for ch in remaining.chars() {
                let advance =
                    freetype_advance_x(capture_atlases, resolved_name_ref, ch, ch, base_size)
                        .unwrap_or(0.0);
                measured += advance;
                let glyph_hadv_tmp = advance * TEXT_SCALE;
                update_cpv_width_for_char(&mut max_cpv_width_tmp, cpv_xadv_tmp, glyph_hadv_tmp, ch);
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
            w_scaled += measured * global.scale;
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
    let lh_override: Option<f32> = segments.iter().find_map(|segment| {
        segment.line_height.map(|spec| {
            let size = resolve_segment_font_size(segment.size, text.size);
            match spec {
                LineHeight::Pixels(value) => value,
                LineHeight::Em(value) => value * size,
                LineHeight::Percent(value) => {
                    FACE_LINE_HEIGHT * value / 100.0 * (size / TMP_POINT_SIZE * TEXT_SCALE)
                }
            }
        })
    });
    let n_lines = line_max_sizes.len();

    // TMP lineGap 实测为 75.625（= m_LineHeight - (ascentLine - descentLine) + 0.625）
    // 0.625 是 TMP 内部的行间距修正项（通过 Frida 多数据点拟合确认）
    const ASCENT_LINE: f32 = 66.0;
    const DESCENT_LINE: f32 = -9.0;
    /// Face line height of the profile fonts, in the same units as the sampling
    /// point size. A percentage line height is a share of this value.
    const FACE_LINE_HEIGHT: f32 = 150.0;
    const LINE_GAP: f32 = FACE_LINE_HEIGHT - (-DESCENT_LINE + ASCENT_LINE) + 0.625;

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
    let has_outline = outline_override.map_or(text.outline_size > 0.0, |o| o.size > 0.0);
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

    capture_timings.measure_ns = capture_elapsed_ns(measure_started);
    let command_build_started = capture_timing_enabled.then(std::time::Instant::now);
    let mut render_consumed: Vec<usize> = vec![0; segments.len()];
    let mut draw_ops = Vec::new();
    let mut decorations = Vec::new();

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
                        if let Some(terminal_x) =
                            static_line_indent_terminal_x(pct, sw, max_rw, effective_align)
                        {
                            cursor_x = terminal_x;
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
            let (sr, sg, sb) = seg.color.unwrap_or((def_color.r, def_color.g, def_color.b));
            let sa_u8 = effective_vertex_alpha_u8(seg.alpha, def_color.a);
            let sa = sa_u8 as f32 / 255.0;

            let part_chars: Vec<char> = part.chars().collect();
            let mut measured = 0.0f32;
            for ch in &part_chars {
                let (display, char_scale) = transform_char_for_segment(*ch, seg);
                // One glyph is drawn per character, so the mark background is
                // measured over the same first codepoint the draw loop renders.
                let display_char = display.chars().next().unwrap_or(*ch);
                measured += freetype_advance_x(
                    capture_atlases,
                    resolved_name_ref,
                    *ch,
                    display_char,
                    render_size,
                )
                .unwrap_or(0.0)
                    * seg_scale
                    * char_scale;
            }

            if let Some((mr, mg, mb, ma)) = seg.mark_color {
                decorations.push(TextDecorationOp {
                    rgba: [
                        mr as f32 / 255.0,
                        mg as f32 / 255.0,
                        mb as f32 / 255.0,
                        ma as f32 / 255.0,
                    ],
                    kind: TextDecorationKind::MarkRect {
                        x: cursor_x,
                        y: ly - render_size * 0.85,
                        width: measured,
                        height: render_size * 1.1,
                    },
                });
            }

            let seg_chars: Vec<char> = part_chars;
            for ch in &seg_chars {
                let (ch_str, char_scale) = transform_char_for_segment(*ch, seg);
                let effective_scale = seg_scale * char_scale;
                let mono_cell = resolve_indent_value(seg.monospace, seg_size, box_w)
                    .map(|width| {
                        if seg.duospace && matches!(*ch, '.' | ':' | ',') {
                            width / 2.0
                        } else {
                            width
                        }
                    })
                    .unwrap_or(0.0);
                // 查询 SDF glyph，获取 FreeType 度量（与 TMP FontEngine 同源，NO_HINTING）
                let sdf_glyph = if capture_atlases.is_some() {
                    None
                } else {
                    lookup_or_generate(resolved_name_ref, *ch)
                };
                let metric_ch = ch_str.chars().next().unwrap_or(*ch);
                let atlas_metrics =
                    atlas_layout_glyph_metrics(capture_atlases, resolved_name_ref, metric_ch);
                let ft_scale = atlas_metrics.map_or_else(
                    || render_size / sdf_outline::sampling_point_size(),
                    |metrics| render_size / metrics.point_size,
                );
                let ft_advance_x = atlas_metrics
                    .map(|metrics| metrics.advance_x * ft_scale)
                    .or_else(|| sdf_glyph.as_ref().map(|g| g.plane_advance_x() * ft_scale))
                    .or_else(|| {
                        // Outline-free glyphs (the space) carry an hmtx advance
                        // but cannot produce an SDF.
                        sdf_outline::glyph_advance_x(resolved_name_ref, *ch)
                            .map(|advance| advance * ft_scale)
                    });
                let ft_pivot_x = atlas_metrics
                    .map(|metrics| (metrics.bearing_x + metrics.width / 2.0) * ft_scale)
                    .or_else(|| {
                        sdf_glyph
                            .as_ref()
                            .map(|g| (g.plane_bearing_x() + g.plane_width() / 2.0) * ft_scale)
                    });
                // 优先使用 FreeType 度量计算 pivot，回退到 Skia
                // A glyph with no FreeType metrics has no outline, so it is not
                // rasterized and its pivot is never consumed.
                let pivot_x = ft_pivot_x.unwrap_or(0.0);
                // FreeType Y 中心：TMP 使用 FontEngine 的 bearingY - height/2
                // Skia Y-down 对应: -(bearing_y_75 - height_75/2) * ft_scale
                let ft_pivot_y = atlas_metrics
                    .map(|metrics| -(metrics.bearing_y - metrics.height / 2.0) * ft_scale)
                    .or_else(|| {
                        sdf_glyph
                            .as_ref()
                            .map(|g| -(g.plane_bearing_y() - g.plane_height() / 2.0) * ft_scale)
                    });
                let pivot_y = ft_pivot_y.unwrap_or(0.0);
                // SDF footprint：墨迹盒各边外扩 spread，即 atlas 矩形。绕墨迹中心
                // 对称，所以半展直接就是 (size/2 + spread)。
                let ft_half_extents = atlas_metrics
                    .map(|metrics| {
                        (
                            (metrics.width / 2.0 + metrics.spread) * ft_scale,
                            (metrics.height / 2.0 + metrics.spread) * ft_scale,
                        )
                    })
                    .or_else(|| {
                        sdf_glyph.as_ref().map(|g| {
                            let spread = sdf_outline::sampling_spread();
                            (
                                (g.plane_width() / 2.0 + spread) * ft_scale,
                                (g.plane_height() / 2.0 + spread) * ft_scale,
                            )
                        })
                    });
                let (half_w, half_h) = ft_half_extents.unwrap_or((0.0, 0.0));
                // TMP italic shear 公式（从源码 + Frida 5 字符验证推导）：
                // midPoint = height/2 + TMP_SPREAD; center_shift = 0.35 * (bY - h - spread) * base_eS
                // 等价于：shear_cx = 0.35 * (bearingY - height - spread) * ft_scale
                // base_eS 不含 scale 标签（center 在 scale 变换下不变，已验证）
                let shear_cx = if seg.italic {
                    if let Some(metrics) = atlas_metrics {
                        0.35 * (metrics.bearing_y - metrics.height - metrics.spread) * ft_scale
                    } else if let Some(g) = sdf_glyph.as_ref() {
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
                    // pivot_x is the FreeType glyph-ink centre.
                    cursor_x + mono_cell / 2.0 - pivot_x
                } else {
                    cursor_x
                };

                draw_ops.push(DrawCharOp {
                    ch: ch_str,
                    x: draw_x,
                    y: ly + baseline_shift,
                    pivot_x,
                    pivot_y,
                    half_w,
                    half_h,
                    shear_cx,
                    scale_x: effective_scale,
                    skew_x: if seg.italic { -0.21 } else { 0.0 },
                    rotate_deg: seg.rotate.unwrap_or(0.0),
                    font_size: render_size,
                    face: [sr as f32 / 255.0, sg as f32 / 255.0, sb as f32 / 255.0, 1.0],
                    sdf_params: if has_outline {
                        resolve_outline_params(outline_override, text, md, render_size)
                    } else {
                        None
                    },
                    mesh_carrier: crate::sdf::material::runtime_like_mesh_carrier(
                        render_size,
                        seg.bold,
                        sa_u8,
                    ),
                });

                if mono_cell > 0.0 {
                    cursor_x += mono_cell + cspace_px;
                } else {
                    // Cursor advance comes from FreeType, the same engine TMP uses.
                    // A glyph with no metrics is not drawn, so it advances nothing.
                    let adv = ft_advance_x.unwrap_or(0.0);
                    cursor_x += adv * effective_scale + cspace_px;
                }
            }

            if seg.underline && !seg_chars.is_empty() {
                decorations.push(TextDecorationOp {
                    rgba: [sr as f32 / 255.0, sg as f32 / 255.0, sb as f32 / 255.0, sa],
                    kind: TextDecorationKind::Line {
                        x0: cursor_x - measured,
                        x1: cursor_x,
                        y: ly + baseline_shift + render_size * 0.15,
                        stroke_width: (render_size * 0.05).max(1.0),
                    },
                });
            }

            if seg.strikethrough && !seg_chars.is_empty() {
                decorations.push(TextDecorationOp {
                    rgba: [sr as f32 / 255.0, sg as f32 / 255.0, sb as f32 / 255.0, sa],
                    kind: TextDecorationKind::Line {
                        x0: cursor_x - measured,
                        x1: cursor_x,
                        y: ly + baseline_shift - render_size * 0.3,
                        stroke_width: (render_size * 0.05).max(1.0),
                    },
                });
            }
        }

        if !remaining.is_empty() {
            let (fr, fg, fb) = global
                .color
                .unwrap_or((def_color.r, def_color.g, def_color.b));
            let fa_u8 = effective_vertex_alpha_u8(global.alpha, def_color.a);

            draw_ops.push(DrawCharOp {
                ch: remaining.to_string(),
                x: cursor_x,
                y: ly,
                pivot_x: 0.0,
                pivot_y: 0.0,
                half_w: 0.0,
                half_h: 0.0,
                shear_cx: 0.0,
                scale_x: global.scale,
                skew_x: 0.0,
                rotate_deg: 0.0,
                font_size: base_size,
                face: [fr as f32 / 255.0, fg as f32 / 255.0, fb as f32 / 255.0, 1.0],
                sdf_params: if has_outline {
                    resolve_outline_params(outline_override, text, md, base_size)
                } else {
                    None
                },
                mesh_carrier: crate::sdf::material::runtime_like_mesh_carrier(
                    base_size, false, fa_u8,
                ),
            });
        }

        if debug_probe {
            // xAdvance 现在由测量循环的独立 caret 链提供，不再从渲染循环 cursor 计算。
        }
    }

    if let Some(placement) = render_placement {
        let (dx, dy) = text_render_translation(
            align,
            box_w,
            anchor_base,
            placement.anchor_x,
            placement.baseline,
        );
        for op in &mut draw_ops {
            op.x += dx;
            op.y += dy;
        }
    }
    capture_timings.command_build_ns = capture_elapsed_ns(command_build_started);

    let _ = (SDF_DILATE_SCALE, TMP_POINT_SIZE_OUTLINE);

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
    TextLayoutRun {
        font_family: resolved_name,
        draw_ops,
        decorations,
        timings: capture_timings,
    }
}

fn text_render_translation(
    align: i32,
    auto_box_width: f32,
    anchor_base: f32,
    target_anchor_x: f32,
    target_baseline: Option<f32>,
) -> (f32, f32) {
    let auto_anchor_x = match align & 0x07 {
        2 => 0.0,
        4 => auto_box_width / 2.0,
        _ => -auto_box_width / 2.0,
    };
    let dx = target_anchor_x - auto_anchor_x;
    let dy = target_baseline.map_or(0.0, |baseline| baseline - anchor_base);
    (dx, dy)
}

fn static_line_indent_terminal_x(
    pct: f32,
    caret_width: f32,
    preferred_width: f32,
    align: i32,
) -> Option<f32> {
    if pct >= 1.0 {
        return None;
    }
    const TMP_PAD: f32 = 64.0;
    // TextContentView feeds TMP preferredWidth + 64 back into the next frame's
    // RectTransform.  Keep caret_width for alignment, but never use the
    // scale-sensitive caret advance as the feedback container width.
    let feedback_width_tmp = preferred_width * TEXT_SCALE;
    let rect_tmp = (feedback_width_tmp + TMP_PAD) / (1.0 - pct);
    let indent = rect_tmp * pct / TEXT_SCALE;
    Some(match align {
        2 => (indent - caret_width) / 2.0,
        4 => rect_tmp / (2.0 * TEXT_SCALE) - caret_width,
        _ => rect_tmp * (pct - 0.5) / TEXT_SCALE,
    })
}

#[cfg(test)]
mod tests {
    use super::effective_vertex_alpha;

    /// A capture given the outline as resolved RGBA must produce exactly the
    /// glyph stream the color-table route produces for the same color, and a
    /// zero-width override must disable the outline entirely.
    #[test]
    fn outline_override_matches_the_color_table_route() {
        use std::sync::Arc;

        use crate::masterdata::{MasterData, MasterDataProvider, ResolvedColor};
        use crate::types::{ObjectData, Quaternion, TextElement, Vec3};

        struct OutlineProvider;
        impl MasterDataProvider for OutlineProvider {
            fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> {
                None
            }
            fn get_card(&self, _: i32) -> Option<crate::types::CardEntry> {
                None
            }
            fn resolve_color(&self, color_id: i32) -> Option<ResolvedColor> {
                (color_id == 7).then_some(ResolvedColor {
                    r: 204,
                    g: 51,
                    b: 25,
                    a: 230,
                })
            }
            fn resolve_font(&self, _: i32) -> Option<String> {
                Some("FZLanTingHei-DB-GBK".into())
            }
            fn resolve_stamp(&self, _: i32) -> Option<String> {
                None
            }
            fn resolve_resource(&self, _: &str, _: i32) -> Option<crate::masterdata::ResourceInfo> {
                None
            }
            fn resolve_honor(&self, _: i32, _: i32) -> Option<crate::masterdata::ResolvedHonor> {
                None
            }
            fn get_bonds_honor(&self, _: i32) -> Option<crate::types::BondsHonorEntry> {
                None
            }
            fn get_bonds_honor_word(&self, _: i64) -> Option<crate::types::BondsHonorWordEntry> {
                None
            }
            fn get_honor(&self, _: i32) -> Option<crate::types::HonorEntry> {
                None
            }
            fn resolve_unit_vs_sd(&self, _: i32, _: i32) -> i32 {
                0
            }
            fn font_count(&self) -> usize {
                1
            }
            fn color_count(&self) -> usize {
                1
            }
        }

        if crate::sdf::outline::load_font_bytes_for_family("FZLanTingHei-DB-GBK").is_none() {
            eprintln!("skipping: FONT_DIR does not provide the test family");
            return;
        }

        let element = |outline_size: f32| TextElement {
            object_data: ObjectData {
                layer: 0,
                lock: false,
                position: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                rotation: Quaternion {
                    w: 1.0,
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                scale: Vec3 {
                    x: 1.0,
                    y: 1.0,
                    z: 1.0,
                },
                visible: true,
            },
            color_id: 7,
            font_id: 1,
            line_spacing: 0.0,
            outline_color_id: 7,
            outline_size,
            size: 24.0,
            text: "AB7".into(),
            text_type: 2,
        };
        let capture = |text: &TextElement, outline: Option<super::TextOutlineOverride>| {
            let mut glyphs = Vec::new();
            let mut observer =
                |result: Result<super::ResolvedTextSdfGlyph, super::TextSdfCaptureError>| {
                    glyphs.push(result.expect("captured glyph"));
                };
            let md = MasterData::new(Arc::new(OutlineProvider));
            // The element is centre-aligned, so a zero-anchor placement is the
            // identity translation the canvas route historically applied.
            super::capture_text_sdf_with_placement(
                [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                text,
                &md,
                None,
                super::TextRenderPlacement {
                    anchor_x: 0.0,
                    baseline: None,
                },
                outline,
                &mut observer,
            );
            glyphs
        };

        let table_route = capture(&element(0.4), None);
        assert!(!table_route.is_empty(), "capture must produce glyphs");
        let override_route = capture(
            &element(0.0),
            Some(super::TextOutlineOverride {
                rgba: [204.0 / 255.0, 51.0 / 255.0, 25.0 / 255.0, 230.0 / 255.0],
                size: 0.4,
            }),
        );
        assert_eq!(
            override_route, table_route,
            "the override must reproduce the color-table glyph stream"
        );

        let disabled = capture(
            &element(0.4),
            Some(super::TextOutlineOverride {
                rgba: [1.0; 4],
                size: 0.0,
            }),
        );
        let plain = capture(&element(0.0), None);
        assert_eq!(
            disabled, plain,
            "a zero-width override must disable the outline"
        );
        assert_ne!(
            table_route, plain,
            "the outline must actually change the glyph materials"
        );
    }

    #[test]
    fn multiline_line_indent_preserves_every_visible_line_for_feedback() {
        use sekai_profile_renderer_core::MeasuredTextUnit;

        let units = vec![
            MeasuredTextUnit {
                advance: 0.0,
                hard_break: true,
            },
            MeasuredTextUnit {
                advance: 12.0,
                hard_break: false,
            },
            MeasuredTextUnit {
                advance: 18.0,
                hard_break: false,
            },
            MeasuredTextUnit {
                advance: 0.0,
                hard_break: true,
            },
            MeasuredTextUnit {
                advance: 20.0,
                hard_break: false,
            },
        ];

        let selected = super::group_line_advances_tmp(&units, &[false, true, true]).unwrap();
        assert_eq!(selected, vec![vec![12.0, 18.0], vec![20.0]]);
    }

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

    #[test]
    fn cpv_width_keeps_negative_pos_extent() {
        let mut width = 0.0;

        super::update_cpv_width(&mut width, -221.0, 31.0);

        assert!((width - 252.0).abs() < 1e-6);
    }

    #[test]
    fn cpv_width_excludes_trailing_spaces_but_caret_keeps_advancing() {
        let mut width = 0.0;
        let mut xadv = 0.0;

        super::update_cpv_width_for_char(&mut width, xadv, 24.0, ' ');
        xadv += 24.0;
        super::update_cpv_width_for_char(&mut width, xadv, 110.0, '●');
        xadv += 110.0;
        for _ in 0..5 {
            super::update_cpv_width_for_char(&mut width, xadv, 24.0, ' ');
            xadv += 24.0;
        }

        assert!((width - 134.0).abs() < 1e-6);
        assert!((xadv - 254.0).abs() < 1e-6);
    }

    #[test]
    fn render_translation_anchors_completed_layout_without_changing_its_metrics() {
        let left = super::text_render_translation(1, 100.0, 4.2, -30.0, Some(1.8));
        let center = super::text_render_translation(2, 100.0, 4.2, 0.0, None);
        let right = super::text_render_translation(4, 100.0, 4.2, 30.0, None);
        assert!((left.0 - 20.0).abs() < 1e-6);
        assert!((left.1 + 2.4).abs() < 1e-6);
        assert_eq!(center, (0.0, 0.0));
        assert!((right.0 + 20.0).abs() < 1e-6);
        assert_eq!(right.1, 0.0);
    }

    #[test]
    fn static_line_indent_terminal_position_uses_preferred_width_feedback() {
        for (caret_width, preferred_width, pct) in [
            (70.0, 100.0, 0.939),
            (100.0, 100.0, 0.939),
            (120.0, 100.0, 0.939),
        ] {
            let actual =
                super::static_line_indent_terminal_x(pct, caret_width, preferred_width, 1).unwrap();
            let rect_tmp = (preferred_width * super::TEXT_SCALE + 64.0) / (1.0 - pct);
            let expected = rect_tmp * (pct - 0.5) / super::TEXT_SCALE;
            assert!(
                (actual - expected).abs() < 1e-4,
                "caret={caret_width} preferred={preferred_width}: actual={actual} expected={expected}"
            );
        }
    }

    #[test]
    fn captured_sdf_glyph_rejects_non_scalar_text_runs() {
        let captured = super::ResolvedTextSdfGlyph {
            text: "AB".into(),
            font_family: Some("test".into()),
            baseline_origin: crate::sdf::tile::Point2::new(0.0, 0.0),
            font_size: 12.0,
            local_to_device: crate::sdf::tile::Affine2::IDENTITY,
            material: crate::sdf::tile::SdfMaterial::default(),
        };
        assert_eq!(
            captured.single_codepoint(),
            Err(super::TextSdfCommandError::NotSingleScalar)
        );
    }

    #[cfg(feature = "skia-oracle")]
    #[test]
    fn direct_capture_matrix_matches_canvas_concat_recipe() {
        use skia_safe::Point;

        let face = [0.2f32, 0.4, 0.6, 0.75];
        let op = super::DrawCharOp {
            ch: "A".into(),
            x: 13.25,
            y: -8.5,
            pivot_x: 4.75,
            pivot_y: -2.25,
            half_w: 8.5,
            half_h: 11.25,
            shear_cx: 1.5,
            scale_x: 1.35,
            skew_x: -0.21,
            rotate_deg: 37.0,
            font_size: 0.0,
            face,
            sdf_params: Some(crate::sdf::material::SdfOutlineParams {
                outline_r: 0.8,
                outline_g: 0.3,
                outline_b: 0.1,
                outline_a: 0.9,
                outline_size: 0.4,
                font_size: 24.0,
            }),
            mesh_carrier: crate::sdf::material::runtime_like_mesh_carrier(24.0, true, 193),
        };
        let mut surface = skia_safe::surfaces::null((64, 64)).expect("null surface");
        let canvas = surface.canvas();
        canvas.translate(Point::new(17.0, -9.0));
        canvas.rotate(23.0, None);
        canvas.scale((1.2, 0.8));
        canvas.scale((super::TEXT_SCALE, super::TEXT_SCALE));
        let base = canvas.local_to_device();

        canvas.save();
        canvas.concat(&super::glyph_local_matrix(&op));
        let canvas_affine = canvas
            .local_to_device_as_3x3()
            .to_affine()
            .expect("canvas affine");
        let canvas_capture =
            super::resolve_text_sdf_glyph_from_affine(canvas_affine, &op, Some("test-font"))
                .expect("canvas capture");
        canvas.restore();
        let direct_capture =
            super::resolve_text_sdf_glyph_from_matrix(&base, &op, Some("test-font"))
                .expect("direct capture");

        assert_eq!(direct_capture, canvas_capture);
    }

    /// The canvas-free affine base must resolve exactly the glyph the M44
    /// canvas route resolves, including under rotation, non-uniform scale and
    /// fractional translation.
    #[cfg(feature = "skia-oracle")]
    #[test]
    fn affine_capture_base_matches_the_canvas_matrix_route() {
        let bases: [[f32; 6]; 4] = [
            [1.0, 0.0, 0.0, 1.0, 80.7, -20.3],
            [2.0, 0.0, 0.0, 2.0, 0.25, 0.75],
            [0.9271839, 0.3746066, -0.3746066, 0.9271839, 17.0, -9.0],
            [1.2, -0.15, 0.35, 0.8, -3.25, 41.5],
        ];
        let face = [0.2f32, 0.4, 0.6, 0.75];
        let op = super::DrawCharOp {
            ch: "字".into(),
            x: 13.25,
            y: -8.5,
            pivot_x: 4.75,
            pivot_y: -2.25,
            half_w: 8.5,
            half_h: 11.25,
            shear_cx: 1.5,
            scale_x: 1.35,
            skew_x: -0.21,
            rotate_deg: 37.0,
            font_size: 0.0,
            face,
            sdf_params: None,
            mesh_carrier: crate::sdf::material::runtime_like_mesh_carrier(24.0, true, 193),
        };
        for base in bases {
            let m44 = skia_safe::M44::from(skia_safe::Matrix::from_affine(&base));
            let matrix_route =
                super::resolve_text_sdf_glyph_from_matrix(&m44, &op, Some("test-font"))
                    .expect("matrix route");
            let affine_route =
                super::resolve_text_sdf_glyph_from_base_affine(base, &op, Some("test-font"))
                    .expect("affine route");
            assert_eq!(affine_route, matrix_route, "base {base:?}");
        }
    }

    /// The pure-affine glyph transform must equal the SkMatrix chain bit for
    /// bit on every composition the layout can produce: plain, rotated,
    /// scaled, italic-skewed, and all of those combined.
    #[cfg(feature = "skia-oracle")]
    #[test]
    fn glyph_local_affine_matches_the_skia_matrix_chain() {
        let mut case = 0u32;
        for rotate_deg in [0.0f32, 0.0005, 37.0, -218.4, 90.0, 179.99] {
            for scale_x in [1.0f32, 1.35, 0.4821] {
                for skew_x in [0.0f32, -0.21] {
                    let op = super::DrawCharOp {
                        ch: "字".into(),
                        x: 13.25,
                        y: -8.5,
                        pivot_x: 4.75,
                        pivot_y: -2.25,
                        half_w: 8.5,
                        half_h: 11.25,
                        shear_cx: 1.5,
                        scale_x,
                        skew_x,
                        rotate_deg,
                        font_size: 24.0,
                        face: [0.2, 0.4, 0.6, 1.0],
                        sdf_params: None,
                        mesh_carrier: crate::sdf::material::runtime_like_mesh_carrier(
                            24.0, false, 255,
                        ),
                    };
                    let matrix = super::glyph_local_matrix(&op)
                        .to_affine()
                        .expect("affine matrix");
                    let affine = super::glyph_local_affine(&op);
                    for (lane, (a, b)) in affine.iter().zip(matrix.iter()).enumerate() {
                        assert_eq!(
                            a.to_bits(),
                            b.to_bits(),
                            "rotate {rotate_deg} scale {scale_x} skew {skew_x} lane {lane}: {a} vs {b}"
                        );
                    }
                    case += 1;
                }
            }
        }
        assert_eq!(case, 36);
    }

    /// The debug footprint is the atlas rect the rasterizer samples: the glyph
    /// ink box inflated by the sampling spread on every side, centred on the
    /// ink centre. A `<scale>` tag stretches it along X only and a `<rotate>`
    /// tag merely reorients it, so the pair of side lengths is a
    /// rotation-invariant signature of the device footprint.
    #[test]
    fn glyph_quad_footprint_is_the_padded_ink_box_stretched_on_x_only() {
        for rotate_deg in [0.0f32, 37.0, -218.4, 90.0] {
            for scale_x in [1.0f32, 1.2, 6.0] {
                let op = super::DrawCharOp {
                    ch: "\u{25cf}".into(),
                    x: 13.25,
                    y: -8.5,
                    pivot_x: 4.75,
                    pivot_y: -2.25,
                    half_w: 8.5,
                    half_h: 11.25,
                    shear_cx: 0.0,
                    scale_x,
                    skew_x: 0.0,
                    rotate_deg,
                    font_size: 24.0,
                    face: [0.2, 0.4, 0.6, 1.0],
                    sdf_params: None,
                    mesh_carrier: crate::sdf::material::runtime_like_mesh_carrier(24.0, false, 255),
                };
                let quad = super::glyph_quad_corners(&op);
                let side = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).hypot(a.1 - b.1);
                let mut got = [side(quad[0], quad[1]), side(quad[1], quad[2])];
                let mut want = [
                    2.0 * op.half_w * scale_x * super::TEXT_SCALE,
                    2.0 * op.half_h * super::TEXT_SCALE,
                ];
                got.sort_by(|a, b| a.partial_cmp(b).expect("finite side"));
                want.sort_by(|a, b| a.partial_cmp(b).expect("finite side"));
                for (got, want) in got.iter().zip(want.iter()) {
                    assert!(
                        (got - want).abs() <= 1e-3,
                        "rotate {rotate_deg} scale {scale_x}: got {got} want {want}"
                    );
                }
            }
        }
    }

    #[test]
    fn captured_sdf_glyph_maps_manifest_metrics_without_relayout() {
        let captured = super::ResolvedTextSdfGlyph {
            text: "字".into(),
            font_family: Some("test".into()),
            baseline_origin: crate::sdf::tile::Point2::new(10.0, 20.0),
            font_size: 20.0,
            local_to_device: crate::sdf::tile::Affine2::IDENTITY,
            material: crate::sdf::tile::SdfMaterial::default(),
        };
        let glyph = crate::sdf::atlas::SdfAtlasGlyphManifest {
            codepoint: u32::from('字'),
            page: 2,
            rect: [32, 64, 12, 14],
            plane_bearing: [2.0, 7.0],
            plane_size: [4.0, 5.0],
            plane_advance_x: 4.5,
        };
        let command = captured
            .to_sdf_command_from_manifest(7, &glyph, 10.0, 1.0)
            .expect("captured glyph command");
        assert_eq!(command.atlas_page, 2);
        assert_eq!(command.atlas_set, 7);
        assert_eq!(command.atlas_rect, [32, 64, 12, 14]);
        assert_eq!(
            command.quad,
            [
                crate::sdf::tile::Point2::new(12.0, 4.0),
                crate::sdf::tile::Point2::new(24.0, 4.0),
                crate::sdf::tile::Point2::new(24.0, 18.0),
                crate::sdf::tile::Point2::new(12.0, 18.0),
            ]
        );
    }
}
