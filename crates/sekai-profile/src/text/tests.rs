use super::effective_vertex_alpha;
use sekai_profile_core::tmp_text;

/// Metrics come from DejaVu Sans, the one face the build container ships;
/// these layout checks skip when it is not installed.
const LAYOUT_TEST_FAMILY: &str = "DejaVu Sans";

struct LayoutTestFont;
impl crate::masterdata::MasterDataProvider for LayoutTestFont {
    fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> {
        None
    }
    fn get_card(&self, _: i32) -> Option<crate::types::CardEntry> {
        None
    }
    fn resolve_color(&self, _: i32) -> Option<crate::masterdata::ResolvedColor> {
        Some(crate::masterdata::ResolvedColor {
            r: 10,
            g: 20,
            b: 30,
            a: 255,
        })
    }
    fn resolve_font(&self, _: i32) -> Option<String> {
        Some(LAYOUT_TEST_FAMILY.into())
    }
    fn resolve_stamp(&self, _: i32) -> Option<String> {
        None
    }
    fn resolve_resource(&self, _: &str, _: i32) -> Option<crate::masterdata::ResourceInfo> {
        None
    }
    fn resolve_honor(&self, _: i32, _: i32) -> Option<crate::masterdata::ResolvedHonor> {
        None
    }
    fn get_bonds_honor(&self, _: i32) -> Option<crate::types::BondsHonorEntry> {
        None
    }
    fn get_bonds_honor_word(&self, _: i64) -> Option<crate::types::BondsHonorWordEntry> {
        None
    }
    fn get_honor(&self, _: i32) -> Option<crate::types::HonorEntry> {
        None
    }
    fn resolve_unit_vs_sd(&self, id: i32, _: i32) -> i32 {
        id
    }
    fn font_count(&self) -> usize {
        1
    }
    fn color_count(&self) -> usize {
        1
    }
}

fn layout_test_md() -> Option<crate::masterdata::MasterData> {
    if crate::sdf::outline::load_font_bytes_for_family(LAYOUT_TEST_FAMILY).is_none() {
        eprintln!("skipping: {LAYOUT_TEST_FAMILY} is not installed");
        return None;
    }
    Some(crate::masterdata::MasterData::new(std::sync::Arc::new(
        LayoutTestFont,
    )))
}

fn layout_test_element(
    text: &str,
    size: f32,
    line_spacing: f32,
    text_type: i32,
) -> crate::types::TextElement {
    serde_json::from_value(serde_json::json!({
        "objectData": {
            "layer": 0, "lock": false, "visible": true,
            "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
        },
        "colorId": 1, "fontId": 1, "lineSpacing": line_spacing,
        "outlineColorId": 1, "outlineSize": 0.0, "size": size,
        "text": text, "type": text_type
    }))
    .expect("layout test element")
}

fn layout_ops(
    text: &str,
    size: f32,
    line_spacing: f32,
    text_type: i32,
) -> Option<Vec<super::DrawCharOp>> {
    let md = layout_test_md()?;
    let element = layout_test_element(text, size, line_spacing, text_type);
    Some(
        super::layout_text_ops(&element, &md, None, None, None, false)
            .expect("text layout")
            .draw_ops,
    )
}

fn test_advance(ch: char, size: f32) -> f32 {
    super::freetype_advance_x(None, Some(LAYOUT_TEST_FAMILY), ch, ch, size)
        .expect("test font advance")
}

fn assert_close(actual: f32, expected: f32, what: &str) {
    assert!(
        (actual - expected).abs() < 1e-3,
        "{what}: {actual} != {expected}"
    );
}

#[test]
fn superscript_rises_and_subscript_drops_by_the_face_offsets() {
    let Some(ops) = layout_ops("A<sup>B</sup>", 30.0, 0.0, 1) else {
        return;
    };
    // 66 design units at 30 / 75 * 2 per unit, half size, in draw space.
    assert_close(
        ops[0].y - ops[1].y,
        66.0 * 0.8 * 0.5 / 2.0,
        "superscript rise",
    );
    assert_close(ops[1].font_size, 15.0, "superscript size");
    let ops = layout_ops("A<sub>B</sub>", 30.0, 0.0, 1).expect("test font");
    assert_close(ops[1].y - ops[0].y, 9.0 * 0.8 * 0.5 / 2.0, "subscript drop");
}

#[test]
fn smallcaps_shrink_the_glyph_on_both_axes() {
    let Some(ops) = layout_ops("<smallcaps>a</smallcaps>", 30.0, 0.0, 1) else {
        return;
    };
    assert_eq!(ops[0].ch, "A");
    assert_close(ops[0].font_size, 24.0, "small caps size");
    assert_close(ops[0].scale_x, 1.0, "small caps horizontal scale");
}

#[test]
fn italic_slant_matches_its_shear_centre() {
    let Some(ops) = layout_ops("<i>A</i><i angle=20>B</i>", 30.0, 0.0, 1) else {
        return;
    };
    assert_close(ops[0].skew_x, -0.35, "default italic slant");
    assert_close(ops[1].skew_x, -0.20, "italic angle attribute");
}

#[test]
fn closing_cspace_takes_the_spacing_back_from_the_last_character() {
    let Some(ops) = layout_ops("<cspace=10>AB</cspace>C", 30.0, 0.0, 1) else {
        return;
    };
    assert_close(
        ops[1].x - ops[0].x,
        test_advance('A', 30.0) + 5.0,
        "spacing inside",
    );
    assert_close(
        ops[2].x - ops[1].x,
        test_advance('B', 30.0),
        "spacing taken back",
    );
}

#[test]
fn space_tags_move_the_caret_on_their_own_line_only() {
    let Some(ops) = layout_ops("A<space=50>B", 30.0, 0.0, 1) else {
        return;
    };
    assert_close(
        ops[1].x - ops[0].x,
        test_advance('A', 30.0) + 25.0,
        "space advance",
    );
    // Centred: the second line holds no space and centres on its glyph.
    let ops = layout_ops("<space=50>A\nA", 30.0, 0.0, 2).expect("test font");
    assert_close(
        ops[1].x,
        -test_advance('A', 30.0) / 2.0,
        "second line start",
    );
}

#[test]
fn line_indent_pixels_are_layout_units() {
    let Some(plain) = layout_ops("A", 30.0, 0.0, 1) else {
        return;
    };
    let indented = layout_ops("<line-indent=20>A", 30.0, 0.0, 1).expect("test font");
    assert_close(indented[0].x - plain[0].x, 10.0, "line indent");
    let em = layout_ops("<indent=10><line-indent=1em>A", 30.0, 0.0, 1).expect("test font");
    assert_close(em[0].x - plain[0].x, 20.0, "indent plus em line indent");
}

#[test]
fn line_spacing_uses_the_game_factor() {
    let Some(tight) = layout_ops("A\nA", 300.0, 0.0, 1) else {
        return;
    };
    let spaced = layout_ops("A\nA", 300.0, 1.0, 1).expect("test font");
    let gap = |ops: &[super::DrawCharOp]| ops[1].y - ops[0].y;
    // One unit of spacing is 2 * 1.325 * 300 / 100 layout units.
    assert_close(
        gap(&spaced) - gap(&tight),
        2.0 * 1.325 * 3.0 / 2.0,
        "line spacing",
    );
}

#[test]
fn a_character_no_font_has_is_drawn_as_the_missing_glyph_square() {
    let Some(ops) = layout_ops("A\u{6F22}B", 30.0, 0.0, 1) else {
        return;
    };
    assert_eq!(ops[1].ch, "\u{25A1}");
    assert_close(
        ops[2].x - ops[0].x,
        test_advance('A', 30.0) + test_advance('\u{25A1}', 30.0),
        "square advance",
    );
    let md = layout_test_md().expect("test font");
    let element = layout_test_element("A\u{6F22}B A\u{6F22}B", 30.0, 0.0, 1);
    let wrapped = super::wrap_rich_text_to_width(&element, &md, 90.0).expect("wrapped text");
    assert!(wrapped.contains('\n'), "{wrapped:?}");
}

#[test]
fn unknown_font_and_color_ids_draw_with_the_default_rows() {
    use crate::masterdata::{MasterData, MasterDataProvider, ResolvedColor};
    use std::sync::Arc;

    const FIRST_ROW: ResolvedColor = ResolvedColor {
        r: 10,
        g: 20,
        b: 30,
        a: 255,
    };

    struct FirstRowsOnly;
    impl MasterDataProvider for FirstRowsOnly {
        fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> {
            None
        }
        fn get_card(&self, _: i32) -> Option<crate::types::CardEntry> {
            None
        }
        fn resolve_color(&self, color_id: i32) -> Option<ResolvedColor> {
            (color_id == 1).then_some(FIRST_ROW)
        }
        fn default_color(&self) -> Option<ResolvedColor> {
            Some(FIRST_ROW)
        }
        fn resolve_font(&self, font_id: i32) -> Option<String> {
            (font_id == 1).then(|| LAYOUT_TEST_FAMILY.into())
        }
        fn resolve_stamp(&self, _: i32) -> Option<String> {
            None
        }
        fn resolve_resource(&self, _: &str, _: i32) -> Option<crate::masterdata::ResourceInfo> {
            None
        }
        fn resolve_honor(&self, _: i32, _: i32) -> Option<crate::masterdata::ResolvedHonor> {
            None
        }
        fn get_bonds_honor(&self, _: i32) -> Option<crate::types::BondsHonorEntry> {
            None
        }
        fn get_bonds_honor_word(&self, _: i64) -> Option<crate::types::BondsHonorWordEntry> {
            None
        }
        fn get_honor(&self, _: i32) -> Option<crate::types::HonorEntry> {
            None
        }
        fn resolve_unit_vs_sd(&self, id: i32, _: i32) -> i32 {
            id
        }
        fn font_count(&self) -> usize {
            1
        }
        fn color_count(&self) -> usize {
            1
        }
    }

    if crate::sdf::outline::load_font_bytes_for_family(LAYOUT_TEST_FAMILY).is_none() {
        eprintln!("skipping: {LAYOUT_TEST_FAMILY} is not installed");
        return;
    }
    let md = MasterData::new(Arc::new(FirstRowsOnly));
    let element = |font_id: i32, color_id: i32, outline_color_id: i32| {
        let mut element = layout_test_element("<line-indent=15%>AB", 24.0, 0.0, 1);
        element.font_id = font_id;
        element.color_id = color_id;
        element.outline_color_id = outline_color_id;
        element.outline_size = 0.2;
        element
    };
    let capture = |text: &crate::types::TextElement| {
        let mut glyphs = Vec::new();
        let mut observer =
            |result: Result<super::ResolvedTextSdfGlyph, super::TextSdfCaptureError>| {
                glyphs.push(result.expect("captured glyph"));
            };
        super::capture_text_sdf_from_affine(
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            text,
            &md,
            None,
            &mut observer,
        );
        glyphs
    };
    let known = element(1, 1, 1);
    let unknown = element(404, 405, 406);
    let expected = capture(&known);
    assert!(!expected.is_empty(), "the known rows must draw glyphs");
    assert_eq!(capture(&unknown), expected);
    let program = super::line_indent_program(&known, &md);
    assert!(program.is_some());
    assert_eq!(super::line_indent_program(&unknown, &md), program);
}

#[test]
fn unrecognised_tags_are_laid_out_as_text() {
    let Some(ops) = layout_ops("<love>", 30.0, 0.0, 1) else {
        return;
    };
    let text: String = ops.iter().map(|op| op.ch.as_str()).collect();
    assert_eq!(text, "<love>");
}

#[test]
fn sdf_capture_rejects_decoration_spans_without_emitting_partial_glyphs() {
    use crate::masterdata::{MasterData, MasterDataProvider};
    use std::sync::Arc;

    struct NoFonts;
    impl MasterDataProvider for NoFonts {
        fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> {
            None
        }
        fn get_card(&self, _: i32) -> Option<crate::types::CardEntry> {
            None
        }
        fn resolve_color(&self, _: i32) -> Option<crate::masterdata::ResolvedColor> {
            None
        }
        fn resolve_font(&self, _: i32) -> Option<String> {
            panic!("unsupported decoration must be rejected before font lookup");
        }
        fn resolve_stamp(&self, _: i32) -> Option<String> {
            None
        }
        fn resolve_resource(&self, _: &str, _: i32) -> Option<crate::masterdata::ResourceInfo> {
            None
        }
        fn resolve_honor(&self, _: i32, _: i32) -> Option<crate::masterdata::ResolvedHonor> {
            None
        }
        fn get_bonds_honor(&self, _: i32) -> Option<crate::types::BondsHonorEntry> {
            None
        }
        fn get_bonds_honor_word(&self, _: i64) -> Option<crate::types::BondsHonorWordEntry> {
            None
        }
        fn get_honor(&self, _: i32) -> Option<crate::types::HonorEntry> {
            None
        }
        fn resolve_unit_vs_sd(&self, id: i32, _: i32) -> i32 {
            id
        }
        fn font_count(&self) -> usize {
            0
        }
        fn color_count(&self) -> usize {
            0
        }
    }

    let md = MasterData::new(Arc::new(NoFonts));
    for (text, feature) in [
        ("plain <u>underlined</u>", "text underline"),
        ("plain <s>struck</s>", "text strikethrough"),
        ("plain <mark=#ff0000>marked</mark>", "text mark"),
        ("<u> </u>", "text underline"),
    ] {
        let element = serde_json::from_value(serde_json::json!({
            "objectData": {
                "layer": 0, "lock": false, "visible": true,
                "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
            },
            "colorId": 1, "fontId": 1, "lineSpacing": 0.0,
            "outlineColorId": 1, "outlineSize": 0.0, "size": 24.0,
            "text": text, "type": 1
        }))
        .unwrap();
        for placed in [false, true] {
            let mut events = Vec::new();
            let mut observer = |event| events.push(event);
            let affine = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
            if placed {
                super::capture_text_sdf_with_placement(
                    affine,
                    &element,
                    &md,
                    None,
                    super::TextRenderPlacement {
                        anchor_x: 0.0,
                        baseline: None,
                    },
                    None,
                    &mut observer,
                );
            } else {
                super::capture_text_sdf_from_affine(affine, &element, &md, None, &mut observer);
            }
            assert_eq!(events.len(), 1, "text={text}, placed={placed}");
            assert!(matches!(
                events[0],
                Err(super::TextSdfCaptureError::UnsupportedFeature { feature: value })
                    if value == feature
            ));
        }
    }
}

#[test]
fn sdf_capture_accepts_literal_markup_empty_decorations_and_supported_styles() {
    for text in [
        "plain text",
        "<b>bold</b><i>italic</i><color=#ff0000>red</color>",
        "<noparse><u>literal</u><s>literal</s><mark=#ff0000>literal</mark></noparse>",
        "<u></u><s></s><mark=#ff0000></mark>plain",
        "<u>\n</u><s>\n</s><mark=#ff0000>\n</mark>",
    ] {
        let segments = tmp_text::parse_segments(text, 24.0);
        assert_eq!(
            super::validate_sdf_text_segments(&segments),
            Ok(()),
            "text={text}"
        );
    }
}

#[test]
fn rich_text_parsing_retains_decoration_semantics_for_non_capture_consumers() {
    let segments = tmp_text::parse_segments("<u>A</u><s>B</s><mark=#ff000040>C</mark>", 24.0);
    assert_eq!(segments.len(), 3);
    assert_eq!(segments[0].text, "A");
    assert!(segments[0].underline);
    assert_eq!(segments[1].text, "B");
    assert!(segments[1].strikethrough);
    assert_eq!(segments[2].text, "C");
    assert_eq!(segments[2].mark, Some([255, 0, 0, 64]));
}

/// A capture given the outline as resolved RGBA must produce exactly the
/// glyph stream the color-table route produces for the same color, and a
/// zero-width override must keep the underlay on the face edge.
#[test]
fn outline_override_matches_the_color_table_route() {
    use std::sync::Arc;

    use crate::masterdata::{MasterData, MasterDataProvider, ResolvedColor};
    use crate::types::{ObjectData, Quaternion, TextElement, Vec3};

    struct OutlineProvider;
    impl MasterDataProvider for OutlineProvider {
        fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> {
            None
        }
        fn get_card(&self, _: i32) -> Option<crate::types::CardEntry> {
            None
        }
        fn resolve_color(&self, color_id: i32) -> Option<ResolvedColor> {
            (color_id == 7).then_some(ResolvedColor {
                r: 204,
                g: 51,
                b: 25,
                a: 230,
            })
        }
        fn resolve_font(&self, _: i32) -> Option<String> {
            Some("FZLanTingHei-DB-GBK".into())
        }
        fn resolve_stamp(&self, _: i32) -> Option<String> {
            None
        }
        fn resolve_resource(&self, _: &str, _: i32) -> Option<crate::masterdata::ResourceInfo> {
            None
        }
        fn resolve_honor(&self, _: i32, _: i32) -> Option<crate::masterdata::ResolvedHonor> {
            None
        }
        fn get_bonds_honor(&self, _: i32) -> Option<crate::types::BondsHonorEntry> {
            None
        }
        fn get_bonds_honor_word(&self, _: i64) -> Option<crate::types::BondsHonorWordEntry> {
            None
        }
        fn get_honor(&self, _: i32) -> Option<crate::types::HonorEntry> {
            None
        }
        fn resolve_unit_vs_sd(&self, _: i32, _: i32) -> i32 {
            0
        }
        fn font_count(&self) -> usize {
            1
        }
        fn color_count(&self) -> usize {
            1
        }
    }

    if crate::sdf::outline::load_font_bytes_for_family("FZLanTingHei-DB-GBK").is_none() {
        eprintln!("skipping: FONT_DIR does not provide the test family");
        return;
    }

    let element = |outline_size: f32| TextElement {
        object_data: ObjectData {
            layer: 0,
            lock: false,
            position: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            rotation: Quaternion {
                w: 1.0,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            scale: Vec3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            },
            visible: true,
        },
        color_id: 7,
        font_id: 1,
        line_spacing: 0.0,
        outline_color_id: 7,
        outline_size,
        size: 24.0,
        text: "AB7".into(),
        text_type: 2,
    };
    let capture = |text: &TextElement, outline: Option<super::TextOutlineOverride>| {
        let mut glyphs = Vec::new();
        let mut observer =
            |result: Result<super::ResolvedTextSdfGlyph, super::TextSdfCaptureError>| {
                glyphs.push(result.expect("captured glyph"));
            };
        let md = MasterData::new(Arc::new(OutlineProvider));
        // The element is centre-aligned, so a zero-anchor placement is the
        // identity translation.
        super::capture_text_sdf_with_placement(
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            text,
            &md,
            None,
            super::TextRenderPlacement {
                anchor_x: 0.0,
                baseline: None,
            },
            outline,
            &mut observer,
        );
        glyphs
    };

    let table_route = capture(&element(0.4), None);
    assert!(!table_route.is_empty(), "capture must produce glyphs");
    let override_route = capture(
        &element(0.0),
        Some(super::TextOutlineOverride {
            rgba: [204.0 / 255.0, 51.0 / 255.0, 25.0 / 255.0, 230.0 / 255.0],
            size: 0.4,
        }),
    );
    assert_eq!(
        override_route, table_route,
        "the override must reproduce the color-table glyph stream"
    );

    let zero_width = capture(
        &element(0.4),
        Some(super::TextOutlineOverride {
            rgba: [204.0 / 255.0, 51.0 / 255.0, 25.0 / 255.0, 230.0 / 255.0],
            size: 0.0,
        }),
    );
    let plain = capture(&element(0.0), None);
    assert_eq!(
        zero_width, plain,
        "a zero-width override must match the zero-width color-table route"
    );
    assert!(
        plain.iter().all(|glyph| glyph.material.outline[3] > 0.0),
        "a zero-width outline still draws the underlay"
    );
    assert_ne!(
        table_route, plain,
        "the outline must actually change the glyph materials"
    );
}

#[test]
fn multiline_line_indent_preserves_every_visible_line_for_feedback() {
    use sekai_profile_core::MeasuredTextUnit;

    let units = vec![
        MeasuredTextUnit {
            advance: 0.0,
            hard_break: true,
        },
        MeasuredTextUnit {
            advance: 12.0,
            hard_break: false,
        },
        MeasuredTextUnit {
            advance: 18.0,
            hard_break: false,
        },
        MeasuredTextUnit {
            advance: 0.0,
            hard_break: true,
        },
        MeasuredTextUnit {
            advance: 20.0,
            hard_break: false,
        },
    ];

    let selected = super::group_line_advances_tmp(&units, &[false, true, true]).unwrap();
    assert_eq!(selected, vec![vec![12.0, 18.0], vec![20.0]]);
}

#[test]
fn effective_vertex_alpha_caps_override_by_base_alpha() {
    let alpha = effective_vertex_alpha(204, 128);
    assert!((alpha - (128.0 / 255.0)).abs() < 1e-6);
}

#[test]
fn effective_vertex_alpha_uses_override_when_lower_than_base() {
    let alpha = effective_vertex_alpha(64, 255);
    assert!((alpha - (64.0 / 255.0)).abs() < 1e-6);
}

#[test]
fn effective_vertex_alpha_falls_back_to_base_alpha() {
    let alpha = effective_vertex_alpha(255, 64);
    assert!((alpha - (64.0 / 255.0)).abs() < 1e-6);
}

#[test]
fn render_translation_anchors_completed_layout_without_changing_its_metrics() {
    let left = super::text_render_translation(1, 100.0, 4.2, -30.0, Some(1.8));
    let center = super::text_render_translation(2, 100.0, 4.2, 0.0, None);
    let right = super::text_render_translation(4, 100.0, 4.2, 30.0, None);
    assert!((left.0 - 20.0).abs() < 1e-6);
    assert!((left.1 + 2.4).abs() < 1e-6);
    assert_eq!(center, (0.0, 0.0));
    assert!((right.0 + 20.0).abs() < 1e-6);
    assert_eq!(right.1, 0.0);
}

#[test]
fn captured_sdf_glyph_rejects_non_scalar_text_runs() {
    let captured = super::ResolvedTextSdfGlyph {
        text: "AB".into(),
        font_family: Some("test".into()),
        baseline_origin: crate::sdf::tile::Point2::new(0.0, 0.0),
        font_size: 12.0,
        local_to_device: crate::sdf::tile::Affine2::IDENTITY,
        material: crate::sdf::tile::SdfMaterial::default(),
    };
    assert_eq!(
        captured.single_codepoint(),
        Err(super::TextSdfCommandError::NotSingleScalar)
    );
}

/// A captured glyph's device transform is the affine base composed with the
/// glyph's local transform, under rotation, non-uniform scale and
/// fractional translation alike. The pinned values are exact bit patterns.
#[test]
fn affine_capture_composes_the_base_with_the_glyph_transform() {
    // (device base, resolved local-to-device transform)
    let cases: [([f32; 6], [f32; 6]); 4] = [
        (
            [1.0, 0.0, 0.0, 1.0, 80.7, -20.3],
            [1.078158, -0.81245023, 0.3754018, 0.9692501, 100.2, -31.05],
        ),
        (
            [2.0, 0.0, 0.0, 2.0, 0.25, 0.75],
            [2.156316, -1.6249005, 0.7508036, 1.9385002, 39.25, -20.75],
        ),
        (
            [0.9271839, 0.3746066, -0.3746066, 0.9271839, 17.0, -9.0],
            [
                1.304,
                -0.34940565,
                -0.0150209665,
                1.0393012,
                39.10711,
                -11.662399,
            ],
        ),
        (
            [1.2, -0.15, 0.35, 0.8, -3.25, 41.5],
            [
                1.0094321, -0.8116839, 0.7897197, 0.7190898, 16.3875, 29.975002,
            ],
        ),
    ];
    let face = [0.2f32, 0.4, 0.6, 0.75];
    let op = super::DrawCharOp {
        ch: "字".into(),
        x: 13.25,
        y: -8.5,
        pivot_x: 4.75,
        pivot_y: -2.25,
        half_w: 8.5,
        half_h: 11.25,
        shear_cx: 1.5,
        scale_x: 1.35,
        skew_x: -0.21,
        rotate_deg: 37.0,
        font_size: 0.0,
        face,
        sdf_params: None,
        mesh_carrier: crate::sdf::material::runtime_like_mesh_carrier(24.0, true, 193),
    };
    for (base, expected) in cases {
        let glyph = super::resolve_text_sdf_glyph_from_base_affine(base, &op, Some("test-font"))
            .expect("affine route");
        let device = glyph.local_to_device;
        let actual = [
            device.scale_x,
            device.skew_y,
            device.skew_x,
            device.scale_y,
            device.translate_x,
            device.translate_y,
        ];
        assert_eq!(
            actual.map(f32::to_bits),
            expected.map(f32::to_bits),
            "base {base:?}: {actual:?}"
        );
        assert_eq!(glyph.text, "字");
        assert_eq!(glyph.font_family.as_deref(), Some("test-font"));
        assert_eq!(
            glyph.baseline_origin,
            crate::sdf::tile::Point2::new(-op.pivot_x, -op.pivot_y)
        );
    }
}

/// The glyph transform is pinned bit for bit on every composition the
/// layout can produce: plain, rotated (including a rotation below the
/// threshold), scaled, italic-skewed, and all of those combined. The 36
/// transforms are digested in loop order.
#[test]
fn glyph_local_affine_composes_rotation_scale_and_skew_exactly() {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    let mut case = 0u32;
    for rotate_deg in [0.0f32, 0.0005, 37.0, -218.4, 90.0, 179.99] {
        for scale_x in [1.0f32, 1.35, 0.4821] {
            for skew_x in [0.0f32, -0.21] {
                let op = super::DrawCharOp {
                    ch: "字".into(),
                    x: 13.25,
                    y: -8.5,
                    pivot_x: 4.75,
                    pivot_y: -2.25,
                    half_w: 8.5,
                    half_h: 11.25,
                    shear_cx: 1.5,
                    scale_x,
                    skew_x,
                    rotate_deg,
                    font_size: 24.0,
                    face: [0.2, 0.4, 0.6, 1.0],
                    sdf_params: None,
                    mesh_carrier: crate::sdf::material::runtime_like_mesh_carrier(24.0, false, 255),
                };
                let affine = super::glyph_local_affine(&op);
                if rotate_deg.abs() <= 0.001 {
                    // No rotation: the X scale stays on its lane, Y is
                    // untouched and the translation is the anchor.
                    assert_eq!(
                        [affine[0], affine[1], affine[3], affine[4], affine[5]].map(f32::to_bits),
                        [scale_x, 0.0, 1.0, 19.5, -10.75].map(f32::to_bits),
                        "scale {scale_x} skew {skew_x}"
                    );
                    if skew_x == 0.0 {
                        assert_eq!(affine[2].to_bits(), 0.0f32.to_bits());
                    }
                }
                for value in affine {
                    hasher.update(value.to_bits().to_le_bytes());
                }
                case += 1;
            }
        }
    }
    assert_eq!(case, 36);
    assert_eq!(
        hex::encode(hasher.finalize()),
        "f950a70f677a93b78fdc482cbd7b0cdd6b1c65d1536db706e135007f4bbf398c"
    );
}

/// The debug footprint is the atlas rect the rasterizer samples: the glyph
/// ink box inflated by the sampling spread on every side, centred on the
/// ink centre. A `<scale>` tag stretches it along X only and a `<rotate>`
/// tag merely reorients it, so the pair of side lengths is a
/// rotation-invariant signature of the device footprint.
#[test]
fn glyph_quad_footprint_is_the_padded_ink_box_stretched_on_x_only() {
    for rotate_deg in [0.0f32, 37.0, -218.4, 90.0] {
        for scale_x in [1.0f32, 1.2, 6.0] {
            let op = super::DrawCharOp {
                ch: "\u{25cf}".into(),
                x: 13.25,
                y: -8.5,
                pivot_x: 4.75,
                pivot_y: -2.25,
                half_w: 8.5,
                half_h: 11.25,
                shear_cx: 0.0,
                scale_x,
                skew_x: 0.0,
                rotate_deg,
                font_size: 24.0,
                face: [0.2, 0.4, 0.6, 1.0],
                sdf_params: None,
                mesh_carrier: crate::sdf::material::runtime_like_mesh_carrier(24.0, false, 255),
            };
            let quad = super::glyph_quad_corners(&op);
            let side = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).hypot(a.1 - b.1);
            let mut got = [side(quad[0], quad[1]), side(quad[1], quad[2])];
            let mut want = [
                2.0 * op.half_w * scale_x * super::TEXT_SCALE,
                2.0 * op.half_h * super::TEXT_SCALE,
            ];
            got.sort_by(|a, b| a.partial_cmp(b).expect("finite side"));
            want.sort_by(|a, b| a.partial_cmp(b).expect("finite side"));
            for (got, want) in got.iter().zip(want.iter()) {
                assert!(
                    (got - want).abs() <= 1e-3,
                    "rotate {rotate_deg} scale {scale_x}: got {got} want {want}"
                );
            }
        }
    }
}

#[test]
fn captured_sdf_glyph_maps_manifest_metrics_without_relayout() {
    let captured = super::ResolvedTextSdfGlyph {
        text: "字".into(),
        font_family: Some("test".into()),
        baseline_origin: crate::sdf::tile::Point2::new(10.0, 20.0),
        font_size: 20.0,
        local_to_device: crate::sdf::tile::Affine2::IDENTITY,
        material: crate::sdf::tile::SdfMaterial::default(),
    };
    let glyph = crate::sdf::atlas::SdfAtlasGlyphManifest {
        codepoint: u32::from('字'),
        page: 2,
        rect: [32, 64, 12, 14],
        plane_bearing: [2.0, 7.0],
        plane_size: [4.0, 5.0],
        plane_advance_x: 4.5,
    };
    let command = captured
        .to_sdf_command_from_manifest(7, &glyph, 10.0, 1.0)
        .expect("captured glyph command");
    assert_eq!(command.atlas_page, 2);
    assert_eq!(command.atlas_set, 7);
    assert_eq!(command.atlas_rect, [32, 64, 12, 14]);
    assert_eq!(
        command.quad,
        [
            crate::sdf::tile::Point2::new(12.0, 4.0),
            crate::sdf::tile::Point2::new(24.0, 4.0),
            crate::sdf::tile::Point2::new(24.0, 18.0),
            crate::sdf::tile::Point2::new(12.0, 18.0),
        ]
    );
}
