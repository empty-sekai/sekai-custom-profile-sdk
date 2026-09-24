//! Line placement rules both layout engines share.
//!
//! Widths and positions here are in draw units: layout units divided by the
//! face scale, measured from the centre of the text box, which is the space
//! both engines place glyphs in.

use super::face::PROFILE_FACE;
use super::markup::{HorizontalOffset, TextSegment};

/// Padding the game's text view adds to TextMesh Pro's preferred size before
/// feeding it back as the container size, in layout units.
pub const CONTAINER_PADDING: f32 = 64.0;

/// Extends a line's preferred width with a glyph drawn from caret position
/// `caret_before`. Both are in layout units.
pub fn extend_preferred_width(current: f32, caret_before: f32, glyph_advance: f32) -> f32 {
    current.max(caret_before.abs() + glyph_advance)
}

/// [`extend_preferred_width`] for a character: white space moves the caret
/// but does not widen the line, so a trailing space adds nothing while an
/// inner one is counted by the glyph after it.
pub fn extend_preferred_width_for_char(
    current: f32,
    caret_before: f32,
    glyph_advance: f32,
    ch: char,
) -> f32 {
    if ch.is_whitespace() {
        current
    } else {
        extend_preferred_width(current, caret_before, glyph_advance)
    }
}

/// Start of a line with a percentage `<indent>`, for a line `line_width`
/// wide and the TMP alignment code `align` (1 left, 2 centre, 4 right).
/// `None` when the share leaves no room.
pub fn percent_indent_start(share: f32, line_width: f32, align: i32) -> Option<f32> {
    if share >= 1.0 {
        return None;
    }
    let scale = PROFILE_FACE.scale;
    let rect = (line_width * scale + CONTAINER_PADDING) / (1.0 - share);
    let indent = rect * share / scale;
    Some(match align {
        2 => (indent - line_width) / 2.0,
        4 => rect / (2.0 * scale) - line_width,
        _ => rect * (share - 0.5) / scale,
    })
}

/// Settled start of a line with a percentage `<line-indent>`.
///
/// The game's text view feeds TextMesh Pro's preferred width plus the
/// padding back as the next frame's container width, so the settled
/// container follows the preferred width of the widest line, not the caret
/// advance of this one; `caret_width` only aligns the line inside it.
pub fn static_line_indent_start(
    share: f32,
    caret_width: f32,
    preferred_width: f32,
    align: i32,
) -> Option<f32> {
    if share >= 1.0 {
        return None;
    }
    let scale = PROFILE_FACE.scale;
    let rect = (preferred_width * scale + CONTAINER_PADDING) / (1.0 - share);
    let indent = rect * share / scale;
    Some(match align {
        2 => (indent - caret_width) / 2.0,
        4 => rect / (2.0 * scale) - caret_width,
        _ => rect * (share - 0.5) / scale,
    })
}

/// A horizontal offset in draw units; a percentage is a share of `box_width`.
pub fn offset_draw_units(offset: HorizontalOffset, box_width: f32) -> f32 {
    match offset {
        HorizontalOffset::Units(value) => value / PROFILE_FACE.scale,
        HorizontalOffset::Percent(value) => box_width * value / 100.0,
    }
}

/// Caret position at the start of a line whose aligned left edge is
/// `line_left`: the `<indent>` of its first run, then its `<line-indent>`,
/// which TextMesh Pro adds on top. `preferred_width` is the widest line's
/// preferred width, which a percentage line indent settles against.
pub fn line_start(
    line_left: f32,
    line_width: f32,
    preferred_width: f32,
    align: i32,
    first_run: Option<&TextSegment>,
) -> f32 {
    let mut start = line_left;
    let Some(run) = first_run else {
        return start;
    };
    match run.indent {
        Some(HorizontalOffset::Percent(value)) => {
            if let Some(indented) = percent_indent_start(value / 100.0, line_width, align) {
                start = indented;
            }
        }
        Some(HorizontalOffset::Units(value)) => start += value / PROFILE_FACE.scale,
        None => {}
    }
    match run.line_indent {
        Some(HorizontalOffset::Percent(value)) => {
            if let Some(settled) =
                static_line_indent_start(value / 100.0, line_width, preferred_width, align)
            {
                start = settled;
            }
        }
        Some(HorizontalOffset::Units(value)) => start += value / PROFILE_FACE.scale,
        None => {}
    }
    start
}

/// The percentage of a `<line-indent>` that covers every visible run, which
/// is what drives the game's settling line-indent animation. `None` when a
/// visible run has no percentage indent or the runs disagree.
pub fn uniform_line_indent_percent(segments: &[TextSegment]) -> Option<f32> {
    let mut percent = None;
    for segment in segments
        .iter()
        .filter(|segment| segment.text.chars().any(|ch| !ch.is_whitespace()))
    {
        let Some(HorizontalOffset::Percent(value)) = segment.line_indent else {
            return None;
        };
        if !value.is_finite()
            || percent.is_some_and(|current: f32| (current - value).abs() > f32::EPSILON)
        {
            return None;
        }
        percent = Some(value);
    }
    percent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_width_resets_with_the_caret_and_keeps_negative_extents() {
        // A `<pos>` back to the line start measures from there again.
        let mut width = 0.0;
        width = extend_preferred_width(width, 0.0, 36.0);
        width = extend_preferred_width(width, 0.0, 36.0);
        assert_eq!(width, 36.0);
        assert_eq!(extend_preferred_width(0.0, -221.0, 31.0), 252.0);
    }

    #[test]
    fn preferred_width_excludes_trailing_spaces_but_counts_inner_ones() {
        let mut width = 0.0;
        let mut caret = 0.0;
        width = extend_preferred_width_for_char(width, caret, 24.0, ' ');
        caret += 24.0;
        width = extend_preferred_width_for_char(width, caret, 110.0, '●');
        caret += 110.0;
        for _ in 0..5 {
            width = extend_preferred_width_for_char(width, caret, 24.0, ' ');
            caret += 24.0;
        }
        assert!((width - 134.0).abs() < 1e-6);
        assert!((caret - 254.0).abs() < 1e-6);
    }

    #[test]
    fn static_line_indent_uses_the_preferred_width_not_the_caret_width() {
        // A <scale> widens the caret advance but not the preferred width, so
        // the settled start follows the unscaled width.
        let share = 0.96;
        let unscaled = static_line_indent_start(share, 1250.0, 1250.0, 1).unwrap();
        let scaled = static_line_indent_start(share, 75000.0, 1250.0, 1).unwrap();
        assert_eq!(unscaled, scaled);
        assert!((unscaled - (1250.0 * 2.0 + 64.0) / 0.04 * 0.46 / 2.0).abs() < 0.05);
        assert_eq!(static_line_indent_start(1.0, 10.0, 10.0, 1), None);
        for caret_width in [70.0f32, 100.0, 120.0] {
            let actual = static_line_indent_start(0.939, caret_width, 100.0, 1).unwrap();
            let expected = (100.0 * 2.0 + 64.0) / (1.0 - 0.939) * (0.939 - 0.5) / 2.0;
            assert!(
                (actual - expected).abs() < 1e-3,
                "{caret_width}: {actual} != {expected}"
            );
        }
    }

    #[test]
    fn line_indent_adds_to_the_indent_and_uses_draw_units() {
        let segments = super::super::markup::parse_segments("<indent=10><line-indent=1em>A", 30.0);
        assert_eq!(
            line_start(-50.0, 20.0, 20.0, 1, segments.first()),
            -50.0 + 5.0 + 15.0
        );
        assert_eq!(
            offset_draw_units(HorizontalOffset::Percent(50.0), 80.0),
            40.0
        );
    }

    #[test]
    fn a_line_indent_animation_needs_one_percentage_on_every_visible_run() {
        let parse = |text: &str| super::super::markup::parse_segments(text, 30.0);
        assert_eq!(
            uniform_line_indent_percent(&parse(" <line-indent=50%>A\nB")),
            Some(50.0)
        );
        assert_eq!(
            uniform_line_indent_percent(&parse("A<line-indent=50%>B")),
            None
        );
        assert_eq!(
            uniform_line_indent_percent(&parse("<line-indent=20>A")),
            None
        );
    }
}
