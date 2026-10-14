//! Snapshot types: the resolved player data a recipe builder reads.
//!
//! Raw profile JSON carries optional fields, per-region shapes and references
//! that still have to be looked up. Lowering it produces a
//! [`ProfileComponentSnapshot`], where every value a panel can draw is already
//! present and typed. Recipe builders in [`crate::general_recipe`] read only
//! this, so they never touch masterdata or the API payload directly.

mod commands;
mod honor;
mod keys;
mod layer;
mod primary;
mod recipe;
mod snapshot;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use keys::{
    authored_kind_name, card_member_lookup_key, collection_normal_map_lookup_key,
    component_control_id, interaction_region_id, resource_lookup_key, semantic_command_id,
};
pub use layer::{
    assemble_profile_layer, default_profile_interaction_region, rect_quad, union_command_bounds,
};
pub use recipe::lower_identity_general;
pub use snapshot::{
    CardVisualSnapshot, CharacterRankSnapshot, ComponentControlSource, ComponentControlState,
    ComponentImageSnapshot, HonorVisualKind, HonorVisualSnapshot, MusicDifficultySnapshot,
    MusicResultsSnapshot, NormalizedProfileElement, ProfileComponentLowering,
    ProfileComponentSnapshot, ProfileElementRef, ProfileResolveSnapshot, ResourceDescriptor,
    StoryFavoriteSnapshot,
};

use commands::LayerCommands;
use honor::{honor_interaction_region, lower_honor_visual};
use primary::{composite_primary, lower_primary_command, PrimaryContext};
use recipe::{card_member_level_source, general_recipe_node_to_command};

use crate::profile_source::CustomProfileCard;
use crate::{
    AuthoredElementKind, InteractionRegionSource, LayerKind, LayerSource, ParameterValue, Rect,
    SemanticCommandSource, StableId,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedProfileScene {
    pub layers: Vec<LayerSource>,
    pub commands: Vec<SemanticCommandSource>,
    pub interaction_regions: Vec<InteractionRegionSource>,
    pub controls: Vec<ComponentControlSource>,
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum ProfileResolveError {
    #[error("missing region fontId={0}")]
    MissingRegionFont(i32),
    #[error("missing localized renderer text {0}")]
    MissingLocalizedText(String),
    #[error("missing resolved resource descriptor {0}")]
    MissingResource(String),
    #[error("invalid general recipe: {0}")]
    InvalidRecipe(String),
}

pub fn ordered_profile_elements<'a>(
    card: &'a CustomProfileCard,
    document_key: &str,
) -> Vec<NormalizedProfileElement<'a>> {
    let mut values = Vec::with_capacity(card.element_count());
    macro_rules! append {
        ($field:ident, $kind:ident, $variant:ident) => {
            values.extend(card.$field.iter().enumerate().map(|(source_index, value)| {
                normalized(
                    document_key,
                    AuthoredElementKind::$kind,
                    source_index,
                    ProfileElementRef::$variant(value),
                )
            }));
        };
    }
    append!(texts, Text, Text);
    append!(shapes, Shape, Shape);
    append!(card_members, CardMember, CardMember);
    append!(stamps, Stamp, Stamp);
    append!(others, Other, Other);
    append!(bonds_honors, BondsHonor, BondsHonor);
    append!(honors, Honor, Honor);
    append!(collections, Collection, Collection);
    append!(generals, General, General);
    append!(stand_members, StandMember, StandMember);
    append!(general_backgrounds, GeneralBackground, GeneralBackground);
    append!(story_backgrounds, StoryBackground, StoryBackground);
    append!(character_icons, CharacterIcon, CharacterIcon);
    append!(materials, Material, Material);
    append!(user_interface_icons, UserInterfaceIcon, UserInterfaceIcon);
    values.sort_by_key(|element| {
        (
            element.object().layer,
            element.kind as u8,
            element.source_index,
        )
    });
    values
}

pub fn resolve_profile_scene(
    card: &CustomProfileCard,
    document_key: &str,
    snapshot: &ProfileResolveSnapshot,
) -> Result<ResolvedProfileScene, ProfileResolveError> {
    let authored = ordered_profile_elements(card, document_key);
    let mut layers = Vec::with_capacity(authored.len());
    let mut commands = Vec::with_capacity(authored.len());
    let mut interaction_regions = Vec::with_capacity(authored.len());
    let mut controls = Vec::new();
    for element in authored {
        let kind_name = authored_kind_name(element.kind);
        let mut parameters = BTreeMap::from([
            (
                "authored_kind".into(),
                ParameterValue::Text(kind_name.into()),
            ),
            (
                "authored_index".into(),
                ParameterValue::I64(element.source_index as i64),
            ),
        ]);
        let command_id = semantic_command_id(&element.source_key, "primary", 0);
        let context = PrimaryContext {
            snapshot,
            layer_id: element.layer_id,
            command_id,
        };
        // The viewer never builds hidden elements, nor the omitted ones; their
        // layers keep an empty composite.
        let built =
            element.object().visible && !snapshot.omitted_elements.contains(&element.source_key);
        let omikuji = match (built, element.value) {
            (true, ProfileElementRef::Collection(collection)) => snapshot
                .omikuji_visuals
                .get(&element.source_key)
                .map(|visual| (collection, visual)),
            _ => None,
        };
        let (layer_kind, source_content, primary) = if let Some((collection, visual)) = omikuji {
            parameters.insert(
                "omikuji_id".into(),
                ParameterValue::I64(visual.omikuji_id.into()),
            );
            composite_primary(context, "collection", collection.id, &mut parameters)
        } else if built {
            lower_primary_command(element.value, context, &mut parameters)?
        } else {
            (
                LayerKind::Composite,
                String::new(),
                SemanticCommandSource::composite(
                    command_id,
                    element.layer_id,
                    kind_name,
                    Rect::default(),
                ),
            )
        };
        let mut layer_commands = match omikuji {
            Some((_, visual)) => {
                crate::omikuji::lower_omikuji(&element.source_key, element.layer_id, visual)
            }
            None => vec![primary],
        };
        let mut layer_regions = Vec::new();
        let mut layer_controls = Vec::new();
        if built
            && matches!(
                element.value,
                ProfileElementRef::Honor(_) | ProfileElementRef::BondsHonor(_)
            )
        {
            if let Some(visual) = snapshot.honor_visuals.get(&element.source_key) {
                let layout = crate::profile_layout::ElementLayout {
                    cx: 0.0,
                    cy: 0.0,
                    w: if visual.full_size { 380.0 } else { 180.0 },
                    h: 80.0,
                };
                layer_commands.clear();
                lower_honor_visual(
                    &mut LayerCommands::new(
                        &element.source_key,
                        element.layer_id,
                        &mut layer_commands,
                    ),
                    visual,
                    layout,
                )?;
                layer_regions.push(honor_interaction_region(
                    &element.source_key,
                    element.layer_id,
                    0,
                    layout,
                    visual,
                ));
            }
        }
        if let (true, ProfileElementRef::CardMember(card_member)) = (built, element.value) {
            if card_member.show_master_rank.unwrap_or(false) {
                if let Some(visual) = snapshot.card_member_visuals.get(&element.source_key) {
                    let bounds = layer_commands[0].bounds;
                    let overlay = crate::general_recipe::build_card_member_overlay_recipe(
                        card_member.member_type.unwrap_or(2),
                        element.layer_id,
                        &element.source_key,
                        bounds,
                        visual,
                        card_member_level_source(snapshot, visual),
                    );
                    layer_commands.extend(
                        overlay
                            .iter()
                            .map(general_recipe_node_to_command)
                            .collect::<Result<Vec<_>, ProfileResolveError>>()?,
                    );
                }
            }
        }
        if let (true, ProfileElementRef::General(general), Some(component)) =
            (built, element.value, snapshot.component.as_ref())
        {
            if let Some(lowered) = lower_identity_general(
                general.general_type.unwrap_or_default(),
                element.layer_id,
                &element.source_key,
                component,
            )? {
                layer_commands = lowered.commands;
                layer_regions = lowered.interaction_regions;
                layer_controls = lowered.controls;
            }
        }
        let layer = assemble_profile_layer(
            &element,
            layer_kind,
            source_content,
            parameters.clone(),
            &layer_commands,
            snapshot.line_indent.get(&element.source_key).cloned(),
        );
        commands.extend(layer_commands);
        interaction_regions.push(default_profile_interaction_region(
            &element, &layer, parameters,
        ));
        interaction_regions.extend(layer_regions);
        layers.push(layer);
        controls.extend(layer_controls);
    }
    Ok(ResolvedProfileScene {
        layers,
        commands,
        interaction_regions,
        controls,
    })
}

fn normalized<'a>(
    document_key: &str,
    kind: AuthoredElementKind,
    source_index: usize,
    value: ProfileElementRef<'a>,
) -> NormalizedProfileElement<'a> {
    let source_key = format!(
        "{document_key}\0{}-source-{source_index}",
        authored_kind_name(kind)
    );
    let layer_id = StableId::derive("layer-v2", source_key.as_bytes());
    NormalizedProfileElement {
        kind,
        source_index,
        source_key,
        layer_id,
        value,
    }
}
