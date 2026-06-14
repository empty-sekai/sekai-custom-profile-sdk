use crate::text::richtext::{CaseTransform, Indent, InlineAlign, SizeSpec, TextSegment};
use skia_safe::Font;

pub(super) struct RichTextGlobal {
    pub scale: f32,
    pub alpha: Option<f32>,
    pub color: Option<(u8, u8, u8)>,
    pub align: Option<InlineAlign>,
    pub clean: String,
}

pub(super) fn segments_to_global(segs: &[TextSegment]) -> RichTextGlobal {
    let first = segs.first();
    RichTextGlobal {
        scale: first.and_then(|s| s.scale).unwrap_or(1.0),
        alpha: first.and_then(|s| s.alpha),
        color: first.and_then(|s| s.color),
        align: first.and_then(|s| s.align),
        clean: segs.iter().map(|s| s.text.as_str()).collect(),
    }
}

pub(super) fn transform_char_for_segment(ch: char, seg: &TextSegment) -> (String, f32) {
    if seg.smallcaps && ch.is_lowercase() {
        return (ch.to_uppercase().collect::<String>(), 0.8);
    }
    let text = match seg.case_transform {
        CaseTransform::Upper => ch.to_uppercase().collect::<String>(),
        CaseTransform::Lower => ch.to_lowercase().collect::<String>(),
        CaseTransform::None => ch.to_string(),
    };
    (text, 1.0)
}

pub(super) fn resolve_segment_font_size(size: Option<SizeSpec>, base_font_size: f32) -> f32 {
    match size {
        Some(SizeSpec::Absolute(v)) => v,
        Some(SizeSpec::Delta(v)) => base_font_size + v,
        Some(SizeSpec::Percent(v)) => base_font_size * v / 100.0,
        Some(SizeSpec::Em(v)) => base_font_size * v,
        None => base_font_size,
    }
}

pub(super) fn resolve_indent_value(
    spec: Option<Indent>,
    base_font_size: f32,
    box_w: f32,
) -> Option<f32> {
    match spec {
        Some(Indent::Pixels(v)) => Some(v / super::TEXT_SCALE),
        Some(Indent::Em(v)) => Some(v * base_font_size / super::TEXT_SCALE),
        Some(Indent::Percent(v)) => Some(box_w * v / 100.0),
        None => None,
    }
}

pub(super) fn tmp_measure_advance(text: &str, font: &Font, _font_size: f32) -> f32 {
    let mut total = 0.0f32;
    for ch in text.chars() {
        let s = ch.to_string();
        total += font.measure_str(&s, None).0;
    }
    total
}

pub(super) fn is_fullwidth_char(ch: char) -> bool {
    matches!(ch,
        '\u{2000}'..='\u{206F}' |
        '\u{2190}'..='\u{21FF}' |
        '\u{2200}'..='\u{22FF}' |
        '\u{2300}'..='\u{23FF}' |
        '\u{2460}'..='\u{24FF}' |
        '\u{2500}'..='\u{259F}' |
        '\u{25A0}'..='\u{25FF}' |
        '\u{2600}'..='\u{26FF}' |
        '\u{2700}'..='\u{27BF}' |
        '\u{3000}'..='\u{30FF}' |
        '\u{3400}'..='\u{4DBF}' |
        '\u{4E00}'..='\u{9FFF}' |
        '\u{F900}'..='\u{FAFF}' |
        '\u{FE30}'..='\u{FE4F}' |
        '\u{FF01}'..='\u{FF60}'
    )
}

#[cfg(test)]
mod tests {
    use super::is_fullwidth_char;

    #[test]
    fn fullwidth_detection_covers_cjk_and_ascii_halfwidth_gap() {
        assert!(is_fullwidth_char('你'));
        assert!(is_fullwidth_char('（'));
        assert!(!is_fullwidth_char('a'));
    }
}
