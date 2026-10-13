//! Which glyph TextMesh Pro draws for a character, and which characters it
//! draws at all.
//!
//! A dynamic font asset that lacks a character first tries a stand-in from the
//! same face, then the next font in the fallback chain. When no font has it,
//! the character is replaced by [`MISSING_GLYPH_CHARACTER`], and by a space if
//! that is missing too. Text never fails to lay out because of a missing
//! character.

/// The character TextMesh Pro substitutes for one no font can draw.
pub const MISSING_GLYPH_CHARACTER: char = '\u{25A1}';

/// The same-face stand-in a font asset uses for a character it lacks, before
/// any fallback font is searched.
pub fn same_face_alternate(ch: char) -> Option<char> {
    match ch {
        '\u{00A0}' => Some(' '),
        '\u{00AD}' | '\u{2011}' => Some('-'),
        _ => None,
    }
}

/// The face and the glyph character chosen for one source character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphChoice {
    /// Index into the font chain the caller searched, primary first.
    pub face: usize,
    /// The character whose glyph is drawn from that face.
    pub glyph: char,
}

/// Resolves `ch` against a font chain of `face_count` faces, primary first.
///
/// `has_glyph(face, ch)` reports whether a face can supply `ch`. Control
/// characters are layout controls rather than glyphs and are never replaced.
/// `None` means nothing can be drawn and the character advances nothing.
pub fn resolve_glyph(
    ch: char,
    face_count: usize,
    mut has_glyph: impl FnMut(usize, char) -> bool,
) -> Option<GlyphChoice> {
    if ch.is_control() {
        return (face_count > 0 && has_glyph(0, ch)).then_some(GlyphChoice { face: 0, glyph: ch });
    }
    let mut search = |wanted: char| {
        (0..face_count).find_map(|face| {
            if has_glyph(face, wanted) {
                return Some(GlyphChoice {
                    face,
                    glyph: wanted,
                });
            }
            same_face_alternate(wanted)
                .filter(|&alternate| has_glyph(face, alternate))
                .map(|glyph| GlyphChoice { face, glyph })
        })
    };
    search(ch)
        .or_else(|| search(MISSING_GLYPH_CHARACTER))
        .or_else(|| {
            (face_count > 0 && has_glyph(0, ' ')).then_some(GlyphChoice {
                face: 0,
                glyph: ' ',
            })
        })
}

/// Whether TextMesh Pro emits a quad for `ch`. White space, the zero width
/// space, the soft hyphen and the end-of-text marker only move the caret.
pub fn is_drawn(ch: char) -> bool {
    !(ch.is_whitespace() || matches!(ch, '\u{200B}' | '\u{00AD}' | '\u{0003}'))
}

/// Whether `ch` advances the caret. The soft hyphen and the end-of-text
/// marker take no room; a soft hyphen only shows as `-` where a line breaks
/// on it.
pub fn advances_caret(ch: char) -> bool {
    !matches!(ch, '\u{00AD}' | '\u{0003}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain<'a>(faces: &'a [&'a str]) -> impl FnMut(usize, char) -> bool + 'a {
        move |face, ch| faces[face].contains(ch)
    }

    #[test]
    fn missing_characters_become_the_square_then_the_space() {
        let faces = ["AB\u{25A1} ", "C"];
        assert_eq!(
            resolve_glyph('\u{2764}', 2, chain(&faces)),
            Some(GlyphChoice {
                face: 0,
                glyph: MISSING_GLYPH_CHARACTER
            })
        );
        let no_square = ["AB ", "C"];
        assert_eq!(
            resolve_glyph('\u{2764}', 2, chain(&no_square)),
            Some(GlyphChoice {
                face: 0,
                glyph: ' '
            })
        );
        assert_eq!(resolve_glyph('\u{2764}', 2, chain(&["AB", "C"])), None);
    }

    #[test]
    fn fallback_faces_are_searched_in_order() {
        let faces = ["AB\u{25A1}", "C\u{25A1}"];
        assert_eq!(
            resolve_glyph('C', 2, chain(&faces)),
            Some(GlyphChoice {
                face: 1,
                glyph: 'C'
            })
        );
    }

    #[test]
    fn same_face_stand_ins_win_over_the_fallback_face() {
        let faces = ["A- ", "\u{2011}\u{00A0}"];
        assert_eq!(
            resolve_glyph('\u{2011}', 2, chain(&faces)),
            Some(GlyphChoice {
                face: 0,
                glyph: '-'
            })
        );
        assert_eq!(
            resolve_glyph('\u{00A0}', 2, chain(&faces)),
            Some(GlyphChoice {
                face: 0,
                glyph: ' '
            })
        );
        assert_eq!(
            resolve_glyph('\u{00AD}', 2, chain(&faces)),
            Some(GlyphChoice {
                face: 0,
                glyph: '-'
            })
        );
    }

    #[test]
    fn control_characters_are_never_replaced() {
        assert_eq!(resolve_glyph('\r', 1, chain(&["\u{25A1} "])), None);
        assert_eq!(
            resolve_glyph('\t', 1, chain(&["\t"])),
            Some(GlyphChoice {
                face: 0,
                glyph: '\t'
            })
        );
    }

    #[test]
    fn invisible_characters_are_not_drawn() {
        for ch in [' ', '\u{00A0}', '\u{3000}', '\u{200B}', '\u{00AD}', '\n'] {
            assert!(!is_drawn(ch), "{ch:?}");
        }
        assert!(is_drawn('A'));
        assert!(!advances_caret('\u{00AD}'));
        assert!(advances_caret('\u{200B}'));
    }
}
