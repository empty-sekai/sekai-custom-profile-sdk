//! General panel recipes: one backend-neutral node list per panel type.
//!
//! A General panel is one of the fixed card components the game ships (player
//! name, total power, music clear, and so on), each identified by its
//! `general_type` number. [`SUPPORTED_GENERAL_TYPES`] lists the ones this crate
//! builds. Resolving a panel produces a [`GeneralRecipe`]: an ordered list of
//! [`GeneralRecipeNode`]s whose payloads describe images, shapes and text in
//! layout coordinates, with no reference to any raster backend.
//!
//! Panel geometry comes from the measured constants in
//! [`crate::profile_layout`]; text content comes from the snapshot passed in,
//! resolved through [`crate::locale`].

mod card;
mod character_rank;
mod honor;
mod music;
mod nodes;
mod story_favorite;

use serde::{Deserialize, Serialize};

use crate::profile_layout::{CHALLENGE_LIVE, COMMENT, MVP_SUPERSTAR, PLAYER_NAME, TOTAL_POWER};
use crate::profile_scene::{ComponentControlSource, ProfileComponentSnapshot, ProfileResolveError};
use crate::{
    BlendMode, CommandControlBinding, FontRole, InteractionRegionSource, Matrix2d, ParameterValue,
    Rect, ResourceKey, StableId, TextSource,
};

pub use card::{
    build_card_member_overlay_recipe, card_level_source, card_level_text, card_master_rank_key,
    card_rarity_star_count, card_rarity_star_key, card_rarity_suffix, CARD_LEVEL_LOCALIZATION_KEY,
};
pub use nodes::cover_source_rect;

use card::{build_deck_recipe, build_leader_member_recipe};
use character_rank::build_character_rank_recipe;
use honor::build_honors_recipe;
use music::{build_detailed_music_recipe, build_tabbed_music_recipe};
use nodes::{
    count_source, image_node, image_node_without_source, layout_rect, localized_source,
    localized_two_line_sources, shape_node, text_node, RecipeLayer,
};
use story_favorite::build_story_favorite_recipe;

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
    let layer = RecipeLayer {
        layer_id,
        source_key,
    };
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
        3 => build_deck_recipe(layer, snapshot, &mut nodes, &mut interaction_regions)?,
        5 => build_leader_member_recipe(layer, snapshot, &mut nodes, &mut interaction_regions),
        6 => build_honors_recipe(layer, snapshot, &mut nodes, &mut interaction_regions),
        14 => build_story_favorite_recipe(
            layer,
            snapshot,
            &mut nodes,
            &mut controls,
            &mut interaction_regions,
        )?,
        11 | 15 => build_character_rank_recipe(
            general_type,
            layer,
            snapshot,
            &mut nodes,
            &mut controls,
            &mut interaction_regions,
        )?,
        2 => {
            let title_key = "custom_profile.general.total_power.title";
            nodes.push(text_node(
                layer.key("title", 0),
                layout_rect(TOTAL_POWER.elements[0]),
                localized_source(snapshot, title_key)?,
                TOTAL_POWER.elements[0].h,
                [0.33, 0.33, 0.33, 1.0],
                GeneralTextAlign::Left,
            ));
            nodes.push(text_node(
                layer.key("divider", 1),
                layout_rect(TOTAL_POWER.elements[2]),
                TextSource::Authored { value: "|".into() },
                TOTAL_POWER.elements[2].h,
                [0.67, 0.67, 0.67, 1.0],
                GeneralTextAlign::Left,
            ));
            nodes.push(text_node(
                layer.key("value", 2),
                layout_rect(TOTAL_POWER.elements[3]),
                TextSource::ProfileField {
                    field: "totalPower.totalPower".into(),
                    value: snapshot.total_power.to_string(),
                },
                TOTAL_POWER.elements[3].h,
                [0.2, 0.2, 0.2, 1.0],
                GeneralTextAlign::Left,
            ));
        }
        13 => {
            nodes.push(shape_node(
                layer.key("textbox", 0),
                layout_rect(PLAYER_NAME.elements[0]),
                GeneralGeometry::RoundedRect { radius: [8.0, 8.0] },
                [0.922, 0.922, 0.941, 1.0],
            ));
            nodes.push(text_node(
                layer.key("player-name", 1),
                layout_rect(PLAYER_NAME.elements[1]),
                TextSource::ProfileField {
                    field: "user.name".into(),
                    value: snapshot.user_name.clone(),
                },
                PLAYER_NAME.elements[1].h,
                [0.2, 0.2, 0.2, 1.0],
                GeneralTextAlign::Left,
            ));
        }
        4 => {
            let textbox = COMMENT.elements[0];
            nodes.push(shape_node(
                layer.key("textbox", 0),
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
                layer.key("title", 1),
                layout_rect(COMMENT.elements[1]),
                TextSource::Localized {
                    key: key.into(),
                    locale: snapshot.locale.clone(),
                    value,
                },
                COMMENT.elements[1].h,
                [0.53, 0.53, 0.53, 1.0],
                GeneralTextAlign::Center,
            ));
            let content = crate::profile_layout::ElementLayout {
                cx: textbox.cx,
                cy: textbox.cy,
                w: textbox.w - 32.0,
                h: textbox.h - 36.0,
            };
            let mut content_node = text_node(
                layer.key("content", 2),
                layout_rect(content),
                TextSource::ProfileField {
                    field: "userProfile.word".into(),
                    value: snapshot.word.clone(),
                },
                26.0,
                [0.2, 0.2, 0.2, 1.0],
                GeneralTextAlign::Left,
            );
            if let GeneralRecipePayload::Text {
                line_spacing,
                wrap,
                render_baseline,
                ..
            } = &mut content_node.payload
            {
                *line_spacing = 6.0;
                *wrap = true;
                // The measured General rectangle is a draw-anchor contract.
                // The first-line baseline is the textbox top + 18px padding +
                // 26px font size, expressed in TMP-local units.
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
                layer.key("divider", 0),
                layout_rect(divider),
                GeneralGeometry::Rect,
                [0.78, 0.78, 0.78, 0.5],
            ));
            nodes.push(text_node(
                layer.key("title", 1),
                layout_rect(MVP_SUPERSTAR.elements[0]),
                localized_source(snapshot, "custom_profile.general.multiplayer_live.title")?,
                MVP_SUPERSTAR.elements[0].h,
                [0.33, 0.33, 0.33, 1.0],
                GeneralTextAlign::Center,
            ));
            nodes.push(shape_node(
                layer.key("mvp-background", 2),
                layout_rect(MVP_SUPERSTAR.elements[1]),
                GeneralGeometry::RoundedRect {
                    radius: [16.0, 16.0],
                },
                [0.62, 0.62, 0.65, 1.0],
            ));
            nodes.push(text_node(
                layer.key("mvp-label", 3),
                layout_rect(MVP_SUPERSTAR.elements[1]),
                localized_source(snapshot, "custom_profile.general.multiplayer_live.mvp")?,
                MVP_SUPERSTAR.elements[1].h * 0.55,
                [1.0; 4],
                GeneralTextAlign::Center,
            ));
            nodes.push(text_node(
                layer.key("mvp-count", 4),
                layout_rect(MVP_SUPERSTAR.elements[2]),
                count_source(snapshot, snapshot.mvp)?,
                MVP_SUPERSTAR.elements[4].h,
                [0.27, 0.27, 0.27, 1.0],
                GeneralTextAlign::Left,
            ));
            nodes.push(shape_node(
                layer.key("superstar-background", 5),
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
                layer.key("superstar-upper", 6),
                layout_rect(upper),
                superstar_upper,
                18.0,
                [1.0; 4],
                GeneralTextAlign::Center,
            ));
            let lower = crate::profile_layout::ElementLayout {
                cy: MVP_SUPERSTAR.elements[3].cy - 10.0,
                h: 18.0,
                ..MVP_SUPERSTAR.elements[3]
            };
            nodes.push(text_node(
                layer.key("superstar-lower", 7),
                layout_rect(lower),
                superstar_lower,
                18.0,
                [1.0; 4],
                GeneralTextAlign::Center,
            ));
            nodes.push(text_node(
                layer.key("superstar-count", 8),
                layout_rect(MVP_SUPERSTAR.elements[4]),
                count_source(snapshot, snapshot.superstar)?,
                MVP_SUPERSTAR.elements[4].h,
                [0.27, 0.27, 0.27, 1.0],
                GeneralTextAlign::Left,
            ));
        }
        10 => {
            let els = &CHALLENGE_LIVE.elements;
            let divider = crate::profile_layout::ElementLayout { h: 2.0, ..els[4] };
            nodes.push(shape_node(
                layer.key("divider", 0),
                layout_rect(divider),
                GeneralGeometry::Rect,
                [0.78, 0.78, 0.78, 0.5],
            ));
            nodes.push(text_node(
                layer.key("title", 1),
                layout_rect(els[0]),
                localized_source(snapshot, "custom_profile.general.challenge_live.title")?,
                els[0].h,
                [0.33, 0.33, 0.33, 1.0],
                GeneralTextAlign::Center,
            ));
            nodes.push(shape_node(
                layer.key("solo-background", 2),
                layout_rect(els[1]),
                GeneralGeometry::RoundedRect {
                    radius: [16.0, 16.0],
                },
                [0.62, 0.62, 0.65, 1.0],
            ));
            nodes.push(text_node(
                layer.key("solo-label", 3),
                layout_rect(els[1]),
                localized_source(snapshot, "custom_profile.general.challenge_live.solo")?,
                els[1].h * 0.55,
                [1.0; 4],
                GeneralTextAlign::Center,
            ));
            let avatar_bounds = layout_rect(els[2]);
            if let Some(image) = snapshot.challenge_avatar.as_ref() {
                if let Some(descriptor) = image.descriptor.as_ref() {
                    nodes.push(image_node(
                        layer.key("challenge-avatar", 4),
                        avatar_bounds,
                        image,
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
                        layer.key("challenge-avatar", 4),
                        avatar_bounds,
                        GeneralGeometry::Ellipse,
                        [0.87, 0.87, 0.87, 1.0],
                    ));
                }
                interaction_regions.push(layer.item_region(
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
            }
            nodes.push(text_node(
                layer.key("challenge-score", 5),
                layout_rect(els[3]),
                TextSource::ProfileField {
                    field: "userProfile.challengeScore".into(),
                    value: snapshot.challenge_score.to_string(),
                },
                els[3].h,
                [0.27, 0.27, 0.27, 1.0],
                GeneralTextAlign::Left,
            ));
        }
        12 => build_detailed_music_recipe(layer, snapshot, &mut nodes)?,
        16 => build_tabbed_music_recipe(
            layer,
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
                layer.key("background", 0),
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
                layer.key("rank-icon", 1),
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
                layer.key("label", 2),
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
            ));
            nodes.push(text_node(
                layer.key("rank", 3),
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
                layer.key("avatar-background", 0),
                avatar_bounds,
                GeneralGeometry::Ellipse,
                [0.85, 0.85, 0.9, 1.0],
            ));
            if let Some(image) = snapshot.player_avatar.as_ref() {
                if let Some(descriptor) = image.descriptor.as_ref() {
                    nodes.push(image_node(
                        layer.key("avatar", 1),
                        avatar_bounds,
                        image,
                        descriptor,
                        GeneralImageSource::Pixels(cover_source_rect(
                            descriptor.natural_width,
                            descriptor.natural_height,
                            avatar_bounds.width,
                            avatar_bounds.height,
                        )),
                        GeneralImageSourceConstraint::Fast,
                        vec![GeneralClip::Ellipse {
                            bounds: avatar_bounds,
                            anti_alias: true,
                        }],
                    ));
                }
                interaction_regions.push(layer.item_region(
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
                layer.key("avatar-border", 2),
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

fn recipe_node_id(source_key: &str, role: &str, ordinal: u32) -> StableId {
    StableId::derive(
        "general-recipe-node-v1",
        format!("{source_key}\0{role}\0{ordinal}").as_bytes(),
    )
}

#[cfg(test)]
mod tests;
