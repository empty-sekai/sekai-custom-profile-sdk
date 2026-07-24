//! 轮廓几何计算。

use std::slice;

pub(super) use allium_renderer_core::sdf_geometry::{CubicSeg, LineSeg, QuadSeg, Segment, Vec2};
use freetype::ffi;

const FT_POINT_TAG_ON_CURVE: u8 = 0x01;
const FT_POINT_TAG_CUBIC_CONTROL: u8 = 0x02;

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
    point_tags: &[std::ffi::c_char],
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
