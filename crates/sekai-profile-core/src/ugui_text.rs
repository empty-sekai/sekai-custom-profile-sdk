//! Glyph layout of the Unity UI `Text` component with a dynamic font.
//!
//! This text is drawn from FreeType coverage bitmaps rather than signed
//! distance fields. The native and browser renderers take its geometry from
//! [`layout`], which reproduces the engine's text generator for the settings
//! the omikuji slip uses: upper-left alignment, lines that overflow the rect in
//! both directions, no best fit and no rich-text tags. Each renderer supplies
//! the glyphs through [`UguiGlyphRasterizer`] and renders them as the engine
//! does: FreeType at a whole pixel size (character size `S * 64` at 72 dpi),
//! unhinted outlines (`FT_LOAD_NO_HINTING`), the Adobe CFF engine without stem
//! darkening, and 8-bit anti-aliased coverage (`FT_RENDER_MODE_NORMAL`).
//!
//! A character comes from the first face of [`UguiTextSource::face_chain`]
//! whose character map has it: the font asset's file, then
//! [`UguiTextSource::fallback_faces`] in order. A glyph from a fallback face
//! keeps that face's bitmap offsets, size and advance; the padding, advance
//! rounding, tracking and line metrics are always the font asset's.
//!
//! Every glyph becomes a quad over its bitmap with
//! [`UguiFontAsset::character_padding`] empty texels on each side, mapped one
//! texel to one pixel. A renderer samples that cell bilinearly
//! ([`cell_coverage`]) and draws the vertex colour times the coverage.
//! Both renderers lay the text out at [`CARD_PIXELS_PER_UNIT`].
//!
//! All arithmetic is `f32` in the generator's order of operations, so quad
//! corners come out bit for bit as the engine computes them.

use std::sync::Arc;

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{Matrix2d, Quad, Rect};

/// Largest pixel size the generator renders glyphs at.
pub const MAX_PIXEL_SIZE: u32 = 500;
/// Output pixels per canvas unit of the card the renderers draw: the card is
/// rendered at its own size, so glyphs are rendered at the font size.
pub const CARD_PIXELS_PER_UNIT: f32 = 1.0;
/// Offset from a texel's corner to its centre, in texels. [`cell_coverage`]
/// subtracts it before interpolating between texel centres.
pub use crate::pixel_sampling::TEXEL_CENTRE;
/// Added to both rect extents before the text is placed in the rect.
const EXTENT_EPSILON: f32 = 0.0001;
/// `cos(90°)` as `f32`: the residue of the quarter turn that sets glyphs
/// upright.
const COS_QUARTER_TURN: f32 = -4.371_139e-8;
/// Degrees to radians, as the vertical-text modifier converts glyph angles.
const DEGREES_TO_RADIANS: f32 = 0.017_453_292;

/// Metrics of a font asset (the asset, not the font file): its line metrics
/// at [`Self::reference_size`] and the glyph-cache settings the generator
/// reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct UguiFontAsset {
    /// Family the font file is requested by.
    pub family: String,
    /// Size the line metrics below are given at.
    pub reference_size: f32,
    pub ascent: f32,
    pub descent: f32,
    pub line_spacing: f32,
    /// Empty texels kept around each glyph bitmap.
    pub character_padding: u32,
    /// Factor applied to every advance.
    pub tracking: f32,
    /// Advances are rounded to whole pixels when a glyph is cached.
    pub round_advance: bool,
}

/// A face of a font file: the file is requested by `family`, and
/// `face_index` picks the face in it (0 for a file that is not a
/// collection).
#[derive(
    Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Encode, Decode,
)]
pub struct UguiFontFace {
    pub family: String,
    pub face_index: u32,
}

/// A glyph the vertical-text modifier turns and moves after setting the text
/// upright.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct VerticalGlyphOffset {
    pub target: char,
    /// Added to the quad after the turn, in node units (+y up).
    pub offset: [f32; 2],
    /// Counter-clockwise turn about the quad centre.
    pub angle_degrees: f32,
}

/// The vertical-text mesh modifier. The text node itself is turned a quarter
/// turn clockwise, so its lines run down the page; the modifier turns each
/// kana, kanji and Hangul quad a quarter turn back about its centre, which
/// keeps those characters upright. Every other character keeps its quad and
/// reads sideways. Quads of the characters in [`Self::offsets`] are then
/// turned and moved once more.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct UguiVerticalModifier {
    pub offsets: Vec<VerticalGlyphOffset>,
}

/// A solid rectangle drawn under the text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct UguiTextBackdrop {
    /// Fill colour, straight RGBA in `0..=1`.
    pub color: [f32; 4],
    /// The rectangle in layer space (+y down) as the prefab lays it out.
    pub rect: Rect,
    /// When set, the rectangle keeps its top edge and its height becomes the
    /// text's preferred width plus this padding.
    pub fit_padding: Option<f32>,
}

impl UguiTextBackdrop {
    /// The rectangle drawn for text whose preferred width is
    /// `preferred_width`.
    pub fn resolved_rect(&self, preferred_width: f32) -> Rect {
        match self.fit_padding {
            Some(padding) => Rect {
                height: preferred_width + padding,
                ..self.rect
            },
            None => self.rect,
        }
    }
}

/// One `Text` node: its string, settings and placement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct UguiTextSource {
    /// The string the node displays.
    pub text: String,
    pub font: UguiFontAsset,
    /// Faces tried in order for a character the font asset's file has no
    /// glyph for.
    pub fallback_faces: Vec<UguiFontFace>,
    /// Font size in canvas units.
    pub font_size: i32,
    /// Factor applied to the pitch between lines.
    pub line_spacing: f32,
    /// Vertex colour, straight RGBA in `0..=1`, already quantised to 8 bits.
    pub color: [f32; 4],
    /// Width and height of the node's rect.
    pub rect_size: [f32; 2],
    /// Pivot of the rect, `0..=1` from its bottom-left corner.
    pub pivot: [f32; 2],
    /// Maps node space (origin at the pivot, +y up, canvas units) to the
    /// layer.
    pub node_matrix: Matrix2d,
    #[serde(default)]
    pub vertical: Option<UguiVerticalModifier>,
    /// Drawn before the glyphs.
    #[serde(default)]
    pub backdrop: Option<UguiTextBackdrop>,
}

impl UguiTextSource {
    /// The faces glyphs come from, in the order they are tried: the first
    /// face of the font asset's file, then [`Self::fallback_faces`].
    pub fn face_chain(&self) -> Vec<UguiFontFace> {
        std::iter::once(UguiFontFace {
            family: self.font.family.clone(),
            face_index: 0,
        })
        .chain(self.fallback_faces.iter().cloned())
        .collect()
    }
}

/// A glyph rendered by FreeType at one pixel size.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UguiGlyph {
    pub bitmap_left: i32,
    pub bitmap_top: i32,
    pub width: u32,
    pub rows: u32,
    /// Unhinted horizontal advance, 26.6 fixed point.
    pub advance_26_6: i64,
    /// Coverage, `rows` rows of `width` bytes, top row first.
    pub coverage: Vec<u8>,
}

/// Supplies the glyphs of a text's face chain ([`UguiTextSource::face_chain`])
/// to [`layout`].
pub trait UguiGlyphRasterizer {
    /// `ch` rendered at `pixel_size` as the module documentation describes,
    /// from the first face that has a glyph for it, or `None` when none has.
    fn glyph(&mut self, ch: char, pixel_size: u32) -> Option<Arc<UguiGlyph>>;
}

/// One glyph quad of a laid-out text.
#[derive(Clone, Debug, PartialEq)]
pub struct UguiGlyphQuad {
    pub ch: char,
    /// Node-space corners (origin at the pivot, +y up, canvas units) of the
    /// glyph cell's top-left, top-right, bottom-right and bottom-left corners.
    pub corners: Quad,
    /// The glyph whose bitmap the cell holds; `None` for a character that
    /// shows nothing: a control character, or one the font does not cover.
    pub glyph: Option<Arc<UguiGlyph>>,
}

/// Output of [`layout`].
#[derive(Clone, Debug, PartialEq)]
pub struct UguiTextMesh {
    /// Quads in string order: one for every character except U+0020, tab and
    /// line feed.
    pub quads: Vec<UguiGlyphQuad>,
    /// Empty texels around the bitmap in every glyph cell.
    pub cell_padding: u32,
    /// The text's preferred width, its widest line, in canvas units.
    pub preferred_width: f32,
}

/// Pixel size the glyphs of a `font_size` text are rendered at when one
/// canvas unit covers `pixels_per_unit` output pixels.
pub fn pixel_size(font_size: i32, pixels_per_unit: f32) -> u32 {
    let scaled = font_size as f32 * pixels_per_unit;
    if scaled.is_nan() || scaled < 1.0 {
        return 0;
    }
    (scaled as u32).min(MAX_PIXEL_SIZE)
}

/// Coverage of a glyph cell at cell coordinates `(u, v)`: texels from the
/// cell's top-left corner, one texel per bitmap pixel, with `padding` empty
/// texels around the bitmap. The coverage is interpolated bilinearly between
/// the centres of the four nearest texels; texels outside the bitmap are
/// empty.
pub fn cell_coverage(glyph: &UguiGlyph, padding: u32, u: f32, v: f32) -> f32 {
    let texel = |column: i64, row: i64| -> f32 {
        let column = column - i64::from(padding);
        let row = row - i64::from(padding);
        if column < 0 || row < 0 || column >= i64::from(glyph.width) || row >= i64::from(glyph.rows)
        {
            return 0.0;
        }
        f32::from(glyph.coverage[row as usize * glyph.width as usize + column as usize]) / 255.0
    };
    let x = u - TEXEL_CENTRE;
    let y = v - TEXEL_CENTRE;
    let left = x.floor();
    let top = y.floor();
    let fx = x - left;
    let fy = y - top;
    let (column, row) = (left as i64, top as i64);
    let upper = texel(column, row) + (texel(column + 1, row) - texel(column, row)) * fx;
    let lower = texel(column, row + 1) + (texel(column + 1, row + 1) - texel(column, row + 1)) * fx;
    upper + (lower - upper) * fy
}

/// `floor(value + 0.5)`, the generator's rounding.
fn round(value: f32) -> f32 {
    (value + 0.5).floor()
}

/// Cached rect and advance of one character, in pixels.
struct CharacterCell {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    advance: f32,
    glyph: Option<Arc<UguiGlyph>>,
}

fn character_cell(font: &UguiFontAsset, glyph: Option<Arc<UguiGlyph>>) -> CharacterCell {
    let (x, y, width, height, mut advance) = match &glyph {
        Some(glyph) => (
            glyph.bitmap_left as f32,
            glyph.bitmap_top as f32,
            glyph.width as f32,
            -(glyph.rows as f32),
            glyph.advance_26_6 as f32 * 0.015_625,
        ),
        None => (0.0, 0.0, 0.0, 0.0, 0.0),
    };
    if font.round_advance {
        advance = round(advance);
    }
    let padding = font.character_padding as f32;
    CharacterCell {
        x: x - padding,
        y: y + padding,
        width: width + (padding + padding),
        height: height - (padding + padding),
        advance,
        glyph: glyph.filter(|glyph| !glyph.coverage.is_empty()),
    }
}

/// Lays `source` out for a render target with `pixels_per_unit` output
/// pixels per canvas unit.
pub fn layout(
    source: &UguiTextSource,
    pixels_per_unit: f32,
    glyphs: &mut dyn UguiGlyphRasterizer,
) -> UguiTextMesh {
    let font = &source.font;
    let size = pixel_size(source.font_size, pixels_per_unit);
    let mut mesh = UguiTextMesh {
        quads: Vec::new(),
        cell_padding: font.character_padding,
        preferred_width: 0.0,
    };
    if size == 0 {
        return mesh;
    }
    let scale = pixels_per_unit;
    let scaled = |value: f32| (value * size as f32) / font.reference_size;
    let ascent = round(scaled(font.ascent));
    let descent = round(scaled(font.descent));
    let gap = round(scaled(font.line_spacing)) - (ascent - descent);
    // The cached advance of U+0020 advances the pen over a space and sets
    // the tab width.
    let space = character_cell(font, glyphs.glyph(' ', size)).advance * font.tracking;
    let space_advance = round(space);
    let tab_width = if space == 0.0 {
        16
    } else {
        (space * 4.0) as i32
    };

    let mut pen_x = 0.0f32;
    let mut pen_y = 0.0f32;
    let mut line_width = 0.0f32;
    let mut widest_line = 0.0f32;
    let mut min_y = f32::MAX;
    let mut max_y = 0.0f32;
    let mut previous_descent = 0.0f32;
    let mut first_line = true;
    let mut line_start = 0usize;
    // Native quads: corners in pixels, +y down, top-left first.
    let mut native = Vec::<(char, Quad, Option<Arc<UguiGlyph>>)>::new();
    let characters = source.text.chars().map(Some).chain(std::iter::once(None));
    for character in characters {
        match character {
            None | Some('\n') => {
                widest_line = widest_line.max(line_width);
                let advance = if first_line {
                    round(ascent)
                } else {
                    round(source.line_spacing * (gap + (ascent - previous_descent)))
                };
                for (_, corners, _) in &mut native[line_start..] {
                    for corner in corners {
                        corner[1] += advance;
                    }
                }
                pen_y += advance;
                min_y = min_y.min(pen_y - ascent);
                max_y = max_y.max(pen_y - descent);
                previous_descent = descent;
                first_line = false;
                line_start = native.len();
                pen_x = 0.0;
                line_width = 0.0;
            }
            Some(' ') => {
                pen_x += space_advance;
                line_width += space_advance;
            }
            Some('\t') => {
                let stops = (pen_x / tab_width as f32) as i32 + 1;
                pen_x = round((stops * tab_width) as f32);
                line_width = pen_x;
            }
            Some(ch) => {
                // Control characters are never cached: their rect and
                // advance are zero.
                let cell = if u32::from(ch) < 0x20 {
                    CharacterCell {
                        x: 0.0,
                        y: 0.0,
                        width: 0.0,
                        height: 0.0,
                        advance: 0.0,
                        glyph: None,
                    }
                } else {
                    character_cell(font, glyphs.glyph(ch, size))
                };
                let advance = round(cell.advance * font.tracking);
                let (width, height) = if cell.width == 0.0 {
                    (advance, 0.0)
                } else {
                    (cell.width, cell.height)
                };
                let top = (0.5 - cell.y).floor();
                let bottom = (0.5 - (cell.y + height)).floor();
                let right = ((cell.x + width) + 0.5).floor();
                let left = (cell.x + 0.5).floor();
                native.push((
                    ch,
                    [
                        [left + pen_x, top + pen_y],
                        [right + pen_x, top + pen_y],
                        [right + pen_x, bottom + pen_y],
                        [left + pen_x, bottom + pen_y],
                    ],
                    cell.glyph,
                ));
                pen_x += advance;
                line_width += advance;
            }
        }
    }

    let bounds_y = round(min_y);
    let bounds_width = round(widest_line);
    let extent_x = source.rect_size[0] * scale + EXTENT_EPSILON;
    let extent_y = source.rect_size[1] * scale + EXTENT_EPSILON;
    let offset_x = ((0.0 - source.pivot[0] * extent_x) + 0.5).floor();
    let offset_y = ((-bounds_y - (1.0 - source.pivot[1]) * extent_y) + 0.5).floor();
    let units_per_pixel = 1.0 / scale;
    mesh.quads = native
        .into_iter()
        .map(|(ch, corners, glyph)| UguiGlyphQuad {
            ch,
            corners: corners.map(|[x, y]| {
                [
                    (x + offset_x) * units_per_pixel,
                    (-(y + offset_y)) * units_per_pixel,
                ]
            }),
            glyph,
        })
        .collect();
    if let Some(vertical) = &source.vertical {
        set_upright(&mut mesh.quads, &source.text, vertical);
    }
    mesh.preferred_width = bounds_width / scale;
    mesh
}

/// Whether the vertical-text modifier turns `ch` upright: kana, kanji and
/// Hangul syllables.
pub fn turns_upright(ch: char) -> bool {
    matches!(
        u32::from(ch),
        0x3040..=0x309F | 0x30A0..=0x30FF | 0x4E00..=0x9FFF | 0xAC00..=0xD7A3
    )
}

/// Applies the vertical-text modifier. Quad `k` belongs to the `k`-th
/// character of `text` once line feeds, carriage returns and U+0020 are
/// removed; quads past the end of that string are left as they are.
fn set_upright(quads: &mut [UguiGlyphQuad], text: &str, vertical: &UguiVerticalModifier) {
    let characters = text
        .chars()
        .filter(|ch| !matches!(ch, '\n' | '\r' | ' '))
        .collect::<Vec<_>>();
    for (quad, &ch) in quads.iter_mut().zip(&characters) {
        let [first, _, third, _] = quad.corners;
        let centre = [
            first[0] + (third[0] - first[0]) * 0.5,
            first[1] + (third[1] - first[1]) * 0.5,
        ];
        let turn = turns_upright(ch);
        let offset = vertical.offsets.iter().find(|offset| offset.target == ch);
        let trig = offset.map(|offset| {
            let angle = offset.angle_degrees * DEGREES_TO_RADIANS;
            (angle.cos(), angle.sin())
        });
        for corner in &mut quad.corners {
            if turn {
                let dx = corner[0] - centre[0];
                let dy = corner[1] - centre[1];
                corner[0] = centre[0] + (COS_QUARTER_TURN * dx - dy);
                corner[1] = centre[1] + (dx + COS_QUARTER_TURN * dy);
            }
            if let (Some(offset), Some((cos, sin))) = (offset, trig) {
                let dy = corner[1] - centre[1];
                let dx = corner[0] - centre[0];
                corner[0] = (centre[0] + (dx * cos - dy * sin)) + offset.offset[0];
                corner[1] = (centre[1] + (dx * sin + dy * cos)) + offset.offset[1];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FreeType metrics of the few characters the vectors use: pixel size,
    /// character, then bitmap left, bitmap top, width, rows and the 26.6
    /// advance of an unhinted mincho face. Bitmaps are left empty: layout
    /// reads only metrics.
    const METRICS: &[(u32, char, i32, i32, u32, u32, i64)] = &[
        (40, '願', 0, 35, 39, 39, 2560),
        (40, '望', 1, 34, 38, 37, 2560),
        (40, ' ', 0, 0, 0, 0, 768),
        (40, '\u{a0}', 0, 0, 0, 0, 768),
        (36, '夢', 0, 31, 36, 35, 2304),
        (36, 'の', 3, 26, 30, 28, 2304),
        (36, '実', 1, 31, 35, 34, 2304),
        (36, '現', 0, 31, 36, 35, 2304),
        (36, '昔', 1, 31, 34, 34, 2304),
        (36, '縁', 0, 31, 36, 35, 2304),
        (36, ' ', 0, 0, 0, 0, 691),
        (30, '「', 20, 26, 9, 28, 1920),
        (30, 'サ', 1, 24, 28, 27, 1920),
        (30, 'イ', 1, 24, 23, 26, 1920),
        (30, 'ズ', 1, 25, 29, 25, 1920),
        (30, '」', 1, 25, 9, 28, 1920),
        (30, '3', 1, 24, 17, 26, 1190),
        (30, '6', 1, 24, 17, 26, 1190),
        (30, 'P', 0, 24, 21, 25, 1369),
        (30, 't', 0, 20, 12, 22, 741),
        (30, '。', 1, 6, 9, 9, 1920),
        (30, 'ー', 3, 15, 25, 6, 1920),
        (30, ' ', 0, 0, 0, 0, 576),
    ];

    /// Serves [`METRICS`], scaled linearly for other pixel sizes, and records
    /// the sizes it was asked for.
    #[derive(Default)]
    struct MetricTable {
        requested: Vec<(char, u32)>,
    }

    impl UguiGlyphRasterizer for MetricTable {
        fn glyph(&mut self, ch: char, pixel_size: u32) -> Option<Arc<UguiGlyph>> {
            self.requested.push((ch, pixel_size));
            let (size, _, left, top, width, rows, advance) = METRICS
                .iter()
                .copied()
                .find(|entry| entry.1 == ch && entry.0 == pixel_size)
                .or_else(|| {
                    METRICS
                        .iter()
                        .copied()
                        .find(|entry| entry.1 == ch && pixel_size.is_multiple_of(entry.0))
                })?;
            let factor = pixel_size / size;
            Some(Arc::new(UguiGlyph {
                bitmap_left: left * factor as i32,
                bitmap_top: top * factor as i32,
                width: width * factor,
                rows: rows * factor,
                advance_26_6: advance * i64::from(factor),
                coverage: vec![255; (width * rows * factor * factor) as usize],
            }))
        }
    }

    fn mincho(family: &str) -> UguiFontAsset {
        UguiFontAsset {
            family: family.into(),
            reference_size: 16.0,
            ascent: 14.080_001,
            descent: -1.920_000_1,
            line_spacing: 32.0,
            character_padding: 1,
            tracking: 1.0,
            round_advance: true,
        }
    }

    fn slip_offsets() -> Vec<VerticalGlyphOffset> {
        let offset = |target, y, angle_degrees| VerticalGlyphOffset {
            target,
            offset: [0.0, y],
            angle_degrees,
        };
        vec![
            offset('ゃ', 10.0, 0.0),
            offset('ゅ', 10.0, 0.0),
            offset('ょ', 10.0, 0.0),
            offset('っ', 10.0, 0.0),
            offset('、', 22.0, 90.0),
            offset('。', 22.0, 90.0),
            offset('ー', 0.0, 0.0),
            offset('々', 0.0, 90.0),
        ]
    }

    fn source(
        text: &str,
        font_size: i32,
        line_spacing: f32,
        rect_size: [f32; 2],
        pivot: [f32; 2],
    ) -> UguiTextSource {
        UguiTextSource {
            text: text.into(),
            font: mincho("SyntheticMincho"),
            fallback_faces: Vec::new(),
            font_size,
            line_spacing,
            color: [1.0; 4],
            rect_size,
            pivot,
            node_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            vertical: Some(UguiVerticalModifier {
                offsets: slip_offsets(),
            }),
            backdrop: None,
        }
    }

    fn title(text: &str) -> UguiTextSource {
        source(text, 40, 1.0, [100.0, 100.0], [0.0, 0.0])
    }

    fn corners(mesh: &UguiTextMesh) -> Vec<Quad> {
        mesh.quads.iter().map(|quad| quad.corners).collect()
    }

    fn laid_out(source: &UguiTextSource) -> UguiTextMesh {
        layout(source, 1.0, &mut MetricTable::default())
    }

    fn assert_corners(actual: &[Quad], expected: &[Quad]) {
        assert_eq!(actual.len(), expected.len(), "{actual:?}");
        for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            for (corner, (a, e)) in actual.iter().zip(expected).enumerate() {
                assert!(
                    a[0].to_bits() == e[0].to_bits() && a[1].to_bits() == e[1].to_bits(),
                    "quad {index} corner {corner}: {a:?} != {e:?}"
                );
            }
        }
    }

    #[test]
    fn the_quarter_turn_constants_are_the_f32_values_of_cos_90_and_one_degree() {
        assert_eq!(COS_QUARTER_TURN.to_bits(), 0xB33B_BD2E);
        assert_eq!(DEGREES_TO_RADIANS.to_bits(), 0x3C8E_FA35);
        let quarter = 90.0 * DEGREES_TO_RADIANS;
        assert_eq!(quarter.cos(), COS_QUARTER_TURN);
        assert_eq!(quarter.sin(), 1.0);
    }

    #[test]
    fn a_single_line_of_kanji_is_set_upright_in_its_title_rect() {
        let mesh = laid_out(&title("願望"));
        assert_corners(
            &corners(&mesh),
            &[
                [[-1.0, 60.0], [-1.0, 101.0], [40.0, 101.0], [40.0, 60.0]],
                [[40.5, 60.5], [40.5, 100.5], [79.5, 100.5], [79.5, 60.5]],
            ],
        );
        assert_eq!(mesh.preferred_width, 80.0);
        assert_eq!(mesh.cell_padding, 1);
    }

    #[test]
    fn a_space_advances_the_pen_without_a_quad() {
        let mesh = laid_out(&title("願 望"));
        assert_corners(
            &corners(&mesh),
            &[
                [[-1.0, 60.0], [-1.0, 101.0], [40.0, 101.0], [40.0, 60.0]],
                [[52.5, 60.5], [52.5, 100.5], [91.5, 100.5], [91.5, 60.5]],
            ],
        );
        assert_eq!(mesh.preferred_width, 92.0);
    }

    #[test]
    fn a_no_break_space_is_a_blank_two_pixel_quad_that_keeps_its_orientation() {
        let mesh = laid_out(&title("願\u{a0}望"));
        assert_corners(
            &corners(&mesh),
            &[
                [[-1.0, 60.0], [-1.0, 101.0], [40.0, 101.0], [40.0, 60.0]],
                [[39.0, 66.0], [41.0, 66.0], [41.0, 64.0], [39.0, 64.0]],
                [[52.5, 60.5], [52.5, 100.5], [91.5, 100.5], [91.5, 60.5]],
            ],
        );
        assert!(mesh.quads[1].glyph.is_none());
        assert_eq!(mesh.preferred_width, 92.0);
    }

    #[test]
    fn later_lines_advance_by_the_line_spacing_share_of_the_pitch() {
        let mesh = laid_out(&source(
            "夢の実現\n昔の縁",
            36,
            0.62,
            [418.634_28, 134.812_13],
            [0.0, 1.0],
        ));
        assert_corners(
            &corners(&mesh),
            &[
                [[-0.5, -37.5], [-0.5, 0.5], [36.5, 0.5], [36.5, -37.5]],
                [
                    [39.0, -36.0],
                    [39.0, -4.000_001],
                    [69.0, -4.0],
                    [69.0, -36.0],
                ],
                [[72.5, -36.5], [72.5, 0.5], [108.5, 0.5], [108.5, -36.5]],
                [[107.5, -37.5], [107.5, 0.5], [144.5, 0.5], [144.5, -37.5]],
                [[0.0, -81.0], [0.0, -45.0], [36.0, -45.0], [36.0, -81.0]],
                [[39.0, -81.0], [39.0, -49.0], [69.0, -49.0], [69.0, -81.0]],
                [[71.5, -82.5], [71.5, -44.5], [108.5, -44.5], [108.5, -82.5]],
            ],
        );
        assert_eq!(mesh.preferred_width, 144.0);
    }

    #[test]
    fn latin_digits_and_brackets_keep_their_quads_and_listed_marks_turn_and_move() {
        let mesh = laid_out(&source(
            "「サイズ」36Pt。ー",
            30,
            0.62,
            [416.0, 78.0],
            [0.0, 1.0],
        ));
        assert_corners(
            &corners(&mesh),
            &[
                [[19.0, 1.0], [30.0, 1.0], [30.0, -29.0], [19.0, -29.0]],
                [
                    [30.5, -30.5],
                    [30.5, -0.500_000_95],
                    [59.5, -0.499_999_05],
                    [59.5, -30.5],
                ],
                [
                    [58.5, -27.5],
                    [58.5, -2.500_001],
                    [86.5, -2.499_999],
                    [86.5, -27.5],
                ],
                [
                    [92.0, -29.0],
                    [92.0, 1.999_999],
                    [119.0, 2.000_001],
                    [119.0, -29.0],
                ],
                [[120.0, -0.0], [131.0, -0.0], [131.0, -30.0], [120.0, -30.0]],
                [[150.0, -1.0], [169.0, -1.0], [169.0, -29.0], [150.0, -29.0]],
                [[169.0, -1.0], [188.0, -1.0], [188.0, -29.0], [169.0, -29.0]],
                [[187.0, -1.0], [210.0, -1.0], [210.0, -28.0], [187.0, -28.0]],
                [[208.0, -5.0], [222.0, -5.0], [222.0, -29.0], [208.0, -29.0]],
                [[221.0, -8.0], [221.0, 3.0], [232.0, 3.0], [232.0, -8.0]],
                [[262.5, -27.5], [262.5, -0.5], [270.5, -0.5], [270.5, -27.5]],
            ],
        );
        assert_eq!(mesh.preferred_width, 281.0);
    }

    #[test]
    fn without_the_modifier_quads_stay_horizontal() {
        let mut text = title("願望");
        text.vertical = None;
        assert_corners(
            &corners(&laid_out(&text)),
            &[
                [[-1.0, 101.0], [40.0, 101.0], [40.0, 60.0], [-1.0, 60.0]],
                [[40.0, 100.0], [80.0, 100.0], [80.0, 61.0], [40.0, 61.0]],
            ],
        );
    }

    #[test]
    fn upright_turns_cover_kana_kanji_and_hangul_only() {
        for ch in ['あ', 'ゟ', 'ア', 'ー', 'ヿ', '一', '鿿', '가', '힣'] {
            assert!(turns_upright(ch), "{ch}");
        }
        for ch in ['A', '3', '「', '。', '、', '々', '\u{a0}', '\u{3000}', '，'] {
            assert!(!turns_upright(ch), "{ch}");
        }
    }

    #[test]
    fn modifier_characters_skip_line_breaks_carriage_returns_and_spaces() {
        // The carriage return keeps a quad of its own, but the modifier's
        // string drops it, so the kanji after it lines up with the second
        // character of that string.
        let mut table = MetricTable::default();
        let mesh = layout(&title("願\r望"), 1.0, &mut table);
        assert_eq!(mesh.quads.len(), 3);
        assert_eq!(mesh.quads[1].corners, [[40.0, 65.0]; 4]);
        assert!(mesh.quads[1].glyph.is_none());
        // The second modifier character is 望, applied to the zero quad of
        // the carriage return; the last quad is past the end and is left
        // unturned.
        assert_eq!(
            mesh.quads[2].corners,
            [[40.0, 100.0], [80.0, 100.0], [80.0, 61.0], [40.0, 61.0]]
        );
        assert!(!table.requested.contains(&('\r', 40)));
    }

    #[test]
    fn a_tab_moves_the_pen_to_the_next_stop_of_four_spaces() {
        // The space advance is 12 at 40 pixels, so stops are 48 apart.
        let mut text = title("願\t望");
        text.vertical = None;
        let mesh = laid_out(&text);
        assert_eq!(mesh.quads.len(), 2);
        assert_eq!(mesh.quads[1].corners[0], [48.0, 100.0]);
        assert_eq!(mesh.preferred_width, 88.0);
    }

    #[test]
    fn a_character_the_font_lacks_is_a_blank_quad_with_no_advance() {
        let mut text = title("願☃望");
        text.vertical = None;
        let mesh = laid_out(&text);
        assert_eq!(mesh.quads.len(), 3);
        assert_eq!(
            mesh.quads[1].corners,
            [[39.0, 66.0], [41.0, 66.0], [41.0, 64.0], [39.0, 64.0]]
        );
        assert!(mesh.quads[1].glyph.is_none());
        assert_eq!(mesh.quads[2].corners[0], [40.0, 100.0]);
    }

    #[test]
    fn glyphs_are_rendered_at_the_output_pixel_size_and_geometry_scales_back() {
        let text = title("願望");
        let mut table = MetricTable::default();
        let doubled = layout(&text, 2.0, &mut table);
        assert!(table.requested.iter().all(|(_, size)| *size == 80));
        // Metrics that scale exactly with the size give the same node-space
        // text up to the rounding of the doubled line metrics.
        assert_eq!(doubled.preferred_width, 80.0);
        assert_eq!(doubled.quads[0].corners[0], [-0.5, 60.5]);
        assert_eq!(pixel_size(40, 1.0), 40);
        assert_eq!(pixel_size(36, 1.5), 54);
        assert_eq!(pixel_size(30, 0.99), 29);
        assert_eq!(pixel_size(300, 2.0), MAX_PIXEL_SIZE);
        assert_eq!(pixel_size(0, 1.0), 0);
        assert!(layout(
            &source("願", 0, 1.0, [1.0, 1.0], [0.0, 0.0]),
            1.0,
            &mut table
        )
        .quads
        .is_empty());
    }

    #[test]
    fn quads_keep_their_glyph_bitmaps_for_the_renderer() {
        let mesh = laid_out(&title("願望"));
        let glyph = mesh.quads[0].glyph.as_ref().expect("bitmap");
        assert_eq!((glyph.width, glyph.rows), (39, 39));
        // The cell is the bitmap plus the padding on each side, one texel per
        // pixel: 41 texels across a 41-unit quad.
        let [top_left, _, bottom_right, _] = mesh.quads[0].corners;
        assert_eq!(bottom_right[0] - top_left[0], 41.0);
        assert_eq!(
            glyph.width + 2 * mesh.cell_padding,
            (bottom_right[1] - top_left[1]) as u32
        );
    }

    #[test]
    fn a_fitted_backdrop_keeps_its_top_and_grows_with_the_preferred_width() {
        let backdrop = UguiTextBackdrop {
            color: [0.0, 0.8, 0.733_333_35, 1.0],
            rect: Rect {
                x: -6.0,
                y: -210.2,
                width: 50.0,
                height: 126.0,
            },
            fit_padding: Some(34.0),
        };
        assert_eq!(
            backdrop.resolved_rect(160.0),
            Rect {
                height: 194.0,
                ..backdrop.rect
            }
        );
        let fixed = UguiTextBackdrop {
            fit_padding: None,
            ..backdrop.clone()
        };
        assert_eq!(fixed.resolved_rect(160.0), backdrop.rect);
    }

    #[test]
    fn cell_coverage_interpolates_between_texel_centres_and_treats_the_padding_as_empty() {
        // A 2 x 2 bitmap, opaque top row and half-covered bottom row, in a
        // 4 x 4 cell.
        let glyph = UguiGlyph {
            bitmap_left: 0,
            bitmap_top: 2,
            width: 2,
            rows: 2,
            advance_26_6: 3 * 64,
            coverage: vec![255, 255, 128, 128],
        };
        let at = |u: f32, v: f32| cell_coverage(&glyph, 1, u, v);
        // Texel centres return their texels.
        assert_eq!(at(1.5, 1.5), 1.0);
        assert_eq!(at(2.5, 2.5), 128.0 / 255.0);
        assert_eq!(at(0.5, 0.5), 0.0);
        // Halfway between the padding and the first bitmap column, and
        // between the two rows.
        assert_eq!(at(1.0, 1.5), 0.5);
        assert_eq!(at(1.5, 2.0), (1.0 + 128.0 / 255.0) / 2.0);
        // The cell edge and beyond it are empty.
        assert_eq!(at(0.0, 2.0), 0.0);
        assert_eq!(at(4.0, 2.0), 0.0);
        assert_eq!(at(-3.0, 9.0), 0.0);
    }

    #[test]
    fn sources_round_trip_through_json_and_bincode() {
        let mut text = title("願望");
        text.backdrop = Some(UguiTextBackdrop {
            color: [0.0, 0.8, 0.733_333_35, 1.0],
            rect: Rect::default(),
            fit_padding: Some(34.0),
        });
        let json = serde_json::to_value(&text).unwrap();
        assert_eq!(json["vertical"]["offsets"][4]["target"], "、");
        assert_eq!(
            serde_json::from_value::<UguiTextSource>(json).unwrap(),
            text
        );
        let bytes = bincode::encode_to_vec(&text, bincode::config::standard()).unwrap();
        let (decoded, _): (UguiTextSource, usize) =
            bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        assert_eq!(decoded, text);
    }
}
