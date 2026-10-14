//! The character rank panel (types 11 and 15) and its mode tabs.

use super::nodes::{
    image_node, item_region, layout_rect, localized_source, shape_node, text_node, RecipeLayer,
};
use super::{
    GeneralClip, GeneralGeometry, GeneralImageSource, GeneralImageSourceConstraint,
    GeneralRecipeNode, GeneralTextAlign,
};
use crate::profile_scene::{
    ComponentControlSource, ComponentControlState, ProfileComponentSnapshot, ProfileResolveError,
};
use crate::{
    CommandControlBinding, InteractionRegionSource, ParameterValue, Rect, StableId, TextSource,
};

pub(super) fn build_character_rank_recipe(
    general_type: i32,
    layer_id: StableId,
    source_key: &str,
    snapshot: &ProfileComponentSnapshot,
    nodes: &mut Vec<GeneralRecipeNode>,
    controls: &mut Vec<ComponentControlSource>,
    interaction_regions: &mut Vec<InteractionRegionSource>,
) -> Result<(), ProfileResolveError> {
    let layer = RecipeLayer {
        layer_id,
        source_key,
    };
    let shift_y = if general_type == 15 { 150.0 } else { 0.0 };
    let shifted = |layout: crate::profile_layout::ElementLayout| {
        let mut bounds = layout_rect(layout);
        bounds.y += shift_y;
        bounds
    };
    let panel = &crate::profile_layout::CHAR_RANK;
    let active = panel.elements[0];
    let inactive = panel.elements[1];
    let tab_id = crate::profile_scene::component_control_id(source_key, "character-rank-mode");
    let pill_left = (active.cx - active.w / 2.0).min(inactive.cx - inactive.w / 2.0);
    let pill_right = (active.cx + active.w / 2.0).max(inactive.cx + inactive.w / 2.0);
    nodes.push(shape_node(
        layer.key("tabs-background", 0),
        shifted(crate::profile_layout::ElementLayout {
            cx: (pill_left + pill_right) / 2.0,
            cy: active.cy,
            w: pill_right - pill_left,
            h: active.h,
        }),
        GeneralGeometry::RoundedRect {
            radius: [active.h / 2.0; 2],
        },
        [0.88, 0.88, 0.88, 0.7],
    ));
    for (ordinal, role, option, layout) in [
        (1, "tab-selected-character-rank", "character_rank", active),
        (
            2,
            "tab-selected-challenge-live-rank",
            "challenge_live_rank",
            inactive,
        ),
    ] {
        let mut node = shape_node(
            layer.key(role, ordinal),
            shifted(layout),
            GeneralGeometry::RoundedRect {
                radius: [layout.h / 2.0; 2],
            },
            [1.0, 1.0, 1.0, 0.9],
        );
        node.control_bindings
            .push(CommandControlBinding::TabOption {
                control_id: tab_id,
                value: option.into(),
            });
        nodes.push(node);
    }
    for (ordinal, role, key, layout, color, option) in [
        (
            3,
            "character-rank-label-active",
            "custom_profile.general.character_rank.title",
            active,
            [0.2, 0.2, 0.2, 1.0],
            "character_rank",
        ),
        (
            4,
            "character-rank-label-inactive",
            "custom_profile.general.character_rank.title",
            active,
            [0.5, 0.5, 0.5, 1.0],
            "challenge_live_rank",
        ),
        (
            5,
            "challenge-rank-label-inactive",
            "custom_profile.general.character_rank.challenge",
            inactive,
            [0.5, 0.5, 0.5, 1.0],
            "character_rank",
        ),
        (
            6,
            "challenge-rank-label-active",
            "custom_profile.general.character_rank.challenge",
            inactive,
            [0.2, 0.2, 0.2, 1.0],
            "challenge_live_rank",
        ),
    ] {
        let mut node = text_node(
            layer.key(role, ordinal),
            shifted(layout),
            localized_source(snapshot, key)?,
            layout.h * 0.42,
            color,
            GeneralTextAlign::Center,
        );
        node.control_bindings
            .push(CommandControlBinding::TabOption {
                control_id: tab_id,
                value: option.into(),
            });
        nodes.push(node);
    }
    controls.push(ComponentControlSource {
        id: tab_id,
        layer_id,
        role: "character-rank-mode".into(),
        state: ComponentControlState::Tabs {
            options: vec!["character_rank".into(), "challenge_live_rank".into()],
            active: "character_rank".into(),
        },
    });
    for (index, (role, value, layout)) in [
        ("character-rank", "character_rank", active),
        ("challenge-live-rank", "challenge_live_rank", inactive),
    ]
    .into_iter()
    .enumerate()
    {
        let mut region = item_region(
            layer_id,
            source_key,
            role,
            shifted(layout),
            std::collections::BTreeMap::from([
                (
                    "control_id".into(),
                    ParameterValue::Text(format!("{:016x}", tab_id.0)),
                ),
                ("action".into(), ParameterValue::Text("set_tab".into())),
                ("value".into(), ParameterValue::Text(value.into())),
                ("index".into(), ParameterValue::I64(index as i64)),
            ]),
        );
        region.capabilities = vec!["inspect".into(), "activate".into()];
        interaction_regions.push(region);
    }
    let content_start = nodes.len();
    let item_region_start = interaction_regions.len();
    let mut ordinal = 16u32;
    let groups: &[&[i32]] = &[
        &[21, 22, 23, 24, 25, 26],
        &[1, 2, 3, 4],
        &[5, 6, 7, 8],
        &[9, 10, 11, 12],
        &[13, 14, 15, 16],
        &[17, 18, 19, 20],
    ];
    // Grid geometry: the first avatar, the rank pill sharing its left and
    // bottom edges, and the next column / next row avatars for the pitches.
    let first_avatar = panel.elements[2];
    let first_pill = panel.elements[3];
    let column_pitch = panel.elements[4].cx - first_avatar.cx;
    let row_pitch = first_avatar.cy - panel.elements[5].cy;
    let mut slot = 0usize;
    for group in groups {
        for character_id in *group {
            let Some(rank) = snapshot
                .character_ranks
                .iter()
                .find(|rank| rank.character_id == *character_id)
            else {
                continue;
            };
            let column = slot % 4;
            let row = slot / 4;
            let cx = first_avatar.cx + column as f32 * column_pitch;
            let cy = -first_avatar.cy + row as f32 * row_pitch + shift_y;
            slot += 1;
            let radius = first_avatar.w / 2.0;
            let pill_height = first_pill.h;
            let pill_width = first_pill.w;
            let pill_bounds = Rect {
                x: cx - radius,
                y: cy + radius - pill_height,
                width: pill_width,
                height: pill_height,
            };
            nodes.push(shape_node(
                layer.key(&format!("character-{character_id}-background"), ordinal),
                pill_bounds,
                GeneralGeometry::RoundedRect {
                    radius: [pill_height / 2.0; 2],
                },
                [0.204, 0.863, 0.996, 1.0],
            ));
            ordinal += 1;
            let avatar_bounds = Rect {
                x: cx - radius,
                y: cy - radius,
                width: radius * 2.0,
                height: radius * 2.0,
            };
            nodes.push(shape_node(
                layer.key(
                    &format!("character-{character_id}-avatar-background"),
                    ordinal,
                ),
                avatar_bounds,
                GeneralGeometry::Ellipse,
                [0.204, 0.863, 0.996, 1.0],
            ));
            ordinal += 1;
            if let Some(descriptor) = &rank.avatar.descriptor {
                nodes.push(image_node(
                    layer.key(&format!("character-{character_id}-avatar"), ordinal),
                    avatar_bounds,
                    &rank.avatar,
                    descriptor,
                    GeneralImageSource::WholeImageImplicit,
                    GeneralImageSourceConstraint::Implicit,
                    vec![GeneralClip::Ellipse {
                        bounds: avatar_bounds,
                        anti_alias: true,
                    }],
                ));
            } else {
                nodes.push(shape_node(
                    layer.key(
                        &format!("character-{character_id}-avatar-placeholder"),
                        ordinal,
                    ),
                    avatar_bounds,
                    GeneralGeometry::Ellipse,
                    [0.85, 0.85, 0.85, 0.4],
                ));
            }
            ordinal += 1;
            // A character without a cleared challenge stage shows stage 1.
            let challenge_rank = rank.challenge_rank.unwrap_or(1);
            let number_center_x = cx + radius + (137.0 - radius - pill_height / 2.0) / 2.0;
            let number_bounds = Rect {
                x: number_center_x - (137.0 - radius) / 2.0,
                y: cy + radius - pill_height / 2.0 - 11.0,
                width: 137.0 - radius,
                height: 22.0,
            };
            for (mode, field, value) in [
                ("character_rank", "characterRanks", rank.rank),
                (
                    "challenge_live_rank",
                    "challengeLiveSoloStages",
                    challenge_rank,
                ),
            ] {
                let mut node = text_node(
                    layer.key(&format!("character-{character_id}-{mode}"), ordinal),
                    number_bounds,
                    TextSource::ProfileField {
                        field: format!("userProfile.{field}.{character_id}.rank"),
                        value: value.to_string(),
                    },
                    22.0,
                    [0.0, 0.0, 0.0, 1.0],
                    GeneralTextAlign::Center,
                );
                node.control_bindings
                    .push(CommandControlBinding::TabOption {
                        control_id: tab_id,
                        value: mode.into(),
                    });
                nodes.push(node);
                ordinal += 1;
            }
            interaction_regions.push(item_region(
                layer_id,
                source_key,
                &format!("character-{character_id}"),
                Rect {
                    x: pill_bounds.x,
                    y: avatar_bounds.y,
                    width: pill_width,
                    height: 76.0,
                },
                std::collections::BTreeMap::from([
                    (
                        "character_id".into(),
                        ParameterValue::I64((*character_id).into()),
                    ),
                    ("rank".into(), ParameterValue::I64(rank.rank.into())),
                    (
                        "challenge_rank".into(),
                        ParameterValue::I64(challenge_rank.into()),
                    ),
                ]),
            ));
        }
        if !slot.is_multiple_of(4) {
            slot = (slot / 4 + 1) * 4;
        }
    }
    if general_type == 15 {
        let viewport_top =
            (-active.cy + active.h / 2.0 + shift_y).max(-inactive.cy + inactive.h / 2.0 + shift_y);
        let viewport_bottom = 286.0;
        let viewport = Rect {
            x: -panel.w / 2.0,
            y: viewport_top,
            width: panel.w,
            height: viewport_bottom - viewport_top,
        };
        let max = (nodes[content_start..]
            .iter()
            .map(|node| node.bounds.y + node.bounds.height)
            .fold(viewport.y + viewport.height, f32::max)
            - (viewport.y + viewport.height))
            .max(0.0);
        let scroll_id =
            crate::profile_scene::component_control_id(source_key, "character-rank-scroll");
        let clip = GeneralClip::Rect {
            bounds: viewport,
            anti_alias: false,
        };
        for node in &mut nodes[content_start..] {
            node.clips.push(clip.clone());
            node.control_bindings
                .push(CommandControlBinding::ScrollContent {
                    control_id: scroll_id,
                });
        }
        for region in &mut interaction_regions[item_region_start..] {
            region.clip = Some(crate::profile_scene::rect_quad(viewport));
            region
                .control_bindings
                .push(CommandControlBinding::ScrollContent {
                    control_id: scroll_id,
                });
        }
        let track_bounds = Rect {
            x: 475.5,
            y: viewport.y,
            width: 4.0,
            height: viewport.height,
        };
        nodes.push(shape_node(
            layer.key("character-rank-scroll-track", 10_000),
            track_bounds,
            GeneralGeometry::RoundedRect { radius: [2.0; 2] },
            [0.25, 0.25, 0.25, 0.18],
        ));
        let thumb_height = (viewport.height * viewport.height / (viewport.height + max))
            .clamp(40.0, viewport.height);
        let thumb_bounds = Rect {
            height: thumb_height,
            ..track_bounds
        };
        let mut thumb = shape_node(
            layer.key("character-rank-scroll-thumb", 10_001),
            thumb_bounds,
            GeneralGeometry::RoundedRect { radius: [2.0; 2] },
            [0.25, 0.25, 0.25, 0.55],
        );
        thumb
            .control_bindings
            .push(CommandControlBinding::ScrollThumb {
                control_id: scroll_id,
            });
        nodes.push(thumb);
        let mut thumb_region = item_region(
            layer_id,
            source_key,
            "character-rank-scroll-thumb",
            thumb_bounds,
            std::collections::BTreeMap::from([
                (
                    "control_id".into(),
                    ParameterValue::Text(format!("{:016x}", scroll_id.0)),
                ),
                ("action".into(), ParameterValue::Text("scroll_thumb".into())),
            ]),
        );
        thumb_region.capabilities = vec!["inspect".into(), "scroll".into(), "drag".into()];
        thumb_region
            .control_bindings
            .push(CommandControlBinding::ScrollThumb {
                control_id: scroll_id,
            });
        interaction_regions.push(thumb_region);
        controls.push(ComponentControlSource {
            id: scroll_id,
            layer_id,
            role: "character-rank-scroll".into(),
            state: ComponentControlState::Scroll {
                offset: 0.0,
                min: 0.0,
                max,
                viewport_extent: viewport.height,
                content_extent: viewport.height + max,
                step: 105.0,
            },
        });
        let mut scroll_region = item_region(
            layer_id,
            source_key,
            "character-rank-scroll",
            viewport,
            std::collections::BTreeMap::from([
                (
                    "control_id".into(),
                    ParameterValue::Text(format!("{:016x}", scroll_id.0)),
                ),
                ("action".into(), ParameterValue::Text("scroll_by".into())),
            ]),
        );
        scroll_region.capabilities = vec!["inspect".into(), "scroll".into()];
        scroll_region
            .control_bindings
            .push(CommandControlBinding::ScrollViewport {
                control_id: scroll_id,
            });
        interaction_regions.push(scroll_region);
    }
    Ok(())
}
