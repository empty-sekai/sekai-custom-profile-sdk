//! 轮廓几何计算。

use std::slice;

use freetype::ffi;

const CUBIC_STEPS: usize = 128;
const WINDING_QUAD_STEPS: usize = 16;
const WINDING_CUBIC_STEPS: usize = 32;
const FT_POINT_TAG_ON_CURVE: u8 = 0x01;
const FT_POINT_TAG_CUBIC_CONTROL: u8 = 0x02;

#[derive(Clone, Copy)]
pub(super) struct Vec2 {
    x: f32,
    y: f32,
}

impl Vec2 {
    pub(super) fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }
    fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(
            self.x + (other.x - self.x) * t,
            self.y + (other.y - self.y) * t,
        )
    }
    fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y
    }
    fn cross(self, other: Self) -> f32 {
        self.x * other.y - self.y * other.x
    }
    fn length(self) -> f32 {
        self.length_sq().sqrt()
    }
    fn length_sq(self) -> f32 {
        self.x * self.x + self.y * self.y
    }
}

#[derive(Clone, Copy)]
pub(super) struct LineSeg {
    p0: Vec2,
    p1: Vec2,
}
#[derive(Clone, Copy)]
pub(super) struct QuadSeg {
    p0: Vec2,
    p1: Vec2,
    p2: Vec2,
}
#[derive(Clone, Copy)]
pub(super) struct CubicSeg {
    p0: Vec2,
    p1: Vec2,
    p2: Vec2,
    p3: Vec2,
}

#[derive(Clone, Copy)]
pub(super) enum Segment {
    Line(LineSeg),
    Quad(QuadSeg),
    Cubic(CubicSeg),
}

pub(super) unsafe fn extract_segments(outline: &ffi::FT_Outline) -> Vec<Vec<Segment>> {
    let contour_ends = slice::from_raw_parts(outline.contours, outline.n_contours as usize);
    let points = slice::from_raw_parts(outline.points, outline.n_points as usize);
    let tags = slice::from_raw_parts(outline.tags, outline.n_points as usize);
    let mut current_point_index = 0usize;
    let mut contours = Vec::with_capacity(contour_ends.len());
    for &last_point_index_in_contour in contour_ends {
        let last_point_index_in_contour = last_point_index_in_contour as usize;
        let (mut first_point, first_tag) = get_point(
            &mut current_point_index,
            points,
            tags,
            last_point_index_in_contour,
        );
        if (first_tag & FT_POINT_TAG_ON_CURVE) == 0 {
            let mut temp_point_index = last_point_index_in_contour;
            let (last_point, last_tag) = get_point(
                &mut temp_point_index,
                points,
                tags,
                last_point_index_in_contour,
            );
            first_point = if (last_tag & FT_POINT_TAG_ON_CURVE) != 0 {
                last_point
            } else {
                last_point.lerp(first_point, 0.5)
            };
            current_point_index = current_point_index.saturating_sub(1);
        }
        let mut contour = Vec::new();
        let mut current_pen = first_point;
        while current_point_index <= last_point_index_in_contour {
            let (mut point0, tag0) = get_point(
                &mut current_point_index,
                points,
                tags,
                last_point_index_in_contour,
            );
            if (tag0 & FT_POINT_TAG_ON_CURVE) != 0 {
                contour.push(Segment::Line(LineSeg {
                    p0: current_pen,
                    p1: point0,
                }));
                current_pen = point0;
                continue;
            }
            loop {
                if current_point_index > last_point_index_in_contour {
                    contour.push(Segment::Quad(QuadSeg {
                        p0: current_pen,
                        p1: point0,
                        p2: first_point,
                    }));
                    current_pen = first_point;
                    break;
                }
                let (point1, tag1) = get_point(
                    &mut current_point_index,
                    points,
                    tags,
                    last_point_index_in_contour,
                );
                if (tag0 & FT_POINT_TAG_CUBIC_CONTROL) != 0 {
                    let end = if current_point_index <= last_point_index_in_contour {
                        let (point2, _) = get_point(
                            &mut current_point_index,
                            points,
                            tags,
                            last_point_index_in_contour,
                        );
                        point2
                    } else {
                        first_point
                    };
                    contour.push(Segment::Cubic(CubicSeg {
                        p0: current_pen,
                        p1: point0,
                        p2: point1,
                        p3: end,
                    }));
                    current_pen = end;
                    break;
                }
                if (tag1 & FT_POINT_TAG_ON_CURVE) != 0 {
                    contour.push(Segment::Quad(QuadSeg {
                        p0: current_pen,
                        p1: point0,
                        p2: point1,
                    }));
                    current_pen = point1;
                    break;
                }
                let point_half = point0.lerp(point1, 0.5);
                contour.push(Segment::Quad(QuadSeg {
                    p0: current_pen,
                    p1: point0,
                    p2: point_half,
                }));
                current_pen = point_half;
                point0 = point1;
            }
        }
        if !contour.is_empty() && !same_point(current_pen, first_point) {
            contour.push(Segment::Line(LineSeg {
                p0: current_pen,
                p1: first_point,
            }));
        }
        if !contour.is_empty() {
            contours.push(contour);
        }
    }
    contours
}

unsafe fn get_point(
    current_point_index: &mut usize,
    point_positions: &[ffi::FT_Vector],
    point_tags: &[i8],
    last_point_index_in_contour: usize,
) -> (Vec2, u8) {
    debug_assert!(*current_point_index <= last_point_index_in_contour);
    let point_position = point_positions[*current_point_index];
    let point_tag = point_tags[*current_point_index] as u8;
    *current_point_index += 1;
    (
        Vec2::new(point_position.x as f32, point_position.y as f32),
        point_tag,
    )
}

fn same_point(a: Vec2, b: Vec2) -> bool {
    (a.x - b.x).abs() <= 1e-6 && (a.y - b.y).abs() <= 1e-6
}

pub(super) fn dist_to_line(point: Vec2, seg: LineSeg) -> f32 {
    let d = seg.p1.sub(seg.p0);
    let t = point.sub(seg.p0).dot(d) / d.length_sq().max(1e-12);
    let t = t.clamp(0.0, 1.0);
    let closest = Vec2::new(seg.p0.x + d.x * t, seg.p0.y + d.y * t);
    point.sub(closest).length()
}

pub(super) fn dist_to_quad(point: Vec2, seg: QuadSeg) -> f32 {
    let a = Vec2::new(
        seg.p0.x - 2.0 * seg.p1.x + seg.p2.x,
        seg.p0.y - 2.0 * seg.p1.y + seg.p2.y,
    );
    let b = Vec2::new(2.0 * (seg.p1.x - seg.p0.x), 2.0 * (seg.p1.y - seg.p0.y));
    let c = Vec2::new(seg.p0.x - point.x, seg.p0.y - point.y);
    let c3 = 2.0 * a.dot(a);
    let c2 = 3.0 * a.dot(b);
    let c1 = b.dot(b) + 2.0 * a.dot(c);
    let c0 = b.dot(c);
    let mut best = f32::INFINITY;
    for t in solve_cubic_real(c3, c2, c1, c0) {
        best = best.min(dist_to_point_quad(point, seg, t.clamp(0.0, 1.0)));
    }
    best.min(point.sub(seg.p0).length())
        .min(point.sub(seg.p2).length())
}

fn dist_to_point_quad(point: Vec2, seg: QuadSeg, t: f32) -> f32 {
    let mt = 1.0 - t;
    let pos = Vec2::new(
        mt * mt * seg.p0.x + 2.0 * mt * t * seg.p1.x + t * t * seg.p2.x,
        mt * mt * seg.p0.y + 2.0 * mt * t * seg.p1.y + t * t * seg.p2.y,
    );
    point.sub(pos).length()
}

pub(super) fn dist_to_cubic(point: Vec2, seg: CubicSeg) -> f32 {
    let points = subdivide_cubic(seg, CUBIC_STEPS);
    let mut best = f32::INFINITY;
    for window in points.windows(2) {
        best = best.min(dist_to_line(
            point,
            LineSeg {
                p0: window[0],
                p1: window[1],
            },
        ));
    }
    best
}

fn subdivide_cubic(seg: CubicSeg, steps: usize) -> Vec<Vec2> {
    let mut out = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let mt = 1.0 - t;
        out.push(Vec2::new(
            mt * mt * mt * seg.p0.x
                + 3.0 * mt * mt * t * seg.p1.x
                + 3.0 * mt * t * t * seg.p2.x
                + t * t * t * seg.p3.x,
            mt * mt * mt * seg.p0.y
                + 3.0 * mt * mt * t * seg.p1.y
                + 3.0 * mt * t * t * seg.p2.y
                + t * t * t * seg.p3.y,
        ));
    }
    out
}

fn solve_quadratic_real(a: f32, b: f32, c: f32) -> Vec<f32> {
    const EPS: f32 = 1e-6;
    if a.abs() < EPS {
        return if b.abs() < EPS {
            Vec::new()
        } else {
            vec![-c / b]
        };
    }
    let disc = b * b - 4.0 * a * c;
    if disc < -EPS {
        return Vec::new();
    }
    if disc.abs() <= EPS {
        return vec![-b / (2.0 * a)];
    }
    let sqrt_disc = disc.sqrt();
    vec![(-b + sqrt_disc) / (2.0 * a), (-b - sqrt_disc) / (2.0 * a)]
}

fn push_unique_root(out: &mut Vec<f32>, value: f32) {
    if value.is_finite() && !out.iter().any(|root| (*root - value).abs() <= 1e-5) {
        out.push(value);
    }
}

fn solve_cubic_real(c3: f32, c2: f32, c1: f32, c0: f32) -> Vec<f32> {
    const EPS: f32 = 1e-6;
    if c3.abs() < EPS {
        return solve_quadratic_real(c2, c1, c0);
    }
    let a = c2 / c3;
    let b = c1 / c3;
    let c = c0 / c3;
    let p = b - a * a / 3.0;
    let q = (2.0 * a * a * a) / 27.0 - (a * b) / 3.0 + c;
    let disc = q * q / 4.0 + p * p * p / 27.0;
    let shift = a / 3.0;
    let mut roots = Vec::new();
    if disc > EPS {
        let sqrt_disc = disc.sqrt();
        push_unique_root(
            &mut roots,
            (-q / 2.0 + sqrt_disc).cbrt() + (-q / 2.0 - sqrt_disc).cbrt() - shift,
        );
        return roots;
    }
    if disc.abs() <= EPS {
        let u = (-q / 2.0).cbrt();
        push_unique_root(&mut roots, 2.0 * u - shift);
        push_unique_root(&mut roots, -u - shift);
        return roots;
    }
    let r = (-p / 3.0).sqrt();
    if r.abs() <= EPS {
        push_unique_root(&mut roots, -shift);
        return roots;
    }
    let phi = (-q / (2.0 * r * r * r)).clamp(-1.0, 1.0).acos();
    let t = 2.0 * r;
    push_unique_root(&mut roots, t * (phi / 3.0).cos() - shift);
    push_unique_root(
        &mut roots,
        t * ((phi + 2.0 * std::f32::consts::PI) / 3.0).cos() - shift,
    );
    push_unique_root(
        &mut roots,
        t * ((phi + 4.0 * std::f32::consts::PI) / 3.0).cos() - shift,
    );
    roots
}

pub(super) fn winding_number(point: Vec2, contours: &[Vec<Segment>]) -> i32 {
    let mut wn = 0;
    for contour in contours {
        for seg in contour {
            match *seg {
                Segment::Line(seg) => wn += winding_line(point, seg.p0, seg.p1),
                Segment::Quad(seg) => {
                    let mut prev = seg.p0;
                    for i in 1..=WINDING_QUAD_STEPS {
                        let t = i as f32 / WINDING_QUAD_STEPS as f32;
                        let mt = 1.0 - t;
                        let next = Vec2::new(
                            mt * mt * seg.p0.x + 2.0 * mt * t * seg.p1.x + t * t * seg.p2.x,
                            mt * mt * seg.p0.y + 2.0 * mt * t * seg.p1.y + t * t * seg.p2.y,
                        );
                        wn += winding_line(point, prev, next);
                        prev = next;
                    }
                }
                Segment::Cubic(seg) => {
                    let points = subdivide_cubic(seg, WINDING_CUBIC_STEPS);
                    for window in points.windows(2) {
                        wn += winding_line(point, window[0], window[1]);
                    }
                }
            }
        }
    }
    wn
}

fn winding_line(point: Vec2, a: Vec2, b: Vec2) -> i32 {
    if a.y <= point.y {
        if b.y > point.y && b.sub(a).cross(point.sub(a)) > 0.0 {
            return 1;
        }
    } else if b.y <= point.y && b.sub(a).cross(point.sub(a)) < 0.0 {
        return -1;
    }
    0
}
