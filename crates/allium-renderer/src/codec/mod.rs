//! Dependency-free image codecs for the card render path.
//!
//! Only the formats the profile pipeline actually consumes are implemented.
//! Anything else is rejected with a specific error: a decode failure must never
//! degrade into a blank or wrong-coloured image, because the compositor cannot
//! tell those apart from authored content.

pub mod deflate;
pub mod inflate;
pub mod png;

/// Codec failure. Carries a static reason so callers can log without
/// allocating on the error path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecError {
    /// The DEFLATE/zlib stream is malformed or truncated.
    Deflate(&'static str),
    /// The container is structurally invalid.
    Format(&'static str),
    /// A structurally valid feature this decoder deliberately does not implement.
    Unsupported(&'static str),
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Deflate(reason) => write!(f, "deflate stream error: {reason}"),
            Self::Format(reason) => write!(f, "malformed image: {reason}"),
            Self::Unsupported(reason) => write!(f, "unsupported image feature: {reason}"),
        }
    }
}

impl std::error::Error for CodecError {}

/// Multiplies one colour channel by alpha, rounding to nearest.
///
/// This is the form the compositor consumes, and it is bit-identical to what a
/// Skia decode produces internally: both compute `round(value * alpha / 255)`.
#[inline]
pub fn premultiply_channel(value: u8, alpha: u8) -> u8 {
    ((u32::from(value) * u32::from(alpha) + 127) / 255) as u8
}

/// Recovers a colour channel from its premultiplied form the way Skia does.
///
/// Skia stores decoded images premultiplied, so reading one back as
/// non-premultiplied divides by alpha and loses up to 1/255 per channel. That
/// lossy value — not the file's true sample — is what the offline shape-atlas
/// builder hashed into `source_rg8_sha256`, so the runtime has to reproduce it
/// exactly or every deployed atlas fails its source check.
///
/// Skia divides in single precision and converts back with round-half-to-even,
/// so the result follows f32 representation rather than exact rational
/// arithmetic. Integer formulas and `truncate(v * 255 + 0.5)` both disagree with
/// it on hundreds of pairs, and the exact-half cases break in both directions
/// (1/6 rounds down, 3/10 rounds up). The sequence below is that pipeline in the
/// same order and precision: normalise through `1/255`, multiply by the
/// reciprocal of alpha, then round the rescaled result to nearest, ties to even.
/// `skia-unpremul-table` recorded the ground truth, and this reproduces all
/// 32 896 reachable (premultiplied, alpha) pairs; `png-parity --exhaustive`
/// rechecks it against a live Skia.
#[inline]
pub fn unpremultiply_channel_like_skia(premultiplied: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        return 0;
    }
    if alpha == 255 {
        return premultiplied;
    }
    const INV_255: f32 = 1.0 / 255.0;
    let alpha_unit = f32::from(alpha) * INV_255;
    let value_unit = f32::from(premultiplied) * INV_255;
    let recovered = (value_unit * (1.0 / alpha_unit)).clamp(0.0, 1.0);
    (recovered * 255.0).round_ties_even() as u8
}

/// Adler-32 (RFC 1950 section 9).
pub fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    // Chunk so the accumulators cannot overflow before the reduction.
    for block in data.chunks(5552) {
        for &byte in block {
            a += u32::from(byte);
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

/// CRC-32 as used by PNG chunks (ISO 3309, reflected polynomial 0xEDB88320).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adler32_matches_rfc_example() {
        // zlib's documented value for "Wikipedia".
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn adler32_of_empty_input_is_one() {
        assert_eq!(adler32(b""), 1);
    }

    #[test]
    fn crc32_matches_known_check_value() {
        // The standard CRC-32 check value for "123456789".
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn crc32_of_empty_input_is_zero() {
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn premultiply_channel_rounds_to_nearest() {
        // Exactly round(value * alpha / 255), including the .5 boundary.
        assert_eq!(premultiply_channel(255, 255), 255);
        assert_eq!(premultiply_channel(0, 255), 0);
        assert_eq!(premultiply_channel(255, 0), 0);
        for value in 0..=255u32 {
            for alpha in 0..=255u32 {
                let expected = ((value * alpha) as f64 / 255.0).round() as u8;
                assert_eq!(
                    premultiply_channel(value as u8, alpha as u8),
                    expected,
                    "value={value} alpha={alpha}"
                );
            }
        }
    }

    #[test]
    fn unpremultiply_never_exceeds_full_scale_or_inverts_order() {
        for alpha in 1..=255u32 {
            let mut previous = 0u8;
            for premultiplied in 0..=alpha {
                let recovered = unpremultiply_channel_like_skia(premultiplied as u8, alpha as u8);
                assert!(recovered >= previous, "not monotone at alpha={alpha}");
                previous = recovered;
            }
            // A channel at full coverage recovers to full scale.
            assert_eq!(
                unpremultiply_channel_like_skia(alpha as u8, alpha as u8),
                255,
                "alpha={alpha}"
            );
        }
        assert_eq!(unpremultiply_channel_like_skia(0, 0), 0);
    }

    #[test]
    fn unpremultiply_matches_recorded_float_pipeline_samples() {
        // Two pairs where an exact rational round — either half-up or half-even —
        // disagrees with what the f32 pipeline produces.
        assert_eq!(unpremultiply_channel_like_skia(1, 6), 42);
        assert_eq!(unpremultiply_channel_like_skia(3, 10), 77);
    }

    #[test]
    fn premultiply_is_the_left_inverse_of_unpremultiply() {
        // Round-tripping a valid premultiplied value must return it unchanged,
        // which is what keeps the shape-atlas source hash stable.
        for alpha in 0..=255u32 {
            for premultiplied in 0..=alpha {
                let recovered = unpremultiply_channel_like_skia(premultiplied as u8, alpha as u8);
                assert_eq!(
                    premultiply_channel(recovered, alpha as u8),
                    premultiplied as u8,
                    "alpha={alpha} premultiplied={premultiplied}"
                );
            }
        }
    }

    #[test]
    fn adler32_survives_multi_chunk_input() {
        // Longer than the 5552-byte reduction window, so it exercises the
        // chunked accumulation path.
        let data = vec![0xFFu8; 20_000];
        let mut a: u64 = 1;
        let mut b: u64 = 0;
        for &byte in &data {
            a = (a + u64::from(byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        assert_eq!(adler32(&data), ((b << 16) | a) as u32);
    }
}
