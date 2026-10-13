use super::*;

const BOUNDS: Rect = Rect {
    x: 10.0,
    y: 20.0,
    width: 8.0,
    height: 4.0,
};

#[test]
fn shape_samples_are_a_grid_symmetric_about_the_pixel_centre() {
    let mut columns: Vec<f32> = SHAPE_SAMPLES.iter().map(|sample| sample[0]).collect();
    columns.sort_by(f32::total_cmp);
    columns.dedup();
    assert_eq!(columns, [0.25, 0.75]);
    for [x, y] in SHAPE_SAMPLES {
        assert!(SHAPE_SAMPLES.contains(&[2.0 * PIXEL_CENTRE - x, y]));
        assert!(SHAPE_SAMPLES.contains(&[x, 2.0 * PIXEL_CENTRE - y]));
    }
    assert_eq!(SHAPE_SAMPLE_WEIGHT * SHAPE_SAMPLES.len() as f32, 1.0);
}

#[test]
fn nearest_texel_floors_and_clamps_to_the_texture() {
    assert_eq!(nearest_texel(0.0, 4), 0);
    assert_eq!(nearest_texel(0.2499, 4), 0);
    assert_eq!(nearest_texel(0.25, 4), 1);
    assert_eq!(nearest_texel(0.999, 4), 3);
    assert_eq!(nearest_texel(1.0, 4), 3);
    assert_eq!(nearest_texel(-0.5, 4), 0);
    assert_eq!(nearest_texel(0.5, 0), 0);
}

#[test]
fn bilinear_taps_interpolate_between_texel_centres_and_clamp_at_the_edges() {
    // Texel 1's centre is at 1.5 / 4.
    assert_eq!(bilinear_taps(1.5 / 4.0, 4), (1, 2, 0.0));
    assert_eq!(bilinear_taps(2.0 / 4.0, 4), (1, 2, 0.5));
    // Outside the outer texel centres the edge texel is used alone.
    assert_eq!(bilinear_taps(0.0, 4), (0, 1, 0.0));
    assert_eq!(bilinear_taps(1.0, 4), (3, 3, 0.0));
}

#[test]
fn shape_bounds_are_half_open_and_an_inset_shrinks_them() {
    let rect = ShapePrimitive::Rect;
    assert!(shape_contains(&rect, BOUNDS, 10.0, 20.0, 0.0));
    assert!(!shape_contains(&rect, BOUNDS, 18.0, 22.0, 0.0));
    assert!(!shape_contains(&rect, BOUNDS, 12.0, 24.0, 0.0));
    assert!(!shape_contains(&rect, BOUNDS, 10.5, 21.0, 1.0));
    assert!(shape_contains(&rect, BOUNDS, 11.0, 21.0, 1.0));
    // An inset of half the height empties the shape.
    assert!(!shape_contains(&rect, BOUNDS, 14.0, 22.0, 2.0));
}

#[test]
fn ellipses_and_rounded_rectangles_cut_their_corners() {
    let ellipse = ShapePrimitive::Ellipse;
    assert!(shape_contains(&ellipse, BOUNDS, 14.0, 22.0, 0.0));
    assert!(!shape_contains(&ellipse, BOUNDS, 10.5, 20.5, 0.0));
    let rounded = ShapePrimitive::RoundedRect { radius: [2.0, 2.0] };
    assert!(!shape_contains(&rounded, BOUNDS, 10.2, 20.2, 0.0));
    assert!(shape_contains(&rounded, BOUNDS, 12.0, 20.2, 0.0));
    // The inset reduces the radius with the bounds.
    assert!(!shape_contains(&rounded, BOUNDS, 11.2, 21.2, 1.0));
    assert!(shape_contains(&rounded, BOUNDS, 11.8, 21.8, 1.0));
}

#[test]
fn gradients_project_onto_their_line_and_clamp_at_its_ends() {
    let gradient = LinearGradient {
        start: [0.25, 0.5],
        end: [0.75, 0.5],
        start_color: [1.0, 0.0, 0.0, 1.0],
        end_color: [0.0, 0.0, 1.0, 0.5],
    };
    assert_eq!(
        shape_fill([0.0; 4], Some(&gradient), BOUNDS, 14.0, 21.0),
        [0.5, 0.0, 0.5, 0.75]
    );
    assert_eq!(
        shape_fill([0.0; 4], Some(&gradient), BOUNDS, 10.0, 21.0),
        [1.0, 0.0, 0.0, 1.0]
    );
    assert_eq!(
        shape_fill([0.0; 4], Some(&gradient), BOUNDS, 18.0, 21.0),
        [0.0, 0.0, 1.0, 0.5]
    );
    let point = LinearGradient {
        end: gradient.start,
        ..gradient.clone()
    };
    assert_eq!(
        shape_fill([0.0; 4], Some(&point), BOUNDS, 18.0, 21.0),
        [1.0, 0.0, 0.0, 1.0]
    );
    assert_eq!(shape_fill([0.2; 4], None, BOUNDS, 18.0, 21.0), [0.2; 4]);
}

#[test]
fn spans_hold_their_minimum_and_not_their_maximum() {
    assert!(span_contains(2.0, 5.5, 2.0));
    assert!(span_contains(2.0, 5.5, 5.499_999_5));
    assert!(!span_contains(2.0, 5.5, 5.5));
    assert!(!span_contains(2.0, 5.5, 1.999_999_9));
    // Of two spans meeting at a point exactly one holds it.
    for value in [3.0, 3.5, 4.0] {
        assert_ne!(
            span_contains(1.0, value, value),
            span_contains(value, 6.0, value)
        );
    }
}

#[test]
fn a_pixel_span_holds_the_pixels_whose_centres_the_span_holds() {
    let edges = [
        -3.25,
        -0.5,
        0.0,
        0.25,
        0.5,
        0.75,
        1.0,
        1.5,
        2.499_999_8,
        2.5,
        2.500_000_2,
        7.0,
        7.5,
        100.125,
        811.5,
        812.0,
        1829.5,
        1830.0,
    ];
    for min in edges {
        for max in edges {
            let (first, end) = pixel_span(min, max);
            for pixel in 0..1840u32 {
                let pixel = pixel as f32;
                assert_eq!(
                    pixel >= first && pixel < end,
                    span_contains(min, max, pixel + PIXEL_CENTRE),
                    "span {min}..{max} pixel {pixel}"
                );
            }
        }
    }
}

#[test]
fn clip_bounds_accept_only_axis_aligned_rectangles() {
    let rectangle = [[10.0, 20.0], [30.0, 20.0], [30.0, 50.0], [10.0, 50.0]];
    assert_eq!(
        axis_aligned_clip_bounds(rectangle),
        Some([10.0, 20.0, 30.0, 50.0])
    );
    // Either winding, starting at any corner.
    let turned = [[30.0, 50.0], [30.0, 20.0], [10.0, 20.0], [10.0, 50.0]];
    assert_eq!(
        axis_aligned_clip_bounds(turned),
        Some([10.0, 20.0, 30.0, 50.0])
    );
    // Sides within the tolerance of an axis count as on it, and the bounds
    // take the outermost corners.
    let nearly = [
        [10.0, 20.0],
        [30.0, 20.000_05],
        [30.000_05, 50.0],
        [10.0, 50.0],
    ];
    assert_eq!(
        axis_aligned_clip_bounds(nearly),
        Some([10.0, 20.0, 30.000_05, 50.0])
    );
    let rotated = [[10.0, 20.0], [30.0, 21.0], [29.0, 51.0], [9.0, 50.0]];
    assert_eq!(axis_aligned_clip_bounds(rotated), None);
    let flat = [[10.0, 20.0], [30.0, 20.0], [30.0, 20.0], [10.0, 20.0]];
    assert_eq!(axis_aligned_clip_bounds(flat), None);
    let mut infinite = rectangle;
    infinite[2][0] = f32::INFINITY;
    assert_eq!(axis_aligned_clip_bounds(infinite), None);
}

#[test]
fn image_clips_keep_their_inside() {
    assert!(ellipse_clip_contains(BOUNDS, 14.0, 22.0));
    assert!(!ellipse_clip_contains(BOUNDS, 10.3, 20.3));
    assert!(!ellipse_clip_contains(
        Rect {
            width: 0.0,
            ..BOUNDS
        },
        10.0,
        22.0
    ));
    assert!(rounded_rect_clip_contains([1.0, 1.0], BOUNDS, 14.0, 20.1));
    assert!(!rounded_rect_clip_contains([1.0, 1.0], BOUNDS, 10.1, 20.1));
    assert!(rounded_rect_clip_contains([0.0, 1.0], BOUNDS, 10.1, 20.1));
}
