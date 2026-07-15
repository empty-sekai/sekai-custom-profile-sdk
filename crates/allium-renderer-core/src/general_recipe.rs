use serde::{Deserialize, Serialize};

use crate::profile_layout::{
    CHALLENGE_LIVE, COMMENT, MUSIC_CLEAR, MUSIC_CLEAR_TAB, MVP_SUPERSTAR, PLAYER_NAME, TOTAL_POWER,
};
use crate::profile_scene::{
    ComponentControlSource, ComponentControlState, ProfileComponentSnapshot, ProfileResolveError,
};
use crate::{
    BlendMode, CommandControlBinding, FontRole, InteractionRegionSource, Matrix2d, ParameterValue,
    Rect, ResourceKey, StableId, TextSource,
};

pub const SUPPORTED_GENERAL_TYPES: [i32; 15] =
    [2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18];

pub fn general_type_requires_font(general_type: i32, has_live_master_honor: bool) -> bool {
    match general_type {
        5 | 18 => false,
        6 => has_live_master_honor,
        value => SUPPORTED_GENERAL_TYPES.contains(&value),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneralRecipe {
    pub general_type: i32,
    pub source_key: String,
    pub layer_id: StableId,
    pub nodes: Vec<GeneralRecipeNode>,
    pub controls: Vec<ComponentControlSource>,
    pub interaction_regions: Vec<InteractionRegionSource>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneralRecipeNode {
    pub id: StableId,
    pub layer_id: StableId,
    pub role: String,
    pub parent_id: Option<StableId>,
    pub bounds: Rect,
    pub local_matrix: Matrix2d,
    pub clips: Vec<GeneralClip>,
    pub control_bindings: Vec<CommandControlBinding>,
    pub payload: GeneralRecipePayload,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GeneralRecipePayload {
    Group {
        isolation: bool,
        phase: GeneralGroupPhase,
    },
    Shape {
        geometry: GeneralGeometry,
        fill: GeneralFill,
        stroke: Option<GeneralStroke>,
    },
    Text {
        source: TextSource,
        font_role: FontRole,
        font_size: f32,
        color: [f32; 4],
        align: GeneralTextAlign,
        line_spacing: f32,
        wrap: bool,
        #[serde(default)]
        render_baseline: Option<f32>,
    },
    Image {
        resource: ResourceKey,
        natural_size: Option<[f32; 2]>,
        source: GeneralImageSource,
        sampling: GeneralImageSampling,
        source_constraint: GeneralImageSourceConstraint,
        tint: [f32; 4],
        source_field: Option<String>,
        source_id: Option<String>,
        provenance: std::collections::BTreeMap<String, ParameterValue>,
        #[serde(default)]
        blend_mode: BlendMode,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneralGroupPhase {
    Begin,
    End,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum GeneralImageSource {
    WholeImageImplicit,
    WholeImageExplicit,
    Pixels(Rect),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneralImageSampling {
    Nearest,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneralImageSourceConstraint {
    Implicit,
    Fast,
    Strict,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GeneralGeometry {
    Rect,
    RoundedRect { radius: [f32; 2] },
    Ellipse,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum GeneralFill {
    Solid([f32; 4]),
    LinearGradient {
        start: [f32; 2],
        end: [f32; 2],
        colors: Vec<[f32; 4]>,
        stops: Vec<f32>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneralStroke {
    pub color: [f32; 4],
    pub width: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GeneralClip {
    Rect {
        bounds: Rect,
        anti_alias: bool,
    },
    RoundedRect {
        bounds: Rect,
        radius: [f32; 2],
        anti_alias: bool,
    },
    Ellipse {
        bounds: Rect,
        anti_alias: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneralTextAlign {
    Left,
    Center,
    Right,
}

pub fn build_general_recipe(
    general_type: i32,
    layer_id: StableId,
    source_key: &str,
    snapshot: &ProfileComponentSnapshot,
) -> Result<Option<GeneralRecipe>, ProfileResolveError> {
    if !SUPPORTED_GENERAL_TYPES.contains(&general_type) {
        return Ok(None);
    }
    let requires_font = general_type_requires_font(
        general_type,
        snapshot.honor_slots.iter().any(|honor| {
            matches!(
                honor.visual,
                crate::profile_scene::HonorVisualKind::Standard {
                    is_live_master: true,
                    ..
                }
            )
        }),
    );
    if requires_font {
        snapshot
            .region_fonts
            .get(&1)
            .ok_or(ProfileResolveError::MissingRegionFont(1))?;
    }
    let mut nodes = Vec::new();
    let mut controls = Vec::new();
    let mut interaction_regions = Vec::new();
    match general_type {
        3 => build_deck_recipe(
            layer_id,
            source_key,
            snapshot,
            &mut nodes,
            &mut interaction_regions,
        )?,
        5 => build_leader_member_recipe(
            layer_id,
            source_key,
            snapshot,
            &mut nodes,
            &mut interaction_regions,
        ),
        6 => build_honors_recipe(
            layer_id,
            source_key,
            snapshot,
            &mut nodes,
            &mut interaction_regions,
        ),
        14 => build_story_favorite_recipe(
            layer_id,
            source_key,
            snapshot,
            &mut nodes,
            &mut controls,
            &mut interaction_regions,
        )?,
        11 | 15 => build_character_rank_recipe(
            general_type,
            layer_id,
            source_key,
            snapshot,
            &mut nodes,
            &mut controls,
            &mut interaction_regions,
        )?,
        2 => {
            let title_key = "custom_profile.general.total_power.title";
            nodes.push(text_node(
                layer_id,
                source_key,
                "title",
                0,
                layout_rect(TOTAL_POWER.elements[0]),
                localized_source(snapshot, title_key)?,
                TOTAL_POWER.elements[0].h,
                [0.33, 0.33, 0.33, 1.0],
                GeneralTextAlign::Left,
                0.0,
                false,
            ));
            nodes.push(text_node(
                layer_id,
                source_key,
                "divider",
                1,
                layout_rect(TOTAL_POWER.elements[2]),
                TextSource::Authored { value: "|".into() },
                TOTAL_POWER.elements[2].h,
                [0.67, 0.67, 0.67, 1.0],
                GeneralTextAlign::Left,
                0.0,
                false,
            ));
            nodes.push(text_node(
                layer_id,
                source_key,
                "value",
                2,
                layout_rect(TOTAL_POWER.elements[3]),
                TextSource::ProfileField {
                    field: "totalPower.totalPower".into(),
                    value: snapshot.total_power.to_string(),
                },
                TOTAL_POWER.elements[3].h,
                [0.2, 0.2, 0.2, 1.0],
                GeneralTextAlign::Left,
                0.0,
                false,
            ));
        }
        13 => {
            nodes.push(shape_node(
                layer_id,
                source_key,
                "textbox",
                0,
                layout_rect(PLAYER_NAME.elements[0]),
                GeneralGeometry::RoundedRect { radius: [8.0, 8.0] },
                [0.922, 0.922, 0.941, 1.0],
            ));
            nodes.push(text_node(
                layer_id,
                source_key,
                "player-name",
                1,
                layout_rect(PLAYER_NAME.elements[1]),
                TextSource::ProfileField {
                    field: "user.name".into(),
                    value: snapshot.user_name.clone(),
                },
                PLAYER_NAME.elements[1].h,
                [0.2, 0.2, 0.2, 1.0],
                GeneralTextAlign::Left,
                0.0,
                false,
            ));
        }
        4 => {
            let textbox = COMMENT.elements[0];
            nodes.push(shape_node(
                layer_id,
                source_key,
                "textbox",
                0,
                layout_rect(textbox),
                GeneralGeometry::RoundedRect { radius: [8.0, 8.0] },
                [0.922, 0.922, 0.941, 1.0],
            ));
            let key = "custom_profile.general.comment.title";
            let value = snapshot
                .localized_text
                .get(key)
                .cloned()
                .ok_or_else(|| ProfileResolveError::MissingLocalizedText(key.into()))?;
            nodes.push(text_node(
                layer_id,
                source_key,
                "title",
                1,
                layout_rect(COMMENT.elements[1]),
                TextSource::Localized {
                    key: key.into(),
                    locale: snapshot.locale.clone(),
                    value,
                },
                COMMENT.elements[1].h,
                [0.53, 0.53, 0.53, 1.0],
                GeneralTextAlign::Center,
                0.0,
                false,
            ));
            let content = crate::profile_layout::ElementLayout {
                cx: textbox.cx,
                cy: textbox.cy,
                w: textbox.w - 32.0,
                h: textbox.h - 36.0,
            };
            let mut content_node = text_node(
                layer_id,
                source_key,
                "content",
                2,
                layout_rect(content),
                TextSource::ProfileField {
                    field: "userProfile.word".into(),
                    value: snapshot.word.clone(),
                },
                26.0,
                [0.2, 0.2, 0.2, 1.0],
                GeneralTextAlign::Left,
                6.0,
                true,
            );
            if let GeneralRecipePayload::Text {
                render_baseline, ..
            } = &mut content_node.payload
            {
                // The measured General rectangle is a draw-anchor contract.
                // Preserve the legacy first-line baseline: textbox top + 18px
                // padding + 26px font size, expressed in TMP-local units.
                let textbox_top = layout_rect(textbox).y;
                let first_baseline = textbox_top + 18.0 + 26.0;
                let content_center = content_node.bounds.y + content_node.bounds.height / 2.0;
                *render_baseline = Some((first_baseline - content_center) / 2.0);
            }
            nodes.push(content_node);
        }
        9 => {
            let divider = crate::profile_layout::ElementLayout {
                h: 2.0,
                ..MVP_SUPERSTAR.elements[5]
            };
            nodes.push(shape_node(
                layer_id,
                source_key,
                "divider",
                0,
                layout_rect(divider),
                GeneralGeometry::Rect,
                [0.78, 0.78, 0.78, 0.5],
            ));
            nodes.push(text_node(
                layer_id,
                source_key,
                "title",
                1,
                layout_rect(MVP_SUPERSTAR.elements[0]),
                localized_source(snapshot, "custom_profile.general.multiplayer_live.title")?,
                MVP_SUPERSTAR.elements[0].h,
                [0.33, 0.33, 0.33, 1.0],
                GeneralTextAlign::Center,
                0.0,
                false,
            ));
            nodes.push(shape_node(
                layer_id,
                source_key,
                "mvp-background",
                2,
                layout_rect(MVP_SUPERSTAR.elements[1]),
                GeneralGeometry::RoundedRect {
                    radius: [16.0, 16.0],
                },
                [0.62, 0.62, 0.65, 1.0],
            ));
            nodes.push(text_node(
                layer_id,
                source_key,
                "mvp-label",
                3,
                layout_rect(MVP_SUPERSTAR.elements[1]),
                localized_source(snapshot, "custom_profile.general.multiplayer_live.mvp")?,
                MVP_SUPERSTAR.elements[1].h * 0.55,
                [1.0; 4],
                GeneralTextAlign::Center,
                0.0,
                false,
            ));
            nodes.push(text_node(
                layer_id,
                source_key,
                "mvp-count",
                4,
                layout_rect(MVP_SUPERSTAR.elements[2]),
                count_source(snapshot, snapshot.mvp)?,
                MVP_SUPERSTAR.elements[4].h,
                [0.27, 0.27, 0.27, 1.0],
                GeneralTextAlign::Left,
                0.0,
                false,
            ));
            nodes.push(shape_node(
                layer_id,
                source_key,
                "superstar-background",
                5,
                layout_rect(MVP_SUPERSTAR.elements[3]),
                GeneralGeometry::RoundedRect {
                    radius: [16.0, 16.0],
                },
                [0.62, 0.62, 0.65, 1.0],
            ));
            let upper = crate::profile_layout::ElementLayout {
                cy: MVP_SUPERSTAR.elements[3].cy + 10.0,
                h: 18.0,
                ..MVP_SUPERSTAR.elements[3]
            };
            let (superstar_upper, superstar_lower) = localized_two_line_sources(
                snapshot,
                "custom_profile.general.multiplayer_live.superstar",
            )?;
            nodes.push(text_node(
                layer_id,
                source_key,
                "superstar-upper",
                6,
                layout_rect(upper),
                superstar_upper,
                18.0,
                [1.0; 4],
                GeneralTextAlign::Center,
                0.0,
                false,
            ));
            let lower = crate::profile_layout::ElementLayout {
                cy: MVP_SUPERSTAR.elements[3].cy - 10.0,
                h: 18.0,
                ..MVP_SUPERSTAR.elements[3]
            };
            nodes.push(text_node(
                layer_id,
                source_key,
                "superstar-lower",
                7,
                layout_rect(lower),
                superstar_lower,
                18.0,
                [1.0; 4],
                GeneralTextAlign::Center,
                0.0,
                false,
            ));
            nodes.push(text_node(
                layer_id,
                source_key,
                "superstar-count",
                8,
                layout_rect(MVP_SUPERSTAR.elements[4]),
                count_source(snapshot, snapshot.superstar)?,
                MVP_SUPERSTAR.elements[4].h,
                [0.27, 0.27, 0.27, 1.0],
                GeneralTextAlign::Left,
                0.0,
                false,
            ));
        }
        10 => {
            let els = &CHALLENGE_LIVE.elements;
            let divider = crate::profile_layout::ElementLayout { h: 2.0, ..els[4] };
            nodes.push(shape_node(
                layer_id,
                source_key,
                "divider",
                0,
                layout_rect(divider),
                GeneralGeometry::Rect,
                [0.78, 0.78, 0.78, 0.5],
            ));
            nodes.push(text_node(
                layer_id,
                source_key,
                "title",
                1,
                layout_rect(els[0]),
                localized_source(snapshot, "custom_profile.general.challenge_live.title")?,
                els[0].h,
                [0.33, 0.33, 0.33, 1.0],
                GeneralTextAlign::Center,
                0.0,
                false,
            ));
            nodes.push(shape_node(
                layer_id,
                source_key,
                "solo-background",
                2,
                layout_rect(els[1]),
                GeneralGeometry::RoundedRect {
                    radius: [16.0, 16.0],
                },
                [0.62, 0.62, 0.65, 1.0],
            ));
            nodes.push(text_node(
                layer_id,
                source_key,
                "solo-label",
                3,
                layout_rect(els[1]),
                localized_source(snapshot, "custom_profile.general.challenge_live.solo")?,
                els[1].h * 0.55,
                [1.0; 4],
                GeneralTextAlign::Center,
                0.0,
                false,
            ));
            let avatar_bounds = layout_rect(els[2]);
            if let Some(image) = snapshot.challenge_avatar.as_ref() {
                if let Some(descriptor) = image.descriptor.as_ref() {
                    nodes.push(image_node(
                        layer_id,
                        source_key,
                        "challenge-avatar",
                        4,
                        avatar_bounds,
                        descriptor.resource.clone(),
                        Some([descriptor.natural_width, descriptor.natural_height]),
                        GeneralImageSource::WholeImageImplicit,
                        GeneralImageSampling::Nearest,
                        GeneralImageSourceConstraint::Implicit,
                        image,
                        descriptor.provenance.clone(),
                        vec![GeneralClip::Ellipse {
                            bounds: avatar_bounds,
                            anti_alias: true,
                        }],
                    ));
                } else {
                    nodes.push(shape_node(
                        layer_id,
                        source_key,
                        "challenge-avatar",
                        4,
                        avatar_bounds,
                        GeneralGeometry::Ellipse,
                        [0.87, 0.87, 0.87, 1.0],
                    ));
                }
                interaction_regions.push(item_region(
                    layer_id,
                    source_key,
                    "challenge-character",
                    avatar_bounds,
                    std::collections::BTreeMap::from([
                        (
                            "source_field".into(),
                            ParameterValue::Text(image.source_field.clone()),
                        ),
                        (
                            "character_id".into(),
                            ParameterValue::I64(snapshot.challenge_character_id.into()),
                        ),
                    ]),
                ));
            } else {
                nodes.push(shape_node(
                    layer_id,
                    source_key,
                    "challenge-avatar",
                    4,
                    avatar_bounds,
                    GeneralGeometry::Ellipse,
                    [0.87, 0.87, 0.87, 1.0],
                ));
            }
            nodes.push(text_node(
                layer_id,
                source_key,
                "challenge-score",
                5,
                layout_rect(els[3]),
                TextSource::ProfileField {
                    field: "userProfile.challengeScore".into(),
                    value: snapshot.challenge_score.to_string(),
                },
                els[3].h,
                [0.27, 0.27, 0.27, 1.0],
                GeneralTextAlign::Left,
                0.0,
                false,
            ));
        }
        12 => build_detailed_music_recipe(layer_id, source_key, snapshot, &mut nodes)?,
        16 => build_tabbed_music_recipe(
            layer_id,
            source_key,
            snapshot,
            &mut nodes,
            &mut controls,
            &mut interaction_regions,
        )?,
        17 => {
            let pill = Rect {
                x: -110.0,
                y: -26.0,
                width: 220.0,
                height: 52.0,
            };
            let mut background = shape_node(
                layer_id,
                source_key,
                "background",
                0,
                pill,
                GeneralGeometry::RoundedRect {
                    radius: [26.0, 26.0],
                },
                [0.15, 0.15, 0.20, 0.85],
            );
            if let GeneralRecipePayload::Shape { stroke, .. } = &mut background.payload {
                *stroke = Some(GeneralStroke {
                    color: [1.0, 1.0, 1.0, 0.15],
                    width: 1.0,
                });
            }
            nodes.push(background);
            nodes.push(image_node_without_source(
                layer_id,
                source_key,
                "rank-icon",
                1,
                Rect {
                    x: -94.0,
                    y: -16.0,
                    width: 32.0,
                    height: 32.0,
                },
                ResourceKey {
                    namespace: "static".into(),
                    key: "sprite/icon/icon_playerRank".into(),
                },
            ));
            nodes.push(text_node(
                layer_id,
                source_key,
                "label",
                2,
                Rect {
                    x: -54.0,
                    y: -13.0,
                    width: 90.0,
                    height: 26.0,
                },
                localized_source(snapshot, "custom_profile.general.player_level.label")?,
                26.0,
                [0.70, 0.70, 0.75, 1.0],
                GeneralTextAlign::Left,
                0.0,
                false,
            ));
            nodes.push(text_node(
                layer_id,
                source_key,
                "rank",
                3,
                Rect {
                    x: -90.0,
                    y: -13.0,
                    width: 180.0,
                    height: 26.0,
                },
                TextSource::ProfileField {
                    field: "userProfile.userRank".into(),
                    value: snapshot.user_rank.to_string(),
                },
                26.0,
                [1.0; 4],
                GeneralTextAlign::Right,
                0.0,
                false,
            ));
        }
        18 => {
            let avatar_bounds = Rect {
                x: -90.0,
                y: -90.0,
                width: 180.0,
                height: 180.0,
            };
            nodes.push(shape_node(
                layer_id,
                source_key,
                "avatar-background",
                0,
                avatar_bounds,
                GeneralGeometry::Ellipse,
                [0.85, 0.85, 0.9, 1.0],
            ));
            if let Some(image) = snapshot.player_avatar.as_ref() {
                if let Some(descriptor) = image.descriptor.as_ref() {
                    nodes.push(image_node(
                        layer_id,
                        source_key,
                        "avatar",
                        1,
                        avatar_bounds,
                        descriptor.resource.clone(),
                        Some([descriptor.natural_width, descriptor.natural_height]),
                        GeneralImageSource::Pixels(cover_source_rect(
                            descriptor.natural_width,
                            descriptor.natural_height,
                            avatar_bounds.width,
                            avatar_bounds.height,
                        )),
                        GeneralImageSampling::Nearest,
                        GeneralImageSourceConstraint::Fast,
                        image,
                        descriptor.provenance.clone(),
                        vec![GeneralClip::Ellipse {
                            bounds: avatar_bounds,
                            anti_alias: true,
                        }],
                    ));
                }
                interaction_regions.push(item_region(
                    layer_id,
                    source_key,
                    "player-avatar-card",
                    avatar_bounds,
                    std::collections::BTreeMap::from([
                        (
                            "source_field".into(),
                            ParameterValue::Text(image.source_field.clone()),
                        ),
                        (
                            "card_id".into(),
                            ParameterValue::Text(image.source_id.clone()),
                        ),
                    ]),
                ));
            }
            let mut border = shape_node(
                layer_id,
                source_key,
                "avatar-border",
                2,
                Rect {
                    x: -88.0,
                    y: -88.0,
                    width: 176.0,
                    height: 176.0,
                },
                GeneralGeometry::Ellipse,
                [0.0; 4],
            );
            if let GeneralRecipePayload::Shape { stroke, .. } = &mut border.payload {
                *stroke = Some(GeneralStroke {
                    color: [1.0, 1.0, 1.0, 0.85],
                    width: 4.0,
                });
            }
            nodes.push(border);
        }
        _ => unreachable!(),
    }
    Ok(Some(GeneralRecipe {
        general_type,
        source_key: source_key.into(),
        layer_id,
        nodes,
        controls,
        interaction_regions,
    }))
}

fn build_deck_recipe(
    layer_id: StableId,
    source_key: &str,
    snapshot: &ProfileComponentSnapshot,
    nodes: &mut Vec<GeneralRecipeNode>,
    interaction_regions: &mut Vec<InteractionRegionSource>,
) -> Result<(), ProfileResolveError> {
    const SLOT_COUNT: usize = 5;
    const CONTAINER_X: f32 = -390.5;
    const CONTAINER_Y: f32 = -124.5;
    const CONTAINER_WIDTH: f32 = 783.0;
    const CONTAINER_HEIGHT: f32 = 243.0;
    const CARD_WIDTH: f32 = 312.0;
    const CARD_HEIGHT: f32 = 512.0;
    const RAW_ROOT_WIDTH: f32 = 328.0;
    const RAW_ROOT_HEIGHT: f32 = 520.0;
    const BAR_HEIGHT: f32 = 56.39 * CARD_HEIGHT / RAW_ROOT_HEIGHT;
    const LEVEL_FONT_SIZE: f32 = 24.0;

    let slot_width = CONTAINER_WIDTH / SLOT_COUNT as f32;
    let scale = (slot_width / CARD_WIDTH).min(CONTAINER_HEIGHT / CARD_HEIGHT);
    let card_width = CARD_WIDTH * scale;
    let card_height = CARD_HEIGHT * scale;
    let level_template_key = "custom_profile.general.card_level";
    let level_template = snapshot
        .localized_text
        .get(level_template_key)
        .cloned()
        .ok_or_else(|| ProfileResolveError::MissingLocalizedText(level_template_key.into()))?;

    for slot_index in 0..SLOT_COUNT {
        let slot = Rect {
            x: CONTAINER_X + slot_index as f32 * slot_width,
            y: CONTAINER_Y,
            width: slot_width,
            height: CONTAINER_HEIGHT,
        };
        let center_x = slot.x + slot.width / 2.0;
        let card_bounds = Rect {
            x: center_x - card_width / 2.0,
            y: -3.0 - card_height / 2.0,
            width: card_width,
            height: card_height,
        };
        let base_ordinal = slot_index as u32 * 32;
        let member = snapshot.deck_members.get(slot_index);

        if let Some(card) = member {
            interaction_regions.push(item_region(
                layer_id,
                source_key,
                &format!("deck-slot-{slot_index}-card"),
                slot,
                std::collections::BTreeMap::from([
                    (
                        "field".into(),
                        ParameterValue::Text(card.image.source_field.clone()),
                    ),
                    ("card_id".into(), ParameterValue::I64(card.card_id.into())),
                    (
                        "after_training".into(),
                        ParameterValue::Bool(card.after_training),
                    ),
                    (
                        "master_rank".into(),
                        ParameterValue::I64(card.master_rank.into()),
                    ),
                    ("level".into(), ParameterValue::I64(card.level.into())),
                ]),
            ));
        }

        let descriptor = member.and_then(|card| card.image.descriptor.as_ref());
        let Some(descriptor) = descriptor else {
            let fill = member
                .map(|card| deck_attribute_color(&card.attribute, 0.7))
                .unwrap_or([0.45, 0.45, 0.55, 0.6]);
            let mut placeholder = shape_node(
                layer_id,
                source_key,
                &format!("deck-slot-{slot_index}-artwork"),
                base_ordinal,
                slot,
                GeneralGeometry::RoundedRect { radius: [6.0, 6.0] },
                fill,
            );
            if let GeneralRecipePayload::Shape { stroke, .. } = &mut placeholder.payload {
                *stroke = Some(GeneralStroke {
                    color: [1.0, 1.0, 1.0, 0.25],
                    width: 1.0,
                });
            }
            nodes.push(placeholder);
            if let Some(card) = member {
                let font_size = (CONTAINER_HEIGHT * 0.12).max(10.0);
                let line_width = (slot_width - 12.0).max(1.0);
                let center_x = slot.x + 6.0 + line_width / 2.0;
                let info_center_y = slot.y + CONTAINER_HEIGHT - 10.0 - font_size - 2.0;
                let level_center_y = slot.y + CONTAINER_HEIGHT - 10.0;
                nodes.push(text_node(
                    layer_id,
                    source_key,
                    &format!("deck-slot-{slot_index}-card-id"),
                    base_ordinal + 1,
                    Rect {
                        x: center_x - line_width / 2.0,
                        y: info_center_y - font_size / 2.0,
                        width: line_width,
                        height: font_size,
                    },
                    TextSource::ProfileField {
                        field: format!("userProfile.deckMembers.{slot_index}.cardId"),
                        value: format!("#{}", card.card_id),
                    },
                    font_size,
                    [1.0, 1.0, 1.0, 0.9],
                    GeneralTextAlign::Left,
                    0.0,
                    false,
                ));
                nodes.push(text_node(
                    layer_id,
                    source_key,
                    &format!("deck-slot-{slot_index}-level"),
                    base_ordinal + 2,
                    Rect {
                        x: center_x - line_width / 2.0,
                        y: level_center_y - font_size / 2.0,
                        width: line_width,
                        height: font_size,
                    },
                    localized_level_source(
                        snapshot,
                        level_template_key,
                        &level_template,
                        card.level,
                    ),
                    font_size,
                    [1.0, 1.0, 1.0, 0.9],
                    GeneralTextAlign::Left,
                    0.0,
                    false,
                ));
            }
            continue;
        };
        let card = member.expect("descriptor belongs to a deck member");
        let clip = vec![GeneralClip::Rect {
            bounds: slot,
            anti_alias: false,
        }];
        let crop_width = descriptor.natural_width.min(CARD_WIDTH);
        let crop_height = descriptor.natural_height.min(CARD_HEIGHT);
        nodes.push(image_node(
            layer_id,
            source_key,
            &format!("deck-slot-{slot_index}-artwork"),
            base_ordinal,
            card_bounds,
            descriptor.resource.clone(),
            Some([descriptor.natural_width, descriptor.natural_height]),
            GeneralImageSource::Pixels(Rect {
                x: (descriptor.natural_width - CARD_WIDTH).max(0.0) / 2.0,
                y: 0.0,
                width: crop_width,
                height: crop_height,
            }),
            GeneralImageSampling::Nearest,
            GeneralImageSourceConstraint::Fast,
            &card.image,
            descriptor.provenance.clone(),
            clip.clone(),
        ));

        let bar_bounds = transform_card_rect(
            center_x,
            scale,
            Rect {
                x: -CARD_WIDTH / 2.0,
                y: CARD_HEIGHT / 2.0 - BAR_HEIGHT,
                width: CARD_WIDTH,
                height: BAR_HEIGHT,
            },
        );
        nodes.push(static_styled_image_node(
            layer_id,
            source_key,
            &format!("deck-slot-{slot_index}-level-bar"),
            base_ordinal + 1,
            bar_bounds,
            "card/bg_base_wh".into(),
            [68.0 / 255.0, 68.0 / 255.0, 102.0 / 255.0, 1.0],
            clip.clone(),
        ));
        let level_font_size = LEVEL_FONT_SIZE * scale;
        let level_left = card_bounds.x + (12.9 * CARD_WIDTH / RAW_ROOT_WIDTH) * scale;
        nodes.push(text_node(
            layer_id,
            source_key,
            &format!("deck-slot-{slot_index}-level"),
            base_ordinal + 2,
            Rect {
                x: level_left,
                y: bar_bounds.y + (bar_bounds.height - level_font_size) / 2.0,
                width: (card_bounds.x + card_bounds.width - level_left).max(1.0),
                height: level_font_size,
            },
            localized_level_source(snapshot, level_template_key, &level_template, card.level),
            level_font_size,
            [1.0; 4],
            GeneralTextAlign::Left,
            0.0,
            false,
        ));
        nodes.push(static_styled_image_node(
            layer_id,
            source_key,
            &format!("deck-slot-{slot_index}-frame"),
            base_ordinal + 3,
            card_bounds,
            format!("card/cardFrame_M_{}", rarity_suffix(&card.rarity)),
            [1.0; 4],
            clip.clone(),
        ));
        nodes.push(static_styled_image_node(
            layer_id,
            source_key,
            &format!("deck-slot-{slot_index}-attribute"),
            base_ordinal + 4,
            transform_card_rect(
                center_x,
                scale,
                Rect {
                    x: -CARD_WIDTH / 2.0 + 8.0 * CARD_WIDTH / RAW_ROOT_WIDTH,
                    y: -CARD_HEIGHT / 2.0,
                    width: 64.0 * CARD_WIDTH / RAW_ROOT_WIDTH,
                    height: 68.0 * CARD_HEIGHT / RAW_ROOT_HEIGHT,
                },
            ),
            format!("card/icon_attribute_{}_64", card.attribute),
            [1.0; 4],
            clip.clone(),
        ));
        let star_count = rarity_count(&card.rarity);
        for star_index in 0..star_count {
            nodes.push(static_styled_image_node(
                layer_id,
                source_key,
                &format!("deck-slot-{slot_index}-star-{star_index}"),
                base_ordinal + 5 + star_index as u32,
                transform_card_rect(
                    center_x,
                    scale,
                    Rect {
                        x: -CARD_WIDTH / 2.0 + 5.0 + star_index as f32 * 40.0,
                        y: CARD_HEIGHT / 2.0 - 64.0 - 40.0,
                        width: 40.0,
                        height: 40.0,
                    },
                ),
                star_icon_key(&card.rarity, card.after_training).into(),
                [1.0; 4],
                clip.clone(),
            ));
        }
        nodes.push(static_styled_image_node(
            layer_id,
            source_key,
            &format!("deck-slot-{slot_index}-master-rank"),
            base_ordinal + 16,
            transform_card_rect(
                center_x,
                scale,
                Rect {
                    x: CARD_WIDTH / 2.0
                        - 88.0 * CARD_WIDTH / RAW_ROOT_WIDTH
                        - 1.4 * CARD_WIDTH / RAW_ROOT_WIDTH,
                    y: CARD_HEIGHT / 2.0
                        - 88.0 * CARD_HEIGHT / RAW_ROOT_HEIGHT
                        - 0.8 * CARD_HEIGHT / RAW_ROOT_HEIGHT,
                    width: 88.0 * CARD_WIDTH / RAW_ROOT_WIDTH,
                    height: 88.0 * CARD_HEIGHT / RAW_ROOT_HEIGHT,
                },
            ),
            format!("card/masterRank_S_{}", card.master_rank.clamp(0, 5)),
            [1.0; 4],
            clip,
        ));
    }
    Ok(())
}

fn build_character_rank_recipe(
    general_type: i32,
    layer_id: StableId,
    source_key: &str,
    snapshot: &ProfileComponentSnapshot,
    nodes: &mut Vec<GeneralRecipeNode>,
    controls: &mut Vec<ComponentControlSource>,
    interaction_regions: &mut Vec<InteractionRegionSource>,
) -> Result<(), ProfileResolveError> {
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
        layer_id,
        source_key,
        "tabs-background",
        0,
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
            layer_id,
            source_key,
            role,
            ordinal,
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
            layer_id,
            source_key,
            role,
            ordinal,
            shifted(layout),
            localized_source(snapshot, key)?,
            layout.h * 0.42,
            color,
            GeneralTextAlign::Center,
            0.0,
            false,
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
    let columns = [-350.0f32, -150.0, 50.0, 250.0];
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
            let cx = columns[column];
            let cy = -259.0 + row as f32 * 105.0 + shift_y;
            slot += 1;
            let radius = 38.0;
            let pill_height = 60.8;
            let pill_width = 175.0;
            let pill_bounds = Rect {
                x: cx - radius,
                y: cy + radius - pill_height,
                width: pill_width,
                height: pill_height,
            };
            nodes.push(shape_node(
                layer_id,
                source_key,
                &format!("character-{character_id}-background"),
                ordinal,
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
                layer_id,
                source_key,
                &format!("character-{character_id}-avatar-background"),
                ordinal,
                avatar_bounds,
                GeneralGeometry::Ellipse,
                [0.204, 0.863, 0.996, 1.0],
            ));
            ordinal += 1;
            if let Some(descriptor) = &rank.avatar.descriptor {
                nodes.push(image_node(
                    layer_id,
                    source_key,
                    &format!("character-{character_id}-avatar"),
                    ordinal,
                    avatar_bounds,
                    descriptor.resource.clone(),
                    Some([descriptor.natural_width, descriptor.natural_height]),
                    GeneralImageSource::WholeImageImplicit,
                    GeneralImageSampling::Nearest,
                    GeneralImageSourceConstraint::Implicit,
                    &rank.avatar,
                    descriptor.provenance.clone(),
                    vec![GeneralClip::Ellipse {
                        bounds: avatar_bounds,
                        anti_alias: true,
                    }],
                ));
            } else {
                nodes.push(shape_node(
                    layer_id,
                    source_key,
                    &format!("character-{character_id}-avatar-placeholder"),
                    ordinal,
                    avatar_bounds,
                    GeneralGeometry::Ellipse,
                    [0.85, 0.85, 0.85, 0.4],
                ));
            }
            ordinal += 1;
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
                    rank.challenge_rank.unwrap_or(0),
                ),
            ] {
                let mut node = text_node(
                    layer_id,
                    source_key,
                    &format!("character-{character_id}-{mode}"),
                    ordinal,
                    number_bounds,
                    TextSource::ProfileField {
                        field: format!("userProfile.{field}.{character_id}.rank"),
                        value: value.to_string(),
                    },
                    22.0,
                    [0.0, 0.0, 0.0, 1.0],
                    GeneralTextAlign::Center,
                    0.0,
                    false,
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
                        ParameterValue::I64(rank.challenge_rank.unwrap_or(0).into()),
                    ),
                ]),
            ));
        }
        if slot % 4 != 0 {
            slot = (slot / 4 + 1) * 4;
        }
    }
    if general_type == 15 {
        let viewport_top =
            (-active.cy + active.h / 2.0 + shift_y).max(-inactive.cy + inactive.h / 2.0 + shift_y);
        let viewport_bottom = 286.0;
        let viewport = Rect {
            x: -483.5,
            y: viewport_top,
            width: 967.0,
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
            layer_id,
            source_key,
            "character-rank-scroll-track",
            10_000,
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
            layer_id,
            source_key,
            "character-rank-scroll-thumb",
            10_001,
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

fn build_story_favorite_recipe(
    layer_id: StableId,
    source_key: &str,
    snapshot: &ProfileComponentSnapshot,
    nodes: &mut Vec<GeneralRecipeNode>,
    controls: &mut Vec<ComponentControlSource>,
    interaction_regions: &mut Vec<InteractionRegionSource>,
) -> Result<(), ProfileResolveError> {
    let panel = &crate::profile_layout::STORY_FAVORITE;
    nodes.push(text_node(
        layer_id,
        source_key,
        "title",
        0,
        layout_rect(panel.elements[0]),
        localized_source(snapshot, "custom_profile.general.story_favorite.title")?,
        panel.elements[0].h,
        [0.33, 0.33, 0.33, 1.0],
        GeneralTextAlign::Center,
        0.0,
        false,
    ));
    nodes.push(shape_node(
        layer_id,
        source_key,
        "separator",
        1,
        Rect {
            height: 2.0,
            ..layout_rect(panel.elements[1])
        },
        GeneralGeometry::Rect,
        [0.78, 0.78, 0.78, 0.5],
    ));
    let content_start = nodes.len();
    let region_start = interaction_regions.len();
    for (index, favorite) in snapshot.story_favorites.iter().enumerate() {
        let target = panel.elements.get(index + 2).copied().unwrap_or_else(|| {
            let column = index % 2;
            let row = index / 2;
            crate::profile_layout::ElementLayout {
                cx: if column == 0 { -212.0 } else { 212.0 },
                cy: 220.0 - row as f32 * 195.0,
                w: 400.0,
                h: 170.0,
            }
        });
        let bounds = layout_rect(target);
        let role = format!("story-{index}");
        if let Some(descriptor) = &favorite.image.descriptor {
            nodes.push(image_node(
                layer_id,
                source_key,
                &role,
                2 + index as u32,
                bounds,
                descriptor.resource.clone(),
                Some([descriptor.natural_width, descriptor.natural_height]),
                GeneralImageSource::WholeImageImplicit,
                GeneralImageSampling::Nearest,
                GeneralImageSourceConstraint::Implicit,
                &favorite.image,
                descriptor.provenance.clone(),
                Vec::new(),
            ));
        } else {
            nodes.push(shape_node(
                layer_id,
                source_key,
                &format!("{role}-placeholder"),
                2 + index as u32,
                bounds,
                GeneralGeometry::RoundedRect { radius: [8.0, 8.0] },
                [0.82, 0.82, 0.82, 0.4],
            ));
        }
        interaction_regions.push(item_region(
            layer_id,
            source_key,
            &role,
            bounds,
            std::collections::BTreeMap::from([
                (
                    "field".into(),
                    ParameterValue::Text(favorite.image.source_field.clone()),
                ),
                (
                    "source_id".into(),
                    ParameterValue::Text(favorite.image.source_id.clone()),
                ),
                (
                    "story_type".into(),
                    ParameterValue::Text(favorite.story_type.clone()),
                ),
                (
                    "story_id".into(),
                    ParameterValue::I64(favorite.story_id.into()),
                ),
            ]),
        ));
    }
    let viewport = Rect {
        x: -424.0,
        y: -320.0,
        width: 848.0,
        height: 756.0,
    };
    let content_bottom = nodes[content_start..]
        .iter()
        .map(|node| node.bounds.y + node.bounds.height)
        .fold(viewport.y + viewport.height, f32::max);
    let max = (content_bottom - (viewport.y + viewport.height)).max(0.0);
    if max > 0.0 {
        let control_id =
            crate::profile_scene::component_control_id(source_key, "story-favorite-scroll");
        let clip = GeneralClip::Rect {
            bounds: viewport,
            anti_alias: false,
        };
        for node in &mut nodes[content_start..] {
            node.clips.push(clip.clone());
            node.control_bindings
                .push(CommandControlBinding::ScrollContent { control_id });
        }
        for region in &mut interaction_regions[region_start..] {
            region.clip = Some(crate::profile_scene::rect_quad(viewport));
            region
                .control_bindings
                .push(CommandControlBinding::ScrollContent { control_id });
        }
        let track_bounds = Rect {
            x: 475.5,
            y: viewport.y,
            width: 4.0,
            height: viewport.height,
        };
        nodes.push(shape_node(
            layer_id,
            source_key,
            "story-scroll-track",
            10_000,
            track_bounds,
            GeneralGeometry::RoundedRect { radius: [2.0, 2.0] },
            [0.25, 0.25, 0.25, 0.18],
        ));
        let thumb_height = (viewport.height * viewport.height / (viewport.height + max))
            .clamp(40.0, viewport.height);
        let thumb_bounds = Rect {
            height: thumb_height,
            ..track_bounds
        };
        let mut thumb = shape_node(
            layer_id,
            source_key,
            "story-scroll-thumb",
            10_001,
            thumb_bounds,
            GeneralGeometry::RoundedRect { radius: [2.0, 2.0] },
            [0.25, 0.25, 0.25, 0.55],
        );
        thumb
            .control_bindings
            .push(CommandControlBinding::ScrollThumb { control_id });
        nodes.push(thumb);
        let mut thumb_region = item_region(
            layer_id,
            source_key,
            "story-scroll-thumb",
            thumb_bounds,
            std::collections::BTreeMap::from([
                (
                    "control_id".into(),
                    ParameterValue::Text(format!("{:016x}", control_id.0)),
                ),
                ("action".into(), ParameterValue::Text("scroll_thumb".into())),
            ]),
        );
        thumb_region.capabilities = vec!["inspect".into(), "scroll".into(), "drag".into()];
        thumb_region
            .control_bindings
            .push(CommandControlBinding::ScrollThumb { control_id });
        interaction_regions.push(thumb_region);
        controls.push(ComponentControlSource {
            id: control_id,
            layer_id,
            role: "story-favorite-scroll".into(),
            state: ComponentControlState::Scroll {
                offset: 0.0,
                min: 0.0,
                max,
                viewport_extent: viewport.height,
                content_extent: viewport.height + max,
                step: 195.0,
            },
        });
        let mut scroll_region = item_region(
            layer_id,
            source_key,
            "story-favorite-scroll",
            viewport,
            std::collections::BTreeMap::from([
                (
                    "control_id".into(),
                    ParameterValue::Text(format!("{:016x}", control_id.0)),
                ),
                ("action".into(), ParameterValue::Text("scroll_by".into())),
            ]),
        );
        scroll_region.capabilities = vec!["inspect".into(), "scroll".into()];
        scroll_region
            .control_bindings
            .push(CommandControlBinding::ScrollViewport { control_id });
        interaction_regions.push(scroll_region);
    }
    Ok(())
}

fn build_honors_recipe(
    layer_id: StableId,
    source_key: &str,
    snapshot: &ProfileComponentSnapshot,
    nodes: &mut Vec<GeneralRecipeNode>,
    interaction_regions: &mut Vec<InteractionRegionSource>,
) {
    nodes.push(shape_node(
        layer_id,
        source_key,
        "honors-background",
        0,
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
            crate::profile_scene::HonorVisualKind::Standard {
                honor_type,
                has_star,
                is_live_master,
                progress,
                background,
                frame_candidates,
                overlay,
                star,
                star_high,
                live_star_on,
                live_star_off,
            } => build_standard_honor_recipe(
                layer_id,
                source_key,
                honor,
                honor_type,
                *has_star,
                *is_live_master,
                *progress,
                background.as_ref(),
                frame_candidates,
                overlay.as_ref(),
                star.as_ref(),
                star_high.as_ref(),
                live_star_on.as_ref(),
                live_star_off.as_ref(),
                cx,
                cy,
                bounds,
                base,
                nodes,
            ),
            crate::profile_scene::HonorVisualKind::Bonds {
                character_ids,
                backgrounds,
                characters,
                mask,
                frame,
                word,
                star,
                star_high,
            } => build_bonds_honor_recipe(
                layer_id,
                source_key,
                honor,
                *character_ids,
                backgrounds,
                characters,
                mask.as_ref(),
                frame.as_ref(),
                word.as_ref(),
                star.as_ref(),
                star_high.as_ref(),
                cx,
                cy,
                bounds,
                base,
                nodes,
            ),
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
        if let crate::profile_scene::HonorVisualKind::Bonds {
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

#[allow(clippy::too_many_arguments)]
fn build_standard_honor_recipe(
    layer_id: StableId,
    source_key: &str,
    honor: &crate::profile_scene::HonorVisualSnapshot,
    honor_type: &str,
    has_star: bool,
    is_live_master: bool,
    progress: i32,
    background: Option<&crate::profile_scene::ResourceDescriptor>,
    frame_candidates: &[Option<crate::profile_scene::ResourceDescriptor>],
    overlay: Option<&crate::profile_scene::ResourceDescriptor>,
    star: Option<&crate::profile_scene::ResourceDescriptor>,
    star_high: Option<&crate::profile_scene::ResourceDescriptor>,
    live_star_on: Option<&crate::profile_scene::ResourceDescriptor>,
    live_star_off: Option<&crate::profile_scene::ResourceDescriptor>,
    cx: f32,
    cy: f32,
    bounds: Rect,
    base: u32,
    nodes: &mut Vec<GeneralRecipeNode>,
) {
    if let Some(value) = background {
        nodes.push(honor_image_node(
            layer_id,
            source_key,
            &format!("honor-{}-background", honor.honor_id),
            base,
            bounds,
            value,
            GeneralImageSource::WholeImageExplicit,
            GeneralImageSourceConstraint::Fast,
            BlendMode::SrcOver,
        ));
    }
    if let Some(value) = frame_candidates.iter().flatten().next() {
        nodes.push(honor_image_node(
            layer_id,
            source_key,
            &format!("honor-{}-frame", honor.honor_id),
            base + 1,
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
        let (dx, dy) = if is_live_master {
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
            layer_id,
            source_key,
            &format!("honor-{}-overlay", honor.honor_id),
            base + 2,
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
    if is_live_master {
        let font_size = 20.0;
        let baseline = cy + 30.0;
        nodes.push(text_node(
            layer_id,
            source_key,
            &format!("honor-{}-progress", honor.honor_id),
            base + 3,
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
            0.0,
            false,
        ));
        let active = ((progress / 10) % 10 + 1).max(0) as usize;
        let positions: &[(f32, f32)] = if honor.full_size {
            &[
                (223.0, 68.0),
                (216.0, 56.0),
                (208.0, 42.0),
                (216.0, 27.0),
                (223.0, 13.0),
                (295.0, 68.0),
                (304.0, 56.0),
                (311.0, 42.0),
                (303.0, 27.0),
                (295.0, 13.0),
            ]
        } else {
            &[
                (45.0, 68.0),
                (38.0, 56.0),
                (30.0, 42.0),
                (38.0, 27.0),
                (45.0, 13.0),
                (117.0, 68.0),
                (126.0, 56.0),
                (133.0, 42.0),
                (125.0, 27.0),
                (117.0, 13.0),
            ]
        };
        for (index, (x, y)) in positions.iter().enumerate() {
            let value = if index < active {
                live_star_on
            } else {
                live_star_off
            };
            if let Some(value) = value {
                nodes.push(honor_image_node(
                    layer_id,
                    source_key,
                    &format!("honor-{}-live-star-{index}", honor.honor_id),
                    base + 4 + index as u32,
                    Rect {
                        x: bounds.x + x,
                        y: bounds.y + y - 8.0,
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
    } else if has_star && matches!(honor_type, "character" | "achievement") {
        build_honor_star_nodes(
            layer_id,
            source_key,
            honor,
            star,
            star_high,
            bounds,
            base + 16,
            false,
            nodes,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn build_bonds_honor_recipe(
    layer_id: StableId,
    source_key: &str,
    honor: &crate::profile_scene::HonorVisualSnapshot,
    _character_ids: [i32; 2],
    backgrounds: &[Option<crate::profile_scene::ResourceDescriptor>; 2],
    characters: &[Option<crate::profile_scene::ResourceDescriptor>; 2],
    mask: Option<&crate::profile_scene::ResourceDescriptor>,
    frame: Option<&crate::profile_scene::ResourceDescriptor>,
    word: Option<&crate::profile_scene::ResourceDescriptor>,
    star: Option<&crate::profile_scene::ResourceDescriptor>,
    star_high: Option<&crate::profile_scene::ResourceDescriptor>,
    cx: f32,
    cy: f32,
    bounds: Rect,
    base: u32,
    nodes: &mut Vec<GeneralRecipeNode>,
) {
    nodes.push(group_node(
        layer_id,
        source_key,
        &format!("bonds-{}-isolation-begin", honor.honor_id),
        base,
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
                layer_id,
                source_key,
                &format!("bonds-{}-background-{index}", honor.honor_id),
                base + 1 + index as u32,
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
                layer_id,
                source_key,
                &format!("bonds-{}-character-{index}", honor.honor_id),
                base + 3 + index as u32,
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
            layer_id,
            source_key,
            &format!("bonds-{}-mask", honor.honor_id),
            base + 5,
            bounds,
            value,
            GeneralImageSource::WholeImageImplicit,
            GeneralImageSourceConstraint::Implicit,
            BlendMode::DstIn,
        ));
    }
    nodes.push(group_node(
        layer_id,
        source_key,
        &format!("bonds-{}-isolation-end", honor.honor_id),
        base + 6,
        bounds,
        GeneralGroupPhase::End,
    ));
    if let Some(value) = frame {
        nodes.push(honor_image_node(
            layer_id,
            source_key,
            &format!("bonds-{}-frame", honor.honor_id),
            base + 7,
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
            layer_id,
            source_key,
            &format!("bonds-{}-word", honor.honor_id),
            base + 8,
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
        layer_id,
        source_key,
        honor,
        star,
        star_high,
        bounds,
        base + 16,
        true,
        nodes,
    );
}

fn build_honor_star_nodes(
    layer_id: StableId,
    source_key: &str,
    honor: &crate::profile_scene::HonorVisualSnapshot,
    star: Option<&crate::profile_scene::ResourceDescriptor>,
    star_high: Option<&crate::profile_scene::ResourceDescriptor>,
    bounds: Rect,
    base: u32,
    bonds: bool,
    nodes: &mut Vec<GeneralRecipeNode>,
) {
    let mut level = honor.honor_level;
    if bonds {
        if level > 10 {
            level -= 10;
        }
    } else {
        level %= 10;
        if level == 0 && honor.honor_level > 0 {
            level = 10;
        }
    }
    let x = if honor.full_size {
        bounds.x + 54.0
    } else {
        bounds.x + bounds.width / 2.0 - 40.0
    };
    let y = bounds.y + 63.0;
    if let Some(value) = star {
        for index in 0..level.min(5) {
            nodes.push(honor_image_node(
                layer_id,
                source_key,
                &format!("honor-{}-star-{index}", honor.honor_id),
                base + index as u32,
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
                    layer_id,
                    source_key,
                    &format!("honor-{}-star-high-{index}", honor.honor_id),
                    base + 8 + index as u32,
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

fn transform_card_rect(center_x: f32, scale: f32, rect: Rect) -> Rect {
    Rect {
        x: center_x + rect.x * scale,
        y: -3.0 + rect.y * scale,
        width: rect.width * scale,
        height: rect.height * scale,
    }
}

fn localized_level_source(
    snapshot: &ProfileComponentSnapshot,
    key: &str,
    template: &str,
    level: i32,
) -> TextSource {
    TextSource::Localized {
        key: key.into(),
        locale: snapshot.locale.clone(),
        value: template.replace("{level}", &level.to_string()),
    }
}

fn deck_attribute_color(attribute: &str, alpha: f32) -> [f32; 4] {
    let (red, green, blue) = match attribute {
        "cool" => (97.0, 148.0, 199.0),
        "cute" => (235.0, 133.0, 141.0),
        "happy" => (242.0, 184.0, 102.0),
        "mysterious" => (148.0, 112.0, 204.0),
        _ => (138.0, 189.0, 143.0),
    };
    [red / 255.0, green / 255.0, blue / 255.0, alpha]
}

fn build_leader_member_recipe(
    layer_id: StableId,
    source_key: &str,
    snapshot: &ProfileComponentSnapshot,
    nodes: &mut Vec<GeneralRecipeNode>,
    interaction_regions: &mut Vec<InteractionRegionSource>,
) {
    let cover = Rect {
        x: -471.0,
        y: -266.0,
        width: 940.0,
        height: 530.0,
    };
    let Some(card) = snapshot.leader_card.as_ref() else {
        nodes.push(shape_node(
            layer_id,
            source_key,
            "leader-artwork",
            0,
            cover,
            GeneralGeometry::Rect,
            [0.4, 0.4, 0.4, 1.0],
        ));
        return;
    };

    interaction_regions.push(item_region(
        layer_id,
        source_key,
        "leader-card",
        cover,
        std::collections::BTreeMap::from([
            (
                "source_field".into(),
                ParameterValue::Text(card.image.source_field.clone()),
            ),
            ("card_id".into(), ParameterValue::I64(card.card_id.into())),
            (
                "after_training".into(),
                ParameterValue::Bool(card.after_training),
            ),
            (
                "master_rank".into(),
                ParameterValue::I64(card.master_rank.into()),
            ),
            ("level".into(), ParameterValue::I64(card.level.into())),
        ]),
    ));

    let Some(descriptor) = card.image.descriptor.as_ref() else {
        nodes.push(shape_node(
            layer_id,
            source_key,
            "leader-artwork",
            0,
            cover,
            GeneralGeometry::Rect,
            [0.4, 0.4, 0.4, 1.0],
        ));
        return;
    };

    nodes.push(image_node(
        layer_id,
        source_key,
        "leader-artwork",
        0,
        cover,
        descriptor.resource.clone(),
        Some([descriptor.natural_width, descriptor.natural_height]),
        GeneralImageSource::Pixels(cover_source_rect(
            descriptor.natural_width,
            descriptor.natural_height,
            cover.width,
            cover.height,
        )),
        GeneralImageSampling::Nearest,
        GeneralImageSourceConstraint::Fast,
        &card.image,
        descriptor.provenance.clone(),
        Vec::new(),
    ));

    nodes.push(static_overlay_image_node(
        layer_id,
        source_key,
        "leader-frame",
        1,
        cover,
        format!("card/cardFrame_L_{}", rarity_suffix(&card.rarity)),
    ));
    nodes.push(static_overlay_image_node(
        layer_id,
        source_key,
        "leader-attribute",
        2,
        Rect {
            x: 341.0,
            y: -266.0,
            width: 88.0,
            height: 92.0,
        },
        format!("card/icon_attribute_{}_88", card.attribute),
    ));

    let star_count = rarity_count(&card.rarity);
    let star_start_y = 39.0 + (4usize.saturating_sub(star_count)) as f32 * 48.0;
    for index in 0..star_count {
        nodes.push(static_overlay_image_node(
            layer_id,
            source_key,
            &format!("leader-star-{index}"),
            3 + index as u32,
            Rect {
                x: -447.0,
                y: star_start_y + index as f32 * 48.0,
                width: 56.0,
                height: 56.0,
            },
            star_icon_key(&card.rarity, card.after_training).into(),
        ));
    }
    nodes.push(static_overlay_image_node(
        layer_id,
        source_key,
        "leader-master-rank",
        3 + star_count as u32,
        Rect {
            x: 341.0,
            y: 136.0,
            width: 104.0,
            height: 104.0,
        },
        format!("card/masterRank_L_{}", card.master_rank.clamp(0, 5)),
    ));
}

fn shape_node(
    layer_id: StableId,
    source_key: &str,
    role: &str,
    ordinal: u32,
    bounds: Rect,
    geometry: GeneralGeometry,
    color: [f32; 4],
) -> GeneralRecipeNode {
    GeneralRecipeNode {
        id: recipe_node_id(source_key, role, ordinal),
        layer_id,
        role: role.into(),
        parent_id: None,
        bounds,
        local_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        clips: Vec::new(),
        control_bindings: Vec::new(),
        payload: GeneralRecipePayload::Shape {
            geometry,
            fill: GeneralFill::Solid(color),
            stroke: None,
        },
    }
}

fn group_node(
    layer_id: StableId,
    source_key: &str,
    role: &str,
    ordinal: u32,
    bounds: Rect,
    phase: GeneralGroupPhase,
) -> GeneralRecipeNode {
    GeneralRecipeNode {
        id: recipe_node_id(source_key, role, ordinal),
        layer_id,
        role: role.into(),
        parent_id: None,
        bounds,
        local_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        clips: Vec::new(),
        control_bindings: Vec::new(),
        payload: GeneralRecipePayload::Group {
            isolation: true,
            phase,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn honor_image_node(
    layer_id: StableId,
    source_key: &str,
    role: &str,
    ordinal: u32,
    bounds: Rect,
    descriptor: &crate::profile_scene::ResourceDescriptor,
    source: GeneralImageSource,
    source_constraint: GeneralImageSourceConstraint,
    blend_mode: BlendMode,
) -> GeneralRecipeNode {
    GeneralRecipeNode {
        id: recipe_node_id(source_key, role, ordinal),
        layer_id,
        role: role.into(),
        parent_id: None,
        bounds,
        local_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        clips: Vec::new(),
        control_bindings: Vec::new(),
        payload: GeneralRecipePayload::Image {
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
    }
}

#[allow(clippy::too_many_arguments)]
fn text_node(
    layer_id: StableId,
    source_key: &str,
    role: &str,
    ordinal: u32,
    bounds: Rect,
    source: TextSource,
    font_size: f32,
    color: [f32; 4],
    align: GeneralTextAlign,
    line_spacing: f32,
    wrap: bool,
) -> GeneralRecipeNode {
    GeneralRecipeNode {
        id: recipe_node_id(source_key, role, ordinal),
        layer_id,
        role: role.into(),
        parent_id: None,
        bounds,
        local_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        clips: Vec::new(),
        control_bindings: Vec::new(),
        payload: GeneralRecipePayload::Text {
            source,
            font_role: FontRole::RegionFontId(1),
            font_size,
            color,
            align,
            line_spacing,
            wrap,
            render_baseline: None,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn image_node(
    layer_id: StableId,
    source_key: &str,
    role: &str,
    ordinal: u32,
    bounds: Rect,
    resource: ResourceKey,
    natural_size: Option<[f32; 2]>,
    source: GeneralImageSource,
    sampling: GeneralImageSampling,
    source_constraint: GeneralImageSourceConstraint,
    image: &crate::profile_scene::ComponentImageSnapshot,
    provenance: std::collections::BTreeMap<String, ParameterValue>,
    clips: Vec<GeneralClip>,
) -> GeneralRecipeNode {
    GeneralRecipeNode {
        id: recipe_node_id(source_key, role, ordinal),
        layer_id,
        role: role.into(),
        parent_id: None,
        bounds,
        local_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        clips,
        control_bindings: Vec::new(),
        payload: GeneralRecipePayload::Image {
            resource,
            natural_size,
            source,
            sampling,
            source_constraint,
            tint: [1.0; 4],
            source_field: Some(image.source_field.clone()),
            source_id: Some(image.source_id.clone()),
            provenance,
            blend_mode: BlendMode::SrcOver,
        },
    }
}

fn image_node_without_source(
    layer_id: StableId,
    source_key: &str,
    role: &str,
    ordinal: u32,
    bounds: Rect,
    resource: ResourceKey,
) -> GeneralRecipeNode {
    GeneralRecipeNode {
        id: recipe_node_id(source_key, role, ordinal),
        layer_id,
        role: role.into(),
        parent_id: None,
        bounds,
        local_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        clips: Vec::new(),
        control_bindings: Vec::new(),
        payload: GeneralRecipePayload::Image {
            resource,
            natural_size: None,
            source: GeneralImageSource::WholeImageExplicit,
            sampling: GeneralImageSampling::Nearest,
            source_constraint: GeneralImageSourceConstraint::Fast,
            tint: [1.0; 4],
            source_field: None,
            source_id: None,
            provenance: std::collections::BTreeMap::new(),
            blend_mode: BlendMode::SrcOver,
        },
    }
}

fn static_overlay_image_node(
    layer_id: StableId,
    source_key: &str,
    role: &str,
    ordinal: u32,
    bounds: Rect,
    key: String,
) -> GeneralRecipeNode {
    static_styled_image_node(
        layer_id,
        source_key,
        role,
        ordinal,
        bounds,
        key,
        [1.0; 4],
        Vec::new(),
    )
}

#[allow(clippy::too_many_arguments)]
fn static_styled_image_node(
    layer_id: StableId,
    source_key: &str,
    role: &str,
    ordinal: u32,
    bounds: Rect,
    key: String,
    tint: [f32; 4],
    clips: Vec<GeneralClip>,
) -> GeneralRecipeNode {
    GeneralRecipeNode {
        id: recipe_node_id(source_key, role, ordinal),
        layer_id,
        role: role.into(),
        parent_id: None,
        bounds,
        local_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        clips,
        control_bindings: Vec::new(),
        payload: GeneralRecipePayload::Image {
            resource: ResourceKey {
                namespace: "static".into(),
                key,
            },
            natural_size: None,
            source: GeneralImageSource::WholeImageImplicit,
            sampling: GeneralImageSampling::Nearest,
            source_constraint: GeneralImageSourceConstraint::Implicit,
            tint,
            source_field: None,
            source_id: None,
            provenance: std::collections::BTreeMap::new(),
            blend_mode: BlendMode::SrcOver,
        },
    }
}

fn cover_source_rect(
    source_width: f32,
    source_height: f32,
    target_width: f32,
    target_height: f32,
) -> Rect {
    let source_ratio = source_width / source_height;
    let target_ratio = target_width / target_height;
    if source_ratio > target_ratio {
        let width = source_height * target_ratio;
        Rect {
            x: (source_width - width) / 2.0,
            y: 0.0,
            width,
            height: source_height,
        }
    } else {
        let height = source_width / target_ratio;
        Rect {
            x: 0.0,
            y: (source_height - height) / 2.0,
            width: source_width,
            height,
        }
    }
}

fn rarity_suffix(rarity: &str) -> &str {
    if rarity == "rarity_birthday" {
        "bd"
    } else {
        rarity.rsplit('_').next().unwrap_or("1")
    }
}

fn rarity_count(rarity: &str) -> usize {
    if rarity == "rarity_birthday" {
        1
    } else {
        rarity
            .rsplit('_')
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1)
    }
}

fn star_icon_key(rarity: &str, trained: bool) -> &'static str {
    if rarity == "rarity_birthday" {
        "card/rarity_birthday"
    } else if trained {
        "card/rarity_star_afterTraining"
    } else {
        "card/rarity_star_normal"
    }
}

fn item_region(
    layer_id: StableId,
    source_key: &str,
    role: &str,
    bounds: Rect,
    resolved_data: std::collections::BTreeMap<String, ParameterValue>,
) -> InteractionRegionSource {
    InteractionRegionSource {
        id: StableId::derive(
            "general-interaction-v1",
            format!("{source_key}\0{role}").as_bytes(),
        ),
        layer_id,
        role: role.into(),
        bounds,
        quad: crate::profile_scene::rect_quad(bounds),
        matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        hit_geometry: crate::profile_scene::rect_quad(bounds),
        clip: None,
        control_bindings: Vec::new(),
        resolved_data,
        capabilities: vec!["inspect".into(), "select_item".into()],
    }
}

fn layout_rect(layout: crate::profile_layout::ElementLayout) -> Rect {
    Rect {
        x: layout.cx - layout.w / 2.0,
        y: -layout.cy - layout.h / 2.0,
        width: layout.w,
        height: layout.h,
    }
}

fn recipe_node_id(source_key: &str, role: &str, ordinal: u32) -> StableId {
    StableId::derive(
        "general-recipe-node-v1",
        format!("{source_key}\0{role}\0{ordinal}").as_bytes(),
    )
}

fn localized_source(
    snapshot: &ProfileComponentSnapshot,
    key: &str,
) -> Result<TextSource, ProfileResolveError> {
    let value = snapshot
        .localized_text
        .get(key)
        .cloned()
        .ok_or_else(|| ProfileResolveError::MissingLocalizedText(key.into()))?;
    Ok(TextSource::Localized {
        key: key.into(),
        locale: snapshot.locale.clone(),
        value,
    })
}

fn localized_two_line_sources(
    snapshot: &ProfileComponentSnapshot,
    key: &str,
) -> Result<(TextSource, TextSource), ProfileResolveError> {
    let value = snapshot
        .localized_text
        .get(key)
        .cloned()
        .ok_or_else(|| ProfileResolveError::MissingLocalizedText(key.into()))?;
    let normalized = value.replace("\r\n", "\n");
    let (upper, lower) = normalized
        .split_once('\n')
        .or_else(|| normalized.rsplit_once(char::is_whitespace))
        .map(|(upper, lower)| (upper.trim_end(), lower.trim_start()))
        .unwrap_or((normalized.as_str(), ""));
    let source = |value: &str| TextSource::Localized {
        key: key.into(),
        locale: snapshot.locale.clone(),
        value: value.into(),
    };
    Ok((source(upper), source(lower)))
}

fn count_source(
    snapshot: &ProfileComponentSnapshot,
    count: i32,
) -> Result<TextSource, ProfileResolveError> {
    let key = "custom_profile.general.count_times";
    let template = snapshot
        .localized_text
        .get(key)
        .cloned()
        .ok_or_else(|| ProfileResolveError::MissingLocalizedText(key.into()))?;
    Ok(TextSource::Localized {
        key: key.into(),
        locale: snapshot.locale.clone(),
        value: template.replace("{count}", &count.to_string()),
    })
}

fn build_detailed_music_recipe(
    layer_id: StableId,
    source_key: &str,
    snapshot: &ProfileComponentSnapshot,
    nodes: &mut Vec<GeneralRecipeNode>,
) -> Result<(), ProfileResolveError> {
    let results = snapshot.music_results.clone().unwrap_or_default();
    let difficulties = [
        ("easy", results.easy),
        ("normal", results.normal),
        ("hard", results.hard),
        ("expert", results.expert),
        ("master", results.master),
        ("append", results.append),
    ];
    let colors = [
        [0.000, 0.859, 0.451, 1.0],
        [0.149, 0.792, 0.996, 1.0],
        [0.996, 0.788, 0.000, 1.0],
        [0.996, 0.239, 0.447, 1.0],
        [0.788, 0.169, 1.000, 1.0],
        [0.843, 0.741, 1.000, 1.0],
    ];
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
            layer_id,
            source_key,
            &format!("{group_role}-heading-background"),
            ordinal,
            layout_rect(bubble),
            GeneralGeometry::RoundedRect {
                radius: [bubble.h / 2.0, bubble.h / 2.0],
            },
            [0.737, 0.737, 0.753, 1.0],
        ));
        ordinal += 1;
        nodes.push(text_node(
            layer_id,
            source_key,
            &format!("{group_role}-heading"),
            ordinal,
            layout_rect(bubble),
            localized_source(snapshot, heading_key)?,
            bubble.h * 0.42,
            [1.0; 4],
            GeneralTextAlign::Center,
            0.0,
            false,
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
                layer_id,
                source_key,
                &format!("{group_role}-{difficulty_role}-label-background"),
                ordinal,
                layout_rect(label),
                GeneralGeometry::RoundedRect { radius: [8.0, 8.0] },
                colors[difficulty_index],
            ));
            ordinal += 1;
            nodes.push(text_node(
                layer_id,
                source_key,
                &format!("{group_role}-{difficulty_role}-label"),
                ordinal,
                layout_rect(label),
                localized_source(
                    snapshot,
                    &format!("custom_profile.general.music.difficulty.{difficulty_role}"),
                )?,
                28.0 * 0.55,
                [1.0; 4],
                GeneralTextAlign::Center,
                0.0,
                false,
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
                layer_id,
                source_key,
                &format!("{group_role}-{difficulty_role}-count"),
                ordinal,
                layout_rect(number),
                TextSource::ProfileField {
                    field: format!("userMusicDifficultyClearCount.{difficulty_role}.{field_name}"),
                    value: value.to_string(),
                },
                22.0,
                [0.2, 0.2, 0.2, 1.0],
                GeneralTextAlign::Center,
                0.0,
                false,
            ));
            ordinal += 1;
        }
    }
    Ok(())
}

fn build_tabbed_music_recipe(
    layer_id: StableId,
    source_key: &str,
    snapshot: &ProfileComponentSnapshot,
    nodes: &mut Vec<GeneralRecipeNode>,
    controls: &mut Vec<ComponentControlSource>,
    interaction_regions: &mut Vec<InteractionRegionSource>,
) -> Result<(), ProfileResolveError> {
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
    let colors = [
        [0.000, 0.859, 0.451, 1.0],
        [0.149, 0.792, 0.996, 1.0],
        [0.996, 0.788, 0.000, 1.0],
        [0.996, 0.239, 0.447, 1.0],
        [0.788, 0.169, 1.000, 1.0],
    ];
    let mut ordinal = 0u32;
    for (difficulty_index, (role, label_index, number_index, values)) in
        difficulties.iter().enumerate()
    {
        let measured_label = MUSIC_CLEAR_TAB.elements[*label_index];
        let label = crate::profile_layout::ElementLayout {
            cx: measured_label.cx,
            cy: -20.0,
            w: measured_label.w,
            h: 43.0,
        };
        let mut background = shape_node(
            layer_id,
            source_key,
            &format!("{role}-label-background"),
            ordinal,
            layout_rect(label),
            GeneralGeometry::RoundedRect { radius: [8.0, 8.0] },
            colors.get(difficulty_index).copied().unwrap_or([0.0; 4]),
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
            layer_id,
            source_key,
            &format!("{role}-label"),
            ordinal,
            layout_rect(label),
            localized_source(
                snapshot,
                &format!("custom_profile.general.music.difficulty.{role}"),
            )?,
            43.0 * 0.55,
            [1.0; 4],
            GeneralTextAlign::Center,
            0.0,
            false,
        ));
        ordinal += 1;
        let measured_number = MUSIC_CLEAR_TAB.elements[*number_index];
        let number = crate::profile_layout::ElementLayout {
            cx: measured_number.cx,
            cy: -63.0,
            w: measured_number.w,
            h: 29.0,
        };
        for option in options {
            let (field, value) = match option {
                "clear" => ("liveClear", values.clear),
                "full_combo" => ("fullCombo", values.full_combo),
                _ => ("allPerfect", values.all_perfect),
            };
            let mut node = text_node(
                layer_id,
                source_key,
                &format!("{role}-{option}-count"),
                ordinal,
                layout_rect(number),
                TextSource::ProfileField {
                    field: format!("userMusicDifficultyClearCount.{role}.{field}"),
                    value: value.to_string(),
                },
                29.0,
                [0.2, 0.2, 0.2, 1.0],
                GeneralTextAlign::Center,
                0.0,
                false,
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
        layer_id,
        source_key,
        "append-separator",
        ordinal,
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
        layer_id,
        source_key,
        "tab-background",
        ordinal,
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
            layer_id,
            source_key,
            &format!("tab-{active}-selected"),
            ordinal,
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
                layer_id,
                source_key,
                &format!("tab-{active}-{label_option}-label"),
                ordinal,
                layout_rect(label),
                localized_source(snapshot, key)?,
                tab.h * 0.42,
                if active == label_option {
                    [0.2, 0.2, 0.2, 1.0]
                } else {
                    [1.0; 4]
                },
                GeneralTextAlign::Center,
                0.0,
                false,
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::profile_scene::{
        MusicDifficultySnapshot, MusicResultsSnapshot, ProfileComponentSnapshot,
    };
    use crate::{FontRole, StableId, TextSource};

    #[test]
    fn all_supported_general_types_resolve_from_one_recipe_entrypoint() {
        let snapshot = ProfileComponentSnapshot {
            locale: "en-US".into(),
            region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
            localized_text: crate::locale::GENERAL_LOCALIZATION_KEYS
                .iter()
                .map(|key| ((*key).into(), (*key).into()))
                .collect(),
            ..ProfileComponentSnapshot::default()
        };
        for general_type in super::SUPPORTED_GENERAL_TYPES {
            let recipe = super::build_general_recipe(
                general_type,
                StableId(general_type as u64),
                &format!("general:{general_type}"),
                &snapshot,
            )
            .unwrap()
            .unwrap();
            assert_eq!(recipe.general_type, general_type);
            assert!(recipe.nodes.iter().all(|node| {
                !matches!(node.payload, super::GeneralRecipePayload::Text { .. })
                    || node.clips.is_empty()
            }));
        }
        for unsupported in [0, 1, 7, 8, 19] {
            assert!(super::build_general_recipe(
                unsupported,
                StableId(0),
                "unsupported",
                &snapshot,
            )
            .unwrap()
            .is_none());
        }
    }

    #[test]
    fn identity_general_recipe_is_the_exact_native_player_name_and_comment_sequence() {
        let snapshot = ProfileComponentSnapshot {
            locale: "en-US".into(),
            region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
            localized_text: BTreeMap::from([(
                "custom_profile.general.comment.title".into(),
                "Bio".into(),
            )]),
            user_name: "Player".into(),
            word: "Hello".into(),
            ..ProfileComponentSnapshot::default()
        };
        let name = super::build_general_recipe(13, StableId(1), "general:13", &snapshot)
            .unwrap()
            .unwrap();
        assert_eq!(name.nodes.len(), 2);
        assert!(matches!(
            &name.nodes[0].payload,
            super::GeneralRecipePayload::Shape {
                geometry: super::GeneralGeometry::RoundedRect { radius: [8.0, 8.0] },
                fill: super::GeneralFill::Solid([0.922, 0.922, 0.941, 1.0]),
                stroke: None,
            }
        ));
        assert_eq!(
            name.nodes[0].bounds,
            crate::Rect {
                x: -273.0,
                y: -32.0,
                width: 544.0,
                height: 64.0
            }
        );
        assert!(matches!(
            &name.nodes[1].payload,
            super::GeneralRecipePayload::Text {
                source: TextSource::ProfileField { field, value },
                font_role: FontRole::RegionFontId(1),
                font_size: 34.0,
                color: [0.2, 0.2, 0.2, 1.0],
                align: super::GeneralTextAlign::Left,
                ..
            } if field == "user.name" && value == "Player"
        ));
        assert!(
            name.nodes[1].clips.is_empty(),
            "measured label bounds are draw anchors, not a text clipping box"
        );
        let lowered_name =
            crate::profile_scene::lower_identity_general(13, StableId(1), "general:13", &snapshot)
                .unwrap()
                .unwrap();
        assert!(matches!(
            &lowered_name.commands[1].payload,
            crate::SemanticCommandPayload::Text {
                size,
                alignment: 1,
                max_width: None,
                max_height: None,
                ..
            } if (*size - 17.0).abs() < 1e-6
        ));
        assert_eq!(
            lowered_name.commands[1].matrix[4],
            name.nodes[1].bounds.x + name.nodes[1].bounds.width / 2.0
        );
        let comment = super::build_general_recipe(4, StableId(2), "general:4", &snapshot)
            .unwrap()
            .unwrap();
        assert_eq!(comment.nodes.len(), 3);
        assert!(matches!(
            &comment.nodes[1].payload,
            super::GeneralRecipePayload::Text {
                source: TextSource::Localized { key, locale, value },
                color: [0.53, 0.53, 0.53, 1.0],
                ..
            } if key == "custom_profile.general.comment.title" && locale == "en-US" && value == "Bio"
        ));
        assert!(matches!(
            &comment.nodes[2].payload,
            super::GeneralRecipePayload::Text {
                source: TextSource::ProfileField { field, value },
                color: [0.2, 0.2, 0.2, 1.0],
                line_spacing: 6.0,
                render_baseline: Some(baseline),
                ..
            } if field == "userProfile.word" && value == "Hello"
                && (*baseline + 13.5).abs() < 1e-6
        ));
        assert!(
            comment.nodes[2].clips.is_empty(),
            "General text bounds may guide wrapping but must never clip glyphs"
        );

        let lowered =
            crate::profile_scene::lower_identity_general(4, StableId(2), "general:4", &snapshot)
                .unwrap()
                .unwrap();
        assert_eq!(
            lowered
                .commands
                .iter()
                .map(|value| value.id)
                .collect::<Vec<_>>(),
            comment
                .nodes
                .iter()
                .map(|value| value.id)
                .collect::<Vec<_>>()
        );
        for (command, node) in lowered.commands.iter().zip(&comment.nodes) {
            assert_eq!(command.bounds, node.bounds);
            assert_eq!(command.control_bindings, node.control_bindings);
        }
        assert_eq!(
            lowered.commands[2].render_placement,
            Some(crate::TextRenderPlacementSource {
                anchor_x: -comment.nodes[2].bounds.width / 4.0,
                baseline: Some(-13.5),
            })
        );
        assert!(matches!(
            &lowered.commands[2].payload,
            crate::SemanticCommandPayload::Text {
                max_width: Some(width),
                max_height: None,
                ..
            } if *width == comment.nodes[2].bounds.width
        ));
    }

    #[test]
    fn stats_general_recipe_preserves_native_divider_and_two_line_superstar_text() {
        let snapshot = ProfileComponentSnapshot {
            locale: "en-US".into(),
            region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
            localized_text: BTreeMap::from([
                (
                    "custom_profile.general.total_power.title".into(),
                    "Power".into(),
                ),
                (
                    "custom_profile.general.multiplayer_live.title".into(),
                    "Multiplayer".into(),
                ),
                (
                    "custom_profile.general.multiplayer_live.mvp".into(),
                    "Region MVP".into(),
                ),
                (
                    "custom_profile.general.multiplayer_live.superstar".into(),
                    "Region Super Star".into(),
                ),
                (
                    "custom_profile.general.count_times".into(),
                    "{count} times".into(),
                ),
            ]),
            total_power: 123_456,
            mvp: 7,
            superstar: 8,
            ..ProfileComponentSnapshot::default()
        };
        let power = super::build_general_recipe(2, StableId(3), "general:2", &snapshot)
            .unwrap()
            .unwrap();
        assert_eq!(power.nodes.len(), 3);
        assert!(matches!(
            &power.nodes[1].payload,
            super::GeneralRecipePayload::Text {
                source: TextSource::Authored { value },
                font_role: FontRole::RegionFontId(1),
                font_size: 34.0,
                color: [0.67, 0.67, 0.67, 1.0],
                ..
            } if value == "|"
        ));

        let multiplayer = super::build_general_recipe(9, StableId(4), "general:9", &snapshot)
            .unwrap()
            .unwrap();
        assert_eq!(
            multiplayer
                .nodes
                .iter()
                .map(|node| node.role.as_str())
                .collect::<Vec<_>>(),
            vec![
                "divider",
                "title",
                "mvp-background",
                "mvp-label",
                "mvp-count",
                "superstar-background",
                "superstar-upper",
                "superstar-lower",
                "superstar-count",
            ]
        );
        assert!(matches!(
            &multiplayer.nodes[3].payload,
            super::GeneralRecipePayload::Text {
                source: TextSource::Localized { key, locale, value },
                ..
            } if key == "custom_profile.general.multiplayer_live.mvp"
                && locale == "en-US"
                && value == "Region MVP"
        ));
        assert!(matches!(
            &multiplayer.nodes[3].payload,
            super::GeneralRecipePayload::Text { font_size, .. }
                if (*font_size - 30.8).abs() < 1e-5
        ));
        assert!(matches!(
            &multiplayer.nodes[6].payload,
            super::GeneralRecipePayload::Text {
                source: TextSource::Localized { key, locale, value },
                font_size: 18.0,
                ..
            } if key == "custom_profile.general.multiplayer_live.superstar"
                && locale == "en-US"
                && value == "Region Super"
        ));
        assert!(matches!(
            &multiplayer.nodes[7].payload,
            super::GeneralRecipePayload::Text {
                source: TextSource::Localized { key, locale, value },
                font_size: 18.0,
                ..
            } if key == "custom_profile.general.multiplayer_live.superstar"
                && locale == "en-US"
                && value == "Star"
        ));
    }

    #[test]
    fn detailed_music_recipe_expands_native_three_by_six_sequence_without_backend_loops() {
        let localized_text = crate::locale::GENERAL_LOCALIZATION_KEYS
            .iter()
            .map(|key| ((*key).into(), (*key).into()))
            .collect();
        let snapshot = ProfileComponentSnapshot {
            locale: "en-US".into(),
            region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
            localized_text,
            music_results: Some(MusicResultsSnapshot {
                easy: MusicDifficultySnapshot {
                    clear: 1,
                    full_combo: 2,
                    all_perfect: 3,
                },
                normal: MusicDifficultySnapshot {
                    clear: 4,
                    full_combo: 5,
                    all_perfect: 6,
                },
                ..MusicResultsSnapshot::default()
            }),
            ..ProfileComponentSnapshot::default()
        };
        let recipe = super::build_general_recipe(12, StableId(5), "general:12", &snapshot)
            .unwrap()
            .unwrap();
        assert_eq!(recipe.nodes.len(), 60);
        assert_eq!(
            recipe
                .nodes
                .iter()
                .take(5)
                .map(|node| node.role.as_str())
                .collect::<Vec<_>>(),
            vec![
                "clear-heading-background",
                "clear-heading",
                "clear-easy-label-background",
                "clear-easy-label",
                "clear-easy-count",
            ]
        );
        assert!(matches!(
            &recipe.nodes[4].payload,
            super::GeneralRecipePayload::Text {
                source: TextSource::ProfileField { field, value },
                ..
            } if field == "userMusicDifficultyClearCount.easy.liveClear" && value == "1"
        ));
        assert!(matches!(
            &recipe.nodes[24].payload,
            super::GeneralRecipePayload::Text {
                source: TextSource::ProfileField { field, value },
                ..
            } if field == "userMusicDifficultyClearCount.easy.fullCombo" && value == "2"
        ));
    }

    #[test]
    fn tabbed_music_recipe_binds_counts_selected_background_and_text_color_to_one_control() {
        let localized_text = crate::locale::GENERAL_LOCALIZATION_KEYS
            .iter()
            .map(|key| ((*key).into(), (*key).into()))
            .collect();
        let snapshot = ProfileComponentSnapshot {
            locale: "en-US".into(),
            region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
            localized_text,
            music_results: Some(MusicResultsSnapshot {
                easy: MusicDifficultySnapshot {
                    clear: 1,
                    full_combo: 2,
                    all_perfect: 3,
                },
                ..MusicResultsSnapshot::default()
            }),
            ..ProfileComponentSnapshot::default()
        };
        let recipe = super::build_general_recipe(16, StableId(6), "general:16", &snapshot)
            .unwrap()
            .unwrap();
        assert_eq!(recipe.controls.len(), 1);
        assert!(matches!(
            &recipe.controls[0].state,
            crate::profile_scene::ComponentControlState::Tabs { options, active }
                if options == &["clear", "full_combo", "all_perfect"] && active == "clear"
        ));
        let control_id = recipe.controls[0].id;
        for option in ["clear", "full_combo", "all_perfect"] {
            assert_eq!(
                recipe.nodes.iter().filter(|node| node.control_bindings.iter().any(|binding| {
                    matches!(binding, crate::CommandControlBinding::TabOption { control_id: id, value }
                        if *id == control_id && value == option)
                })).count(),
                10,
                "option={option} must own six counts, one selected background and three state-colored tab labels"
            );
        }
        assert_eq!(recipe.interaction_regions.len(), 3);
        for (region, option) in
            recipe
                .interaction_regions
                .iter()
                .zip(["clear", "full_combo", "all_perfect"])
        {
            assert!(
                region.control_bindings.is_empty(),
                "inactive tab {option} must remain hittable"
            );
            assert_eq!(
                region.resolved_data.get("action"),
                Some(&crate::ParameterValue::Text("set_tab".into()))
            );
            assert_eq!(
                region.resolved_data.get("value"),
                Some(&crate::ParameterValue::Text(option.into()))
            );
        }
    }

    #[test]
    fn challenge_rank_and_player_avatar_are_owned_by_the_shared_recipe() {
        fn image(
            field: &str,
            id: &str,
            namespace: &str,
            key: &str,
            width: f32,
            height: f32,
        ) -> crate::profile_scene::ComponentImageSnapshot {
            crate::profile_scene::ComponentImageSnapshot {
                source_field: field.into(),
                source_id: id.into(),
                descriptor: Some(crate::profile_scene::ResourceDescriptor {
                    resource: crate::ResourceKey {
                        namespace: namespace.into(),
                        key: key.into(),
                    },
                    natural_width: width,
                    natural_height: height,
                    provenance: BTreeMap::new(),
                }),
            }
        }

        let snapshot = ProfileComponentSnapshot {
            locale: "en-US".into(),
            region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
            localized_text: BTreeMap::from([
                (
                    "custom_profile.general.challenge_live.title".into(),
                    "Challenge Show".into(),
                ),
                (
                    "custom_profile.general.challenge_live.solo".into(),
                    "Solo".into(),
                ),
                (
                    "custom_profile.general.player_level.label".into(),
                    "Rank".into(),
                ),
            ]),
            user_rank: 321,
            challenge_score: 987_654,
            challenge_character_id: 7,
            challenge_avatar: Some(image(
                "userProfile.challengeLiveSoloResult.characterId",
                "7",
                "static",
                "chara_avatar/chara07_02",
                76.0,
                76.0,
            )),
            player_avatar: Some(image(
                "userProfile.leaderCard",
                "1007",
                "assets",
                "thumbnail/chara/demo_after_training",
                940.0,
                530.0,
            )),
            ..ProfileComponentSnapshot::default()
        };

        for (general_type, expected_roles) in [
            (
                10,
                vec![
                    "divider",
                    "title",
                    "solo-background",
                    "solo-label",
                    "challenge-avatar",
                    "challenge-score",
                ],
            ),
            (17, vec!["background", "rank-icon", "label", "rank"]),
            (18, vec!["avatar-background", "avatar", "avatar-border"]),
        ] {
            let recipe = super::build_general_recipe(
                general_type,
                StableId(general_type as u64),
                &format!("general:{general_type}"),
                &snapshot,
            )
            .unwrap()
            .expect("shared recipe must own this General type");
            assert_eq!(
                recipe
                    .nodes
                    .iter()
                    .map(|node| node.role.as_str())
                    .collect::<Vec<_>>(),
                expected_roles
            );
        }

        let challenge = super::build_general_recipe(10, StableId(10), "general:10", &snapshot)
            .unwrap()
            .unwrap();
        assert_eq!(
            challenge.nodes[4].bounds,
            crate::Rect {
                x: -268.0,
                y: -14.0,
                width: 92.0,
                height: 92.0,
            }
        );
        assert!(matches!(
            &challenge.nodes[4].payload,
            super::GeneralRecipePayload::Image {
                source: super::GeneralImageSource::WholeImageImplicit,
                sampling: super::GeneralImageSampling::Nearest,
                source_constraint: super::GeneralImageSourceConstraint::Implicit,
                ..
            }
        ));
        assert!(matches!(
            challenge.nodes[4].clips.as_slice(),
            [super::GeneralClip::Ellipse { bounds, .. }] if *bounds == challenge.nodes[4].bounds
        ));
        assert_eq!(challenge.interaction_regions.len(), 1);
        assert_eq!(
            challenge.interaction_regions[0]
                .resolved_data
                .get("character_id"),
            Some(&crate::ParameterValue::I64(7))
        );

        let rank = super::build_general_recipe(17, StableId(17), "general:17", &snapshot)
            .unwrap()
            .unwrap();
        assert_eq!(rank.nodes[2].bounds.x, -54.0);
        assert_eq!(rank.nodes[2].bounds.width, 90.0);
        let lowered_rank =
            crate::profile_scene::lower_identity_general(17, StableId(17), "general:17", &snapshot)
                .unwrap()
                .unwrap();
        assert!(matches!(
            &lowered_rank.commands[3].payload,
            crate::SemanticCommandPayload::Text {
                size,
                alignment: 4,
                ..
            } if (*size - 13.0).abs() < 1e-6
        ));
        let placement = lowered_rank.commands[3].render_placement.unwrap();
        assert_eq!(placement.anchor_x, 45.0);
        assert!((placement.baseline.unwrap() - 4.55).abs() < 1e-5);

        let avatar = super::build_general_recipe(18, StableId(18), "general:18", &snapshot)
            .unwrap()
            .unwrap();
        assert!(matches!(
            &avatar.nodes[1].payload,
            super::GeneralRecipePayload::Image {
                source: super::GeneralImageSource::Pixels(crate::Rect {
                    x: 205.0,
                    y: 0.0,
                    width: 530.0,
                    height: 530.0,
                }),
                source_constraint: super::GeneralImageSourceConstraint::Fast,
                ..
            }
        ));
        assert_eq!(
            avatar.nodes[2].id,
            super::recipe_node_id("general:18", "avatar-border", 2)
        );
    }

    #[test]
    fn leader_member_recipe_preserves_native_cover_stack_and_availability_branch() {
        let card = crate::profile_scene::CardVisualSnapshot {
            card_id: 2001,
            after_training: false,
            master_rank: 3,
            level: 50,
            rarity: "rarity_4".into(),
            attribute: "cool".into(),
            image: crate::profile_scene::ComponentImageSnapshot {
                source_field: "userProfile.leaderCard".into(),
                source_id: "2001".into(),
                descriptor: Some(crate::profile_scene::ResourceDescriptor {
                    resource: crate::ResourceKey {
                        namespace: "assets".into(),
                        key: "character/member_small/demo/card_normal".into(),
                    },
                    natural_width: 1200.0,
                    natural_height: 600.0,
                    provenance: BTreeMap::new(),
                }),
            },
        };
        let recipe = super::build_general_recipe(
            5,
            StableId(5),
            "general:5",
            &ProfileComponentSnapshot {
                leader_card: Some(card.clone()),
                ..ProfileComponentSnapshot::default()
            },
        )
        .unwrap()
        .expect("type 5 must not require a font");
        assert_eq!(
            recipe
                .nodes
                .iter()
                .map(|node| node.role.as_str())
                .collect::<Vec<_>>(),
            vec![
                "leader-artwork",
                "leader-frame",
                "leader-attribute",
                "leader-star-0",
                "leader-star-1",
                "leader-star-2",
                "leader-star-3",
                "leader-master-rank",
            ]
        );
        assert!(matches!(
            &recipe.nodes[0].payload,
            super::GeneralRecipePayload::Image {
                source: super::GeneralImageSource::Pixels(source),
                source_constraint: super::GeneralImageSourceConstraint::Fast,
                ..
            } if source.x == 67.924_5
                && source.y == 0.0
                && source.width == 1064.151
                && source.height == 600.0
        ));
        assert_eq!(recipe.nodes[3].bounds.y, 39.0);
        assert_eq!(recipe.nodes[6].bounds.y, 183.0);
        assert_eq!(recipe.interaction_regions.len(), 1);
        assert_eq!(
            recipe.interaction_regions[0].resolved_data.get("level"),
            Some(&crate::ParameterValue::I64(50))
        );

        let mut unavailable = card;
        unavailable.image.descriptor = None;
        let fallback = super::build_general_recipe(
            5,
            StableId(5),
            "general:5",
            &ProfileComponentSnapshot {
                leader_card: Some(unavailable),
                ..ProfileComponentSnapshot::default()
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(fallback.nodes.len(), 1);
        assert_eq!(fallback.nodes[0].role, "leader-artwork");
        assert_eq!(fallback.nodes[0].id, recipe.nodes[0].id);
        assert!(matches!(
            fallback.nodes[0].payload,
            super::GeneralRecipePayload::Shape { .. }
        ));
    }

    #[test]
    fn leader_member_recipe_does_not_clamp_native_rarity_count() {
        let mut card = crate::profile_scene::CardVisualSnapshot {
            card_id: 2001,
            after_training: false,
            master_rank: 0,
            level: 1,
            rarity: "rarity_5".into(),
            attribute: "cool".into(),
            image: crate::profile_scene::ComponentImageSnapshot {
                source_field: "userProfile.leaderCard".into(),
                source_id: "2001".into(),
                descriptor: None,
            },
        };
        card.image.descriptor = Some(crate::profile_scene::ResourceDescriptor {
            resource: crate::ResourceKey {
                namespace: "assets".into(),
                key: "character/member_small/demo/card_normal".into(),
            },
            natural_width: 1200.0,
            natural_height: 600.0,
            provenance: BTreeMap::new(),
        });
        let recipe = super::build_general_recipe(
            5,
            StableId(5),
            "general:5",
            &ProfileComponentSnapshot {
                leader_card: Some(card),
                ..ProfileComponentSnapshot::default()
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            recipe
                .nodes
                .iter()
                .filter(|node| node.role.starts_with("leader-star-"))
                .count(),
            5
        );
        assert_eq!(
            recipe
                .nodes
                .iter()
                .find(|node| node.role == "leader-star-4")
                .unwrap()
                .bounds
                .y,
            231.0
        );
    }

    #[test]
    fn deck_recipe_preserves_five_native_slots_crop_order_and_localized_sdf_level() {
        let card = crate::profile_scene::CardVisualSnapshot {
            card_id: 3001,
            after_training: true,
            master_rank: 2,
            level: 60,
            rarity: "rarity_4".into(),
            attribute: "cute".into(),
            image: crate::profile_scene::ComponentImageSnapshot {
                source_field: "userProfile.deckMembers".into(),
                source_id: "3001".into(),
                descriptor: Some(crate::profile_scene::ResourceDescriptor {
                    resource: crate::ResourceKey {
                        namespace: "assets".into(),
                        key: "character/member_cutout/demo/after_training".into(),
                    },
                    natural_width: 600.0,
                    natural_height: 576.0,
                    provenance: BTreeMap::new(),
                }),
            },
        };
        let mut missing = card.clone();
        missing.card_id = 3002;
        missing.image.source_id = "3002".into();
        missing.image.descriptor = None;
        let recipe = super::build_general_recipe(
            3,
            StableId(3),
            "general:3",
            &ProfileComponentSnapshot {
                locale: "en-US".into(),
                region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
                localized_text: BTreeMap::from([(
                    "custom_profile.general.card_level".into(),
                    "Lv.{level}".into(),
                )]),
                deck_members: vec![card, missing],
                ..ProfileComponentSnapshot::default()
            },
        )
        .unwrap()
        .unwrap();

        let slot0 = recipe
            .nodes
            .iter()
            .filter(|node| node.role.starts_with("deck-slot-0-"))
            .collect::<Vec<_>>();
        assert_eq!(
            slot0
                .iter()
                .map(|node| node.role.as_str())
                .collect::<Vec<_>>(),
            vec![
                "deck-slot-0-artwork",
                "deck-slot-0-level-bar",
                "deck-slot-0-level",
                "deck-slot-0-frame",
                "deck-slot-0-attribute",
                "deck-slot-0-star-0",
                "deck-slot-0-star-1",
                "deck-slot-0-star-2",
                "deck-slot-0-star-3",
                "deck-slot-0-master-rank",
            ]
        );
        assert_eq!(slot0[0].bounds.width, 148.078_13);
        assert_eq!(slot0[0].bounds.height, 243.0);
        assert!(matches!(
            slot0[0].payload,
            super::GeneralRecipePayload::Image {
                source: super::GeneralImageSource::Pixels(crate::Rect {
                    x: 144.0,
                    y: 0.0,
                    width: 312.0,
                    height: 512.0,
                }),
                source_constraint: super::GeneralImageSourceConstraint::Fast,
                ..
            }
        ));
        assert!(matches!(
            &slot0[1].payload,
            super::GeneralRecipePayload::Image { resource, tint, .. }
                if resource.namespace == "static"
                    && resource.key == "card/bg_base_wh"
                    && *tint == [68.0 / 255.0, 68.0 / 255.0, 102.0 / 255.0, 1.0]
        ));
        assert!(matches!(
            &slot0[2].payload,
            super::GeneralRecipePayload::Text {
                source: crate::TextSource::Localized { key, value, .. },
                font_role: crate::FontRole::RegionFontId(1),
                ..
            } if key == "custom_profile.general.card_level" && value == "Lv.60"
        ));
        assert!(slot0[2].clips.is_empty());
        assert!(slot0
            .iter()
            .filter(|node| !matches!(node.payload, super::GeneralRecipePayload::Text { .. }))
            .all(|node| matches!(node.clips.as_slice(), [super::GeneralClip::Rect { .. }])));

        for slot in 1..5 {
            assert!(recipe.nodes.iter().any(|node| {
                node.role == format!("deck-slot-{slot}-artwork")
                    && matches!(node.payload, super::GeneralRecipePayload::Shape { .. })
            }));
        }
        assert_eq!(recipe.interaction_regions.len(), 2);
    }

    #[test]
    fn full_deck_exposes_five_independent_card_interaction_regions() {
        let deck_members = (0..5)
            .map(|slot| crate::profile_scene::CardVisualSnapshot {
                card_id: 4001 + slot,
                after_training: slot % 2 == 0,
                master_rank: slot,
                level: 50 + slot,
                rarity: "rarity_4".into(),
                attribute: "cool".into(),
                image: crate::profile_scene::ComponentImageSnapshot {
                    source_field: "userProfile.deckMembers".into(),
                    source_id: (4001 + slot).to_string(),
                    descriptor: None,
                },
            })
            .collect();
        let recipe = super::build_general_recipe(
            3,
            StableId(3),
            "general:3",
            &ProfileComponentSnapshot {
                locale: "en-US".into(),
                region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
                localized_text: BTreeMap::from([(
                    "custom_profile.general.card_level".into(),
                    "Lv.{level}".into(),
                )]),
                deck_members,
                ..ProfileComponentSnapshot::default()
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(recipe.interaction_regions.len(), 5);
        for slot in 0..5 {
            let region = recipe
                .interaction_regions
                .iter()
                .find(|region| region.role == format!("deck-slot-{slot}-card"))
                .unwrap();
            assert_eq!(
                region.resolved_data.get("card_id"),
                Some(&crate::ParameterValue::I64(4001 + slot))
            );
            assert_eq!(region.bounds.width, 783.0 / 5.0);
            assert!(region.capabilities.contains(&"select_item".into()));
        }
    }

    #[test]
    fn bonds_honor_recipe_uses_one_isolation_and_group_level_dst_in() {
        let descriptor = |namespace: &str, key: &str, width: f32, height: f32| {
            crate::profile_scene::ResourceDescriptor {
                resource: crate::ResourceKey {
                    namespace: namespace.into(),
                    key: key.into(),
                },
                natural_width: width,
                natural_height: height,
                provenance: BTreeMap::new(),
            }
        };
        let honor = crate::profile_scene::HonorVisualSnapshot {
            source_field: "userProfile.honorSlots".into(),
            source_id: "100".into(),
            honor_id: 100,
            honor_level: 27,
            full_size: true,
            visual: crate::profile_scene::HonorVisualKind::Bonds {
                character_ids: [1, 2],
                backgrounds: [
                    Some(descriptor("static", "honor/bonds/1", 380.0, 80.0)),
                    Some(descriptor("static", "honor/bonds/2", 380.0, 80.0)),
                ],
                characters: [
                    Some(descriptor(
                        "assets",
                        "bonds_honor/chr_sd_01_01",
                        160.0,
                        160.0,
                    )),
                    Some(descriptor(
                        "assets",
                        "bonds_honor/chr_sd_02_01",
                        160.0,
                        160.0,
                    )),
                ],
                mask: Some(descriptor("static", "honor/mask_degree_main", 380.0, 80.0)),
                frame: Some(descriptor("static", "honor/frame_degree_m_3", 380.0, 80.0)),
                word: Some(descriptor(
                    "assets",
                    "bonds_honor/word/demo_01",
                    180.0,
                    40.0,
                )),
                star: Some(descriptor("static", "honor/icon_degreeLv", 16.0, 16.0)),
                star_high: Some(descriptor("static", "honor/icon_degreeLv6", 16.0, 16.0)),
            },
        };
        let recipe = super::build_general_recipe(
            6,
            StableId(6),
            "general:6",
            &ProfileComponentSnapshot {
                honor_slots: vec![honor.clone()],
                ..ProfileComponentSnapshot::default()
            },
        )
        .unwrap()
        .unwrap();
        let roles = recipe
            .nodes
            .iter()
            .map(|node| node.role.as_str())
            .collect::<Vec<_>>();
        let begin = roles
            .iter()
            .position(|role| role.ends_with("isolation-begin"))
            .unwrap();
        let mask = roles
            .iter()
            .position(|role| role.ends_with("-mask"))
            .unwrap();
        let end = roles
            .iter()
            .position(|role| role.ends_with("isolation-end"))
            .unwrap();
        let frame = roles
            .iter()
            .position(|role| role.ends_with("-frame"))
            .unwrap();
        assert!(begin < mask && mask < end && end < frame);
        assert!(matches!(
            recipe.nodes[mask].payload,
            super::GeneralRecipePayload::Image {
                blend_mode: crate::BlendMode::DstIn,
                ..
            }
        ));
        assert_eq!(
            roles
                .iter()
                .filter(|role| role.contains("-star-") && !role.contains("star-high"))
                .count(),
            5
        );
        assert_eq!(
            roles
                .iter()
                .filter(|role| role.contains("star-high"))
                .count(),
            12
        );
        assert_eq!(
            recipe.nodes[begin].bounds,
            crate::Rect {
                x: -378.0,
                y: -40.0,
                width: 380.0,
                height: 80.0
            }
        );
        let lowered = crate::profile_scene::lower_identity_general(
            6,
            StableId(6),
            "general:6",
            &ProfileComponentSnapshot {
                honor_slots: vec![honor.clone()],
                ..ProfileComponentSnapshot::default()
            },
        )
        .unwrap()
        .unwrap();
        assert!(lowered.commands.iter().any(|command| matches!(
            command.payload,
            crate::SemanticCommandPayload::Composite {
                operation: crate::CompositeOperation::BeginIsolation,
                ..
            }
        )));
        assert!(lowered.commands.iter().any(|command| matches!(
            command.payload,
            crate::SemanticCommandPayload::Composite {
                operation: crate::CompositeOperation::EndIsolation,
                ..
            }
        )));
        let mut without_mask = honor;
        if let crate::profile_scene::HonorVisualKind::Bonds { mask, .. } = &mut without_mask.visual
        {
            *mask = None;
        }
        let unmasked = super::build_general_recipe(
            6,
            StableId(6),
            "general:6",
            &ProfileComponentSnapshot {
                honor_slots: vec![without_mask],
                ..ProfileComponentSnapshot::default()
            },
        )
        .unwrap()
        .unwrap();
        assert!(!unmasked.nodes.iter().any(|node| matches!(
            node.payload,
            super::GeneralRecipePayload::Image {
                blend_mode: crate::BlendMode::DstIn,
                ..
            }
        )));
        assert_eq!(
            unmasked
                .nodes
                .iter()
                .filter(|node| matches!(node.payload, super::GeneralRecipePayload::Group { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn standard_honor_selects_first_available_frame_candidate_without_font() {
        let descriptor = |namespace: &str, key: &str| crate::profile_scene::ResourceDescriptor {
            resource: crate::ResourceKey {
                namespace: namespace.into(),
                key: key.into(),
            },
            natural_width: 180.0,
            natural_height: 80.0,
            provenance: BTreeMap::new(),
        };
        let honor = crate::profile_scene::HonorVisualSnapshot {
            source_field: "userProfile.honorSlots".into(),
            source_id: "7".into(),
            honor_id: 7,
            honor_level: 1,
            full_size: false,
            visual: crate::profile_scene::HonorVisualKind::Standard {
                honor_type: "achievement".into(),
                has_star: false,
                is_live_master: false,
                progress: 0,
                background: None,
                frame_candidates: vec![None, Some(descriptor("static", "honor/frame_degree_s_1"))],
                overlay: None,
                star: None,
                star_high: None,
                live_star_on: None,
                live_star_off: None,
            },
        };
        let recipe = super::build_general_recipe(
            6,
            StableId(6),
            "general:6",
            &ProfileComponentSnapshot {
                honor_slots: vec![honor],
                ..ProfileComponentSnapshot::default()
            },
        )
        .unwrap()
        .unwrap();
        assert!(recipe.nodes.iter().any(|node| matches!(&node.payload, super::GeneralRecipePayload::Image { resource, .. } if resource.key == "honor/frame_degree_s_1")));
    }

    #[test]
    fn story_favorite_recipe_preserves_measured_slots_and_scrolls_only_the_content_group() {
        let favorites = (0..10)
            .map(|index| crate::profile_scene::StoryFavoriteSnapshot {
                story_id: index + 1,
                story_type: "event".into(),
                image: crate::profile_scene::ComponentImageSnapshot {
                    source_field: "userProfile.storyFavorites".into(),
                    source_id: format!("event:{}", index + 1),
                    descriptor: Some(crate::profile_scene::ResourceDescriptor {
                        resource: crate::ResourceKey {
                            namespace: "assets".into(),
                            key: format!("story/{index}"),
                        },
                        natural_width: 400.0,
                        natural_height: 170.0,
                        provenance: BTreeMap::new(),
                    }),
                },
            })
            .collect();
        let snapshot = ProfileComponentSnapshot {
            locale: "en-US".into(),
            region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
            localized_text: BTreeMap::from([(
                "custom_profile.general.story_favorite.title".into(),
                "Favorite stories".into(),
            )]),
            story_favorites: favorites,
            ..ProfileComponentSnapshot::default()
        };
        let recipe = super::build_general_recipe(14, StableId(14), "general:14", &snapshot)
            .unwrap()
            .unwrap();
        assert_eq!(
            recipe
                .nodes
                .iter()
                .filter(|node| node.role.starts_with("story-") && !node.role.contains("scroll"))
                .count(),
            10
        );
        for index in 0..8 {
            let node = recipe
                .nodes
                .iter()
                .find(|node| node.role == format!("story-{index}"))
                .unwrap();
            assert_eq!(
                node.bounds,
                super::layout_rect(crate::profile_layout::STORY_FAVORITE.elements[index + 2])
            );
        }
        let track = recipe
            .nodes
            .iter()
            .find(|node| node.role == "story-scroll-track")
            .unwrap();
        assert_eq!((track.bounds.x, track.bounds.width), (475.5, 4.0));
        assert!(recipe
            .interaction_regions
            .iter()
            .find(|region| region.role == "story-favorite-scroll")
            .unwrap()
            .control_bindings
            .iter()
            .any(|binding| matches!(binding, crate::CommandControlBinding::ScrollViewport { .. })));
        let thumb_region = recipe
            .interaction_regions
            .iter()
            .find(|region| region.role == "story-scroll-thumb")
            .unwrap();
        assert_eq!(
            (thumb_region.bounds.x, thumb_region.bounds.width),
            (475.5, 4.0)
        );
        assert!(thumb_region
            .capabilities
            .iter()
            .any(|value| value == "drag"));
        assert!(thumb_region
            .control_bindings
            .iter()
            .any(|binding| matches!(binding, crate::CommandControlBinding::ScrollThumb { .. })));
        let content = recipe
            .nodes
            .iter()
            .find(|node| node.role == "story-0")
            .unwrap();
        assert!(matches!(
            content.control_bindings[0],
            crate::CommandControlBinding::ScrollContent { .. }
        ));
        assert!(matches!(
            content.clips[0],
            super::GeneralClip::Rect {
                bounds: crate::Rect {
                    x: -424.0,
                    y: -320.0,
                    width: 848.0,
                    height: 756.0
                },
                ..
            }
        ));
        assert_eq!(recipe.controls.len(), 1);
        assert_eq!(
            recipe
                .interaction_regions
                .iter()
                .filter(|region| {
                    region.role.starts_with("story-") && !region.role.contains("scroll")
                })
                .count(),
            10
        );
    }

    #[test]
    fn compact_character_rank_recipe_uses_true_circle_tabs_and_patch_only_scroll_bindings() {
        let ranks = (1..=26)
            .map(|character_id| crate::profile_scene::CharacterRankSnapshot {
                character_id,
                rank: 10 + character_id,
                challenge_rank: Some(20 + character_id),
                avatar: crate::profile_scene::ComponentImageSnapshot {
                    source_field: "userProfile.characterRanks".into(),
                    source_id: character_id.to_string(),
                    descriptor: Some(crate::profile_scene::ResourceDescriptor {
                        resource: crate::ResourceKey {
                            namespace: "static".into(),
                            key: format!("chara_avatar/chara{character_id:02}_02"),
                        },
                        natural_width: 128.0,
                        natural_height: 128.0,
                        provenance: BTreeMap::new(),
                    }),
                },
            })
            .collect();
        let snapshot = ProfileComponentSnapshot {
            locale: "en-US".into(),
            region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
            localized_text: BTreeMap::from([
                (
                    "custom_profile.general.character_rank.title".into(),
                    "Character rank".into(),
                ),
                (
                    "custom_profile.general.character_rank.challenge".into(),
                    "Challenge rank".into(),
                ),
            ]),
            character_ranks: ranks,
            ..ProfileComponentSnapshot::default()
        };
        let recipe = super::build_general_recipe(15, StableId(15), "general:15", &snapshot)
            .unwrap()
            .unwrap();
        assert_eq!(recipe.controls.len(), 2);
        assert_eq!(
            recipe
                .nodes
                .iter()
                .filter(|node| node.role.contains("label-"))
                .count(),
            4
        );
        let avatar = recipe
            .nodes
            .iter()
            .find(|node| node.role == "character-21-avatar")
            .unwrap();
        assert!(matches!(
            avatar.clips[0],
            super::GeneralClip::Ellipse {
                anti_alias: true,
                ..
            }
        ));
        assert_eq!((avatar.bounds.width, avatar.bounds.height), (76.0, 76.0));
        for mode in ["character_rank", "challenge_live_rank"] {
            let number = recipe
                .nodes
                .iter()
                .find(|node| node.role == format!("character-21-{mode}"))
                .unwrap();
            assert!(number.control_bindings.iter().any(|binding| matches!(binding, crate::CommandControlBinding::TabOption { value, .. } if value == mode)));
            assert!(number.control_bindings.iter().any(|binding| matches!(
                binding,
                crate::CommandControlBinding::ScrollContent { .. }
            )));
            assert!(matches!(
                number.clips[0],
                super::GeneralClip::Rect {
                    bounds: crate::Rect {
                        x: -483.5,
                        y: -175.5,
                        width: 967.0,
                        height: 461.5
                    },
                    ..
                }
            ));
        }
        let track = recipe
            .nodes
            .iter()
            .find(|node| node.role == "character-rank-scroll-track")
            .unwrap();
        assert_eq!((track.bounds.x, track.bounds.width), (475.5, 4.0));
        assert!(recipe
            .interaction_regions
            .iter()
            .find(|region| region.role == "character-rank-scroll")
            .unwrap()
            .control_bindings
            .iter()
            .any(|binding| matches!(binding, crate::CommandControlBinding::ScrollViewport { .. })));
        let thumb_region = recipe
            .interaction_regions
            .iter()
            .find(|region| region.role == "character-rank-scroll-thumb")
            .unwrap();
        assert_eq!(
            (thumb_region.bounds.x, thumb_region.bounds.width),
            (475.5, 4.0)
        );
        assert!(thumb_region
            .capabilities
            .iter()
            .any(|value| value == "drag"));
        assert!(thumb_region
            .control_bindings
            .iter()
            .any(|binding| matches!(binding, crate::CommandControlBinding::ScrollThumb { .. })));
        assert_eq!(
            recipe
                .interaction_regions
                .iter()
                .filter(|region| region.resolved_data.contains_key("character_id"))
                .count(),
            26
        );
        assert_eq!(
            recipe
                .nodes
                .iter()
                .filter(|node| node.role.ends_with("-character_rank")
                    || node.role.ends_with("-challenge_live_rank"))
                .count(),
            52
        );
        let final_character = recipe
            .nodes
            .iter()
            .find(|node| node.role == "character-20-character_rank")
            .unwrap();
        assert!((final_character.bounds.y - 517.6).abs() < 0.001);
        let full = super::build_general_recipe(11, StableId(11), "general:11", &snapshot)
            .unwrap()
            .unwrap();
        assert_eq!(full.controls.len(), 1);
        assert!(!full.nodes.iter().any(|node| node.role.contains("scroll")));
        assert_eq!(
            full.interaction_regions
                .iter()
                .filter(|region| region.resolved_data.contains_key("character_id"))
                .count(),
            26
        );
        let full_avatar = full
            .nodes
            .iter()
            .find(|node| node.role == "character-21-avatar")
            .unwrap();
        assert_eq!(full_avatar.bounds.y, -297.0);
        assert_eq!(
            full.nodes
                .iter()
                .find(|node| node.role == "tab-selected-character-rank")
                .unwrap()
                .bounds
                .y,
            -384.0
        );
    }
}
