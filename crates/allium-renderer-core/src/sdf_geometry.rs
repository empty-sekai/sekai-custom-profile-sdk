const CUBIC_DISTANCE_STEPS: usize = 128;
const WINDING_QUAD_STEPS: usize = 16;
const WINDING_CUBIC_STEPS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(
            self.x + (other.x - self.x) * t,
            self.y + (other.y - self.y) * t,
        )
    }
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y
    }
    pub fn cross(self, other: Self) -> f32 {
        self.x * other.y - self.y * other.x
    }
    pub fn length(self) -> f32 {
        self.length_sq().sqrt()
    }
    pub fn length_sq(self) -> f32 {
        self.x * self.x + self.y * self.y
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineSeg {
    pub p0: Vec2,
    pub p1: Vec2,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadSeg {
    pub p0: Vec2,
    pub p1: Vec2,
    pub p2: Vec2,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubicSeg {
    pub p0: Vec2,
    pub p1: Vec2,
    pub p2: Vec2,
    pub p3: Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    Line(LineSeg),
    Quad(QuadSeg),
    Cubic(CubicSeg),
}

impl Segment {
    pub fn line(p0: Vec2, p1: Vec2) -> Self {
        Self::Line(LineSeg { p0, p1 })
    }
}

#[derive(Clone, Copy)]
struct Bounds {
    min: Vec2,
    max: Vec2,
}

impl Bounds {
    fn from_points(points: &[Vec2]) -> Self {
        let mut min = Vec2::new(f32::INFINITY, f32::INFINITY);
        let mut max = Vec2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
        for point in points {
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
        }
        Self { min, max }
    }

    fn lower_bound_sq(self, point: Vec2) -> f32 {
        let dx = if point.x < self.min.x {
            self.min.x - point.x
        } else if point.x > self.max.x {
            point.x - self.max.x
        } else {
            0.0
        };
        let dy = if point.y < self.min.y {
            self.min.y - point.y
        } else if point.y > self.max.y {
            point.y - self.max.y
        } else {
            0.0
        };
        dx * dx + dy * dy
    }

    fn union(self, other: Self) -> Self {
        Self {
            min: Vec2::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            max: Vec2::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        }
    }

    fn center_axis(self, axis: usize) -> f32 {
        if axis == 0 {
            (self.min.x + self.max.x) * 0.5
        } else {
            (self.min.y + self.max.y) * 0.5
        }
    }
}

#[derive(Clone, Copy)]
enum DistancePrimitive {
    Line(LineSeg, Bounds),
    Quad(QuadSeg, Bounds),
}

impl DistancePrimitive {
    fn line(line: LineSeg) -> Self {
        Self::Line(line, Bounds::from_points(&[line.p0, line.p1]))
    }
    fn quad(quad: QuadSeg) -> Self {
        Self::Quad(quad, Bounds::from_points(&[quad.p0, quad.p1, quad.p2]))
    }
    fn lower_bound_sq(self, point: Vec2) -> f32 {
        match self {
            Self::Line(_, bounds) | Self::Quad(_, bounds) => bounds.lower_bound_sq(point),
        }
    }
    fn bounds(self) -> Bounds {
        match self {
            Self::Line(_, bounds) | Self::Quad(_, bounds) => bounds,
        }
    }
    fn distance(self, point: Vec2) -> f32 {
        match self {
            Self::Line(line, _) => dist_to_line(point, line),
            Self::Quad(quad, _) => dist_to_quad(point, quad),
        }
    }
}

enum DistanceBvh {
    Leaf {
        bounds: Bounds,
        primitives: Vec<DistancePrimitive>,
    },
    Branch {
        bounds: Bounds,
        left: Box<DistanceBvh>,
        right: Box<DistanceBvh>,
    },
}

impl DistanceBvh {
    fn build(mut primitives: Vec<DistancePrimitive>) -> Option<Self> {
        if primitives.is_empty() {
            return None;
        }
        let bounds = primitives
            .iter()
            .skip(1)
            .fold(primitives[0].bounds(), |all, primitive| {
                all.union(primitive.bounds())
            });
        if primitives.len() <= 8 {
            return Some(Self::Leaf { bounds, primitives });
        }
        let axis = if bounds.max.x - bounds.min.x >= bounds.max.y - bounds.min.y {
            0
        } else {
            1
        };
        primitives.sort_unstable_by(|left, right| {
            left.bounds()
                .center_axis(axis)
                .total_cmp(&right.bounds().center_axis(axis))
        });
        let right_primitives = primitives.split_off(primitives.len() / 2);
        let left = Box::new(Self::build(primitives).expect("non-empty BVH left"));
        let right = Box::new(Self::build(right_primitives).expect("non-empty BVH right"));
        Some(Self::Branch {
            bounds,
            left,
            right,
        })
    }

    fn bounds(&self) -> Bounds {
        match self {
            Self::Leaf { bounds, .. } | Self::Branch { bounds, .. } => *bounds,
        }
    }

    fn distance(&self, point: Vec2, mut best: f32) -> f32 {
        if self.bounds().lower_bound_sq(point) >= best * best {
            return best;
        }
        match self {
            Self::Leaf { primitives, .. } => {
                for primitive in primitives {
                    if primitive.lower_bound_sq(point) < best * best {
                        best = best.min(primitive.distance(point));
                    }
                }
                best
            }
            Self::Branch { left, right, .. } => {
                let left_bound = left.bounds().lower_bound_sq(point);
                let right_bound = right.bounds().lower_bound_sq(point);
                if left_bound <= right_bound {
                    best = left.distance(point, best);
                    right.distance(point, best)
                } else {
                    best = right.distance(point, best);
                    left.distance(point, best)
                }
            }
        }
    }
}

/// Immutable exact-profile SDF query structure shared by native and WASM.
/// Cubic subdivision and winding edges are prepared once per glyph; bounds
/// pruning cannot change the legacy distance result.
pub struct AnalyticDistanceField {
    distance_bvh: Option<DistanceBvh>,
    winding_edges: Vec<LineSeg>,
}

impl AnalyticDistanceField {
    pub fn new(contours: &[Vec<Segment>]) -> Self {
        let segment_count = contours.iter().map(Vec::len).sum::<usize>();
        let mut distance_primitives = Vec::with_capacity(segment_count);
        let mut winding_edges = Vec::with_capacity(segment_count * 4);
        for segment in contours.iter().flatten().copied() {
            match segment {
                Segment::Line(line) => {
                    distance_primitives.push(DistancePrimitive::line(line));
                    winding_edges.push(line);
                }
                Segment::Quad(quad) => {
                    distance_primitives.push(DistancePrimitive::quad(quad));
                    append_quad_lines(&mut winding_edges, quad, WINDING_QUAD_STEPS);
                }
                Segment::Cubic(cubic) => {
                    let distance_start = distance_primitives.len();
                    append_cubic_lines_to(cubic, CUBIC_DISTANCE_STEPS, |line| {
                        distance_primitives.push(DistancePrimitive::line(line));
                    });
                    debug_assert_eq!(
                        distance_primitives.len() - distance_start,
                        CUBIC_DISTANCE_STEPS
                    );
                    append_cubic_lines_to(cubic, WINDING_CUBIC_STEPS, |line| {
                        winding_edges.push(line)
                    });
                }
            }
        }
        Self {
            distance_bvh: DistanceBvh::build(distance_primitives),
            winding_edges,
        }
    }

    pub fn signed_distance(&self, point: Vec2) -> f32 {
        let best = self
            .distance_bvh
            .as_ref()
            .map(|bvh| bvh.distance(point, f32::INFINITY))
            .unwrap_or(f32::INFINITY);
        if self.winding_number(point) != 0 {
            -best
        } else {
            best
        }
    }

    fn winding_number(&self, point: Vec2) -> i32 {
        self.winding_edges
            .iter()
            .map(|line| winding_line(point, line.p0, line.p1))
            .sum()
    }
}

fn append_quad_lines(out: &mut Vec<LineSeg>, quad: QuadSeg, steps: usize) {
    let mut previous = quad.p0;
    for index in 1..=steps {
        let t = index as f32 / steps as f32;
        let mt = 1.0 - t;
        let next = Vec2::new(
            mt * mt * quad.p0.x + 2.0 * mt * t * quad.p1.x + t * t * quad.p2.x,
            mt * mt * quad.p0.y + 2.0 * mt * t * quad.p1.y + t * t * quad.p2.y,
        );
        out.push(LineSeg {
            p0: previous,
            p1: next,
        });
        previous = next;
    }
}

fn append_cubic_lines_to(cubic: CubicSeg, steps: usize, mut append: impl FnMut(LineSeg)) {
    let mut previous = cubic.p0;
    for index in 1..=steps {
        let t = index as f32 / steps as f32;
        let mt = 1.0 - t;
        let next = Vec2::new(
            mt * mt * mt * cubic.p0.x
                + 3.0 * mt * mt * t * cubic.p1.x
                + 3.0 * mt * t * t * cubic.p2.x
                + t * t * t * cubic.p3.x,
            mt * mt * mt * cubic.p0.y
                + 3.0 * mt * mt * t * cubic.p1.y
                + 3.0 * mt * t * t * cubic.p2.y
                + t * t * t * cubic.p3.y,
        );
        append(LineSeg {
            p0: previous,
            p1: next,
        });
        previous = next;
    }
}

fn dist_to_line(point: Vec2, segment: LineSeg) -> f32 {
    let delta = segment.p1.sub(segment.p0);
    let t = (point.sub(segment.p0).dot(delta) / delta.length_sq().max(1e-12)).clamp(0.0, 1.0);
    point
        .sub(Vec2::new(
            segment.p0.x + delta.x * t,
            segment.p0.y + delta.y * t,
        ))
        .length()
}

fn dist_to_quad(point: Vec2, segment: QuadSeg) -> f32 {
    let a = Vec2::new(
        segment.p0.x - 2.0 * segment.p1.x + segment.p2.x,
        segment.p0.y - 2.0 * segment.p1.y + segment.p2.y,
    );
    let b = Vec2::new(
        2.0 * (segment.p1.x - segment.p0.x),
        2.0 * (segment.p1.y - segment.p0.y),
    );
    let c = segment.p0.sub(point);
    let mut best = f32::INFINITY;
    for t in solve_cubic_real(
        2.0 * a.dot(a),
        3.0 * a.dot(b),
        b.dot(b) + 2.0 * a.dot(c),
        b.dot(c),
    ) {
        best = best.min(dist_to_point_quad(point, segment, t.clamp(0.0, 1.0)));
    }
    best.min(point.sub(segment.p0).length())
        .min(point.sub(segment.p2).length())
}

fn dist_to_point_quad(point: Vec2, segment: QuadSeg, t: f32) -> f32 {
    let mt = 1.0 - t;
    point
        .sub(Vec2::new(
            mt * mt * segment.p0.x + 2.0 * mt * t * segment.p1.x + t * t * segment.p2.x,
            mt * mt * segment.p0.y + 2.0 * mt * t * segment.p1.y + t * t * segment.p2.y,
        ))
        .length()
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
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < -EPS {
        return Vec::new();
    }
    if discriminant.abs() <= EPS {
        return vec![-b / (2.0 * a)];
    }
    let root = discriminant.sqrt();
    vec![(-b + root) / (2.0 * a), (-b - root) / (2.0 * a)]
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
    let discriminant = q * q / 4.0 + p * p * p / 27.0;
    let shift = a / 3.0;
    let mut roots = Vec::new();
    if discriminant > EPS {
        let root = discriminant.sqrt();
        push_unique_root(
            &mut roots,
            (-q / 2.0 + root).cbrt() + (-q / 2.0 - root).cbrt() - shift,
        );
    } else if discriminant.abs() <= EPS {
        let u = (-q / 2.0).cbrt();
        push_unique_root(&mut roots, 2.0 * u - shift);
        push_unique_root(&mut roots, -u - shift);
    } else {
        let radius = (-p / 3.0).sqrt();
        if radius.abs() <= EPS {
            push_unique_root(&mut roots, -shift);
        } else {
            let phi = (-q / (2.0 * radius * radius * radius))
                .clamp(-1.0, 1.0)
                .acos();
            let scale = 2.0 * radius;
            for turn in [0.0, 2.0, 4.0] {
                push_unique_root(
                    &mut roots,
                    scale * ((phi + turn * std::f32::consts::PI) / 3.0).cos() - shift,
                );
            }
        }
    }
    roots
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
