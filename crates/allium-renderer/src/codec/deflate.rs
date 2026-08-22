//! DEFLATE compressor (RFC 1951) and zlib writer (RFC 1950).
//!
//! Dependency-free. Emits fixed-Huffman blocks with hash-chain LZ77 matching:
//! enough to keep encoded assets a reasonable size without carrying a dynamic
//! Huffman optimizer. Output is deterministic for a given input.

use super::CodecError;

const WINDOW_SIZE: usize = 32_768;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
/// Hash-chain buckets. Power of two so the mask is a single AND.
const HASH_BITS: usize = 15;
const HASH_SIZE: usize = 1 << HASH_BITS;
/// Chain walk limit. Bounds worst-case time on highly repetitive input.
const MAX_CHAIN: usize = 128;

struct BitWriter {
    out: Vec<u8>,
    bit_buf: u32,
    bit_count: u32,
}

impl BitWriter {
    fn new(capacity: usize) -> Self {
        Self {
            out: Vec::with_capacity(capacity),
            bit_buf: 0,
            bit_count: 0,
        }
    }

    /// Writes `count` low bits of `value`, LSB first (RFC 1951 packing).
    fn bits(&mut self, value: u32, count: u32) {
        self.bit_buf |= value << self.bit_count;
        self.bit_count += count;
        while self.bit_count >= 8 {
            self.out.push((self.bit_buf & 0xFF) as u8);
            self.bit_buf >>= 8;
            self.bit_count -= 8;
        }
    }

    /// Writes a Huffman code, which is packed MSB first.
    fn code(&mut self, code: u32, count: u32) {
        let mut reversed = 0u32;
        for i in 0..count {
            reversed |= ((code >> i) & 1) << (count - 1 - i);
        }
        self.bits(reversed, count);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bit_count > 0 {
            self.out.push((self.bit_buf & 0xFF) as u8);
        }
        self.out
    }
}

/// Fixed literal/length code per RFC 1951 section 3.2.6.
fn fixed_literal_code(sym: u16) -> (u32, u32) {
    match sym {
        0..=143 => (0x30 + u32::from(sym), 8),
        144..=255 => (0x190 + u32::from(sym) - 144, 9),
        256..=279 => (u32::from(sym) - 256, 7),
        _ => (0xC0 + u32::from(sym) - 280, 8),
    }
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

fn length_symbol(length: usize) -> (u16, u32, u32) {
    let mut idx = 0usize;
    for i in (0..LENGTH_BASE.len()).rev() {
        if length >= usize::from(LENGTH_BASE[i]) {
            idx = i;
            break;
        }
    }
    let extra_bits = u32::from(LENGTH_EXTRA[idx]);
    let extra = (length - usize::from(LENGTH_BASE[idx])) as u32;
    (257 + idx as u16, extra, extra_bits)
}

fn distance_symbol(distance: usize) -> (u16, u32, u32) {
    let mut idx = 0usize;
    for i in (0..DIST_BASE.len()).rev() {
        if distance >= usize::from(DIST_BASE[i]) {
            idx = i;
            break;
        }
    }
    let extra_bits = u32::from(DIST_EXTRA[idx]);
    let extra = (distance - usize::from(DIST_BASE[idx])) as u32;
    (idx as u16, extra, extra_bits)
}

fn hash3(data: &[u8], pos: usize) -> usize {
    let a = u32::from(data[pos]);
    let b = u32::from(data[pos + 1]);
    let c = u32::from(data[pos + 2]);
    (((a << 10) ^ (b << 5) ^ c).wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize % HASH_SIZE
}

/// Compresses `data` into a single fixed-Huffman DEFLATE stream.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut writer = BitWriter::new(data.len() / 2 + 64);
    // BFINAL=1, BTYPE=01 (fixed Huffman)
    writer.bits(1, 1);
    writer.bits(1, 2);

    let mut head = vec![usize::MAX; HASH_SIZE];
    let mut prev = vec![usize::MAX; data.len().max(1)];

    let mut pos = 0usize;
    while pos < data.len() {
        let mut best_len = 0usize;
        let mut best_dist = 0usize;

        if pos + MIN_MATCH <= data.len() {
            let h = hash3(data, pos);
            let mut candidate = head[h];
            let max_len = (data.len() - pos).min(MAX_MATCH);
            let window_floor = pos.saturating_sub(WINDOW_SIZE);
            let mut walked = 0usize;
            while candidate != usize::MAX && candidate >= window_floor && walked < MAX_CHAIN {
                walked += 1;
                // Compare forward; `candidate < pos` always holds so this never
                // reads past the cursor.
                let mut len = 0usize;
                while len < max_len && data[candidate + len] == data[pos + len] {
                    len += 1;
                }
                if len > best_len {
                    best_len = len;
                    best_dist = pos - candidate;
                    if len == max_len {
                        break;
                    }
                }
                candidate = prev[candidate];
            }
            // Insert the current position after searching so a match never has
            // distance 0.
            prev[pos] = head[h];
            head[h] = pos;
        }

        if best_len >= MIN_MATCH {
            let (lsym, lextra, lbits) = length_symbol(best_len);
            let (code, nbits) = fixed_literal_code(lsym);
            writer.code(code, nbits);
            if lbits > 0 {
                writer.bits(lextra, lbits);
            }
            let (dsym, dextra, dbits) = distance_symbol(best_dist);
            writer.code(u32::from(dsym), 5);
            if dbits > 0 {
                writer.bits(dextra, dbits);
            }
            // Register the interior positions so later matches can reach them.
            for k in 1..best_len {
                let p = pos + k;
                if p + MIN_MATCH <= data.len() {
                    let h = hash3(data, p);
                    prev[p] = head[h];
                    head[h] = p;
                }
            }
            pos += best_len;
        } else {
            let (code, nbits) = fixed_literal_code(u16::from(data[pos]));
            writer.code(code, nbits);
            pos += 1;
        }
    }

    // End-of-block symbol 256.
    let (code, nbits) = fixed_literal_code(256);
    writer.code(code, nbits);
    writer.finish()
}

/// Wraps `deflate` output in a zlib stream (RFC 1950).
pub fn zlib_compress(data: &[u8]) -> Result<Vec<u8>, CodecError> {
    let body = deflate(data);
    let mut out = Vec::with_capacity(body.len() + 6);
    // CMF: deflate, 32K window. FLG chosen so (CMF<<8 | FLG) % 31 == 0.
    let cmf = 0x78u8;
    let flg = {
        let base = (u16::from(cmf) << 8) as u32;
        let rem = (base % 31) as u8;
        if rem == 0 {
            0
        } else {
            31 - rem
        }
    };
    out.push(cmf);
    out.push(flg);
    out.extend_from_slice(&body);
    out.extend_from_slice(&super::adler32(data).to_be_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::inflate::{inflate, zlib_decompress};
    use super::*;

    fn round_trip(data: &[u8]) {
        let compressed = deflate(data);
        let back = inflate(&compressed, data.len()).expect("inflate");
        assert_eq!(back, data, "deflate/inflate round trip mismatch");
    }

    #[test]
    fn empty_input_round_trips() {
        round_trip(b"");
    }

    #[test]
    fn literal_only_round_trips() {
        round_trip(b"abcdefghij");
    }

    #[test]
    fn highly_repetitive_input_round_trips() {
        round_trip(&vec![0xABu8; 100_000]);
    }

    #[test]
    fn long_match_at_max_length_round_trips() {
        let mut data = vec![0u8; MAX_MATCH * 3];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i % 7) as u8;
        }
        round_trip(&data);
    }

    #[test]
    fn mixed_text_round_trips() {
        let text = b"the quick brown fox jumps over the lazy dog; the quick brown fox again";
        round_trip(text);
    }

    #[test]
    fn zlib_round_trips_and_carries_a_valid_header() {
        let data: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let z = zlib_compress(&data).expect("compress");
        assert_eq!(z[0] & 0x0F, 8, "compression method must be deflate");
        assert_eq!(((u16::from(z[0]) << 8) | u16::from(z[1])) % 31, 0);
        let back = zlib_decompress(&z, data.len()).expect("decompress");
        assert_eq!(back, data);
    }

    #[test]
    fn compression_actually_shrinks_repetitive_input() {
        let data = vec![0x5Au8; 64_000];
        assert!(
            deflate(&data).len() < data.len() / 10,
            "fixed-Huffman LZ77 should compress a constant buffer by >10x"
        );
    }

    #[test]
    fn output_is_deterministic() {
        let data: Vec<u8> = (0..20_000u32).map(|i| (i * 31 % 97) as u8).collect();
        assert_eq!(deflate(&data), deflate(&data));
    }
}
