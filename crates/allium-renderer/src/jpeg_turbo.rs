use std::ffi::{c_char, c_int, c_ulong, c_void, CStr};

unsafe extern "C" {
    fn allium_jpeg_encode_rgba(
        rgba: *const u8,
        width: u32,
        height: u32,
        quality: c_int,
        output: *mut *mut u8,
        output_length: *mut c_ulong,
        error_message: *mut c_char,
        error_capacity: usize,
    ) -> c_int;
    fn allium_jpeg_encode_yuv420(
        y_plane: *const u8,
        cb_plane: *const u8,
        cr_plane: *const u8,
        width: u32,
        height: u32,
        y_stride: u32,
        chroma_stride: u32,
        quality: c_int,
        output: *mut *mut u8,
        output_length: *mut c_ulong,
        error_message: *mut c_char,
        error_capacity: usize,
    ) -> c_int;
    fn allium_jpeg_free(output: *mut c_void);
}

pub fn encode_rgba(rgba: &[u8], width: u32, height: u32, quality: u32) -> Result<Vec<u8>, String> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "JPEG RGBA dimensions overflow".to_string())?;
    if rgba.len() != expected {
        return Err(format!(
            "JPEG RGBA length mismatch: expected {expected}, got {}",
            rgba.len()
        ));
    }
    let quality = i32::try_from(quality)
        .ok()
        .filter(|quality| (1..=100).contains(quality))
        .ok_or_else(|| format!("invalid JPEG quality {quality}"))?;
    let mut output = std::ptr::null_mut();
    let mut output_length: c_ulong = 0;
    let mut error = [0 as c_char; 256];
    let status = unsafe {
        allium_jpeg_encode_rgba(
            rgba.as_ptr(),
            width,
            height,
            quality,
            &mut output,
            &mut output_length,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    if status != 0 {
        let detail = unsafe { CStr::from_ptr(error.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        return Err(if detail.is_empty() {
            format!("libjpeg-turbo encode failed with status {status}")
        } else {
            format!("libjpeg-turbo encode failed: {detail}")
        });
    }
    if output.is_null() {
        return Err("libjpeg-turbo returned a null output".into());
    }
    let length = match usize::try_from(output_length) {
        Ok(length) => length,
        Err(_) => {
            unsafe { allium_jpeg_free(output.cast()) };
            return Err("libjpeg-turbo output length overflow".into());
        }
    };
    let encoded = unsafe { std::slice::from_raw_parts(output, length) }.to_vec();
    unsafe { allium_jpeg_free(output.cast()) };
    Ok(encoded)
}

pub fn encode_rgba_avx512_yuv420(
    rgba: &[u8],
    width: u32,
    height: u32,
    quality: u32,
) -> Result<Vec<u8>, String> {
    let mut scratch = Vec::new();
    encode_rgba_avx512_yuv420_with_scratch(rgba, width, height, quality, &mut scratch)
}

pub fn encode_rgba_avx512_yuv420_with_scratch(
    rgba: &[u8],
    width: u32,
    height: u32,
    quality: u32,
    scratch: &mut Vec<u8>,
) -> Result<Vec<u8>, String> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "JPEG RGBA dimensions overflow".to_string())?;
    if rgba.len() != expected {
        return Err(format!(
            "JPEG RGBA length mismatch: expected {expected}, got {}",
            rgba.len()
        ));
    }
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err(format!(
            "AVX-512 JPEG YUV420 requires positive even dimensions, got {width}x{height}"
        ));
    }
    let quality = i32::try_from(quality)
        .ok()
        .filter(|quality| (1..=100).contains(quality))
        .ok_or_else(|| format!("invalid JPEG quality {quality}"))?;
    #[cfg(target_arch = "x86_64")]
    if !(std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw"))
    {
        return Err("AVX-512 JPEG YUV420 is unavailable".into());
    }
    #[cfg(not(target_arch = "x86_64"))]
    return Err("AVX-512 JPEG YUV420 is unavailable".into());

    #[cfg(target_arch = "x86_64")]
    let yuv =
        unsafe { rgba_to_full_range_yuv420_avx512(rgba, width as usize, height as usize, scratch) };
    let mut output = std::ptr::null_mut();
    let mut output_length: c_ulong = 0;
    let mut error = [0 as c_char; 256];
    let status = unsafe {
        allium_jpeg_encode_yuv420(
            yuv.y_plane().as_ptr(),
            yuv.cb_plane().as_ptr(),
            yuv.cr_plane().as_ptr(),
            width,
            height,
            yuv.y_stride as u32,
            yuv.chroma_stride as u32,
            quality,
            &mut output,
            &mut output_length,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    if status != 0 {
        let detail = unsafe { CStr::from_ptr(error.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        return Err(if detail.is_empty() {
            format!("libjpeg-turbo raw YUV encode failed with status {status}")
        } else {
            format!("libjpeg-turbo raw YUV encode failed: {detail}")
        });
    }
    if output.is_null() {
        return Err("libjpeg-turbo returned a null raw YUV output".into());
    }
    let length = match usize::try_from(output_length) {
        Ok(length) => length,
        Err(_) => {
            unsafe { allium_jpeg_free(output.cast()) };
            return Err("libjpeg-turbo raw YUV output length overflow".into());
        }
    };
    let encoded = unsafe { std::slice::from_raw_parts(output, length) }.to_vec();
    unsafe { allium_jpeg_free(output.cast()) };
    Ok(encoded)
}

struct Yuv420Buffer<'a> {
    bytes: &'a mut [u8],
    y_stride: usize,
    chroma_stride: usize,
    y_length: usize,
    chroma_length: usize,
}

impl Yuv420Buffer<'_> {
    fn y_plane(&self) -> &[u8] {
        &self.bytes[..self.y_length]
    }

    fn cb_plane(&self) -> &[u8] {
        &self.bytes[self.y_length..self.y_length + self.chroma_length]
    }

    fn cr_plane(&self) -> &[u8] {
        &self.bytes[self.y_length + self.chroma_length..]
    }
}

pub fn yuv420_scratch_len(width: u32, height: u32) -> Result<usize, String> {
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err(format!(
            "JPEG YUV420 scratch requires positive even dimensions, got {width}x{height}"
        ));
    }
    let width = width as usize;
    let height = height as usize;
    let y_stride = width.div_ceil(16) * 16;
    let chroma_stride = (width / 2).div_ceil(8) * 8;
    y_stride
        .checked_mul(height)
        .and_then(|y_length| {
            chroma_stride
                .checked_mul(height / 2)
                .and_then(|chroma_length| y_length.checked_add(chroma_length.checked_mul(2)?))
        })
        .ok_or_else(|| "JPEG YUV420 scratch dimensions overflow".to_string())
}

fn allocate_yuv420(storage: &mut Vec<u8>, width: usize, height: usize) -> Yuv420Buffer<'_> {
    let y_stride = width.div_ceil(16) * 16;
    let chroma_stride = (width / 2).div_ceil(8) * 8;
    let y_length = y_stride * height;
    let chroma_length = chroma_stride * (height / 2);
    let total_length = y_length + chroma_length * 2;
    storage.resize(total_length, 0);
    Yuv420Buffer {
        bytes: storage.as_mut_slice(),
        y_stride,
        chroma_stride,
        y_length,
        chroma_length,
    }
}

fn full_range_y(red: i32, green: i32, blue: i32) -> u8 {
    ((77 * red + 150 * green + 29 * blue + 128) >> 8).clamp(0, 255) as u8
}

fn full_range_cb(red: i32, green: i32, blue: i32) -> u8 {
    (((-43 * red - 85 * green + 128 * blue + 128) >> 8) + 128).clamp(0, 255) as u8
}

fn full_range_cr(red: i32, green: i32, blue: i32) -> u8 {
    (((128 * red - 107 * green - 21 * blue + 128) >> 8) + 128).clamp(0, 255) as u8
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn rgba_to_full_range_yuv420_avx512<'a>(
    rgba: &[u8],
    width: usize,
    height: usize,
    storage: &'a mut Vec<u8>,
) -> Yuv420Buffer<'a> {
    use std::arch::x86_64::*;

    let output = allocate_yuv420(storage, width, height);
    let y_stride = output.y_stride;
    let chroma_stride = output.chroma_stride;
    let y_plane = output.bytes.as_mut_ptr();
    let cb_plane = y_plane.add(output.y_length);
    let cr_plane = cb_plane.add(output.chroma_length);
    let even = _mm512_setr_epi32(0, 2, 4, 6, 8, 10, 12, 14, 0, 0, 0, 0, 0, 0, 0, 0);
    let odd = _mm512_setr_epi32(1, 3, 5, 7, 9, 11, 13, 15, 0, 0, 0, 0, 0, 0, 0, 0);
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
                y_plane.add(y * y_stride + x).cast(),
                avx512_full_range_y(top_r0, top_g0, top_b0),
            );
            _mm_storeu_si128(
                y_plane.add(y * y_stride + x + 16).cast(),
                avx512_full_range_y(top_r1, top_g1, top_b1),
            );
            _mm_storeu_si128(
                y_plane.add((y + 1) * y_stride + x).cast(),
                avx512_full_range_y(bottom_r0, bottom_g0, bottom_b0),
            );
            _mm_storeu_si128(
                y_plane.add((y + 1) * y_stride + x + 16).cast(),
                avx512_full_range_y(bottom_r1, bottom_g1, bottom_b1),
            );
            let r0 = avx512_pair_average(top_r0, bottom_r0, even, odd);
            let r1 = avx512_pair_average(top_r1, bottom_r1, even, odd);
            let g0 = avx512_pair_average(top_g0, bottom_g0, even, odd);
            let g1 = avx512_pair_average(top_g1, bottom_g1, even, odd);
            let b0 = avx512_pair_average(top_b0, bottom_b0, even, odd);
            let b1 = avx512_pair_average(top_b1, bottom_b1, even, odd);
            let red = _mm512_inserti64x4::<1>(r0, _mm512_castsi512_si256(r1));
            let green = _mm512_inserti64x4::<1>(g0, _mm512_castsi512_si256(g1));
            let blue = _mm512_inserti64x4::<1>(b0, _mm512_castsi512_si256(b1));
            let chroma = (y / 2) * chroma_stride + x / 2;
            _mm_storeu_si128(
                cb_plane.add(chroma).cast(),
                avx512_full_range_cb(red, green, blue),
            );
            _mm_storeu_si128(
                cr_plane.add(chroma).cast(),
                avx512_full_range_cr(red, green, blue),
            );
            x += 32;
        }
        for x in (simd_width..width).step_by(2) {
            let mut red = 0i32;
            let mut green = 0i32;
            let mut blue = 0i32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let source = ((y + dy) * width + x + dx) * 4;
                    let r = i32::from(rgba[source]);
                    let g = i32::from(rgba[source + 1]);
                    let b = i32::from(rgba[source + 2]);
                    *y_plane.add((y + dy) * y_stride + x + dx) = full_range_y(r, g, b);
                    red += r;
                    green += g;
                    blue += b;
                }
            }
            let chroma = (y / 2) * chroma_stride + x / 2;
            *cb_plane.add(chroma) =
                full_range_cb((red + 2) >> 2, (green + 2) >> 2, (blue + 2) >> 2);
            *cr_plane.add(chroma) =
                full_range_cr((red + 2) >> 2, (green + 2) >> 2, (blue + 2) >> 2);
        }
        let y_padding = y_stride - width;
        if y_padding != 0 {
            let top_last = *y_plane.add(y * y_stride + width - 1);
            let bottom_last = *y_plane.add((y + 1) * y_stride + width - 1);
            y_plane
                .add(y * y_stride + width)
                .write_bytes(top_last, y_padding);
            y_plane
                .add((y + 1) * y_stride + width)
                .write_bytes(bottom_last, y_padding);
        }
        let chroma_width = width / 2;
        let chroma_padding = chroma_stride - chroma_width;
        if chroma_padding != 0 {
            let row = (y / 2) * chroma_stride;
            let cb_last = *cb_plane.add(row + chroma_width - 1);
            let cr_last = *cr_plane.add(row + chroma_width - 1);
            cb_plane
                .add(row + chroma_width)
                .write_bytes(cb_last, chroma_padding);
            cr_plane
                .add(row + chroma_width)
                .write_bytes(cr_last, chroma_padding);
        }
    }
    output
}

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

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn avx512_full_range_y(
    red: std::arch::x86_64::__m512i,
    green: std::arch::x86_64::__m512i,
    blue: std::arch::x86_64::__m512i,
) -> std::arch::x86_64::__m128i {
    use std::arch::x86_64::*;
    let value = _mm512_srli_epi32::<8>(_mm512_add_epi32(
        _mm512_add_epi32(
            _mm512_add_epi32(
                _mm512_mullo_epi32(red, _mm512_set1_epi32(77)),
                _mm512_mullo_epi32(green, _mm512_set1_epi32(150)),
            ),
            _mm512_mullo_epi32(blue, _mm512_set1_epi32(29)),
        ),
        _mm512_set1_epi32(128),
    ));
    _mm512_cvtusepi32_epi8(value)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn avx512_full_range_cb(
    red: std::arch::x86_64::__m512i,
    green: std::arch::x86_64::__m512i,
    blue: std::arch::x86_64::__m512i,
) -> std::arch::x86_64::__m128i {
    use std::arch::x86_64::*;
    let value = _mm512_add_epi32(
        _mm512_srai_epi32::<8>(_mm512_add_epi32(
            _mm512_add_epi32(
                _mm512_add_epi32(
                    _mm512_mullo_epi32(red, _mm512_set1_epi32(-43)),
                    _mm512_mullo_epi32(green, _mm512_set1_epi32(-85)),
                ),
                _mm512_mullo_epi32(blue, _mm512_set1_epi32(128)),
            ),
            _mm512_set1_epi32(128),
        )),
        _mm512_set1_epi32(128),
    );
    _mm512_cvtusepi32_epi8(value)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn avx512_full_range_cr(
    red: std::arch::x86_64::__m512i,
    green: std::arch::x86_64::__m512i,
    blue: std::arch::x86_64::__m512i,
) -> std::arch::x86_64::__m128i {
    use std::arch::x86_64::*;
    let value = _mm512_add_epi32(
        _mm512_srai_epi32::<8>(_mm512_add_epi32(
            _mm512_add_epi32(
                _mm512_add_epi32(
                    _mm512_mullo_epi32(red, _mm512_set1_epi32(128)),
                    _mm512_mullo_epi32(green, _mm512_set1_epi32(-107)),
                ),
                _mm512_mullo_epi32(blue, _mm512_set1_epi32(-21)),
            ),
            _mm512_set1_epi32(128),
        )),
        _mm512_set1_epi32(128),
    );
    _mm512_cvtusepi32_epi8(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_dimensions_and_quality() {
        assert!(encode_rgba(&[0, 0, 0], 1, 1, 90).is_err());
        assert!(encode_rgba(&[0, 0, 0, 255], 1, 1, 0).is_err());
    }

    #[test]
    fn emits_deterministic_jpeg() {
        let rgba = [17, 34, 51, 255, 68, 85, 102, 255];
        let first = encode_rgba(&rgba, 2, 1, 90).expect("first JPEG");
        let second = encode_rgba(&rgba, 2, 1, 90).expect("second JPEG");
        assert_eq!(first, second);
        assert!(first.starts_with(&[0xff, 0xd8]));
        assert!(first.ends_with(&[0xff, 0xd9]));
    }

    #[test]
    fn rejects_zero_sized_images_before_ffi() {
        assert!(encode_rgba(&[], 0, 1, 90).is_err());
        assert!(encode_rgba(&[], 1, 0, 90).is_err());
    }

    #[test]
    fn yuv420_scratch_layout_is_checked() {
        assert_eq!(yuv420_scratch_len(32, 16), Ok(768));
        assert!(yuv420_scratch_len(31, 16).is_err());
        assert!(yuv420_scratch_len(32, 15).is_err());
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn reused_yuv420_scratch_does_not_change_output() {
        if !(std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw"))
        {
            return;
        }
        let width = 34u32;
        let height = 18u32;
        let rgba = (0..width as usize * height as usize)
            .flat_map(|pixel| {
                [
                    pixel.wrapping_mul(17) as u8,
                    pixel.wrapping_mul(29) as u8,
                    pixel.wrapping_mul(43) as u8,
                    255,
                ]
            })
            .collect::<Vec<_>>();
        let mut scratch = Vec::new();
        let first = encode_rgba_avx512_yuv420_with_scratch(&rgba, width, height, 90, &mut scratch)
            .expect("first raw YUV420 JPEG");
        scratch.fill(0x5a);
        let second = encode_rgba_avx512_yuv420_with_scratch(&rgba, width, height, 90, &mut scratch)
            .expect("reused raw YUV420 JPEG");
        assert_eq!(first, second);
    }
}
