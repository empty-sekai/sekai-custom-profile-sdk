use sekai_profile_core::ugui_text::UguiFontFace;
use sekai_profile_core::{Rect, StableId};

use super::*;

/// A 2 x 2 glyph: opaque top row, half-covered bottom row.
fn glyph() -> UguiGlyph {
    UguiGlyph {
        bitmap_left: 0,
        bitmap_top: 2,
        width: 2,
        rows: 2,
        advance_26_6: 3 * 64,
        coverage: vec![255, 255, 128, 128],
    }
}

fn canvas(width: u32, height: u32) -> Vec<u8> {
    vec![0; (width * height * 4) as usize]
}

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * width + x) * 4) as usize;
    pixels[offset..offset + 4].try_into().expect("pixel")
}

fn draw(pixels: &mut [u8], width: u32, corners: [[f32; 2]; 4], color: [f32; 4]) -> u64 {
    GlyphTarget {
        pixels,
        width,
        height: width,
        clip: None,
        blend_mode: BlendMode::SrcOver,
    }
    .draw(corners, &glyph(), 1, color)
    .expect("drawn")
}

#[test]
fn a_pixel_aligned_cell_copies_its_texels_times_the_vertex_colour() {
    let mut pixels = canvas(6, 6);
    // The 4 x 4 padded cell over pixels (1, 1)..(5, 5).
    let blended = draw(
        &mut pixels,
        6,
        [[1.0, 1.0], [5.0, 1.0], [5.0, 5.0], [1.0, 5.0]],
        [1.0, 0.0, 0.0, 1.0],
    );
    assert_eq!(blended, 4);
    assert_eq!(pixel(&pixels, 6, 2, 2), [255, 0, 0, 255]);
    assert_eq!(pixel(&pixels, 6, 3, 2), [255, 0, 0, 255]);
    assert_eq!(pixel(&pixels, 6, 2, 3), [128, 0, 0, 128]);
    // The padding texels are empty.
    for (x, y) in [(1, 1), (4, 2), (2, 4), (0, 0)] {
        assert_eq!(pixel(&pixels, 6, x, y), [0; 4], "({x}, {y})");
    }
}

#[test]
fn coverage_scales_a_premultiplied_colour_and_blends_source_over() {
    let mut pixels = canvas(6, 6);
    let offset = ((3 * 6 + 2) * 4) as usize;
    pixels[offset..offset + 4].copy_from_slice(&[0, 0, 255, 255]);
    // Grey at full alpha, premultiplied.
    let grey = 79.0 / 255.0;
    draw(
        &mut pixels,
        6,
        [[1.0, 1.0], [5.0, 1.0], [5.0, 5.0], [1.0, 5.0]],
        [grey, grey, grey, 1.0],
    );
    assert_eq!(pixel(&pixels, 6, 2, 2), [79, 79, 79, 255]);
    // Half coverage over blue: 40 + 0, 40 + 0, 40 + 127, 128 + 127.
    assert_eq!(pixel(&pixels, 6, 2, 3), [40, 40, 167, 255]);
}

#[test]
fn a_half_pixel_offset_samples_between_texels() {
    let mut pixels = canvas(6, 6);
    draw(
        &mut pixels,
        6,
        [[1.5, 1.0], [5.5, 1.0], [5.5, 5.0], [1.5, 5.0]],
        [1.0; 4],
    );
    // Pixel 2's centre is texel 1.0 of the cell: halfway between the
    // padding and the first glyph column.
    assert_eq!(pixel(&pixels, 6, 2, 2), [128; 4]);
    assert_eq!(pixel(&pixels, 6, 3, 2), [255; 4]);
    assert_eq!(pixel(&pixels, 6, 4, 2), [128; 4]);
}

#[test]
fn a_turned_cell_draws_its_first_row_down_the_canvas() {
    let mut pixels = canvas(6, 6);
    // A quarter turn clockwise: the cell's rows run down, its first row
    // on the right.
    draw(
        &mut pixels,
        6,
        [[5.0, 1.0], [5.0, 5.0], [1.0, 5.0], [1.0, 1.0]],
        [1.0; 4],
    );
    assert_eq!(pixel(&pixels, 6, 3, 2), [255; 4]);
    assert_eq!(pixel(&pixels, 6, 3, 3), [255; 4]);
    assert_eq!(pixel(&pixels, 6, 2, 2), [128; 4]);
    assert_eq!(pixel(&pixels, 6, 2, 3), [128; 4]);
}

#[test]
fn pixels_outside_the_quad_or_the_clip_are_untouched() {
    let mut pixels = canvas(6, 6);
    let blended = GlyphTarget {
        pixels: &mut pixels,
        width: 6,
        height: 6,
        clip: Some(AxisAlignedClip {
            min_x: 0.0,
            min_y: 0.0,
            max_x: 3.0,
            max_y: 6.0,
        }),
        blend_mode: BlendMode::SrcOver,
    }
    .draw(
        [[1.0, 1.0], [5.0, 1.0], [5.0, 5.0], [1.0, 5.0]],
        &glyph(),
        1,
        [1.0; 4],
    )
    .expect("drawn");
    assert_eq!(blended, 2);
    assert_eq!(pixel(&pixels, 6, 2, 2), [255; 4]);
    assert_eq!(pixel(&pixels, 6, 3, 2), [0; 4]);
    // A degenerate quad draws nothing.
    assert_eq!(draw(&mut pixels, 6, [[1.0, 1.0]; 4], [1.0; 4]), 0);
}

const SIZE: u32 = 160;

fn layer(matrix: Matrix2d) -> LayerSource {
    LayerSource {
        id: StableId(1),
        parent_id: None,
        kind: sekai_profile_core::LayerKind::Composite,
        authored_kind: sekai_profile_core::AuthoredElementKind::Collection,
        authored_index: 0,
        game_layer: 1,
        z: 1,
        authored_visible: true,
        source_content: String::new(),
        resolved_parameters: Default::default(),
        bounds: Rect::default(),
        quad: [[0.0; 2]; 4],
        matrix,
        hit_geometry: [[0.0; 2]; 4],
        line_indent: None,
    }
}

/// `text` in white at size 40 in a 100 x 100 rect pivoted at its
/// bottom-left corner, which the node matrix puts at layer (10, 110).
fn text_source(text: &str, node_matrix: Matrix2d) -> UguiTextSource {
    UguiTextSource {
        text: text.into(),
        font: sekai_profile_core::omikuji::font_asset("DejaVu Sans"),
        fallback_faces: Vec::new(),
        font_size: 40,
        line_spacing: 1.0,
        color: [1.0; 4],
        rect_size: [100.0, 100.0],
        pivot: [0.0, 0.0],
        node_matrix,
        vertical: None,
        backdrop: Some(sekai_profile_core::ugui_text::UguiTextBackdrop {
            color: [0.0, 0.0, 1.0, 1.0],
            rect: Rect {
                x: 130.0,
                y: 0.0,
                width: 20.0,
                height: 20.0,
            },
            fit_padding: Some(10.0),
        }),
    }
}

fn render(source: UguiTextSource) -> Option<(Vec<u8>, RasterImageStats)> {
    if crate::sdf::outline::resolve_font_path("DejaVu Sans").is_none() {
        eprintln!("DejaVu Sans is not installed; skipping");
        return None;
    }
    let command = SemanticCommandSource::ugui_text(
        StableId(2),
        StableId(1),
        "text",
        Rect::default(),
        source.clone(),
    );
    let mut pixels = canvas(SIZE, SIZE);
    let stats = raster_ugui_text_command(
        &mut pixels,
        SIZE,
        SIZE,
        &command,
        &layer([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        &source,
        0.0,
        ImageExecutor::Scalar,
    )
    .expect("rendered");
    Some((pixels, stats))
}

#[test]
fn a_fallback_family_without_an_installed_file_fails_the_command() {
    let mut source = text_source("H", [1.0, 0.0, 0.0, -1.0, 10.0, 110.0]);
    source.fallback_faces = vec![UguiFontFace {
        family: "Uninstalled Fallback".into(),
        face_index: 2,
    }];
    let command = SemanticCommandSource::ugui_text(
        StableId(2),
        StableId(1),
        "omikuji-title",
        Rect::default(),
        source.clone(),
    );
    let result = raster_ugui_text_command(
        &mut canvas(SIZE, SIZE),
        SIZE,
        SIZE,
        &command,
        &layer([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        &source,
        0.0,
        ImageExecutor::Scalar,
    );
    // The text's own font is checked first; "H" needs no fallback, and
    // the fallback is required all the same.
    let family = if crate::sdf::outline::resolve_font_path("DejaVu Sans").is_some() {
        "Uninstalled Fallback"
    } else {
        "DejaVu Sans"
    };
    match result {
        Err(ProfileCompositorError::MissingFont {
            role,
            family: missing,
        }) => {
            assert_eq!((role.as_str(), missing.as_str()), ("omikuji-title", family));
        }
        other => panic!("{:?}", other.map(|stats| stats.fragments)),
    }
}

fn alpha_sum(pixels: &[u8], columns: std::ops::Range<u32>) -> u64 {
    (0..SIZE)
        .flat_map(|y| columns.clone().map(move |x| (x, y)))
        .map(|(x, y)| u64::from(pixel(pixels, SIZE, x, y)[3]))
        .sum()
}

#[test]
fn a_text_command_draws_its_fitted_backdrop_and_its_freetype_coverage() {
    let upright = [1.0, 0.0, 0.0, -1.0, 10.0, 110.0];
    let Some((pixels, stats)) = render(text_source("HH", upright)) else {
        return;
    };
    let mut glyphs = UguiGlyphs::for_chain(&[UguiFontFace {
        family: "DejaVu Sans".into(),
        face_index: 0,
    }])
    .expect("font");
    let glyph =
        sekai_profile_core::ugui_text::UguiGlyphRasterizer::glyph(&mut glyphs, 'H', 40).expect("H");
    // Pixel-aligned cells copy the coverage bitmap one texel per pixel.
    let coverage = glyph
        .coverage
        .iter()
        .map(|&value| u64::from(value))
        .sum::<u64>();
    assert_eq!(alpha_sum(&pixels, 0..130), 2 * coverage);
    // The backdrop keeps its top edge and runs the preferred width (two
    // rounded advances) plus the padding down the layer.
    let advance = ((glyph.advance_26_6 as f32 / 64.0) + 0.5).floor() as u32;
    let height = 2 * advance + 10;
    assert_eq!(pixel(&pixels, SIZE, 140, height - 1), [0, 0, 255, 255]);
    assert_eq!(pixel(&pixels, SIZE, 140, height), [0; 4]);
    assert_eq!(alpha_sum(&pixels, 130..SIZE), 20 * u64::from(height) * 255);
    assert_eq!(stats.fragments, stats.scalar_fragments);
}

#[test]
fn a_turned_text_node_keeps_its_coverage() {
    // The quarter turn of the slip's text nodes.
    let turned = sekai_profile_core::omikuji::text_node_matrix([10.0, -30.0]);
    let upright = [1.0, 0.0, 0.0, -1.0, 10.0, 110.0];
    let (Some((straight, _)), Some((sideways, _))) = (
        render(text_source("H", upright)),
        render(text_source("H", turned)),
    ) else {
        return;
    };
    assert_eq!(alpha_sum(&sideways, 0..130), alpha_sum(&straight, 0..130));
}
