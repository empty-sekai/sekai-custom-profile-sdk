use std::arch::x86_64::*;

use super::{
    PlannedCommand, SdfAccumulationMode, SdfAtlasSource, SdfCommandMaterial, SdfDestination,
    SdfExecutionStats, SdfPrimitiveKind, SdfSwizzledFormat, SdfSwizzledPage, SdfTileError,
    SdfTilePlan,
};

const LANES: usize = 16;
const LANE_OFFSETS: [f32; LANES] = [
    0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
];

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi,fma")]
pub(super) unsafe fn execute(
    plan: &SdfTilePlan,
    atlas: &impl SdfAtlasSource,
    destination: SdfDestination,
    output: &mut [u8],
    accumulation: SdfAccumulationMode,
) -> Result<SdfExecutionStats, SdfTileError> {
    let pixel_count = usize::try_from(plan.grid.canvas_width)
        .ok()
        .and_then(|width| {
            usize::try_from(plan.grid.canvas_height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or(SdfTileError::SizeOverflow)?;
    let expected = pixel_count
        .checked_mul(4)
        .ok_or(SdfTileError::SizeOverflow)?;
    if output.len() != expected {
        return Err(SdfTileError::OutputLength {
            expected,
            actual: output.len(),
        });
    }
    if destination == SdfDestination::LoadExisting
        && accumulation == SdfAccumulationMode::F32Tile
        && plan.direct_axis_shape_spans().is_some()
    {
        return execute_axis_shape_direct_over(plan, output);
    }
    if let SdfDestination::Clear(clear) = destination {
        for pixel in output.chunks_exact_mut(4) {
            pixel.copy_from_slice(&clear);
        }
    }

    let tile_width = usize::from(plan.grid.tile_width);
    let tile_height = usize::from(plan.grid.tile_height);
    let tile_pixels = tile_width
        .checked_mul(tile_height)
        .ok_or(SdfTileError::SizeOverflow)?;
    let clear_f32 = match destination {
        SdfDestination::Clear(clear) => clear.map(|channel| f32::from(channel) / 255.0),
        SdfDestination::LoadExisting => [0.0; 4],
    };
    let mut tile = [
        vec![clear_f32[0]; tile_pixels],
        vec![clear_f32[1]; tile_pixels],
        vec![clear_f32[2]; tile_pixels],
        vec![clear_f32[3]; tile_pixels],
    ];
    let mut stats = SdfExecutionStats::default();
    let tiles_x = plan.grid.tiles_x();
    let tile_count =
        u32::try_from(plan.tile_offsets.len() - 1).map_err(|_| SdfTileError::SizeOverflow)?;

    for tile_index in 0..tile_count {
        let spans = plan
            .spans_for_tile(tile_index)
            .ok_or(SdfTileError::CorruptPlan)?;
        if spans.is_empty() {
            continue;
        }
        let tile_x = tile_index % tiles_x;
        let tile_y = tile_index / tiles_x;
        let origin_x = tile_x * u32::from(plan.grid.tile_width);
        let origin_y = tile_y * u32::from(plan.grid.tile_height);
        let valid_width = (plan.grid.canvas_width - origin_x).min(u32::from(plan.grid.tile_width));
        let valid_height =
            (plan.grid.canvas_height - origin_y).min(u32::from(plan.grid.tile_height));
        match destination {
            SdfDestination::Clear(_) => {
                for channel in 0..4 {
                    tile[channel].fill(clear_f32[channel]);
                }
            }
            SdfDestination::LoadExisting => {
                for channel in &mut tile {
                    channel.fill(0.0);
                }
                load_existing_tile_rgba8(
                    &mut tile,
                    tile_width,
                    valid_width as usize,
                    valid_height as usize,
                    origin_x as usize,
                    origin_y as usize,
                    plan.grid.canvas_width as usize,
                    output,
                );
            }
        }

        let mut cached_command_index = usize::MAX;
        let mut cached_command = None;
        let mut cached_page = None;
        for span in spans {
            let command_index =
                usize::try_from(span.command).map_err(|_| SdfTileError::CorruptPlan)?;
            if command_index != cached_command_index {
                let command = plan
                    .commands
                    .get(command_index)
                    .ok_or(SdfTileError::CorruptPlan)?;
                let page = if plan.axis_shape_program(command_index).is_some() {
                    None
                } else {
                    let page = atlas
                        .swizzled_page(command.source.atlas_set, command.source.atlas_page)
                        .ok_or(SdfTileError::SimdAtlasUnavailable {
                            atlas_set: command.source.atlas_set,
                            page: command.source.atlas_page,
                        })?;
                    validate_page(command, page)?;
                    Some(page)
                };
                cached_command_index = command_index;
                cached_command = Some(command);
                cached_page = page;
            }
            let command = cached_command.ok_or(SdfTileError::CorruptPlan)?;
            let y = origin_y + u32::from(span.row);
            if let Some(program) = plan.axis_shape_program(command_index) {
                let mut fast_fragments = 0u64;
                let mut fast_spans = 0u64;
                super::for_each_axis_shape_span(
                    command,
                    program,
                    y,
                    origin_x + u32::from(span.x0),
                    origin_x + u32::from(span.x1),
                    |begin, end, source| {
                        fast_spans += 1;
                        fast_fragments += u64::from(end - begin);
                        blend_constant_span(
                            &mut tile,
                            usize::from(span.row) * tile_width + (begin - origin_x) as usize,
                            (end - begin) as usize,
                            source,
                            accumulation,
                        );
                    },
                )?;
                stats.shaded_fragment_count += fast_fragments;
                stats.shape_shaded_fragment_count += fast_fragments;
                stats.blended_fragment_count += fast_fragments;
                stats.shape_blended_fragment_count += fast_fragments;
                stats.precomputed_shape_fragment_count += fast_fragments;
                stats.precomputed_shape_span_count += fast_spans;
                continue;
            }
            let page = cached_page.ok_or(SdfTileError::CorruptPlan)?;
            let py = y as f32 + 0.5;
            let mut local_x = usize::from(span.x0);
            let end_x = usize::from(span.x1);
            while local_x < end_x {
                let remaining = (end_x - local_x).min(LANES);
                let mask = first_n_mask(remaining);
                let first_x = (origin_x as usize + local_x) as f32 + 0.5;
                let lanes = _mm512_loadu_ps(LANE_OFFSETS.as_ptr());
                let px = _mm512_add_ps(_mm512_set1_ps(first_x), lanes);
                let tx = _mm512_fmadd_ps(
                    _mm512_set1_ps(command.tx_dx),
                    px,
                    _mm512_fmadd_ps(
                        _mm512_set1_ps(command.tx_dy),
                        _mm512_set1_ps(py),
                        _mm512_set1_ps(command.tx_c),
                    ),
                );
                let ty = _mm512_fmadd_ps(
                    _mm512_set1_ps(command.ty_dx),
                    px,
                    _mm512_fmadd_ps(
                        _mm512_set1_ps(command.ty_dy),
                        _mm512_set1_ps(py),
                        _mm512_set1_ps(command.ty_c),
                    ),
                );
                let pixel = usize::from(span.row) * tile_width + local_x;
                let (source, used_swizzle, sampled_per_lane) = match command.source.kind {
                    SdfPrimitiveKind::Text => shade_text_packet(command, page, tx, ty, mask)?,
                    SdfPrimitiveKind::Shape => shade_shape_packet(command, page, tx, ty, mask)?,
                };
                blend_packet(&mut tile, pixel, mask, source, accumulation);

                let fragments = remaining as u64;
                stats.simd_packet_count += 1;
                if used_swizzle {
                    stats.swizzled_packet_count += 1;
                } else {
                    stats.gather_fallback_packet_count += 1;
                }
                stats.shaded_fragment_count += fragments;
                stats.sampled_texel_count += fragments * sampled_per_lane;
                stats.blended_fragment_count += fragments;
                match command.source.kind {
                    SdfPrimitiveKind::Text => {
                        stats.text_shaded_fragment_count += fragments;
                        stats.text_blended_fragment_count += fragments;
                    }
                    SdfPrimitiveKind::Shape => {
                        stats.shape_shaded_fragment_count += fragments;
                        stats.shape_blended_fragment_count += fragments;
                    }
                }
                local_x += LANES;
            }
        }

        let write_span = |row: usize, x0: usize, x1: usize, output: &mut [u8]| unsafe {
            write_span_rgba8(
                &tile,
                tile_width,
                row,
                x0,
                x1,
                origin_x as usize,
                origin_y as usize,
                plan.grid.canvas_width as usize,
                output,
            );
        };
        match destination {
            SdfDestination::Clear(_) => {
                for row in 0..valid_height as usize {
                    write_span(row, 0, valid_width as usize, output);
                }
            }
            SdfDestination::LoadExisting => {
                for span in spans {
                    write_span(
                        usize::from(span.row),
                        usize::from(span.x0),
                        usize::from(span.x1),
                        output,
                    );
                }
            }
        }
    }
    Ok(stats)
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi,fma")]
unsafe fn blend_constant_span(
    tile: &mut [Vec<f32>; 4],
    mut pixel: usize,
    mut length: usize,
    source: [f32; 4],
    accumulation: SdfAccumulationMode,
) {
    let source = source.map(|channel| _mm512_set1_ps(channel));
    while length != 0 {
        let packet = length.min(LANES);
        blend_packet(tile, pixel, first_n_mask(packet), source, accumulation);
        pixel += packet;
        length -= packet;
    }
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi,fma")]
unsafe fn execute_axis_shape_direct_over(
    plan: &SdfTilePlan,
    output: &mut [u8],
) -> Result<SdfExecutionStats, SdfTileError> {
    let spans = plan
        .direct_axis_shape_spans()
        .ok_or(SdfTileError::CorruptPlan)?;
    let mut stats = SdfExecutionStats {
        direct_output_run_count: 1,
        ..SdfExecutionStats::default()
    };
    for span in spans {
        let fragments = u64::from(span.x1 - span.x0);
        let packets = fragments.div_ceil(LANES as u64);
        blend_output_constant_span_rgba8(
            output,
            (span.y as usize * plan.grid.canvas_width as usize + span.x0 as usize) * 4,
            (span.x1 - span.x0) as usize,
            span.source,
        );
        stats.shaded_fragment_count += fragments;
        stats.shape_shaded_fragment_count += fragments;
        stats.blended_fragment_count += fragments;
        stats.shape_blended_fragment_count += fragments;
        stats.simd_packet_count += packets;
        stats.direct_output_packet_count += packets;
        stats.precomputed_shape_fragment_count += fragments;
        stats.precomputed_shape_span_count += 1;
    }
    Ok(stats)
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi,fma")]
unsafe fn blend_output_constant_span_rgba8(
    output: &mut [u8],
    mut output_pixel: usize,
    mut pixels: usize,
    source: [f32; 4],
) {
    let source = source.map(|channel| _mm512_set1_ps(channel));
    let channel_base = _mm512_castsi128_si512(_mm_setr_epi8(
        0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 48, 52, 56, 60,
    ));
    let inverse_alpha = _mm512_sub_ps(_mm512_set1_ps(1.0), source[3]);
    let scale = _mm512_set1_ps(1.0 / 255.0);
    while pixels != 0 {
        let lanes = pixels.min(LANES);
        let byte_mask = first_n_byte_mask(lanes * 4);
        let packed = _mm512_maskz_loadu_epi8(byte_mask, output.as_ptr().add(output_pixel).cast());
        let mut quantized = [_mm_setzero_si128(); 4];
        for channel in 0..4 {
            let indices = _mm512_add_epi8(channel_base, _mm512_set1_epi8(channel as i8));
            let bytes = _mm512_castsi512_si128(_mm512_permutexvar_epi8(indices, packed));
            let destination = _mm512_mul_ps(_mm512_cvtepi32_ps(_mm512_cvtepu8_epi32(bytes)), scale);
            quantized[channel] =
                quantize_packet(_mm512_fmadd_ps(destination, inverse_alpha, source[channel]));
        }
        let [r, g, b, a] = quantized;
        let rg_lo = _mm_unpacklo_epi8(r, g);
        let rg_hi = _mm_unpackhi_epi8(r, g);
        let ba_lo = _mm_unpacklo_epi8(b, a);
        let ba_hi = _mm_unpackhi_epi8(b, a);
        let rgba0 = _mm_unpacklo_epi16(rg_lo, ba_lo);
        let rgba1 = _mm_unpackhi_epi16(rg_lo, ba_lo);
        let rgba2 = _mm_unpacklo_epi16(rg_hi, ba_hi);
        let rgba3 = _mm_unpackhi_epi16(rg_hi, ba_hi);
        let packed = _mm512_inserti32x4(
            _mm512_inserti32x4(
                _mm512_inserti32x4(_mm512_castsi128_si512(rgba0), rgba1, 1),
                rgba2,
                2,
            ),
            rgba3,
            3,
        );
        _mm512_mask_storeu_epi8(
            output.as_mut_ptr().add(output_pixel).cast(),
            byte_mask,
            packed,
        );
        output_pixel += lanes * 4;
        pixels -= lanes;
    }
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi")]
unsafe fn load_existing_tile_rgba8(
    tile: &mut [Vec<f32>; 4],
    tile_width: usize,
    valid_width: usize,
    valid_height: usize,
    origin_x: usize,
    origin_y: usize,
    canvas_width: usize,
    output: &[u8],
) {
    let channel_base = _mm512_castsi128_si512(_mm_setr_epi8(
        0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 48, 52, 56, 60,
    ));
    let scale = _mm512_set1_ps(1.0 / 255.0);
    for row in 0..valid_height {
        let mut x = 0usize;
        while x < valid_width {
            let remaining = (valid_width - x).min(LANES);
            let active = first_n_mask(remaining);
            let output_pixel = ((origin_y + row) * canvas_width + origin_x + x) * 4;
            let rgba = _mm512_maskz_loadu_epi8(
                first_n_byte_mask(remaining * 4),
                output.as_ptr().add(output_pixel).cast(),
            );
            let tile_pixel = row * tile_width + x;
            for channel in 0..4 {
                let indices = _mm512_add_epi8(channel_base, _mm512_set1_epi8(channel as i8));
                let bytes = _mm512_castsi512_si128(_mm512_permutexvar_epi8(indices, rgba));
                let values = _mm512_mul_ps(_mm512_cvtepi32_ps(_mm512_cvtepu8_epi32(bytes)), scale);
                _mm512_mask_storeu_ps(tile[channel].as_mut_ptr().add(tile_pixel), active, values);
            }
            x += LANES;
        }
    }
}

#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn write_span_rgba8(
    tile: &[Vec<f32>; 4],
    tile_width: usize,
    row: usize,
    x0: usize,
    x1: usize,
    origin_x: usize,
    origin_y: usize,
    canvas_width: usize,
    output: &mut [u8],
) {
    let mut x = x0;
    while x < x1 {
        let remaining = (x1 - x).min(LANES);
        let active = first_n_mask(remaining);
        let tile_pixel = row * tile_width + x;
        let r = quantize_packet(_mm512_maskz_loadu_ps(
            active,
            tile[0].as_ptr().add(tile_pixel),
        ));
        let g = quantize_packet(_mm512_maskz_loadu_ps(
            active,
            tile[1].as_ptr().add(tile_pixel),
        ));
        let b = quantize_packet(_mm512_maskz_loadu_ps(
            active,
            tile[2].as_ptr().add(tile_pixel),
        ));
        let a = quantize_packet(_mm512_maskz_loadu_ps(
            active,
            tile[3].as_ptr().add(tile_pixel),
        ));

        let rg_lo = _mm_unpacklo_epi8(r, g);
        let rg_hi = _mm_unpackhi_epi8(r, g);
        let ba_lo = _mm_unpacklo_epi8(b, a);
        let ba_hi = _mm_unpackhi_epi8(b, a);
        let rgba0 = _mm_unpacklo_epi16(rg_lo, ba_lo);
        let rgba1 = _mm_unpackhi_epi16(rg_lo, ba_lo);
        let rgba2 = _mm_unpacklo_epi16(rg_hi, ba_hi);
        let rgba3 = _mm_unpackhi_epi16(rg_hi, ba_hi);
        let packed = _mm512_inserti32x4(
            _mm512_inserti32x4(
                _mm512_inserti32x4(_mm512_castsi128_si512(rgba0), rgba1, 1),
                rgba2,
                2,
            ),
            rgba3,
            3,
        );
        let output_pixel = ((origin_y + row) * canvas_width + origin_x + x) * 4;
        _mm512_mask_storeu_epi8(
            output.as_mut_ptr().add(output_pixel).cast(),
            first_n_byte_mask(remaining * 4),
            packed,
        );
        x += LANES;
    }
}

#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn quantize_packet(value: __m512) -> __m128i {
    let rounded = floor_ps(_mm512_add_ps(
        _mm512_mul_ps(clamp01(value), _mm512_set1_ps(255.0)),
        _mm512_set1_ps(0.5),
    ));
    _mm512_cvtusepi32_epi8(_mm512_cvttps_epi32(rounded))
}

fn validate_page(command: &PlannedCommand, page: SdfSwizzledPage<'_>) -> Result<(), SdfTileError> {
    let expected = match command.source.kind {
        SdfPrimitiveKind::Text => SdfSwizzledFormat::TextR8,
        SdfPrimitiveKind::Shape => SdfSwizzledFormat::ShapeRg8,
    };
    if page.format != expected {
        return Err(SdfTileError::AtlasFormatMismatch {
            kind: command.source.kind,
        });
    }
    let channels = match page.format {
        SdfSwizzledFormat::TextR8 => 1usize,
        SdfSwizzledFormat::ShapeRg8 => 2usize,
    };
    let expected_bytes = usize::try_from(page.width)
        .ok()
        .and_then(|width| {
            usize::try_from(page.height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|texels| texels.checked_mul(channels))
        .ok_or(SdfTileError::SizeOverflow)?;
    if page.width % 8 != 0 || page.height % 8 != 0 || page.payload.len() != expected_bytes {
        return Err(SdfTileError::SimdAtlasUnavailable {
            atlas_set: command.source.atlas_set,
            page: command.source.atlas_page,
        });
    }
    Ok(())
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi,fma")]
unsafe fn shade_text_packet(
    command: &PlannedCommand,
    page: SdfSwizzledPage<'_>,
    tx: __m512,
    ty: __m512,
    active: __mmask16,
) -> Result<([__m512; 4], bool, u64), SdfTileError> {
    let material = match command.source.material {
        SdfCommandMaterial::Text(material) => material,
        _ => return Err(SdfTileError::MaterialKindMismatch { command: 0 }),
    };
    let floor_x = floor_ps(tx);
    let floor_y = floor_ps(ty);
    let fx = _mm512_sub_ps(tx, floor_x);
    let fy = _mm512_sub_ps(ty, floor_y);
    let ix = _mm512_cvttps_epi32(floor_x);
    let iy = _mm512_cvttps_epi32(floor_y);
    let [rect_x, rect_y, rect_width, rect_height] = command.source.atlas_rect;
    let x0 = clamp_i32(ix, rect_x as i32, (rect_x + rect_width - 1) as i32);
    let y0 = clamp_i32(iy, rect_y as i32, (rect_y + rect_height - 1) as i32);
    let x1 = clamp_i32(
        _mm512_add_epi32(ix, _mm512_set1_epi32(1)),
        rect_x as i32,
        (rect_x + rect_width - 1) as i32,
    );
    let y1 = clamp_i32(
        _mm512_add_epi32(iy, _mm512_set1_epi32(1)),
        rect_y as i32,
        (rect_y + rect_height - 1) as i32,
    );
    let (texels, used_swizzle) = sample_text(page, x0, x1, y0, y1, active)?;
    let [p00, p10, p01, p11] = texels;
    let top = _mm512_fmadd_ps(_mm512_sub_ps(p10, p00), fx, p00);
    let bottom = _mm512_fmadd_ps(_mm512_sub_ps(p11, p01), fx, p01);
    let sdf = _mm512_fmadd_ps(_mm512_sub_ps(bottom, top), fy, top);
    let zero = _mm512_setzero_ps();
    let one = _mm512_set1_ps(1.0);
    let face_t = clamp01(_mm512_fmadd_ps(
        sdf,
        _mm512_set1_ps(material.face_scale.max(0.0001)),
        _mm512_set1_ps(-material.face_bias),
    ));
    let outline_base = clamp01(_mm512_fmadd_ps(
        sdf,
        _mm512_set1_ps(material.outline_scale.max(0.0001)),
        _mm512_set1_ps(-material.outline_bias),
    ));
    let outline_edge = _mm512_min_ps(
        _mm512_max_ps(_mm512_mul_ps(sdf, _mm512_set1_ps(12.5)), zero),
        one,
    );
    let outline_t = _mm512_mul_ps(outline_base, outline_edge);
    let outline_weight = _mm512_mul_ps(
        outline_t,
        // Match the scalar oracle's operation order exactly. It performs a
        // multiply followed by a subtraction rather than a fused negative
        // multiply-add; this matters at the final RGBA8 rounding boundary.
        _mm512_sub_ps(one, _mm512_mul_ps(_mm512_set1_ps(material.face[3]), face_t)),
    );
    let vertex_alpha = _mm512_set1_ps(material.vertex_alpha.clamp(0.0, 1.0));
    let mut source = [_mm512_setzero_ps(); 4];
    for channel in 0..4 {
        source[channel] = _mm512_mul_ps(
            _mm512_fmadd_ps(
                _mm512_set1_ps(material.outline[channel]),
                outline_weight,
                _mm512_mul_ps(_mm512_set1_ps(material.face[channel]), face_t),
            ),
            vertex_alpha,
        );
    }
    Ok((source, used_swizzle, 4))
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi,fma")]
unsafe fn shade_shape_packet(
    command: &PlannedCommand,
    page: SdfSwizzledPage<'_>,
    tx: __m512,
    ty: __m512,
    active: __mmask16,
) -> Result<([__m512; 4], bool, u64), SdfTileError> {
    let material = match command.source.material {
        SdfCommandMaterial::Shape(material) => material,
        _ => return Err(SdfTileError::MaterialKindMismatch { command: 0 }),
    };
    let nearest_x = _mm512_cvttps_epi32(floor_ps(_mm512_add_ps(tx, _mm512_set1_ps(0.5))));
    let nearest_y = _mm512_cvttps_epi32(floor_ps(_mm512_add_ps(ty, _mm512_set1_ps(0.5))));
    let [rect_x, rect_y, rect_width, rect_height] = command.source.atlas_rect;
    let x = clamp_i32(nearest_x, rect_x as i32, (rect_x + rect_width - 1) as i32);
    let y = clamp_i32(nearest_y, rect_y as i32, (rect_y + rect_height - 1) as i32);
    let ((distance, gate), used_swizzle) = sample_shape(page, x, y, active)?;
    let gate_scale = _mm512_mul_ps(gate, _mm512_set1_ps(1.0 / 255.0));
    let face_coverage = shape_coverage(
        distance,
        gate_scale,
        command.shape_face_offset,
        command.shape_coverage_scale,
    );
    let outline_coverage = shape_coverage(
        distance,
        gate_scale,
        command.shape_outline_offset,
        command.shape_coverage_scale,
    );
    let outline_weight = _mm512_mul_ps(
        outline_coverage,
        _mm512_sub_ps(_mm512_set1_ps(1.0), face_coverage),
    );
    let face_alpha = _mm512_mul_ps(_mm512_set1_ps(material.face[3]), face_coverage);
    let outline_above_weight = _mm512_mul_ps(
        outline_weight,
        _mm512_sub_ps(_mm512_set1_ps(1.0), face_alpha),
    );
    let mut source = [_mm512_setzero_ps(); 4];
    for channel in 0..4 {
        source[channel] = _mm512_fmadd_ps(
            _mm512_set1_ps(material.outline[channel]),
            outline_above_weight,
            _mm512_mul_ps(_mm512_set1_ps(material.face[channel]), face_coverage),
        );
    }
    Ok((source, used_swizzle, 1))
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi,fma")]
unsafe fn sample_text(
    page: SdfSwizzledPage<'_>,
    x0: __m512i,
    x1: __m512i,
    y0: __m512i,
    y1: __m512i,
    active: __mmask16,
) -> Result<([__m512; 4], bool), SdfTileError> {
    let block00 = block_ids(page.width, x0, y0);
    let block10 = block_ids(page.width, x1, y0);
    let block01 = block_ids(page.width, x0, y1);
    let block11 = block_ids(page.width, x1, y1);
    let first_block = _mm512_cvtsi512_si32(block00);
    let common = _mm512_set1_epi32(first_block);
    let same = _mm512_cmpeq_epi32_mask(block00, common)
        & _mm512_cmpeq_epi32_mask(block10, common)
        & _mm512_cmpeq_epi32_mask(block01, common)
        & _mm512_cmpeq_epi32_mask(block11, common)
        & active;
    let denominator = _mm512_set1_ps(255.0);
    let indices = text_sample_indices(x0, x1, y0, y1);
    if same == active {
        let sampled = sample_text_block(page, first_block, indices)?;
        return Ok((
            sampled.map(|value| _mm512_div_ps(_mm512_cvtepi32_ps(value), denominator)),
            true,
        ));
    }

    // A horizontally scaled text packet commonly straddles exactly two 8x8
    // swizzle blocks. Four i32 gathers are substantially more expensive than
    // loading both contiguous blocks and selecting lanes from them.
    let blocks = [block00, block10, block01, block11];
    let next_block = first_block.checked_add(1);
    let previous_block = first_block.checked_sub(1);
    for second_block in [next_block, previous_block].into_iter().flatten() {
        if text_blocks_covered_by_pair(blocks, first_block, second_block, active) {
            let first = sample_text_block(page, first_block, indices)?;
            let second = sample_text_block(page, second_block, indices)?;
            let sampled = std::array::from_fn(|index| {
                let second_lanes =
                    _mm512_cmpeq_epi32_mask(blocks[index], _mm512_set1_epi32(second_block))
                        & active;
                _mm512_mask_blend_epi32(second_lanes, first[index], second[index])
            });
            return Ok((
                sampled.map(|value| _mm512_div_ps(_mm512_cvtepi32_ps(value), denominator)),
                true,
            ));
        }
    }
    let p00 = gather_texel(page, x0, y0, 0, active)?;
    let p10 = gather_texel(page, x1, y0, 0, active)?;
    let p01 = gather_texel(page, x0, y1, 0, active)?;
    let p11 = gather_texel(page, x1, y1, 0, active)?;
    let byte_mask = _mm512_set1_epi32(0xff);
    Ok((
        [p00, p10, p01, p11].map(|value| {
            _mm512_div_ps(
                _mm512_cvtepi32_ps(_mm512_and_si512(value, byte_mask)),
                denominator,
            )
        }),
        false,
    ))
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi")]
unsafe fn text_sample_indices(x0: __m512i, x1: __m512i, y0: __m512i, y1: __m512i) -> __m512i {
    let i00 = _mm512_cvtepi32_epi8(local_offsets(x0, y0));
    let i10 = _mm512_cvtepi32_epi8(local_offsets(x1, y0));
    let i01 = _mm512_cvtepi32_epi8(local_offsets(x0, y1));
    let i11 = _mm512_cvtepi32_epi8(local_offsets(x1, y1));
    let mut indices = _mm512_castsi128_si512(i00);
    indices = _mm512_inserti32x4(indices, i10, 1);
    indices = _mm512_inserti32x4(indices, i01, 2);
    _mm512_inserti32x4(indices, i11, 3)
}

#[target_feature(enable = "avx512f")]
unsafe fn text_blocks_covered_by_pair(
    blocks: [__m512i; 4],
    first_block: i32,
    second_block: i32,
    active: __mmask16,
) -> bool {
    let first = _mm512_set1_epi32(first_block);
    let second = _mm512_set1_epi32(second_block);
    blocks.into_iter().all(|block| {
        ((_mm512_cmpeq_epi32_mask(block, first) | _mm512_cmpeq_epi32_mask(block, second)) & active)
            == active
    })
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi")]
unsafe fn sample_text_block(
    page: SdfSwizzledPage<'_>,
    block: i32,
    indices: __m512i,
) -> Result<[__m512i; 4], SdfTileError> {
    let block_offset = usize::try_from(block)
        .ok()
        .and_then(|block| block.checked_mul(64))
        .filter(|offset| offset.saturating_add(64) <= page.payload.len())
        .ok_or(SdfTileError::CorruptPlan)?;
    let table = _mm512_loadu_si512(page.payload.as_ptr().add(block_offset).cast());
    let sampled = _mm512_permutexvar_epi8(indices, table);
    Ok([
        _mm512_cvtepu8_epi32(_mm512_castsi512_si128(sampled)),
        _mm512_cvtepu8_epi32(_mm512_extracti32x4_epi32(sampled, 1)),
        _mm512_cvtepu8_epi32(_mm512_extracti32x4_epi32(sampled, 2)),
        _mm512_cvtepu8_epi32(_mm512_extracti32x4_epi32(sampled, 3)),
    ])
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi,fma")]
unsafe fn sample_shape(
    page: SdfSwizzledPage<'_>,
    x: __m512i,
    y: __m512i,
    active: __mmask16,
) -> Result<((__m512, __m512), bool), SdfTileError> {
    let blocks = block_ids(page.width, x, y);
    let first_block = _mm512_cvtsi512_si32(blocks);
    let common = _mm512_set1_epi32(first_block);
    let first_lanes = _mm512_cmpeq_epi32_mask(blocks, common) & active;
    let base = _mm512_slli_epi32(local_offsets(x, y), 1);
    let distance_index = _mm512_cvtepi32_epi8(base);
    let gate_index = _mm512_cvtepi32_epi8(_mm512_add_epi32(base, _mm512_set1_epi32(1)));
    let mut indices = _mm512_castsi128_si512(distance_index);
    indices = _mm512_inserti32x4(indices, gate_index, 1);

    if first_lanes == active {
        let (distance, gate) = sample_shape_block(page, first_block, indices)?;
        return Ok((
            (_mm512_cvtepi32_ps(distance), _mm512_cvtepi32_ps(gate)),
            true,
        ));
    }

    // A 16-pixel packet normally straddles two horizontally adjacent 8x8
    // blocks. Keep that common case on contiguous loads instead of sending
    // every lane through an i32 gather. Negative X scales are handled by the
    // previous-block candidate as well.
    let next_block = first_block
        .checked_add(1)
        .ok_or(SdfTileError::CorruptPlan)?;
    let next_lanes = _mm512_cmpeq_epi32_mask(blocks, _mm512_set1_epi32(next_block)) & active;
    let previous_block = first_block
        .checked_sub(1)
        .ok_or(SdfTileError::CorruptPlan)?;
    let previous_lanes =
        _mm512_cmpeq_epi32_mask(blocks, _mm512_set1_epi32(previous_block)) & active;
    let (second_block, second_lanes) = if first_lanes | next_lanes == active {
        (next_block, next_lanes)
    } else if first_lanes | previous_lanes == active {
        (previous_block, previous_lanes)
    } else {
        if let Some((distance, gate)) = sample_shape_up_to_eight_blocks(
            page,
            blocks,
            first_block,
            first_lanes,
            indices,
            active,
        )? {
            return Ok((
                (_mm512_cvtepi32_ps(distance), _mm512_cvtepi32_ps(gate)),
                true,
            ));
        }
        let packed = gather_texel(page, x, y, 0, active)?;
        let distance = _mm512_and_si512(packed, _mm512_set1_epi32(0xff));
        let gate = _mm512_and_si512(_mm512_srli_epi32(packed, 8), _mm512_set1_epi32(0xff));
        return Ok((
            (_mm512_cvtepi32_ps(distance), _mm512_cvtepi32_ps(gate)),
            false,
        ));
    };
    let (first_distance, first_gate) = sample_shape_block(page, first_block, indices)?;
    let (second_distance, second_gate) = sample_shape_block(page, second_block, indices)?;
    let distance = _mm512_mask_blend_epi32(second_lanes, first_distance, second_distance);
    let gate = _mm512_mask_blend_epi32(second_lanes, first_gate, second_gate);
    Ok((
        (_mm512_cvtepi32_ps(distance), _mm512_cvtepi32_ps(gate)),
        true,
    ))
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi")]
unsafe fn sample_shape_up_to_eight_blocks(
    page: SdfSwizzledPage<'_>,
    blocks: __m512i,
    first_block: i32,
    first_lanes: __mmask16,
    indices: __m512i,
    active: __mmask16,
) -> Result<Option<(__m512i, __m512i)>, SdfTileError> {
    let mut block_ids = [0i32; 8];
    let mut lane_masks = [0u16; 8];
    block_ids[0] = first_block;
    lane_masks[0] = first_lanes;
    let mut covered = first_lanes;
    let mut block_count = 1usize;
    while covered != active && block_count < block_ids.len() {
        let missing = active & !covered;
        let packed = _mm512_maskz_compress_epi32(missing, blocks);
        let block = _mm512_cvtsi512_si32(packed);
        let lanes = _mm512_cmpeq_epi32_mask(blocks, _mm512_set1_epi32(block)) & active;
        block_ids[block_count] = block;
        lane_masks[block_count] = lanes;
        covered |= lanes;
        block_count += 1;
    }
    if covered != active {
        return Ok(None);
    }

    let (mut distance, mut gate) = sample_shape_block(page, block_ids[0], indices)?;
    for index in 1..block_count {
        let (next_distance, next_gate) = sample_shape_block(page, block_ids[index], indices)?;
        distance = _mm512_mask_blend_epi32(lane_masks[index], distance, next_distance);
        gate = _mm512_mask_blend_epi32(lane_masks[index], gate, next_gate);
    }
    Ok(Some((distance, gate)))
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi")]
unsafe fn sample_shape_block(
    page: SdfSwizzledPage<'_>,
    block: i32,
    indices: __m512i,
) -> Result<(__m512i, __m512i), SdfTileError> {
    let block_offset = usize::try_from(block)
        .ok()
        .and_then(|block| block.checked_mul(128))
        .filter(|offset| offset.saturating_add(128) <= page.payload.len())
        .ok_or(SdfTileError::CorruptPlan)?;
    let table0 = _mm512_loadu_si512(page.payload.as_ptr().add(block_offset).cast());
    let table1 = _mm512_loadu_si512(page.payload.as_ptr().add(block_offset + 64).cast());
    let sampled = _mm512_permutex2var_epi8(table0, indices, table1);
    Ok((
        _mm512_cvtepu8_epi32(_mm512_castsi512_si128(sampled)),
        _mm512_cvtepu8_epi32(_mm512_extracti32x4_epi32(sampled, 1)),
    ))
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi,fma")]
unsafe fn gather_texel(
    page: SdfSwizzledPage<'_>,
    x: __m512i,
    y: __m512i,
    channel: i32,
    active: __mmask16,
) -> Result<__m512i, SdfTileError> {
    let (block_bytes, channels) = match page.format {
        SdfSwizzledFormat::TextR8 => (64, 1),
        SdfSwizzledFormat::ShapeRg8 => (128, 2),
    };
    let offsets = _mm512_add_epi32(
        _mm512_add_epi32(
            _mm512_mullo_epi32(block_ids(page.width, x, y), _mm512_set1_epi32(block_bytes)),
            _mm512_mullo_epi32(local_offsets(x, y), _mm512_set1_epi32(channels)),
        ),
        _mm512_set1_epi32(channel),
    );
    let Some(max_offset) = page.payload.len().checked_sub(std::mem::size_of::<i32>()) else {
        return Err(SdfTileError::CorruptPlan);
    };
    let below_zero = _mm512_cmplt_epi32_mask(offsets, _mm512_setzero_si512());
    let above_payload = if max_offset > i32::MAX as usize {
        0
    } else {
        _mm512_cmpgt_epi32_mask(offsets, _mm512_set1_epi32(max_offset as i32))
    };
    if (below_zero | above_payload) & active != 0 {
        return Err(SdfTileError::CorruptPlan);
    }
    Ok(_mm512_mask_i32gather_epi32::<1>(
        _mm512_setzero_si512(),
        active,
        offsets,
        page.payload.as_ptr().cast(),
    ))
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi,fma")]
unsafe fn shape_coverage(distance: __m512, gate_scale: __m512, offset: f32, scale: f32) -> __m512 {
    let coverage = clamp01(_mm512_mul_ps(
        _mm512_add_ps(distance, _mm512_set1_ps(offset)),
        _mm512_set1_ps(scale),
    ));
    _mm512_mul_ps(coverage, gate_scale)
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi,fma")]
unsafe fn blend_packet(
    tile: &mut [Vec<f32>; 4],
    pixel: usize,
    active: __mmask16,
    source: [__m512; 4],
    accumulation: SdfAccumulationMode,
) {
    let inverse_alpha = _mm512_sub_ps(_mm512_set1_ps(1.0), source[3]);
    for channel in 0..4 {
        let destination = _mm512_maskz_loadu_ps(active, tile[channel].as_ptr().add(pixel));
        let mut result = _mm512_fmadd_ps(destination, inverse_alpha, source[channel]);
        if accumulation == SdfAccumulationMode::Rgba8Writeback {
            result = _mm512_mul_ps(
                floor_ps(_mm512_add_ps(
                    _mm512_mul_ps(clamp01(result), _mm512_set1_ps(255.0)),
                    _mm512_set1_ps(0.5),
                )),
                _mm512_set1_ps(1.0 / 255.0),
            );
        }
        _mm512_mask_storeu_ps(tile[channel].as_mut_ptr().add(pixel), active, result);
    }
}

#[target_feature(enable = "avx512f")]
unsafe fn floor_ps(value: __m512) -> __m512 {
    _mm512_roundscale_ps::<{ _MM_FROUND_TO_NEG_INF | _MM_FROUND_NO_EXC }>(value)
}

#[target_feature(enable = "avx512f")]
unsafe fn clamp01(value: __m512) -> __m512 {
    _mm512_min_ps(
        _mm512_max_ps(value, _mm512_setzero_ps()),
        _mm512_set1_ps(1.0),
    )
}

#[target_feature(enable = "avx512f")]
unsafe fn clamp_i32(value: __m512i, minimum: i32, maximum: i32) -> __m512i {
    _mm512_min_epi32(
        _mm512_max_epi32(value, _mm512_set1_epi32(minimum)),
        _mm512_set1_epi32(maximum),
    )
}

#[target_feature(enable = "avx512f")]
unsafe fn block_ids(width: u32, x: __m512i, y: __m512i) -> __m512i {
    _mm512_add_epi32(
        _mm512_mullo_epi32(
            _mm512_srli_epi32(y, 3),
            _mm512_set1_epi32((width / 8) as i32),
        ),
        _mm512_srli_epi32(x, 3),
    )
}

#[target_feature(enable = "avx512f")]
unsafe fn local_offsets(x: __m512i, y: __m512i) -> __m512i {
    _mm512_add_epi32(
        _mm512_slli_epi32(_mm512_and_si512(y, _mm512_set1_epi32(7)), 3),
        _mm512_and_si512(x, _mm512_set1_epi32(7)),
    )
}

fn first_n_mask(lanes: usize) -> __mmask16 {
    if lanes == LANES {
        u16::MAX
    } else {
        ((1u32 << lanes) - 1) as u16
    }
}

fn first_n_byte_mask(bytes: usize) -> __mmask64 {
    if bytes == 64 {
        u64::MAX
    } else {
        (1u64 << bytes) - 1
    }
}
