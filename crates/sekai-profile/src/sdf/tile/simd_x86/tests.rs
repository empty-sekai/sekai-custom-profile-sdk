use super::*;

fn simd_available() -> bool {
    std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
        && std::arch::is_x86_feature_detected!("avx512vbmi")
        && std::arch::is_x86_feature_detected!("fma")
}

/// Values on both sides of every rounding boundary of the 8-bit output.
fn rounding_boundaries() -> Vec<f32> {
    let mut values = vec![0.0, 1.0, 0.001_960_784_2];
    for step in 0..255u16 {
        let centre = (f32::from(step) + 0.5) / 255.0;
        values.extend([
            f32::from_bits(centre.to_bits() - 1),
            centre,
            f32::from_bits(centre.to_bits() + 1),
        ]);
    }
    values
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi,fma")]
unsafe fn quantize_lanes(values: &[f32; LANES]) -> [u8; LANES] {
    let mut bytes = [0u8; LANES];
    _mm_storeu_si128(
        bytes.as_mut_ptr().cast(),
        quantize_packet(_mm512_loadu_ps(values.as_ptr())),
    );
    bytes
}

#[test]
fn quantize_matches_the_scalar_rounding() {
    if !simd_available() {
        return;
    }
    for chunk in rounding_boundaries().chunks(LANES) {
        let mut lanes = [0.0f32; LANES];
        lanes[..chunk.len()].copy_from_slice(chunk);
        // SAFETY: the required target features were detected above.
        let bytes = unsafe { quantize_lanes(&lanes) };
        for (value, byte) in chunk.iter().zip(bytes) {
            assert_eq!(byte, super::super::quantize(*value), "value {value:e}");
        }
    }
}

#[test]
fn rgba8_writeback_blend_matches_the_scalar_source_over() {
    if !simd_available() {
        return;
    }
    let sources = [
        [0.0, 0.0, 0.0, 0.0],
        [0.25, 0.1, 0.05, 0.3],
        [0.301_960_8, 0.2, 0.1, 0.501_960_8],
        [0.9, 0.5, 0.2, 1.0],
    ];
    for source in sources {
        for first in (0..=255u16).step_by(LANES) {
            let destination: Vec<f32> = (first..first + LANES as u16)
                .map(|byte| f32::from(byte) / 255.0)
                .collect();
            let mut tile = [
                destination.clone(),
                destination.clone(),
                destination.clone(),
                destination.clone(),
            ];
            // SAFETY: the required target features were detected above.
            unsafe {
                blend_packet(
                    &mut tile,
                    0,
                    first_n_mask(LANES),
                    source.map(|channel| _mm512_set1_ps(channel)),
                    SdfAccumulationMode::Rgba8Writeback,
                );
            }
            for (lane, value) in destination.iter().enumerate() {
                let mut expected = [*value; 4];
                super::super::source_over_rgba8(&mut expected, source);
                for channel in 0..4 {
                    assert_eq!(
                        tile[channel][lane].to_bits(),
                        expected[channel].to_bits(),
                        "destination {value} source {source:?} channel {channel}"
                    );
                }
            }
        }
    }
}

#[test]
fn existing_rgba8_loads_match_the_scalar_conversion() {
    if !simd_available() {
        return;
    }
    let output: Vec<u8> = (0..=255u8).flat_map(|byte| [byte; 4]).collect();
    let mut tile = [
        vec![0.0; 256],
        vec![0.0; 256],
        vec![0.0; 256],
        vec![0.0; 256],
    ];
    // SAFETY: the required target features were detected above.
    unsafe { load_existing_tile_rgba8(&mut tile, 256, 256, 1, 0, 0, 256, &output) };
    for byte in 0..=255u8 {
        let expected = f32::from(byte) / 255.0;
        for channel in &tile {
            assert_eq!(channel[usize::from(byte)].to_bits(), expected.to_bits());
        }
    }
}

#[test]
fn direct_output_blend_matches_the_scalar_over_path() {
    if !simd_available() {
        return;
    }
    let source = [0.301_960_8, 0.2, 0.1, 0.501_960_8];
    let mut output: Vec<u8> = (0..=255u8).flat_map(|byte| [byte; 4]).collect();
    let expected: Vec<u8> = output
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            let destination = f32::from(*byte) / 255.0;
            let channel = index % 4;
            super::super::quantize(destination.mul_add(1.0 - source[3], source[channel]))
        })
        .collect();
    // SAFETY: the required target features were detected above.
    unsafe { blend_output_constant_span_rgba8(&mut output, 0, 256, source) };
    assert_eq!(output, expected);
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi,fma")]
unsafe fn shape_coverage_lanes(
    distance: &[f32; LANES],
    gate: &[f32; LANES],
    offset: f32,
    scale: f32,
) -> [f32; LANES] {
    let mut coverage = [0.0f32; LANES];
    _mm512_storeu_ps(
        coverage.as_mut_ptr(),
        shape_coverage(
            _mm512_loadu_ps(distance.as_ptr()),
            _mm512_loadu_ps(gate.as_ptr()),
            offset,
            scale,
        ),
    );
    coverage
}

#[test]
fn shape_coverage_matches_the_scalar_texel_coverage() {
    if !simd_available() {
        return;
    }
    let sharpness = 1.5f32;
    for threshold in [0.5f32, 0.5475, 0.595, 0.7375, 0.2875] {
        let (offset, scale) = crate::sdf::shape::coverage_terms(threshold, sharpness);
        for gate in [255u8, 200, 128, 3, 1] {
            for first in (0..=255u16).step_by(LANES) {
                let distance: [f32; LANES] =
                    std::array::from_fn(|lane| f32::from(first + lane as u16));
                let gates = [f32::from(gate); LANES];
                // SAFETY: the required target features were detected above.
                let coverage = unsafe { shape_coverage_lanes(&distance, &gates, offset, scale) };
                for (lane, actual) in coverage.iter().enumerate() {
                    let texel = crate::sdf::shape::ShapeSdfTexel {
                        distance: (first + lane as u16) as u8,
                        gate,
                    };
                    let expected = crate::sdf::shape::texel_coverage(texel, threshold, sharpness);
                    assert_eq!(
                        actual.to_bits(),
                        expected.to_bits(),
                        "threshold {threshold} texel {texel:?}"
                    );
                }
            }
        }
    }
}

#[target_feature(enable = "avx512f,avx512bw,avx512vbmi,fma")]
unsafe fn nearest_lanes(values: &[f32; LANES]) -> [i32; LANES] {
    let mut indices = [0i32; LANES];
    _mm512_storeu_si512(
        indices.as_mut_ptr().cast(),
        nearest_texel(_mm512_loadu_ps(values.as_ptr())),
    );
    indices
}

#[test]
fn nearest_texel_matches_the_scalar_half_texel_rule() {
    if !simd_available() {
        return;
    }
    let mut values = [0.0f32; LANES];
    for (lane, value) in values.iter_mut().enumerate() {
        let centre = lane as f32 + 0.5;
        *value = match lane % 3 {
            0 => f32::from_bits(centre.to_bits() - 1),
            1 => centre,
            _ => f32::from_bits(centre.to_bits() + 1),
        };
    }
    values[0] = 0.499_999_97;
    // SAFETY: the required target features were detected above.
    let indices = unsafe { nearest_lanes(&values) };
    for (value, index) in values.iter().zip(indices) {
        let floor = value.floor();
        let expected = floor as i32 + i32::from(value - floor >= 0.5);
        assert_eq!(index, expected, "coordinate {value:e}");
    }
}

#[test]
fn packed_texel_reads_valid_text_and_shape_payload_tails() {
    let payload = [10, 20, 30, 40, 50];
    assert_eq!(read_packed_texel(&payload, 4, 1), Ok(50));
    assert_eq!(read_packed_texel(&payload, 3, 2), Ok(40 | (50 << 8)));
    assert!(read_packed_texel(&payload, 4, 2).is_err());
    assert!(read_packed_texel(&payload, -1, 1).is_err());
}
