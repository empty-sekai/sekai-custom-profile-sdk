//! PNG decoding and encoding, restricted to what the card render path uses.
//!
//! Supported on decode: bit depth 8, non-interlaced, colour types
//! grayscale / RGB / palette / grayscale+alpha / RGBA, with optional `tRNS`.
//! A survey of the shipped profile asset corpus found 8-bit RGBA throughout,
//! with a small number of 8-bit RGB files; the remaining 8-bit colour types are
//! accepted so caller-supplied assets do not fail for no good reason.
//!
//! Deliberately rejected (never silently approximated): 16-bit samples and
//! Adam7 interlacing.
//!
//! Encoding always emits 8-bit RGBA, non-interlaced, filter type 0.

use super::{crc32, deflate::zlib_compress, inflate::zlib_decompress, CodecError};

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// A decoded image: tightly packed, non-premultiplied 8-bit RGBA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, no padding.
    pub pixels: Vec<u8>,
}

impl RgbaImage {
    pub fn row_bytes(&self) -> usize {
        self.width as usize * 4
    }
}

/// Returns true when `data` starts with the PNG signature.
///
/// Extensions lie: the asset trees contain JPEG payloads named `.png`. Callers
/// should dispatch on content, not on file name.
pub fn is_png(data: &[u8]) -> bool {
    data.len() >= 8 && data[..8] == SIGNATURE
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorType {
    Gray,
    Rgb,
    Palette,
    GrayAlpha,
    Rgba,
}

impl ColorType {
    fn from_byte(value: u8) -> Result<Self, CodecError> {
        match value {
            0 => Ok(Self::Gray),
            2 => Ok(Self::Rgb),
            3 => Ok(Self::Palette),
            4 => Ok(Self::GrayAlpha),
            6 => Ok(Self::Rgba),
            _ => Err(CodecError::Format("unknown PNG colour type")),
        }
    }

    /// Bytes per pixel in the *filtered* scanline representation.
    fn channels(self) -> usize {
        match self {
            Self::Gray | Self::Palette => 1,
            Self::GrayAlpha => 2,
            Self::Rgb => 3,
            Self::Rgba => 4,
        }
    }
}

struct Header {
    width: u32,
    height: u32,
    color_type: ColorType,
}

fn be_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Decodes a PNG into non-premultiplied RGBA8.
pub fn decode(data: &[u8]) -> Result<RgbaImage, CodecError> {
    if !is_png(data) {
        return Err(CodecError::Format("missing PNG signature"));
    }

    let mut pos = 8usize;
    let mut header: Option<Header> = None;
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut trns_palette: Vec<u8> = Vec::new();
    let mut trns_gray: Option<u16> = None;
    let mut trns_rgb: Option<[u16; 3]> = None;
    let mut idat: Vec<u8> = Vec::new();

    while pos + 8 <= data.len() {
        let length = be_u32(&data[pos..pos + 4]) as usize;
        let kind = &data[pos + 4..pos + 8];
        let body_start = pos + 8;
        let body_end = body_start
            .checked_add(length)
            .ok_or(CodecError::Format("chunk length overflow"))?;
        if body_end + 4 > data.len() {
            return Err(CodecError::Format("chunk extends past end of file"));
        }
        let body = &data[body_start..body_end];
        let stored_crc = be_u32(&data[body_end..body_end + 4]);
        if crc32(&data[pos + 4..body_end]) != stored_crc {
            return Err(CodecError::Format("chunk CRC mismatch"));
        }

        match kind {
            b"IHDR" => {
                if body.len() != 13 {
                    return Err(CodecError::Format("IHDR must be 13 bytes"));
                }
                let width = be_u32(&body[0..4]);
                let height = be_u32(&body[4..8]);
                let bit_depth = body[8];
                let color_type = ColorType::from_byte(body[9])?;
                let compression = body[10];
                let filter = body[11];
                let interlace = body[12];
                if width == 0 || height == 0 {
                    return Err(CodecError::Format("zero-sized image"));
                }
                if bit_depth != 8 {
                    return Err(CodecError::Unsupported("PNG bit depth other than 8"));
                }
                if compression != 0 {
                    return Err(CodecError::Format("unknown PNG compression method"));
                }
                if filter != 0 {
                    return Err(CodecError::Format("unknown PNG filter method"));
                }
                if interlace != 0 {
                    return Err(CodecError::Unsupported("Adam7 interlaced PNG"));
                }
                header = Some(Header {
                    width,
                    height,
                    color_type,
                });
            }
            b"PLTE" => {
                if body.len() % 3 != 0 {
                    return Err(CodecError::Format("PLTE length is not a multiple of 3"));
                }
                palette = body.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
            }
            b"tRNS" => {
                let Some(h) = header.as_ref() else {
                    return Err(CodecError::Format("tRNS before IHDR"));
                };
                match h.color_type {
                    ColorType::Palette => trns_palette = body.to_vec(),
                    ColorType::Gray => {
                        if body.len() < 2 {
                            return Err(CodecError::Format("grayscale tRNS too short"));
                        }
                        trns_gray = Some(u16::from_be_bytes([body[0], body[1]]));
                    }
                    ColorType::Rgb => {
                        if body.len() < 6 {
                            return Err(CodecError::Format("RGB tRNS too short"));
                        }
                        trns_rgb = Some([
                            u16::from_be_bytes([body[0], body[1]]),
                            u16::from_be_bytes([body[2], body[3]]),
                            u16::from_be_bytes([body[4], body[5]]),
                        ]);
                    }
                    // tRNS is meaningless for colour types that already carry alpha.
                    ColorType::GrayAlpha | ColorType::Rgba => {}
                }
            }
            b"IDAT" => idat.extend_from_slice(body),
            // IEND carries no data, so it is only an early exit here. Its
            // absence is not treated as corruption: the checks below prove the
            // pixel data is complete on its own, and real encoders do ship files
            // that stop after the last frame chunk.
            b"IEND" => break,
            _ => {}
        }
        pos = body_end + 4;
    }

    let header = header.ok_or(CodecError::Format("missing IHDR"))?;
    if idat.is_empty() {
        return Err(CodecError::Format("missing IDAT"));
    }
    if header.color_type == ColorType::Palette && palette.is_empty() {
        return Err(CodecError::Format("palette image without PLTE"));
    }

    let channels = header.color_type.channels();
    let width = header.width as usize;
    let height = header.height as usize;
    let stride = width
        .checked_mul(channels)
        .ok_or(CodecError::Format("scanline width overflow"))?;
    let expected = stride
        .checked_add(1)
        .and_then(|s| s.checked_mul(height))
        .ok_or(CodecError::Format("image size overflow"))?;

    let raw = zlib_decompress(&idat, expected)?;
    if raw.len() != expected {
        return Err(CodecError::Format("IDAT size does not match IHDR"));
    }

    let unfiltered = unfilter(&raw, stride, height, channels)?;

    let mut pixels = vec![0u8; width * height * 4];
    for y in 0..height {
        let src = &unfiltered[y * stride..(y + 1) * stride];
        let dst = &mut pixels[y * width * 4..(y + 1) * width * 4];
        expand_row(
            src,
            dst,
            width,
            header.color_type,
            &palette,
            &trns_palette,
            trns_gray,
            trns_rgb,
        )?;
    }

    Ok(RgbaImage {
        width: header.width,
        height: header.height,
        pixels,
    })
}

/// Reverses the per-scanline filters in place, returning the raw sample rows
/// with the filter bytes stripped.
fn unfilter(
    raw: &[u8],
    stride: usize,
    height: usize,
    channels: usize,
) -> Result<Vec<u8>, CodecError> {
    let mut out = vec![0u8; stride * height];
    for y in 0..height {
        let filter = raw[y * (stride + 1)];
        let src = &raw[y * (stride + 1) + 1..y * (stride + 1) + 1 + stride];
        // Split so the previous row stays readable while the current row is written.
        let (done, current) = out.split_at_mut(y * stride);
        let prev = if y == 0 {
            None
        } else {
            Some(&done[(y - 1) * stride..])
        };
        let row = &mut current[..stride];
        match filter {
            0 => row.copy_from_slice(src),
            1 => {
                for i in 0..stride {
                    let left = if i >= channels { row[i - channels] } else { 0 };
                    row[i] = src[i].wrapping_add(left);
                }
            }
            2 => {
                for i in 0..stride {
                    let up = prev.map_or(0, |p| p[i]);
                    row[i] = src[i].wrapping_add(up);
                }
            }
            3 => {
                for i in 0..stride {
                    let left = if i >= channels { row[i - channels] } else { 0 };
                    let up = prev.map_or(0, |p| p[i]);
                    let avg = ((u16::from(left) + u16::from(up)) / 2) as u8;
                    row[i] = src[i].wrapping_add(avg);
                }
            }
            4 => {
                for i in 0..stride {
                    let left = if i >= channels { row[i - channels] } else { 0 };
                    let up = prev.map_or(0, |p| p[i]);
                    let up_left = if i >= channels {
                        prev.map_or(0, |p| p[i - channels])
                    } else {
                        0
                    };
                    row[i] = src[i].wrapping_add(paeth(left, up, up_left));
                }
            }
            _ => return Err(CodecError::Format("unknown scanline filter type")),
        }
    }
    Ok(out)
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let pa = (p - i16::from(a)).abs();
    let pb = (p - i16::from(b)).abs();
    let pc = (p - i16::from(c)).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[allow(clippy::too_many_arguments)]
fn expand_row(
    src: &[u8],
    dst: &mut [u8],
    width: usize,
    color_type: ColorType,
    palette: &[[u8; 3]],
    trns_palette: &[u8],
    trns_gray: Option<u16>,
    trns_rgb: Option<[u16; 3]>,
) -> Result<(), CodecError> {
    for x in 0..width {
        let out = &mut dst[x * 4..x * 4 + 4];
        match color_type {
            ColorType::Gray => {
                let g = src[x];
                let a = match trns_gray {
                    Some(key) if u16::from(g) == key => 0,
                    _ => 255,
                };
                out.copy_from_slice(&[g, g, g, a]);
            }
            ColorType::GrayAlpha => {
                let g = src[x * 2];
                out.copy_from_slice(&[g, g, g, src[x * 2 + 1]]);
            }
            ColorType::Rgb => {
                let r = src[x * 3];
                let g = src[x * 3 + 1];
                let b = src[x * 3 + 2];
                let a = match trns_rgb {
                    Some(key)
                        if u16::from(r) == key[0]
                            && u16::from(g) == key[1]
                            && u16::from(b) == key[2] =>
                    {
                        0
                    }
                    _ => 255,
                };
                out.copy_from_slice(&[r, g, b, a]);
            }
            ColorType::Rgba => {
                out.copy_from_slice(&src[x * 4..x * 4 + 4]);
            }
            ColorType::Palette => {
                let idx = usize::from(src[x]);
                let entry = palette
                    .get(idx)
                    .ok_or(CodecError::Format("palette index out of range"))?;
                let a = trns_palette.get(idx).copied().unwrap_or(255);
                out.copy_from_slice(&[entry[0], entry[1], entry[2], a]);
            }
        }
    }
    Ok(())
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Encodes non-premultiplied RGBA8 as an 8-bit RGBA, non-interlaced PNG.
/// Stores every row unfiltered.
///
/// Rendered pages carry wide flat regions and rows that repeat their predecessor
/// exactly, and DEFLATE matches those across rows at a distance of one stride.
/// Filtering rewrites each row into residuals, which can break that match — so
/// for this kind of content leaving the bytes alone is often the smaller of the
/// two, and [`encode_rgba`] tries both.
fn store_scanlines(height: u32, pixels: &[u8], stride: usize) -> Vec<u8> {
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0);
        raw.extend_from_slice(&pixels[y * stride..(y + 1) * stride]);
    }
    raw
}

/// Applies the PNG filter that leaves each row's residual closest to zero.
///
/// The choice is per row, using libpng's minimum-sum-of-absolute-differences
/// heuristic. This is a pure size decision — every filter is exactly reversible,
/// so the decoded pixels do not depend on it.
fn filter_scanlines(height: u32, pixels: &[u8], stride: usize) -> Vec<u8> {
    const BPP: usize = 4;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    let mut previous = vec![0u8; stride];
    // One scratch row per filter, reused across the image.
    let mut candidates = [
        vec![0u8; stride],
        vec![0u8; stride],
        vec![0u8; stride],
        vec![0u8; stride],
        vec![0u8; stride],
    ];
    for y in 0..height as usize {
        let row = &pixels[y * stride..(y + 1) * stride];
        for x in 0..stride {
            let current = row[x];
            let left = if x >= BPP { row[x - BPP] } else { 0 };
            let up = previous[x];
            let up_left = if x >= BPP { previous[x - BPP] } else { 0 };
            candidates[0][x] = current;
            candidates[1][x] = current.wrapping_sub(left);
            candidates[2][x] = current.wrapping_sub(up);
            candidates[3][x] = current.wrapping_sub(((u16::from(left) + u16::from(up)) / 2) as u8);
            candidates[4][x] = current.wrapping_sub(paeth(left, up, up_left));
        }
        let best = (0..candidates.len())
            .min_by_key(|index| {
                candidates[*index]
                    .iter()
                    // Residuals are read as signed, so 0xFF is a distance of 1.
                    .map(|byte| u64::from(byte.wrapping_add(128).abs_diff(128)))
                    .sum::<u64>()
            })
            .expect("five filter candidates");
        raw.push(best as u8);
        raw.extend_from_slice(&candidates[best]);
        previous.copy_from_slice(row);
    }
    raw
}

pub fn encode_rgba(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, CodecError> {
    if width == 0 || height == 0 {
        return Err(CodecError::Format("zero-sized image"));
    }
    let stride = (width as usize)
        .checked_mul(4)
        .ok_or(CodecError::Format("scanline width overflow"))?;
    let needed = stride
        .checked_mul(height as usize)
        .ok_or(CodecError::Format("image size overflow"))?;
    if pixels.len() != needed {
        return Err(CodecError::Format(
            "pixel buffer length does not match size",
        ));
    }

    // Neither filtering strategy dominates: which one wins depends on how much of
    // the page repeats row to row. Compressing both and keeping the smaller costs
    // one extra DEFLATE pass and removes the guess. The two passes are
    // independent, so they run concurrently when a pool is available; the
    // selected bytes are identical either way.
    #[cfg(feature = "parallel")]
    let (stored, filtered) = rayon::join(
        || zlib_compress(&store_scanlines(height, pixels, stride)),
        || zlib_compress(&filter_scanlines(height, pixels, stride)),
    );
    #[cfg(not(feature = "parallel"))]
    let (stored, filtered) = (
        zlib_compress(&store_scanlines(height, pixels, stride)),
        zlib_compress(&filter_scanlines(height, pixels, stride)),
    );
    let (stored, filtered) = (stored?, filtered?);
    let compressed = if filtered.len() < stored.len() {
        filtered
    } else {
        stored
    };

    let mut out = Vec::with_capacity(compressed.len() + 128);
    out.extend_from_slice(&SIGNATURE);

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &compressed);
    chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32) -> Vec<u8> {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&[
                    (x % 256) as u8,
                    (y % 256) as u8,
                    ((x + y) % 256) as u8,
                    ((x * y) % 256) as u8,
                ]);
            }
        }
        pixels
    }

    #[test]
    fn encode_decode_round_trips_exactly() {
        let (w, h) = (61u32, 37u32);
        let pixels = gradient(w, h);
        let encoded = encode_rgba(w, h, &pixels).expect("encode");
        let decoded = decode(&encoded).expect("decode");
        assert_eq!(decoded.width, w);
        assert_eq!(decoded.height, h);
        assert_eq!(decoded.pixels, pixels, "round trip must be lossless");
    }

    #[test]
    fn single_pixel_round_trips() {
        let pixels = vec![1, 2, 3, 4];
        let encoded = encode_rgba(1, 1, &pixels).expect("encode");
        assert_eq!(decode(&encoded).expect("decode").pixels, pixels);
    }

    /// A horizontal ramp is cheapest under Sub; a row repeating the one above is
    /// cheapest under Up. The encoder has to pick per row rather than settling on
    /// one filter for the image.
    #[test]
    fn the_chosen_filter_follows_the_row_content() {
        let (w, h) = (64u32, 3u32);
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                // Rows 0 and 1 are the same ramp; row 2 is flat.
                let value = if y < 2 { (x * 3) as u8 } else { 200 };
                pixels.extend_from_slice(&[value, value, value, 255]);
            }
        }
        let stride = (w * 4) as usize;
        let raw = filter_scanlines(h, &pixels, stride);
        let filters: Vec<u8> = (0..h as usize).map(|y| raw[y * (stride + 1)]).collect();
        assert_eq!(filters[0], 1, "a horizontal ramp is cheapest under Sub");
        assert_eq!(
            filters[1], 2,
            "repeating the row above is cheapest under Up"
        );
        assert_ne!(filters[1], filters[2], "the choice must follow the content");
        assert_eq!(
            decode(&encode_rgba(w, h, &pixels).expect("encode"))
                .expect("decode")
                .pixels,
            pixels
        );
    }

    /// The encoder commits to whichever strategy compresses smaller, so its IDAT
    /// can never be larger than either one alone.
    #[test]
    fn the_encoder_keeps_the_smaller_of_the_two_strategies() {
        let stride_of = |w: u32| (w * 4) as usize;
        // A gradient favours filtering; rows that repeat favour storing.
        let mut repeating = Vec::new();
        for y in 0..64u32 {
            for x in 0..64u32 {
                let value = ((x * 7 + (y / 16) * 31) % 256) as u8;
                repeating.extend_from_slice(&[value, value.wrapping_mul(3), value, 255]);
            }
        }
        for (w, h, pixels) in [(256u32, 256u32, gradient(256, 256)), (64, 64, repeating)] {
            let stored = zlib_compress(&store_scanlines(h, &pixels, stride_of(w)))
                .expect("compress")
                .len();
            let filtered = zlib_compress(&filter_scanlines(h, &pixels, stride_of(w)))
                .expect("compress")
                .len();
            let encoded = encode_rgba(w, h, &pixels).expect("encode");
            // Signature (8) + IHDR chunk (25) + IDAT overhead (12) + IEND (12).
            assert_eq!(encoded.len(), stored.min(filtered) + 57);
            assert_eq!(decode(&encoded).expect("decode").pixels, pixels);
        }
    }

    #[test]
    fn large_image_round_trips() {
        let (w, h) = (512u32, 256u32);
        let pixels = gradient(w, h);
        let encoded = encode_rgba(w, h, &pixels).expect("encode");
        assert_eq!(decode(&encoded).expect("decode").pixels, pixels);
    }

    #[test]
    fn is_png_rejects_jpeg_payloads() {
        // The static asset tree contains JPEG files named `.png`.
        assert!(!is_png(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F']));
        assert!(is_png(&SIGNATURE));
    }

    #[test]
    fn corrupt_crc_is_rejected() {
        let mut encoded = encode_rgba(2, 2, &gradient(2, 2)).expect("encode");
        // Flip a byte inside the IHDR body; the CRC must catch it.
        encoded[20] ^= 0xFF;
        assert!(matches!(decode(&encoded), Err(CodecError::Format(_))));
    }

    #[test]
    fn mismatched_pixel_buffer_is_rejected() {
        assert!(encode_rgba(4, 4, &[0u8; 10]).is_err());
    }

    #[test]
    fn missing_signature_is_rejected() {
        assert!(decode(b"not a png at all").is_err());
    }

    #[test]
    fn sixteen_bit_depth_is_reported_as_unsupported() {
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&2u32.to_be_bytes());
        ihdr.extend_from_slice(&2u32.to_be_bytes());
        ihdr.extend_from_slice(&[16, 6, 0, 0, 0]);
        let mut data = Vec::new();
        data.extend_from_slice(&SIGNATURE);
        chunk(&mut data, b"IHDR", &ihdr);
        chunk(&mut data, b"IEND", &[]);
        assert!(matches!(decode(&data), Err(CodecError::Unsupported(_))));
    }

    #[test]
    fn interlaced_png_is_reported_as_unsupported() {
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&2u32.to_be_bytes());
        ihdr.extend_from_slice(&2u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 1]);
        let mut data = Vec::new();
        data.extend_from_slice(&SIGNATURE);
        chunk(&mut data, b"IHDR", &ihdr);
        chunk(&mut data, b"IEND", &[]);
        assert!(matches!(decode(&data), Err(CodecError::Unsupported(_))));
    }

    #[test]
    fn truncated_idat_is_rejected() {
        let mut encoded = encode_rgba(8, 8, &gradient(8, 8)).expect("encode");
        encoded.truncate(encoded.len() / 2);
        assert!(decode(&encoded).is_err());
    }

    #[test]
    fn every_filter_type_reverses_correctly() {
        // Build a 4-row RGBA image and hand-encode each row with a different
        // filter, then check the decoder reproduces the original samples.
        let (w, h, channels) = (4usize, 4usize, 4usize);
        let stride = w * channels;
        let original: Vec<u8> = (0..stride * h).map(|i| (i * 7 % 251) as u8).collect();

        let mut raw = Vec::new();
        for y in 0..h {
            let filter = y as u8; // 0..=3
            raw.push(filter);
            let row = &original[y * stride..(y + 1) * stride];
            let prev = if y == 0 {
                vec![0u8; stride]
            } else {
                original[(y - 1) * stride..y * stride].to_vec()
            };
            for i in 0..stride {
                let left = if i >= channels {
                    original[y * stride + i - channels]
                } else {
                    0
                };
                let up = prev[i];
                let encoded = match filter {
                    0 => row[i],
                    1 => row[i].wrapping_sub(left),
                    2 => row[i].wrapping_sub(up),
                    3 => row[i].wrapping_sub(((u16::from(left) + u16::from(up)) / 2) as u8),
                    _ => unreachable!(),
                };
                raw.push(encoded);
            }
        }

        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&(w as u32).to_be_bytes());
        ihdr.extend_from_slice(&(h as u32).to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut data = Vec::new();
        data.extend_from_slice(&SIGNATURE);
        chunk(&mut data, b"IHDR", &ihdr);
        chunk(&mut data, b"IDAT", &zlib_compress(&raw).expect("compress"));
        chunk(&mut data, b"IEND", &[]);

        assert_eq!(decode(&data).expect("decode").pixels, original);
    }

    #[test]
    fn paeth_filter_reverses_correctly() {
        let (w, h, channels) = (5usize, 3usize, 4usize);
        let stride = w * channels;
        let original: Vec<u8> = (0..stride * h).map(|i| (i * 13 % 239) as u8).collect();
        let mut raw = Vec::new();
        for y in 0..h {
            raw.push(4);
            for i in 0..stride {
                let left = if i >= channels {
                    original[y * stride + i - channels]
                } else {
                    0
                };
                let up = if y == 0 {
                    0
                } else {
                    original[(y - 1) * stride + i]
                };
                let up_left = if y == 0 || i < channels {
                    0
                } else {
                    original[(y - 1) * stride + i - channels]
                };
                raw.push(original[y * stride + i].wrapping_sub(paeth(left, up, up_left)));
            }
        }
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&(w as u32).to_be_bytes());
        ihdr.extend_from_slice(&(h as u32).to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut data = Vec::new();
        data.extend_from_slice(&SIGNATURE);
        chunk(&mut data, b"IHDR", &ihdr);
        chunk(&mut data, b"IDAT", &zlib_compress(&raw).expect("compress"));
        chunk(&mut data, b"IEND", &[]);
        assert_eq!(decode(&data).expect("decode").pixels, original);
    }
}
