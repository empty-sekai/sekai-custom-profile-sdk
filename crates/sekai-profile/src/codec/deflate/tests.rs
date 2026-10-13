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
