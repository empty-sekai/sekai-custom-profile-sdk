use super::*;

#[test]
fn one_dimensional_transform_measures_to_the_nearest_seed() {
    let f = [0.0, INF, INF, 0.0];
    let mut d = [0.0; 4];
    let mut v = [0usize; 5];
    let mut z = [0.0f32; 5];
    distance_transform_1d(&f, &mut d, &mut v, &mut z);
    assert_eq!(d, [0.0, 1.0, 1.0, 0.0]);
}

#[test]
fn mask_distance_places_the_edge_between_cell_centres() {
    let inside = [false, false, true, true];
    assert_eq!(
        signed_distance_from_mask(&inside, 4, 1),
        [1.5, 0.5, -0.5, -1.5]
    );
}

#[test]
fn mask_without_inside_cells_saturates_to_the_diagonal() {
    let distance = signed_distance_from_mask(&[false; 9], 3, 3);
    let cap = (18.0f32).sqrt() - 0.5;
    assert!(distance.iter().all(|&value| value == cap));
}

#[test]
fn grid_rounds_the_metrics_box_outward_and_adds_the_spread_margin() {
    let grid = GlyphSdfGrid::from_metrics(1.25, 40.5, 30.0, 41.0, 6.0);
    assert_eq!(grid.left, -5.0);
    assert_eq!(grid.top, 47.0);
    assert_eq!(grid.width, 31 + 12);
    assert_eq!(grid.height, 42 + 12);
}

/// Coverage of a disc, point-sampled on a `ss`-times finer pixel grid.
fn disc_coverage(radius: f32, ss: usize, grid: GlyphSdfGrid) -> (Vec<u8>, usize, usize) {
    let width = grid.width * ss;
    let rows = grid.height * ss;
    let mut buffer = vec![0u8; width * rows];
    for row in 0..rows {
        for column in 0..width {
            let x = grid.left + (column as f32 + 0.5) / ss as f32;
            let y = grid.top - (row as f32 + 0.5) / ss as f32;
            if (x * x + y * y).sqrt() <= radius {
                buffer[row * width + column] = 255;
            }
        }
    }
    (buffer, width, rows)
}

#[test]
fn edt_error_shrinks_with_the_supersample_factor() {
    let radius = 20.3;
    let spread = 6.0;
    let grid = GlyphSdfGrid::from_metrics(-radius, radius, 2.0 * radius, 2.0 * radius, spread);
    let mean_error = |ss: usize| {
        let (buffer, width, rows) = disc_coverage(radius, ss, grid);
        let coverage = CoverageBitmap {
            buffer: &buffer,
            width,
            rows,
            pitch: width,
            left: (grid.left as i32) * ss as i32,
            top: (grid.top as i32) * ss as i32,
        };
        let pixels = edt_sdf(&coverage, grid, ss, spread);
        let mut sum = 0.0f32;
        let mut count = 0usize;
        for py in 0..grid.height {
            for px in 0..grid.width {
                let x = grid.left + px as f32 + 0.5;
                let y = grid.top - py as f32 - 0.5;
                let exact = encode_distance((x * x + y * y).sqrt() - radius, spread);
                if exact == 0 || exact == 255 {
                    continue;
                }
                sum += (f32::from(pixels[py * grid.width + px]) - f32::from(exact)).abs();
                count += 1;
            }
        }
        sum / count as f32
    };
    let (e1, e2, e4) = (mean_error(1), mean_error(2), mean_error(4));
    assert!(e2 < e1 * 0.7, "ss2 {e2} vs ss1 {e1}");
    assert!(e4 < e2 * 0.7, "ss4 {e4} vs ss2 {e2}");
    assert!(e4 < 1.5, "ss4 mean gray error {e4}");
}

#[test]
fn edt_reads_coverage_at_the_bitmap_origin() {
    // One covered bitmap pixel whose origin places it at cell column 3,
    // row 5 of the grid, i.e. texel column 1, row 2 at ss = 2: that texel
    // is the brightest of the field.
    let grid = GlyphSdfGrid {
        left: 0.0,
        top: 8.0,
        width: 4,
        height: 8,
    };
    let buffer = [255u8];
    let shifted = CoverageBitmap {
        buffer: &buffer,
        width: 1,
        rows: 1,
        pitch: 1,
        left: 3,
        top: 11,
    };
    let pixels = edt_sdf(&shifted, grid, 2, 6.0);
    let brightest = pixels
        .iter()
        .enumerate()
        .max_by_key(|(_, value)| **value)
        .map(|(index, _)| index)
        .expect("the grid has texels");
    assert_eq!(brightest, 2 * grid.width + 1);
}

#[test]
fn outline_with_implied_on_curve_points_closes_through_the_first_point() {
    let points = [
        Vec2::new(0.0, 0.0),
        Vec2::new(64.0, 0.0),
        Vec2::new(64.0, 64.0),
        Vec2::new(0.0, 64.0),
    ];
    // on, off, off, on: the two off-curve points imply a midpoint.
    let tags = [1u8, 0, 0, 1];
    let contours = outline_contours(&points, &tags, &[3]);
    assert_eq!(contours.len(), 1);
    assert_eq!(
        contours[0],
        vec![
            Segment::Quad(QuadSeg {
                p0: Vec2::new(0.0, 0.0),
                p1: Vec2::new(64.0, 0.0),
                p2: Vec2::new(64.0, 32.0),
            }),
            Segment::Quad(QuadSeg {
                p0: Vec2::new(64.0, 32.0),
                p1: Vec2::new(64.0, 64.0),
                p2: Vec2::new(0.0, 64.0),
            }),
            Segment::line(Vec2::new(0.0, 64.0), Vec2::new(0.0, 0.0)),
        ]
    );
}
