//! DEFLATE decompressor (RFC 1951) and zlib stream reader (RFC 1950).
//!
//! Dependency-free. Malformed input is rejected instead of yielding partial
//! data, so a corrupt asset surfaces as an error rather than as wrong pixels.

use super::CodecError;

/// Maximum bits in a DEFLATE Huffman code.
const MAX_BITS: usize = 15;

/// Canonical Huffman decoding table built from code lengths.
struct Huffman {
    /// `counts[n]` = number of codes of length `n`.
    counts: [u16; MAX_BITS + 1],
    /// Symbols ordered by (length, symbol).
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Self, CodecError> {
        let mut counts = [0u16; MAX_BITS + 1];
        for &l in lengths {
            let l = usize::from(l);
            if l > MAX_BITS {
                return Err(CodecError::Deflate("code length exceeds 15"));
            }
            counts[l] += 1;
        }
        // An entirely unused table (all lengths zero) is legal; over-subscribed is not.
        let mut left: i32 = 1;
        for len in 1..=MAX_BITS {
            left <<= 1;
            left -= i32::from(counts[len]);
            if left < 0 {
                return Err(CodecError::Deflate("over-subscribed Huffman code"));
            }
        }
        let mut offsets = [0u16; MAX_BITS + 2];
        for len in 1..=MAX_BITS {
            offsets[len + 1] = offsets[len] + counts[len];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                let slot = usize::from(offsets[usize::from(l)]);
                if slot >= symbols.len() {
                    return Err(CodecError::Deflate("Huffman symbol overflow"));
                }
                symbols[slot] = sym as u16;
                offsets[usize::from(l)] += 1;
            }
        }
        counts[0] = 0;
        Ok(Self { counts, symbols })
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buf: u32,
    bit_count: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            bit_buf: 0,
            bit_count: 0,
        }
    }

    fn need(&mut self, bits: u32) -> Result<(), CodecError> {
        while self.bit_count < bits {
            let byte = *self
                .data
                .get(self.pos)
                .ok_or(CodecError::Deflate("unexpected end of deflate stream"))?;
            self.pos += 1;
            self.bit_buf |= u32::from(byte) << self.bit_count;
            self.bit_count += 8;
        }
        Ok(())
    }

    fn bits(&mut self, bits: u32) -> Result<u32, CodecError> {
        if bits == 0 {
            return Ok(0);
        }
        self.need(bits)?;
        let value = self.bit_buf & ((1u32 << bits) - 1);
        self.bit_buf >>= bits;
        self.bit_count -= bits;
        Ok(value)
    }

    fn align_to_byte(&mut self) {
        let drop = self.bit_count % 8;
        self.bit_buf >>= drop;
        self.bit_count -= drop;
    }

    /// Decodes one canonical Huffman symbol, walking code lengths shortest-first
    /// per RFC 1951 section 3.2.2.
    fn symbol(&mut self, table: &Huffman) -> Result<u16, CodecError> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..=MAX_BITS {
            code |= self.bits(1)? as i32;
            let count = i32::from(table.counts[len]);
            if code - first < count {
                let slot = (index + (code - first)) as usize;
                return table
                    .symbols
                    .get(slot)
                    .copied()
                    .ok_or(CodecError::Deflate("invalid Huffman symbol"));
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(CodecError::Deflate("invalid Huffman code"))
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
const CODE_LENGTH_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

fn fixed_tables() -> Result<(Huffman, Huffman), CodecError> {
    let mut lit = [0u8; 288];
    for (i, l) in lit.iter_mut().enumerate() {
        *l = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    let dist = [5u8; 30];
    Ok((Huffman::new(&lit)?, Huffman::new(&dist)?))
}

/// Inflates a raw DEFLATE stream. `size_hint` preallocates the output buffer.
pub fn inflate(data: &[u8], size_hint: usize) -> Result<Vec<u8>, CodecError> {
    let mut reader = BitReader::new(data);
    let mut out = Vec::with_capacity(size_hint);
    loop {
        let last = reader.bits(1)?;
        let kind = reader.bits(2)?;
        match kind {
            0 => {
                reader.align_to_byte();
                // Read the aligned header through the bit reader so an already
                // buffered byte cannot be skipped.
                let len = reader.bits(16)? as usize;
                let nlen = reader.bits(16)? as usize;
                if len ^ 0xFFFF != nlen {
                    return Err(CodecError::Deflate("stored block length check failed"));
                }
                for _ in 0..len {
                    out.push(reader.bits(8)? as u8);
                }
            }
            1 | 2 => {
                let (lit, dist) = if kind == 1 {
                    fixed_tables()?
                } else {
                    read_dynamic_tables(&mut reader)?
                };
                inflate_block(&mut reader, &lit, &dist, &mut out)?;
            }
            _ => return Err(CodecError::Deflate("reserved deflate block type")),
        }
        if last == 1 {
            break;
        }
    }
    Ok(out)
}

fn read_dynamic_tables(reader: &mut BitReader<'_>) -> Result<(Huffman, Huffman), CodecError> {
    let hlit = reader.bits(5)? as usize + 257;
    let hdist = reader.bits(5)? as usize + 1;
    let hclen = reader.bits(4)? as usize + 4;
    if hlit > 286 || hdist > 30 {
        return Err(CodecError::Deflate("dynamic table size out of range"));
    }
    let mut code_lengths = [0u8; 19];
    for &slot in CODE_LENGTH_ORDER.iter().take(hclen) {
        code_lengths[slot] = reader.bits(3)? as u8;
    }
    let code_table = Huffman::new(&code_lengths)?;

    let total = hlit + hdist;
    let mut lengths = vec![0u8; total];
    let mut i = 0usize;
    while i < total {
        let sym = reader.symbol(&code_table)?;
        match sym {
            0..=15 => {
                lengths[i] = sym as u8;
                i += 1;
            }
            16 => {
                let prev = i
                    .checked_sub(1)
                    .and_then(|p| lengths.get(p).copied())
                    .ok_or(CodecError::Deflate("repeat with no previous length"))?;
                let repeat = 3 + reader.bits(2)? as usize;
                if i + repeat > total {
                    return Err(CodecError::Deflate("length repeat overruns table"));
                }
                for _ in 0..repeat {
                    lengths[i] = prev;
                    i += 1;
                }
            }
            17 | 18 => {
                let repeat = if sym == 17 {
                    3 + reader.bits(3)? as usize
                } else {
                    11 + reader.bits(7)? as usize
                };
                if i + repeat > total {
                    return Err(CodecError::Deflate("zero repeat overruns table"));
                }
                i += repeat;
            }
            _ => return Err(CodecError::Deflate("invalid code-length symbol")),
        }
    }
    let lit = Huffman::new(&lengths[..hlit])?;
    let dist = Huffman::new(&lengths[hlit..])?;
    Ok((lit, dist))
}

fn inflate_block(
    reader: &mut BitReader<'_>,
    lit: &Huffman,
    dist: &Huffman,
    out: &mut Vec<u8>,
) -> Result<(), CodecError> {
    loop {
        let sym = reader.symbol(lit)?;
        match sym {
            0..=255 => out.push(sym as u8),
            256 => return Ok(()),
            257..=285 => {
                let idx = usize::from(sym) - 257;
                let length = usize::from(LENGTH_BASE[idx])
                    + reader.bits(u32::from(LENGTH_EXTRA[idx]))? as usize;
                let dsym = usize::from(reader.symbol(dist)?);
                if dsym >= DIST_BASE.len() {
                    return Err(CodecError::Deflate("invalid distance symbol"));
                }
                let distance = usize::from(DIST_BASE[dsym])
                    + reader.bits(u32::from(DIST_EXTRA[dsym]))? as usize;
                if distance == 0 || distance > out.len() {
                    return Err(CodecError::Deflate("distance exceeds window"));
                }
                let start = out.len() - distance;
                for k in 0..length {
                    let byte = out[start + k];
                    out.push(byte);
                }
            }
            _ => return Err(CodecError::Deflate("invalid literal/length symbol")),
        }
    }
}

/// Reads a zlib stream (RFC 1950): 2-byte header, DEFLATE payload, Adler-32.
pub fn zlib_decompress(data: &[u8], size_hint: usize) -> Result<Vec<u8>, CodecError> {
    if data.len() < 6 {
        return Err(CodecError::Deflate("zlib stream too short"));
    }
    let cmf = data[0];
    let flg = data[1];
    if cmf & 0x0F != 8 {
        return Err(CodecError::Deflate("unsupported zlib compression method"));
    }
    if ((u16::from(cmf) << 8) | u16::from(flg)) % 31 != 0 {
        return Err(CodecError::Deflate("zlib header check failed"));
    }
    if flg & 0x20 != 0 {
        return Err(CodecError::Deflate("preset dictionary is not supported"));
    }
    let out = inflate(&data[2..], size_hint)?;
    // The trailing Adler-32 is the stream's own integrity claim; verifying it
    // makes a truncated or corrupted asset fail loudly.
    let trailer = &data[data.len() - 4..];
    let expected = u32::from_be_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
    if expected != super::adler32(&out) {
        return Err(CodecError::Deflate("zlib Adler-32 mismatch"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_block_round_trips() {
        // BFINAL=1, BTYPE=00, LEN=3, NLEN=!3, payload "abc"
        let stream = [0x01, 0x03, 0x00, 0xFC, 0xFF, b'a', b'b', b'c'];
        assert_eq!(inflate(&stream, 3).expect("inflate"), b"abc");
    }

    #[test]
    fn truncated_stream_is_an_error_not_partial_output() {
        let stream = [0x01, 0x03, 0x00, 0xFC, 0xFF, b'a'];
        assert!(inflate(&stream, 3).is_err());
    }

    #[test]
    fn over_subscribed_table_is_rejected() {
        assert!(Huffman::new(&[1, 1, 1]).is_err());
    }

    #[test]
    fn zlib_header_check_is_enforced() {
        assert!(zlib_decompress(&[0x78, 0x00, 0, 0, 0, 0], 0).is_err());
    }

    #[test]
    fn reserved_block_type_is_rejected() {
        // BFINAL=1, BTYPE=11
        assert!(inflate(&[0x07], 0).is_err());
    }
}
