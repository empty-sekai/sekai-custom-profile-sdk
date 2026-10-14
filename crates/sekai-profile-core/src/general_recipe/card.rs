//! Card overlays, the deck (type 3) and leader member (type 5) panels, the card
//! level caption, and rarity and master-rank sprite keys.

use super::nodes::{
    cover_source_rect, image_node, item_region, layout_rect, shape_node, text_node, NodeKey,
    RecipeLayer,
};
use super::{
    GeneralClip, GeneralGeometry, GeneralImageSampling, GeneralImageSource,
    GeneralImageSourceConstraint, GeneralRecipeNode, GeneralRecipePayload, GeneralStroke,
    GeneralTextAlign,
};
use crate::profile_scene::{CardVisualSnapshot, ProfileComponentSnapshot, ProfileResolveError};
use crate::{
    BlendMode, InteractionRegionSource, ParameterValue, Rect, ResourceKey, StableId, TextSource,
};

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
    let overlay = CardOverlay {
        layer: RecipeLayer {
            layer_id,
            source_key,
        },
        role_prefix: "card-member",
        first_ordinal: 1,
        card,
    };
    if member_type == 1 {
        let scale = (bounds.width / 312.0).min(bounds.height / 512.0);
        overlay.push_cropped(bounds, scale, level, Vec::new(), &mut nodes);
    } else {
        overlay.push_full(bounds, &mut nodes);
    }
    nodes
}

pub(super) fn build_deck_recipe(
    layer_id: StableId,
    source_key: &str,
    snapshot: &ProfileComponentSnapshot,
    nodes: &mut Vec<GeneralRecipeNode>,
    interaction_regions: &mut Vec<InteractionRegionSource>,
) -> Result<(), ProfileResolveError> {
    let layer = RecipeLayer {
        layer_id,
        source_key,
    };
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
                layer.key(&format!("deck-slot-{slot_index}-artwork"), base_ordinal),
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
                    layer.key(&format!("deck-slot-{slot_index}-card-id"), base_ordinal + 1),
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
                ));
                nodes.push(text_node(
                    layer.key(&format!("deck-slot-{slot_index}-level"), base_ordinal + 2),
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
            layer.key(&format!("deck-slot-{slot_index}-artwork"), base_ordinal),
            card_bounds,
            &card.image,
            descriptor,
            GeneralImageSource::Pixels(Rect {
                x: (descriptor.natural_width - CARD_WIDTH).max(0.0) / 2.0,
                y: 0.0,
                width: crop_width,
                height: crop_height,
            }),
            GeneralImageSourceConstraint::Fast,
            clip.clone(),
        ));
        CardOverlay {
            layer,
            role_prefix: &format!("deck-slot-{slot_index}"),
            first_ordinal: base_ordinal + 1,
            card,
        }
        .push_cropped(
            card_bounds,
            scale,
            card_level_source(&snapshot.locale, &level_template, card.level),
            clip,
            nodes,
        );
    }
    Ok(())
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

/// The information overlay of one card: the frame, attribute icon, rarity
/// stars and master-rank badge drawn over its artwork. Node roles start with
/// `role_prefix`, and node ordinals count up from `first_ordinal`.
#[derive(Clone, Copy)]
struct CardOverlay<'a> {
    layer: RecipeLayer<'a>,
    role_prefix: &'a str,
    first_ordinal: u32,
    card: &'a CardVisualSnapshot,
}

impl CardOverlay<'_> {
    /// Overlay of a card cropped to its 312 x 512 artwork and drawn into
    /// `card_bounds` at `scale`, with a level bar captioned `level_source`.
    /// The sprites share `clips`; the caption is not clipped.
    fn push_cropped(
        self,
        card_bounds: Rect,
        scale: f32,
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

        let CardOverlay {
            layer,
            role_prefix,
            first_ordinal,
            card,
        } = self;
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
            layer.key(&format!("{role_prefix}-level-bar"), first_ordinal),
            bar_bounds,
            "card/bg_base_wh".into(),
            [68.0 / 255.0, 68.0 / 255.0, 102.0 / 255.0, 1.0],
            clips.clone(),
        ));
        let level_font_size = LEVEL_FONT_SIZE * scale;
        let level_left = card_bounds.x + (12.9 * CARD_WIDTH / RAW_ROOT_WIDTH) * scale;
        nodes.push(text_node(
            layer.key(&format!("{role_prefix}-level"), first_ordinal + 1),
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
        ));
        nodes.push(static_styled_image_node(
            layer.key(&format!("{role_prefix}-frame"), first_ordinal + 2),
            card_bounds,
            format!("card/cardFrame_M_{}", card_rarity_suffix(&card.rarity)),
            [1.0; 4],
            clips.clone(),
        ));
        nodes.push(static_styled_image_node(
            layer.key(&format!("{role_prefix}-attribute"), first_ordinal + 3),
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
                layer.key(
                    &format!("{role_prefix}-star-{star_index}"),
                    first_ordinal + 4 + star_index as u32,
                ),
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
                layer.key(&format!("{role_prefix}-master-rank"), first_ordinal + 15),
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

    /// Overlay of a full card drawn into `bounds`, laid out as the leader
    /// member panel lays it out.
    fn push_full(self, bounds: Rect, nodes: &mut Vec<GeneralRecipeNode>) {
        let CardOverlay {
            layer,
            role_prefix,
            first_ordinal,
            card,
        } = self;
        let layout = &crate::profile_layout::LEADER_MEMBER;
        let artwork = layout_rect(layout.elements[0]);
        let stars = layout_rect(layout.elements[1]);
        let attribute = layout_rect(layout.elements[2]);
        let master_rank = layout_rect(layout.elements[3]);
        let right_offset = |rect: Rect| artwork.x + artwork.width - rect.x;
        let bottom_offset = |rect: Rect| artwork.y + artwork.height - rect.y;
        nodes.push(static_overlay_image_node(
            layer.key(&format!("{role_prefix}-frame"), first_ordinal),
            bounds,
            format!("card/cardFrame_L_{}", card_rarity_suffix(&card.rarity)),
        ));
        nodes.push(static_overlay_image_node(
            layer.key(&format!("{role_prefix}-attribute"), first_ordinal + 1),
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
                layer.key(
                    &format!("{role_prefix}-star-{index}"),
                    first_ordinal + 2 + index as u32,
                ),
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
                layer.key(
                    &format!("{role_prefix}-master-rank"),
                    first_ordinal + 2 + star_count as u32,
                ),
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
}

pub(super) fn build_leader_member_recipe(
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
    let cover = layout_rect(crate::profile_layout::LEADER_MEMBER.elements[0]);
    let Some(card) = snapshot.leader_card.as_ref() else {
        nodes.push(shape_node(
            layer.key("leader-artwork", 0),
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
            layer.key("leader-artwork", 0),
            cover,
            GeneralGeometry::Rect,
            [0.4, 0.4, 0.4, 1.0],
        ));
        return;
    };

    nodes.push(image_node(
        layer.key("leader-artwork", 0),
        cover,
        &card.image,
        descriptor,
        GeneralImageSource::Pixels(cover_source_rect(
            descriptor.natural_width,
            descriptor.natural_height,
            cover.width,
            cover.height,
        )),
        GeneralImageSourceConstraint::Fast,
        Vec::new(),
    ));

    CardOverlay {
        layer,
        role_prefix: "leader",
        first_ordinal: 1,
        card,
    }
    .push_full(cover, nodes);
}

fn static_overlay_image_node(key: NodeKey<'_>, bounds: Rect, sprite: String) -> GeneralRecipeNode {
    static_styled_image_node(key, bounds, sprite, [1.0; 4], Vec::new())
}

/// Bundled `static` sprite tinted with `tint` and clipped by `clips`.
fn static_styled_image_node(
    key: NodeKey<'_>,
    bounds: Rect,
    sprite: String,
    tint: [f32; 4],
    clips: Vec<GeneralClip>,
) -> GeneralRecipeNode {
    GeneralRecipeNode {
        clips,
        ..key.node(
            bounds,
            GeneralRecipePayload::Image {
                resource: ResourceKey {
                    namespace: "static".into(),
                    key: sprite,
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
        )
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
