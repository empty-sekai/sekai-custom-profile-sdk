use super::*;

fn text(segments: &[TextSegment]) -> String {
    segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect()
}

fn one(raw: &str) -> TextSegment {
    let segments = parse_segments(raw, 30.0);
    let visible: Vec<_> = segments
        .iter()
        .filter(|segment| !segment.text.is_empty())
        .collect();
    assert_eq!(visible.len(), 1, "{raw:?} produced {segments:?}");
    visible[0].clone()
}

#[test]
fn unrecognised_or_malformed_tags_stay_in_the_text() {
    for raw in [
        "<(_ _)>",
        "<love>",
        "<color=foo>x",
        "</alpha>",
        "<color=#12345>x",
        "<alpha=#8>x",
        "I <3 you > me",
        "<Leo/need>",
        "<i angle=500>x",
        "<b a b>x",
        "<space=10%>x",
        "<cspace=10%>x",
        "<voffset=10%>x",
        "<width=1em>x",
        "<align=middle>x",
        "<font=Arial>x",
        "<sprite=0>",
        "<style=H1>x",
        "<gradient=Rainbow>x",
        "<table>",
        "<td>",
    ] {
        assert_eq!(text(&parse_segments(raw, 30.0)), raw, "{raw:?}");
    }
}

#[test]
fn a_second_opening_bracket_restarts_the_tag() {
    let segments = parse_segments("<a<b>x", 30.0);
    assert_eq!(text(&segments), "<ax");
    assert!(!segments[0].bold);
    assert!(segments.last().unwrap().bold);
}

#[test]
fn tag_names_match_all_lowercase_or_all_uppercase_only() {
    assert_eq!(one("<COLOR=red>x").color, Some([255, 0, 0]));
    assert!(one("<B>x").bold);
    for raw in [
        "<Color=red>x",
        "<Size=50>x",
        "<color=RED>x",
        "<Color=#ff0000>x",
    ] {
        assert_eq!(text(&parse_segments(raw, 30.0)), raw, "{raw:?}");
    }
}

#[test]
fn line_break_and_space_tags_are_replaced_in_any_case() {
    assert_eq!(
        text(&parse_segments("a<br>b<BR>c<Br>d", 30.0)),
        "a\nb\nc\nd"
    );
    assert_eq!(
        text(&parse_segments("a<NbSp>b<zwsp>c", 30.0)),
        "a\u{00A0}b\u{200B}c"
    );
}

#[test]
fn carriage_return_joiner_and_soft_hyphen_are_not_tags() {
    for raw in ["a<cr>b", "a<zwj>b", "a<shy>b"] {
        assert_eq!(text(&parse_segments(raw, 30.0)), raw, "{raw:?}");
    }
}

#[test]
fn named_colours_use_the_tmp_values() {
    let cases = [
        ("red", [255, 0, 0]),
        ("lightblue", [173, 216, 230]),
        ("blue", [0, 0, 255]),
        ("grey", [128, 128, 128]),
        ("black", [0, 0, 0]),
        ("green", [0, 255, 0]),
        ("white", [255, 255, 255]),
        ("orange", [255, 128, 0]),
        ("purple", [160, 32, 240]),
        ("yellow", [255, 235, 4]),
    ];
    for (name, rgb) in cases {
        let segment = one(&format!("<color={name}>x"));
        assert_eq!(segment.color, Some(rgb), "{name}");
        assert_eq!(segment.alpha, 255, "{name}");
    }
    assert_eq!(one("<color=\"red\">x").color, Some([255, 0, 0]));
}

#[test]
fn hex_colours_are_counted_in_characters() {
    // A multi-byte character is one character of the tag, so these values
    // have the wrong length and stay literal instead of being sliced.
    for raw in ["<alpha=#日>x", "<color=#日日>x", "<#日日>x", "<#é1>x"] {
        assert_eq!(text(&parse_segments(raw, 30.0)), raw, "{raw:?}");
    }
    // Four characters is the short form; an unknown digit reads as 15.
    assert_eq!(one("<#é12>b").color, Some([255, 17, 34]));
    assert_eq!(one("<#FEF2EW>highlight").color, Some([0xFE, 0xF2, 0xEF]));
    assert_eq!(one("<color=#é1f>d").color, Some([255, 17, 255]));
}

#[test]
fn colour_tags_replace_alpha_and_closing_restores_the_element_colour() {
    let segments = parse_segments("<alpha=#80><#FF0000>a<#00FF0080>b", 30.0);
    assert_eq!(segments[0].alpha, 255);
    assert_eq!(segments[1].alpha, 128);

    let segments = parse_segments("<color=#ff0000>R</color>G", 30.0);
    assert_eq!(segments[0].color, Some([255, 0, 0]));
    assert_eq!(segments[1].text, "G");
    assert_eq!(segments[1].color, None);

    let segments = parse_segments("<alpha=#40>AB<color=#ff0000>C", 30.0);
    assert_eq!((segments[0].color, segments[0].alpha), (None, 0x40));
    assert_eq!(
        (segments[1].color, segments[1].alpha),
        (Some([255, 0, 0]), 255)
    );
}

#[test]
fn spacing_lengths_resolve_their_unit_at_the_tag() {
    assert_eq!(one("<cspace=10>x").character_spacing, 10.0);
    assert_eq!(one("<cspace=0.5em>x").character_spacing, 15.0);
    assert_eq!(one("<voffset=1em>x").baseline_offset, 30.0);
    assert_eq!(one("<voffset=12px>x").baseline_offset, 12.0);
    // The size in effect at the tag decides an em value, not the size of
    // the characters that follow.
    assert_eq!(one("<cspace=1em><size=60>x").character_spacing, 30.0);
    assert_eq!(one("<mspace=2em>x").monospace, Some(60.0));
}

#[test]
fn closing_cspace_takes_the_spacing_back_from_the_last_character() {
    let segments = parse_segments("<cspace=10>AB</cspace>C", 30.0);
    assert_eq!(segments[0].text, "AB");
    assert_eq!(segments[0].character_spacing, 10.0);
    assert_eq!(segments[1].caret, Some(CaretCommand::Advance(-10.0)));
    assert_eq!(segments[2].text, "C");
    assert_eq!(segments[2].character_spacing, 0.0);
    // Nothing to take back before the first character.
    let segments = parse_segments("<cspace=10></cspace>C", 30.0);
    assert_eq!(segments.len(), 1);
}

#[test]
fn space_and_position_are_caret_commands() {
    let segments = parse_segments("A<space=50>B<space=1em>C", 30.0);
    assert_eq!(segments[1].caret, Some(CaretCommand::Advance(50.0)));
    assert_eq!(segments[3].caret, Some(CaretCommand::Advance(30.0)));
    let segments = parse_segments("<pos=50%>A<pos=2em>B", 30.0);
    assert_eq!(
        segments[0].caret,
        Some(CaretCommand::MoveTo(HorizontalOffset::Percent(50.0)))
    );
    assert_eq!(
        segments[2].caret,
        Some(CaretCommand::MoveTo(HorizontalOffset::Units(60.0)))
    );
}

#[test]
fn indents_accept_pixels_font_units_and_percentages() {
    let segments = parse_segments("<indent=100>A<indent=50%>B<indent=1em>C", 30.0);
    assert_eq!(segments[0].indent, Some(HorizontalOffset::Units(100.0)));
    assert_eq!(segments[1].indent, Some(HorizontalOffset::Percent(50.0)));
    assert_eq!(segments[2].indent, Some(HorizontalOffset::Units(30.0)));
    let segments = parse_segments(
        "<line-indent=20>A<line-indent=1em>B<line-indent=50%>C",
        30.0,
    );
    assert_eq!(segments[0].line_indent, Some(HorizontalOffset::Units(20.0)));
    assert_eq!(segments[1].line_indent, Some(HorizontalOffset::Units(30.0)));
    assert_eq!(
        segments[2].line_indent,
        Some(HorizontalOffset::Percent(50.0))
    );
}

#[test]
fn line_height_resolves_pixels_font_units_and_face_percentages() {
    assert_eq!(one("<line-height=20>x").line_height, Some(20.0));
    assert_eq!(one("<line-height=1em>x").line_height, Some(30.0));
    // 50% of the 150-unit face line height at 30 / 75 * 2 layout units per
    // design unit.
    assert_eq!(one("<line-height=50%>x").line_height, Some(60.0));
    let segments = parse_segments("<line-height=20>a</line-height>b", 30.0);
    assert_eq!(segments[1].line_height, None);
}

#[test]
fn sizes_resolve_against_the_element_size() {
    let segments = parse_segments(
        "<size=40>A<size=+10>B<size=-5>C<size=150%>D<size=1.5em>E</size>F",
        30.0,
    );
    let sizes: Vec<f32> = segments
        .iter()
        .flat_map(|segment| segment.text.chars().map(|_| segment.font_size))
        .collect();
    assert_eq!(sizes, [40.0, 40.0, 25.0, 45.0, 45.0, 45.0]);
}

#[test]
fn super_and_subscript_shift_the_baseline_by_the_face_offsets() {
    let face = PROFILE_FACE;
    let sup = &parse_segments("<sup>x", 75.0)[0];
    assert_eq!(sup.font_scale, face.superscript_size);
    // Positive is upward: 66 design units at 75pt, scale 2, half size.
    assert_eq!(sup.baseline_offset, 66.0);
    let sub = &parse_segments("<sub>x", 75.0)[0];
    assert_eq!(sub.baseline_offset, -9.0);
    let segments = parse_segments("<sup>a</sup>b", 75.0);
    assert_eq!(
        (segments[1].font_scale, segments[1].baseline_offset),
        (1.0, 0.0)
    );
    // A voffset replaces the shift; closing it returns to the baseline.
    let segments = parse_segments("<sup><voffset=5>a</voffset>b", 75.0);
    assert_eq!(segments[0].baseline_offset, 5.0);
    assert_eq!(segments[1].baseline_offset, 0.0);
}

#[test]
fn italic_uses_the_face_angle_or_the_angle_attribute() {
    assert_eq!(one("<i>x").italic, Some(PROFILE_FACE.italic_style));
    assert_eq!(one("<i angle=20>x").italic, Some(20));
    let segments = parse_segments("<i>a</i>b", 30.0);
    assert_eq!(segments[1].italic, None);
}

#[test]
fn case_tags_map_one_character_to_one() {
    let upper = one("<uppercase>aß</uppercase>");
    assert_eq!(upper.transform_char('a'), ('A', 1.0));
    assert_eq!(upper.transform_char('ß'), ('ß', 1.0));
    let small = one("<smallcaps>abc</smallcaps>");
    assert_eq!(small.case, CaseTransform::SmallCaps);
    assert_eq!(small.transform_char('a'), ('A', SMALL_CAPS_SCALE));
    assert_eq!(small.transform_char('A'), ('A', 1.0));
    // Uppercase wins over small caps when both are open.
    assert_eq!(one("<smallcaps><allcaps>a").case, CaseTransform::Upper);
}

#[test]
fn scale_and_rotate_share_one_transform() {
    let segment = one("<scale=2><rotate=10>x");
    assert_eq!((segment.scale, segment.rotate), (None, Some(10.0)));
    let segments = parse_segments("<rotate=10>a</scale>b", 30.0);
    assert_eq!((segments[1].scale, segments[1].rotate), (None, None));
}

#[test]
fn alignment_accepts_the_tmp_values() {
    assert_eq!(
        one("<align=center>AB</align>").align,
        Some(InlineAlign::Center)
    );
    assert_eq!(
        one("<align=justified>x").align,
        Some(InlineAlign::Justified)
    );
}

#[test]
fn mark_colour_and_alpha_follow_the_attribute_forms() {
    assert_eq!(
        one("<mark=#FF000040>a").mark,
        Some([0xFF, 0x00, 0x00, 0x40])
    );
    assert_eq!(one("<mark=#FF0000>a").mark, Some([0xFF, 0x00, 0x00, 0xFF]));
    assert_eq!(one("<mark>a").mark, Some([255, 255, 0, 64]));
    assert_eq!(one("<mark=#日1日1>a").mark, Some([255, 255, 255, 255]));
    let segments = parse_segments("<mark>a</mark>b", 30.0);
    assert_eq!(segments[1].mark, None);
}

#[test]
fn mspace_ignores_attributes_it_does_not_know() {
    let segment = one("<mspace=10 duospace=1>a");
    assert_eq!(segment.monospace, Some(10.0));
}

#[test]
fn noparse_keeps_tags_literal_but_line_breaks_still_apply() {
    assert_eq!(text(&parse_segments("<noparse><b></noparse>", 30.0)), "<b>");
    assert_eq!(
        text(&parse_segments("<noparse>a<br>b</noparse>", 30.0)),
        "a\nb"
    );
    assert_eq!(text(&parse_segments("a</noparse>b", 30.0)), "ab");
}

#[test]
fn resource_tags_accept_only_the_default_asset_and_style_closers_vanish() {
    assert_eq!(
        text(&parse_segments(
            "<font=default>a</font><material=Default>b</material>",
            30.0
        )),
        "ab"
    );
    assert_eq!(text(&parse_segments("a</style>b", 30.0)), "ab");
}

#[test]
fn visible_scalars_point_back_at_their_source() {
    let scalars = visible_scalars("a<br>b<nobr>c</nobr>");
    let summary: Vec<_> = scalars
        .iter()
        .map(|scalar| (scalar.ch, scalar.source.clone(), scalar.no_break))
        .collect();
    assert_eq!(
        summary,
        [
            ('a', 0..1, false),
            ('\n', 1..5, false),
            ('b', 5..6, false),
            ('c', 12..13, true)
        ]
    );
}

#[test]
fn lines_keep_caret_commands_where_they_occur() {
    let segments = parse_segments("A<space=50>B\n<b>C</b>\n", 30.0);
    let lines = split_lines(&segments);
    assert_eq!(lines.len(), 2);
    let first: Vec<_> = lines[0]
        .iter()
        .map(|piece| (piece.text, piece.segment.caret))
        .collect();
    assert_eq!(
        first,
        [
            ("A", None),
            ("", Some(CaretCommand::Advance(50.0))),
            ("B", None)
        ]
    );
    assert_eq!(lines[1].len(), 1);
    assert!(lines[1][0].segment.bold);
}
