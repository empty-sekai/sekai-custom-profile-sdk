//! Where TextMesh Pro may break a line, and soft wrapping of markup to a
//! width.
//!
//! A line may break after white space, `-`, the zero width space and the soft
//! hyphen, unless the character is inside `<nobr>` or is one of the
//! non-breaking spaces and hyphens. Between CJK characters a line may break
//! anywhere except before a character that must not begin a line and after
//! one that must not end a line. Until a line has offered its first break
//! opportunity, it may also break between any two characters, so a word longer
//! than the width is split rather than overflowing.
//!
//! The two character tables are TextMesh Pro's default line-breaking rules.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::markup::{visible_scalars, VisibleScalar};

/// Characters that must not end a line (opening brackets and the like).
const LEADING_CHARACTERS: &str = include_str!("line_breaking_leading.txt");
/// Characters that must not begin a line (closing brackets, sentence-ending
/// punctuation, small kana and the like).
const FOLLOWING_CHARACTERS: &str = include_str!("line_breaking_following.txt");

fn table(source: &'static str, cell: &'static OnceLock<Vec<char>>) -> &'static [char] {
    cell.get_or_init(|| {
        let mut chars: Vec<char> = source.chars().collect();
        chars.sort_unstable();
        chars.dedup();
        chars
    })
}

/// Whether a line must not end with `ch`.
pub fn is_leading_character(ch: char) -> bool {
    static CELL: OnceLock<Vec<char>> = OnceLock::new();
    table(LEADING_CHARACTERS, &CELL).binary_search(&ch).is_ok()
}

/// Whether a line must not begin with `ch`.
pub fn is_following_character(ch: char) -> bool {
    static CELL: OnceLock<Vec<char>> = OnceLock::new();
    table(FOLLOWING_CHARACTERS, &CELL)
        .binary_search(&ch)
        .is_ok()
}

/// Whether `ch` is in the ranges where TextMesh Pro breaks between
/// characters: Hangul, CJK ideographs, compatibility forms and full-width
/// forms.
pub fn is_cjk_break_character(ch: char) -> bool {
    let code = u32::from(ch);
    matches!(
        code,
        0x1101..=0x11FE
            | 0xA961..=0xA97E
            | 0xAC01..=0xD7FE
            | 0x2E81..=0x9FFE
            | 0xF901..=0xFAFE
            | 0xFE31..=0xFE4E
            | 0xFF01..=0xFFEE
    )
}

/// Whether a line may break after `ch` because of the character itself.
fn is_break_character(ch: char) -> bool {
    (ch.is_whitespace() || matches!(ch, '\u{200B}' | '-' | '\u{00AD}'))
        && !matches!(
            ch,
            '\u{00A0}' | '\u{2007}' | '\u{2011}' | '\u{202F}' | '\u{2060}'
        )
}

/// Whether a character too wide for the line forces a break. White space and
/// the zero-width controls may hang past the edge.
fn checks_width(ch: char) -> bool {
    !(ch.is_whitespace() || matches!(ch, '\u{200B}' | '\u{00AD}' | '\u{0003}'))
}

/// One measured visible character of a text, in layout order.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeasuredTextUnit {
    pub advance: f32,
    pub hard_break: bool,
}

/// Indices after which a line breaks when `scalars` with the given advances
/// are laid out in `max_width`.
fn soft_breaks(
    scalars: &[VisibleScalar],
    units: &[MeasuredTextUnit],
    max_width: f32,
) -> Vec<usize> {
    let mut breaks = Vec::new();
    let mut line_start = 0usize;
    let mut width = 0.0f32;
    let mut saved: Option<usize> = None;
    let mut first_word = true;
    let mut index = 0usize;
    while index < units.len() {
        let unit = units[index];
        if unit.hard_break {
            line_start = index + 1;
            width = 0.0;
            saved = None;
            first_word = true;
            index += 1;
            continue;
        }
        let scalar = &scalars[index];
        let advance = unit.advance.max(0.0);
        if checks_width(scalar.ch) && index != line_start && width + advance > max_width {
            let after = saved.unwrap_or(index - 1);
            breaks.push(after);
            line_start = after + 1;
            width = 0.0;
            saved = None;
            first_word = true;
            index = after + 1;
            continue;
        }
        width += advance;

        if !scalar.no_break && is_break_character(scalar.ch) {
            saved = Some(index);
            first_word = false;
        } else if !scalar.no_break && is_cjk_break_character(scalar.ch) {
            let next_follows = scalars
                .get(index + 1)
                .is_some_and(|next| is_following_character(next.ch));
            if !is_leading_character(scalar.ch) {
                if !next_follows {
                    saved = Some(index);
                    first_word = false;
                }
                if first_word {
                    saved = Some(index);
                }
            } else if first_word && index == line_start {
                saved = Some(index);
            }
        } else if first_word {
            saved = Some(index);
        }
        index += 1;
    }
    breaks
}

/// Inserts soft line breaks into TMP markup without splitting tags.
///
/// `units` holds one entry per visible character of `raw` as
/// [`visible_scalars`] reports them, a line feed marked as a hard break. A
/// break goes in front of the first character of the new line; a soft hyphen
/// the line breaks on is shown as `-`.
pub fn wrap_tmp_markup(
    raw: &str,
    units: &[MeasuredTextUnit],
    max_width: f32,
) -> Result<String, &'static str> {
    if !max_width.is_finite() || max_width <= 0.0 {
        return Err("max_width must be finite and positive");
    }
    let scalars = visible_scalars(raw);
    if scalars.len() != units.len() {
        return Err("measured unit count does not match visible markup text");
    }
    let breaks = soft_breaks(&scalars, units, max_width);
    let mut output = String::with_capacity(raw.len() + breaks.len());
    let mut cursor = 0usize;
    for after in breaks {
        let hyphen = &scalars[after];
        if hyphen.ch == '\u{00AD}' && raw[hyphen.source.clone()] == *"\u{00AD}" {
            output.push_str(&raw[cursor..hyphen.source.start]);
            output.push('-');
            cursor = hyphen.source.end;
        }
        let insert_at = scalars
            .get(after + 1)
            .map_or(raw.len(), |next| next.source.start);
        output.push_str(&raw[cursor..insert_at]);
        output.push('\n');
        cursor = insert_at;
    }
    output.push_str(&raw[cursor..]);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(text: &str, advance: f32) -> Vec<MeasuredTextUnit> {
        visible_scalars(text)
            .iter()
            .map(|scalar| MeasuredTextUnit {
                advance: if scalar.ch == '\n' { 0.0 } else { advance },
                hard_break: scalar.ch == '\n',
            })
            .collect()
    }

    fn wrap(text: &str, advance: f32, width: f32) -> String {
        wrap_tmp_markup(text, &units(text, advance), width).unwrap()
    }

    #[test]
    fn breaks_go_between_characters_without_splitting_markup() {
        assert_eq!(
            wrap("<color=#ff0000>ABCD</color>", 60.0, 130.0),
            "<color=#ff0000>AB\nCD</color>"
        );
    }

    #[test]
    fn authored_newlines_survive_and_unit_drift_is_rejected() {
        assert_eq!(wrap("A\nB", 40.0, 50.0), "A\nB");
        let three = units("A\nB", 40.0);
        assert!(wrap_tmp_markup("AB", &three, 50.0).is_err());
    }

    #[test]
    fn latin_words_wrap_at_the_last_space() {
        assert_eq!(
            wrap("hello wonderful world", 10.0, 120.0),
            "hello \nwonderful \nworld"
        );
    }

    #[test]
    fn a_word_wider_than_the_line_is_split_between_characters() {
        assert_eq!(wrap("abcdefgh", 10.0, 35.0), "abc\ndef\ngh");
    }

    #[test]
    fn closing_punctuation_does_not_begin_a_line() {
        // The full stop may not start a line, so the character before it
        // moves down with it.
        assert_eq!(
            wrap("ありがとう。よろしく", 10.0, 55.0),
            "ありがと\nう。よろし\nく"
        );
    }

    #[test]
    fn opening_brackets_do_not_end_a_line() {
        assert_eq!(wrap("あいう「えお」", 10.0, 40.0), "あいう\n「えお」");
    }

    #[test]
    fn unrecognised_markup_is_measured_as_text() {
        // `<3` is not a tag, so its characters take room and line breaking
        // stays on.
        let text = "I <3 you so much";
        assert_eq!(wrap(text, 10.0, 85.0), "I <3 you \nso much");
        // Neither is a bracketed emoticon; it is measured like any other
        // characters instead of making the whole text unwrappable.
        assert_eq!(
            wrap("ok <(_ _)> thanks", 10.0, 105.0),
            "ok <(_ _)> \nthanks"
        );
    }

    #[test]
    fn nobr_keeps_words_together() {
        assert_eq!(
            wrap("<nobr>ab cd</nobr> ef", 10.0, 55.0),
            "<nobr>ab cd</nobr> \nef"
        );
    }

    #[test]
    fn a_soft_hyphen_shows_where_the_line_breaks_on_it() {
        let text = "ab\u{00AD}cd";
        let mut measured = units(text, 10.0);
        measured[2].advance = 0.0;
        assert_eq!(wrap_tmp_markup(text, &measured, 25.0).unwrap(), "ab-\ncd");
    }

    #[test]
    fn line_breaking_tables_hold_the_default_rules() {
        for ch in ['(', '「', '（', '$'] {
            assert!(is_leading_character(ch), "{ch:?}");
        }
        for ch in [')', '」', '。', '、', 'ー', 'ッ', 'ゃ', '!', '?'] {
            assert!(is_following_character(ch), "{ch:?}");
        }
        assert!(!is_following_character('あ'));
        assert!(is_cjk_break_character('漢'));
        assert!(is_cjk_break_character('あ'));
        assert!(!is_cjk_break_character('A'));
    }
}
