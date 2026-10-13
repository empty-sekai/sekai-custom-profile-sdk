//! Environment-neutral preparation for authored profile-card scenes.
//!
//! TMP parsing and glyph measurement deliberately do not live here. Callers pass
//! line-indent programs produced by the shared text compiler.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::masterdata::{CollectionResourceType, ProfileMasterData, ResourceInfo};
use crate::omikuji::{OmikujiClient, OmikujiPlan};
use crate::profile_data::{MusicDifficultyStats as ProfileMusicStats, ProfileData};
use crate::profile_scene::{
    card_member_lookup_key, collection_normal_map_lookup_key, ordered_profile_elements,
    resource_lookup_key, CardVisualSnapshot, CharacterRankSnapshot, ComponentImageSnapshot,
    HonorVisualKind, HonorVisualSnapshot, MusicDifficultySnapshot, MusicResultsSnapshot,
    ProfileComponentSnapshot, ProfileElementRef, ProfileResolveSnapshot, ResolvedProfileScene,
    ResourceDescriptor, StoryFavoriteSnapshot,
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
    /// SDF text fonts by master-data font id.
    pub fonts: BTreeMap<i32, String>,
    /// Every font family the page draws with: the values of [`Self::fonts`]
    /// and [`Self::ugui_font_families`].
    pub font_families: BTreeSet<String>,
    /// Families of every face of the uGUI texts ([`crate::ugui_text`]) the
    /// page draws, fallback faces included. Their glyphs come from FreeType
    /// bitmaps rather than an SDF atlas.
    #[serde(default)]
    pub ugui_font_families: BTreeSet<String>,
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
        locale,
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
        ugui_font_families: BTreeSet::new(),
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
                material,
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
                if let crate::ImageMaterial::LitBadge { normal_map } = material {
                    collect_command_resource(
                        &mut resources,
                        &format!("command.{}.normal_map", command.id.0),
                        normal_map,
                        normal_map_metric(),
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
            crate::SemanticCommandPayload::UguiText(source) => {
                output
                    .ugui_font_families
                    .extend(source.face_chain().into_iter().map(|face| face.family));
            }
            crate::SemanticCommandPayload::Composite { .. }
            | crate::SemanticCommandPayload::Shape { .. } => {}
        }
    }
    output
        .font_families
        .extend(output.ugui_font_families.iter().cloned());
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
    locale: &str,
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
            AuthoredResource::MissingRow | AuthoredResource::NotDrawn => {
                snapshot.omitted_elements.insert(element.source_key);
            }
            AuthoredResource::Omikuji(plan) => {
                let client = OmikujiClient::for_masterdata(masterdata, locale);
                snapshot
                    .omikuji_visuals
                    .insert(element.source_key, plan.visual(client));
            }
            AuthoredResource::Request(request) => {
                insert_request(snapshot, request, resource_metadata)
            }
            AuthoredResource::LitBadge { image, normal_map } => {
                insert_request(snapshot, image, resource_metadata);
                insert_request(snapshot, normal_map, resource_metadata);
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
    /// The row names a prefab whose content is not laid out: an omikuji
    /// collection of an unknown slip prefab. The element is not drawn and
    /// requests nothing.
    NotDrawn,
    /// The resource to load, keyed by [`resource_lookup_key`].
    Request(ProfileResourceRequest),
    /// A can-badge collection: its image, drawn with the lit badge material,
    /// and the material's normal map, keyed by
    /// [`collection_normal_map_lookup_key`].
    LitBadge {
        image: ProfileResourceRequest,
        normal_map: ProfileResourceRequest,
    },
    /// An omikuji collection: the fortune slip of its `targetId` row, drawn
    /// with the images [`OmikujiPlan::cover`] and [`OmikujiPlan::fortune`].
    Omikuji(OmikujiPlan),
}

/// Resolves the resource an authored image-like element draws: shapes, card
/// members, stamps and the `customProfile*Resources` image kinds. Keys follow
/// the master-data rows (`resourceLoadVal/fileName`, card and stamp asset
/// bundles); a missing row yields [`AuthoredResource::MissingRow`] rather than
/// an invented key. Collections follow their row's collection type: can
/// badges are [`AuthoredResource::LitBadge`], and omikuji collections
/// [`AuthoredResource::Omikuji`] when their `targetId` names an `omikujis` row
/// ([`AuthoredResource::MissingRow`] otherwise) and their prefab is one
/// [`crate::omikuji`] lays out ([`AuthoredResource::NotDrawn`] otherwise).
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
        ProfileElementRef::Collection(value) => {
            return collection_resource(masterdata, value.id, value.target_id)
        }
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

fn normal_map_metric() -> ResourceMetric {
    ResourceMetric {
        width: crate::badge_material::NORMAL_MAP_SIZE,
        height: crate::badge_material::NORMAL_MAP_SIZE,
    }
}

fn collection_resource(
    masterdata: &(impl ProfileMasterData + ?Sized),
    id: i32,
    target_id: Option<i32>,
) -> AuthoredResource {
    let Some(row) = masterdata.resolve_resource("collection", id) else {
        return AuthoredResource::MissingRow;
    };
    let image = row_resource_request(&row, "collection", id, SMALL_RESOURCE);
    match row.collection_type {
        CollectionResourceType::Omikuji => {
            let Some(omikuji) = target_id.and_then(|id| masterdata.resolve_omikuji(id)) else {
                return AuthoredResource::MissingRow;
            };
            match crate::omikuji::prefab(&row.load_value, &row.file_name) {
                Some(prefab) => AuthoredResource::Omikuji(OmikujiPlan {
                    prefab,
                    row: omikuji,
                }),
                None => AuthoredResource::NotDrawn,
            }
        }
        CollectionResourceType::CanBadge => AuthoredResource::LitBadge {
            image,
            normal_map: ProfileResourceRequest {
                lookup_key: collection_normal_map_lookup_key(id),
                resource: crate::badge_material::normal_map_resource(),
                fallback: normal_map_metric(),
            },
        },
        _ => AuthoredResource::Request(image),
    }
}

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
    let row = masterdata.resolve_resource(table_kind, id)?;
    Some(row_resource_request(&row, lookup_kind, id, fallback))
}

fn row_resource_request(
    row: &ResourceInfo,
    lookup_kind: &str,
    id: i32,
    fallback: ResourceMetric,
) -> ProfileResourceRequest {
    ProfileResourceRequest {
        lookup_key: resource_lookup_key(lookup_kind, id, ""),
        resource: ResourceKey {
            namespace: "assets".into(),
            key: format!("{}/{}", row.load_value, row.file_name),
        },
        fallback,
    }
}

fn insert_request(
    snapshot: &mut ProfileResolveSnapshot,
    request: ProfileResourceRequest,
    metadata: &impl ResourceMetadata,
) {
    let metric = metadata
        .metric(&request.resource)
        .unwrap_or(request.fallback);
    let kind = if request.resource.namespace == "static" {
        "renderer_static"
    } else {
        "master_data"
    };
    snapshot.resources.insert(
        request.lookup_key,
        ResourceDescriptor {
            resource: request.resource,
            natural_width: metric.width,
            natural_height: metric.height,
            provenance: BTreeMap::from([("kind".into(), ParameterValue::Text(kind.into()))]),
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
mod tests;
