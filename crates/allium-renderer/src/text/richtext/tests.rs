//! 富文本解析回归测试。

use super::{parse_rich_segments, CaseTransform, Indent, InlineAlign, SizeSpec};

#[test]
fn hash_color_without_alpha_resets_alpha_override() {
    let segs = parse_rich_segments("<alpha=#80><#FF0000>a<#00FF0080>b");
    assert_eq!(segs.len(), 2);
    assert_eq!(segs[0].alpha, None);
    assert_eq!(segs[1].alpha, Some(128.0 / 255.0));
}

#[test]
fn smallcaps_tag_sets_flag_without_touching_plain_uppercase_mode() {
    let segs = parse_rich_segments("<smallcaps>abc</smallcaps>");
    assert_eq!(segs.len(), 1);
    assert!(segs[0].smallcaps);
    assert_eq!(segs[0].case_transform, CaseTransform::None);
}

#[test]
fn align_tag_sets_inline_alignment_override() {
    let segs = parse_rich_segments("<align=center>AB</align>");
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].align, Some(InlineAlign::Center));
}

#[test]
fn indent_tag_parses_px_percent_and_em_units() {
    let segs = parse_rich_segments("<indent=100>A<indent=50%>B<indent=1em>C");
    assert_eq!(segs.len(), 3);
    assert_eq!(segs[0].indent, Some(Indent::Pixels(100.0)));
    assert_eq!(segs[1].indent, Some(Indent::Percent(50.0)));
    assert_eq!(segs[2].indent, Some(Indent::Em(1.0)));
}

#[test]
fn size_tag_parses_absolute_delta_percent_and_em_units() {
    let segs = parse_rich_segments("<size=40>A<size=+10>B<size=-5>C<size=150%>D<size=1.5em>E");
    assert_eq!(segs.len(), 5);
    assert_eq!(segs[0].size, Some(SizeSpec::Absolute(40.0)));
    assert_eq!(segs[1].size, Some(SizeSpec::Delta(10.0)));
    assert_eq!(segs[2].size, Some(SizeSpec::Delta(-5.0)));
    assert_eq!(segs[3].size, Some(SizeSpec::Percent(150.0)));
    assert_eq!(segs[4].size, Some(SizeSpec::Em(1.5)));
}
