//! The detailed (type 12) and tabbed (type 16) music clear panels.

use super::nodes::{layout_rect, localized_source, shape_node, text_node, RecipeLayer};
use super::{
    GeneralFill, GeneralGeometry, GeneralRecipeNode, GeneralRecipePayload, GeneralTextAlign,
};
use crate::profile_layout::{MUSIC_CLEAR, MUSIC_CLEAR_TAB};
use crate::profile_scene::{
    ComponentControlSource, ComponentControlState, ProfileComponentSnapshot, ProfileResolveError,
};
use crate::{
    CommandControlBinding, InteractionRegionSource, ParameterValue, Rect, StableId, TextSource,
};

/// Solid label colours for the six music difficulties, in difficulty order:
/// easy, normal, hard, expert, master, append.
///
/// Components that draw the append label as a gradient still index this table
/// and then replace the fill, so the append entry is only what a solid append
/// label would be.
const DIFFICULTY_LABEL_COLORS: [[f32; 4]; 6] = [
    [0.000, 0.859, 0.451, 1.0],
    [0.149, 0.792, 0.996, 1.0],
    [0.996, 0.788, 0.000, 1.0],
    [0.996, 0.239, 0.447, 1.0],
    [0.788, 0.169, 1.000, 1.0],
    [0.843, 0.741, 1.000, 1.0],
];

pub(super) fn build_detailed_music_recipe(
    layer_id: StableId,
    source_key: &str,
    snapshot: &ProfileComponentSnapshot,
    nodes: &mut Vec<GeneralRecipeNode>,
) -> Result<(), ProfileResolveError> {
    let layer = RecipeLayer {
        layer_id,
        source_key,
    };
    let results = snapshot.music_results.clone().unwrap_or_default();
    let difficulties = [
        ("easy", results.easy),
        ("normal", results.normal),
        ("hard", results.hard),
        ("expert", results.expert),
        ("master", results.master),
        ("append", results.append),
    ];
    let colors = DIFFICULTY_LABEL_COLORS;
    let groups = [
        (
            "clear",
            "custom_profile.general.music.clear",
            0usize,
            1usize,
            "liveClear",
        ),
        (
            "full-combo",
            "custom_profile.general.music.full_combo",
            2,
            3,
            "fullCombo",
        ),
        (
            "all-perfect",
            "custom_profile.general.music.all_perfect",
            4,
            5,
            "allPerfect",
        ),
    ];
    let mut ordinal = 0u32;
    for (group_index, (group_role, heading_key, bubble_index, bar_index, field_name)) in
        groups.iter().enumerate()
    {
        let bubble = MUSIC_CLEAR.elements[*bubble_index];
        nodes.push(shape_node(
            layer.key(&format!("{group_role}-heading-background"), ordinal),
            layout_rect(bubble),
            GeneralGeometry::RoundedRect {
                radius: [bubble.h / 2.0, bubble.h / 2.0],
            },
            [0.737, 0.737, 0.753, 1.0],
        ));
        ordinal += 1;
        nodes.push(text_node(
            layer.key(&format!("{group_role}-heading"), ordinal),
            layout_rect(bubble),
            localized_source(snapshot, heading_key)?,
            bubble.h * 0.42,
            [1.0; 4],
            GeneralTextAlign::Center,
        ));
        ordinal += 1;
        let bar = MUSIC_CLEAR.elements[*bar_index];
        let column_width = bar.w / 6.0;
        let label_center_y = bar.cy + bar.h * 0.24;
        let number_center_y = bar.cy - bar.h * 0.26;
        for (difficulty_index, (difficulty_role, values)) in difficulties.iter().enumerate() {
            let center_x = bar.cx - bar.w / 2.0 + column_width * (difficulty_index as f32 + 0.5);
            let label = crate::profile_layout::ElementLayout {
                cx: center_x,
                cy: label_center_y,
                w: column_width * 0.9,
                h: 28.0,
            };
            nodes.push(shape_node(
                layer.key(
                    &format!("{group_role}-{difficulty_role}-label-background"),
                    ordinal,
                ),
                layout_rect(label),
                GeneralGeometry::RoundedRect { radius: [8.0, 8.0] },
                colors[difficulty_index],
            ));
            ordinal += 1;
            nodes.push(text_node(
                layer.key(&format!("{group_role}-{difficulty_role}-label"), ordinal),
                layout_rect(label),
                localized_source(
                    snapshot,
                    &format!("custom_profile.general.music.difficulty.{difficulty_role}"),
                )?,
                28.0 * 0.55,
                [1.0; 4],
                GeneralTextAlign::Center,
            ));
            ordinal += 1;
            let number = crate::profile_layout::ElementLayout {
                cx: center_x,
                cy: number_center_y,
                w: column_width,
                h: 22.0,
            };
            let value = match group_index {
                0 => values.clear,
                1 => values.full_combo,
                _ => values.all_perfect,
            };
            nodes.push(text_node(
                layer.key(&format!("{group_role}-{difficulty_role}-count"), ordinal),
                layout_rect(number),
                TextSource::ProfileField {
                    field: format!("userMusicDifficultyClearCount.{difficulty_role}.{field_name}"),
                    value: value.to_string(),
                },
                22.0,
                [0.2, 0.2, 0.2, 1.0],
                GeneralTextAlign::Center,
            ));
            ordinal += 1;
        }
    }
    Ok(())
}

pub(super) fn build_tabbed_music_recipe(
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
    let control_id = StableId::derive(
        "general-control-v1",
        format!("{source_key}\0music-result-mode").as_bytes(),
    );
    let options = ["clear", "full_combo", "all_perfect"];
    controls.push(ComponentControlSource {
        id: control_id,
        layer_id,
        role: "music-result-mode".into(),
        state: ComponentControlState::Tabs {
            options: options.iter().map(|value| (*value).into()).collect(),
            active: "clear".into(),
        },
    });
    let results = snapshot.music_results.clone().unwrap_or_default();
    let difficulties = [
        ("easy", 1usize, 2usize, results.easy),
        ("normal", 3, 4, results.normal),
        ("hard", 5, 6, results.hard),
        ("expert", 7, 8, results.expert),
        ("master", 9, 10, results.master),
        ("append", 12, 13, results.append),
    ];
    let colors = DIFFICULTY_LABEL_COLORS;
    let mut ordinal = 0u32;
    for (difficulty_index, (role, label_index, number_index, values)) in
        difficulties.iter().enumerate()
    {
        let label = MUSIC_CLEAR_TAB.elements[*label_index];
        let mut background = shape_node(
            layer.key(&format!("{role}-label-background"), ordinal),
            layout_rect(label),
            GeneralGeometry::RoundedRect { radius: [8.0, 8.0] },
            colors[difficulty_index],
        );
        if *role == "append" {
            background.payload = GeneralRecipePayload::Shape {
                geometry: GeneralGeometry::RoundedRect { radius: [8.0, 8.0] },
                fill: GeneralFill::LinearGradient {
                    start: [0.0, 0.5],
                    end: [1.0, 0.5],
                    colors: vec![
                        [206.0 / 255.0, 191.0 / 255.0, 1.0, 1.0],
                        [233.0 / 255.0, 182.0 / 255.0, 252.0 / 255.0, 1.0],
                    ],
                    stops: vec![0.0, 1.0],
                },
                stroke: None,
            };
        }
        nodes.push(background);
        ordinal += 1;
        nodes.push(text_node(
            layer.key(&format!("{role}-label"), ordinal),
            layout_rect(label),
            localized_source(
                snapshot,
                &format!("custom_profile.general.music.difficulty.{role}"),
            )?,
            label.h * 0.55,
            [1.0; 4],
            GeneralTextAlign::Center,
        ));
        ordinal += 1;
        let number = MUSIC_CLEAR_TAB.elements[*number_index];
        for option in options {
            let (field, value) = match option {
                "clear" => ("liveClear", values.clear),
                "full_combo" => ("fullCombo", values.full_combo),
                _ => ("allPerfect", values.all_perfect),
            };
            let mut node = text_node(
                layer.key(&format!("{role}-{option}-count"), ordinal),
                layout_rect(number),
                TextSource::ProfileField {
                    field: format!("userMusicDifficultyClearCount.{role}.{field}"),
                    value: value.to_string(),
                },
                number.h,
                [0.2, 0.2, 0.2, 1.0],
                GeneralTextAlign::Center,
            );
            node.control_bindings
                .push(CommandControlBinding::TabOption {
                    control_id,
                    value: option.into(),
                });
            nodes.push(node);
            ordinal += 1;
        }
    }
    let separator = MUSIC_CLEAR_TAB.elements[11];
    nodes.push(shape_node(
        layer.key("append-separator", ordinal),
        Rect {
            x: separator.cx - 1.0,
            y: -separator.cy - separator.h / 2.0,
            width: 2.0,
            height: separator.h,
        },
        GeneralGeometry::Rect,
        [0.78, 0.78, 0.78, 0.6],
    ));
    ordinal += 1;
    let tab = MUSIC_CLEAR_TAB.elements[0];
    nodes.push(shape_node(
        layer.key("tab-background", ordinal),
        layout_rect(tab),
        GeneralGeometry::RoundedRect {
            radius: [tab.h / 2.0, tab.h / 2.0],
        },
        [0.737, 0.737, 0.753, 1.0],
    ));
    ordinal += 1;
    let segment_width = tab.w / 3.0;
    let tab_keys = [
        "custom_profile.general.music.clear",
        "custom_profile.general.music.full_combo",
        "custom_profile.general.music.all_perfect",
    ];
    for (active_index, active) in options.iter().enumerate() {
        let selected = crate::profile_layout::ElementLayout {
            cx: tab.cx - tab.w / 2.0 + segment_width * (active_index as f32 + 0.5),
            cy: tab.cy,
            w: segment_width,
            h: tab.h,
        };
        let mut selected_node = shape_node(
            layer.key(&format!("tab-{active}-selected"), ordinal),
            layout_rect(selected),
            GeneralGeometry::RoundedRect {
                radius: [tab.h / 2.0, tab.h / 2.0],
            },
            [1.0, 1.0, 1.0, 0.9],
        );
        selected_node
            .control_bindings
            .push(CommandControlBinding::TabOption {
                control_id,
                value: (*active).into(),
            });
        nodes.push(selected_node);
        ordinal += 1;
        for (label_index, (label_option, key)) in options.iter().zip(tab_keys).enumerate() {
            let label = crate::profile_layout::ElementLayout {
                cx: tab.cx - tab.w / 2.0 + segment_width * (label_index as f32 + 0.5),
                cy: tab.cy,
                w: segment_width,
                h: tab.h,
            };
            let mut text = text_node(
                layer.key(&format!("tab-{active}-{label_option}-label"), ordinal),
                layout_rect(label),
                localized_source(snapshot, key)?,
                tab.h * 0.42,
                if active == label_option {
                    [0.2, 0.2, 0.2, 1.0]
                } else {
                    [1.0; 4]
                },
                GeneralTextAlign::Center,
            );
            text.control_bindings
                .push(CommandControlBinding::TabOption {
                    control_id,
                    value: (*active).into(),
                });
            nodes.push(text);
            ordinal += 1;
        }
    }
    for (index, (option, key)) in options.iter().zip(tab_keys).enumerate() {
        let bounds = Rect {
            x: tab.cx - tab.w / 2.0 + segment_width * index as f32,
            y: -tab.cy - tab.h / 2.0,
            width: segment_width,
            height: tab.h,
        };
        interaction_regions.push(InteractionRegionSource {
            id: StableId::derive(
                "general-interaction-v1",
                format!("{source_key}\0music-tab\0{option}").as_bytes(),
            ),
            layer_id,
            role: format!("music-tab-{option}"),
            bounds,
            quad: crate::profile_scene::rect_quad(bounds),
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            hit_geometry: crate::profile_scene::rect_quad(bounds),
            clip: Some(crate::profile_scene::rect_quad(layout_rect(tab))),
            control_bindings: Vec::new(),
            resolved_data: std::collections::BTreeMap::from([
                ("action".into(), ParameterValue::Text("set_tab".into())),
                (
                    "control_id".into(),
                    ParameterValue::Text(format!("{:016x}", control_id.0)),
                ),
                ("value".into(), ParameterValue::Text((*option).into())),
                ("label_key".into(), ParameterValue::Text(key.into())),
            ]),
            capabilities: vec!["inspect".into(), "activate".into()],
        });
    }
    Ok(())
}
