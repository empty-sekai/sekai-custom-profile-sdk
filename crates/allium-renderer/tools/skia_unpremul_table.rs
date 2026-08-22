//! Dumps Skia's premultiplied-to-unpremultiplied mapping for every reachable
//! (premultiplied value, alpha) pair.
//!
//! The shape-atlas source hash is taken over Skia's non-premultiplied read, so
//! the engine has to reproduce that value once Skia is gone. This records the
//! ground truth rather than assuming an algorithm: for each (sample, alpha) it
//! encodes a 1x1 PNG, lets Skia decode and read it back, and writes
//! `table[alpha * 256 + premultiplied] = skia_value` to a 65 536-byte file.
//!
//! Usage: skia-unpremul-table <out-file>

use allium_renderer::codec::{png, premultiply_channel};

fn skia_unpremul(bytes: &[u8]) -> Option<u8> {
    let data = skia_safe::Data::new_copy(bytes);
    let image = skia_safe::Image::from_encoded(data)?;
    let mut rgba = [0u8; 4];
    let info = skia_safe::ImageInfo::new(
        (1, 1),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        None,
    );
    if !image.read_pixels(
        &info,
        &mut rgba,
        4,
        (0, 0),
        skia_safe::image::CachingHint::Disallow,
    ) {
        return None;
    }
    Some(rgba[0])
}

fn main() {
    let Some(out) = std::env::args().nth(1) else {
        eprintln!("usage: skia-unpremul-table <out-file>");
        std::process::exit(2);
    };
    // 0xFF marks a pair Skia never produced, so gaps are visible rather than
    // silently reading as a valid zero.
    let mut table = vec![0xFFu8; 256 * 256];
    let mut covered = vec![false; 256 * 256];
    for alpha in 0..=255u16 {
        for value in 0..=255u16 {
            let a = alpha as u8;
            let v = value as u8;
            let Ok(encoded) = png::encode_rgba(1, 1, &[v, v, v, a]) else {
                eprintln!("encode failed at value={v} alpha={a}");
                std::process::exit(1);
            };
            let Some(read_back) = skia_unpremul(&encoded) else {
                eprintln!("skia refused value={v} alpha={a}");
                std::process::exit(1);
            };
            let premultiplied = premultiply_channel(v, a);
            let slot = usize::from(a) * 256 + usize::from(premultiplied);
            if covered[slot] && table[slot] != read_back {
                eprintln!(
                    "skia is not a function of (premultiplied, alpha): \
                     alpha={a} premultiplied={premultiplied} gave {} then {read_back}",
                    table[slot]
                );
                std::process::exit(1);
            }
            table[slot] = read_back;
            covered[slot] = true;
        }
    }
    let reachable = (0..=255usize)
        .flat_map(|a| (0..=a).map(move |p| a * 256 + p))
        .filter(|slot| covered[*slot])
        .count();
    let expected: usize = (0..=255usize).map(|a| a + 1).sum();
    println!("reachable (premultiplied, alpha) pairs covered: {reachable} / {expected}");
    std::fs::write(&out, &table).expect("write table");
    println!("wrote {out}");
}
