//! RGBA to limited-range YUV 4:2:0 conversion, per frame and per macroblock, with
//! AVX-512 kernels.

use super::mp4::{H264EncoderConfig, H264YuvConverter};
use crate::animation::AnimationError;

#[cfg(test)]
pub(in crate::animation) fn rgba_to_yuv420p(
    rgba: &[u8],
    width: u32,
    height: u32,
    config: &H264EncoderConfig,
) -> Result<Vec<u8>, AnimationError> {
    let mut yuv = Vec::new();
    rgba_to_yuv420p_into(rgba, width, height, config, &mut yuv)?;
    Ok(yuv)
}

pub(in crate::animation) fn rgba_to_yuv420p_into(
    rgba: &[u8],
    width: u32,
    height: u32,
    config: &H264EncoderConfig,
    yuv: &mut Vec<u8>,
) -> Result<(), AnimationError> {
    if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err(AnimationError::Internal(
            "YUV420 conversion requires positive even dimensions",
        ));
    }
    let pixels = width as usize * height as usize;
    if rgba.len() != pixels * 4 {
        return Err(AnimationError::Internal(
            "YUV420 conversion received a truncated RGBA frame",
        ));
    }
    yuv.resize(pixels + pixels / 2, 0);
    config.validate()?;
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
        && config.yuv_converter != H264YuvConverter::Scalar
    {
        // SAFETY: runtime feature detection satisfies the target-feature
        // contract and the output size/dimensions were checked above.
        unsafe { rgba_to_yuv420p_avx512(rgba, width as usize, height as usize, yuv) };
        if config.validate_avx512 {
            let mut scalar = vec![0u8; yuv.len()];
            rgba_to_yuv420p_scalar(rgba, width as usize, height as usize, &mut scalar);
            if let Some((offset, (&actual, &expected))) = yuv
                .iter()
                .zip(&scalar)
                .enumerate()
                .find(|(_, (actual, expected))| actual != expected)
            {
                return Err(AnimationError::YuvMismatch {
                    offset,
                    actual,
                    expected,
                });
            }
        }
        return Ok(());
    }
    if config.require_avx512
        || config.validate_avx512
        || config.yuv_converter == H264YuvConverter::Avx512
    {
        return Err(AnimationError::Avx512Unavailable);
    }
    rgba_to_yuv420p_scalar(rgba, width as usize, height as usize, yuv);
    Ok(())
}

pub(in crate::animation) fn rgba_to_yuv420p_macroblocks_into(
    rgba: &[u8],
    width: u32,
    height: u32,
    changed_macroblocks: &[u32],
    config: &H264EncoderConfig,
    yuv: &mut [u8],
) -> Result<(), AnimationError> {
    if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err(AnimationError::Internal(
            "YUV420 macroblock conversion requires positive even dimensions",
        ));
    }
    let pixels = width as usize * height as usize;
    if rgba.len() != pixels * 4 || yuv.len() != pixels + pixels / 2 {
        return Err(AnimationError::Internal(
            "YUV420 macroblock conversion requires an initialized frame",
        ));
    }
    config.validate()?;
    let mb_count = width.div_ceil(16) as usize * height.div_ceil(16) as usize;
    if changed_macroblocks
        .iter()
        .any(|&index| index as usize >= mb_count)
    {
        return Err(AnimationError::Internal(
            "YUV420 changed macroblock index is out of range",
        ));
    }
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
        && config.yuv_converter != H264YuvConverter::Scalar
    {
        let scalar_oracle = config.validate_avx512.then(|| yuv.to_vec());
        // SAFETY: runtime feature detection and the bounds checks above satisfy
        // the target-feature and memory contracts.
        unsafe {
            rgba_to_yuv420p_macroblocks_avx512(
                rgba,
                width as usize,
                height as usize,
                changed_macroblocks,
                yuv,
            )
        };
        if let Some(mut scalar) = scalar_oracle {
            rgba_to_yuv420p_macroblocks_scalar(
                rgba,
                width as usize,
                height as usize,
                changed_macroblocks,
                &mut scalar,
            );
            if let Some((offset, (&actual, &expected))) = yuv
                .iter()
                .zip(&scalar)
                .enumerate()
                .find(|(_, (actual, expected))| actual != expected)
            {
                return Err(AnimationError::YuvMismatch {
                    offset,
                    actual,
                    expected,
                });
            }
        }
        return Ok(());
    }
    if config.require_avx512
        || config.validate_avx512
        || config.yuv_converter == H264YuvConverter::Avx512
    {
        return Err(AnimationError::Avx512Unavailable);
    }
    rgba_to_yuv420p_macroblocks_scalar(
        rgba,
        width as usize,
        height as usize,
        changed_macroblocks,
        yuv,
    );
    Ok(())
}

fn rgba_to_yuv420p_macroblocks_scalar(
    rgba: &[u8],
    width: usize,
    height: usize,
    changed_macroblocks: &[u32],
    yuv: &mut [u8],
) {
    let mb_width = width.div_ceil(16);
    for &index in changed_macroblocks {
        let index = index as usize;
        let x = index % mb_width * 16;
        let y = index / mb_width * 16;
        let block = MacroblockRect {
            x,
            y,
            width: (width - x).min(16),
            height: (height - y).min(16),
        };
        rgba_to_yuv420p_block_scalar(rgba, width, block, yuv);
    }
}

/// The pixels of a frame one macroblock covers: 16x16, clipped at the right
/// and bottom edges of the frame.
#[derive(Clone, Copy)]
struct MacroblockRect {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

/// Converts the pixels `block` covers of a `width`-pixel-wide RGBA frame into
/// an existing yuv420p buffer whose plane split follows from `yuv.len()`.
fn rgba_to_yuv420p_block_scalar(rgba: &[u8], width: usize, block: MacroblockRect, yuv: &mut [u8]) {
    let MacroblockRect {
        x: start_x,
        y: start_y,
        width: block_width,
        height: block_height,
    } = block;
    let pixels = yuv.len() * 2 / 3;
    let (y_plane, chroma) = yuv.split_at_mut(pixels);
    let (u_plane, v_plane) = chroma.split_at_mut(pixels / 4);
    for y in (start_y..start_y + block_height).step_by(2) {
        for x in (start_x..start_x + block_width).step_by(2) {
            let mut r_sum = 0i32;
            let mut g_sum = 0i32;
            let mut b_sum = 0i32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let source = ((y + dy) * width + x + dx) * 4;
                    let r = rgba[source] as i32;
                    let g = rgba[source + 1] as i32;
                    let b = rgba[source + 2] as i32;
                    y_plane[(y + dy) * width + x + dx] = rgb_to_limited_y(r, g, b);
                    r_sum += r;
                    g_sum += g;
                    b_sum += b;
                }
            }
            let chroma_index = (y / 2) * (width / 2) + x / 2;
            u_plane[chroma_index] =
                rgb_to_limited_u((r_sum + 2) >> 2, (g_sum + 2) >> 2, (b_sum + 2) >> 2);
            v_plane[chroma_index] =
                rgb_to_limited_v((r_sum + 2) >> 2, (g_sum + 2) >> 2, (b_sum + 2) >> 2);
        }
    }
}

/// Converts only the listed macroblocks of one RGBA frame into an existing
/// yuv420p buffer.
///
/// # Safety
///
/// The caller must verify `avx512f` and `avx512bw` support, since the body
/// executes AVX-512 instructions unconditionally.
///
/// `width` and `height` must be non-zero and even, with `width * height * 4`
/// free of overflow; each block is converted two rows at a time.
///
/// `rgba` must be valid for `width * height * 4` bytes, and `yuv.len()` must
/// equal `width * height * 3 / 2`. The vector path writes the Y plane at
/// offset 0 and the U and V planes from `width * height` and
/// `width * height * 5 / 4` through raw pointers, while partial-width blocks
/// go to `rgba_to_yuv420p_block_scalar`, which derives the plane split from
/// `yuv.len()`; the two layouts agree only at that exact length.
///
/// Every index in `changed_macroblocks` must be below
/// `width.div_ceil(16) * height.div_ceil(16)`. A larger index places the
/// block's first row at or past `height`, and the unchecked row loads and
/// stores would then leave both buffers.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn rgba_to_yuv420p_macroblocks_avx512(
    rgba: &[u8],
    width: usize,
    height: usize,
    changed_macroblocks: &[u32],
    yuv: &mut [u8],
) {
    use std::arch::x86_64::*;
    let pixels = width * height;
    let y_plane = yuv.as_mut_ptr();
    let u_plane = y_plane.add(pixels);
    let v_plane = u_plane.add(pixels / 4);
    let even = _mm512_setr_epi32(0, 2, 4, 6, 8, 10, 12, 14, 0, 0, 0, 0, 0, 0, 0, 0);
    let odd = _mm512_setr_epi32(1, 3, 5, 7, 9, 11, 13, 15, 0, 0, 0, 0, 0, 0, 0, 0);
    let mb_width = width.div_ceil(16);
    for &index in changed_macroblocks {
        let index = index as usize;
        let start_x = index % mb_width * 16;
        let start_y = index / mb_width * 16;
        let block_width = (width - start_x).min(16);
        let block_height = (height - start_y).min(16);
        if block_width != 16 {
            let block = MacroblockRect {
                x: start_x,
                y: start_y,
                width: block_width,
                height: block_height,
            };
            rgba_to_yuv420p_block_scalar(rgba, width, block, yuv);
            continue;
        }
        for y in (start_y..start_y + block_height).step_by(2) {
            let top = _mm512_loadu_si512(rgba.as_ptr().add((y * width + start_x) * 4).cast());
            let bottom =
                _mm512_loadu_si512(rgba.as_ptr().add(((y + 1) * width + start_x) * 4).cast());
            let (top_r, top_g, top_b) = avx512_rgb_channels(top);
            let (bottom_r, bottom_g, bottom_b) = avx512_rgb_channels(bottom);
            _mm_storeu_si128(
                y_plane.add(y * width + start_x).cast(),
                avx512_limited_y(top_r, top_g, top_b),
            );
            _mm_storeu_si128(
                y_plane.add((y + 1) * width + start_x).cast(),
                avx512_limited_y(bottom_r, bottom_g, bottom_b),
            );
            let r = avx512_pair_average(top_r, bottom_r, even, odd);
            let g = avx512_pair_average(top_g, bottom_g, even, odd);
            let b = avx512_pair_average(top_b, bottom_b, even, odd);
            let chroma = (y / 2) * (width / 2) + start_x / 2;
            _mm_storel_epi64(u_plane.add(chroma).cast(), avx512_limited_u(r, g, b));
            _mm_storel_epi64(v_plane.add(chroma).cast(), avx512_limited_v(r, g, b));
        }
    }
}

pub(in crate::animation) fn rgba_to_yuv420p_scalar(
    rgba: &[u8],
    width: usize,
    height: usize,
    yuv: &mut [u8],
) {
    let pixels = width * height;
    let (y_plane, chroma) = yuv.split_at_mut(pixels);
    let (u_plane, v_plane) = chroma.split_at_mut(pixels / 4);
    for y in (0..height).step_by(2) {
        for x in (0..width).step_by(2) {
            let mut r_sum = 0i32;
            let mut g_sum = 0i32;
            let mut b_sum = 0i32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let source = ((y + dy) * width + x + dx) * 4;
                    let r = rgba[source] as i32;
                    let g = rgba[source + 1] as i32;
                    let b = rgba[source + 2] as i32;
                    y_plane[(y + dy) * width + x + dx] = rgb_to_limited_y(r, g, b);
                    r_sum += r;
                    g_sum += g;
                    b_sum += b;
                }
            }
            let r = (r_sum + 2) >> 2;
            let g = (g_sum + 2) >> 2;
            let b = (b_sum + 2) >> 2;
            let chroma_index = (y / 2) * (width / 2) + x / 2;
            u_plane[chroma_index] = rgb_to_limited_u(r, g, b);
            v_plane[chroma_index] = rgb_to_limited_v(r, g, b);
        }
    }
}

fn rgb_to_limited_y(r: i32, g: i32, b: i32) -> u8 {
    (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16).clamp(0, 255) as u8
}

fn rgb_to_limited_u(r: i32, g: i32, b: i32) -> u8 {
    (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255) as u8
}

fn rgb_to_limited_v(r: i32, g: i32, b: i32) -> u8 {
    (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255) as u8
}

/// Converts one whole RGBA frame to limited-range yuv420p.
///
/// # Safety
///
/// The caller must verify `avx512f` and `avx512bw` support, since the body
/// executes AVX-512 instructions unconditionally.
///
/// `width` and `height` must be non-zero and even, with `width * height * 4`
/// free of overflow: rows are converted in pairs and the scalar tail reads
/// pixel pairs, so an odd dimension would read one row or column past the
/// frame.
///
/// `rgba` must be valid for `width * height * 4` bytes. The vector path loads
/// 32 pixels at a time from each row pair while `x < width & !31`, and the
/// scalar tail reads the remaining columns.
///
/// `yuv` must be valid for `width * height * 3 / 2` bytes, laid out as I420:
/// Y at offset 0, U at `width * height`, V at `width * height * 5 / 4`, rows
/// tightly packed. The body writes 16 luma bytes at `y * width + x` and 16
/// chroma bytes at `(y / 2) * (width / 2) + x / 2` in each plane through raw
/// pointers, with no bounds check.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
#[inline(never)]
pub(in crate::animation) unsafe fn rgba_to_yuv420p_avx512(
    rgba: &[u8],
    width: usize,
    height: usize,
    yuv: &mut [u8],
) {
    use std::arch::x86_64::*;

    let pixels = width * height;
    let y_plane = yuv.as_mut_ptr();
    let u_plane = y_plane.add(pixels);
    let v_plane = u_plane.add(pixels / 4);
    let even = _mm512_setr_epi32(0, 2, 4, 6, 8, 10, 12, 14, 0, 0, 0, 0, 0, 0, 0, 0);
    let odd = _mm512_setr_epi32(1, 3, 5, 7, 9, 11, 13, 15, 0, 0, 0, 0, 0, 0, 0, 0);
    // Thirty-two RGBA pixels produce sixteen 2x2 chroma samples. Keeping two
    // adjacent 16-pixel blocks together therefore gives both luma and chroma
    // arithmetic a genuinely full 512-bit, sixteen-i32-lane batch.
    let simd_width = width & !31;
    for y in (0..height).step_by(2) {
        let top = rgba.as_ptr().add(y * width * 4);
        let bottom = top.add(width * 4);
        let mut x = 0usize;
        while x < simd_width {
            let top_pixels0 = _mm512_loadu_si512(top.add(x * 4).cast());
            let top_pixels1 = _mm512_loadu_si512(top.add((x + 16) * 4).cast());
            let bottom_pixels0 = _mm512_loadu_si512(bottom.add(x * 4).cast());
            let bottom_pixels1 = _mm512_loadu_si512(bottom.add((x + 16) * 4).cast());
            let (top_r0, top_g0, top_b0) = avx512_rgb_channels(top_pixels0);
            let (top_r1, top_g1, top_b1) = avx512_rgb_channels(top_pixels1);
            let (bottom_r0, bottom_g0, bottom_b0) = avx512_rgb_channels(bottom_pixels0);
            let (bottom_r1, bottom_g1, bottom_b1) = avx512_rgb_channels(bottom_pixels1);

            _mm_storeu_si128(
                y_plane.add(y * width + x).cast(),
                avx512_limited_y(top_r0, top_g0, top_b0),
            );
            _mm_storeu_si128(
                y_plane.add(y * width + x + 16).cast(),
                avx512_limited_y(top_r1, top_g1, top_b1),
            );
            _mm_storeu_si128(
                y_plane.add((y + 1) * width + x).cast(),
                avx512_limited_y(bottom_r0, bottom_g0, bottom_b0),
            );
            _mm_storeu_si128(
                y_plane.add((y + 1) * width + x + 16).cast(),
                avx512_limited_y(bottom_r1, bottom_g1, bottom_b1),
            );

            let r0 = avx512_pair_average(top_r0, bottom_r0, even, odd);
            let r1 = avx512_pair_average(top_r1, bottom_r1, even, odd);
            let g0 = avx512_pair_average(top_g0, bottom_g0, even, odd);
            let g1 = avx512_pair_average(top_g1, bottom_g1, even, odd);
            let b0 = avx512_pair_average(top_b0, bottom_b0, even, odd);
            let b1 = avx512_pair_average(top_b1, bottom_b1, even, odd);
            let r = _mm512_inserti64x4::<1>(r0, _mm512_castsi512_si256(r1));
            let g = _mm512_inserti64x4::<1>(g0, _mm512_castsi512_si256(g1));
            let b = _mm512_inserti64x4::<1>(b0, _mm512_castsi512_si256(b1));
            let u = avx512_limited_u(r, g, b);
            let v = avx512_limited_v(r, g, b);
            let chroma = (y / 2) * (width / 2) + x / 2;
            _mm_storeu_si128(u_plane.add(chroma).cast(), u);
            _mm_storeu_si128(v_plane.add(chroma).cast(), v);
            x += 32;
        }

        for x in (simd_width..width).step_by(2) {
            let mut r_sum = 0i32;
            let mut g_sum = 0i32;
            let mut b_sum = 0i32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let source = ((y + dy) * width + x + dx) * 4;
                    let r = rgba[source] as i32;
                    let g = rgba[source + 1] as i32;
                    let b = rgba[source + 2] as i32;
                    *y_plane.add((y + dy) * width + x + dx) = rgb_to_limited_y(r, g, b);
                    r_sum += r;
                    g_sum += g;
                    b_sum += b;
                }
            }
            let r = (r_sum + 2) >> 2;
            let g = (g_sum + 2) >> 2;
            let b = (b_sum + 2) >> 2;
            let chroma = (y / 2) * (width / 2) + x / 2;
            *u_plane.add(chroma) = rgb_to_limited_u(r, g, b);
            *v_plane.add(chroma) = rgb_to_limited_v(r, g, b);
        }
    }
}

/// Splits sixteen packed RGBA8 pixels, one per 32-bit lane, into red, green
/// and blue lanes holding `0..=255`.
///
/// # Safety
///
/// The caller must verify `avx512f` support, the feature this function is
/// compiled with; it uses `_mm512_and_si512` and `_mm512_srli_epi32` and
/// touches no memory.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
unsafe fn avx512_rgb_channels(
    pixels: std::arch::x86_64::__m512i,
) -> (
    std::arch::x86_64::__m512i,
    std::arch::x86_64::__m512i,
    std::arch::x86_64::__m512i,
) {
    use std::arch::x86_64::*;
    let mask = _mm512_set1_epi32(255);
    (
        _mm512_and_si512(pixels, mask),
        _mm512_and_si512(_mm512_srli_epi32::<8>(pixels), mask),
        _mm512_and_si512(_mm512_srli_epi32::<16>(pixels), mask),
    )
}

/// Limited-range luma for the sixteen lanes of `r`, `g` and `b`, each holding
/// `0..=255`, narrowed to sixteen bytes with unsigned saturation.
///
/// # Safety
///
/// The caller must verify `avx512f` and `avx512bw` support, the features this
/// function is compiled with; it uses `_mm512_mullo_epi32` and
/// `_mm512_cvtusepi32_epi8` and touches no memory.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn avx512_limited_y(
    r: std::arch::x86_64::__m512i,
    g: std::arch::x86_64::__m512i,
    b: std::arch::x86_64::__m512i,
) -> std::arch::x86_64::__m128i {
    use std::arch::x86_64::*;
    let value = _mm512_add_epi32(
        _mm512_srli_epi32::<8>(_mm512_add_epi32(
            _mm512_add_epi32(
                _mm512_add_epi32(
                    _mm512_mullo_epi32(r, _mm512_set1_epi32(66)),
                    _mm512_mullo_epi32(g, _mm512_set1_epi32(129)),
                ),
                _mm512_mullo_epi32(b, _mm512_set1_epi32(25)),
            ),
            _mm512_set1_epi32(128),
        )),
        _mm512_set1_epi32(16),
    );
    _mm512_cvtusepi32_epi8(value)
}

/// Averages horizontally adjacent lane pairs of `top` and `bottom`, selected
/// by the lane indices in `even` and `odd`, into eight 2x2 chroma samples in
/// the low eight lanes, rounding with `+2` before the shift by two.
///
/// # Safety
///
/// The caller must verify `avx512f` support, the feature this function is
/// compiled with; it uses `_mm512_permutexvar_epi32`, which reads only the low
/// four bits of each index lane, and touches no memory.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
unsafe fn avx512_pair_average(
    top: std::arch::x86_64::__m512i,
    bottom: std::arch::x86_64::__m512i,
    even: std::arch::x86_64::__m512i,
    odd: std::arch::x86_64::__m512i,
) -> std::arch::x86_64::__m512i {
    use std::arch::x86_64::*;
    _mm512_srli_epi32::<2>(_mm512_add_epi32(
        _mm512_add_epi32(
            _mm512_add_epi32(
                _mm512_permutexvar_epi32(even, top),
                _mm512_permutexvar_epi32(odd, top),
            ),
            _mm512_add_epi32(
                _mm512_permutexvar_epi32(even, bottom),
                _mm512_permutexvar_epi32(odd, bottom),
            ),
        ),
        _mm512_set1_epi32(2),
    ))
}

/// Limited-range Cb for the sixteen lanes of `r`, `g` and `b`, each holding
/// `0..=255`, narrowed to sixteen bytes with unsigned saturation.
///
/// # Safety
///
/// The caller must verify `avx512f` and `avx512bw` support, the features this
/// function is compiled with; it uses `_mm512_mullo_epi32`,
/// `_mm512_srai_epi32` and `_mm512_cvtusepi32_epi8` and touches no memory.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn avx512_limited_u(
    r: std::arch::x86_64::__m512i,
    g: std::arch::x86_64::__m512i,
    b: std::arch::x86_64::__m512i,
) -> std::arch::x86_64::__m128i {
    use std::arch::x86_64::*;
    let value = _mm512_add_epi32(
        _mm512_srai_epi32::<8>(_mm512_add_epi32(
            _mm512_add_epi32(
                _mm512_add_epi32(
                    _mm512_mullo_epi32(r, _mm512_set1_epi32(-38)),
                    _mm512_mullo_epi32(g, _mm512_set1_epi32(-74)),
                ),
                _mm512_mullo_epi32(b, _mm512_set1_epi32(112)),
            ),
            _mm512_set1_epi32(128),
        )),
        _mm512_set1_epi32(128),
    );
    _mm512_cvtusepi32_epi8(value)
}

/// Limited-range Cr for the sixteen lanes of `r`, `g` and `b`, each holding
/// `0..=255`, narrowed to sixteen bytes with unsigned saturation.
///
/// # Safety
///
/// The caller must verify `avx512f` and `avx512bw` support, the features this
/// function is compiled with; it uses `_mm512_mullo_epi32`,
/// `_mm512_srai_epi32` and `_mm512_cvtusepi32_epi8` and touches no memory.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn avx512_limited_v(
    r: std::arch::x86_64::__m512i,
    g: std::arch::x86_64::__m512i,
    b: std::arch::x86_64::__m512i,
) -> std::arch::x86_64::__m128i {
    use std::arch::x86_64::*;
    let value = _mm512_add_epi32(
        _mm512_srai_epi32::<8>(_mm512_add_epi32(
            _mm512_add_epi32(
                _mm512_add_epi32(
                    _mm512_mullo_epi32(r, _mm512_set1_epi32(112)),
                    _mm512_mullo_epi32(g, _mm512_set1_epi32(-94)),
                ),
                _mm512_mullo_epi32(b, _mm512_set1_epi32(-18)),
            ),
            _mm512_set1_epi32(128),
        )),
        _mm512_set1_epi32(128),
    );
    _mm512_cvtusepi32_epi8(value)
}
