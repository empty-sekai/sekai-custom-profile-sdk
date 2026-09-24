//! TextMesh Pro text rules shared by every layout engine.
//!
//! [`markup`] reads the rich-text tags, [`line_breaking`] decides where lines
//! may wrap, [`glyph`] picks the glyph drawn for each character, [`layout`]
//! holds the line placement rules the engines share, and [`face`] the profile
//! font metrics the others rely on.
//! [`strip_tmp_tags`] and [`numeric_text_runs`] report the visible text the
//! same way the layout engines see it, so the character indices they produce
//! line up with laid-out glyphs.

pub mod face;
pub mod glyph;
pub mod layout;
pub mod line_breaking;
pub mod markup;

use serde::{Deserialize, Serialize};

pub use face::{TmpFaceInfo, DEFAULT_LINE_SPACING_FACTOR, PROFILE_FACE};
pub use line_breaking::{wrap_tmp_markup, MeasuredTextUnit};
pub use markup::{parse_segments, split_lines, visible_scalars, TextSegment, VisibleScalar};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NumericTextRun {
    pub text: String,
    pub plain_start: u32,
    pub plain_end: u32,
}

/// The text left once markup is resolved: accepted tags removed, `<br>`,
/// `<nbsp>` and `<zwsp>` replaced by their characters, and anything that is
/// not a valid tag kept as written.
pub fn strip_tmp_tags(source: &str) -> String {
    visible_scalars(source)
        .into_iter()
        .map(|scalar| scalar.ch)
        .collect()
}

/// Runs of ASCII digits in the visible text, indexed by visible character.
pub fn numeric_text_runs(source: &str) -> Vec<NumericTextRun> {
    let chars: Vec<char> = visible_scalars(source)
        .into_iter()
        .map(|scalar| scalar.ch)
        .collect();
    let mut runs = Vec::new();
    let mut cursor = 0;
    while cursor < chars.len() {
        if !chars[cursor].is_ascii_digit() {
            cursor += 1;
            continue;
        }
        let start = cursor;
        while cursor < chars.len() && chars[cursor].is_ascii_digit() {
            cursor += 1;
        }
        runs.push(NumericTextRun {
            text: chars[start..cursor].iter().collect(),
            plain_start: start as u32,
            plain_end: cursor as u32,
        });
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmp_tags_do_not_split_contiguous_ascii_digits() {
        assert_eq!(
            numeric_text_runs("<color=#fff>12</color><b>34</b>"),
            vec![NumericTextRun {
                text: "1234".into(),
                plain_start: 0,
                plain_end: 4,
            }]
        );
    }

    #[test]
    fn visible_symbols_and_newlines_split_runs_and_leading_zeroes_survive() {
        assert_eq!(
            numeric_text_runs("0012-34\n56"),
            vec![
                NumericTextRun {
                    text: "0012".into(),
                    plain_start: 0,
                    plain_end: 4
                },
                NumericTextRun {
                    text: "34".into(),
                    plain_start: 5,
                    plain_end: 7
                },
                NumericTextRun {
                    text: "56".into(),
                    plain_start: 8,
                    plain_end: 10
                },
            ]
        );
    }

    #[test]
    fn replaced_tags_count_as_the_characters_they_become() {
        // `<br>` is one line feed and `<nbsp>` one space, so the digits after
        // them sit at the indices the layout gives their glyphs.
        assert_eq!(
            numeric_text_runs("HP<br>100"),
            vec![NumericTextRun {
                text: "100".into(),
                plain_start: 3,
                plain_end: 6,
            }]
        );
        assert_eq!(
            numeric_text_runs("<nbsp>12"),
            vec![NumericTextRun {
                text: "12".into(),
                plain_start: 1,
                plain_end: 3,
            }]
        );
    }

    #[test]
    fn literal_markup_keeps_its_characters() {
        assert_eq!(strip_tmp_tags("<noparse><b>1</b></noparse>2"), "<b>1</b>2");
        assert_eq!(strip_tmp_tags("a<br>b<zwsp>c"), "a\nb\u{200B}c");
        assert_eq!(strip_tmp_tags("<love>3"), "<love>3");
    }
}
