//! APNG encoding with per-frame delta rectangles.

use super::checked_frame;
use crate::animation::{AnimationEncodeSpec, AnimationError};

pub(super) fn encode_apng<F>(
    spec: &AnimationEncodeSpec,
    expected: usize,
    frame_at: &mut F,
) -> Result<Vec<u8>, AnimationError>
where
    F: FnMut(u32) -> Result<Vec<u8>, AnimationError>,
{
    use flate2::{write::ZlibEncoder, Compression};
    use std::io::Write;

    let mut output = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&spec.width.to_be_bytes());
    ihdr.extend_from_slice(&spec.height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // RGBA8, deflate, no interlace.
    write_png_chunk(&mut output, b"IHDR", &ihdr);
    let mut actl = Vec::with_capacity(8);
    actl.extend_from_slice(&spec.frame_count.to_be_bytes());
    actl.extend_from_slice(&(if spec.looped { 0u32 } else { 1u32 }).to_be_bytes());
    write_png_chunk(&mut output, b"acTL", &actl);

    let mut sequence = 0u32;
    let mut previous: Option<Vec<u8>> = None;
    for index in 0..spec.frame_count {
        let rgba = checked_frame(frame_at, index, expected)?;
        let (offset_x, offset_y, frame_width, frame_height, pixels) =
            if let Some(before) = &previous {
                rgba_delta_rect(before, &rgba, spec.width, spec.height)
            } else {
                (0, 0, spec.width, spec.height, rgba.clone())
            };
        let mut fctl = Vec::with_capacity(26);
        fctl.extend_from_slice(&sequence.to_be_bytes());
        sequence += 1;
        fctl.extend_from_slice(&frame_width.to_be_bytes());
        fctl.extend_from_slice(&frame_height.to_be_bytes());
        fctl.extend_from_slice(&offset_x.to_be_bytes());
        fctl.extend_from_slice(&offset_y.to_be_bytes());
        fctl.extend_from_slice(&1u16.to_be_bytes());
        fctl.extend_from_slice(&(spec.fps as u16).to_be_bytes());
        fctl.extend_from_slice(&[0, 0]); // dispose NONE, blend SOURCE.
        write_png_chunk(&mut output, b"fcTL", &fctl);

        let mut scanlines = Vec::with_capacity(pixels.len() + frame_height as usize);
        let row_bytes = frame_width as usize * 4;
        for row in pixels.chunks_exact(row_bytes) {
            scanlines.push(0); // PNG filter NONE.
            scanlines.extend_from_slice(row);
        }
        let mut zlib = ZlibEncoder::new(Vec::new(), Compression::best());
        let compression_failed = |source: std::io::Error| AnimationError::ApngCompression {
            frame: index,
            source,
        };
        zlib.write_all(&scanlines).map_err(compression_failed)?;
        let compressed = zlib.finish().map_err(compression_failed)?;
        if index == 0 {
            write_png_chunk(&mut output, b"IDAT", &compressed);
        } else {
            let mut fdat = Vec::with_capacity(compressed.len() + 4);
            fdat.extend_from_slice(&sequence.to_be_bytes());
            sequence += 1;
            fdat.extend_from_slice(&compressed);
            write_png_chunk(&mut output, b"fdAT", &fdat);
        }
        previous = Some(rgba);
        // The stream only grows, so stop as soon as it crosses the budget.
        if output.len() > spec.output_budget_bytes {
            return Err(AnimationError::OutputBudgetExceeded {
                bytes: output.len(),
                budget: spec.output_budget_bytes,
            });
        }
    }
    write_png_chunk(&mut output, b"IEND", &[]);
    Ok(output)
}

fn write_png_chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    output.extend_from_slice(&crc.finalize().to_be_bytes());
}

fn rgba_delta_rect(
    previous: &[u8],
    current: &[u8],
    width: u32,
    height: u32,
) -> (u32, u32, u32, u32, Vec<u8>) {
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut changed = false;
    for (index, (before, after)) in previous
        .as_chunks::<4>()
        .0
        .iter()
        .zip(current.chunks_exact(4))
        .enumerate()
    {
        if before == after {
            continue;
        }
        let x = index as u32 % width;
        let y = index as u32 / width;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
        changed = true;
    }
    if !changed {
        return (0, 0, 1, 1, current[..4].to_vec());
    }
    let rect_width = max_x - min_x + 1;
    let rect_height = max_y - min_y + 1;
    let mut pixels = Vec::with_capacity(rect_width as usize * rect_height as usize * 4);
    for y in min_y..=max_y {
        let start = ((y * width + min_x) * 4) as usize;
        let end = start + rect_width as usize * 4;
        pixels.extend_from_slice(&current[start..end]);
    }
    (min_x, min_y, rect_width, rect_height, pixels)
}
