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
/// This is the premultiplied form the compositor consumes:
/// `round(value * alpha / 255)`.
#[inline]
pub fn premultiply_channel(value: u8, alpha: u8) -> u8 {
    ((u32::from(value) * u32::from(alpha) + 127) / 255) as u8
}

/// Recovers a colour channel from its premultiplied form.
///
/// The round trip through premultiplied storage is lossy, and more so as alpha
/// falls: near half alpha a channel can move by one step, at alpha 3 by tens of
/// steps. This function fixes the value read back.
///
/// The division runs in single precision: normalise both inputs through
/// `1/255`, multiply by the reciprocal of alpha, clamp to `[0, 1]`, then
/// rescale by 255 and round to nearest with ties to even. The result therefore
/// follows f32 representation rather than exact rational arithmetic, and the
/// exact-half cases round both ways: (1, 6) gives 42 and (3, 10) gives 77.
/// [`premultiply_channel`] is its left inverse on every reachable pair.
#[inline]
pub fn unpremultiply_channel(premultiplied: u8, alpha: u8) -> u8 {
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

/// Test support for code that has to fail cleanly when memory runs out.
///
/// A test cannot cap its own process without starving the tests running next
/// to it, so the capped body runs in a fresh copy of the test binary: the
/// visible test calls [`run_in_child`](address_space::run_in_child) with the
/// name of an ignored test, and that test wraps its body in
/// [`with_budget`](address_space::with_budget).
#[cfg(all(test, target_os = "linux", target_env = "gnu"))]
pub(crate) mod address_space {
    use std::process::Command;

    const CHILD_ENV: &str = "SEKAI_PROFILE_RENDERER_ADDRESS_SPACE_CHILD";
    const RLIMIT_AS: i32 = 9;

    #[repr(C)]
    struct RLimit {
        current: u64,
        maximum: u64,
    }

    // SAFETY: the signatures below match the host C ABI on glibc, whose
    // `getrlimit(2)`/`setrlimit(2)` take the `RLIMIT_AS` selector and a
    // two-`u64` limit struct. Every call site passes a live local of that
    // exact layout.
    unsafe extern "C" {
        fn getrlimit(resource: i32, limit: *mut RLimit) -> i32;
        fn setrlimit(resource: i32, limit: *const RLimit) -> i32;
    }

    fn mapped_bytes() -> u64 {
        let status = std::fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
        let kib: u64 = status
            .lines()
            .find_map(|line| line.strip_prefix("VmSize:"))
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse().ok())
            .expect("VmSize in /proc/self/status");
        kib * 1024
    }

    /// True inside a child started by [`run_in_child`]. Capped tests return
    /// early otherwise, so running ignored tests in-process cannot cap the
    /// whole harness.
    pub(crate) fn in_child() -> bool {
        std::env::var_os(CHILD_ENV).is_some()
    }

    /// Runs `body` with at most `budget` more bytes of address space than the
    /// process has mapped when it starts, then lifts the cap again.
    pub(crate) fn with_budget<T>(budget: u64, body: impl FnOnce() -> T) -> T {
        assert!(in_child(), "a capped body must run through run_in_child");
        let mut saved = RLimit {
            current: 0,
            maximum: 0,
        };
        // SAFETY: the declaration above matches glibc's
        // `getrlimit(resource, *mut RLimit)`; `RLIMIT_AS` is a valid resource
        // selector and `&mut saved` is a live, properly aligned `RLimit`.
        assert_eq!(unsafe { getrlimit(RLIMIT_AS, &mut saved) }, 0, "getrlimit");
        let capped = RLimit {
            current: (mapped_bytes() + budget).min(saved.maximum),
            maximum: saved.maximum,
        };
        // SAFETY: the declaration above matches glibc's
        // `setrlimit(resource, *const RLimit)`; `&capped` is a live, properly
        // aligned `RLimit` that the call only reads.
        assert_eq!(unsafe { setrlimit(RLIMIT_AS, &capped) }, 0, "setrlimit");
        let result = body();
        // SAFETY: same contract as the call above; `&saved` is still live and
        // still holds the limits `getrlimit` returned.
        assert_eq!(unsafe { setrlimit(RLIMIT_AS, &saved) }, 0, "restore rlimit");
        result
    }

    /// Runs the ignored test `name` in a fresh copy of this test binary and
    /// asserts that it ran and passed.
    pub(crate) fn run_in_child(name: &str) {
        let output = Command::new(std::env::current_exe().expect("test binary path"))
            .args([
                "--exact",
                name,
                "--ignored",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            // One malloc arena, so every allocation has to come out of the
            // capped address space instead of a per-thread heap reserved
            // before the cap applied.
            .env("MALLOC_ARENA_MAX", "1")
            .output()
            .expect("spawn child test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "{name} failed in its child process ({}):\n{stdout}\n{stderr}",
            output.status
        );
        assert!(
            stdout.contains("1 passed"),
            "{name} did not run in its child process:\n{stdout}"
        );
    }
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
                let recovered = unpremultiply_channel(premultiplied as u8, alpha as u8);
                assert!(recovered >= previous, "not monotone at alpha={alpha}");
                previous = recovered;
            }
            // A channel at full coverage recovers to full scale.
            assert_eq!(
                unpremultiply_channel(alpha as u8, alpha as u8),
                255,
                "alpha={alpha}"
            );
        }
        assert_eq!(unpremultiply_channel(0, 0), 0);
    }

    #[test]
    fn unpremultiply_matches_recorded_float_pipeline_samples() {
        // Two pairs where an exact rational round — either half-up or half-even —
        // disagrees with what the f32 pipeline produces.
        assert_eq!(unpremultiply_channel(1, 6), 42);
        assert_eq!(unpremultiply_channel(3, 10), 77);
    }

    /// Every reachable (premultiplied, alpha) pair, alpha-major, pinned as one
    /// digest so any change to the single-precision pipeline is caught.
    #[test]
    fn unpremultiply_over_every_reachable_pair_is_pinned() {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        let mut pairs = 0usize;
        for alpha in 0..=255u8 {
            for premultiplied in 0..=alpha {
                hasher.update([unpremultiply_channel(premultiplied, alpha)]);
                pairs += 1;
            }
        }
        assert_eq!(pairs, 32_896);
        assert_eq!(
            hex::encode(hasher.finalize()),
            "1d3633f72ad78c52e63a96ab7c21d5251f80fa68681161abb4e7f7aa68a9663e"
        );
    }

    #[test]
    fn premultiply_is_the_left_inverse_of_unpremultiply() {
        // Round-tripping a valid premultiplied value must return it unchanged,
        // which is what keeps the shape-atlas source hash stable.
        for alpha in 0..=255u32 {
            for premultiplied in 0..=alpha {
                let recovered = unpremultiply_channel(premultiplied as u8, alpha as u8);
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
