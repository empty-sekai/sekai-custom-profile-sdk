use std::arch::x86_64::*;

use allium_renderer_core::{BlendMode, LinearGradient, Matrix2d, Rect, ShapePrimitive};

use super::ImageClipGeometry;

#[target_feature(enable = "avx512f,avx512bw")]
pub(super) unsafe fn blend_rgba8_packet(
    destination: *mut u8,
    source: *const u32,
    active: u16,
    blend_mode: BlendMode,
) {
    let active = active as __mmask16;
    let source = _mm512_maskz_loadu_epi32(active, source.cast());
    blend_rgba8_vector(destination, source, active, blend_mode);
}

#[target_feature(enable = "avx512f,avx512bw")]
pub(super) unsafe fn gather_and_blend_rgba8_packet(
    destination: *mut u8,
    source_row: *const u8,
    source_columns: *const u32,
    active: u16,
    blend_mode: BlendMode,
) {
    let active = active as __mmask16;
    let columns = _mm512_maskz_loadu_epi32(active, source_columns.cast());
    let source = _mm512_mask_i32gather_epi32::<4>(
        _mm512_setzero_si512(),
        active,
        columns,
        source_row.cast(),
    );
    blend_rgba8_vector(destination, source, active, blend_mode);
}

#[target_feature(enable = "avx512f,avx512bw")]
pub(super) unsafe fn axis_aligned_image_clip_mask(
    local_x: *const f32,
    local_y: f32,
    bounds: Rect,
    clip: ImageClipGeometry,
    active: u16,
) -> u16 {
    let active = active as __mmask16;
    let local_x = _mm512_maskz_loadu_ps(active, local_x);
    let local_y = _mm512_set1_ps(local_y);
    image_clip_mask(local_x, local_y, bounds, clip, active) as u16
}

#[target_feature(enable = "avx512f,avx512bw")]
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn sample_affine_and_blend_rgba8_packet(
    destination: *mut u8,
    source: *const u8,
    source_row_bytes: i32,
    source_width: u32,
    source_height: u32,
    inverse: &Matrix2d,
    bounds: Rect,
    uv: Rect,
    x: u32,
    y: u32,
    lane_count: u32,
    blend_mode: BlendMode,
    image_clip: Option<ImageClipGeometry>,
) -> u16 {
    let packet_mask = if lane_count == 16 {
        u16::MAX
    } else {
        (1u16 << lane_count) - 1
    } as __mmask16;
    let px = _mm512_add_ps(
        _mm512_set1_ps(x as f32),
        _mm512_setr_ps(
            0.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5, 9.5, 10.5, 11.5, 12.5, 13.5, 14.5, 15.5,
        ),
    );
    let py = _mm512_set1_ps(y as f32 + 0.5);
    let local_x = _mm512_fmadd_ps(
        _mm512_set1_ps(inverse[0]),
        px,
        _mm512_fmadd_ps(_mm512_set1_ps(inverse[2]), py, _mm512_set1_ps(inverse[4])),
    );
    let local_y = _mm512_fmadd_ps(
        _mm512_set1_ps(inverse[1]),
        px,
        _mm512_fmadd_ps(_mm512_set1_ps(inverse[3]), py, _mm512_set1_ps(inverse[5])),
    );
    let u = _mm512_div_ps(
        _mm512_sub_ps(local_x, _mm512_set1_ps(bounds.x)),
        _mm512_set1_ps(bounds.width),
    );
    let v = _mm512_div_ps(
        _mm512_sub_ps(local_y, _mm512_set1_ps(bounds.y)),
        _mm512_set1_ps(bounds.height),
    );
    let zero = _mm512_setzero_ps();
    let one = _mm512_set1_ps(1.0);
    let mut active = packet_mask
        & _mm512_cmp_ps_mask::<_CMP_GE_OQ>(u, zero)
        & _mm512_cmp_ps_mask::<_CMP_LT_OQ>(u, one)
        & _mm512_cmp_ps_mask::<_CMP_GE_OQ>(v, zero)
        & _mm512_cmp_ps_mask::<_CMP_LT_OQ>(v, one);
    if let Some(clip) = image_clip {
        active = image_clip_mask(local_x, local_y, bounds, clip, active);
    }
    if active == 0 {
        return 0;
    }

    let source_x = _mm512_mul_ps(
        _mm512_add_ps(
            _mm512_set1_ps(uv.x),
            _mm512_mul_ps(u, _mm512_set1_ps(uv.width)),
        ),
        _mm512_set1_ps(source_width as f32),
    );
    let source_y = _mm512_mul_ps(
        _mm512_add_ps(
            _mm512_set1_ps(uv.y),
            _mm512_mul_ps(v, _mm512_set1_ps(uv.height)),
        ),
        _mm512_set1_ps(source_height as f32),
    );
    let source_x = _mm512_min_ps(
        _mm512_max_ps(
            _mm512_roundscale_ps::<{ _MM_FROUND_TO_NEG_INF | _MM_FROUND_NO_EXC }>(source_x),
            zero,
        ),
        _mm512_set1_ps(source_width.saturating_sub(1) as f32),
    );
    let source_y = _mm512_min_ps(
        _mm512_max_ps(
            _mm512_roundscale_ps::<{ _MM_FROUND_TO_NEG_INF | _MM_FROUND_NO_EXC }>(source_y),
            zero,
        ),
        _mm512_set1_ps(source_height.saturating_sub(1) as f32),
    );
    let source_x = _mm512_cvttps_epi32(source_x);
    let source_y = _mm512_cvttps_epi32(source_y);
    let offsets = _mm512_add_epi32(
        _mm512_mullo_epi32(source_y, _mm512_set1_epi32(source_row_bytes)),
        _mm512_slli_epi32::<2>(source_x),
    );
    let source_pixels =
        _mm512_mask_i32gather_epi32::<1>(_mm512_setzero_si512(), active, offsets, source.cast());
    blend_rgba8_vector(destination, source_pixels, active, blend_mode);
    active as u16
}

#[target_feature(enable = "avx512f,avx512bw,fma")]
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn raster_semantic_shape_packet(
    destination: *mut u8,
    x: u32,
    y: u32,
    lane_count: u32,
    inverse: &Matrix2d,
    bounds: Rect,
    primitive: &ShapePrimitive,
    fill: [f32; 4],
    gradient: Option<&LinearGradient>,
    stroke: [f32; 4],
    stroke_width: f32,
) -> u16 {
    let packet_mask = if lane_count == 16 {
        u16::MAX
    } else {
        (1u16 << lane_count) - 1
    } as __mmask16;
    let lanes = _mm512_setr_ps(
        0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
    );
    let zero = _mm512_setzero_ps();
    let one = _mm512_set1_ps(1.0);
    let quarter = _mm512_set1_ps(0.25);
    let mut accumulated = [zero; 4];
    let mut covered = 0u16;

    for (offset_x, offset_y) in [(0.25f32, 0.25f32), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
        let px = _mm512_add_ps(
            _mm512_add_ps(_mm512_set1_ps(x as f32), lanes),
            _mm512_set1_ps(offset_x),
        );
        let py = _mm512_set1_ps(y as f32 + offset_y);
        let local_x = _mm512_fmadd_ps(
            _mm512_set1_ps(inverse[0]),
            px,
            _mm512_fmadd_ps(_mm512_set1_ps(inverse[2]), py, _mm512_set1_ps(inverse[4])),
        );
        let local_y = _mm512_fmadd_ps(
            _mm512_set1_ps(inverse[1]),
            px,
            _mm512_fmadd_ps(_mm512_set1_ps(inverse[3]), py, _mm512_set1_ps(inverse[5])),
        );
        let outer = semantic_shape_mask(local_x, local_y, bounds, primitive, 0.0, packet_mask);
        if outer == 0 {
            continue;
        }
        covered |= outer as u16;
        let inner = if stroke_width > 0.0 {
            semantic_shape_mask(local_x, local_y, bounds, primitive, stroke_width, outer)
        } else {
            outer
        };
        let stroke_lanes = outer & !inner;
        let mut color = if let Some(gradient) = gradient {
            let u = _mm512_div_ps(
                _mm512_sub_ps(local_x, _mm512_set1_ps(bounds.x)),
                _mm512_set1_ps(bounds.width),
            );
            let v = _mm512_div_ps(
                _mm512_sub_ps(local_y, _mm512_set1_ps(bounds.y)),
                _mm512_set1_ps(bounds.height),
            );
            let dx = gradient.end[0] - gradient.start[0];
            let dy = gradient.end[1] - gradient.start[1];
            let denominator = dx.mul_add(dx, dy * dy);
            let t = if denominator <= f32::EPSILON {
                zero
            } else {
                let numerator = _mm512_fmadd_ps(
                    _mm512_sub_ps(u, _mm512_set1_ps(gradient.start[0])),
                    _mm512_set1_ps(dx),
                    _mm512_mul_ps(
                        _mm512_sub_ps(v, _mm512_set1_ps(gradient.start[1])),
                        _mm512_set1_ps(dy),
                    ),
                );
                _mm512_min_ps(
                    _mm512_max_ps(_mm512_div_ps(numerator, _mm512_set1_ps(denominator)), zero),
                    one,
                )
            };
            std::array::from_fn(|channel| {
                _mm512_fmadd_ps(
                    _mm512_set1_ps(gradient.end_color[channel] - gradient.start_color[channel]),
                    t,
                    _mm512_set1_ps(gradient.start_color[channel]),
                )
            })
        } else {
            fill.map(|value| _mm512_set1_ps(value))
        };
        for channel in 0..4 {
            color[channel] = _mm512_mask_blend_ps(
                stroke_lanes,
                color[channel],
                _mm512_set1_ps(stroke[channel]),
            );
            color[channel] = _mm512_min_ps(_mm512_max_ps(color[channel], zero), one);
        }
        let alpha = color[3];
        for channel in 0..3 {
            let contribution = _mm512_mul_ps(_mm512_mul_ps(color[channel], alpha), quarter);
            accumulated[channel] = _mm512_add_ps(
                accumulated[channel],
                _mm512_maskz_mov_ps(outer, contribution),
            );
        }
        accumulated[3] = _mm512_add_ps(
            accumulated[3],
            _mm512_maskz_mov_ps(outer, _mm512_mul_ps(alpha, quarter)),
        );
    }

    let active = covered as __mmask16;
    if active == 0 {
        return 0;
    }
    let byte_mask = _mm512_set1_epi32(0xff);
    let mut packed = _mm512_setzero_si512();
    for (channel, value) in accumulated.into_iter().enumerate() {
        let value = _mm512_min_ps(_mm512_max_ps(value, zero), one);
        let quantized = _mm512_cvttps_epi32(_mm512_add_ps(
            _mm512_mul_ps(value, _mm512_set1_ps(255.0)),
            _mm512_set1_ps(0.5),
        ));
        packed = _mm512_or_si512(
            packed,
            _mm512_sllv_epi32(
                _mm512_and_si512(quantized, byte_mask),
                _mm512_set1_epi32((channel * 8) as i32),
            ),
        );
    }
    blend_rgba8_vector(destination, packed, active, BlendMode::SrcOver);
    covered
}

#[target_feature(enable = "avx512f")]
unsafe fn semantic_shape_mask(
    x: __m512,
    y: __m512,
    bounds: Rect,
    primitive: &ShapePrimitive,
    inset: f32,
    active: __mmask16,
) -> __mmask16 {
    let left = bounds.x + inset;
    let top = bounds.y + inset;
    let right = bounds.x + bounds.width - inset;
    let bottom = bounds.y + bounds.height - inset;
    if left >= right || top >= bottom {
        return 0;
    }
    let mut inside = active
        & _mm512_cmp_ps_mask::<_CMP_GE_OQ>(x, _mm512_set1_ps(left))
        & _mm512_cmp_ps_mask::<_CMP_LT_OQ>(x, _mm512_set1_ps(right))
        & _mm512_cmp_ps_mask::<_CMP_GE_OQ>(y, _mm512_set1_ps(top))
        & _mm512_cmp_ps_mask::<_CMP_LT_OQ>(y, _mm512_set1_ps(bottom));
    if inside == 0 {
        return 0;
    }
    let one = _mm512_set1_ps(1.0);
    match primitive {
        ShapePrimitive::Rect => inside,
        ShapePrimitive::Ellipse => {
            let rx = (right - left) * 0.5;
            let ry = (bottom - top) * 0.5;
            let nx = _mm512_div_ps(
                _mm512_sub_ps(x, _mm512_set1_ps((left + right) * 0.5)),
                _mm512_set1_ps(rx),
            );
            let ny = _mm512_div_ps(
                _mm512_sub_ps(y, _mm512_set1_ps((top + bottom) * 0.5)),
                _mm512_set1_ps(ry),
            );
            let distance = _mm512_add_ps(_mm512_mul_ps(nx, nx), _mm512_mul_ps(ny, ny));
            inside &= _mm512_cmp_ps_mask::<_CMP_LE_OQ>(distance, one);
            inside
        }
        ShapePrimitive::RoundedRect { radius } => {
            let rx = (radius[0] - inset).max(0.0).min((right - left) * 0.5);
            let ry = (radius[1] - inset).max(0.0).min((bottom - top) * 0.5);
            if rx == 0.0 || ry == 0.0 {
                return inside;
            }
            let cx = _mm512_min_ps(
                _mm512_max_ps(x, _mm512_set1_ps(left + rx)),
                _mm512_set1_ps(right - rx),
            );
            let cy = _mm512_min_ps(
                _mm512_max_ps(y, _mm512_set1_ps(top + ry)),
                _mm512_set1_ps(bottom - ry),
            );
            let nx = _mm512_div_ps(_mm512_sub_ps(x, cx), _mm512_set1_ps(rx));
            let ny = _mm512_div_ps(_mm512_sub_ps(y, cy), _mm512_set1_ps(ry));
            let distance = _mm512_add_ps(_mm512_mul_ps(nx, nx), _mm512_mul_ps(ny, ny));
            inside &= _mm512_cmp_ps_mask::<_CMP_LE_OQ>(distance, one);
            inside
        }
        ShapePrimitive::AssetMask { .. } => 0,
    }
}

#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn image_clip_mask(
    local_x: __m512,
    local_y: __m512,
    bounds: Rect,
    clip: ImageClipGeometry,
    active: __mmask16,
) -> __mmask16 {
    let half_width = bounds.width * 0.5;
    let half_height = bounds.height * 0.5;
    if half_width <= 0.0 || half_height <= 0.0 {
        return 0;
    }
    let zero = _mm512_setzero_ps();
    let one = _mm512_set1_ps(1.0);
    let center_x = _mm512_set1_ps(bounds.x + half_width);
    let center_y = _mm512_set1_ps(bounds.y + half_height);
    let delta_x = _mm512_sub_ps(local_x, center_x);
    let delta_y = _mm512_sub_ps(local_y, center_y);
    let abs_x = _mm512_max_ps(delta_x, _mm512_sub_ps(zero, delta_x));
    let abs_y = _mm512_max_ps(delta_y, _mm512_sub_ps(zero, delta_y));
    let clip_mask = match clip {
        ImageClipGeometry::Ellipse => {
            let normalized_x = _mm512_div_ps(delta_x, _mm512_set1_ps(half_width));
            let normalized_y = _mm512_div_ps(delta_y, _mm512_set1_ps(half_height));
            let distance = _mm512_fmadd_ps(
                normalized_x,
                normalized_x,
                _mm512_mul_ps(normalized_y, normalized_y),
            );
            _mm512_cmp_ps_mask::<_CMP_LE_OQ>(distance, one)
        }
        ImageClipGeometry::RoundedRect { radius } => {
            let radius_x = radius[0].abs().min(half_width);
            let radius_y = radius[1].abs().min(half_height);
            if radius_x == 0.0 || radius_y == 0.0 {
                active
            } else {
                let distance_x = _mm512_sub_ps(abs_x, _mm512_set1_ps(half_width - radius_x));
                let distance_y = _mm512_sub_ps(abs_y, _mm512_set1_ps(half_height - radius_y));
                let inside_axis = _mm512_cmp_ps_mask::<_CMP_LE_OQ>(distance_x, zero)
                    | _mm512_cmp_ps_mask::<_CMP_LE_OQ>(distance_y, zero);
                let normalized_x = _mm512_div_ps(distance_x, _mm512_set1_ps(radius_x));
                let normalized_y = _mm512_div_ps(distance_y, _mm512_set1_ps(radius_y));
                let corner_distance = _mm512_fmadd_ps(
                    normalized_x,
                    normalized_x,
                    _mm512_mul_ps(normalized_y, normalized_y),
                );
                inside_axis | _mm512_cmp_ps_mask::<_CMP_LE_OQ>(corner_distance, one)
            }
        }
    };
    active & clip_mask
}

#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn blend_rgba8_vector(
    destination: *mut u8,
    source: __m512i,
    mut active: __mmask16,
    blend_mode: BlendMode,
) {
    let byte_mask = _mm512_set1_epi32(0xff);
    let source_alpha = _mm512_and_si512(_mm512_srli_epi32::<24>(source), byte_mask);

    if blend_mode == BlendMode::SrcOver {
        let opaque =
            active & _mm512_cmpeq_epi32_mask(source_alpha, _mm512_set1_epi32(u8::MAX as i32));
        if opaque != 0 {
            _mm512_mask_storeu_epi32(destination.cast(), opaque, source);
        }
        let transparent = active & _mm512_cmpeq_epi32_mask(source_alpha, _mm512_setzero_si512());
        active &= !(opaque | transparent);
        if active == 0 {
            return;
        }
    }

    let destination_pixels = _mm512_maskz_loadu_epi32(active, destination.cast());
    let destination_alpha =
        _mm512_and_si512(_mm512_srli_epi32::<24>(destination_pixels), byte_mask);
    let inverse_source_alpha = _mm512_sub_epi32(_mm512_set1_epi32(255), source_alpha);
    let mut packed = _mm512_setzero_si512();

    for channel in 0..4 {
        let shift = channel * 8;
        let source_channel = _mm512_and_si512(
            _mm512_srlv_epi32(source, _mm512_set1_epi32(shift)),
            byte_mask,
        );
        let destination_channel = _mm512_and_si512(
            _mm512_srlv_epi32(destination_pixels, _mm512_set1_epi32(shift)),
            byte_mask,
        );
        let output = match blend_mode {
            BlendMode::SrcOver => _mm512_min_epi32(
                _mm512_add_epi32(
                    source_channel,
                    mul_div_255_round(destination_channel, inverse_source_alpha),
                ),
                byte_mask,
            ),
            BlendMode::SrcIn => mul_div_255_round(source_channel, destination_alpha),
            BlendMode::DstIn => mul_div_255_round(destination_channel, source_alpha),
            BlendMode::Multiply | BlendMode::Screen | BlendMode::Add => return,
        };
        packed = _mm512_or_si512(packed, _mm512_sllv_epi32(output, _mm512_set1_epi32(shift)));
    }

    _mm512_mask_storeu_epi32(destination.cast(), active, packed);
}

#[target_feature(enable = "avx512f")]
unsafe fn mul_div_255_round(left: __m512i, right: __m512i) -> __m512i {
    let product = _mm512_mullo_epi32(left, right);
    let biased = _mm512_add_epi32(product, _mm512_set1_epi32(128));
    _mm512_srli_epi32::<8>(_mm512_add_epi32(biased, _mm512_srli_epi32::<8>(biased)))
}
