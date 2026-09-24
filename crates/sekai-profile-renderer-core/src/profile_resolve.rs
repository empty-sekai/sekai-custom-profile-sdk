//! Environment-neutral preparation for authored profile-card scenes.
//!
//! TMP parsing and glyph measurement deliberately do not live here. Callers pass
//! line-indent programs produced by the shared text compiler.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::masterdata::ProfileMasterData;
use crate::profile_data::{MusicDifficultyStats as ProfileMusicStats, ProfileData};
use crate::profile_scene::{
    card_member_lookup_key, ordered_profile_elements, resource_lookup_key, CardVisualSnapshot,
    CharacterRankSnapshot, ComponentImageSnapshot, HonorVisualKind, HonorVisualSnapshot,
    MusicDifficultySnapshot, MusicResultsSnapshot, ProfileComponentSnapshot, ProfileElementRef,
    ProfileResolveSnapshot, ResolvedProfileScene, ResourceDescriptor, StoryFavoriteSnapshot,
};
use crate::profile_source::CustomProfileCard;
use crate::{LineIndentSource, ParameterValue, ResourceKey};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceMetric {
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceAvailability {
    #[default]
    Unknown,
    Available,
    Unavailable,
}

pub trait ResourceMetadata {
    fn metric(&self, resource: &ResourceKey) -> Option<ResourceMetric>;

    fn availability(&self, _: &ResourceKey) -> ResourceAvailability {
        ResourceAvailability::Unknown
    }
}

impl ResourceMetadata for () {
    fn metric(&self, _: &ResourceKey) -> Option<ResourceMetric> {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileResourceRequest {
    pub lookup_key: String,
    pub resource: ResourceKey,
    pub fallback: ResourceMetric,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileTextLayoutPreparation {
    pub id: String,
    pub parent_id: String,
    pub dynamic_layer_id: String,
    pub text: String,
    pub font_id: i32,
    pub font_family: String,
    pub z: f32,
    pub transform_matrix: [f32; 6],
    pub font_size: f32,
    pub color: [f32; 4],
    pub outline_color: [f32; 4],
    pub color_rgb: [f32; 3],
    pub outline_width: f32,
    pub line_spacing: f32,
    pub text_type: i32,
}

/// Complete renderer-owned text input used only to derive the glyph set.
/// This also includes component, profile, master-data, and localized text
/// emitted by semantic commands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileGlyphPreparation {
    pub text: String,
    pub font_id: i32,
    pub font_family: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AuthoredProfilePreparation {
    pub fonts: BTreeMap<i32, String>,
    pub font_families: BTreeSet<String>,
    pub layout_layers: Vec<ProfileTextLayoutPreparation>,
    pub glyph_layers: Vec<ProfileGlyphPreparation>,
    pub resources: Vec<ProfileResourceRequest>,
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum AuthoredProfileResolveError {
    #[error("master data did not resolve font id {0}")]
    MissingFont(i32),
    #[error("resolved text command references missing layer matrix {0}")]
    MissingLayerMatrix(String),
    #[error(transparent)]
    Scene(#[from] crate::profile_scene::ProfileResolveError),
}

/// Builds the complete environment-neutral resolve snapshot for all authored
/// element kinds, including profile components and both honor variants.
pub fn build_profile_snapshot(
    card: &CustomProfileCard,
    profile: Option<&ProfileData>,
    masterdata: &impl ProfileMasterData,
    document_key: &str,
    locale: &str,
    resource_metadata: &impl ResourceMetadata,
    line_indent: BTreeMap<String, LineIndentSource>,
) -> Result<ProfileResolveSnapshot, AuthoredProfileResolveError> {
    build_profile_snapshot_inner(
        card,
        profile,
        masterdata,
        document_key,
        locale,
        resource_metadata,
        line_indent,
        None,
    )
}

fn build_profile_snapshot_inner(
    card: &CustomProfileCard,
    profile: Option<&ProfileData>,
    masterdata: &impl ProfileMasterData,
    document_key: &str,
    locale: &str,
    resource_metadata: &impl ResourceMetadata,
    line_indent: BTreeMap<String, LineIndentSource>,
    localized_text: Option<&BTreeMap<String, String>>,
) -> Result<ProfileResolveSnapshot, AuthoredProfileResolveError> {
    let mut snapshot = ProfileResolveSnapshot {
        line_indent,
        ..ProfileResolveSnapshot::default()
    };
    populate_authored_resources(
        card,
        masterdata,
        document_key,
        resource_metadata,
        &mut snapshot,
    )?;
    populate_card_member_visuals(card, profile, masterdata, document_key, &mut snapshot)?;

    let owned_honors = profile.map(|profile| &profile.owned_honors);
    for element in ordered_profile_elements(card, document_key) {
        // Hidden honors are not built either.
        if !element.object().visible {
            continue;
        }
        match element.value {
            ProfileElementRef::Honor(value) => {
                let Some(level) = crate::profile_data::placed_honor_level(
                    owned_honors,
                    value.id,
                    value.honor_level,
                ) else {
                    continue;
                };
                if let Some(visual) = standard_honor_visual(
                    "customProfile.honors",
                    value.id,
                    level,
                    value.full_size,
                    profile,
                    masterdata,
                    resource_metadata,
                ) {
                    snapshot.honor_visuals.insert(element.source_key, visual);
                }
            }
            ProfileElementRef::BondsHonor(value) => {
                let Some(level) = crate::profile_data::placed_bonds_honor_level(
                    owned_honors,
                    value.id,
                    value.honor_level,
                ) else {
                    continue;
                };
                if let Some(visual) = bonds_honor_visual(
                    "customProfile.bondsHonors",
                    value.id,
                    level,
                    value.full_size,
                    value.word_id,
                    value.inverse,
                    value.use_unit_virtual_singer,
                    masterdata,
                    resource_metadata,
                ) {
                    snapshot.honor_visuals.insert(element.source_key, visual);
                }
            }
            _ => {}
        }
    }
    if let Some(profile) = profile {
        let has_live_master_honor = profile.honor_slots.iter().any(|slot| {
            slot.profile_honor_type != "bonds"
                && masterdata
                    .resolve_honor(slot.honor_id, slot.honor_level)
                    .is_some_and(|honor| honor.is_live_master)
        });
        let component_requires_font = card.generals.iter().any(|general| {
            general.object_data.visible
                && general.general_type.is_some_and(|general_type| {
                    crate::general_recipe::general_type_requires_font(
                        general_type,
                        has_live_master_honor,
                    )
                })
        });
        snapshot.component = Some(build_component(
            profile,
            masterdata,
            locale,
            resource_metadata,
            localized_text,
            component_requires_font,
        )?);
        if component_requires_font {
            if let Some(font) = masterdata.resolve_font(1) {
                snapshot.fonts.insert(1, font);
            }
        }
    }
    Ok(snapshot)
}

fn populate_card_member_visuals(
    card: &CustomProfileCard,
    profile: Option<&ProfileData>,
    masterdata: &impl ProfileMasterData,
    document_key: &str,
    snapshot: &mut ProfileResolveSnapshot,
) -> Result<(), AuthoredProfileResolveError> {
    let mut needs_level_font = false;
    for element in ordered_profile_elements(card, document_key) {
        let ProfileElementRef::CardMember(value) = element.value else {
            continue;
        };
        if !value.show_master_rank.unwrap_or(false)
            || !element.object().visible
            || snapshot.omitted_elements.contains(&element.source_key)
        {
            continue;
        }
        let Some(entry) = masterdata.get_card(value.id) else {
            continue;
        };
        let member_type = value.member_type.unwrap_or(2);
        let user_card = profile.and_then(|profile| profile.user_cards.get(&value.id));
        // Card views draw trained stars once special training is done, whatever
        // illustration the element shows. Without player data the element's
        // own illustration choice is all there is to go on.
        let trained = user_card.map_or(value.use_after_special_training.unwrap_or(false), |card| {
            card.special_training_done
        });
        let descriptor = snapshot
            .resources
            .get(&card_member_lookup_key(value))
            .cloned();
        snapshot.card_member_visuals.insert(
            element.source_key,
            CardVisualSnapshot {
                card_id: value.id,
                after_training: trained,
                master_rank: user_card.map_or(0, |card| card.master_rank),
                level: user_card.map_or(60, |card| card.level),
                rarity: entry.card_rarity_type,
                attribute: entry.attr,
                image: ComponentImageSnapshot {
                    source_field: "customProfile.cardMembers".into(),
                    source_id: value.id.to_string(),
                    descriptor,
                },
            },
        );
        needs_level_font |= member_type == 1;
    }
    if needs_level_font {
        snapshot.fonts.insert(
            1,
            masterdata
                .resolve_font(1)
                .ok_or(AuthoredProfileResolveError::MissingFont(1))?,
        );
    }
    Ok(())
}

pub fn compile_profile_scene(
    card: &CustomProfileCard,
    profile: Option<&ProfileData>,
    masterdata: &impl ProfileMasterData,
    document_key: &str,
    locale: &str,
    resource_metadata: &impl ResourceMetadata,
    line_indent: BTreeMap<String, LineIndentSource>,
) -> Result<ResolvedProfileScene, AuthoredProfileResolveError> {
    let snapshot = build_profile_snapshot(
        card,
        profile,
        masterdata,
        document_key,
        locale,
        resource_metadata,
        line_indent,
    )?;
    Ok(crate::profile_scene::resolve_profile_scene(
        card,
        document_key,
        &snapshot,
    )?)
}

pub fn compile_profile_scene_with_localizations(
    card: &CustomProfileCard,
    profile: Option<&ProfileData>,
    masterdata: &impl ProfileMasterData,
    document_key: &str,
    locale: &str,
    resource_metadata: &impl ResourceMetadata,
    line_indent: BTreeMap<String, LineIndentSource>,
    localized_text: &BTreeMap<String, String>,
) -> Result<ResolvedProfileScene, AuthoredProfileResolveError> {
    let snapshot = build_profile_snapshot_inner(
        card,
        profile,
        masterdata,
        document_key,
        locale,
        resource_metadata,
        line_indent,
        Some(localized_text),
    )?;
    Ok(crate::profile_scene::resolve_profile_scene(
        card,
        document_key,
        &snapshot,
    )?)
}

/// Resolves the complete font and resource dependency set for all authored
/// kinds and profile components without fetching or decoding any resource.
pub fn prepare_profile(
    card: &CustomProfileCard,
    profile: Option<&ProfileData>,
    masterdata: &impl ProfileMasterData,
    document_key: &str,
    locale: &str,
) -> Result<AuthoredProfilePreparation, AuthoredProfileResolveError> {
    prepare_profile_inner(card, profile, masterdata, document_key, locale, None)
}

pub fn prepare_profile_with_localizations(
    card: &CustomProfileCard,
    profile: Option<&ProfileData>,
    masterdata: &impl ProfileMasterData,
    document_key: &str,
    locale: &str,
    localized_text: &BTreeMap<String, String>,
) -> Result<AuthoredProfilePreparation, AuthoredProfileResolveError> {
    prepare_profile_inner(
        card,
        profile,
        masterdata,
        document_key,
        locale,
        Some(localized_text),
    )
}

fn prepare_profile_inner(
    card: &CustomProfileCard,
    profile: Option<&ProfileData>,
    masterdata: &impl ProfileMasterData,
    document_key: &str,
    locale: &str,
    localized_text: Option<&BTreeMap<String, String>>,
) -> Result<AuthoredProfilePreparation, AuthoredProfileResolveError> {
    let snapshot = build_profile_snapshot_inner(
        card,
        profile,
        masterdata,
        document_key,
        locale,
        &(),
        BTreeMap::new(),
        localized_text,
    )?;
    let mut fonts = snapshot.fonts.clone();
    if let Some(component) = &snapshot.component {
        fonts.extend(component.region_fonts.clone());
    }
    let mut output = AuthoredProfilePreparation {
        font_families: fonts.values().cloned().collect(),
        fonts,
        layout_layers: Vec::new(),
        glyph_layers: Vec::new(),
        resources: Vec::new(),
    };
    let mut resources = BTreeMap::<String, ProfileResourceRequest>::new();
    for (lookup_key, descriptor) in &snapshot.resources {
        collect_descriptor(&mut resources, lookup_key.clone(), descriptor);
    }
    let built = ordered_profile_elements(card, document_key)
        .into_iter()
        .filter(|element| {
            element.object().visible && !snapshot.omitted_elements.contains(&element.source_key)
        })
        .map(|element| element.source_key)
        .collect::<BTreeSet<_>>();
    for (source_key, visual) in &snapshot.honor_visuals {
        if built.contains(source_key) {
            collect_honor_descriptors(&mut resources, source_key, visual);
        }
    }
    let scene = crate::profile_scene::resolve_profile_scene(card, document_key, &snapshot)?;
    let layer_matrices = scene
        .layers
        .iter()
        .map(|layer| (layer.id.clone(), layer.matrix))
        .collect::<BTreeMap<_, _>>();
    let source_keys = ordered_profile_elements(card, document_key)
        .into_iter()
        .map(|element| (element.layer_id, element.source_key))
        .collect::<BTreeMap<_, _>>();
    for (command_index, command) in scene.commands.iter().enumerate() {
        let fallback = ResourceMetric {
            width: command.bounds.width.abs().max(1.0),
            height: command.bounds.height.abs().max(1.0),
        };
        match &command.payload {
            crate::SemanticCommandPayload::Text {
                source,
                font_role: crate::FontRole::RegionFontId(font_id),
                size,
                color,
                outline_color,
                outline_size,
                line_spacing,
                alignment,
                ..
            } => {
                let text = match source {
                    crate::TextSource::Authored { value }
                    | crate::TextSource::ProfileField { value, .. }
                    | crate::TextSource::MasterData { value, .. }
                    | crate::TextSource::Localized { value, .. } => value.clone(),
                };
                let font_family = output
                    .fonts
                    .get(font_id)
                    .cloned()
                    .ok_or(AuthoredProfileResolveError::MissingFont(*font_id))?;
                output.glyph_layers.push(ProfileGlyphPreparation {
                    text: text.clone(),
                    font_id: *font_id,
                    font_family: font_family.clone(),
                });
                let parent_matrix =
                    layer_matrices
                        .get(&command.layer_id)
                        .copied()
                        .ok_or_else(|| {
                            AuthoredProfileResolveError::MissingLayerMatrix(format!(
                                "{:016x}",
                                command.layer_id.0
                            ))
                        })?;
                output.layout_layers.push(ProfileTextLayoutPreparation {
                    id: format!("{:016x}", command.id.0),
                    parent_id: format!("{:016x}", command.layer_id.0),
                    dynamic_layer_id: source_keys
                        .get(&command.layer_id)
                        .cloned()
                        .unwrap_or_else(|| format!("{:016x}", command.layer_id.0)),
                    text,
                    font_id: *font_id,
                    font_family,
                    z: command_index as f32,
                    transform_matrix: multiply_matrix(parent_matrix, command.matrix),
                    font_size: *size,
                    color: *color,
                    outline_color: *outline_color,
                    color_rgb: [color[0] * 255.0, color[1] * 255.0, color[2] * 255.0],
                    outline_width: *outline_size,
                    line_spacing: *line_spacing,
                    text_type: i32::from(*alignment),
                });
            }
            crate::SemanticCommandPayload::Image {
                resource,
                alpha_mask,
                ..
            } => {
                collect_command_resource(
                    &mut resources,
                    &format!("command.{}.image", command.id.0),
                    resource,
                    fallback,
                );
                if let Some(mask) = alpha_mask {
                    collect_command_resource(
                        &mut resources,
                        &format!("command.{}.alpha_mask", command.id.0),
                        mask,
                        fallback,
                    );
                }
            }
            crate::SemanticCommandPayload::Shape {
                primitive: crate::ShapePrimitive::AssetMask { resource },
                ..
            } => collect_command_resource(
                &mut resources,
                &format!("command.{}.shape_mask", command.id.0),
                resource,
                fallback,
            ),
            crate::SemanticCommandPayload::Composite { .. }
            | crate::SemanticCommandPayload::Shape { .. } => {}
        }
    }
    output.resources = resources.into_values().collect();
    Ok(output)
}

fn multiply_matrix(parent: [f32; 6], local: [f32; 6]) -> [f32; 6] {
    [
        parent[0] * local[0] + parent[2] * local[1],
        parent[1] * local[0] + parent[3] * local[1],
        parent[0] * local[2] + parent[2] * local[3],
        parent[1] * local[2] + parent[3] * local[3],
        parent[0] * local[4] + parent[2] * local[5] + parent[4],
        parent[1] * local[4] + parent[3] * local[5] + parent[5],
    ]
}

fn collect_command_resource(
    resources: &mut BTreeMap<String, ProfileResourceRequest>,
    lookup_key: &str,
    resource: &ResourceKey,
    fallback: ResourceMetric,
) {
    let identity = format!("{}\0{}", resource.namespace, resource.key);
    resources
        .entry(identity)
        .or_insert_with(|| ProfileResourceRequest {
            lookup_key: lookup_key.into(),
            resource: resource.clone(),
            fallback,
        });
}

fn collect_descriptor(
    resources: &mut BTreeMap<String, ProfileResourceRequest>,
    lookup_key: String,
    descriptor: &ResourceDescriptor,
) {
    let identity = format!(
        "{}\0{}",
        descriptor.resource.namespace, descriptor.resource.key
    );
    resources
        .entry(identity)
        .or_insert_with(|| ProfileResourceRequest {
            lookup_key,
            resource: descriptor.resource.clone(),
            fallback: ResourceMetric {
                width: descriptor.natural_width,
                height: descriptor.natural_height,
            },
        });
}

fn collect_honor_descriptors(
    resources: &mut BTreeMap<String, ProfileResourceRequest>,
    role: &str,
    honor: &HonorVisualSnapshot,
) {
    let mut add = |name: &str, descriptor: &ResourceDescriptor| {
        collect_descriptor(resources, format!("{role}.{name}"), descriptor)
    };
    match &honor.visual {
        HonorVisualKind::Standard {
            background,
            frame_candidates,
            overlay,
            star,
            star_high,
            ..
        } => {
            for (name, value) in [
                ("background", background.as_ref()),
                ("overlay", overlay.as_ref()),
                ("star", star.as_ref()),
                ("star_high", star_high.as_ref()),
            ] {
                if let Some(value) = value {
                    add(name, value);
                }
            }
            for (index, value) in frame_candidates.iter().enumerate() {
                if let Some(value) = value {
                    add(&format!("frame.{index}"), value);
                }
            }
        }
        HonorVisualKind::Bonds {
            backgrounds,
            characters,
            mask,
            frame,
            word,
            star,
            star_high,
            ..
        } => {
            if let Some(value) = &backgrounds[0] {
                add("background.0", value);
            }
            if let Some(value) = &backgrounds[1] {
                add("background.1", value);
            }
            if let Some(value) = &characters[0] {
                add("character.0", value);
            }
            if let Some(value) = &characters[1] {
                add("character.1", value);
            }
            if let Some(value) = mask {
                add("mask", value);
            }
            if let Some(value) = frame {
                add("frame", value);
            }
            if let Some(word) = word {
                add("word", word);
            }
            if let Some(value) = star {
                add("star", value);
            }
            if let Some(value) = star_high {
                add("star_high", value);
            }
        }
    }
}

fn populate_authored_resources(
    card: &CustomProfileCard,
    masterdata: &impl ProfileMasterData,
    document_key: &str,
    resource_metadata: &impl ResourceMetadata,
    snapshot: &mut ProfileResolveSnapshot,
) -> Result<(), AuthoredProfileResolveError> {
    for element in ordered_profile_elements(card, document_key) {
        // The viewer does not build hidden elements, so they need neither
        // fonts nor resources.
        if !element.object().visible {
            continue;
        }
        match element.value {
            ProfileElementRef::Text(text) => {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    snapshot.fonts.entry(text.font_id)
                {
                    entry.insert(
                        masterdata
                            .resolve_font_or_default(text.font_id)
                            .ok_or(AuthoredProfileResolveError::MissingFont(text.font_id))?,
                    );
                }
                insert_color(snapshot, masterdata, text.color_id);
                insert_color(snapshot, masterdata, text.outline_color_id);
            }
            ProfileElementRef::Shape(shape) => {
                insert_color(snapshot, masterdata, shape.color_id);
                insert_color(snapshot, masterdata, shape.outline_color_id);
            }
            ProfileElementRef::UserInterfaceIcon(icon) => {
                insert_color(snapshot, masterdata, icon.color_id);
            }
            _ => {}
        }
        match authored_resource(element.value, masterdata) {
            AuthoredResource::None => {}
            AuthoredResource::MissingRow => {
                snapshot.omitted_elements.insert(element.source_key);
            }
            AuthoredResource::Request(request) => {
                insert_request(snapshot, request, resource_metadata)
            }
        }
    }
    Ok(())
}

/// Master-data resource an authored element draws; see [`authored_resource`].
#[derive(Clone, Debug, PartialEq)]
pub enum AuthoredResource {
    /// The element kind draws no master-data resource of its own (text,
    /// honors and General panels).
    None,
    /// The master-data row the element references does not exist. The game
    /// does not build such an element.
    MissingRow,
    /// The resource to load, keyed by [`resource_lookup_key`].
    Request(ProfileResourceRequest),
}

/// Resolves the resource an authored image-like element draws: shapes, card
/// members, stamps and the `customProfile*Resources` image kinds. Keys follow
/// the master-data rows (`resourceLoadVal/fileName`, card and stamp asset
/// bundles); a missing row yields [`AuthoredResource::MissingRow`] rather than
/// an invented key.
pub fn authored_resource(
    element: ProfileElementRef<'_>,
    masterdata: &(impl ProfileMasterData + ?Sized),
) -> AuthoredResource {
    let request = match element {
        ProfileElementRef::Shape(value) => master_resource_request(
            masterdata,
            "shape",
            "shape",
            value.id,
            ResourceMetric {
                width: 1024.0,
                height: 1024.0,
            },
        ),
        ProfileElementRef::CardMember(value) => {
            let member_type = value.member_type.unwrap_or(2);
            masterdata
                .get_card(value.id)
                .map(|card| ProfileResourceRequest {
                    lookup_key: card_member_lookup_key(value),
                    resource: ResourceKey {
                        namespace: "assets".into(),
                        key: card_artwork_key(
                            &card.asset_bundle_name,
                            member_type,
                            value.use_after_special_training.unwrap_or(false),
                        ),
                    },
                    fallback: if member_type == 1 {
                        ResourceMetric {
                            width: 312.0,
                            height: 512.0,
                        }
                    } else {
                        ResourceMetric {
                            width: 940.0,
                            height: 530.0,
                        }
                    },
                })
        }
        ProfileElementRef::Stamp(value) => {
            masterdata
                .resolve_stamp(value.id)
                .map(|bundle| ProfileResourceRequest {
                    lookup_key: resource_lookup_key("stamp", value.id, ""),
                    resource: ResourceKey {
                        namespace: "assets".into(),
                        key: format!("stamp/{bundle}/{bundle}"),
                    },
                    fallback: ResourceMetric {
                        width: 100.0,
                        height: 100.0,
                    },
                })
        }
        ProfileElementRef::Other(value) => {
            master_resource_request(masterdata, "other", "etc", value.id, SMALL_RESOURCE)
        }
        ProfileElementRef::Collection(value) => master_resource_request(
            masterdata,
            "collection",
            "collection",
            value.id,
            SMALL_RESOURCE,
        ),
        ProfileElementRef::StandMember(value) => master_resource_request(
            masterdata,
            "stand-member",
            "standing",
            value.id,
            SMALL_RESOURCE,
        ),
        ProfileElementRef::GeneralBackground(value) => master_resource_request(
            masterdata,
            "general-background",
            "general_bg",
            value.id,
            SMALL_RESOURCE,
        ),
        ProfileElementRef::StoryBackground(value) => master_resource_request(
            masterdata,
            "story-background",
            "story_bg",
            value.id,
            SMALL_RESOURCE,
        ),
        ProfileElementRef::CharacterIcon(value) => master_resource_request(
            masterdata,
            "character-icon",
            "character_icon",
            value.id,
            SMALL_RESOURCE,
        ),
        ProfileElementRef::Material(value) => {
            master_resource_request(masterdata, "material", "material", value.id, SMALL_RESOURCE)
        }
        ProfileElementRef::UserInterfaceIcon(value) => master_resource_request(
            masterdata,
            "user-interface-icon",
            "user_interface_icon",
            value.id,
            SMALL_RESOURCE,
        ),
        ProfileElementRef::Text(_)
        | ProfileElementRef::Honor(_)
        | ProfileElementRef::BondsHonor(_)
        | ProfileElementRef::General(_) => return AuthoredResource::None,
    };
    request.map_or(AuthoredResource::MissingRow, AuthoredResource::Request)
}

const SMALL_RESOURCE: ResourceMetric = ResourceMetric {
    width: 100.0,
    height: 100.0,
};

/// Card artwork key: the cut-out illustration for cropped cards
/// (`member_type` 1, deck slots) and the small full illustration otherwise.
pub fn card_artwork_key(asset_bundle_name: &str, member_type: i32, after_training: bool) -> String {
    let training = if after_training {
        "after_training"
    } else {
        "normal"
    };
    if member_type == 1 {
        format!("character/member_cutout/{asset_bundle_name}/{training}")
    } else {
        format!("character/member_small/{asset_bundle_name}/card_{training}")
    }
}

fn build_component(
    profile: &ProfileData,
    masterdata: &impl ProfileMasterData,
    locale: &str,
    metadata: &impl ResourceMetadata,
    localized_text: Option<&BTreeMap<String, String>>,
    requires_font: bool,
) -> Result<ProfileComponentSnapshot, AuthoredProfileResolveError> {
    let font = if requires_font {
        Some(
            masterdata
                .resolve_font(1)
                .ok_or(AuthoredProfileResolveError::MissingFont(1))?,
        )
    } else {
        None
    };
    let image = |field: &str,
                 source_id: String,
                 key: String,
                 size: ResourceMetric,
                 table: &str,
                 id: i32| ComponentImageSnapshot {
        source_field: field.into(),
        source_id,
        descriptor: optional_descriptor("assets", key, size, table, id, metadata),
    };
    let static_image = |field: &str,
                        source_id: String,
                        key: String,
                        size: ResourceMetric,
                        table: &str,
                        id: i32| ComponentImageSnapshot {
        source_field: field.into(),
        source_id,
        descriptor: optional_descriptor("static", key, size, table, id, metadata),
    };
    let story_favorites =
        crate::profile_data::story_favorite_slots(&profile.story_favorites, |story| story.share_no)
            .into_iter()
            .map(|story| {
                let Some(story) = story else {
                    return empty_story_favorite();
                };
                StoryFavoriteSnapshot {
                    story_id: story.story_id,
                    story_type: story.story_type.clone(),
                    image: ComponentImageSnapshot {
                        source_field: "userProfile.storyFavorites".into(),
                        source_id: format!("{}:{}", story.story_type, story.story_id),
                        descriptor: masterdata
                            .resolve_story_banner(&story.story_type, story.story_id)
                            .and_then(|key| {
                                optional_descriptor(
                                    "assets",
                                    key,
                                    ResourceMetric {
                                        width: 400.0,
                                        height: 170.0,
                                    },
                                    "storyFavorites",
                                    story.story_id,
                                    metadata,
                                )
                            }),
                    },
                }
            })
            .collect();
    let character_ranks = profile
        .character_ranks
        .iter()
        .map(|rank| CharacterRankSnapshot {
            character_id: rank.character_id,
            rank: rank.rank,
            challenge_rank: profile
                .challenge_ranks
                .iter()
                .find(|value| value.character_id == rank.character_id)
                .map(|value| value.rank),
            avatar: static_image(
                "userProfile.characterRanks",
                rank.character_id.to_string(),
                format!("chara_avatar/chara{:02}_02", rank.character_id),
                ResourceMetric {
                    width: 76.0,
                    height: 76.0,
                },
                "gameCharacters",
                rank.character_id,
            ),
        })
        .collect();
    let challenge_avatar = (profile.challenge_character_id > 0).then(|| {
        static_image(
            "userProfile.challengeLiveSoloResult.characterId",
            profile.challenge_character_id.to_string(),
            format!("chara_avatar/chara{:02}_02", profile.challenge_character_id),
            ResourceMetric {
                width: 76.0,
                height: 76.0,
            },
            "gameCharacters",
            profile.challenge_character_id,
        )
    });
    let card_visual = |card: &crate::profile_data::CardState,
                       member_type: i32,
                       trained_stars: bool,
                       field: &str,
                       size: ResourceMetric| {
        masterdata.get_card(card.card_id).map(|entry| {
            let key = card_artwork_key(&entry.asset_bundle_name, member_type, card.after_training);
            CardVisualSnapshot {
                card_id: card.card_id,
                after_training: trained_stars,
                master_rank: card.master_rank,
                level: card.level,
                rarity: entry.card_rarity_type,
                attribute: entry.attr,
                image: image(
                    field,
                    card.card_id.to_string(),
                    key,
                    size,
                    "cards",
                    card.card_id,
                ),
            }
        })
    };
    let deck_members = profile
        .deck_members
        .iter()
        .filter_map(|card| {
            card_visual(
                card,
                1,
                card.special_training_done,
                "userProfile.deckMembers",
                ResourceMetric {
                    width: 600.0,
                    height: 576.0,
                },
            )
        })
        .collect();
    // The leader card's stars follow the illustration the player shows.
    let leader_card = profile.leader_card.as_ref().and_then(|card| {
        card_visual(
            card,
            2,
            card.after_training,
            "userProfile.leaderCard",
            ResourceMetric {
                width: 940.0,
                height: 530.0,
            },
        )
    });
    let player_avatar = profile.leader_card.as_ref().and_then(|card| {
        masterdata.get_card(card.card_id).map(|entry| {
            let training = if card.after_training {
                "after_training"
            } else {
                "normal"
            };
            image(
                "userProfile.leaderCard",
                card.card_id.to_string(),
                format!("thumbnail/chara/{}_{training}", entry.asset_bundle_name),
                ResourceMetric {
                    width: 180.0,
                    height: 180.0,
                },
                "cards",
                card.card_id,
            )
        })
    });
    let mut slots = profile.honor_slots.iter().collect::<Vec<_>>();
    slots.sort_by(|a, b| b.full_size.cmp(&a.full_size));
    if slots.len() >= 2 {
        slots.swap(0, 1);
    }
    let honor_slots = slots
        .into_iter()
        .enumerate()
        .filter_map(|(index, slot)| {
            if slot.profile_honor_type == "bonds" {
                let (inverse, use_unit_virtual_singer) = slot.bonds_honor_view_flags();
                bonds_honor_visual(
                    "userProfile.honorSlots",
                    slot.honor_id,
                    slot.honor_level,
                    index == 0,
                    slot.bonds_honor_word_id.unwrap_or_default(),
                    inverse,
                    use_unit_virtual_singer,
                    masterdata,
                    metadata,
                )
            } else {
                standard_honor_visual(
                    "userProfile.honorSlots",
                    slot.honor_id,
                    slot.honor_level,
                    index == 0,
                    Some(profile),
                    masterdata,
                    metadata,
                )
            }
        })
        .collect();
    let localized_text = crate::locale::GENERAL_LOCALIZATION_KEYS
        .iter()
        .filter_map(|key| {
            let value = match localized_text {
                Some(snapshot) => snapshot.get(*key).cloned(),
                None => masterdata
                    .resolve_localized_text(key)
                    .or_else(|| crate::locale::resolve(locale, key)),
            };
            value.map(|value| ((*key).into(), value))
        })
        .collect();
    Ok(ProfileComponentSnapshot {
        locale: locale.into(),
        region_fonts: font.into_iter().map(|font| (1, font)).collect(),
        localized_text,
        user_name: profile.user_name.clone(),
        word: profile.word.clone(),
        user_rank: profile.user_rank,
        total_power: profile.total_power,
        mvp: profile.mvp,
        superstar: profile.superstar,
        challenge_score: profile.challenge_score,
        challenge_character_id: profile.challenge_character_id,
        challenge_avatar,
        music_results: profile.music_results.as_ref().map(music_results),
        story_favorites,
        player_avatar,
        character_ranks,
        deck_members,
        leader_card,
        honor_slots,
    })
}

pub fn build_profile_component_snapshot(
    profile: &ProfileData,
    masterdata: &impl ProfileMasterData,
    locale: &str,
    metadata: &impl ResourceMetadata,
    localized_text: Option<&BTreeMap<String, String>>,
    requires_font: bool,
) -> Result<ProfileComponentSnapshot, AuthoredProfileResolveError> {
    build_component(
        profile,
        masterdata,
        locale,
        metadata,
        localized_text,
        requires_font,
    )
}

fn empty_story_favorite() -> StoryFavoriteSnapshot {
    StoryFavoriteSnapshot {
        story_id: 0,
        story_type: String::new(),
        image: ComponentImageSnapshot {
            source_field: "userProfile.storyFavorites".into(),
            source_id: String::new(),
            descriptor: None,
        },
    }
}

fn music_results(value: &crate::profile_data::MusicResults) -> MusicResultsSnapshot {
    fn one(value: ProfileMusicStats) -> MusicDifficultySnapshot {
        MusicDifficultySnapshot {
            clear: value.clear,
            full_combo: value.full_combo,
            all_perfect: value.all_perfect,
        }
    }
    MusicResultsSnapshot {
        easy: one(value.easy),
        normal: one(value.normal),
        hard: one(value.hard),
        expert: one(value.expert),
        master: one(value.master),
        append: one(value.append),
    }
}

fn standard_honor_visual(
    source_field: &str,
    id: i32,
    level: i32,
    full_size: bool,
    profile: Option<&ProfileData>,
    masterdata: &impl ProfileMasterData,
    metadata: &impl ResourceMetadata,
) -> Option<HonorVisualSnapshot> {
    let resolved = masterdata.resolve_honor(id, level)?;
    let width = if full_size { 380.0 } else { 180.0 };
    let size = ResourceMetric {
        width,
        height: 80.0,
    };
    let plan = resolved.asset_plan(level, full_size);
    let descriptor = |resource: &ResourceKey, metric, table| {
        optional_descriptor(
            &resource.namespace,
            resource.key.clone(),
            metric,
            table,
            id,
            metadata,
        )
    };
    let frame_candidates = plan
        .frame_candidates
        .iter()
        .map(|candidate| {
            candidate
                .as_ref()
                .and_then(|key| descriptor(key, size, "honor_frame"))
        })
        .collect();
    let overlay = plan
        .overlay
        .as_ref()
        .and_then(|key| descriptor(key, size, "honor_overlay"));
    let progress = profile
        .and_then(|profile| {
            resolved
                .honor_mission_type
                .as_ref()
                .and_then(|kind| profile.honor_mission_progress.get(kind))
        })
        .copied()
        .unwrap_or_default();
    Some(HonorVisualSnapshot {
        source_field: source_field.into(),
        source_id: id.to_string(),
        honor_id: id,
        honor_level: level,
        full_size,
        visual: HonorVisualKind::Standard {
            has_star: resolved.draws_level_stars(),
            honor_type: resolved.honor_type,
            is_live_master: resolved.is_live_master,
            progress,
            background: descriptor(&plan.background, size, "honor_background"),
            frame_candidates,
            overlay,
            star: plan.star.as_ref().and_then(|key| {
                descriptor(
                    key,
                    ResourceMetric {
                        width: 16.0,
                        height: 16.0,
                    },
                    "honor_static",
                )
            }),
            star_high: plan.star_high.as_ref().and_then(|key| {
                descriptor(
                    key,
                    ResourceMetric {
                        width: 16.0,
                        height: 16.0,
                    },
                    "honor_static",
                )
            }),
            live_star_on: None,
            live_star_off: None,
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn bonds_honor_visual(
    source_field: &str,
    id: i32,
    level: i32,
    full_size: bool,
    word_id: i64,
    inverse: bool,
    use_unit_virtual_singer: bool,
    masterdata: &impl ProfileMasterData,
    metadata: &impl ResourceMetadata,
) -> Option<HonorVisualSnapshot> {
    let plan = crate::masterdata::bonds_honor_asset_plan(
        masterdata,
        id,
        full_size,
        word_id,
        inverse,
        use_unit_virtual_singer,
    )?;
    let size = ResourceMetric {
        width: if full_size { 380.0 } else { 180.0 },
        height: 80.0,
    };
    let star_size = ResourceMetric {
        width: 16.0,
        height: 16.0,
    };
    let descriptor = |resource: &ResourceKey, metric, table, table_id| {
        optional_descriptor(
            &resource.namespace,
            resource.key.clone(),
            metric,
            table,
            table_id,
            metadata,
        )
    };
    let character_size = ResourceMetric {
        width: 160.0,
        height: 160.0,
    };
    let word_size = ResourceMetric {
        width: 180.0,
        height: 40.0,
    };
    Some(HonorVisualSnapshot {
        source_field: source_field.into(),
        source_id: id.to_string(),
        honor_id: id,
        honor_level: level,
        full_size,
        visual: HonorVisualKind::Bonds {
            character_ids: plan.character_ids,
            backgrounds: plan
                .backgrounds
                .each_ref()
                .map(|key| descriptor(key, size, "bonds_honor_background", id)),
            characters: plan
                .characters
                .each_ref()
                .map(|key| descriptor(key, character_size, "bonds_honor_character", id)),
            mask: descriptor(&plan.mask, size, "honor_static", id),
            frame: descriptor(&plan.frame, size, "honor_frame", id),
            word: plan
                .word
                .as_ref()
                .and_then(|key| descriptor(key, word_size, "bonds_honor_word", word_id as i32)),
            star: descriptor(&plan.star, star_size, "honor_static", id),
            star_high: descriptor(&plan.star_high, star_size, "honor_static", id),
        },
    })
}

fn optional_descriptor(
    namespace: &str,
    key: String,
    fallback: ResourceMetric,
    table: &str,
    id: i32,
    metadata: &impl ResourceMetadata,
) -> Option<ResourceDescriptor> {
    let resource = ResourceKey {
        namespace: namespace.into(),
        key,
    };
    if metadata.availability(&resource) == ResourceAvailability::Unavailable {
        return None;
    }
    let metric = metadata.metric(&resource).unwrap_or(fallback);
    Some(ResourceDescriptor {
        resource,
        natural_width: metric.width,
        natural_height: metric.height,
        provenance: BTreeMap::from([
            ("kind".into(), ParameterValue::Text("master_data".into())),
            ("table".into(), ParameterValue::Text(table.into())),
            ("id".into(), ParameterValue::I64(id.into())),
        ]),
    })
}

fn master_resource_request(
    masterdata: &(impl ProfileMasterData + ?Sized),
    lookup_kind: &str,
    table_kind: &str,
    id: i32,
    fallback: ResourceMetric,
) -> Option<ProfileResourceRequest> {
    let resource = masterdata.resolve_resource(table_kind, id)?;
    Some(ProfileResourceRequest {
        lookup_key: resource_lookup_key(lookup_kind, id, ""),
        resource: ResourceKey {
            namespace: "assets".into(),
            key: format!("{}/{}", resource.load_value, resource.file_name),
        },
        fallback,
    })
}

fn insert_request(
    snapshot: &mut ProfileResolveSnapshot,
    request: ProfileResourceRequest,
    metadata: &impl ResourceMetadata,
) {
    let metric = metadata
        .metric(&request.resource)
        .unwrap_or(request.fallback);
    snapshot.resources.insert(
        request.lookup_key,
        ResourceDescriptor {
            resource: request.resource,
            natural_width: metric.width,
            natural_height: metric.height,
            provenance: BTreeMap::from([(
                "kind".into(),
                ParameterValue::Text("master_data".into()),
            )]),
        },
    );
}

fn insert_color(
    snapshot: &mut ProfileResolveSnapshot,
    masterdata: &impl ProfileMasterData,
    id: i32,
) {
    if snapshot.colors.contains_key(&id) {
        return;
    }
    if let Some(color) = masterdata.resolve_color_or_default(id) {
        snapshot.colors.insert(
            id,
            [
                color.r as f32 / 255.0,
                color.g as f32 / 255.0,
                color.b as f32 / 255.0,
                color.a as f32 / 255.0,
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn honor_frames_resolve_custom_fallback_or_no_frame() {
        struct Available<'a>(&'a [&'a str]);
        impl super::ResourceMetadata for Available<'_> {
            fn metric(&self, _: &crate::ResourceKey) -> Option<super::ResourceMetric> {
                None
            }
            fn availability(&self, key: &crate::ResourceKey) -> super::ResourceAvailability {
                if self.0.contains(&key.key.as_str()) {
                    super::ResourceAvailability::Available
                } else {
                    super::ResourceAvailability::Unavailable
                }
            }
        }
        let masterdata = |rarity: &str| {
            let mut md = crate::masterdata::JsonMasterData::new("en");
            md.insert_value(
                "honors",
                serde_json::json!([{
                    "id": 1, "assetbundleName": "honor_sample", "honorRarity": rarity,
                    "groupId": 1, "levels": [{ "level": 1 }]
                }]),
            )
            .unwrap();
            md.insert_value(
                "honorGroups",
                serde_json::json!([{
                    "id": 1, "honorType": "event", "frameName": "custom_frame"
                }]),
            )
            .unwrap();
            md
        };
        let card = serde_json::from_value(serde_json::json!({"honors": [{
            "objectData": {
                "layer": 0, "lock": false, "visible": true,
                "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
            },
            "id": 1, "honorLevel": 1, "fullSize": true
        }]}))
        .unwrap();
        let custom = "honor_frame/custom_frame/frame_degree_m_3";
        let fallback = "honor/frame_degree_m_3";
        let low_custom = "honor_frame/custom_frame/frame_degree_m_1";
        let low_fallback = "honor/frame_degree_m_1";
        for (rarity, available, expected) in [
            ("high", vec![custom, fallback], Some(("assets", custom))),
            ("high", vec![custom], Some(("assets", custom))),
            ("high", vec![fallback], Some(("static", fallback))),
            ("high", vec![], None),
            // Below the high rarity the group frame is never used.
            (
                "low",
                vec![low_custom, low_fallback],
                Some(("static", low_fallback)),
            ),
            ("low", vec![low_custom], None),
        ] {
            let visual = super::standard_honor_visual(
                "honor",
                1,
                1,
                true,
                None,
                &masterdata(rarity),
                &Available(&available),
            )
            .unwrap();
            let source_key = crate::profile_scene::ordered_profile_elements(&card, "frames")[0]
                .source_key
                .clone();
            let snapshot = crate::profile_scene::ProfileResolveSnapshot {
                honor_visuals: std::collections::BTreeMap::from([(source_key, visual)]),
                ..Default::default()
            };
            let scene =
                crate::profile_scene::resolve_profile_scene(&card, "frames", &snapshot).unwrap();
            let frames = scene
                .commands
                .iter()
                .filter_map(|command| match &command.payload {
                    crate::SemanticCommandPayload::Image { resource, .. }
                        if resource.key.contains("frame_degree") =>
                    {
                        Some((resource.namespace.as_str(), resource.key.as_str()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(frames, expected.into_iter().collect::<Vec<_>>());
        }
    }
    use super::*;
    use crate::masterdata::JsonMasterData;

    #[test]
    fn compiles_a_text_and_shape_scene_from_synthetic_tables() {
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "texts": [{ "objectData": { "layer": 1, "lock": false, "position": {"x":0.0,"y":0.0,"z":0.0}, "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0}, "scale": {"x":1.0,"y":1.0,"z":1.0}, "visible": true }, "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 2, "outlineSize": 0.0, "size": 32.0, "text": "42", "type": 0 }],
            "shapes": [{ "objectData": { "layer": 2, "lock": false, "position": {"x":0.0,"y":0.0,"z":0.0}, "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0}, "scale": {"x":1.0,"y":1.0,"z":1.0}, "visible": true }, "alpha": 1.0, "colorId": 1, "id": 8, "outlineAlpha": 0.0, "outlineColorId": 2, "outlineSize": 0.0 }]
        })).unwrap();
        let mut data = JsonMasterData::new("cn");
        data.insert_value(
            "customProfileTextFonts",
            serde_json::json!([{ "id": 1, "fontName": "SyntheticSans" }]),
        )
        .unwrap();
        data.insert_value("customProfileTextColors", serde_json::json!([{ "id": 1, "colorCode": "#ffffff" }, { "id": 2, "colorCode": "#00000000" }])).unwrap();
        data.insert_value("customProfileShapeResources", serde_json::json!([{ "id": 8, "fileName": "shape_round", "resourceLoadVal": "custom_profile/shape", "customProfileResourceType": "shape" }])).unwrap();
        let scene =
            compile_profile_scene(&card, None, &data, "synthetic", "cn", &(), BTreeMap::new())
                .unwrap();
        assert_eq!(scene.layers.len(), 2);
        assert_eq!(scene.commands.len(), 2);
        assert_eq!(scene.commands[0].numeric_text_runs[0].text, "42");
    }

    fn visible_object(layer: i32, visible: bool) -> serde_json::Value {
        serde_json::json!({
            "layer": layer, "lock": false, "visible": visible,
            "position": {"x":0.0,"y":0.0,"z":0.0},
            "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0},
            "scale": {"x":1.0,"y":1.0,"z":1.0}
        })
    }

    fn text_and_shape_tables() -> JsonMasterData {
        let mut data = JsonMasterData::new("cn");
        data.insert_value(
            "customProfileTextFonts",
            serde_json::json!([{ "id": 1, "fontName": "SyntheticSans" }, { "id": 2, "fontName": "SyntheticRound" }]),
        )
        .unwrap();
        data.insert_value(
            "customProfileTextColors",
            serde_json::json!([{ "id": 7, "colorCode": "#444466" }, { "id": 2, "colorCode": "#ffffff" }]),
        )
        .unwrap();
        data.insert_value(
            "customProfileShapeResources",
            serde_json::json!([{ "id": 1, "fileName": "round", "resourceLoadVal": "custom_profile/shape", "customProfileResourceType": "shape" }]),
        )
        .unwrap();
        data
    }

    fn layer_commands(
        scene: &crate::profile_scene::ResolvedProfileScene,
        kind: crate::AuthoredElementKind,
        index: u32,
    ) -> Vec<&crate::SemanticCommandSource> {
        let layer = scene
            .layers
            .iter()
            .find(|layer| layer.authored_kind == kind && layer.authored_index == index)
            .expect("every authored element keeps its layer");
        scene
            .commands
            .iter()
            .filter(|command| command.layer_id == layer.id)
            .collect()
    }

    #[test]
    fn elements_without_a_master_data_row_are_left_out_instead_of_requesting_invented_keys() {
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "shapes": [
                { "objectData": visible_object(1, true), "alpha": 1.0, "colorId": 7, "id": 1, "outlineAlpha": 0.0, "outlineColorId": 7, "outlineSize": 0.0 },
                { "objectData": visible_object(2, true), "alpha": 1.0, "colorId": 7, "id": 99, "outlineAlpha": 0.0, "outlineColorId": 7, "outlineSize": 0.0 }
            ],
            "stamps": [{ "objectData": visible_object(3, true), "id": 5 }],
            "others": [{ "objectData": visible_object(4, true), "id": 6 }],
            "cardMembers": [{ "objectData": visible_object(5, true), "id": 7, "type": 1 }]
        }))
        .unwrap();
        let data = text_and_shape_tables();
        let preparation = prepare_profile(&card, None, &data, "missing-rows", "cn").unwrap();
        assert_eq!(
            preparation
                .resources
                .iter()
                .map(|request| request.resource.key.as_str())
                .collect::<Vec<_>>(),
            vec!["custom_profile/shape/round"]
        );
        let scene = compile_profile_scene(
            &card,
            None,
            &data,
            "missing-rows",
            "cn",
            &(),
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(scene.layers.len(), 5);
        for (kind, index) in [
            (crate::AuthoredElementKind::Shape, 1),
            (crate::AuthoredElementKind::Stamp, 0),
            (crate::AuthoredElementKind::Other, 0),
            (crate::AuthoredElementKind::CardMember, 0),
        ] {
            let commands = layer_commands(&scene, kind, index);
            assert_eq!(commands.len(), 1, "{kind:?}");
            assert!(
                matches!(
                    commands[0].payload,
                    crate::SemanticCommandPayload::Composite { .. }
                ),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn hidden_elements_are_not_resolved_and_request_nothing() {
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "texts": [{ "objectData": visible_object(1, false), "colorId": 7, "fontId": 404, "lineSpacing": 0.0, "outlineColorId": 7, "outlineSize": 0.0, "size": 32.0, "text": "hidden", "type": 0 }],
            "shapes": [{ "objectData": visible_object(2, false), "alpha": 1.0, "colorId": 7, "id": 1, "outlineAlpha": 0.0, "outlineColorId": 7, "outlineSize": 0.0 }]
        }))
        .unwrap();
        let data = text_and_shape_tables();
        let preparation = prepare_profile(&card, None, &data, "hidden", "cn").unwrap();
        assert!(preparation.resources.is_empty());
        assert!(preparation.glyph_layers.is_empty());
        assert!(preparation.layout_layers.is_empty());
        let scene = compile_profile_scene(&card, None, &data, "hidden", "cn", &(), BTreeMap::new())
            .unwrap();
        assert_eq!(scene.layers.len(), 2);
        assert!(scene.layers.iter().all(|layer| !layer.authored_visible));
        assert!(scene.commands.iter().all(|command| matches!(
            command.payload,
            crate::SemanticCommandPayload::Composite { .. }
        )));
    }

    #[test]
    fn unknown_font_keeps_the_default_face_and_unknown_colors_use_the_first_row() {
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "texts": [{ "objectData": visible_object(1, true), "colorId": 404, "fontId": 404, "lineSpacing": 0.0, "outlineColorId": 405, "outlineSize": 0.2, "size": 32.0, "text": "text", "type": 0 }],
            "shapes": [{ "objectData": visible_object(2, true), "alpha": 0.5, "colorId": 406, "id": 1, "outlineAlpha": 0.25, "outlineColorId": 407, "outlineSize": 0.3 }]
        }))
        .unwrap();
        let data = text_and_shape_tables();
        let preparation = prepare_profile(&card, None, &data, "fallback", "cn").unwrap();
        assert_eq!(preparation.layout_layers[0].font_family, "SyntheticSans");
        let scene =
            compile_profile_scene(&card, None, &data, "fallback", "cn", &(), BTreeMap::new())
                .unwrap();
        let first_row = [
            0x44 as f32 / 255.0,
            0x44 as f32 / 255.0,
            0x66 as f32 / 255.0,
        ];
        let text = layer_commands(&scene, crate::AuthoredElementKind::Text, 0);
        let crate::SemanticCommandPayload::Text {
            color,
            outline_color,
            ..
        } = &text[0].payload
        else {
            panic!("text command expected");
        };
        assert_eq!(color[..3], first_row);
        assert_eq!(outline_color[..3], first_row);
        let shape = layer_commands(&scene, crate::AuthoredElementKind::Shape, 0);
        let crate::SemanticCommandPayload::Shape { fill, stroke, .. } = &shape[0].payload else {
            panic!("shape command expected");
        };
        assert_eq!(*fill, [first_row[0], first_row[1], first_row[2], 0.5]);
        assert_eq!(*stroke, [first_row[0], first_row[1], first_row[2], 0.25]);
    }

    #[test]
    fn deck_stars_follow_special_training_and_story_favorites_keep_share_slots() {
        let raw = serde_json::json!({
            "userCards": [{
                "cardId": 1, "defaultImage": "original",
                "specialTrainingStatus": "done", "masterRank": 0, "level": 5
            }],
            "userDeck": { "leader": 1, "member1": 1 },
            "userStoryFavorites": [
                { "storyId": 3, "storyType": "event_story", "shareNo": 3 },
                { "storyId": 4, "storyType": "unit_story", "shareNo": 1 }
            ]
        });
        let profile = ProfileData::from_json(&raw);
        let mut data = JsonMasterData::new("cn");
        data.insert_value(
            "cards",
            serde_json::json!([{ "id": 1, "assetbundleName": "card_sample", "cardRarityType": "rarity_4", "attr": "cool", "characterId": 1 }]),
        )
        .unwrap();
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({})).unwrap();
        let snapshot = build_profile_snapshot(
            &card,
            Some(&profile),
            &data,
            "deck",
            "cn",
            &(),
            BTreeMap::new(),
        )
        .unwrap();
        let component = snapshot.component.unwrap();
        let deck = &component.deck_members[0];
        assert!(deck.after_training, "trained stars");
        assert_eq!(
            deck.image.descriptor.as_ref().unwrap().resource.key,
            "character/member_cutout/card_sample/normal"
        );
        assert!(
            !component.leader_card.as_ref().unwrap().after_training,
            "the leader card's stars follow the shown illustration"
        );
        assert_eq!(
            component
                .story_favorites
                .iter()
                .map(|story| story.story_id)
                .collect::<Vec<_>>(),
            vec![4, 0, 3]
        );
    }

    #[test]
    fn authored_card_member_overlay_reuses_deck_and_leader_recipes_conditionally() {
        let object = |layer| {
            serde_json::json!({
                "layer": layer,
                "lock": false,
                "position": {"x":0.0,"y":0.0,"z":0.0},
                "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0},
                "scale": {"x":1.0,"y":1.0,"z":1.0},
                "visible": true
            })
        };
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "cardMembers": [
                {
                    "objectData": object(1), "id": 1001, "type": 1,
                    "showMasterRank": true
                },
                {
                    "objectData": object(2), "id": 1002, "type": 2,
                    "showMasterRank": true, "useAfterSpecialTraining": false
                },
                {
                    "objectData": object(3), "id": 1003, "type": 1,
                    "showMasterRank": false
                },
                {
                    "objectData": object(4), "id": 1003, "type": 1,
                    "showMasterRank": true
                }
            ]
        }))
        .unwrap();
        let mut data = JsonMasterData::new("cn");
        data.insert_value(
            "customProfileTextFonts",
            serde_json::json!([{ "id": 1, "fontName": "SyntheticSans" }]),
        )
        .unwrap();
        data.insert_value(
            "cards",
            serde_json::json!([
                {"id":1001,"assetbundleName":"card_a","cardRarityType":"rarity_4","attr":"cute","characterId":1},
                {"id":1002,"assetbundleName":"card_b","cardRarityType":"rarity_3","attr":"cool","characterId":2},
                {"id":1003,"assetbundleName":"card_c","cardRarityType":"rarity_2","attr":"pure","characterId":3}
            ]),
        )
        .unwrap();
        let profile = ProfileData {
            user_cards: BTreeMap::from([
                (
                    1001,
                    crate::profile_data::CardState {
                        card_id: 1001,
                        after_training: true,
                        special_training_done: false,
                        master_rank: 4,
                        level: 37,
                    },
                ),
                (
                    1002,
                    crate::profile_data::CardState {
                        card_id: 1002,
                        after_training: false,
                        special_training_done: true,
                        master_rank: 2,
                        level: 28,
                    },
                ),
            ]),
            ..ProfileData::default()
        };
        let scene = compile_profile_scene(
            &card,
            Some(&profile),
            &data,
            "card-member-overlay",
            "cn",
            &(),
            BTreeMap::new(),
        )
        .unwrap();

        let commands_for = |index| {
            let layer = scene
                .layers
                .iter()
                .find(|layer| layer.authored_index == index)
                .unwrap();
            scene
                .commands
                .iter()
                .filter(|command| command.layer_id == layer.id)
                .collect::<Vec<_>>()
        };
        let cropped = commands_for(0);
        assert_eq!(
            cropped
                .iter()
                .map(|command| command.role.as_str())
                .collect::<Vec<_>>(),
            vec![
                "card-member",
                "card-member-level-bar",
                "card-member-level",
                "card-member-frame",
                "card-member-attribute",
                "card-member-star-0",
                "card-member-star-1",
                "card-member-star-2",
                "card-member-star-3",
                "card-member-master-rank",
            ]
        );
        assert!(matches!(
            &cropped[2].payload,
            crate::SemanticCommandPayload::Text {
                source: crate::TextSource::Localized { key, value, .. },
                font_role: crate::FontRole::RegionFontId(1),
                ..
            } if key == "custom_profile.general.card_level" && value == "Lv.37"
        ));
        // The cropped card shows the untrained illustration of a trained card
        // but still draws trained stars and the large master-rank badge.
        assert!(matches!(
            &cropped[5].payload,
            crate::SemanticCommandPayload::Image { resource, .. }
                if resource.key == "card/rarity_star_normal"
        ));
        assert!(matches!(
            &cropped[9].payload,
            crate::SemanticCommandPayload::Image { resource, .. }
                if resource.key == "card/masterRank_L_4"
        ));

        let full = commands_for(1);
        assert_eq!(
            full.iter()
                .map(|command| command.role.as_str())
                .collect::<Vec<_>>(),
            vec![
                "card-member",
                "card-member-frame",
                "card-member-attribute",
                "card-member-star-0",
                "card-member-star-1",
                "card-member-star-2",
                "card-member-master-rank",
            ]
        );
        assert!(matches!(
            &full[3].payload,
            crate::SemanticCommandPayload::Image { resource, .. }
                if resource.key == "card/rarity_star_afterTraining"
        ));
        assert!(matches!(
            &full[6].payload,
            crate::SemanticCommandPayload::Image { resource, .. }
                if resource.key == "card/masterRank_L_2"
        ));

        let image_only = commands_for(2);
        assert_eq!(image_only.len(), 1);
        assert_eq!(image_only[0].role, "card-member");

        // A card the player does not own has master rank 0: no badge.
        let unranked = commands_for(3);
        assert!(unranked
            .iter()
            .all(|command| command.role != "card-member-master-rank"));
        assert!(unranked
            .iter()
            .any(|command| command.role == "card-member-level"));
    }

    #[test]
    fn preparation_requests_only_resources_used_by_authored_general_types() {
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({})).unwrap();
        let mut data = JsonMasterData::new("cn");
        data.insert_value(
            "customProfileTextFonts",
            serde_json::json!([{ "id": 1, "fontName": "SyntheticSans" }]),
        )
        .unwrap();
        data.insert_value(
            "cards",
            serde_json::json!([{
                "id": 1007,
                "assetbundleName": "card_sample",
                "cardRarityType": "rarity_3",
                "attr": "cool",
                "characterId": 7
            }]),
        )
        .unwrap();
        let profile = ProfileData {
            challenge_character_id: 7,
            leader_card: Some(crate::profile_data::CardState {
                card_id: 1007,
                after_training: true,
                ..crate::profile_data::CardState::default()
            }),
            ..ProfileData::default()
        };
        let preparation =
            prepare_profile(&card, Some(&profile), &data, "unused-components", "cn").unwrap();
        assert!(
            preparation.resources.is_empty(),
            "profile data alone must not demand challenge/player avatar resources"
        );

        let avatar_card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "generals": [{
                "objectData": {
                    "layer": 1, "lock": false,
                    "position": {"x":0.0,"y":0.0,"z":0.0},
                    "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0},
                    "scale": {"x":1.0,"y":1.0,"z":1.0}, "visible": true
                },
                "type": 18
            }]
        }))
        .unwrap();
        let mut no_font_data = JsonMasterData::new("cn");
        no_font_data
            .insert_value(
                "cards",
                serde_json::json!([{
                    "id": 1007, "assetbundleName": "card_sample",
                    "cardRarityType": "rarity_3", "attr": "cool", "characterId": 7
                }]),
            )
            .unwrap();
        let avatar_preparation = prepare_profile(
            &avatar_card,
            Some(&profile),
            &no_font_data,
            "avatar-only",
            "cn",
        )
        .unwrap();
        assert!(avatar_preparation.fonts.is_empty());
        assert_eq!(avatar_preparation.resources.len(), 1);
        assert_eq!(
            avatar_preparation.resources[0].resource.key,
            "thumbnail/chara/card_sample_after_training"
        );

        let used_generals = [10, 17, 18]
            .into_iter()
            .enumerate()
            .map(|(index, general_type)| {
                serde_json::json!({
                    "objectData": {
                        "layer": index as i32 + 1, "lock": false,
                        "position": {"x":0.0,"y":0.0,"z":0.0},
                        "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0},
                        "scale": {"x":1.0,"y":1.0,"z":1.0}, "visible": true
                    },
                    "type": general_type
                })
            })
            .collect::<Vec<_>>();
        let used_card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "generals": used_generals
        }))
        .unwrap();
        let used =
            prepare_profile(&used_card, Some(&profile), &data, "used-components", "cn").unwrap();
        assert_eq!(
            used.resources
                .iter()
                .map(|request| request.resource.key.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "chara_avatar/chara07_02",
                "sprite/icon/icon_playerRank",
                "thumbnail/chara/card_sample_after_training",
            ])
        );
    }

    #[test]
    fn preparation_owns_complete_text_layout_with_full_affine_matrix() {
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "texts": [{
                "objectData": {
                    "layer": 1,
                    "lock": false,
                    "position": {"x": 12.0, "y": 34.0, "z": 0.0},
                    "rotation": {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0},
                    "scale": {"x": -1.0, "y": 1.0, "z": 1.0},
                    "visible": true
                },
                "colorId": 1,
                "fontId": 1,
                "lineSpacing": 2.0,
                "outlineColorId": 2,
                "outlineSize": 1.5,
                "size": 32.0,
                "text": "<b>42</b>",
                "type": 2
            }]
        }))
        .unwrap();
        let mut data = JsonMasterData::new("cn");
        data.insert_value(
            "customProfileTextFonts",
            serde_json::json!([{ "id": 1, "fontName": "SyntheticSans" }]),
        )
        .unwrap();
        data.insert_value(
            "customProfileTextColors",
            serde_json::json!([
                { "id": 1, "colorCode": "#112233cc" },
                { "id": 2, "colorCode": "#445566aa" }
            ]),
        )
        .unwrap();

        let preparation = prepare_profile(&card, None, &data, "affine", "cn").unwrap();
        assert_eq!(preparation.layout_layers.len(), 1);
        let layer = &preparation.layout_layers[0];
        assert_eq!(layer.text, "<b>42</b>");
        assert_eq!(layer.font_family, "SyntheticSans");
        assert_eq!(layer.font_size, 32.0);
        assert_eq!(layer.outline_width, 1.5);
        assert_eq!(layer.text_type, 2);
        assert!(layer.transform_matrix[0] < 0.0);
        assert!(
            layer.transform_matrix[0] * layer.transform_matrix[3]
                - layer.transform_matrix[1] * layer.transform_matrix[2]
                < 0.0
        );
    }

    #[test]
    fn extended_pipeline_lowers_all_authored_kinds_and_component_controls() {
        fn object(layer: i32) -> serde_json::Value {
            serde_json::json!({ "layer": layer, "lock": false, "position": {"x":0.0,"y":0.0,"z":0.0}, "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0}, "scale": {"x":1.0,"y":1.0,"z":1.0}, "visible": true })
        }
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "texts": [{ "objectData": object(1), "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 2, "outlineSize": 0.0, "size": 24.0, "text": "123", "type": 0 }],
            "shapes": [{ "objectData": object(2), "alpha": 1.0, "colorId": 1, "id": 1, "outlineAlpha": 1.0, "outlineColorId": 2, "outlineSize": 1.0 }],
            "cardMembers": [{ "objectData": object(3), "id": 1, "type": 2, "showMasterRank": false, "useAfterSpecialTraining": false }],
            "stamps": [{ "objectData": object(4), "id": 1 }],
            "others": [{ "objectData": object(5), "id": 1 }],
            "bondsHonors": [{ "objectData": object(6), "id": 2, "wordId": 3, "fullSize": true, "inverse": false, "useUnitVirtualSinger": false, "honorLevel": 1 }],
            "honors": [{ "objectData": object(7), "id": 1, "fullSize": false, "honorLevel": 1 }],
            "collections": [{ "objectData": object(8), "id": 1, "targetId": null }],
            "generals": [{ "objectData": object(9), "type": 15 }],
            "standMembers": [{ "objectData": object(10), "id": 1 }],
            "generalBackgrounds": [{ "objectData": object(11), "id": 1 }],
            "storyBackgrounds": [{ "objectData": object(12), "id": 1 }]
        })).unwrap();
        let mut data = JsonMasterData::new("cn");
        data.insert_value(
            "customProfileTextFonts",
            serde_json::json!([{ "id": 1, "fontName": "SyntheticSans" }]),
        )
        .unwrap();
        data.insert_value("customProfileTextColors", serde_json::json!([{ "id": 1, "colorCode": "#ffffff" }, { "id": 2, "colorCode": "#000000" }])).unwrap();
        data.insert_value("customProfileShapeResources", serde_json::json!([{ "id": 1, "fileName": "shape", "resourceLoadVal": "shape", "customProfileResourceType": "shape" }])).unwrap();
        data.insert_value("cards", serde_json::json!([{ "id": 1, "assetbundleName": "card_sample", "cardRarityType": "rarity_3", "attr": "cool", "characterId": 1 }])).unwrap();
        data.insert_value(
            "stamps",
            serde_json::json!([{ "id": 1, "assetbundleName": "stamp_sample" }]),
        )
        .unwrap();
        for (table, load) in [
            ("customProfileEtcResources", "etc"),
            ("customProfileCollectionResources", "collection"),
            ("customProfileMemberStandingPictureResources", "standing"),
            ("customProfileGeneralBackgroundResources", "general_bg"),
            ("customProfileStoryBackgroundResources", "story_bg"),
        ] {
            data.insert_value(table, serde_json::json!([{ "id": 1, "fileName": "sample", "resourceLoadVal": load, "customProfileResourceType": load }])).unwrap();
        }
        data.insert_value("honors", serde_json::json!([{ "id": 1, "assetbundleName": "honor_sample", "honorRarity": "high", "groupId": 1, "levels": [{"level":1,"assetbundleName":null,"honorRarity":null}], "honorMissionType": null }])).unwrap();
        data.insert_value(
            "honorGroups",
            serde_json::json!([{ "id": 1, "honorType": "normal", "frameName": "custom_frame" }]),
        )
        .unwrap();
        data.insert_value("bondsHonors", serde_json::json!([{ "id": 2, "gameCharacterUnitId1": 1, "gameCharacterUnitId2": 2, "honorRarity": "high" }])).unwrap();
        data.insert_value(
            "bondsHonorWords",
            serde_json::json!([{ "id": 3, "assetbundleName": "word_sample" }]),
        )
        .unwrap();
        let profile = ProfileData {
            user_name: "Sample".into(),
            character_ranks: (1..=12)
                .map(|id| crate::profile_data::CharacterRank {
                    character_id: id,
                    rank: 10 + id,
                })
                .collect(),
            challenge_ranks: (1..=12)
                .map(|id| crate::profile_data::CharacterRank {
                    character_id: id,
                    rank: id,
                })
                .collect(),
            ..ProfileData::default()
        };
        let preparation =
            prepare_profile(&card, Some(&profile), &data, "synthetic-all-kinds", "cn").unwrap();
        let scene = compile_profile_scene(
            &card,
            Some(&profile),
            &data,
            "synthetic-all-kinds",
            "cn",
            &(),
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(scene.layers.len(), 12);
        assert!(scene.commands.len() > 12);
        let bonds_commands = scene
            .commands
            .iter()
            .filter(|command| command.role.starts_with("bonds-2-"))
            .collect::<Vec<_>>();
        let bonds_roles = bonds_commands
            .iter()
            .map(|command| command.role.as_str())
            .collect::<Vec<_>>();
        let begin = bonds_roles
            .iter()
            .position(|role| *role == "bonds-2-isolation-begin")
            .unwrap();
        let mask = bonds_roles
            .iter()
            .position(|role| *role == "bonds-2-mask")
            .unwrap();
        let end = bonds_roles
            .iter()
            .position(|role| *role == "bonds-2-isolation-end")
            .unwrap();
        assert!(begin < mask && mask < end);
        assert_eq!(bonds_commands[mask].blend_mode, crate::BlendMode::DstIn);
        assert!(bonds_commands.iter().all(|command| match &command.payload {
            crate::SemanticCommandPayload::Image { alpha_mask, .. } => alpha_mask.is_none(),
            _ => true,
        }));
        assert!(scene.controls.iter().any(|control| matches!(
            &control.state,
            crate::profile_scene::ComponentControlState::Tabs { .. }
        )));
        assert!(scene.controls.iter().any(|control| matches!(
            &control.state,
            crate::profile_scene::ComponentControlState::Scroll { .. }
        )));
        assert!(scene
            .interaction_regions
            .iter()
            .any(|region| region.role.contains("character")));
        assert!(preparation.font_families.contains("SyntheticSans"));
        assert_eq!(
            preparation.fonts.get(&1).map(String::as_str),
            Some("SyntheticSans")
        );
        assert!(preparation.layout_layers.len() > 1);
        assert_eq!(
            preparation.layout_layers[0].dynamic_layer_id,
            "synthetic-all-kinds\0text-source-0"
        );
        assert_eq!(preparation.layout_layers[0].font_family, "SyntheticSans");
        let prepared_text = preparation
            .glyph_layers
            .iter()
            .map(|layer| (layer.text.as_str(), layer.font_id))
            .collect::<BTreeSet<_>>();
        let mut component_text_count = 0;
        for command in &scene.commands {
            if let crate::SemanticCommandPayload::Text {
                source,
                font_role: crate::FontRole::RegionFontId(font_id),
                ..
            } = &command.payload
            {
                let value = match source {
                    crate::TextSource::Authored { value } => value,
                    crate::TextSource::ProfileField { value, .. }
                    | crate::TextSource::MasterData { value, .. }
                    | crate::TextSource::Localized { value, .. } => {
                        component_text_count += 1;
                        value
                    }
                };
                assert!(
                    prepared_text.contains(&(value.as_str(), *font_id)),
                    "missing prepared glyph text {value:?} for font {font_id}"
                );
            }
        }
        assert!(
            component_text_count > 0,
            "fixture must exercise component text demand"
        );
        assert!(preparation.resources.iter().any(|request| {
            request.resource.namespace == "static"
                && request.resource.key == "chara_avatar/chara01_02"
        }));
        for key in [
            "honor/icon_degreeLv",
            "honor/icon_degreeLv6",
            "honor/mask_degree_main",
            "honor/frame_degree_m_3",
            "honor/bonds/1",
            "honor/bonds/2",
        ] {
            assert!(
                preparation.resources.iter().any(|request| {
                    request.resource.namespace == "static" && request.resource.key == key
                }),
                "canonical local honor resource must be static: {key}"
            );
        }
        for key in [
            "honor/live_master_honor_star_1",
            "honor/live_master_honor_star_2",
        ] {
            assert!(
                preparation
                    .resources
                    .iter()
                    .all(|request| request.resource.key != key),
                "Live Master decoration must not request removed resource: {key}"
            );
        }
        assert!(
            preparation.resources.iter().any(|request| {
                request.resource.namespace == "assets"
                    && request.resource.key == "honor_frame/custom_frame/frame_degree_s_3"
            }),
            "custom honor frame must remain an unpacked asset"
        );
        for key in ["bonds_honor/chr_sd_01_01", "bonds_honor/chr_sd_02_01"] {
            assert!(
                preparation.resources.iter().any(|request| {
                    request.resource.namespace == "assets" && request.resource.key == key
                }),
                "bonds character must remain an unpacked asset: {key}"
            );
        }
        let prepared = preparation
            .resources
            .iter()
            .map(|request| format!("{}\0{}", request.resource.namespace, request.resource.key))
            .collect::<BTreeSet<_>>();
        for command in &scene.commands {
            let mut required = Vec::new();
            match &command.payload {
                crate::SemanticCommandPayload::Image {
                    resource,
                    alpha_mask,
                    ..
                } => {
                    required.push(resource);
                    if let Some(mask) = alpha_mask {
                        required.push(mask);
                    }
                }
                crate::SemanticCommandPayload::Shape {
                    primitive: crate::ShapePrimitive::AssetMask { resource },
                    ..
                } => required.push(resource),
                _ => {}
            }
            for resource in required {
                assert!(
                    prepared.contains(&format!("{}\0{}", resource.namespace, resource.key)),
                    "missing prepared resource {}:{}",
                    resource.namespace,
                    resource.key
                );
            }
        }
    }

    #[test]
    fn strict_external_localization_snapshot_drives_general_glyphs_without_region_fallback() {
        fn object(layer: i32) -> serde_json::Value {
            serde_json::json!({ "layer": layer, "lock": false, "position": {"x":0.0,"y":0.0,"z":0.0}, "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0}, "scale": {"x":1.0,"y":1.0,"z":1.0}, "visible": true })
        }
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "generals": [{ "objectData": object(4), "type": 4 }]
        }))
        .unwrap();
        let mut data = JsonMasterData::new("cn");
        data.insert_value(
            "customProfileTextFonts",
            serde_json::json!([{ "id": 1, "fontName": "SyntheticSans" }]),
        )
        .unwrap();
        let profile = ProfileData {
            word: "玩家原文".into(),
            ..ProfileData::default()
        };
        let localized = BTreeMap::from([(
            "custom_profile.general.comment.title".into(),
            "External Bio".into(),
        )]);
        let prepared = prepare_profile_with_localizations(
            &card,
            Some(&profile),
            &data,
            "localized-general",
            "cn",
            &localized,
        )
        .unwrap();
        assert!(prepared
            .glyph_layers
            .iter()
            .any(|layer| layer.text == "External Bio"));
        assert!(prepared
            .glyph_layers
            .iter()
            .any(|layer| layer.text == "玩家原文"));

        let error = prepare_profile_with_localizations(
            &card,
            Some(&profile),
            &data,
            "missing-localized-general",
            "cn",
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("custom_profile.general.comment.title"));
    }

    #[test]
    fn unavailable_provider_resource_is_preserved_as_component_state_not_fake_dimensions() {
        struct Unavailable;
        impl super::ResourceMetadata for Unavailable {
            fn metric(&self, _: &crate::ResourceKey) -> Option<super::ResourceMetric> {
                None
            }

            fn availability(&self, _: &crate::ResourceKey) -> super::ResourceAvailability {
                super::ResourceAvailability::Unavailable
            }
        }

        let profile = ProfileData {
            challenge_character_id: 7,
            ..ProfileData::default()
        };
        let data = JsonMasterData::new("cn");
        let snapshot =
            super::build_component(&profile, &data, "cn", &Unavailable, None, false).unwrap();
        assert!(snapshot
            .challenge_avatar
            .as_ref()
            .is_some_and(|image| image.descriptor.is_none()));
    }

    fn honor_object() -> serde_json::Value {
        serde_json::json!({
            "layer": 1, "lock": false,
            "position": {"x": 0.0, "y": 0.0, "z": 0.0},
            "rotation": {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0},
            "scale": {"x": 1.0, "y": 1.0, "z": 1.0}, "visible": true
        })
    }

    #[test]
    fn bonds_honor_word_art_follows_the_honor_rarity() {
        let mut data = JsonMasterData::new("cn");
        data.insert_value(
            "bondsHonors",
            serde_json::json!([
                { "id": 1, "gameCharacterUnitId1": 1, "gameCharacterUnitId2": 2, "honorRarity": "low" },
                { "id": 2, "gameCharacterUnitId1": 1, "gameCharacterUnitId2": 2, "honorRarity": "middle" }
            ]),
        )
        .unwrap();
        data.insert_value(
            "bondsHonorWords",
            serde_json::json!([{ "id": 9, "assetbundleName": "honorname_0102_01" }]),
        )
        .unwrap();
        for (id, expected) in [
            (1, "bonds_honor/word/honorname_0102_01_01"),
            (2, "bonds_honor/word/honorname_0102_01_02"),
        ] {
            let visual =
                super::bonds_honor_visual("bonds", id, 1, true, 9, false, false, &data, &())
                    .unwrap();
            let HonorVisualKind::Bonds { word, .. } = visual.visual else {
                panic!("bonds visual expected");
            };
            assert_eq!(word.unwrap().resource.key, expected);
        }
    }

    #[test]
    fn placed_honors_take_the_owned_level_and_skip_unowned_honors() {
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "honors": [
                { "objectData": honor_object(), "id": 10, "fullSize": true },
                { "objectData": honor_object(), "id": 11, "fullSize": false }
            ],
            "bondsHonors": [
                { "objectData": honor_object(), "id": 20, "wordId": 0, "fullSize": false, "inverse": false, "useUnitVirtualSinger": false },
                { "objectData": honor_object(), "id": 21, "wordId": 0, "fullSize": false, "inverse": false, "useUnitVirtualSinger": false }
            ]
        }))
        .unwrap();
        let mut data = JsonMasterData::new("cn");
        data.insert_value(
            "honors",
            serde_json::json!([
                { "id": 10, "assetbundleName": "honor_0010", "honorRarity": "low", "groupId": 1, "levels": [{"level": 1}, {"level": 2}] },
                { "id": 11, "assetbundleName": "honor_0011", "honorRarity": "low", "groupId": 1, "levels": [{"level": 1}, {"level": 2}] }
            ]),
        )
        .unwrap();
        data.insert_value(
            "honorGroups",
            serde_json::json!([{ "id": 1, "honorType": "character" }]),
        )
        .unwrap();
        data.insert_value(
            "bondsHonors",
            serde_json::json!([
                { "id": 20, "gameCharacterUnitId1": 1, "gameCharacterUnitId2": 2, "honorRarity": "low" },
                { "id": 21, "gameCharacterUnitId1": 1, "gameCharacterUnitId2": 3, "honorRarity": "low" }
            ]),
        )
        .unwrap();
        let levels = |snapshot: &crate::profile_scene::ProfileResolveSnapshot| {
            snapshot
                .honor_visuals
                .values()
                .map(|visual| (visual.honor_id, visual.honor_level))
                .collect::<BTreeMap<_, _>>()
        };
        let snapshot = |profile: Option<&ProfileData>| {
            build_profile_snapshot(
                &card,
                profile,
                &data,
                "owned-honors",
                "cn",
                &(),
                BTreeMap::new(),
            )
            .unwrap()
        };

        // Without a profile the element's own level is kept.
        assert_eq!(
            levels(&snapshot(None)),
            BTreeMap::from([(10, 1), (11, 1), (20, 1), (21, 1)])
        );

        // userHonors rows are [honorId, level, obtainedAt]; userBondsHonors rows
        // are objects. Honors missing from either list are not drawn.
        let profile = ProfileData::from_json(&serde_json::json!({
            "userHonors": [[10, 34, 1700000000000_i64], [99, 2, null]],
            "userBondsHonors": [{ "bondsHonorId": 21, "level": 7, "obtainedAt": 0 }]
        }));
        assert_eq!(
            levels(&snapshot(Some(&profile))),
            BTreeMap::from([(10, 34), (21, 7)])
        );

        // Object-shaped honor rows are read the same way.
        let profile = ProfileData::from_json(&serde_json::json!({
            "userHonors": [{ "honorId": 11, "level": 3 }],
            "userBondsHonors": []
        }));
        assert_eq!(levels(&snapshot(Some(&profile))), BTreeMap::from([(11, 3)]));
    }

    #[test]
    fn hidden_honors_are_not_resolved_and_request_nothing() {
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "honors": [{ "objectData": visible_object(1, false), "id": 10, "fullSize": true, "honorLevel": 2 }],
            "bondsHonors": [{ "objectData": visible_object(2, false), "id": 20, "wordId": 0, "fullSize": false, "inverse": false, "useUnitVirtualSinger": false, "honorLevel": 1 }]
        }))
        .unwrap();
        let mut data = JsonMasterData::new("cn");
        data.insert_value(
            "honors",
            serde_json::json!([{ "id": 10, "assetbundleName": "honor_0010", "honorRarity": "low", "groupId": 1, "levels": [{"level": 1}, {"level": 2}] }]),
        )
        .unwrap();
        data.insert_value(
            "honorGroups",
            serde_json::json!([{ "id": 1, "honorType": "character" }]),
        )
        .unwrap();
        data.insert_value(
            "bondsHonors",
            serde_json::json!([{ "id": 20, "gameCharacterUnitId1": 1, "gameCharacterUnitId2": 2, "honorRarity": "low" }]),
        )
        .unwrap();
        let snapshot = build_profile_snapshot(
            &card,
            None,
            &data,
            "hidden-honors",
            "cn",
            &(),
            BTreeMap::new(),
        )
        .unwrap();
        assert!(snapshot.honor_visuals.is_empty());
        let preparation = prepare_profile(&card, None, &data, "hidden-honors", "cn").unwrap();
        assert!(preparation.resources.is_empty());
    }

    #[test]
    fn card_honors_draw_level_stars_for_every_multi_level_honor_type() {
        let honor_card = |level: i32| -> CustomProfileCard {
            serde_json::from_value(serde_json::json!({
                "honors": [{ "objectData": honor_object(), "id": 30, "fullSize": true, "honorLevel": level }]
            }))
            .unwrap()
        };
        for (honor_type, level, stars, high_stars) in [
            ("limitevent", 3, 3, 0),
            ("limitevent", 7, 5, 2),
            ("limitevent", 13, 3, 0),
            ("character", 4, 4, 0),
            ("achievement", 10, 5, 5),
            ("event", 7, 0, 0),
        ] {
            let mut data = JsonMasterData::new("cn");
            data.insert_value(
                "honors",
                serde_json::json!([{ "id": 30, "assetbundleName": "honor_0030", "honorRarity": "middle", "groupId": 1, "levels": [{"level": 1}, {"level": 2}] }]),
            )
            .unwrap();
            data.insert_value(
                "honorGroups",
                serde_json::json!([{ "id": 1, "honorType": honor_type }]),
            )
            .unwrap();
            let card = honor_card(level);
            let scene = compile_profile_scene(
                &card,
                None,
                &data,
                "honor-stars",
                "cn",
                &(),
                BTreeMap::new(),
            )
            .unwrap();
            let roles = layer_commands(&scene, crate::AuthoredElementKind::Honor, 0)
                .into_iter()
                .map(|command| command.role.clone())
                .collect::<Vec<_>>();
            let count = |prefix: &str| roles.iter().filter(|role| role.starts_with(prefix)).count();
            let high = count("honor-30-star-high-");
            assert_eq!(
                (count("honor-30-star-") - high, high),
                (stars, high_stars),
                "{honor_type} level {level}: {roles:?}"
            );
        }
    }

    fn prepared_honor_resource_keys(honor_type: &str, asset_bundle_name: &str) -> Vec<String> {
        let object = serde_json::json!({
            "layer": 1, "lock": false,
            "position": {"x": 0.0, "y": 0.0, "z": 0.0},
            "rotation": {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0},
            "scale": {"x": 1.0, "y": 1.0, "z": 1.0}, "visible": true
        });
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "honors": [{ "objectData": object, "id": 42, "fullSize": false, "honorLevel": 1 }]
        }))
        .unwrap();
        let mut data = JsonMasterData::new("cn");
        data.insert_value("honors", serde_json::json!([{
            "id": 42, "assetbundleName": asset_bundle_name, "honorRarity": "middle",
            "groupId": 1, "levels": [{"level": 1, "assetbundleName": null, "honorRarity": null}],
            "honorMissionType": null
        }])).unwrap();
        data.insert_value(
            "honorGroups",
            serde_json::json!([{
                "id": 1, "honorType": honor_type, "hasStar": false, "isLiveMaster": false
            }]),
        )
        .unwrap();
        let prepared = prepare_profile(&card, None, &data, "character-honor", "cn").unwrap();
        prepared
            .resources
            .into_iter()
            .map(|request| request.resource.key)
            .collect()
    }

    #[test]
    fn honor_types_without_rank_art_request_no_rank_overlay() {
        for (honor_type, asset_bundle_name) in [
            ("character", "honor_0042"),
            ("achievement", "honor_0105"),
            ("limitevent", "honor_se_ln_1"),
            ("limitevent", "honor_top_000020"),
        ] {
            let keys = prepared_honor_resource_keys(honor_type, asset_bundle_name);
            assert!(keys.iter().any(|key| key.ends_with("/degree_sub")));
            assert!(
                !keys.iter().any(|key| key.ends_with("/rank_sub")),
                "{honor_type} {asset_bundle_name} unexpectedly requested rank_sub: {keys:?}"
            );
        }
    }

    #[test]
    fn overlay_bearing_honor_bundles_request_rank_assets() {
        for (honor_type, asset_bundle_name) in [
            ("sekai_echo", "honor_0182"),
            ("event", "honor_memorial"),
            ("event", "honor_0305"),
        ] {
            let keys = prepared_honor_resource_keys(honor_type, asset_bundle_name);
            assert!(
                keys.iter()
                    .any(|key| key == &format!("honor/{asset_bundle_name}/rank_sub")),
                "{honor_type} {asset_bundle_name} did not request rank_sub: {keys:?}"
            );
        }
    }

    /// Colours plus one row in each icon table: character icon 1, material 3
    /// and user-interface icon 2. Colour 7 is the first row.
    fn icon_tables() -> JsonMasterData {
        let mut data = text_and_shape_tables();
        data.insert_value(
            "customProfileTextColors",
            serde_json::json!([{ "id": 7, "colorCode": "#444466" }, { "id": 2, "colorCode": "#ff8000" }]),
        )
        .unwrap();
        for (table, resource_type, id, file_name) in [
            (
                "customProfileCharacterIconResources",
                "character_icon",
                1,
                "profile_chr_icon_ichika",
            ),
            (
                "customProfileMaterialResources",
                "material",
                3,
                "profile_icon_item_0003",
            ),
            (
                "customProfileUserInterfaceIconResources",
                "user_interface_icon",
                2,
                "profile_icon_0002",
            ),
        ] {
            data.insert_value(
                table,
                serde_json::json!([{
                    "id": id, "seq": id, "customProfileResourceType": resource_type,
                    "resourceLoadType": "assetbundle",
                    "resourceLoadVal": format!("custom_profile/{resource_type}"),
                    "fileName": file_name
                }]),
            )
            .unwrap();
        }
        data
    }

    /// A version-4 page: one element of each icon kind that resolves, one of
    /// each kind whose row is missing, and a hidden character icon.
    fn icon_card() -> CustomProfileCard {
        serde_json::from_value(serde_json::json!({
            "characterIcons": [
                { "objectData": visible_object(1, true), "id": 1 },
                { "objectData": visible_object(4, true), "id": 99 },
                { "objectData": visible_object(8, false), "id": 1 }
            ],
            "materials": [
                { "objectData": visible_object(2, true), "id": 3 },
                { "objectData": visible_object(5, true), "id": 98 }
            ],
            "userInterfaceIcons": [
                { "objectData": visible_object(3, true), "id": 2, "colorId": 2, "alpha": 0.5 },
                { "objectData": visible_object(6, true), "id": 97, "colorId": 2, "alpha": 1.0 },
                { "objectData": visible_object(7, true), "id": 2, "colorId": 404, "alpha": 0.3 }
            ]
        }))
        .unwrap()
    }

    fn game_layer_commands(
        scene: &crate::profile_scene::ResolvedProfileScene,
        game_layer: i32,
    ) -> Vec<&crate::SemanticCommandSource> {
        let layer = scene
            .layers
            .iter()
            .find(|layer| layer.game_layer == game_layer)
            .expect("every authored element keeps its layer");
        scene
            .commands
            .iter()
            .filter(|command| command.layer_id == layer.id)
            .collect()
    }

    #[test]
    fn icon_elements_parse_and_request_the_keys_of_their_master_rows() {
        let card = icon_card();
        assert_eq!(card.element_count(), 8);
        let data = icon_tables();
        let preparation = prepare_profile(&card, None, &data, "icons", "jp").unwrap();
        let mut keys = preparation
            .resources
            .iter()
            .map(|request| request.resource.key.as_str())
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "custom_profile/character_icon/profile_chr_icon_ichika",
                "custom_profile/material/profile_icon_item_0003",
                "custom_profile/user_interface_icon/profile_icon_0002",
            ]
        );
        let scene =
            compile_profile_scene(&card, None, &data, "icons", "jp", &(), BTreeMap::new()).unwrap();
        assert_eq!(
            scene
                .layers
                .iter()
                .map(|layer| layer.game_layer)
                .collect::<Vec<_>>(),
            (1..=8).collect::<Vec<_>>()
        );
    }

    #[test]
    fn icon_elements_draw_their_images_and_skip_missing_rows_and_hidden_elements() {
        let card = icon_card();
        let data = icon_tables();
        let scene =
            compile_profile_scene(&card, None, &data, "icons", "jp", &(), BTreeMap::new()).unwrap();
        let image = |game_layer: i32| {
            let commands = game_layer_commands(&scene, game_layer);
            assert_eq!(commands.len(), 1, "layer {game_layer}");
            match &commands[0].payload {
                crate::SemanticCommandPayload::Image { resource, tint, .. } => {
                    (resource.key.clone(), *tint)
                }
                other => panic!("layer {game_layer} draws {other:?}"),
            }
        };
        assert_eq!(
            image(1),
            (
                "custom_profile/character_icon/profile_chr_icon_ichika".into(),
                [1.0; 4]
            )
        );
        assert_eq!(
            image(2),
            (
                "custom_profile/material/profile_icon_item_0003".into(),
                [1.0; 4]
            )
        );
        for game_layer in [4, 5, 6, 8] {
            let commands = game_layer_commands(&scene, game_layer);
            assert_eq!(commands.len(), 1, "layer {game_layer}");
            assert!(
                matches!(
                    commands[0].payload,
                    crate::SemanticCommandPayload::Composite { .. }
                ),
                "layer {game_layer}"
            );
        }
        assert!(
            !scene
                .layers
                .iter()
                .find(|layer| layer.game_layer == 8)
                .unwrap()
                .authored_visible
        );
    }

    #[test]
    fn user_interface_icons_are_tinted_by_their_colour_row_and_color32_alpha() {
        let card = icon_card();
        let data = icon_tables();
        let scene =
            compile_profile_scene(&card, None, &data, "icons", "jp", &(), BTreeMap::new()).unwrap();
        let tint = |game_layer: i32| match &game_layer_commands(&scene, game_layer)[0].payload {
            crate::SemanticCommandPayload::Image { resource, tint, .. } => {
                assert_eq!(
                    resource.key,
                    "custom_profile/user_interface_icon/profile_icon_0002"
                );
                *tint
            }
            other => panic!("layer {game_layer} draws {other:?}"),
        };
        // 0.5 * 255 = 127.5 and 0.3 * 255 = 76.5 both round to the even step.
        assert_eq!(tint(3), [1.0, 128.0 / 255.0, 0.0, 128.0 / 255.0]);
        // An unknown colour takes the first row, like text and shapes.
        assert_eq!(
            tint(7),
            [
                0x44 as f32 / 255.0,
                0x44 as f32 / 255.0,
                0x66 as f32 / 255.0,
                76.0 / 255.0
            ]
        );
    }

    #[test]
    fn master_data_without_the_icon_tables_leaves_icons_out_and_draws_the_rest() {
        let mut value = serde_json::to_value(icon_card()).unwrap();
        value["shapes"] = serde_json::json!([{
            "objectData": visible_object(9, true), "alpha": 1.0, "colorId": 7, "id": 1,
            "outlineAlpha": 0.0, "outlineColorId": 7, "outlineSize": 0.0
        }]);
        let card: CustomProfileCard = serde_json::from_value(value).unwrap();
        for table in [
            "customProfileCharacterIconResources",
            "customProfileMaterialResources",
            "customProfileUserInterfaceIconResources",
        ] {
            assert!(
                !crate::masterdata::PROFILE_MASTERDATA_TABLES.contains(&table),
                "{table} is not shipped by every region"
            );
            assert!(crate::masterdata::PROFILE_OPTIONAL_MASTERDATA_TABLES.contains(&table));
        }
        let data = text_and_shape_tables();
        let preparation = prepare_profile(&card, None, &data, "no-icon-tables", "cn").unwrap();
        assert_eq!(
            preparation
                .resources
                .iter()
                .map(|request| request.resource.key.as_str())
                .collect::<Vec<_>>(),
            ["custom_profile/shape/round"]
        );
        let scene = compile_profile_scene(
            &card,
            None,
            &data,
            "no-icon-tables",
            "cn",
            &(),
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(scene.layers.len(), 9);
        assert!(matches!(
            game_layer_commands(&scene, 9)[0].payload,
            crate::SemanticCommandPayload::Shape { .. }
        ));
        for game_layer in 1..=8 {
            assert!(
                game_layer_commands(&scene, game_layer)
                    .iter()
                    .all(|command| matches!(
                        command.payload,
                        crate::SemanticCommandPayload::Composite { .. }
                    )),
                "layer {game_layer}"
            );
        }
    }
}
