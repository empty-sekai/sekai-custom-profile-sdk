//! DEFLATE compressor (RFC 1951) and zlib writer (RFC 1950).
//!
//! Dependency-free. Hash-chain LZ77 matching feeds a single block, encoded with
//! either the fixed Huffman codes or a dynamic table fitted to the block's own
//! symbol frequencies, whichever comes out smaller. Output is deterministic for
//! a given input.

use super::CodecError;

const WINDOW_SIZE: usize = 32_768;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
/// Hash-chain buckets. Power of two so the mask is a single AND.
const HASH_BITS: usize = 15;
const HASH_SIZE: usize = 1 << HASH_BITS;
/// Chain walk limit. Bounds worst-case time on highly repetitive input.
const MAX_CHAIN: usize = 128;
/// A match this long is good enough to stop improving on: the bits saved by a
/// longer one no longer pay for the chain walk.
const NICE_MATCH: usize = 258;
/// Once a match this long is in hand, walk the rest of the chain at a quarter
/// of the budget.
const GOOD_MATCH: usize = 32;

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

/// Symbol index for each match length; built once, indexed by `length`.
fn length_symbol_index(length: usize) -> usize {
    static TABLE: std::sync::OnceLock<[u8; MAX_MATCH + 1]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0u8; MAX_MATCH + 1];
        for (slot, entry) in table.iter_mut().enumerate().skip(MIN_MATCH) {
            let mut idx = 0usize;
            for i in (0..LENGTH_BASE.len()).rev() {
                if slot >= usize::from(LENGTH_BASE[i]) {
                    idx = i;
                    break;
                }
            }
            *entry = idx as u8;
        }
        table
    });
    usize::from(table[length])
}

fn length_symbol(length: usize) -> (u16, u32, u32) {
    let idx = length_symbol_index(length);
    let extra_bits = u32::from(LENGTH_EXTRA[idx]);
    let extra = (length - usize::from(LENGTH_BASE[idx])) as u32;
    (257 + idx as u16, extra, extra_bits)
}

fn distance_symbol(distance: usize) -> (u16, u32, u32) {
    // Two codes per power of two above 4, so the index falls out of the bit
    // width: symbols come in pairs (base, base + half-range).
    let idx = if distance <= 4 {
        distance - 1
    } else {
        // Each power-of-two band [2^(b-1), 2^b) splits into two symbols at
        // three quarters of the band's end.
        let n = distance - 1;
        let bits = (usize::BITS - n.leading_zeros()) as usize;
        let quarter = 1usize << (bits - 2);
        2 * (bits - 1) + usize::from(n >= 3 * quarter)
    };
    let extra_bits = u32::from(DIST_EXTRA[idx]);
    let extra = (distance - usize::from(DIST_BASE[idx])) as u32;
    (idx as u16, extra, extra_bits)
}

fn hash3(data: &[u8], pos: usize) -> usize {
    // Only the low 3 bytes take part, so two positions hash equally exactly
    // when their next 3 bytes agree; the 4th byte read is just a cheaper load.
    let word = if pos + 4 <= data.len() {
        u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
    } else {
        u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], 0])
    };
    (((word & 0x00FF_FFFF).wrapping_mul(0x9E37_79B1)) >> (32 - HASH_BITS)) as usize
}

/// Longest common prefix of `data[a..]` and `data[b..]`, capped at `max_len`,
/// compared a word at a time.
fn match_length(data: &[u8], a: usize, b: usize, max_len: usize) -> usize {
    let mut len = 0usize;
    while len + 8 <= max_len {
        let left = u64::from_le_bytes(data[a + len..a + len + 8].try_into().expect("8 bytes"));
        let right = u64::from_le_bytes(data[b + len..b + len + 8].try_into().expect("8 bytes"));
        let diff = left ^ right;
        if diff != 0 {
            return len + (diff.trailing_zeros() / 8) as usize;
        }
        len += 8;
    }
    while len < max_len && data[a + len] == data[b + len] {
        len += 1;
    }
    len
}

/// One LZ77 decision: a literal byte, or a back-reference.
#[derive(Clone, Copy)]
enum Token {
    Literal(u8),
    Match { length: u16, distance: u16 },
}

/// Compresses `data` into a single DEFLATE block.
///
/// The input is tokenized once, then encoded with whichever block type is
/// smaller: fixed Huffman, or a dynamic Huffman table built from the token
/// frequencies. Dynamic wins on most real input because it can spend short codes
/// on the bytes that actually occur, but it carries a table, so short or uniform
/// input is cheaper with the fixed codes.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    let tokens = tokenize(data);
    let fixed = encode_fixed_block(&tokens);
    match encode_dynamic_block(&tokens) {
        Some(dynamic) if dynamic.len() < fixed.len() => dynamic,
        _ => fixed,
    }
}

fn tokenize(data: &[u8]) -> Vec<Token> {
    let mut tokens = Vec::with_capacity(data.len() / 2 + 1);
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
            let mut budget = MAX_CHAIN;
            while candidate != usize::MAX && candidate >= window_floor && walked < budget {
                walked += 1;
                // A candidate can only improve on the best match if it also
                // agrees at the byte the best match would extend past, so one
                // probe rejects most of the chain without a full compare.
                // `candidate < pos` always holds, so nothing reads past the
                // cursor.
                if best_len == 0
                    || (pos + best_len < data.len()
                        && data[candidate + best_len] == data[pos + best_len])
                {
                    let len = match_length(data, candidate, pos, max_len);
                    if len > best_len {
                        best_len = len;
                        best_dist = pos - candidate;
                        if len == max_len || len >= NICE_MATCH {
                            break;
                        }
                        if len >= GOOD_MATCH {
                            budget = budget.min(walked + MAX_CHAIN / 2);
                        }
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
            tokens.push(Token::Match {
                length: best_len as u16,
                distance: best_dist as u16,
            });
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
            tokens.push(Token::Literal(data[pos]));
            pos += 1;
        }
    }
    tokens
}

fn encode_fixed_block(tokens: &[Token]) -> Vec<u8> {
    let mut writer = BitWriter::new(tokens.len() + 64);
    // BFINAL=1, BTYPE=01 (fixed Huffman)
    writer.bits(1, 1);
    writer.bits(1, 2);
    for token in tokens {
        match *token {
            Token::Literal(byte) => {
                let (code, nbits) = fixed_literal_code(u16::from(byte));
                writer.code(code, nbits);
            }
            Token::Match { length, distance } => {
                let (symbol, extra, bits) = length_symbol(usize::from(length));
                let (code, nbits) = fixed_literal_code(symbol);
                writer.code(code, nbits);
                if bits > 0 {
                    writer.bits(extra, bits);
                }
                let (symbol, extra, bits) = distance_symbol(usize::from(distance));
                writer.code(u32::from(symbol), 5);
                if bits > 0 {
                    writer.bits(extra, bits);
                }
            }
        }
    }
    // End-of-block symbol 256.
    let (code, nbits) = fixed_literal_code(256);
    writer.code(code, nbits);
    writer.finish()
}

/// Order the code-length alphabet's lengths are transmitted in (RFC 1951 3.2.7).
const CODE_LENGTH_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Builds canonical Huffman code lengths for `frequencies`, none longer than
/// `limit` bits.
///
/// Symbols that never occur get length 0 and no code. When the natural code
/// exceeds the limit, the frequencies are flattened and the code rebuilt: that
/// costs a little compression and keeps the result a valid prefix code, which an
/// unbounded code would not be.
fn code_lengths(frequencies: &[u32], limit: u8) -> Vec<u8> {
    let mut scaled: Vec<u32> = frequencies.to_vec();
    loop {
        let lengths = huffman_lengths(&scaled);
        if lengths.iter().all(|length| *length <= limit) {
            return lengths;
        }
        for frequency in scaled.iter_mut() {
            if *frequency > 1 {
                *frequency = (*frequency + 1) / 2;
            }
        }
    }
}

/// Plain Huffman construction: repeatedly merge the two least frequent nodes and
/// count how deep each symbol ends up.
fn huffman_lengths(frequencies: &[u32]) -> Vec<u8> {
    #[derive(Clone)]
    struct Node {
        weight: u64,
        leaves: Vec<usize>,
    }

    let mut nodes: Vec<Node> = frequencies
        .iter()
        .enumerate()
        .filter(|(_, frequency)| **frequency > 0)
        .map(|(symbol, frequency)| Node {
            weight: u64::from(*frequency),
            leaves: vec![symbol],
        })
        .collect();
    let mut lengths = vec![0u8; frequencies.len()];
    match nodes.len() {
        0 => return lengths,
        // A single symbol still needs one bit: a zero-length code cannot be read
        // back, and the decoder rejects an under-subscribed table.
        1 => {
            lengths[nodes[0].leaves[0]] = 1;
            return lengths;
        }
        _ => {}
    }
    while nodes.len() > 1 {
        nodes.sort_by(|left, right| {
            right
                .weight
                .cmp(&left.weight)
                .then_with(|| right.leaves.len().cmp(&left.leaves.len()))
        });
        let first = nodes.pop().expect("at least two nodes");
        let second = nodes.pop().expect("at least two nodes");
        for symbol in first.leaves.iter().chain(second.leaves.iter()) {
            lengths[*symbol] = lengths[*symbol].saturating_add(1);
        }
        let mut leaves = first.leaves;
        leaves.extend(second.leaves);
        nodes.push(Node {
            weight: first.weight + second.weight,
            leaves,
        });
    }
    lengths
}

/// Assigns canonical codes for the given lengths, matching the decoder's rule.
fn canonical_codes(lengths: &[u8]) -> Vec<u32> {
    let max = lengths.iter().copied().max().unwrap_or(0);
    let mut counts = vec![0u32; usize::from(max) + 1];
    for length in lengths {
        if *length > 0 {
            counts[usize::from(*length)] += 1;
        }
    }
    let mut next = vec![0u32; usize::from(max) + 2];
    let mut code = 0u32;
    for bits in 1..=usize::from(max) {
        code = (code + counts[bits - 1]) << 1;
        next[bits] = code;
    }
    lengths
        .iter()
        .map(|length| {
            if *length == 0 {
                0
            } else {
                let assigned = next[usize::from(*length)];
                next[usize::from(*length)] += 1;
                assigned
            }
        })
        .collect()
}

/// Run-length encodes a code-length sequence with symbols 16, 17 and 18.
fn encode_code_lengths(lengths: &[u8]) -> Vec<(u8, u32, u32)> {
    let mut out: Vec<(u8, u32, u32)> = Vec::new();
    let mut index = 0usize;
    while index < lengths.len() {
        let value = lengths[index];
        let total = lengths[index..]
            .iter()
            .take_while(|length| **length == value)
            .count();
        index += total;
        let mut run = total;
        if value == 0 {
            while run >= 3 {
                if run >= 11 {
                    let emit = run.min(138);
                    out.push((18, (emit - 11) as u32, 7));
                    run -= emit;
                } else {
                    let emit = run.min(10);
                    out.push((17, (emit - 3) as u32, 3));
                    run -= emit;
                }
            }
            for _ in 0..run {
                out.push((0, 0, 0));
            }
        } else {
            out.push((value, 0, 0));
            run -= 1;
            while run >= 3 {
                let emit = run.min(6);
                out.push((16, (emit - 3) as u32, 2));
                run -= emit;
            }
            for _ in 0..run {
                out.push((value, 0, 0));
            }
        }
    }
    out
}

fn encode_dynamic_block(tokens: &[Token]) -> Option<Vec<u8>> {
    let mut literal_frequencies = [0u32; 286];
    let mut distance_frequencies = [0u32; 30];
    literal_frequencies[256] = 1;
    for token in tokens {
        match *token {
            Token::Literal(byte) => literal_frequencies[usize::from(byte)] += 1,
            Token::Match { length, distance } => {
                let (symbol, _, _) = length_symbol(usize::from(length));
                literal_frequencies[usize::from(symbol)] += 1;
                let (symbol, _, _) = distance_symbol(usize::from(distance));
                distance_frequencies[usize::from(symbol)] += 1;
            }
        }
    }

    let literal_lengths = code_lengths(&literal_frequencies, 15);
    let mut distance_lengths = code_lengths(&distance_frequencies, 15);
    // The decoder rejects an empty distance table, so a literal-only block still
    // declares one unused single-bit code.
    if distance_lengths.iter().all(|length| *length == 0) {
        distance_lengths[0] = 1;
    }

    let hlit = (257..=286)
        .rev()
        .find(|count| literal_lengths[count - 1] != 0)
        .unwrap_or(257);
    let hdist = (1..=30)
        .rev()
        .find(|count| distance_lengths[count - 1] != 0)
        .unwrap_or(1);

    let mut combined = literal_lengths[..hlit].to_vec();
    combined.extend_from_slice(&distance_lengths[..hdist]);
    let encoded = encode_code_lengths(&combined);

    let mut code_length_frequencies = [0u32; 19];
    for (symbol, _, _) in &encoded {
        code_length_frequencies[usize::from(*symbol)] += 1;
    }
    let code_length_lengths = code_lengths(&code_length_frequencies, 7);
    let hclen = (4..=19)
        .rev()
        .find(|count| code_length_lengths[CODE_LENGTH_ORDER[count - 1]] != 0)
        .unwrap_or(4);

    let literal_codes = canonical_codes(&literal_lengths);
    let distance_codes = canonical_codes(&distance_lengths);
    let code_length_codes = canonical_codes(&code_length_lengths);

    let mut writer = BitWriter::new(tokens.len() + 128);
    // BFINAL=1, BTYPE=10 (dynamic Huffman)
    writer.bits(1, 1);
    writer.bits(2, 2);
    writer.bits((hlit - 257) as u32, 5);
    writer.bits((hdist - 1) as u32, 5);
    writer.bits((hclen - 4) as u32, 4);
    for slot in CODE_LENGTH_ORDER.iter().take(hclen) {
        writer.bits(u32::from(code_length_lengths[*slot]), 3);
    }
    for (symbol, extra, bits) in &encoded {
        let index = usize::from(*symbol);
        writer.code(
            code_length_codes[index],
            u32::from(code_length_lengths[index]),
        );
        if *bits > 0 {
            writer.bits(*extra, *bits);
        }
    }
    let emit = |writer: &mut BitWriter, symbol: usize| -> Option<()> {
        (literal_lengths[symbol] > 0).then(|| {
            writer.code(literal_codes[symbol], u32::from(literal_lengths[symbol]));
        })
    };
    for token in tokens {
        match *token {
            Token::Literal(byte) => emit(&mut writer, usize::from(byte))?,
            Token::Match { length, distance } => {
                let (symbol, extra, bits) = length_symbol(usize::from(length));
                emit(&mut writer, usize::from(symbol))?;
                if bits > 0 {
                    writer.bits(extra, bits);
                }
                let (symbol, extra, bits) = distance_symbol(usize::from(distance));
                let index = usize::from(symbol);
                if distance_lengths[index] == 0 {
                    return None;
                }
                writer.code(distance_codes[index], u32::from(distance_lengths[index]));
                if bits > 0 {
                    writer.bits(extra, bits);
                }
            }
        }
    }
    emit(&mut writer, 256)?;
    Some(writer.finish())
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
            "LZ77 should compress a constant buffer by >10x"
        );
    }

    /// A skewed byte distribution is exactly what the dynamic table exists for:
    /// the fixed codes spend 8 bits on every literal regardless of how often it
    /// occurs.
    #[test]
    fn a_dynamic_table_beats_the_fixed_codes_on_skewed_literals() {
        let mut data = Vec::new();
        for i in 0..40_000u32 {
            data.push(match i % 16 {
                0 => 0x41,
                1 => 0x42,
                2 => 0x43,
                _ => 0x20,
            });
        }
        // Break up the matches so the block is literal-dominated.
        for (i, byte) in data.iter_mut().enumerate() {
            if i % 5 == 0 {
                *byte = (i % 251) as u8;
            }
        }
        let tokens = tokenize(&data);
        let fixed = encode_fixed_block(&tokens).len();
        let dynamic = encode_dynamic_block(&tokens).expect("dynamic block").len();
        assert!(
            dynamic < fixed,
            "dynamic {dynamic} should beat fixed {fixed} on a skewed distribution"
        );
        assert_eq!(
            deflate(&data).len(),
            dynamic,
            "deflate must pick the smaller"
        );
        round_trip(&data);
    }

    /// A table has to be transmitted before it can pay off, so on short input the
    /// fixed codes win and `deflate` must take them.
    #[test]
    fn the_fixed_codes_win_when_a_table_would_not_pay_for_itself() {
        for data in [
            b"abcdefghij".as_slice(),
            b"hello".as_slice(),
            b"".as_slice(),
        ] {
            let tokens = tokenize(data);
            let fixed = encode_fixed_block(&tokens);
            let dynamic = encode_dynamic_block(&tokens).expect("dynamic block");
            assert!(
                fixed.len() < dynamic.len(),
                "{} bytes of input: fixed {} should beat dynamic {}",
                data.len(),
                fixed.len(),
                dynamic.len()
            );
            assert_eq!(deflate(data), fixed, "deflate must pick the smaller");
            round_trip(data);
        }
    }

    /// Uniform bytes give a table nothing to exploit beyond the fixed codes'
    /// 9-bit tail, so the two encodings must land close together and both decode.
    #[test]
    fn uniform_noise_compresses_about_as_well_either_way() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let data: Vec<u8> = (0..8_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 33) as u8
            })
            .collect();
        let tokens = tokenize(&data);
        let fixed = encode_fixed_block(&tokens).len();
        let dynamic = encode_dynamic_block(&tokens).expect("dynamic block").len();
        let spread = fixed.abs_diff(dynamic);
        assert!(
            spread * 20 < fixed,
            "neither encoding should win by more than 5% on noise: fixed {fixed} vs dynamic {dynamic}"
        );
        assert_eq!(deflate(&data).len(), fixed.min(dynamic));
        round_trip(&data);
    }

    /// Both block encodings must decode to the input on their own, so a size tie
    /// broken either way stays correct.
    #[test]
    fn both_block_encodings_decode_to_the_input() {
        let cases: Vec<Vec<u8>> = vec![
            b"".to_vec(),
            b"a".to_vec(),
            vec![0u8; 300],
            b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab".to_vec(),
            (0..600u32).map(|i| (i % 3) as u8).collect(),
            (0..2_000u32).map(|i| (i * 7 % 256) as u8).collect(),
        ];
        for data in cases {
            let tokens = tokenize(&data);
            let fixed = encode_fixed_block(&tokens);
            assert_eq!(
                inflate(&fixed, data.len()).expect("inflate fixed"),
                data,
                "fixed block round trip"
            );
            let dynamic = encode_dynamic_block(&tokens).expect("dynamic block");
            assert_eq!(
                inflate(&dynamic, data.len()).expect("inflate dynamic"),
                data,
                "dynamic block round trip"
            );
        }
    }

    /// Every code the encoder builds has to satisfy the Kraft equality the
    /// decoder checks, at every length limit the two alphabets use.
    #[test]
    fn generated_code_lengths_form_a_complete_prefix_code() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for case in 0..200u32 {
            let alphabet = 2 + (case as usize % 30);
            let frequencies: Vec<u32> = (0..alphabet)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    (state % 4096) as u32
                })
                .collect();
            for limit in [7u8, 15] {
                let lengths = code_lengths(&frequencies, limit);
                assert!(lengths.iter().all(|length| *length <= limit));
                let used = lengths.iter().filter(|length| **length > 0).count();
                let declared = frequencies.iter().filter(|f| **f > 0).count();
                assert_eq!(used, declared, "only the used symbols get a code");
                if used < 2 {
                    continue;
                }
                let kraft: f64 = lengths
                    .iter()
                    .filter(|length| **length > 0)
                    .map(|length| 0.5f64.powi(i32::from(*length)))
                    .sum();
                assert!(
                    (kraft - 1.0).abs() < 1e-9,
                    "case {case} limit {limit}: Kraft sum {kraft}"
                );
            }
        }
    }

    /// The closed-form distance index must agree with a scan of the RFC table
    /// over the entire distance range.
    #[test]
    fn distance_symbol_matches_the_rfc_table_for_every_distance() {
        for distance in 1..=32_768usize {
            let mut expected = 0usize;
            for i in (0..DIST_BASE.len()).rev() {
                if distance >= usize::from(DIST_BASE[i]) {
                    expected = i;
                    break;
                }
            }
            let (symbol, extra, extra_bits) = distance_symbol(distance);
            assert_eq!(usize::from(symbol), expected, "distance {distance}");
            assert_eq!(extra as usize, distance - usize::from(DIST_BASE[expected]));
            assert_eq!(extra_bits, u32::from(DIST_EXTRA[expected]));
        }
    }

    /// Same for the length table, which is a plain lookup.
    #[test]
    fn length_symbol_matches_the_rfc_table_for_every_length() {
        for length in MIN_MATCH..=MAX_MATCH {
            let mut expected = 0usize;
            for i in (0..LENGTH_BASE.len()).rev() {
                if length >= usize::from(LENGTH_BASE[i]) {
                    expected = i;
                    break;
                }
            }
            let (symbol, extra, extra_bits) = length_symbol(length);
            assert_eq!(usize::from(symbol), 257 + expected, "length {length}");
            assert_eq!(extra as usize, length - usize::from(LENGTH_BASE[expected]));
            assert_eq!(extra_bits, u32::from(LENGTH_EXTRA[expected]));
        }
    }

    #[test]
    fn code_length_run_encoding_reproduces_the_sequence() {
        let cases: Vec<Vec<u8>> = vec![
            vec![0; 200],
            vec![4; 200],
            {
                let mut v = vec![0u8; 150];
                v.extend(std::iter::repeat_n(6u8, 9));
                v.extend(std::iter::repeat_n(0u8, 2));
                v.push(3);
                v
            },
            (0..287u32).map(|i| (i % 16) as u8).collect(),
        ];
        for lengths in cases {
            let encoded = encode_code_lengths(&lengths);
            let mut decoded: Vec<u8> = Vec::new();
            for (symbol, extra, _) in &encoded {
                match symbol {
                    16 => {
                        let last = *decoded.last().expect("repeat needs a previous length");
                        decoded.extend(std::iter::repeat_n(last, 3 + *extra as usize));
                    }
                    17 => decoded.extend(std::iter::repeat_n(0u8, 3 + *extra as usize)),
                    18 => decoded.extend(std::iter::repeat_n(0u8, 11 + *extra as usize)),
                    value => decoded.push(*value),
                }
            }
            assert_eq!(decoded, lengths, "run encoding must be lossless");
        }
    }

    #[test]
    fn output_is_deterministic() {
        let data: Vec<u8> = (0..20_000u32).map(|i| (i * 31 % 97) as u8).collect();
        assert_eq!(deflate(&data), deflate(&data));
    }
}
