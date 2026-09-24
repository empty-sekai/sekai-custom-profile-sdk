//! Glyph SDF rasterization shared by the native renderer and the browser runtime.
//!
//! Both backends load unhinted FreeType outlines themselves. Everything after
//! that lives here, so the two produce the same texels: decomposing the outline
//! into [`Segment`]s, the texel grid a glyph occupies ([`GlyphSdfGrid`]), the
//! exact distance fill ([`analytic_sdf`]), the Euclidean distance transform over
//! a supersampled coverage bitmap ([`edt_sdf`]) and the gray encoding
//! ([`encode_distance`]).
//!
//! Distances are measured in pixels at the sampling point size. A texel stores
//! `0.5 - d / (2 * spread)`, so the outline sits at 0.5, the inside is brighter
//! and the encoded band reaches `spread` pixels to either side, matching the
//! TextMesh Pro gradient scale.

use crate::sdf_geometry::{AnalyticDistanceField, CubicSeg, LineSeg, QuadSeg, Segment, Vec2};

const POINT_TAG_ON_CURVE: u8 = 0x01;
const POINT_TAG_CUBIC_CONTROL: u8 = 0x02;

/// Supersampled coverage at or above this value counts as inside the glyph.
pub const EDT_COVERAGE_THRESHOLD: u8 = 128;

/// Texel grid of one glyph SDF, in pixels at the sampling point size (y up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphSdfGrid {
    /// X of the grid's left edge; always an integer.
    pub left: f32,
    /// Y of the grid's top edge; always an integer.
    pub top: f32,
    pub width: usize,
    pub height: usize,
}

impl GlyphSdfGrid {
    /// The glyph's metrics box rounded outward to whole pixels, grown by
    /// `ceil(spread)` texels on every side so the distance band is not clipped.
    pub fn from_metrics(
        bearing_x: f32,
        bearing_y: f32,
        width: f32,
        height: f32,
        spread: f32,
    ) -> Self {
        let margin = spread.ceil();
        let left = bearing_x.floor() - margin;
        let top = bearing_y.ceil() + margin;
        let right = (bearing_x + width).ceil() + margin;
        let bottom = (bearing_y - height).floor() - margin;
        Self {
            left,
            top,
            width: (right - left).max(1.0) as usize,
            height: (top - bottom).max(1.0) as usize,
        }
    }
}

/// Encodes a signed distance (negative inside) as an SDF texel.
pub fn encode_distance(signed_distance: f32, spread: f32) -> u8 {
    let gray = (0.5 - signed_distance / (2.0 * spread)).clamp(0.0, 1.0);
    (gray * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Samples the exact signed distance to `contours` at every texel centre.
///
/// Contour coordinates are FreeType 26.6 units at the sampling point size.
pub fn analytic_sdf(contours: &[Vec<Segment>], grid: GlyphSdfGrid, spread: f32) -> Vec<u8> {
    let field = AnalyticDistanceField::new(contours);
    let left_26_6 = grid.left * 64.0;
    let top_26_6 = grid.top * 64.0;
    let mut pixels = vec![0u8; grid.width * grid.height];
    for py in 0..grid.height {
        for px in 0..grid.width {
            let point = Vec2::new(
                left_26_6 + (px as f32 + 0.5) * 64.0,
                top_26_6 - (py as f32 + 0.5) * 64.0,
            );
            pixels[py * grid.width + px] =
                encode_distance(field.signed_distance(point) / 64.0, spread);
        }
    }
    pixels
}

/// An 8-bit coverage bitmap of one glyph, rasterized at `supersample` times
/// the sampling point size.
///
/// `left` and `top` place the bitmap's first column and row in supersampled
/// pixels, exactly as FreeType reports `bitmap_left` and `bitmap_top`.
#[derive(Clone, Copy, Debug)]
pub struct CoverageBitmap<'a> {
    pub buffer: &'a [u8],
    pub width: usize,
    pub rows: usize,
    pub pitch: usize,
    pub left: i32,
    pub top: i32,
}

/// Builds the SDF from a supersampled coverage bitmap with a Euclidean
/// distance transform.
///
/// Every texel of `grid` is split into `supersample`² cells that line up with
/// the bitmap's pixels. A cell is inside when its coverage reaches
/// [`EDT_COVERAGE_THRESHOLD`]; its distance to the outline is the distance to
/// the nearest cell of the other kind less half a cell, since the edge lies
/// between the two cell centres. A texel stores the mean of its cells.
pub fn edt_sdf(
    coverage: &CoverageBitmap<'_>,
    grid: GlyphSdfGrid,
    supersample: usize,
    spread: f32,
) -> Vec<u8> {
    let ss = supersample.max(1);
    let raster_width = grid.width * ss;
    let raster_height = grid.height * ss;
    let inside = coverage_mask(coverage, grid, ss);
    let distance = signed_distance_from_mask(&inside, raster_width, raster_height);
    let cells = (ss * ss) as f32;
    let mut pixels = vec![0u8; grid.width * grid.height];
    for py in 0..grid.height {
        for px in 0..grid.width {
            let mut sum = 0.0f32;
            for sy in 0..ss {
                let row = (py * ss + sy) * raster_width + px * ss;
                sum += distance[row..row + ss].iter().sum::<f32>();
            }
            pixels[py * grid.width + px] = encode_distance(sum / cells / ss as f32, spread);
        }
    }
    pixels
}

/// Thresholds the coverage bitmap onto the supersampled cells of `grid`.
fn coverage_mask(coverage: &CoverageBitmap<'_>, grid: GlyphSdfGrid, ss: usize) -> Vec<bool> {
    let raster_width = grid.width * ss;
    let raster_height = grid.height * ss;
    let mut inside = vec![false; raster_width * raster_height];
    if coverage.width == 0 || coverage.rows == 0 {
        return inside;
    }
    // Cell column `rx` spans [origin_x + rx, origin_x + rx + 1) and bitmap
    // column `bx` spans [left + bx, left + bx + 1); rows count down from the
    // top edge in both.
    let origin_x = grid.left as i64 * ss as i64;
    let origin_y = grid.top as i64 * ss as i64;
    let column_offset = origin_x - i64::from(coverage.left);
    let row_offset = i64::from(coverage.top) - origin_y;
    for ry in 0..raster_height {
        let by = row_offset + ry as i64;
        if by < 0 || by >= coverage.rows as i64 {
            continue;
        }
        let source = &coverage.buffer[by as usize * coverage.pitch..];
        for rx in 0..raster_width {
            let bx = column_offset + rx as i64;
            if bx >= 0 && bx < coverage.width as i64 {
                inside[ry * raster_width + rx] = source[bx as usize] >= EDT_COVERAGE_THRESHOLD;
            }
        }
    }
    inside
}

/// Decomposes a FreeType outline into closed contours.
///
/// `points` and `tags` are the outline's points and raw point tags;
/// `contour_ends` holds the index of each contour's last point. Conic runs
/// become quadratic segments through their implied on-curve midpoints.
pub fn outline_contours(points: &[Vec2], tags: &[u8], contour_ends: &[usize]) -> Vec<Vec<Segment>> {
    let point_at = |index: &mut usize, last: usize| -> (Vec2, u8) {
        debug_assert!(*index <= last);
        let value = (points[*index], tags[*index]);
        *index += 1;
        value
    };
    let mut current = 0usize;
    let mut contours = Vec::with_capacity(contour_ends.len());
    for &last in contour_ends {
        let (mut first_point, first_tag) = point_at(&mut current, last);
        if first_tag & POINT_TAG_ON_CURVE == 0 {
            let mut last_index = last;
            let (last_point, last_tag) = point_at(&mut last_index, last);
            first_point = if last_tag & POINT_TAG_ON_CURVE != 0 {
                last_point
            } else {
                last_point.lerp(first_point, 0.5)
            };
            current = current.saturating_sub(1);
        }
        let mut contour = Vec::new();
        let mut pen = first_point;
        while current <= last {
            let (mut point0, tag0) = point_at(&mut current, last);
            if tag0 & POINT_TAG_ON_CURVE != 0 {
                contour.push(Segment::Line(LineSeg {
                    p0: pen,
                    p1: point0,
                }));
                pen = point0;
                continue;
            }
            loop {
                if current > last {
                    contour.push(Segment::Quad(QuadSeg {
                        p0: pen,
                        p1: point0,
                        p2: first_point,
                    }));
                    pen = first_point;
                    break;
                }
                let (point1, tag1) = point_at(&mut current, last);
                if tag0 & POINT_TAG_CUBIC_CONTROL != 0 {
                    let end = if current <= last {
                        point_at(&mut current, last).0
                    } else {
                        first_point
                    };
                    contour.push(Segment::Cubic(CubicSeg {
                        p0: pen,
                        p1: point0,
                        p2: point1,
                        p3: end,
                    }));
                    pen = end;
                    break;
                }
                if tag1 & POINT_TAG_ON_CURVE != 0 {
                    contour.push(Segment::Quad(QuadSeg {
                        p0: pen,
                        p1: point0,
                        p2: point1,
                    }));
                    pen = point1;
                    break;
                }
                let midpoint = point0.lerp(point1, 0.5);
                contour.push(Segment::Quad(QuadSeg {
                    p0: pen,
                    p1: point0,
                    p2: midpoint,
                }));
                pen = midpoint;
                point0 = point1;
            }
        }
        if !contour.is_empty() && !same_point(pen, first_point) {
            contour.push(Segment::Line(LineSeg {
                p0: pen,
                p1: first_point,
            }));
        }
        if !contour.is_empty() {
            contours.push(contour);
        }
    }
    contours
}

fn same_point(a: Vec2, b: Vec2) -> bool {
    (a.x - b.x).abs() <= 1e-6 && (a.y - b.y).abs() <= 1e-6
}

const INF: f32 = 1e20;

/// Signed distance from every cell centre to the mask boundary, in cells,
/// negative inside.
///
/// The boundary lies halfway between an inside and an outside cell, so the
/// centre-to-centre distance from the transform is shortened by half a cell.
fn signed_distance_from_mask(inside: &[bool], width: usize, height: usize) -> Vec<f32> {
    let n = width * height;
    let mut to_outside = vec![0.0f32; n];
    let mut to_inside = vec![0.0f32; n];
    for i in 0..n {
        if inside[i] {
            to_outside[i] = INF;
        } else {
            to_inside[i] = INF;
        }
    }
    squared_distance_transform(&mut to_outside, width, height);
    squared_distance_transform(&mut to_inside, width, height);
    // A mask with no cell of one kind leaves that field at INF; the diagonal
    // bounds the result well past any spread.
    let cap = ((width * width + height * height) as f32).sqrt();
    (0..n)
        .map(|i| {
            if inside[i] {
                0.5 - to_outside[i].max(0.0).sqrt().min(cap)
            } else {
                to_inside[i].max(0.0).sqrt().min(cap) - 0.5
            }
        })
        .collect()
}

/// In-place exact squared Euclidean distance transform (Felzenszwalb and
/// Huttenlocher, 2012): cells holding 0 are seeds, cells holding [`INF`]
/// receive the squared distance to the nearest seed. Columns first, then rows.
fn squared_distance_transform(grid: &mut [f32], width: usize, height: usize) {
    if width == 0 || height == 0 {
        return;
    }
    let cap = width.max(height);
    let mut f = vec![0.0f32; cap];
    let mut d = vec![0.0f32; cap];
    let mut v = vec![0usize; cap + 1];
    let mut z = vec![0.0f32; cap + 1];
    for x in 0..width {
        for y in 0..height {
            f[y] = grid[y * width + x];
        }
        distance_transform_1d(&f[..height], &mut d[..height], &mut v, &mut z);
        for y in 0..height {
            grid[y * width + x] = d[y];
        }
    }
    for y in 0..height {
        let row = &mut grid[y * width..(y + 1) * width];
        f[..width].copy_from_slice(row);
        distance_transform_1d(&f[..width], &mut d[..width], &mut v, &mut z);
        row.copy_from_slice(&d[..width]);
    }
}

/// `d[q] = min over q' of f[q'] + (q - q')²`, via the lower envelope of
/// parabolas. `v` and `z` are scratch buffers of at least `f.len() + 1`.
fn distance_transform_1d(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k = 0usize;
    v[0] = 0;
    z[0] = -INF;
    z[1] = INF;
    for q in 1..n {
        let mut s = parabola_intersection(f, q, v[k]);
        while s <= z[k] {
            k -= 1;
            s = parabola_intersection(f, q, v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = INF;
    }
    let mut k = 0usize;
    for (q, out) in d.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let dq = q as f32 - v[k] as f32;
        *out = dq * dq + f[v[k]];
    }
}

#[inline]
fn parabola_intersection(f: &[f32], q: usize, vk: usize) -> f32 {
    ((f[q] + (q * q) as f32) - (f[vk] + (vk * vk) as f32)) / (2.0 * q as f32 - 2.0 * vk as f32)
}

#[cfg(test)]
mod tests {
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
}
