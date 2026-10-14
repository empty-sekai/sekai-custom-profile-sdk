//! The honor panel (type 6): standard and bonds honors and their stars.

use super::nodes::{group_node, item_region, shape_node, text_node, NodeKey, RecipeLayer};
use super::{
    GeneralGeometry, GeneralGroupPhase, GeneralImageSampling, GeneralImageSource,
    GeneralImageSourceConstraint, GeneralRecipeNode, GeneralRecipePayload, GeneralTextAlign,
};
use crate::profile_scene::{
    HonorVisualKind, HonorVisualSnapshot, ProfileComponentSnapshot, ResourceDescriptor,
};
use crate::{BlendMode, InteractionRegionSource, ParameterValue, Rect, StableId, TextSource};

pub(super) fn build_honors_recipe(
    layer_id: StableId,
    source_key: &str,
    snapshot: &ProfileComponentSnapshot,
    nodes: &mut Vec<GeneralRecipeNode>,
    interaction_regions: &mut Vec<InteractionRegionSource>,
) {
    let layer = RecipeLayer {
        layer_id,
        source_key,
    };
    nodes.push(shape_node(
        layer.key("honors-background", 0),
        Rect {
            x: -392.0,
            y: -90.5,
            width: 788.0,
            height: 179.0,
        },
        GeneralGeometry::RoundedRect {
            radius: [12.0, 12.0],
        },
        [0.53, 0.53, 0.53, 0.25],
    ));
    let centers = [(-188.0, 0.0), (101.0, 0.0), (288.0, -1.0)];
    for (slot, honor) in snapshot.honor_slots.iter().take(3).enumerate() {
        let (cx, cy) = centers[slot];
        let width = if slot == 0 { 380.0 } else { 180.0 };
        let bounds = Rect {
            x: cx - width / 2.0,
            y: cy - 40.0,
            width,
            height: 80.0,
        };
        let base = 1 + slot as u32 * 64;
        match &honor.visual {
            HonorVisualKind::Standard { .. } => {
                build_standard_honor_recipe(layer, honor, cx, cy, bounds, base, nodes);
            }
            HonorVisualKind::Bonds { .. } => {
                build_bonds_honor_recipe(layer, honor, cx, cy, bounds, base, nodes);
            }
        }
        let mut data = std::collections::BTreeMap::from([
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
        ]);
        if let HonorVisualKind::Bonds {
            character_ids,
            word,
            ..
        } = &honor.visual
        {
            data.insert(
                "character_id_0".into(),
                ParameterValue::I64(character_ids[0].into()),
            );
            data.insert(
                "character_id_1".into(),
                ParameterValue::I64(character_ids[1].into()),
            );
            if let Some(word) = word {
                if let Some(ParameterValue::I64(id)) = word.provenance.get("id") {
                    data.insert("word_id".into(), ParameterValue::I64(*id));
                }
            }
        }
        interaction_regions.push(item_region(
            layer_id,
            source_key,
            &format!("honor-slot-{slot}"),
            bounds,
            data,
        ));
    }
}

/// Background, frame, overlay and level stars of a standard honor. A live
/// master honor shows its mission progress in place of the stars.
fn build_standard_honor_recipe(
    layer: RecipeLayer<'_>,
    honor: &HonorVisualSnapshot,
    cx: f32,
    cy: f32,
    bounds: Rect,
    base: u32,
    nodes: &mut Vec<GeneralRecipeNode>,
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
        unreachable!("standard honor recipe received a bonds honor");
    };
    if let Some(value) = background {
        nodes.push(honor_image_node(
            layer.key(&format!("honor-{}-background", honor.honor_id), base),
            bounds,
            value,
            GeneralImageSource::WholeImageExplicit,
            GeneralImageSourceConstraint::Fast,
            BlendMode::SrcOver,
        ));
    }
    if let Some(value) = frame_candidates.iter().flatten().next() {
        nodes.push(honor_image_node(
            layer.key(&format!("honor-{}-frame", honor.honor_id), base + 1),
            Rect {
                x: cx - value.natural_width / 2.0,
                y: cy - 40.0,
                width: value.natural_width,
                height: value.natural_height,
            },
            value,
            GeneralImageSource::WholeImageExplicit,
            GeneralImageSourceConstraint::Fast,
            BlendMode::SrcOver,
        ));
    }
    if let Some(value) = overlay {
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
        } else if (honor.full_size && value.natural_width == 380.0)
            || (!honor.full_size && value.natural_height == 80.0)
        {
            (0.0, 0.0)
        } else if honor.full_size {
            (190.0, 0.0)
        } else {
            (34.0, 42.0)
        };
        nodes.push(honor_image_node(
            layer.key(&format!("honor-{}-overlay", honor.honor_id), base + 2),
            Rect {
                x: bounds.x + dx,
                y: bounds.y + dy,
                width: value.natural_width,
                height: value.natural_height,
            },
            value,
            GeneralImageSource::WholeImageExplicit,
            GeneralImageSourceConstraint::Fast,
            BlendMode::SrcOver,
        ));
    }
    if *is_live_master {
        let font_size = 20.0;
        let baseline = cy + 30.0;
        nodes.push(text_node(
            layer.key(&format!("honor-{}-progress", honor.honor_id), base + 3),
            Rect {
                x: cx + if honor.full_size { 40.0 } else { -40.0 },
                y: baseline - font_size * 0.35 - font_size / 2.0,
                width: 80.0,
                height: font_size,
            },
            TextSource::ProfileField {
                field: format!("userHonorMissions.{}", honor.honor_id),
                value: progress.to_string(),
            },
            font_size,
            [1.0; 4],
            GeneralTextAlign::Center,
        ));
    } else if *has_star {
        build_honor_star_nodes(
            layer,
            honor,
            star.as_ref(),
            star_high.as_ref(),
            bounds,
            base + 16,
            nodes,
        );
    }
}

/// The two character halves of a bonds honor, isolated and masked as one
/// group, then its frame, word art and level stars.
fn build_bonds_honor_recipe(
    layer: RecipeLayer<'_>,
    honor: &HonorVisualSnapshot,
    cx: f32,
    cy: f32,
    bounds: Rect,
    base: u32,
    nodes: &mut Vec<GeneralRecipeNode>,
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
        unreachable!("bonds honor recipe received a standard honor");
    };
    nodes.push(group_node(
        layer.key(&format!("bonds-{}-isolation-begin", honor.honor_id), base),
        bounds,
        GeneralGroupPhase::Begin,
    ));
    for (index, value) in backgrounds.iter().enumerate() {
        if let Some(value) = value {
            let source = if index == 0 {
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: value.natural_width / 2.0,
                    height: value.natural_height,
                }
            } else {
                Rect {
                    x: value.natural_width / 2.0,
                    y: 0.0,
                    width: value.natural_width - value.natural_width / 2.0,
                    height: value.natural_height,
                }
            };
            nodes.push(honor_image_node(
                layer.key(
                    &format!("bonds-{}-background-{index}", honor.honor_id),
                    base + 1 + index as u32,
                ),
                Rect {
                    x: bounds.x + index as f32 * bounds.width / 2.0,
                    y: bounds.y,
                    width: bounds.width / 2.0,
                    height: bounds.height,
                },
                value,
                GeneralImageSource::Pixels(source),
                GeneralImageSourceConstraint::Strict,
                BlendMode::SrcOver,
            ));
        }
    }
    let offset = if honor.full_size { 120.0 } else { 30.0 };
    for (index, value) in characters.iter().enumerate() {
        if let Some(value) = value {
            let sw = value.natural_width * 0.8;
            let sh = value.natural_height * 0.8;
            let raw_left = cx + if index == 0 { -offset } else { offset } - sw / 2.0;
            let left = if index == 0 {
                raw_left
            } else {
                raw_left.max(cx)
            };
            let right = if index == 0 {
                (raw_left + sw).min(cx)
            } else {
                raw_left + sw
            };
            let draw_width = (right - left).max(0.0);
            let src_x = (left - raw_left) / 0.8;
            let source = Rect {
                x: src_x,
                y: 0.0,
                width: draw_width / 0.8,
                height: value.natural_height,
            };
            nodes.push(honor_image_node(
                layer.key(
                    &format!("bonds-{}-character-{index}", honor.honor_id),
                    base + 3 + index as u32,
                ),
                Rect {
                    x: left,
                    y: cy + 40.0 - sh,
                    width: draw_width,
                    height: sh,
                },
                value,
                GeneralImageSource::Pixels(source),
                GeneralImageSourceConstraint::Strict,
                BlendMode::SrcOver,
            ));
        }
    }
    if let Some(value) = mask {
        nodes.push(honor_image_node(
            layer.key(&format!("bonds-{}-mask", honor.honor_id), base + 5),
            bounds,
            value,
            GeneralImageSource::WholeImageImplicit,
            GeneralImageSourceConstraint::Implicit,
            BlendMode::DstIn,
        ));
    }
    nodes.push(group_node(
        layer.key(&format!("bonds-{}-isolation-end", honor.honor_id), base + 6),
        bounds,
        GeneralGroupPhase::End,
    ));
    if let Some(value) = frame {
        nodes.push(honor_image_node(
            layer.key(&format!("bonds-{}-frame", honor.honor_id), base + 7),
            Rect {
                x: cx - value.natural_width / 2.0,
                y: cy - 40.0,
                width: value.natural_width,
                height: value.natural_height,
            },
            value,
            GeneralImageSource::WholeImageExplicit,
            GeneralImageSourceConstraint::Fast,
            BlendMode::SrcOver,
        ));
    }
    if let Some(value) = word {
        nodes.push(honor_image_node(
            layer.key(&format!("bonds-{}-word", honor.honor_id), base + 8),
            Rect {
                x: cx - value.natural_width / 2.0,
                y: cy - value.natural_height / 2.0,
                width: value.natural_width,
                height: value.natural_height,
            },
            value,
            GeneralImageSource::WholeImageExplicit,
            GeneralImageSourceConstraint::Fast,
            BlendMode::SrcOver,
        ));
    }
    build_honor_star_nodes(
        layer,
        honor,
        star.as_ref(),
        star_high.as_ref(),
        bounds,
        base + 16,
        nodes,
    );
}

fn build_honor_star_nodes(
    layer: RecipeLayer<'_>,
    honor: &HonorVisualSnapshot,
    star: Option<&ResourceDescriptor>,
    star_high: Option<&ResourceDescriptor>,
    bounds: Rect,
    base: u32,
    nodes: &mut Vec<GeneralRecipeNode>,
) {
    let level = crate::masterdata::honor_level_star_count(honor.honor_level);
    let x = if honor.full_size {
        bounds.x + 54.0
    } else {
        bounds.x + bounds.width / 2.0 - 40.0
    };
    let y = bounds.y + 63.0;
    if let Some(value) = star {
        for index in 0..level.min(5) {
            nodes.push(honor_image_node(
                layer.key(
                    &format!("honor-{}-star-{index}", honor.honor_id),
                    base + index as u32,
                ),
                Rect {
                    x: x + index as f32 * 16.0,
                    y,
                    width: value.natural_width,
                    height: value.natural_height,
                },
                value,
                GeneralImageSource::WholeImageExplicit,
                GeneralImageSourceConstraint::Fast,
                BlendMode::SrcOver,
            ));
        }
    }
    if level > 5 {
        if let Some(value) = star_high {
            for index in 0..(level - 5) {
                nodes.push(honor_image_node(
                    layer.key(
                        &format!("honor-{}-star-high-{index}", honor.honor_id),
                        base + 8 + index as u32,
                    ),
                    Rect {
                        x: x + index as f32 * 16.0,
                        y,
                        width: value.natural_width,
                        height: value.natural_height,
                    },
                    value,
                    GeneralImageSource::WholeImageExplicit,
                    GeneralImageSourceConstraint::Fast,
                    BlendMode::SrcOver,
                ));
            }
        }
    }
}

/// Honor sprite drawn from its resolved descriptor with nearest filtering.
fn honor_image_node(
    key: NodeKey<'_>,
    bounds: Rect,
    descriptor: &ResourceDescriptor,
    source: GeneralImageSource,
    source_constraint: GeneralImageSourceConstraint,
    blend_mode: BlendMode,
) -> GeneralRecipeNode {
    key.node(
        bounds,
        GeneralRecipePayload::Image {
            resource: descriptor.resource.clone(),
            natural_size: Some([descriptor.natural_width, descriptor.natural_height]),
            source,
            sampling: GeneralImageSampling::Nearest,
            source_constraint,
            tint: [1.0; 4],
            source_field: None,
            source_id: None,
            provenance: descriptor.provenance.clone(),
            blend_mode,
        },
    )
}
