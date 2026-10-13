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

use serde::{Deserialize, Serialize};

use crate::profile_layout::{
    CHALLENGE_LIVE, COMMENT, MUSIC_CLEAR, MUSIC_CLEAR_TAB, MVP_SUPERSTAR, PLAYER_NAME, TOTAL_POWER,
};
use crate::profile_scene::{
    CardVisualSnapshot, ComponentControlSource, ComponentControlState, ProfileComponentSnapshot,
    ProfileResolveError,
};
use crate::{
    BlendMode, CommandControlBinding, FontRole, InteractionRegionSource, Matrix2d, ParameterValue,
    Rect, ResourceKey, StableId, TextSource,
};

pub const SUPPORTED_GENERAL_TYPES: [i32; 15] =
    [2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18];

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

/// Localization key of the card level caption (`Lv.{level}`).
pub const CARD_LEVEL_LOCALIZATION_KEY: &str = "custom_profile.general.card_level";

/// Formats a card level caption from its localized template. Levels below 10
/// are preceded by two spaces, as the game's card view writes them.
pub fn card_level_text(template: &str, level: i32) -> String {
    let level = if level <= 9 {
        format!("  {level}")
    } else {
        level.to_string()
    };
    template.replace("{level}", &level)
}

/// Card level caption as a localized text source.
pub fn card_level_source(locale: &str, template: &str, level: i32) -> TextSource {
    TextSource::Localized {
        key: CARD_LEVEL_LOCALIZATION_KEY.into(),
        locale: locale.into(),
        value: card_level_text(template, level),
    }
}

/// Builds the authored `CardMember` information overlay from the same recipes
/// used by General type 3 (cropped deck card) and type 5 (full leader card).
/// The artwork remains a separate authored command so `showMasterRank=false`
/// can preserve the original image-only contract. `level` is the caption the
/// cropped (type 1) overlay draws; see [`card_level_source`].
pub fn build_card_member_overlay_recipe(
    member_type: i32,
    layer_id: StableId,
    source_key: &str,
    bounds: Rect,
    card: &CardVisualSnapshot,
    level: TextSource,
) -> Vec<GeneralRecipeNode> {
    let mut nodes = Vec::new();
    if member_type == 1 {
        let scale = (bounds.width / 312.0).min(bounds.height / 512.0);
        push_cropped_card_overlay_nodes(
            layer_id,
            source_key,
            "card-member",
            1,
            bounds,
            scale,
            card,
            level,
            Vec::new(),
            &mut nodes,
        );
    } else {
        push_full_card_overlay_nodes(
            layer_id,
            source_key,
            "card-member",
            1,
            bounds,
            card,
            &mut nodes,
        );
    }
    nodes
}

fn build_deck_recipe(
    layer_id: StableId,
    source_key: &str,
    snapshot: &ProfileComponentSnapshot,
    nodes: &mut Vec<GeneralRecipeNode>,
    interaction_regions: &mut Vec<InteractionRegionSource>,
) -> Result<(), ProfileResolveError> {
    const SLOT_COUNT: usize = 5;
    const CARD_WIDTH: f32 = 312.0;
    const CARD_HEIGHT: f32 = 512.0;

    let container = layout_rect(crate::profile_layout::DECK.elements[0]);
    let slot_width = container.width / SLOT_COUNT as f32;
    let scale = (slot_width / CARD_WIDTH).min(container.height / CARD_HEIGHT);
    let card_width = CARD_WIDTH * scale;
    let card_height = CARD_HEIGHT * scale;
    let level_template = snapshot
        .localized_text
        .get(CARD_LEVEL_LOCALIZATION_KEY)
        .cloned()
        .ok_or_else(|| {
            ProfileResolveError::MissingLocalizedText(CARD_LEVEL_LOCALIZATION_KEY.into())
        })?;

    for slot_index in 0..SLOT_COUNT {
        let slot = Rect {
            x: container.x + slot_index as f32 * slot_width,
            width: slot_width,
            ..container
        };
        let center_x = slot.x + slot.width / 2.0;
        let center_y = container.y + container.height / 2.0;
        let card_bounds = Rect {
            x: center_x - card_width / 2.0,
            y: center_y - card_height / 2.0,
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
                let font_size = (container.height * 0.12).max(10.0);
                let line_width = (slot_width - 12.0).max(1.0);
                let center_x = slot.x + 6.0 + line_width / 2.0;
                let info_center_y = slot.y + container.height - 10.0 - font_size - 2.0;
                let level_center_y = slot.y + container.height - 10.0;
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
                    card_level_source(&snapshot.locale, &level_template, card.level),
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
        push_cropped_card_overlay_nodes(
            layer_id,
            source_key,
            &format!("deck-slot-{slot_index}"),
            base_ordinal + 1,
            card_bounds,
            scale,
            card,
            card_level_source(&snapshot.locale, &level_template, card.level),
            clip,
            nodes,
        );
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
                        ParameterValue::I64(challenge_rank.into()),
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
    // Every slot is drawn and slots nobody shares stay empty. Slots past the
    // measured ones continue the two-column grid.
    let measured_slots = &panel.elements[2..];
    let slot_count = measured_slots.len().max(snapshot.story_favorites.len());
    for index in 0..slot_count {
        let target = measured_slots.get(index).copied().unwrap_or_else(|| {
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
        let Some(favorite) = snapshot
            .story_favorites
            .get(index)
            .filter(|favorite| favorite.story_id > 0)
        else {
            nodes.push(shape_node(
                layer_id,
                source_key,
                &format!("{role}-empty"),
                2 + index as u32,
                bounds,
                GeneralGeometry::RoundedRect { radius: [8.0, 8.0] },
                [0.82, 0.82, 0.82, 0.4],
            ));
            continue;
        };
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
                ..
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
    } else if has_star {
        build_honor_star_nodes(
            layer_id,
            source_key,
            honor,
            star,
            star_high,
            bounds,
            base + 16,
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

fn transform_card_rect(center_x: f32, center_y: f32, scale: f32, rect: Rect) -> Rect {
    Rect {
        x: center_x + rect.x * scale,
        y: center_y + rect.y * scale,
        width: rect.width * scale,
        height: rect.height * scale,
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

#[allow(clippy::too_many_arguments)]
fn push_cropped_card_overlay_nodes(
    layer_id: StableId,
    source_key: &str,
    role_prefix: &str,
    first_ordinal: u32,
    card_bounds: Rect,
    scale: f32,
    card: &CardVisualSnapshot,
    level_source: TextSource,
    clips: Vec<GeneralClip>,
    nodes: &mut Vec<GeneralRecipeNode>,
) {
    const CARD_WIDTH: f32 = 312.0;
    const CARD_HEIGHT: f32 = 512.0;
    const RAW_ROOT_WIDTH: f32 = 328.0;
    const RAW_ROOT_HEIGHT: f32 = 520.0;
    const BAR_HEIGHT: f32 = 56.39 * CARD_HEIGHT / RAW_ROOT_HEIGHT;
    const LEVEL_FONT_SIZE: f32 = 24.0;

    let center_x = card_bounds.x + card_bounds.width / 2.0;
    let center_y = card_bounds.y + card_bounds.height / 2.0;
    let transform = |rect| transform_card_rect(center_x, center_y, scale, rect);
    let bar_bounds = transform(Rect {
        x: -CARD_WIDTH / 2.0,
        y: CARD_HEIGHT / 2.0 - BAR_HEIGHT,
        width: CARD_WIDTH,
        height: BAR_HEIGHT,
    });
    nodes.push(static_styled_image_node(
        layer_id,
        source_key,
        &format!("{role_prefix}-level-bar"),
        first_ordinal,
        bar_bounds,
        "card/bg_base_wh".into(),
        [68.0 / 255.0, 68.0 / 255.0, 102.0 / 255.0, 1.0],
        clips.clone(),
    ));
    let level_font_size = LEVEL_FONT_SIZE * scale;
    let level_left = card_bounds.x + (12.9 * CARD_WIDTH / RAW_ROOT_WIDTH) * scale;
    nodes.push(text_node(
        layer_id,
        source_key,
        &format!("{role_prefix}-level"),
        first_ordinal + 1,
        Rect {
            x: level_left,
            y: bar_bounds.y + (bar_bounds.height - level_font_size) / 2.0,
            width: (card_bounds.x + card_bounds.width - level_left).max(1.0),
            height: level_font_size,
        },
        level_source,
        level_font_size,
        [1.0; 4],
        GeneralTextAlign::Left,
        0.0,
        false,
    ));
    nodes.push(static_styled_image_node(
        layer_id,
        source_key,
        &format!("{role_prefix}-frame"),
        first_ordinal + 2,
        card_bounds,
        format!("card/cardFrame_M_{}", card_rarity_suffix(&card.rarity)),
        [1.0; 4],
        clips.clone(),
    ));
    nodes.push(static_styled_image_node(
        layer_id,
        source_key,
        &format!("{role_prefix}-attribute"),
        first_ordinal + 3,
        transform(Rect {
            x: -CARD_WIDTH / 2.0 + 8.0 * CARD_WIDTH / RAW_ROOT_WIDTH,
            y: -CARD_HEIGHT / 2.0,
            width: 64.0 * CARD_WIDTH / RAW_ROOT_WIDTH,
            height: 68.0 * CARD_HEIGHT / RAW_ROOT_HEIGHT,
        }),
        format!("card/icon_attribute_{}_64", card.attribute),
        [1.0; 4],
        clips.clone(),
    ));
    for star_index in 0..card_rarity_star_count(&card.rarity) {
        nodes.push(static_styled_image_node(
            layer_id,
            source_key,
            &format!("{role_prefix}-star-{star_index}"),
            first_ordinal + 4 + star_index as u32,
            transform(Rect {
                x: -CARD_WIDTH / 2.0 + 5.0 + star_index as f32 * 40.0,
                y: CARD_HEIGHT / 2.0 - 64.0 - 40.0,
                width: 40.0,
                height: 40.0,
            }),
            card_rarity_star_key(&card.rarity, card.after_training).into(),
            [1.0; 4],
            clips.clone(),
        ));
    }
    if let Some(key) = card_master_rank_key(card.master_rank) {
        nodes.push(static_styled_image_node(
            layer_id,
            source_key,
            &format!("{role_prefix}-master-rank"),
            first_ordinal + 15,
            transform(Rect {
                x: CARD_WIDTH / 2.0
                    - 88.0 * CARD_WIDTH / RAW_ROOT_WIDTH
                    - 1.4 * CARD_WIDTH / RAW_ROOT_WIDTH,
                y: CARD_HEIGHT / 2.0
                    - 88.0 * CARD_HEIGHT / RAW_ROOT_HEIGHT
                    - 0.8 * CARD_HEIGHT / RAW_ROOT_HEIGHT,
                width: 88.0 * CARD_WIDTH / RAW_ROOT_WIDTH,
                height: 88.0 * CARD_HEIGHT / RAW_ROOT_HEIGHT,
            }),
            key,
            [1.0; 4],
            clips,
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn push_full_card_overlay_nodes(
    layer_id: StableId,
    source_key: &str,
    role_prefix: &str,
    first_ordinal: u32,
    bounds: Rect,
    card: &CardVisualSnapshot,
    nodes: &mut Vec<GeneralRecipeNode>,
) {
    let layout = &crate::profile_layout::LEADER_MEMBER;
    let artwork = layout_rect(layout.elements[0]);
    let stars = layout_rect(layout.elements[1]);
    let attribute = layout_rect(layout.elements[2]);
    let master_rank = layout_rect(layout.elements[3]);
    let right_offset = |rect: Rect| artwork.x + artwork.width - rect.x;
    let bottom_offset = |rect: Rect| artwork.y + artwork.height - rect.y;
    nodes.push(static_overlay_image_node(
        layer_id,
        source_key,
        &format!("{role_prefix}-frame"),
        first_ordinal,
        bounds,
        format!("card/cardFrame_L_{}", card_rarity_suffix(&card.rarity)),
    ));
    nodes.push(static_overlay_image_node(
        layer_id,
        source_key,
        &format!("{role_prefix}-attribute"),
        first_ordinal + 1,
        Rect {
            x: bounds.x + bounds.width - right_offset(attribute),
            y: bounds.y + (attribute.y - artwork.y),
            width: attribute.width,
            height: attribute.height,
        },
        format!("card/icon_attribute_{}_88", card.attribute),
    ));
    const STAR_SLOTS: usize = 4;
    let star_count = card_rarity_star_count(&card.rarity);
    let star_size = stars.width;
    let star_pitch = (stars.height - star_size) / (STAR_SLOTS - 1) as f32;
    let star_start_y = bounds.y + bounds.height - bottom_offset(stars)
        + (STAR_SLOTS.saturating_sub(star_count)) as f32 * star_pitch;
    for index in 0..star_count {
        nodes.push(static_overlay_image_node(
            layer_id,
            source_key,
            &format!("{role_prefix}-star-{index}"),
            first_ordinal + 2 + index as u32,
            Rect {
                x: bounds.x + (stars.x - artwork.x),
                y: star_start_y + index as f32 * star_pitch,
                width: star_size,
                height: star_size,
            },
            card_rarity_star_key(&card.rarity, card.after_training).into(),
        ));
    }
    if let Some(key) = card_master_rank_key(card.master_rank) {
        nodes.push(static_overlay_image_node(
            layer_id,
            source_key,
            &format!("{role_prefix}-master-rank"),
            first_ordinal + 2 + star_count as u32,
            Rect {
                x: bounds.x + bounds.width - right_offset(master_rank),
                y: bounds.y + bounds.height - bottom_offset(master_rank),
                width: master_rank.width,
                height: master_rank.height,
            },
            key,
        ));
    }
}

fn build_leader_member_recipe(
    layer_id: StableId,
    source_key: &str,
    snapshot: &ProfileComponentSnapshot,
    nodes: &mut Vec<GeneralRecipeNode>,
    interaction_regions: &mut Vec<InteractionRegionSource>,
) {
    let cover = layout_rect(crate::profile_layout::LEADER_MEMBER.elements[0]);
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

    push_full_card_overlay_nodes(layer_id, source_key, "leader", 1, cover, card, nodes);
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

/// Source rectangle that covers a `target_width` x `target_height` box with a
/// `source_width` x `source_height` image, centred on both axes.
pub fn cover_source_rect(
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

/// Frame sprite suffix of a `cardRarityType`: `bd` for birthday cards,
/// otherwise the rarity number.
pub fn card_rarity_suffix(rarity: &str) -> &str {
    if rarity == "rarity_birthday" {
        "bd"
    } else {
        rarity.rsplit('_').next().unwrap_or("1")
    }
}

/// Number of rarity stars a card shows: one for birthday cards, otherwise
/// the rarity number.
pub fn card_rarity_star_count(rarity: &str) -> usize {
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

/// Rarity star sprite key. `trained` selects the trained star.
pub fn card_rarity_star_key(rarity: &str, trained: bool) -> &'static str {
    if rarity == "rarity_birthday" {
        "card/rarity_birthday"
    } else if trained {
        "card/rarity_star_afterTraining"
    } else {
        "card/rarity_star_normal"
    }
}

/// Master-rank badge sprite key. Card overlays hide the badge at rank 0.
pub fn card_master_rank_key(master_rank: i32) -> Option<String> {
    (master_rank >= 1).then(|| format!("card/masterRank_L_{}", master_rank.min(5)))
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
    let colors = DIFFICULTY_LABEL_COLORS;
    let mut ordinal = 0u32;
    for (difficulty_index, (role, label_index, number_index, values)) in
        difficulties.iter().enumerate()
    {
        let label = MUSIC_CLEAR_TAB.elements[*label_index];
        let mut background = shape_node(
            layer_id,
            source_key,
            &format!("{role}-label-background"),
            ordinal,
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
            layer_id,
            source_key,
            &format!("{role}-label"),
            ordinal,
            layout_rect(label),
            localized_source(
                snapshot,
                &format!("custom_profile.general.music.difficulty.{role}"),
            )?,
            label.h * 0.55,
            [1.0; 4],
            GeneralTextAlign::Center,
            0.0,
            false,
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
                layer_id,
                source_key,
                &format!("{role}-{option}-count"),
                ordinal,
                layout_rect(number),
                TextSource::ProfileField {
                    field: format!("userMusicDifficultyClearCount.{role}.{field}"),
                    value: value.to_string(),
                },
                number.h,
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
mod tests;
