//! Face metrics of the profile fonts and the line spacing the game derives
//! from a text element.
//!
//! TextMesh Pro reads these values from a font asset's `FaceInfo`. The
//! profile fonts share one set, so the layout engines take them from here
//! instead of carrying their own copies.

/// The `FaceInfo` fields TextMesh Pro consults while laying out text, in font
/// design units at [`TmpFaceInfo::point_size`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TmpFaceInfo {
    pub point_size: f32,
    /// `FaceInfo.scale`: the factor between point sizes and layout units.
    pub scale: f32,
    pub ascent_line: f32,
    pub descent_line: f32,
    pub line_height: f32,
    pub superscript_offset: f32,
    pub superscript_size: f32,
    pub subscript_offset: f32,
    pub subscript_size: f32,
    /// Default `<i>` slant, in hundredths of the glyph height.
    pub italic_style: i32,
}

/// Face metrics shared by every profile font.
pub const PROFILE_FACE: TmpFaceInfo = TmpFaceInfo {
    point_size: 75.0,
    scale: 2.0,
    ascent_line: 66.0,
    descent_line: -9.0,
    line_height: 150.0,
    superscript_offset: 66.0,
    superscript_size: 0.5,
    subscript_offset: -9.0,
    subscript_size: 0.5,
    italic_style: 35,
};

impl TmpFaceInfo {
    /// Layout units per font design unit at `font_size`.
    pub fn font_scale(&self, font_size: f32) -> f32 {
        font_size / self.point_size * self.scale
    }
}

/// The game's default for the `custom_profile_text_line_spacing_factor`
/// master config value, which multiplies a text element's line spacing
/// before it reaches TextMesh Pro.
pub const DEFAULT_LINE_SPACING_FACTOR: f32 = 1.325;

/// Extra offset between two lines, in layout units, for an element with the
/// given line spacing and base font size.
///
/// The element's value is handed to TextMesh Pro as
/// `lineSpacing * FaceInfo.scale * factor`, and TextMesh Pro adds it per line
/// scaled by one hundredth of the base font size.
pub fn line_spacing_offset(
    line_spacing: f32,
    font_size: f32,
    face: &TmpFaceInfo,
    factor: f32,
) -> f32 {
    line_spacing * face.scale * factor * font_size * 0.01
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_spacing_uses_the_game_factor_not_the_point_size_ratio() {
        // One unit of line spacing at size 100 moves the next line by
        // 2 * 1.325 layout units, not 2 * 100 / 75.
        let offset = line_spacing_offset(1.0, 100.0, &PROFILE_FACE, DEFAULT_LINE_SPACING_FACTOR);
        assert!((offset - 2.65).abs() < 1e-5, "{offset}");
    }
}
