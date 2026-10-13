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
fn dimensions_come_from_a_header_the_decoder_accepts() {
    let encoded = encode_rgba(5, 3, &gradient(5, 3)).expect("encode");
    assert_eq!(dimensions(&encoded).expect("dimensions"), (5, 3));

    let mut corrupt = encoded.clone();
    corrupt[20] ^= 0xFF;
    assert!(matches!(dimensions(&corrupt), Err(CodecError::Format(_))));

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&2u32.to_be_bytes());
    ihdr.extend_from_slice(&2u32.to_be_bytes());
    ihdr.extend_from_slice(&[16, 6, 0, 0, 0]);
    let mut sixteen_bit = Vec::new();
    sixteen_bit.extend_from_slice(&SIGNATURE);
    chunk(&mut sixteen_bit, b"IHDR", &ihdr);
    chunk(&mut sixteen_bit, b"IEND", &[]);
    assert!(matches!(
        dimensions(&sixteen_bit),
        Err(CodecError::Unsupported(_))
    ));

    assert!(dimensions(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F']).is_err());
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

fn header(width: u32, height: u32, color_type: u8) -> Vec<u8> {
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, color_type, 0, 0, 0]);
    ihdr
}

/// A PNG made of exactly these chunks, in this order.
fn assemble(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
    let mut data = SIGNATURE.to_vec();
    for (kind, body) in chunks {
        chunk(&mut data, kind, body);
    }
    data
}

/// Image data for unfiltered scanlines.
fn scanlines(rows: &[&[u8]]) -> Vec<u8> {
    let mut raw = Vec::new();
    for row in rows {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    zlib_compress(&raw).expect("compress")
}

/// A zlib stream of at least `min_len` zero bytes: one literal, then
/// fixed-code matches of 258 bytes at distance 1, 13 bits each.
fn zero_run_stream(min_len: usize) -> Vec<u8> {
    struct Bits {
        out: Vec<u8>,
        pending: u64,
        count: u32,
    }
    impl Bits {
        fn put(&mut self, value: u32, count: u32) {
            self.pending |= u64::from(value) << self.count;
            self.count += count;
            while self.count >= 8 {
                self.out.push(self.pending as u8);
                self.pending >>= 8;
                self.count -= 8;
            }
        }
        /// Huffman codes are packed most significant bit first.
        fn code(&mut self, code: u32, count: u32) {
            for bit in (0..count).rev() {
                self.put((code >> bit) & 1, 1);
            }
        }
    }
    let mut bits = Bits {
        out: vec![0x78, 0x01],
        pending: 0,
        count: 0,
    };
    bits.put(1, 1);
    bits.put(1, 2);
    bits.code(0x30, 8);
    let matches = min_len.saturating_sub(1).div_ceil(258);
    for _ in 0..matches {
        bits.code(0xC5, 8);
        bits.code(0, 5);
    }
    bits.code(0, 7);
    if bits.count > 0 {
        bits.out.push(bits.pending as u8);
    }
    let len = 1 + matches * 258;
    let adler = (((len % 65_521) as u32) << 16) | 1;
    bits.out.extend_from_slice(&adler.to_be_bytes());
    bits.out
}

#[test]
fn the_zero_run_fixture_is_a_valid_stream() {
    use super::super::inflate::zlib_decompress;
    let stream = zero_run_stream(1000);
    assert_eq!(
        zlib_decompress(&stream, 0).expect("inflate"),
        vec![0u8; 1033]
    );
}

/// The header's size is only a claim until the image data backs it: a
/// header declaring an enormous image over a few bytes of data has to be
/// refused, not trusted with an allocation.
#[test]
fn a_header_larger_than_its_image_data_is_refused() {
    let data = assemble(&[
        (b"IHDR", &header(0x7FFF_FFFF, 0x7FFF_FFFF, 6)),
        (b"IDAT", &scanlines(&[&[0, 0, 0, 0]])),
        (b"IEND", &[]),
    ]);
    assert!(matches!(decode(&data), Err(CodecError::Format(_))));
}

/// A 1x1 image needs five bytes of image data. Data that inflates far past
/// that has to be refused once it passes the declared size, not after it
/// has all been inflated.
#[test]
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn image_data_that_inflates_past_the_header_stops_at_the_declared_size() {
    crate::codec::address_space::run_in_child("codec::png::tests::inflation_past_the_header_child");
}

#[test]
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[ignore = "runs under a capped address space in a child process"]
fn inflation_past_the_header_child() {
    use crate::codec::address_space;
    if !address_space::in_child() {
        return;
    }
    let data = assemble(&[
        (b"IHDR", &header(1, 1, 6)),
        (b"IDAT", &zero_run_stream(256 << 20)),
        (b"IEND", &[]),
    ]);
    match address_space::with_budget(64 << 20, || decode(&data)) {
        Ok(image) => panic!("decoded a {}x{} image", image.width, image.height),
        Err(error) => assert!(matches!(error, CodecError::Format(_)), "{error}"),
    }
}

#[test]
fn the_header_has_to_be_the_first_chunk() {
    let data = assemble(&[
        (b"tEXt", b"Comment\0first"),
        (b"IHDR", &header(1, 1, 6)),
        (b"IDAT", &scanlines(&[&[1, 2, 3, 4]])),
        (b"IEND", &[]),
    ]);
    assert!(matches!(decode(&data), Err(CodecError::Format(_))));
    assert!(matches!(dimensions(&data), Err(CodecError::Format(_))));
}

/// `dimensions` reports the first header, so decoding must not size the
/// image from a later one.
#[test]
fn a_second_header_is_rejected() {
    let data = assemble(&[
        (b"IHDR", &header(1, 1, 6)),
        (b"IHDR", &header(2, 1, 6)),
        (b"IDAT", &scanlines(&[&[1, 2, 3, 4, 5, 6, 7, 8]])),
        (b"IEND", &[]),
    ]);
    assert_eq!(dimensions(&data).expect("dimensions"), (1, 1));
    assert!(matches!(decode(&data), Err(CodecError::Format(_))));
}

/// Width and height are PNG four-byte integers, which stop at 2^31 - 1.
#[test]
fn dimensions_beyond_the_png_integer_range_are_rejected() {
    for (width, height) in [
        (0x8000_0000u32, 1u32),
        (1, 0x8000_0000),
        (u32::MAX, u32::MAX),
    ] {
        let data = assemble(&[(b"IHDR", &header(width, height, 6)), (b"IEND", &[])]);
        assert!(
            matches!(dimensions(&data), Err(CodecError::Format(_))),
            "{width}x{height}"
        );
        assert!(
            matches!(decode(&data), Err(CodecError::Format(_))),
            "{width}x{height}"
        );
    }
}

/// The case of a chunk type's first letter marks it ancillary (lowercase,
/// safe to skip) or critical (uppercase, needed to read the image).
#[test]
fn an_unknown_critical_chunk_stops_the_decode() {
    let pixels = [1, 2, 3, 4];
    let image_data = scanlines(&[&pixels]);
    let with = |kind: &[u8; 4]| {
        assemble(&[
            (b"IHDR", &header(1, 1, 6)),
            (kind, b"?"),
            (b"IDAT", &image_data),
            (b"IEND", &[]),
        ])
    };
    assert_eq!(
        decode(&with(b"prVt")).expect("ancillary chunk").pixels,
        pixels
    );
    assert!(matches!(
        decode(&with(b"PRVT")),
        Err(CodecError::Unsupported(_))
    ));
}

/// Some encoders leave unused bytes after the compressed image data;
/// decoders are expected to ignore them.
#[test]
fn trailing_bytes_after_the_image_data_stream_are_ignored() {
    let pixels = gradient(3, 2);
    let mut image_data = scanlines(&[&pixels[..12], &pixels[12..]]);
    image_data.extend_from_slice(&[0; 5]);
    let data = assemble(&[
        (b"IHDR", &header(3, 2, 6)),
        (b"IDAT", &image_data),
        (b"IEND", &[]),
    ]);
    assert_eq!(decode(&data).expect("decode").pixels, pixels);
}

/// A tRNS colour key is stored in two bytes per sample; at bit depth 8 only
/// the low byte applies and the rest has to be masked off before comparing.
#[test]
fn transparent_colour_keys_use_only_the_low_eight_bits() {
    let gray = assemble(&[
        (b"IHDR", &header(2, 1, 0)),
        (b"tRNS", &[0x01, 7]),
        (b"IDAT", &scanlines(&[&[7, 8]])),
        (b"IEND", &[]),
    ]);
    assert_eq!(
        decode(&gray).expect("gray").pixels,
        [7, 7, 7, 0, 8, 8, 8, 255]
    );
    let rgb = assemble(&[
        (b"IHDR", &header(2, 1, 2)),
        (b"tRNS", &[0x01, 10, 0x02, 20, 0x03, 30]),
        (b"IDAT", &scanlines(&[&[10, 20, 30, 10, 20, 31]])),
        (b"IEND", &[]),
    ]);
    assert_eq!(
        decode(&rgb).expect("rgb").pixels,
        [10, 20, 30, 0, 10, 20, 31, 255]
    );
}
