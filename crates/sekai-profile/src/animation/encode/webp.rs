//! Animated WebP encoding.

use super::checked_frame;
use crate::animation::AnimationEncodeSpec;

pub(super) fn encode_webp<F>(
    spec: &AnimationEncodeSpec,
    expected: usize,
    frame_at: &mut F,
) -> Result<Vec<u8>, String>
where
    F: FnMut(u32) -> Result<Vec<u8>, String>,
{
    use webp_animation::prelude::*;
    let mut encoder = Encoder::new_with_options(
        (spec.width, spec.height),
        EncoderOptions {
            anim_params: webp_animation::AnimParams {
                loop_count: if spec.looped { 0 } else { 1 },
            },
            ..Default::default()
        },
    )
    .map_err(|error| format!("WebP encoder init: {error}"))?;
    for index in 0..spec.frame_count {
        let rgba = checked_frame(frame_at, index, expected)?;
        let timestamp = index as i32 * 1000 / spec.fps as i32;
        encoder
            .add_frame(&rgba, timestamp)
            .map_err(|error| format!("WebP frame {index}: {error}"))?;
    }
    let final_timestamp = spec.frame_count as i32 * 1000 / spec.fps as i32;
    let data = encoder
        .finalize(final_timestamp)
        .map_err(|error| format!("WebP finalize: {error}"))?;
    Ok(data.to_vec())
}
