//! Lowering of placed honors and bonds honors to image and composite commands.

use std::collections::BTreeMap;

use super::commands::{LayerCommands, TextStyle};
use super::layer::component_interaction_region;
use super::snapshot::{HonorVisualKind, HonorVisualSnapshot, ResourceDescriptor};
use super::ProfileResolveError;
use crate::profile_layout::ElementLayout;
use crate::{InteractionRegionSource, ParameterValue, Rect, StableId};

pub(super) fn lower_honor_visual(
    commands: &mut LayerCommands<'_>,
    honor: &HonorVisualSnapshot,
    layout: ElementLayout,
) -> Result<(), ProfileResolveError> {
    match &honor.visual {
        HonorVisualKind::Standard { .. } => lower_standard_honor(commands, honor, layout),
        HonorVisualKind::Bonds { .. } => lower_bonds_honor(commands, honor, layout),
    }
    Ok(())
}

/// Background, frame, overlay and level stars of a standard honor. A live
/// master honor shows its mission progress in place of the stars.
fn lower_standard_honor(
    commands: &mut LayerCommands<'_>,
    honor: &HonorVisualSnapshot,
    layout: ElementLayout,
) {
    let HonorVisualKind::Standard {
        honor_type,
        has_star,
        is_live_master,
        progress,
        background,
        frame_candidates,
        overlay,
        star,
        star_high,
        ..
    } = &honor.visual
    else {
        unreachable!("standard honor lowering received a bonds honor");
    };
    let (w, h) = (layout.w, layout.h);
    let origin_x = layout.cx;
    let origin_y = -layout.cy;
    let full_bounds = Rect {
        x: origin_x - w / 2.0,
        y: origin_y - h / 2.0,
        width: w,
        height: h,
    };
    if let Some(background) = background {
        commands.push_image(
            &format!("honor-{}-background", honor.honor_id),
            background,
            full_bounds,
            None,
        );
    }
    if let Some(frame) = frame_candidates.iter().flatten().next() {
        let frame_bounds = Rect {
            x: origin_x - frame.natural_width / 2.0,
            y: origin_y - h / 2.0,
            width: frame.natural_width,
            height: frame.natural_height,
        };
        commands.push_image(
            &format!("honor-{}-frame", honor.honor_id),
            frame,
            frame_bounds,
            None,
        );
    }
    if let Some(overlay) = overlay {
        let (dx, dy) = if *is_live_master {
            if honor.full_size {
                (218.0, 3.0)
            } else {
                (40.0, 3.0)
            }
        } else if honor_type == "rank_match" {
            if honor.full_size {
                (190.0, 0.0)
            } else {
                (17.0, 42.0)
            }
        } else if (honor.full_size && overlay.natural_width == 380.0)
            || (!honor.full_size && overlay.natural_height == 80.0)
        {
            (0.0, 0.0)
        } else if honor.full_size {
            (190.0, 0.0)
        } else {
            (34.0, 42.0)
        };
        let overlay_bounds = Rect {
            x: origin_x - w / 2.0 + dx,
            y: origin_y - h / 2.0 + dy,
            width: overlay.natural_width,
            height: overlay.natural_height,
        };
        commands.push_image(
            &format!("honor-{}-overlay", honor.honor_id),
            overlay,
            overlay_bounds,
            None,
        );
    }
    if *is_live_master {
        let text_layout = ElementLayout {
            cx: origin_x - w / 2.0 + if honor.full_size { 270.0 } else { 90.0 },
            cy: -(origin_y - h / 2.0 + 60.0),
            w: 80.0,
            h: 20.0,
        };
        commands.push_profile_text(
            &format!("honor-{}-progress", honor.honor_id),
            &format!("userHonorMissions.{}", honor.honor_id),
            progress.to_string(),
            text_layout,
            TextStyle {
                alignment: 2,
                size: 20.0,
                color: [1.0; 4],
            },
        );
    } else if *has_star {
        lower_honor_stars(commands, honor, layout, star.as_ref(), star_high.as_ref());
    }
}

/// The two character halves of a bonds honor, isolated and masked as one
/// group, then its frame, word art and level stars.
fn lower_bonds_honor(
    commands: &mut LayerCommands<'_>,
    honor: &HonorVisualSnapshot,
    layout: ElementLayout,
) {
    let HonorVisualKind::Bonds {
        backgrounds,
        characters,
        mask,
        frame,
        word,
        star,
        star_high,
        ..
    } = &honor.visual
    else {
        unreachable!("bonds honor lowering received a standard honor");
    };
    let (w, h) = (layout.w, layout.h);
    let origin_x = layout.cx;
    let origin_y = -layout.cy;
    let group_bounds = Rect {
        x: origin_x - w / 2.0,
        y: origin_y - h / 2.0,
        width: w,
        height: h,
    };
    commands.push_composite(
        &format!("bonds-{}-isolation-begin", honor.honor_id),
        group_bounds,
        crate::CompositeOperation::BeginIsolation,
    );
    for (index, background) in backgrounds.iter().enumerate() {
        let Some(background) = background else {
            continue;
        };
        let bounds = Rect {
            x: origin_x - w / 2.0 + index as f32 * w / 2.0,
            y: origin_y - h / 2.0,
            width: w / 2.0,
            height: h,
        };
        let uv = if index == 0 {
            Rect {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            }
        } else {
            Rect {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            }
        };
        commands.push_image(
            &format!("bonds-{}-background-{index}", honor.honor_id),
            background,
            bounds,
            Some(uv),
        );
    }
    let offset = if honor.full_size { 120.0 } else { 30.0 };
    for (index, character) in characters.iter().enumerate() {
        let Some(character) = character else { continue };
        let sw = character.natural_width * 0.8;
        let sh = character.natural_height * 0.8;
        let center = origin_x + if index == 0 { -offset } else { offset };
        let raw_left = center - sw / 2.0;
        let clipped_left = if index == 0 {
            raw_left
        } else {
            raw_left.max(origin_x)
        };
        let clipped_right = if index == 0 {
            (raw_left + sw).min(origin_x)
        } else {
            raw_left + sw
        };
        let width = (clipped_right - clipped_left).max(0.0);
        let uv_x = (clipped_left - raw_left) / sw.max(f32::EPSILON);
        let uv = Rect {
            x: uv_x,
            y: 0.0,
            width: width / sw.max(f32::EPSILON),
            height: 1.0,
        };
        let bounds = Rect {
            x: clipped_left,
            y: origin_y + h / 2.0 - sh,
            width,
            height: sh,
        };
        commands.push_image(
            &format!("bonds-{}-character-{index}", honor.honor_id),
            character,
            bounds,
            Some(uv),
        );
    }
    if let Some(mask) = mask {
        commands
            .push_image(
                &format!("bonds-{}-mask", honor.honor_id),
                mask,
                group_bounds,
                None,
            )
            .blend_mode = crate::BlendMode::DstIn;
    }
    commands.push_composite(
        &format!("bonds-{}-isolation-end", honor.honor_id),
        group_bounds,
        crate::CompositeOperation::EndIsolation,
    );
    if let Some(frame) = frame {
        let frame_bounds = Rect {
            x: origin_x - frame.natural_width / 2.0,
            y: origin_y - h / 2.0,
            width: frame.natural_width,
            height: frame.natural_height,
        };
        commands.push_image(
            &format!("bonds-{}-frame", honor.honor_id),
            frame,
            frame_bounds,
            None,
        );
    }
    if let Some(word) = word {
        let bounds = Rect {
            x: origin_x - word.natural_width / 2.0,
            y: origin_y - word.natural_height / 2.0,
            width: word.natural_width,
            height: word.natural_height,
        };
        commands.push_image(
            &format!("bonds-{}-word", honor.honor_id),
            word,
            bounds,
            None,
        );
    }
    lower_honor_stars(commands, honor, layout, star.as_ref(), star_high.as_ref());
}

/// Level stars of a placed honor: the first five levels of every ten use
/// `star`, the next five draw `star_high` over them.
fn lower_honor_stars(
    commands: &mut LayerCommands<'_>,
    honor: &HonorVisualSnapshot,
    layout: ElementLayout,
    star: Option<&ResourceDescriptor>,
    star_high: Option<&ResourceDescriptor>,
) {
    let level = crate::masterdata::honor_level_star_count(honor.honor_level);
    let origin_x = layout.cx;
    let origin_y = -layout.cy;
    let base_x = if honor.full_size {
        origin_x - layout.w / 2.0 + 54.0
    } else {
        origin_x - 40.0
    };
    let base_y = origin_y - layout.h / 2.0 + 63.0;
    if let Some(star) = star {
        for index in 0..level.min(5) {
            let bounds = Rect {
                x: base_x + index as f32 * 16.0,
                y: base_y,
                width: star.natural_width,
                height: star.natural_height,
            };
            commands.push_image(
                &format!("honor-{}-star-{index}", honor.honor_id),
                star,
                bounds,
                None,
            );
        }
    }
    let Some(star_high) = star_high else {
        return;
    };
    for index in 0..(level - 5).max(0) {
        let bounds = Rect {
            x: base_x + index as f32 * 16.0,
            y: base_y,
            width: star_high.natural_width,
            height: star_high.natural_height,
        };
        commands.push_image(
            &format!("honor-{}-star-high-{index}", honor.honor_id),
            star_high,
            bounds,
            None,
        );
    }
}

pub(super) fn honor_interaction_region(
    source_key: &str,
    layer_id: StableId,
    ordinal: u32,
    layout: ElementLayout,
    honor: &HonorVisualSnapshot,
) -> InteractionRegionSource {
    component_interaction_region(
        source_key,
        layer_id,
        &format!("honor-{}", honor.honor_id),
        ordinal,
        layout,
        BTreeMap::from([
            (
                "field".into(),
                ParameterValue::Text(honor.source_field.clone()),
            ),
            (
                "honor_id".into(),
                ParameterValue::I64(honor.honor_id.into()),
            ),
            (
                "honor_level".into(),
                ParameterValue::I64(honor.honor_level.into()),
            ),
            ("full_size".into(), ParameterValue::Bool(honor.full_size)),
        ]),
        &["inspect", "select_item"],
    )
}
