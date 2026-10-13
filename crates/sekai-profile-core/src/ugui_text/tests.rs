use super::*;

/// FreeType metrics of the few characters the vectors use: pixel size,
/// character, then bitmap left, bitmap top, width, rows and the 26.6
/// advance of an unhinted mincho face. Bitmaps are left empty: layout
/// reads only metrics.
const METRICS: &[(u32, char, i32, i32, u32, u32, i64)] = &[
    (40, '願', 0, 35, 39, 39, 2560),
    (40, '望', 1, 34, 38, 37, 2560),
    (40, ' ', 0, 0, 0, 0, 768),
    (40, '\u{a0}', 0, 0, 0, 0, 768),
    (36, '夢', 0, 31, 36, 35, 2304),
    (36, 'の', 3, 26, 30, 28, 2304),
    (36, '実', 1, 31, 35, 34, 2304),
    (36, '現', 0, 31, 36, 35, 2304),
    (36, '昔', 1, 31, 34, 34, 2304),
    (36, '縁', 0, 31, 36, 35, 2304),
    (36, ' ', 0, 0, 0, 0, 691),
    (30, '「', 20, 26, 9, 28, 1920),
    (30, 'サ', 1, 24, 28, 27, 1920),
    (30, 'イ', 1, 24, 23, 26, 1920),
    (30, 'ズ', 1, 25, 29, 25, 1920),
    (30, '」', 1, 25, 9, 28, 1920),
    (30, '3', 1, 24, 17, 26, 1190),
    (30, '6', 1, 24, 17, 26, 1190),
    (30, 'P', 0, 24, 21, 25, 1369),
    (30, 't', 0, 20, 12, 22, 741),
    (30, '。', 1, 6, 9, 9, 1920),
    (30, 'ー', 3, 15, 25, 6, 1920),
    (30, ' ', 0, 0, 0, 0, 576),
];

/// Serves [`METRICS`], scaled linearly for other pixel sizes, and records
/// the sizes it was asked for.
#[derive(Default)]
struct MetricTable {
    requested: Vec<(char, u32)>,
}

impl UguiGlyphRasterizer for MetricTable {
    fn glyph(&mut self, ch: char, pixel_size: u32) -> Option<Arc<UguiGlyph>> {
        self.requested.push((ch, pixel_size));
        let (size, _, left, top, width, rows, advance) = METRICS
            .iter()
            .copied()
            .find(|entry| entry.1 == ch && entry.0 == pixel_size)
            .or_else(|| {
                METRICS
                    .iter()
                    .copied()
                    .find(|entry| entry.1 == ch && pixel_size.is_multiple_of(entry.0))
            })?;
        let factor = pixel_size / size;
        Some(Arc::new(UguiGlyph {
            bitmap_left: left * factor as i32,
            bitmap_top: top * factor as i32,
            width: width * factor,
            rows: rows * factor,
            advance_26_6: advance * i64::from(factor),
            coverage: vec![255; (width * rows * factor * factor) as usize],
        }))
    }
}

fn mincho(family: &str) -> UguiFontAsset {
    UguiFontAsset {
        family: family.into(),
        reference_size: 16.0,
        ascent: 14.080_001,
        descent: -1.920_000_1,
        line_spacing: 32.0,
        character_padding: 1,
        tracking: 1.0,
        round_advance: true,
    }
}

fn slip_offsets() -> Vec<VerticalGlyphOffset> {
    let offset = |target, y, angle_degrees| VerticalGlyphOffset {
        target,
        offset: [0.0, y],
        angle_degrees,
    };
    vec![
        offset('ゃ', 10.0, 0.0),
        offset('ゅ', 10.0, 0.0),
        offset('ょ', 10.0, 0.0),
        offset('っ', 10.0, 0.0),
        offset('、', 22.0, 90.0),
        offset('。', 22.0, 90.0),
        offset('ー', 0.0, 0.0),
        offset('々', 0.0, 90.0),
    ]
}

fn source(
    text: &str,
    font_size: i32,
    line_spacing: f32,
    rect_size: [f32; 2],
    pivot: [f32; 2],
) -> UguiTextSource {
    UguiTextSource {
        text: text.into(),
        font: mincho("SyntheticMincho"),
        fallback_faces: Vec::new(),
        font_size,
        line_spacing,
        color: [1.0; 4],
        rect_size,
        pivot,
        node_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        vertical: Some(UguiVerticalModifier {
            offsets: slip_offsets(),
        }),
        backdrop: None,
    }
}

fn title(text: &str) -> UguiTextSource {
    source(text, 40, 1.0, [100.0, 100.0], [0.0, 0.0])
}

fn corners(mesh: &UguiTextMesh) -> Vec<Quad> {
    mesh.quads.iter().map(|quad| quad.corners).collect()
}

fn laid_out(source: &UguiTextSource) -> UguiTextMesh {
    layout(source, 1.0, &mut MetricTable::default())
}

fn assert_corners(actual: &[Quad], expected: &[Quad]) {
    assert_eq!(actual.len(), expected.len(), "{actual:?}");
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        for (corner, (a, e)) in actual.iter().zip(expected).enumerate() {
            assert!(
                a[0].to_bits() == e[0].to_bits() && a[1].to_bits() == e[1].to_bits(),
                "quad {index} corner {corner}: {a:?} != {e:?}"
            );
        }
    }
}

#[test]
fn the_quarter_turn_constants_are_the_f32_values_of_cos_90_and_one_degree() {
    assert_eq!(COS_QUARTER_TURN.to_bits(), 0xB33B_BD2E);
    assert_eq!(DEGREES_TO_RADIANS.to_bits(), 0x3C8E_FA35);
    let quarter = 90.0 * DEGREES_TO_RADIANS;
    assert_eq!(quarter.cos(), COS_QUARTER_TURN);
    assert_eq!(quarter.sin(), 1.0);
}

#[test]
fn a_single_line_of_kanji_is_set_upright_in_its_title_rect() {
    let mesh = laid_out(&title("願望"));
    assert_corners(
        &corners(&mesh),
        &[
            [[-1.0, 60.0], [-1.0, 101.0], [40.0, 101.0], [40.0, 60.0]],
            [[40.5, 60.5], [40.5, 100.5], [79.5, 100.5], [79.5, 60.5]],
        ],
    );
    assert_eq!(mesh.preferred_width, 80.0);
    assert_eq!(mesh.cell_padding, 1);
}

#[test]
fn a_space_advances_the_pen_without_a_quad() {
    let mesh = laid_out(&title("願 望"));
    assert_corners(
        &corners(&mesh),
        &[
            [[-1.0, 60.0], [-1.0, 101.0], [40.0, 101.0], [40.0, 60.0]],
            [[52.5, 60.5], [52.5, 100.5], [91.5, 100.5], [91.5, 60.5]],
        ],
    );
    assert_eq!(mesh.preferred_width, 92.0);
}

#[test]
fn a_no_break_space_is_a_blank_two_pixel_quad_that_keeps_its_orientation() {
    let mesh = laid_out(&title("願\u{a0}望"));
    assert_corners(
        &corners(&mesh),
        &[
            [[-1.0, 60.0], [-1.0, 101.0], [40.0, 101.0], [40.0, 60.0]],
            [[39.0, 66.0], [41.0, 66.0], [41.0, 64.0], [39.0, 64.0]],
            [[52.5, 60.5], [52.5, 100.5], [91.5, 100.5], [91.5, 60.5]],
        ],
    );
    assert!(mesh.quads[1].glyph.is_none());
    assert_eq!(mesh.preferred_width, 92.0);
}

#[test]
fn later_lines_advance_by_the_line_spacing_share_of_the_pitch() {
    let mesh = laid_out(&source(
        "夢の実現\n昔の縁",
        36,
        0.62,
        [418.634_28, 134.812_13],
        [0.0, 1.0],
    ));
    assert_corners(
        &corners(&mesh),
        &[
            [[-0.5, -37.5], [-0.5, 0.5], [36.5, 0.5], [36.5, -37.5]],
            [
                [39.0, -36.0],
                [39.0, -4.000_001],
                [69.0, -4.0],
                [69.0, -36.0],
            ],
            [[72.5, -36.5], [72.5, 0.5], [108.5, 0.5], [108.5, -36.5]],
            [[107.5, -37.5], [107.5, 0.5], [144.5, 0.5], [144.5, -37.5]],
            [[0.0, -81.0], [0.0, -45.0], [36.0, -45.0], [36.0, -81.0]],
            [[39.0, -81.0], [39.0, -49.0], [69.0, -49.0], [69.0, -81.0]],
            [[71.5, -82.5], [71.5, -44.5], [108.5, -44.5], [108.5, -82.5]],
        ],
    );
    assert_eq!(mesh.preferred_width, 144.0);
}

#[test]
fn latin_digits_and_brackets_keep_their_quads_and_listed_marks_turn_and_move() {
    let mesh = laid_out(&source(
        "「サイズ」36Pt。ー",
        30,
        0.62,
        [416.0, 78.0],
        [0.0, 1.0],
    ));
    assert_corners(
        &corners(&mesh),
        &[
            [[19.0, 1.0], [30.0, 1.0], [30.0, -29.0], [19.0, -29.0]],
            [
                [30.5, -30.5],
                [30.5, -0.500_000_95],
                [59.5, -0.499_999_05],
                [59.5, -30.5],
            ],
            [
                [58.5, -27.5],
                [58.5, -2.500_001],
                [86.5, -2.499_999],
                [86.5, -27.5],
            ],
            [
                [92.0, -29.0],
                [92.0, 1.999_999],
                [119.0, 2.000_001],
                [119.0, -29.0],
            ],
            [[120.0, -0.0], [131.0, -0.0], [131.0, -30.0], [120.0, -30.0]],
            [[150.0, -1.0], [169.0, -1.0], [169.0, -29.0], [150.0, -29.0]],
            [[169.0, -1.0], [188.0, -1.0], [188.0, -29.0], [169.0, -29.0]],
            [[187.0, -1.0], [210.0, -1.0], [210.0, -28.0], [187.0, -28.0]],
            [[208.0, -5.0], [222.0, -5.0], [222.0, -29.0], [208.0, -29.0]],
            [[221.0, -8.0], [221.0, 3.0], [232.0, 3.0], [232.0, -8.0]],
            [[262.5, -27.5], [262.5, -0.5], [270.5, -0.5], [270.5, -27.5]],
        ],
    );
    assert_eq!(mesh.preferred_width, 281.0);
}

#[test]
fn without_the_modifier_quads_stay_horizontal() {
    let mut text = title("願望");
    text.vertical = None;
    assert_corners(
        &corners(&laid_out(&text)),
        &[
            [[-1.0, 101.0], [40.0, 101.0], [40.0, 60.0], [-1.0, 60.0]],
            [[40.0, 100.0], [80.0, 100.0], [80.0, 61.0], [40.0, 61.0]],
        ],
    );
}

#[test]
fn upright_turns_cover_kana_kanji_and_hangul_only() {
    for ch in ['あ', 'ゟ', 'ア', 'ー', 'ヿ', '一', '鿿', '가', '힣'] {
        assert!(turns_upright(ch), "{ch}");
    }
    for ch in ['A', '3', '「', '。', '、', '々', '\u{a0}', '\u{3000}', '，'] {
        assert!(!turns_upright(ch), "{ch}");
    }
}

#[test]
fn modifier_characters_skip_line_breaks_carriage_returns_and_spaces() {
    // The carriage return keeps a quad of its own, but the modifier's
    // string drops it, so the kanji after it lines up with the second
    // character of that string.
    let mut table = MetricTable::default();
    let mesh = layout(&title("願\r望"), 1.0, &mut table);
    assert_eq!(mesh.quads.len(), 3);
    assert_eq!(mesh.quads[1].corners, [[40.0, 65.0]; 4]);
    assert!(mesh.quads[1].glyph.is_none());
    // The second modifier character is 望, applied to the zero quad of
    // the carriage return; the last quad is past the end and is left
    // unturned.
    assert_eq!(
        mesh.quads[2].corners,
        [[40.0, 100.0], [80.0, 100.0], [80.0, 61.0], [40.0, 61.0]]
    );
    assert!(!table.requested.contains(&('\r', 40)));
}

#[test]
fn a_tab_moves_the_pen_to_the_next_stop_of_four_spaces() {
    // The space advance is 12 at 40 pixels, so stops are 48 apart.
    let mut text = title("願\t望");
    text.vertical = None;
    let mesh = laid_out(&text);
    assert_eq!(mesh.quads.len(), 2);
    assert_eq!(mesh.quads[1].corners[0], [48.0, 100.0]);
    assert_eq!(mesh.preferred_width, 88.0);
}

#[test]
fn a_character_the_font_lacks_is_a_blank_quad_with_no_advance() {
    let mut text = title("願☃望");
    text.vertical = None;
    let mesh = laid_out(&text);
    assert_eq!(mesh.quads.len(), 3);
    assert_eq!(
        mesh.quads[1].corners,
        [[39.0, 66.0], [41.0, 66.0], [41.0, 64.0], [39.0, 64.0]]
    );
    assert!(mesh.quads[1].glyph.is_none());
    assert_eq!(mesh.quads[2].corners[0], [40.0, 100.0]);
}

#[test]
fn glyphs_are_rendered_at_the_output_pixel_size_and_geometry_scales_back() {
    let text = title("願望");
    let mut table = MetricTable::default();
    let doubled = layout(&text, 2.0, &mut table);
    assert!(table.requested.iter().all(|(_, size)| *size == 80));
    // Metrics that scale exactly with the size give the same node-space
    // text up to the rounding of the doubled line metrics.
    assert_eq!(doubled.preferred_width, 80.0);
    assert_eq!(doubled.quads[0].corners[0], [-0.5, 60.5]);
    assert_eq!(pixel_size(40, 1.0), 40);
    assert_eq!(pixel_size(36, 1.5), 54);
    assert_eq!(pixel_size(30, 0.99), 29);
    assert_eq!(pixel_size(300, 2.0), MAX_PIXEL_SIZE);
    assert_eq!(pixel_size(0, 1.0), 0);
    assert!(layout(
        &source("願", 0, 1.0, [1.0, 1.0], [0.0, 0.0]),
        1.0,
        &mut table
    )
    .quads
    .is_empty());
}

#[test]
fn quads_keep_their_glyph_bitmaps_for_the_renderer() {
    let mesh = laid_out(&title("願望"));
    let glyph = mesh.quads[0].glyph.as_ref().expect("bitmap");
    assert_eq!((glyph.width, glyph.rows), (39, 39));
    // The cell is the bitmap plus the padding on each side, one texel per
    // pixel: 41 texels across a 41-unit quad.
    let [top_left, _, bottom_right, _] = mesh.quads[0].corners;
    assert_eq!(bottom_right[0] - top_left[0], 41.0);
    assert_eq!(
        glyph.width + 2 * mesh.cell_padding,
        (bottom_right[1] - top_left[1]) as u32
    );
}

#[test]
fn a_fitted_backdrop_keeps_its_top_and_grows_with_the_preferred_width() {
    let backdrop = UguiTextBackdrop {
        color: [0.0, 0.8, 0.733_333_35, 1.0],
        rect: Rect {
            x: -6.0,
            y: -210.2,
            width: 50.0,
            height: 126.0,
        },
        fit_padding: Some(34.0),
    };
    assert_eq!(
        backdrop.resolved_rect(160.0),
        Rect {
            height: 194.0,
            ..backdrop.rect
        }
    );
    let fixed = UguiTextBackdrop {
        fit_padding: None,
        ..backdrop.clone()
    };
    assert_eq!(fixed.resolved_rect(160.0), backdrop.rect);
}

#[test]
fn cell_coverage_interpolates_between_texel_centres_and_treats_the_padding_as_empty() {
    // A 2 x 2 bitmap, opaque top row and half-covered bottom row, in a
    // 4 x 4 cell.
    let glyph = UguiGlyph {
        bitmap_left: 0,
        bitmap_top: 2,
        width: 2,
        rows: 2,
        advance_26_6: 3 * 64,
        coverage: vec![255, 255, 128, 128],
    };
    let at = |u: f32, v: f32| cell_coverage(&glyph, 1, u, v);
    // Texel centres return their texels.
    assert_eq!(at(1.5, 1.5), 1.0);
    assert_eq!(at(2.5, 2.5), 128.0 / 255.0);
    assert_eq!(at(0.5, 0.5), 0.0);
    // Halfway between the padding and the first bitmap column, and
    // between the two rows.
    assert_eq!(at(1.0, 1.5), 0.5);
    assert_eq!(at(1.5, 2.0), (1.0 + 128.0 / 255.0) / 2.0);
    // The cell edge and beyond it are empty.
    assert_eq!(at(0.0, 2.0), 0.0);
    assert_eq!(at(4.0, 2.0), 0.0);
    assert_eq!(at(-3.0, 9.0), 0.0);
}

#[test]
fn sources_round_trip_through_json_and_bincode() {
    let mut text = title("願望");
    text.backdrop = Some(UguiTextBackdrop {
        color: [0.0, 0.8, 0.733_333_35, 1.0],
        rect: Rect::default(),
        fit_padding: Some(34.0),
    });
    let json = serde_json::to_value(&text).unwrap();
    assert_eq!(json["vertical"]["offsets"][4]["target"], "、");
    assert_eq!(
        serde_json::from_value::<UguiTextSource>(json).unwrap(),
        text
    );
    let bytes = bincode::encode_to_vec(&text, bincode::config::standard()).unwrap();
    let (decoded, _): (UguiTextSource, usize) =
        bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
    assert_eq!(decoded, text);
}
