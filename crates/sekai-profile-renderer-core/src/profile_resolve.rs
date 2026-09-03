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
    ordered_profile_elements, resource_lookup_key, CardVisualSnapshot, CharacterRankSnapshot,
    ComponentImageSnapshot, HonorVisualKind, HonorVisualSnapshot, MusicDifficultySnapshot,
    MusicResultsSnapshot, ProfileComponentSnapshot, ProfileElementRef, ProfileResolveSnapshot,
    ResolvedProfileScene, ResourceDescriptor, StoryFavoriteSnapshot,
};
use crate::profile_source::{CardMemberElement, CustomProfileCard};
use crate::{LineIndentSource, ParameterValue, ResourceKey, StableId};

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
    #[error("profile component and honor resolution requires the extended resolver: {0}")]
    NeedsExtendedResolution(&'static str),
    #[error("resolved text command references missing layer matrix {0}")]
    MissingLayerMatrix(String),
    #[error(transparent)]
    Scene(#[from] crate::profile_scene::ProfileResolveError),
}

/// Resolves external dependencies for authored image, shape, and text elements.
/// Component and honor elements fail explicitly until their normalized visual
/// records are supplied by the extended resolver.
pub fn prepare_authored_profile(
    card: &CustomProfileCard,
    masterdata: &impl ProfileMasterData,
    document_key: &str,
) -> Result<AuthoredProfilePreparation, AuthoredProfileResolveError> {
    let mut output = AuthoredProfilePreparation::default();
    for element in ordered_profile_elements(card, document_key) {
        match element.value {
            ProfileElementRef::Text(value) => {
                let family = masterdata
                    .resolve_font(value.font_id)
                    .ok_or(AuthoredProfileResolveError::MissingFont(value.font_id))?;
                output.font_families.insert(family.clone());
                output.fonts.insert(value.font_id, family.clone());
                output.glyph_layers.push(ProfileGlyphPreparation {
                    text: value.text.clone(),
                    font_id: value.font_id,
                    font_family: family,
                });
            }
            ProfileElementRef::Shape(value) => {
                let key = masterdata
                    .resolve_resource("shape", value.id)
                    .map(|v| format!("custom_profile/shape/{}", v.file_name))
                    .unwrap_or_else(|| format!("custom_profile/shape/{}", value.id));
                push_resource(
                    &mut output,
                    "shape",
                    value.id,
                    "",
                    key,
                    ResourceMetric {
                        width: 1024.0,
                        height: 1024.0,
                    },
                );
            }
            ProfileElementRef::CardMember(value) => {
                let member_type = value.member_type.unwrap_or(2);
                let training = if value.use_after_special_training.unwrap_or(false) {
                    "after_training"
                } else {
                    "normal"
                };
                let key = masterdata
                    .get_card(value.id)
                    .map(|card| {
                        if member_type == 1 {
                            format!(
                                "character/member_cutout/{}/{training}",
                                card.asset_bundle_name
                            )
                        } else {
                            format!(
                                "character/member_small/{}/card_{training}",
                                card.asset_bundle_name
                            )
                        }
                    })
                    .unwrap_or_else(|| format!("card_member/{}", value.id));
                push_resource(
                    &mut output,
                    "card-member",
                    value.id,
                    &format!("{member_type}:{training}"),
                    key,
                    if member_type == 1 {
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
                );
                if value.show_master_rank.unwrap_or(false) {
                    append_default_card_member_overlay_preparation(
                        &mut output,
                        value,
                        masterdata,
                        &element.source_key,
                    )?;
                }
            }
            ProfileElementRef::Stamp(value) => {
                let bundle = masterdata
                    .resolve_stamp(value.id)
                    .unwrap_or_else(|| format!("stamp{:04}", value.id));
                push_resource(
                    &mut output,
                    "stamp",
                    value.id,
                    "",
                    format!("stamp/{bundle}/{bundle}"),
                    ResourceMetric {
                        width: 100.0,
                        height: 100.0,
                    },
                );
            }
            ProfileElementRef::Other(value) => {
                push_master_resource(&mut output, masterdata, "other", "etc", value.id)
            }
            ProfileElementRef::Collection(value) => push_master_resource(
                &mut output,
                masterdata,
                "collection",
                "collection",
                value.id,
            ),
            ProfileElementRef::StandMember(value) => push_master_resource(
                &mut output,
                masterdata,
                "stand-member",
                "standing",
                value.id,
            ),
            ProfileElementRef::GeneralBackground(value) => push_master_resource(
                &mut output,
                masterdata,
                "general-background",
                "general_bg",
                value.id,
            ),
            ProfileElementRef::StoryBackground(value) => push_master_resource(
                &mut output,
                masterdata,
                "story-background",
                "story_bg",
                value.id,
            ),
            ProfileElementRef::Honor(_) | ProfileElementRef::BondsHonor(_) => {
                return Err(AuthoredProfileResolveError::NeedsExtendedResolution(
                    "honor visual",
                ))
            }
            ProfileElementRef::General(_) => {
                return Err(AuthoredProfileResolveError::NeedsExtendedResolution(
                    "profile component",
                ))
            }
        }
    }
    Ok(output)
}

pub fn build_authored_profile_snapshot(
    card: &CustomProfileCard,
    masterdata: &impl ProfileMasterData,
    document_key: &str,
    resource_metadata: &impl ResourceMetadata,
    line_indent: BTreeMap<String, LineIndentSource>,
) -> Result<ProfileResolveSnapshot, AuthoredProfileResolveError> {
    let preparation = prepare_authored_profile(card, masterdata, document_key)?;
    let mut snapshot = ProfileResolveSnapshot {
        line_indent,
        ..ProfileResolveSnapshot::default()
    };
    for text in &card.texts {
        snapshot.fonts.insert(
            text.font_id,
            masterdata
                .resolve_font(text.font_id)
                .ok_or(AuthoredProfileResolveError::MissingFont(text.font_id))?,
        );
        insert_color(&mut snapshot, masterdata, text.color_id);
        insert_color(&mut snapshot, masterdata, text.outline_color_id);
    }
    for shape in &card.shapes {
        insert_color(&mut snapshot, masterdata, shape.color_id);
        insert_color(&mut snapshot, masterdata, shape.outline_color_id);
    }
    for request in preparation.resources {
        let metric = resource_metadata
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
    populate_card_member_visuals(card, None, masterdata, document_key, &mut snapshot)?;
    Ok(snapshot)
}

pub fn compile_authored_profile_scene(
    card: &CustomProfileCard,
    masterdata: &impl ProfileMasterData,
    document_key: &str,
    resource_metadata: &impl ResourceMetadata,
    line_indent: BTreeMap<String, LineIndentSource>,
) -> Result<ResolvedProfileScene, AuthoredProfileResolveError> {
    let snapshot = build_authored_profile_snapshot(
        card,
        masterdata,
        document_key,
        resource_metadata,
        line_indent,
    )?;
    Ok(crate::profile_scene::resolve_profile_scene(
        card,
        document_key,
        &snapshot,
    )?)
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

    for element in ordered_profile_elements(card, document_key) {
        match element.value {
            ProfileElementRef::Honor(value) => {
                if let Some(visual) = standard_honor_visual(
                    "customProfile.honors",
                    value.id,
                    value.honor_level,
                    value.full_size,
                    profile,
                    masterdata,
                    resource_metadata,
                ) {
                    snapshot.honor_visuals.insert(element.source_key, visual);
                }
            }
            ProfileElementRef::BondsHonor(value) => {
                if let Some(visual) = bonds_honor_visual(
                    "customProfile.bondsHonors",
                    value.id,
                    value.honor_level,
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
            general.general_type.is_some_and(|general_type| {
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
        if !value.show_master_rank.unwrap_or(false) {
            continue;
        }
        let Some(entry) = masterdata.get_card(value.id) else {
            continue;
        };
        let member_type = value.member_type.unwrap_or(2);
        let training = if value.use_after_special_training.unwrap_or(false) {
            "after_training"
        } else {
            "normal"
        };
        let user_card = profile.and_then(|profile| profile.user_cards.get(&value.id));
        let after_training = value
            .use_after_special_training
            .unwrap_or_else(|| user_card.is_some_and(|card| card.after_training));
        let lookup_key = resource_lookup_key(
            "card-member",
            value.id,
            &format!("{member_type}:{training}"),
        );
        let descriptor = snapshot.resources.get(&lookup_key).cloned();
        snapshot.card_member_visuals.insert(
            element.source_key,
            CardVisualSnapshot {
                card_id: value.id,
                after_training,
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
    for (source_key, visual) in &snapshot.honor_visuals {
        collect_honor_descriptors(&mut resources, source_key, visual);
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
    for text in &card.texts {
        snapshot.fonts.insert(
            text.font_id,
            masterdata
                .resolve_font(text.font_id)
                .ok_or(AuthoredProfileResolveError::MissingFont(text.font_id))?,
        );
        insert_color(snapshot, masterdata, text.color_id);
        insert_color(snapshot, masterdata, text.outline_color_id);
    }
    for shape in &card.shapes {
        insert_color(snapshot, masterdata, shape.color_id);
        insert_color(snapshot, masterdata, shape.outline_color_id);
    }
    for element in ordered_profile_elements(card, document_key) {
        let request = match element.value {
            ProfileElementRef::Shape(value) => Some(resource_request(
                "shape",
                value.id,
                "",
                masterdata
                    .resolve_resource("shape", value.id)
                    .map(|v| format!("custom_profile/shape/{}", v.file_name))
                    .unwrap_or_else(|| format!("custom_profile/shape/{}", value.id)),
                ResourceMetric {
                    width: 1024.0,
                    height: 1024.0,
                },
            )),
            ProfileElementRef::CardMember(value) => {
                let member_type = value.member_type.unwrap_or(2);
                let training = if value.use_after_special_training.unwrap_or(false) {
                    "after_training"
                } else {
                    "normal"
                };
                let key = masterdata
                    .get_card(value.id)
                    .map(|card| {
                        if member_type == 1 {
                            format!(
                                "character/member_cutout/{}/{training}",
                                card.asset_bundle_name
                            )
                        } else {
                            format!(
                                "character/member_small/{}/card_{training}",
                                card.asset_bundle_name
                            )
                        }
                    })
                    .unwrap_or_else(|| format!("card_member/{}", value.id));
                Some(resource_request(
                    "card-member",
                    value.id,
                    &format!("{member_type}:{training}"),
                    key,
                    if member_type == 1 {
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
                ))
            }
            ProfileElementRef::Stamp(value) => {
                let bundle = masterdata
                    .resolve_stamp(value.id)
                    .unwrap_or_else(|| format!("stamp{:04}", value.id));
                Some(resource_request(
                    "stamp",
                    value.id,
                    "",
                    format!("stamp/{bundle}/{bundle}"),
                    ResourceMetric {
                        width: 100.0,
                        height: 100.0,
                    },
                ))
            }
            ProfileElementRef::Other(value) => Some(master_resource_request(
                masterdata, "other", "etc", value.id,
            )),
            ProfileElementRef::Collection(value) => Some(master_resource_request(
                masterdata,
                "collection",
                "collection",
                value.id,
            )),
            ProfileElementRef::StandMember(value) => Some(master_resource_request(
                masterdata,
                "stand-member",
                "standing",
                value.id,
            )),
            ProfileElementRef::GeneralBackground(value) => Some(master_resource_request(
                masterdata,
                "general-background",
                "general_bg",
                value.id,
            )),
            ProfileElementRef::StoryBackground(value) => Some(master_resource_request(
                masterdata,
                "story-background",
                "story_bg",
                value.id,
            )),
            _ => None,
        };
        if let Some(request) = request {
            insert_request(snapshot, request, resource_metadata);
        }
    }
    Ok(())
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
    let story_favorites = profile
        .story_favorites
        .iter()
        .map(|story| StoryFavoriteSnapshot {
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
                       field: &str,
                       size: ResourceMetric| {
        masterdata.get_card(card.card_id).map(|entry| {
            let training = if card.after_training {
                "after_training"
            } else {
                "normal"
            };
            let key = if member_type == 1 {
                format!(
                    "character/member_cutout/{}/{training}",
                    entry.asset_bundle_name
                )
            } else {
                format!(
                    "character/member_small/{}/card_{training}",
                    entry.asset_bundle_name
                )
            };
            CardVisualSnapshot {
                card_id: card.card_id,
                after_training: card.after_training,
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
                "userProfile.deckMembers",
                ResourceMetric {
                    width: 600.0,
                    height: 576.0,
                },
            )
        })
        .collect();
    let leader_card = profile.leader_card.as_ref().and_then(|card| {
        card_visual(
            card,
            2,
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
    let (width, suffix, size_char) = if full_size {
        (380.0, "main", "m")
    } else {
        (180.0, "sub", "s")
    };
    let size = ResourceMetric {
        width,
        height: 80.0,
    };
    let background_name = resolved.effective_background_asset_bundle_name().to_owned();
    let background_dir = if resolved.honor_type == "rank_match" {
        "rank_live/honor"
    } else {
        "honor"
    };
    let rarity = honor_rarity_number(&resolved.honor_rarity);
    let frame_candidates = vec![
        resolved.frame_name.as_ref().and_then(|name| {
            optional_descriptor(
                "assets",
                format!("honor_frame/{name}/frame_degree_{size_char}_{rarity}"),
                size,
                "honor_frame",
                id,
                metadata,
            )
        }),
        optional_descriptor(
            "static",
            format!("honor/frame_degree_{size_char}_{rarity}"),
            size,
            "honor_frame",
            id,
            metadata,
        ),
    ];
    let overlay = if resolved.has_rank_overlay() {
        let (overlay_dir, overlay_name) = if resolved.honor_type == "rank_match" {
            ("rank_live/honor", suffix.into())
        } else if resolved.is_live_master {
            ("honor", "scroll".into())
        } else {
            ("honor", format!("rank_{suffix}"))
        };
        optional_descriptor(
            "assets",
            format!(
                "{overlay_dir}/{}/{overlay_name}",
                resolved.asset_bundle_name
            ),
            size,
            "honor_overlay",
            id,
            metadata,
        )
    } else {
        // Standard character, achievement, event and limited-event degree-only
        // bundles do not receive a synthetic overlay. Special shared-background
        // families are selected by `has_rank_overlay` above.
        None
    };
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
            honor_type: resolved.honor_type,
            has_star: resolved.has_star,
            is_live_master: resolved.is_live_master,
            progress,
            background: optional_descriptor(
                "assets",
                format!("{background_dir}/{background_name}/degree_{suffix}"),
                size,
                "honor_background",
                id,
                metadata,
            ),
            frame_candidates,
            overlay,
            star: optional_descriptor(
                "static",
                "honor/icon_degreeLv".into(),
                ResourceMetric {
                    width: 16.0,
                    height: 16.0,
                },
                "honor_static",
                id,
                metadata,
            ),
            star_high: optional_descriptor(
                "static",
                "honor/icon_degreeLv6".into(),
                ResourceMetric {
                    width: 16.0,
                    height: 16.0,
                },
                "honor_static",
                id,
                metadata,
            ),
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
    let entry = masterdata.get_bonds_honor(id)?;
    let (first, second) = if inverse {
        (entry.game_character_unit_id2, entry.game_character_unit_id1)
    } else {
        (entry.game_character_unit_id1, entry.game_character_unit_id2)
    };
    let characters = if use_unit_virtual_singer {
        [
            masterdata.resolve_unit_virtual_singer(first, second),
            masterdata.resolve_unit_virtual_singer(second, first),
        ]
    } else {
        [first, second]
    };
    let (width, size_char) = if full_size {
        (380.0, "m")
    } else {
        (180.0, "s")
    };
    let size = ResourceMetric {
        width,
        height: 80.0,
    };
    let background = |character: i32| {
        optional_descriptor(
            "static",
            if full_size {
                format!("honor/bonds/{character}")
            } else {
                format!("honor/bonds/{character}_sub")
            },
            size,
            "bonds_honor_background",
            id,
            metadata,
        )
    };
    let character = |character: i32| {
        optional_descriptor(
            "assets",
            format!("bonds_honor/chr_sd_{character:02}_01"),
            ResourceMetric {
                width: 160.0,
                height: 160.0,
            },
            "bonds_honor_character",
            id,
            metadata,
        )
    };
    let rarity = honor_rarity_number(&entry.honor_rarity);
    Some(HonorVisualSnapshot {
        source_field: source_field.into(),
        source_id: id.to_string(),
        honor_id: id,
        honor_level: level,
        full_size,
        visual: HonorVisualKind::Bonds {
            character_ids: characters,
            backgrounds: [background(first), background(second)],
            characters: [character(characters[0]), character(characters[1])],
            mask: optional_descriptor(
                "static",
                if full_size {
                    "honor/mask_degree_main".into()
                } else {
                    "honor/mask_degree_sub".into()
                },
                size,
                "honor_static",
                id,
                metadata,
            ),
            frame: optional_descriptor(
                "static",
                format!("honor/frame_degree_{size_char}_{rarity}"),
                size,
                "honor_frame",
                id,
                metadata,
            ),
            word: full_size
                .then(|| masterdata.get_bonds_honor_word(word_id))
                .flatten()
                .and_then(|word| {
                    optional_descriptor(
                        "assets",
                        format!("bonds_honor/word/{}_01", word.assetbundle_name),
                        ResourceMetric {
                            width: 180.0,
                            height: 40.0,
                        },
                        "bonds_honor_word",
                        word_id as i32,
                        metadata,
                    )
                }),
            star: optional_descriptor(
                "static",
                "honor/icon_degreeLv".into(),
                ResourceMetric {
                    width: 16.0,
                    height: 16.0,
                },
                "honor_static",
                id,
                metadata,
            ),
            star_high: optional_descriptor(
                "static",
                "honor/icon_degreeLv6".into(),
                ResourceMetric {
                    width: 16.0,
                    height: 16.0,
                },
                "honor_static",
                id,
                metadata,
            ),
        },
    })
}

fn honor_rarity_number(value: &str) -> i32 {
    match value {
        "low" => 1,
        "middle" => 2,
        "high" => 3,
        _ => 4,
    }
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

fn resource_request(
    kind: &str,
    id: i32,
    variant: &str,
    key: String,
    fallback: ResourceMetric,
) -> ProfileResourceRequest {
    ProfileResourceRequest {
        lookup_key: resource_lookup_key(kind, id, variant),
        resource: ResourceKey {
            namespace: "assets".into(),
            key,
        },
        fallback,
    }
}

fn master_resource_request(
    masterdata: &impl ProfileMasterData,
    lookup_kind: &str,
    table_kind: &str,
    id: i32,
) -> ProfileResourceRequest {
    let key = masterdata
        .resolve_resource(table_kind, id)
        .map(|value| format!("{}/{}", value.load_value, value.file_name))
        .unwrap_or_else(|| format!("{table_kind}/{id}"));
    resource_request(
        lookup_kind,
        id,
        "",
        key,
        ResourceMetric {
            width: 100.0,
            height: 100.0,
        },
    )
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

fn push_master_resource(
    output: &mut AuthoredProfilePreparation,
    masterdata: &impl ProfileMasterData,
    lookup_kind: &str,
    table_kind: &str,
    id: i32,
) {
    let key = masterdata
        .resolve_resource(table_kind, id)
        .map(|v| format!("{}/{}", v.load_value, v.file_name))
        .unwrap_or_else(|| format!("{table_kind}/{id}"));
    push_resource(
        output,
        lookup_kind,
        id,
        "",
        key,
        ResourceMetric {
            width: 100.0,
            height: 100.0,
        },
    );
}

fn push_resource(
    output: &mut AuthoredProfilePreparation,
    kind: &str,
    id: i32,
    variant: &str,
    key: String,
    fallback: ResourceMetric,
) {
    output.resources.push(ProfileResourceRequest {
        lookup_key: resource_lookup_key(kind, id, variant),
        resource: ResourceKey {
            namespace: "assets".into(),
            key,
        },
        fallback,
    });
}

fn append_default_card_member_overlay_preparation(
    output: &mut AuthoredProfilePreparation,
    value: &CardMemberElement,
    masterdata: &impl ProfileMasterData,
    source_key: &str,
) -> Result<(), AuthoredProfileResolveError> {
    let Some(entry) = masterdata.get_card(value.id) else {
        return Ok(());
    };
    let member_type = value.member_type.unwrap_or(2);
    let visual = CardVisualSnapshot {
        card_id: value.id,
        after_training: value.use_after_special_training.unwrap_or(false),
        master_rank: 0,
        level: 60,
        rarity: entry.card_rarity_type,
        attribute: entry.attr,
        image: ComponentImageSnapshot {
            source_field: "customProfile.cardMembers".into(),
            source_id: value.id.to_string(),
            descriptor: None,
        },
    };
    let bounds = if member_type == 1 {
        let family = masterdata
            .resolve_font(1)
            .ok_or(AuthoredProfileResolveError::MissingFont(1))?;
        output.font_families.insert(family.clone());
        output.fonts.insert(1, family.clone());
        output.glyph_layers.push(ProfileGlyphPreparation {
            text: "Lv.60".into(),
            font_id: 1,
            font_family: family,
        });
        crate::Rect {
            x: -156.0,
            y: -256.0,
            width: 312.0,
            height: 512.0,
        }
    } else {
        crate::Rect {
            x: -470.0,
            y: -265.0,
            width: 940.0,
            height: 530.0,
        }
    };
    for node in crate::general_recipe::build_card_member_overlay_recipe(
        member_type,
        StableId::derive("card-member-preparation", source_key.as_bytes()),
        source_key,
        bounds,
        &visual,
    ) {
        if let crate::general_recipe::GeneralRecipePayload::Image { resource, .. } = node.payload {
            output.resources.push(ProfileResourceRequest {
                lookup_key: format!("card-member-overlay\0{}", node.role),
                resource,
                fallback: ResourceMetric {
                    width: node.bounds.width.abs().max(1.0),
                    height: node.bounds.height.abs().max(1.0),
                },
            });
        }
    }
    Ok(())
}

fn insert_color(
    snapshot: &mut ProfileResolveSnapshot,
    masterdata: &impl ProfileMasterData,
    id: i32,
) {
    if snapshot.colors.contains_key(&id) {
        return;
    }
    if let Some(color) = masterdata.resolve_color(id) {
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
        data.insert_value("customProfileShapeResources", serde_json::json!([{ "id": 8, "fileName": "shape_round", "resourceLoadVal": "ignored", "customProfileResourceType": "shape" }])).unwrap();
        let scene = compile_authored_profile_scene(&card, &data, "synthetic", &(), BTreeMap::new())
            .unwrap();
        assert_eq!(scene.layers.len(), 2);
        assert_eq!(scene.commands.len(), 2);
        assert_eq!(scene.commands[0].numeric_text_runs[0].text, "42");
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
                        master_rank: 4,
                        level: 37,
                    },
                ),
                (
                    1002,
                    crate::profile_data::CardState {
                        card_id: 1002,
                        after_training: true,
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
                source: crate::TextSource::ProfileField { field, value },
                font_role: crate::FontRole::RegionFontId(1),
                ..
            } if field == "userCards.1001.level" && value == "Lv.37"
        ));
        assert!(matches!(
            &cropped[5].payload,
            crate::SemanticCommandPayload::Image { resource, .. }
                if resource.key == "card/rarity_star_afterTraining"
        ));
        assert!(matches!(
            &cropped[9].payload,
            crate::SemanticCommandPayload::Image { resource, .. }
                if resource.key == "card/masterRank_S_4"
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
                if resource.key == "card/rarity_star_normal"
        ));
        assert!(matches!(
            &full[6].payload,
            crate::SemanticCommandPayload::Image { resource, .. }
                if resource.key == "card/masterRank_L_2"
        ));

        let image_only = commands_for(2);
        assert_eq!(image_only.len(), 1);
        assert_eq!(image_only[0].role, "card-member");
    }

    #[test]
    fn rejects_components_instead_of_silently_building_placeholder_commands() {
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "generals": [{ "objectData": { "layer": 1, "lock": false, "position": {"x":0.0,"y":0.0,"z":0.0}, "rotation": {"w":1.0,"x":0.0,"y":0.0,"z":0.0}, "scale": {"x":1.0,"y":1.0,"z":1.0}, "visible": true }, "type": 1 }]
        })).unwrap();
        let error =
            prepare_authored_profile(&card, &JsonMasterData::new("cn"), "synthetic").unwrap_err();
        assert_eq!(
            error,
            AuthoredProfileResolveError::NeedsExtendedResolution("profile component")
        );
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
    fn standard_honor_bundles_do_not_request_absent_rank_overlays() {
        for (honor_type, asset_bundle_name) in [
            ("character", "honor_0042"),
            ("achievement", "honor_0105"),
            ("event", "honor_0245"),
            ("limitevent", "honor_se_ln_1"),
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
        for (honor_type, asset_bundle_name) in
            [("sekai_echo", "honor_0182"), ("event", "honor_memorial")]
        {
            let keys = prepared_honor_resource_keys(honor_type, asset_bundle_name);
            assert!(
                keys.iter()
                    .any(|key| key == &format!("honor/{asset_bundle_name}/rank_sub")),
                "{honor_type} {asset_bundle_name} did not request rank_sub: {keys:?}"
            );
        }
    }
}
