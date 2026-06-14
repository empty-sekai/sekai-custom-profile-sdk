use std::env;
use std::fs;
use std::path::PathBuf;

use allium_renderer::sdf::outline as outline_sdf;

fn main() {
    let mut args = env::args().skip(1);
    let out_dir = PathBuf::from(
        args.next()
            .expect("usage: dump_outline_sdf <out_dir> <font_family> <chars>"),
    );
    let family = args.next().expect("missing font_family");
    let chars = args.next().expect("missing chars");

    fs::create_dir_all(&out_dir).expect("create out dir failed");

    let mut manifest = String::from("[\n");
    let mut first = true;
    let chars_to_dump: Vec<char> = if let Some(rest) = chars.strip_prefix("U+") {
        rest.split(',')
            .filter(|part| !part.is_empty())
            .map(|part| {
                let value =
                    u32::from_str_radix(part, 16).unwrap_or_else(|_| panic!("bad codepoint {part}"));
                char::from_u32(value).unwrap_or_else(|| panic!("invalid codepoint U+{part}"))
            })
            .collect()
    } else {
        chars.chars().collect()
    };
    for ch in chars_to_dump {
        let glyph = outline_sdf::lookup_or_generate(Some(&family), ch)
            .unwrap_or_else(|| panic!("glyph generation failed for U+{:04X}", ch as u32));
        let file_name = format!("U+{:04X}.pgm", ch as u32);
        let mut bytes = format!("P5\n{} {}\n255\n", glyph.width(), glyph.height()).into_bytes();
        for y in 0..glyph.height() {
            for x in 0..glyph.width() {
                let gray = (glyph.sample_gray(x as f32, y as f32) * 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8;
                bytes.push(gray);
            }
        }
        fs::write(out_dir.join(&file_name), bytes).expect("write pgm failed");

        if !first {
            manifest.push_str(",\n");
        }
        first = false;
        manifest.push_str(&format!(
            "  {{\"char\":\"{}\",\"unicode\":\"U+{:04X}\",\"width\":{},\"height\":{},\"bearingX\":{},\"bearingY\":{},\"planeBearingX\":{},\"planeBearingY\":{},\"planeWidth\":{},\"planeHeight\":{},\"file\":\"{}\"}}",
            ch,
            ch as u32,
            glyph.width(),
            glyph.height(),
            glyph.bearing_x(),
            glyph.bearing_y(),
            glyph.plane_bearing_x(),
            glyph.plane_bearing_y(),
            glyph.plane_width(),
            glyph.plane_height(),
            file_name
        ));
    }
    manifest.push_str("\n]\n");
    fs::write(out_dir.join("manifest.json"), manifest).expect("write manifest failed");
}
