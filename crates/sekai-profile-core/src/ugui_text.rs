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
mod tests;
