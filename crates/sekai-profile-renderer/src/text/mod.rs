//! 文本渲染相关模块。
//!
//! 富文本规则、断行与缺字替换都来自 core 的 `tmp_text`；这里只做度量与字形排布。

pub(crate) mod simple_raster;

use crate::masterdata::{MasterData, ResolvedColor};
use crate::sdf::outline::{self as sdf_outline, lookup_or_generate};
use crate::types::TextElement;
use sekai_profile_renderer_core::tmp_text::{
    self, glyph, layout as tmp_layout,
    markup::{CaretCommand, InlineAlign, TextSegment},
    DEFAULT_LINE_SPACING_FACTOR, PROFILE_FACE,
};
#[cfg(feature = "skia-oracle")]
use skia_safe::Matrix;

/// TMP FontAsset 全局缩放因子 (m_FaceInfo.m_Scale)。
pub const TEXT_SCALE: f32 = PROFILE_FACE.scale;

/// Final draw-space placement for text that has already been laid out by the
/// TMP-compatible path. This never changes parsing, advances, line breaks,
/// alignment, or glyph layout; it only translates the completed glyph ops.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextRenderPlacement {
    /// Target left/center/right anchor in the caller's local draw space.
    pub anchor_x: f32,
    pub baseline: Option<f32>,
}

/// Loads the font bytes for every installed profile atlas family before a
/// worker announces READY.
///
/// The bytes are a process-lifetime cache, so warming them here keeps the
/// first request from paying a disk read. This does not inspect request text
/// and does not generate glyphs.
pub fn prewarm_profile_font_families<'a>(
    families: impl IntoIterator<Item = &'a str>,
) -> Result<(u64, u64), String> {
    let started = std::time::Instant::now();
    let mut count = 0u64;
    for family in families {
        sdf_outline::load_font_bytes_for_family(family)
            .ok_or_else(|| format!("profile font prewarm could not resolve family {family}"))?;
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

/// The glyph TextMesh Pro draws for `ch`: the declared family first, then
/// the profile fallback families when atlases are installed, then the
/// missing-glyph square and the space. `None` leaves nothing to draw.
fn resolve_draw_char(
    atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
    family: Option<&str>,
    ch: char,
) -> Option<char> {
    let family = family?;
    let fallbacks: &[&str] = if atlases.is_some() {
        crate::sdf::atlas::PROFILE_TEXT_FALLBACK_FONT_FAMILIES
    } else {
        &[]
    };
    let face = |index: usize| {
        if index == 0 {
            family
        } else {
            fallbacks[index - 1]
        }
    };
    glyph::resolve_glyph(ch, 1 + fallbacks.len(), |index, candidate| {
        let face = face(index);
        atlases
            .and_then(|set| set.atlas_for_font_family(face))
            .is_some_and(|(_, atlas)| atlas.glyph(u32::from(candidate)).is_some())
            || sdf_outline::glyph_advance_x(Some(face), candidate).is_some()
    })
    .map(|choice| choice.glyph)
}

/// Caret advance of `ch`, drawn as `display`, at `size`.
fn layout_advance(
    atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
    family: Option<&str>,
    ch: char,
    display: char,
    size: f32,
) -> f32 {
    if !glyph::advances_caret(ch) {
        return 0.0;
    }
    resolve_draw_char(atlases, family, display)
        .and_then(|glyph| freetype_advance_x(atlases, family, glyph, glyph, size))
        .unwrap_or(0.0)
}

/// Vertex alpha: the markup alpha caps the element alpha.
fn effective_vertex_alpha_u8(markup_alpha: u8, base_alpha_u8: u8) -> u8 {
    markup_alpha.min(base_alpha_u8)
}

#[cfg_attr(not(test), allow(dead_code))]
fn effective_vertex_alpha(markup_alpha: u8, base_alpha_u8: u8) -> f32 {
    effective_vertex_alpha_u8(markup_alpha, base_alpha_u8) as f32 / 255.0
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
    let segments = tmp_text::parse_segments(&text.text, text.size);
    let percent = tmp_layout::uniform_line_indent_percent(&segments)?;
    Some(sekai_profile_renderer_core::LineIndentSource {
        percent,
        line_advances_tmp: measure_line_advances_tmp(text, md, &segments, atlases)?,
        rotation_deg: 0.0,
        scale_x: 1.0,
        alignment: (text.text_type & 0x07) as u8,
    })
}

fn measure_line_advances_tmp(
    text: &TextElement,
    md: &MasterData,
    segments: &[TextSegment],
    atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
) -> Option<Vec<Vec<f32>>> {
    // TMP's preferred width, which the line-indent feedback reads, ignores
    // `<scale>`; only the drawn advance is stretched.
    let units = measure_text_units_tmp(text, md, segments, atlases, false);
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

/// One TMP-unit advance per visible character, in the order
/// [`tmp_text::visible_scalars`] reports them. A caret advance is carried by
/// the character that follows it. `scaled` applies `<scale>` to the advances.
fn measure_text_units_tmp(
    text: &TextElement,
    md: &MasterData,
    segments: &[TextSegment],
    atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
    scaled: bool,
) -> Vec<sekai_profile_renderer_core::MeasuredTextUnit> {
    let family = md.resolve_font(text.font_id);
    let mut units = Vec::new();
    let mut pending_advance = 0.0f32;
    for seg in segments {
        match seg.caret {
            Some(CaretCommand::Advance(advance)) => {
                pending_advance += advance;
                continue;
            }
            Some(CaretCommand::MoveTo(_)) => continue,
            None => {}
        }
        let measure_size = seg.render_size();
        let seg_scale = if scaled {
            seg.scale.unwrap_or(1.0)
        } else {
            1.0
        };
        for ch in seg.text.chars() {
            if ch == '\n' {
                units.push(sekai_profile_renderer_core::MeasuredTextUnit {
                    advance: 0.0,
                    hard_break: true,
                });
                pending_advance = 0.0;
                continue;
            }
            let (display, char_scale) = seg.transform_char(ch);
            let advance = layout_advance(
                atlases,
                family.as_deref(),
                ch,
                display,
                measure_size * char_scale,
            );
            units.push(sekai_profile_renderer_core::MeasuredTextUnit {
                advance: advance * seg_scale * TEXT_SCALE + seg.character_spacing + pending_advance,
                hard_break: false,
            });
            pending_advance = 0.0;
        }
    }
    units
}

pub fn wrap_rich_text_to_width(
    text: &TextElement,
    md: &MasterData,
    max_width: f32,
) -> Option<String> {
    let segments = tmp_text::parse_segments(&text.text, text.size);
    let units = measure_text_units_tmp(text, md, &segments, None, true);
    sekai_profile_renderer_core::wrap_tmp_markup(&text.text, &units, max_width).ok()
}

pub(crate) fn wrap_rich_text_to_width_with_atlases(
    text: &TextElement,
    md: &MasterData,
    max_width: f32,
    atlases: &crate::sdf::atlas::MappedSdfAtlasSet,
) -> Option<String> {
    let segments = tmp_text::parse_segments(&text.text, text.size);
    let units = measure_text_units_tmp(text, md, &segments, Some(atlases), true);
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
    UnsupportedFeature { feature: &'static str },
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
/// 复合顺序对齐 TMP 的 FX 矩阵（`v' = C + M·(v−C)`，`M = Rotate·Scale`）：
/// 绕 glyph center（= anchor）施加 **R 外层、S 内层**，italic skew 最内层（TMP
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
        // italic skew 最内层：TMP 在 FX 矩阵前先改顶点。
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
            face_color,
            op.sdf_params.as_ref(),
        ),
    })
}

/// A completed TMP-compatible layout: the glyph operations and the timings of
/// the phases that produced them. Rendering and capture both consume this one
/// stream.
struct TextLayoutRun {
    font_family: Option<String>,
    draw_ops: Vec<DrawCharOp>,
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
    let mut run = match layout_text_ops(text, md, None, atlases, None, true) {
        Ok(run) => run,
        Err(error) => {
            observer(Err(error));
            return TextSdfCaptureTimings::default();
        }
    };
    let emit_started = Some(std::time::Instant::now());
    for op in &run.draw_ops {
        if !op.ch.chars().any(glyph::is_drawn) {
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
    let mut run = match layout_text_ops(text, md, Some(placement), atlases, outline_override, true)
    {
        Ok(run) => run,
        Err(error) => {
            observer(Err(error));
            return TextSdfCaptureTimings::default();
        }
    };
    let emit_started = Some(std::time::Instant::now());
    for op in &run.draw_ops {
        if !op.ch.chars().any(glyph::is_drawn) {
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

/// The underlay every glyph of a text element carries. Card fonts render with
/// `UNDERLAY_ON`, so it is present at any outline size, including zero, where
/// it shares the face edge.
fn resolve_outline_params(
    outline_override: Option<TextOutlineOverride>,
    text: &TextElement,
    md: &MasterData,
) -> Option<crate::sdf::material::SdfOutlineParams> {
    if let Some(outline) = outline_override {
        return Some(crate::sdf::material::SdfOutlineParams {
            outline_r: outline.rgba[0],
            outline_g: outline.rgba[1],
            outline_b: outline.rgba[2],
            outline_a: outline.rgba[3],
            outline_size: outline.size,
        });
    }
    md.resolve_color(text.outline_color_id)
        .map(|oc| crate::sdf::material::SdfOutlineParams {
            outline_r: oc.r as f32 / 255.0,
            outline_g: oc.g as f32 / 255.0,
            outline_b: oc.b as f32 / 255.0,
            outline_a: oc.a as f32 / 255.0,
            outline_size: text.outline_size,
        })
}

/// Reject decoration spans that cannot be represented by SDF glyph commands.
/// The parsed spans distinguish active markup from literal text and empty tags.
fn validate_sdf_text_segments(segments: &[TextSegment]) -> Result<(), TextSdfCaptureError> {
    for segment in segments {
        if !segment.text.chars().any(|ch| ch != '\n') {
            continue;
        }
        let feature = if segment.mark.is_some() {
            Some("text mark")
        } else if segment.underline {
            Some("text underline")
        } else if segment.strikethrough {
            Some("text strikethrough")
        } else {
            None
        };
        if let Some(feature) = feature {
            return Err(TextSdfCaptureError::UnsupportedFeature { feature });
        }
    }
    Ok(())
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
) -> Result<TextLayoutRun, TextSdfCaptureError> {
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

    let segments = tmp_text::parse_segments(&text.text, text.size);
    validate_sdf_text_segments(&segments)?;
    let lines = tmp_text::split_lines(&segments);
    let debug_probe = debug_text_probe_enabled();
    tracing::debug!(
        font_id = text.font_id,
        color_id = text.color_id,
        size = text.size,
        seg_count = segments.len(),
        line_count = lines.len(),
        raw_len = text.text.len(),
        raw_text = %text.text.chars().take(80).collect::<String>(),
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
        return Ok(TextLayoutRun {
            font_family: resolved_name,
            draw_ops: Vec::new(),
            timings: capture_timings,
        });
    }

    let base_size = text.size;
    capture_timings.font_resolve_ns = capture_elapsed_ns(font_resolve_started);

    let layout_setup_started = capture_timing_enabled.then(std::time::Instant::now);
    let face = PROFILE_FACE;
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
        .map_or(base_size, |segment| segment.font_size);

    let mut line_widths: Vec<f32> = Vec::with_capacity(lines.len());
    let mut rect_widths: Vec<f32> = Vec::with_capacity(lines.len());
    let mut line_max_sizes: Vec<f32> = Vec::with_capacity(lines.len());
    let mut tmp_line_probes: Vec<TmpDebugLineProbe> = Vec::new();
    let mut tmp_char_probes: Vec<TmpDebugCharProbe> = Vec::new();

    // 独立 caret 链：追踪 TMP 真实 xAdvance（乘 scale），与 CPV preferredWidth 链分离。
    let mut final_caret_xadv_tmp = 0.0f32;
    // vertical bounds 追踪：voffset 偏移后每个字形的上下极值（TMP 单位）。
    let mut vbounds_max_top_tmp = f32::NEG_INFINITY;
    let mut vbounds_min_bottom_tmp = f32::INFINITY;
    capture_timings.layout_setup_ns = capture_elapsed_ns(layout_setup_started);

    let measure_started = capture_timing_enabled.then(std::time::Instant::now);
    for (line_idx, line) in lines.iter().enumerate() {
        let mut w_scaled = 0.0f32;
        let mut max_seg_size = 0.0f32;
        let mut prev_cspace: Option<f32> = None;
        let mut cpv_xadv_tmp = 0.0f32;
        let mut max_cpv_width_tmp = 0.0f32;
        let mut has_chars = false;
        // caret 链：<scale> 影响字符前进，与 CPV width 链独立。
        let mut caret_xadv_tmp = 0.0f32;
        let mut line_text = String::new();

        for piece in line {
            let seg = piece.segment;
            match seg.caret {
                Some(CaretCommand::Advance(advance)) => {
                    w_scaled += advance / TEXT_SCALE;
                    cpv_xadv_tmp += advance;
                    caret_xadv_tmp += advance;
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
                    // TMP 在 <pos> 处重置 preferredWidth 追踪；百分比在此阶段没有容器宽度。
                    let shift = tmp_layout::offset_draw_units(offset, 0.0) * TEXT_SCALE;
                    cpv_xadv_tmp = shift;
                    caret_xadv_tmp = shift;
                    max_cpv_width_tmp = 0.0;
                    continue;
                }
                None => {}
            }

            let measure_size = seg.render_size();
            let cspace_raw_tmp = seg.character_spacing;
            let seg_scale = seg.scale.unwrap_or(1.0);
            // voffset 用于 vertical bounds 追踪（TMP 单位，Y-up）。
            let voffset_tmp = seg.baseline_offset;
            let glyph_asc_tmp = face.font_scale(measure_size) * face.ascent_line;
            let glyph_des_tmp = -face.font_scale(measure_size) * face.descent_line;
            let mut measured = 0.0f32;
            let mut n_chars = 0usize;
            for ch in piece.text.chars() {
                let (display, char_scale) = seg.transform_char(ch);
                // Advances come from FreeType only: it is the engine TMP itself
                // uses, and it reports the true subpixel advance. Codepoints the
                // declared atlas lacks are pre-warmed into the fallback atlas
                // before rendering, so the atlas is authoritative here and
                // on-demand generation stays off the request path.
                let glyph_hadv_tmp_layout = layout_advance(
                    capture_atlases,
                    resolved_name_ref,
                    ch,
                    display,
                    measure_size * char_scale,
                ) * TEXT_SCALE;
                measured += glyph_hadv_tmp_layout * seg_scale / TEXT_SCALE;
                max_cpv_width_tmp = tmp_layout::extend_preferred_width_for_char(
                    max_cpv_width_tmp,
                    cpv_xadv_tmp,
                    glyph_hadv_tmp_layout,
                    ch,
                );
                // caret 链：字符前进乘以 scale。
                let glyph_hadv_tmp_caret = glyph_hadv_tmp_layout * seg_scale;
                vbounds_max_top_tmp = vbounds_max_top_tmp.max(voffset_tmp + glyph_asc_tmp);
                vbounds_min_bottom_tmp = vbounds_min_bottom_tmp.min(voffset_tmp - glyph_des_tmp);
                if debug_probe {
                    let before = cpv_xadv_tmp;
                    let after = cpv_xadv_tmp + glyph_hadv_tmp_layout + cspace_raw_tmp;
                    tmp_char_probes.push(TmpDebugCharProbe {
                        line_index: line_idx,
                        ch: display.to_string(),
                        seg_size_tmp: measure_size,
                        seg_scale,
                        char_scale,
                        baseline_offset_tmp: voffset_tmp,
                        x_advance_before_tmp: before,
                        glyph_advance_tmp_for_layout: glyph_hadv_tmp_layout,
                        glyph_advance_tmp_for_caret: glyph_hadv_tmp_caret,
                        x_advance_after_tmp: after,
                        preferred_width_candidate_tmp: before.abs() + glyph_hadv_tmp_layout,
                    });
                }
                cpv_xadv_tmp += glyph_hadv_tmp_layout + cspace_raw_tmp;
                caret_xadv_tmp += glyph_hadv_tmp_caret + cspace_raw_tmp;
                n_chars += 1;
            }
            let cspace = cspace_raw_tmp / TEXT_SCALE;
            w_scaled += measured + cspace * n_chars as f32;
            has_chars = true;
            prev_cspace = Some(cspace);
            max_seg_size = max_seg_size.max(seg.font_size);
            if debug_probe {
                line_text.push_str(piece.text);
            }
        }

        if max_seg_size < 0.001 {
            max_seg_size = line_feed_sizes
                .get(line_idx)
                .or_else(|| {
                    line_idx
                        .checked_sub(1)
                        .and_then(|prev| line_feed_sizes.get(prev))
                })
                .copied()
                .unwrap_or(first_size);
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
                text: line_text,
                line_width_tmp_like: w_scaled * TEXT_SCALE,
                preferred_width_tmp: max_cpv_width_tmp,
                max_seg_size_tmp: max_seg_size,
                line_offset_tmp: 0.0,
                line_height_tmp: max_seg_size * TEXT_SCALE,
            });
        }
    }

    let base_line_h = text.size;
    let lh_override: Option<f32> = segments.iter().find_map(|segment| segment.line_height);
    let n_lines = line_max_sizes.len();

    // TMP lineGap = m_LineHeight - (ascentLine - descentLine) + 0.625；0.625 是
    // 与游戏行距对齐的经验修正项。
    let line_gap = face.line_height - (face.ascent_line - face.descent_line) + 0.625;

    let mut line_asc: Vec<f32> = Vec::with_capacity(n_lines);
    let mut line_des: Vec<f32> = Vec::with_capacity(n_lines);
    for i in 0..n_lines {
        let ms = line_max_sizes[i];
        let es = face.font_scale(ms);
        let asc = es * face.ascent_line;
        let des = es * face.descent_line;
        if i == 0 || ms > 0.001 {
            line_asc.push(asc);
            line_des.push(des);
        } else {
            line_asc.push(line_asc[i - 1]);
            line_des.push(line_des[i - 1]);
        }
    }

    let mut line_offsets = vec![0.0f32; n_lines];
    let ls_tmp = tmp_text::face::line_spacing_offset(
        text.line_spacing,
        base_line_h,
        &face,
        DEFAULT_LINE_SPACING_FACTOR,
    );
    for i in 1..n_lines {
        let delta = if let Some(lh) = lh_override {
            lh + ls_tmp
        } else {
            let asc_new = line_asc[i];
            let des_prev = line_des[i - 1];
            let base_scale = face.font_scale(base_line_h);
            asc_new + des_prev.abs() + line_gap * base_scale + ls_tmp
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
    let anchor_base = (effective_max_asc + effective_min_des) / (2.0 * TEXT_SCALE);
    let underlay = resolve_outline_params(outline_override, text, md);
    let max_rw = rect_widths.iter().cloned().fold(0.0f32, f32::max);
    const PAD_ORIGINAL: f32 = 64.0 / TEXT_SCALE;
    let box_w = max_rw + PAD_ORIGINAL;

    let any_italic = segments.iter().any(|seg| seg.italic.is_some());
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
    let mut draw_ops = Vec::new();
    let mut line_op_ranges = Vec::with_capacity(lines.len());
    let default_align = segments.first().and_then(|segment| segment.align);

    for (i, line) in lines.iter().enumerate() {
        let sw = line_widths[i];
        let first_text = line
            .iter()
            .find(|piece| piece.segment.caret.is_none())
            .map(|piece| piece.segment);
        let line_align = first_text
            .and_then(|segment| segment.align)
            .or(default_align);
        // Justified and flush lines are not stretched; they start at the left.
        let effective_align = match line_align {
            Some(InlineAlign::Left | InlineAlign::Justified | InlineAlign::Flush) => 1,
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
        let mut cursor_x = tmp_layout::line_start(lx, sw, max_rw, effective_align, first_text);

        let ops_start = draw_ops.len();
        for piece in line {
            let seg = piece.segment;
            match seg.caret {
                Some(CaretCommand::Advance(advance)) => {
                    cursor_x += advance / TEXT_SCALE;
                    continue;
                }
                Some(CaretCommand::MoveTo(offset)) => {
                    cursor_x = lx + tmp_layout::offset_draw_units(offset, box_w);
                    continue;
                }
                None => {}
            }

            let render_size = seg.render_size();
            let seg_scale = seg.scale.unwrap_or(1.0);
            // TMP 基线偏移 Y-up；绘制空间 Y-down。
            let baseline_shift = -seg.baseline_offset / TEXT_SCALE;
            let cspace_px = seg.character_spacing / TEXT_SCALE;
            let [sr, sg, sb] = seg.color.unwrap_or([def_color.r, def_color.g, def_color.b]);
            let sa_u8 = effective_vertex_alpha_u8(seg.alpha, def_color.a);
            let italic_slope = seg.italic.map(|angle| angle as f32 * 0.01);
            let mono_cell = seg.monospace.map_or(0.0, |cell| cell / TEXT_SCALE);

            for ch in piece.text.chars() {
                let (display, char_scale) = seg.transform_char(ch);
                // <smallcaps> scales the whole glyph, not just its width.
                let glyph_size = render_size * char_scale;
                let Some(glyph_char) =
                    resolve_draw_char(capture_atlases, resolved_name_ref, display)
                else {
                    continue;
                };
                // 查询 SDF glyph，获取 FreeType 度量（与 TMP FontEngine 同源，NO_HINTING）
                let sdf_glyph = if capture_atlases.is_some() {
                    None
                } else {
                    lookup_or_generate(resolved_name_ref, glyph_char)
                };
                let atlas_metrics =
                    atlas_layout_glyph_metrics(capture_atlases, resolved_name_ref, glyph_char);
                let ft_scale = atlas_metrics.map_or_else(
                    || glyph_size / sdf_outline::sampling_point_size(),
                    |metrics| glyph_size / metrics.point_size,
                );
                let ft_advance_x = atlas_metrics
                    .map(|metrics| metrics.advance_x * ft_scale)
                    .or_else(|| sdf_glyph.as_ref().map(|g| g.plane_advance_x() * ft_scale))
                    .or_else(|| {
                        // Outline-free glyphs (the space) carry an hmtx advance
                        // but cannot produce an SDF.
                        sdf_outline::glyph_advance_x(resolved_name_ref, glyph_char)
                            .map(|advance| advance * ft_scale)
                    });
                let ft_pivot_x = atlas_metrics
                    .map(|metrics| (metrics.bearing_x + metrics.width / 2.0) * ft_scale)
                    .or_else(|| {
                        sdf_glyph
                            .as_ref()
                            .map(|g| (g.plane_bearing_x() + g.plane_width() / 2.0) * ft_scale)
                    });
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
                // TMP italic：顶点按 slope = angle / 100 剪切，零剪切线在
                // midPoint = height/2 + spread 处。skew 绕墨迹中心施加，故中心需额外平移
                // shear_cx = slope * (bearingY - height - spread) * ft_scale。
                let shear_cx = match italic_slope {
                    Some(slope) => {
                        if let Some(metrics) = atlas_metrics {
                            slope * (metrics.bearing_y - metrics.height - metrics.spread) * ft_scale
                        } else if let Some(g) = sdf_glyph.as_ref() {
                            let spread = sdf_outline::sampling_spread();
                            slope * (g.plane_bearing_y() - g.plane_height() - spread) * ft_scale
                        } else {
                            0.0
                        }
                    }
                    None => 0.0,
                };
                let draw_x = if mono_cell > 0.0 {
                    // pivot_x is the FreeType glyph-ink centre.
                    cursor_x + mono_cell / 2.0 - pivot_x
                } else {
                    cursor_x
                };

                draw_ops.push(DrawCharOp {
                    ch: glyph_char.to_string(),
                    x: draw_x,
                    y: ly + baseline_shift,
                    pivot_x,
                    pivot_y,
                    half_w,
                    half_h,
                    shear_cx,
                    scale_x: seg_scale,
                    skew_x: italic_slope.map_or(0.0, |slope| -slope),
                    rotate_deg: seg.rotate.unwrap_or(0.0),
                    font_size: glyph_size,
                    face: [sr as f32 / 255.0, sg as f32 / 255.0, sb as f32 / 255.0, 1.0],
                    sdf_params: underlay,
                    mesh_carrier: crate::sdf::material::runtime_like_mesh_carrier(
                        glyph_size, seg.bold, sa_u8,
                    ),
                });

                if mono_cell > 0.0 {
                    cursor_x += mono_cell + cspace_px;
                } else {
                    // Cursor advance comes from FreeType, the same engine TMP uses.
                    let adv = if glyph::advances_caret(ch) {
                        ft_advance_x.unwrap_or(0.0)
                    } else {
                        0.0
                    };
                    cursor_x += adv * seg_scale + cspace_px;
                }
            }
        }
        line_op_ranges.push(ops_start..draw_ops.len());
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
        // 输出每个字符的最终绘制中心坐标（TMP 等效坐标系：乘以 TEXT_SCALE），
        // 与 TMP characterInfo 顶点中心同语义；行与行之间插入 \n 占位，其 center=(0,0)。
        let char_positions: Vec<(String, f32, f32, f32, f32, f32)> = {
            let mut positions = Vec::new();
            for (line_index, range) in line_op_ranges.iter().enumerate() {
                if line_index > 0 {
                    positions.push(("\\n".to_string(), 0.0, 0.0, 1.0, 0.0, 0.0));
                }
                for op in &draw_ops[range.clone()] {
                    let cx = (op.x + op.pivot_x + op.shear_cx) * TEXT_SCALE;
                    let cy = -(op.y + op.pivot_y) * TEXT_SCALE;
                    positions.push((op.ch.clone(), cx, cy, op.scale_x, op.skew_x, op.pivot_x));
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
        // 变换后字形 footprint 四角（[TL,TR,BR,BL]），用于剪切/尺寸的顶点级回归。
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
    Ok(TextLayoutRun {
        font_family: resolved_name,
        draw_ops,
        timings: capture_timings,
    })
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

#[cfg(test)]
mod tests {
    use super::effective_vertex_alpha;
    use sekai_profile_renderer_core::tmp_text;

    /// Metrics come from DejaVu Sans, the one face the build container ships;
    /// these layout checks skip when it is not installed.
    const LAYOUT_TEST_FAMILY: &str = "DejaVu Sans";

    struct LayoutTestFont;
    impl crate::masterdata::MasterDataProvider for LayoutTestFont {
        fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> {
            None
        }
        fn get_card(&self, _: i32) -> Option<crate::types::CardEntry> {
            None
        }
        fn resolve_color(&self, _: i32) -> Option<crate::masterdata::ResolvedColor> {
            Some(crate::masterdata::ResolvedColor {
                r: 10,
                g: 20,
                b: 30,
                a: 255,
            })
        }
        fn resolve_font(&self, _: i32) -> Option<String> {
            Some(LAYOUT_TEST_FAMILY.into())
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
        fn resolve_unit_vs_sd(&self, id: i32, _: i32) -> i32 {
            id
        }
        fn font_count(&self) -> usize {
            1
        }
        fn color_count(&self) -> usize {
            1
        }
    }

    fn layout_test_md() -> Option<crate::masterdata::MasterData> {
        if crate::sdf::outline::load_font_bytes_for_family(LAYOUT_TEST_FAMILY).is_none() {
            eprintln!("skipping: {LAYOUT_TEST_FAMILY} is not installed");
            return None;
        }
        Some(crate::masterdata::MasterData::new(std::sync::Arc::new(
            LayoutTestFont,
        )))
    }

    fn layout_test_element(
        text: &str,
        size: f32,
        line_spacing: f32,
        text_type: i32,
    ) -> crate::types::TextElement {
        serde_json::from_value(serde_json::json!({
            "objectData": {
                "layer": 0, "lock": false, "visible": true,
                "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
            },
            "colorId": 1, "fontId": 1, "lineSpacing": line_spacing,
            "outlineColorId": 1, "outlineSize": 0.0, "size": size,
            "text": text, "type": text_type
        }))
        .expect("layout test element")
    }

    fn layout_ops(
        text: &str,
        size: f32,
        line_spacing: f32,
        text_type: i32,
    ) -> Option<Vec<super::DrawCharOp>> {
        let md = layout_test_md()?;
        let element = layout_test_element(text, size, line_spacing, text_type);
        Some(
            super::layout_text_ops(&element, &md, None, None, None, false)
                .expect("text layout")
                .draw_ops,
        )
    }

    fn test_advance(ch: char, size: f32) -> f32 {
        super::freetype_advance_x(None, Some(LAYOUT_TEST_FAMILY), ch, ch, size)
            .expect("test font advance")
    }

    fn assert_close(actual: f32, expected: f32, what: &str) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "{what}: {actual} != {expected}"
        );
    }

    #[test]
    fn superscript_rises_and_subscript_drops_by_the_face_offsets() {
        let Some(ops) = layout_ops("A<sup>B</sup>", 30.0, 0.0, 1) else {
            return;
        };
        // 66 design units at 30 / 75 * 2 per unit, half size, in draw space.
        assert_close(
            ops[0].y - ops[1].y,
            66.0 * 0.8 * 0.5 / 2.0,
            "superscript rise",
        );
        assert_close(ops[1].font_size, 15.0, "superscript size");
        let ops = layout_ops("A<sub>B</sub>", 30.0, 0.0, 1).expect("test font");
        assert_close(ops[1].y - ops[0].y, 9.0 * 0.8 * 0.5 / 2.0, "subscript drop");
    }

    #[test]
    fn smallcaps_shrink_the_glyph_on_both_axes() {
        let Some(ops) = layout_ops("<smallcaps>a</smallcaps>", 30.0, 0.0, 1) else {
            return;
        };
        assert_eq!(ops[0].ch, "A");
        assert_close(ops[0].font_size, 24.0, "small caps size");
        assert_close(ops[0].scale_x, 1.0, "small caps horizontal scale");
    }

    #[test]
    fn italic_slant_matches_its_shear_centre() {
        let Some(ops) = layout_ops("<i>A</i><i angle=20>B</i>", 30.0, 0.0, 1) else {
            return;
        };
        assert_close(ops[0].skew_x, -0.35, "default italic slant");
        assert_close(ops[1].skew_x, -0.20, "italic angle attribute");
    }

    #[test]
    fn closing_cspace_takes_the_spacing_back_from_the_last_character() {
        let Some(ops) = layout_ops("<cspace=10>AB</cspace>C", 30.0, 0.0, 1) else {
            return;
        };
        assert_close(
            ops[1].x - ops[0].x,
            test_advance('A', 30.0) + 5.0,
            "spacing inside",
        );
        assert_close(
            ops[2].x - ops[1].x,
            test_advance('B', 30.0),
            "spacing taken back",
        );
    }

    #[test]
    fn space_tags_move_the_caret_on_their_own_line_only() {
        let Some(ops) = layout_ops("A<space=50>B", 30.0, 0.0, 1) else {
            return;
        };
        assert_close(
            ops[1].x - ops[0].x,
            test_advance('A', 30.0) + 25.0,
            "space advance",
        );
        // Centred: the second line holds no space and centres on its glyph.
        let ops = layout_ops("<space=50>A\nA", 30.0, 0.0, 2).expect("test font");
        assert_close(
            ops[1].x,
            -test_advance('A', 30.0) / 2.0,
            "second line start",
        );
    }

    #[test]
    fn line_indent_pixels_are_layout_units() {
        let Some(plain) = layout_ops("A", 30.0, 0.0, 1) else {
            return;
        };
        let indented = layout_ops("<line-indent=20>A", 30.0, 0.0, 1).expect("test font");
        assert_close(indented[0].x - plain[0].x, 10.0, "line indent");
        let em = layout_ops("<indent=10><line-indent=1em>A", 30.0, 0.0, 1).expect("test font");
        assert_close(em[0].x - plain[0].x, 20.0, "indent plus em line indent");
    }

    #[test]
    fn line_spacing_uses_the_game_factor() {
        let Some(tight) = layout_ops("A\nA", 300.0, 0.0, 1) else {
            return;
        };
        let spaced = layout_ops("A\nA", 300.0, 1.0, 1).expect("test font");
        let gap = |ops: &[super::DrawCharOp]| ops[1].y - ops[0].y;
        // One unit of spacing is 2 * 1.325 * 300 / 100 layout units.
        assert_close(
            gap(&spaced) - gap(&tight),
            2.0 * 1.325 * 3.0 / 2.0,
            "line spacing",
        );
    }

    #[test]
    fn a_character_no_font_has_is_drawn_as_the_missing_glyph_square() {
        let Some(ops) = layout_ops("A\u{6F22}B", 30.0, 0.0, 1) else {
            return;
        };
        assert_eq!(ops[1].ch, "\u{25A1}");
        assert_close(
            ops[2].x - ops[0].x,
            test_advance('A', 30.0) + test_advance('\u{25A1}', 30.0),
            "square advance",
        );
        let md = layout_test_md().expect("test font");
        let element = layout_test_element("A\u{6F22}B A\u{6F22}B", 30.0, 0.0, 1);
        let wrapped = super::wrap_rich_text_to_width(&element, &md, 90.0).expect("wrapped text");
        assert!(wrapped.contains('\n'), "{wrapped:?}");
    }

    #[test]
    fn unrecognised_tags_are_laid_out_as_text() {
        let Some(ops) = layout_ops("<love>", 30.0, 0.0, 1) else {
            return;
        };
        let text: String = ops.iter().map(|op| op.ch.as_str()).collect();
        assert_eq!(text, "<love>");
    }

    #[test]
    fn sdf_capture_rejects_decoration_spans_without_emitting_partial_glyphs() {
        use crate::masterdata::{MasterData, MasterDataProvider};
        use std::sync::Arc;

        struct NoFonts;
        impl MasterDataProvider for NoFonts {
            fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> {
                None
            }
            fn get_card(&self, _: i32) -> Option<crate::types::CardEntry> {
                None
            }
            fn resolve_color(&self, _: i32) -> Option<crate::masterdata::ResolvedColor> {
                None
            }
            fn resolve_font(&self, _: i32) -> Option<String> {
                panic!("unsupported decoration must be rejected before font lookup");
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
            fn resolve_unit_vs_sd(&self, id: i32, _: i32) -> i32 {
                id
            }
            fn font_count(&self) -> usize {
                0
            }
            fn color_count(&self) -> usize {
                0
            }
        }

        let md = MasterData::new(Arc::new(NoFonts));
        for (text, feature) in [
            ("plain <u>underlined</u>", "text underline"),
            ("plain <s>struck</s>", "text strikethrough"),
            ("plain <mark=#ff0000>marked</mark>", "text mark"),
            ("<u> </u>", "text underline"),
        ] {
            let element = serde_json::from_value(serde_json::json!({
                "objectData": {
                    "layer": 0, "lock": false, "visible": true,
                    "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                    "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
                },
                "colorId": 1, "fontId": 1, "lineSpacing": 0.0,
                "outlineColorId": 1, "outlineSize": 0.0, "size": 24.0,
                "text": text, "type": 1
            }))
            .unwrap();
            for placed in [false, true] {
                let mut events = Vec::new();
                let mut observer = |event| events.push(event);
                let affine = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                if placed {
                    super::capture_text_sdf_with_placement(
                        affine,
                        &element,
                        &md,
                        None,
                        super::TextRenderPlacement {
                            anchor_x: 0.0,
                            baseline: None,
                        },
                        None,
                        &mut observer,
                    );
                } else {
                    super::capture_text_sdf_from_affine(affine, &element, &md, None, &mut observer);
                }
                assert_eq!(events.len(), 1, "text={text}, placed={placed}");
                assert!(matches!(
                    events[0],
                    Err(super::TextSdfCaptureError::UnsupportedFeature { feature: value })
                        if value == feature
                ));
            }
        }
    }

    #[test]
    fn sdf_capture_accepts_literal_markup_empty_decorations_and_supported_styles() {
        for text in [
            "plain text",
            "<b>bold</b><i>italic</i><color=#ff0000>red</color>",
            "<noparse><u>literal</u><s>literal</s><mark=#ff0000>literal</mark></noparse>",
            "<u></u><s></s><mark=#ff0000></mark>plain",
            "<u>\n</u><s>\n</s><mark=#ff0000>\n</mark>",
        ] {
            let segments = tmp_text::parse_segments(text, 24.0);
            assert_eq!(
                super::validate_sdf_text_segments(&segments),
                Ok(()),
                "text={text}"
            );
        }
    }

    #[test]
    fn rich_text_parsing_retains_decoration_semantics_for_non_capture_consumers() {
        let segments = tmp_text::parse_segments("<u>A</u><s>B</s><mark=#ff000040>C</mark>", 24.0);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].text, "A");
        assert!(segments[0].underline);
        assert_eq!(segments[1].text, "B");
        assert!(segments[1].strikethrough);
        assert_eq!(segments[2].text, "C");
        assert_eq!(segments[2].mark, Some([255, 0, 0, 64]));
    }

    /// A capture given the outline as resolved RGBA must produce exactly the
    /// glyph stream the color-table route produces for the same color, and a
    /// zero-width override must keep the underlay on the face edge.
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

        let zero_width = capture(
            &element(0.4),
            Some(super::TextOutlineOverride {
                rgba: [204.0 / 255.0, 51.0 / 255.0, 25.0 / 255.0, 230.0 / 255.0],
                size: 0.0,
            }),
        );
        let plain = capture(&element(0.0), None);
        assert_eq!(
            zero_width, plain,
            "a zero-width override must match the zero-width color-table route"
        );
        assert!(
            plain.iter().all(|glyph| glyph.material.outline[3] > 0.0),
            "a zero-width outline still draws the underlay"
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
        let alpha = effective_vertex_alpha(204, 128);
        assert!((alpha - (128.0 / 255.0)).abs() < 1e-6);
    }

    #[test]
    fn effective_vertex_alpha_uses_override_when_lower_than_base() {
        let alpha = effective_vertex_alpha(64, 255);
        assert!((alpha - (64.0 / 255.0)).abs() < 1e-6);
    }

    #[test]
    fn effective_vertex_alpha_falls_back_to_base_alpha() {
        let alpha = effective_vertex_alpha(255, 64);
        assert!((alpha - (64.0 / 255.0)).abs() < 1e-6);
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
