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
