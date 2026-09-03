use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::profile_source::*;
use crate::{
    AuthoredElementKind, FontRole, InteractionRegionSource, LayerKind, LayerSource,
    LineIndentSource, ParameterValue, Quad, Rect, SemanticCommandPayload, SemanticCommandSource,
    ShapePrimitive, StableId,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProfileElementRef<'a> {
    Text(&'a TextElement),
    Shape(&'a ShapeElement),
    CardMember(&'a CardMemberElement),
    Stamp(&'a StampElement),
    Other(&'a OtherElement),
    BondsHonor(&'a BondsHonorElement),
    Honor(&'a HonorElement),
    Collection(&'a CollectionElement),
    General(&'a GeneralElement),
    StandMember(&'a StandMemberElement),
    GeneralBackground(&'a GeneralBackgroundElement),
    StoryBackground(&'a StoryBackgroundElement),
}

impl ProfileElementRef<'_> {
    pub fn object(&self) -> &ObjectData {
        match self {
            Self::Text(value) => &value.object_data,
            Self::Shape(value) => &value.object_data,
            Self::CardMember(value) => &value.object_data,
            Self::Stamp(value) => &value.object_data,
            Self::Other(value) => &value.object_data,
            Self::BondsHonor(value) => &value.object_data,
            Self::Honor(value) => &value.object_data,
            Self::Collection(value) => &value.object_data,
            Self::General(value) => &value.object_data,
            Self::StandMember(value) => &value.object_data,
            Self::GeneralBackground(value) => &value.object_data,
            Self::StoryBackground(value) => &value.object_data,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedProfileElement<'a> {
    pub kind: AuthoredElementKind,
    pub source_index: usize,
    pub source_key: String,
    pub layer_id: StableId,
    pub value: ProfileElementRef<'a>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileComponentSnapshot {
    pub locale: String,
    pub region_fonts: BTreeMap<i32, String>,
    pub localized_text: BTreeMap<String, String>,
    pub user_name: String,
    pub word: String,
    #[serde(default)]
    pub user_rank: i32,
    #[serde(default)]
    pub total_power: i64,
    #[serde(default)]
    pub mvp: i32,
    #[serde(default)]
    pub superstar: i32,
    #[serde(default)]
    pub challenge_score: i32,
    #[serde(default)]
    pub challenge_character_id: i32,
    #[serde(default)]
    pub challenge_avatar: Option<ComponentImageSnapshot>,
    #[serde(default)]
    pub music_results: Option<MusicResultsSnapshot>,
    #[serde(default)]
    pub story_favorites: Vec<StoryFavoriteSnapshot>,
    #[serde(default)]
    pub player_avatar: Option<ComponentImageSnapshot>,
    #[serde(default)]
    pub character_ranks: Vec<CharacterRankSnapshot>,
    #[serde(default)]
    pub deck_members: Vec<CardVisualSnapshot>,
    #[serde(default)]
    pub leader_card: Option<CardVisualSnapshot>,
    #[serde(default)]
    pub honor_slots: Vec<HonorVisualSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComponentImageSnapshot {
    pub source_field: String,
    pub source_id: String,
    pub descriptor: Option<ResourceDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoryFavoriteSnapshot {
    pub story_id: i32,
    pub story_type: String,
    pub image: ComponentImageSnapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CharacterRankSnapshot {
    pub character_id: i32,
    pub rank: i32,
    pub challenge_rank: Option<i32>,
    pub avatar: ComponentImageSnapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CardVisualSnapshot {
    pub card_id: i32,
    pub after_training: bool,
    pub master_rank: i32,
    pub level: i32,
    pub rarity: String,
    pub attribute: String,
    pub image: ComponentImageSnapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HonorVisualSnapshot {
    pub source_field: String,
    pub source_id: String,
    pub honor_id: i32,
    pub honor_level: i32,
    pub full_size: bool,
    pub visual: HonorVisualKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HonorVisualKind {
    Standard {
        honor_type: String,
        has_star: bool,
        is_live_master: bool,
        progress: i32,
        background: Option<ResourceDescriptor>,
        frame_candidates: Vec<Option<ResourceDescriptor>>,
        overlay: Option<ResourceDescriptor>,
        star: Option<ResourceDescriptor>,
        star_high: Option<ResourceDescriptor>,
        live_star_on: Option<ResourceDescriptor>,
        live_star_off: Option<ResourceDescriptor>,
    },
    Bonds {
        character_ids: [i32; 2],
        backgrounds: [Option<ResourceDescriptor>; 2],
        characters: [Option<ResourceDescriptor>; 2],
        mask: Option<ResourceDescriptor>,
        frame: Option<ResourceDescriptor>,
        word: Option<ResourceDescriptor>,
        star: Option<ResourceDescriptor>,
        star_high: Option<ResourceDescriptor>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MusicDifficultySnapshot {
    pub clear: i32,
    pub full_combo: i32,
    pub all_perfect: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MusicResultsSnapshot {
    pub easy: MusicDifficultySnapshot,
    pub normal: MusicDifficultySnapshot,
    pub hard: MusicDifficultySnapshot,
    pub expert: MusicDifficultySnapshot,
    pub master: MusicDifficultySnapshot,
    pub append: MusicDifficultySnapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileComponentLowering {
    pub commands: Vec<SemanticCommandSource>,
    pub interaction_regions: Vec<InteractionRegionSource>,
    pub controls: Vec<ComponentControlSource>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComponentControlSource {
    pub id: StableId,
    pub layer_id: StableId,
    pub role: String,
    pub state: ComponentControlState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComponentControlState {
    Tabs {
        options: Vec<String>,
        active: String,
    },
    Scroll {
        offset: f32,
        min: f32,
        max: f32,
        viewport_extent: f32,
        content_extent: f32,
        step: f32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceDescriptor {
    pub resource: crate::ResourceKey,
    pub natural_width: f32,
    pub natural_height: f32,
    #[serde(default)]
    pub provenance: BTreeMap<String, ParameterValue>,
}

impl ResourceDescriptor {
    pub fn centered_bounds(&self) -> Rect {
        Rect {
            x: -self.natural_width / 2.0,
            y: -self.natural_height / 2.0,
            width: self.natural_width,
            height: self.natural_height,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileResolveSnapshot {
    pub fonts: BTreeMap<i32, String>,
    pub colors: BTreeMap<i32, [f32; 4]>,
    pub resources: BTreeMap<String, ResourceDescriptor>,
    pub line_indent: BTreeMap<String, LineIndentSource>,
    pub component: Option<ProfileComponentSnapshot>,
    #[serde(default)]
    pub honor_visuals: BTreeMap<String, HonorVisualSnapshot>,
    #[serde(default)]
    pub card_member_visuals: BTreeMap<String, CardVisualSnapshot>,
}

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
}

impl NormalizedProfileElement<'_> {
    pub fn object(&self) -> &ObjectData {
        self.value.object()
    }
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
    values.sort_by_key(|element| {
        (
            element.object().layer,
            element.kind as u8,
            element.source_index,
        )
    });
    values
}

pub fn authored_kind_name(kind: AuthoredElementKind) -> &'static str {
    match kind {
        AuthoredElementKind::Text => "text",
        AuthoredElementKind::Shape => "shape",
        AuthoredElementKind::CardMember => "card-member",
        AuthoredElementKind::Stamp => "stamp",
        AuthoredElementKind::Other => "other",
        AuthoredElementKind::BondsHonor => "bonds-honor",
        AuthoredElementKind::Honor => "honor",
        AuthoredElementKind::Collection => "collection",
        AuthoredElementKind::General => "general",
        AuthoredElementKind::StandMember => "stand-member",
        AuthoredElementKind::GeneralBackground => "general-background",
        AuthoredElementKind::StoryBackground => "story-background",
    }
}

pub fn semantic_command_id(source_key: &str, role: &str, ordinal: u32) -> StableId {
    StableId::derive(
        "semantic-command-v1",
        format!("{source_key}\0{role}\0{ordinal}").as_bytes(),
    )
}

pub fn interaction_region_id(source_key: &str, role: &str, ordinal: u32) -> StableId {
    StableId::derive(
        "interaction-region-v1",
        format!("{source_key}\0{role}\0{ordinal}").as_bytes(),
    )
}

pub fn component_control_id(source_key: &str, role: &str) -> StableId {
    StableId::derive(
        "component-control-v1",
        format!("{source_key}\0{role}").as_bytes(),
    )
}

pub fn assemble_profile_layer(
    element: &NormalizedProfileElement<'_>,
    kind: LayerKind,
    source_content: String,
    resolved_parameters: BTreeMap<String, ParameterValue>,
    commands: &[SemanticCommandSource],
    line_indent: Option<LineIndentSource>,
) -> LayerSource {
    let object = element.object();
    let (x, y, rotation_deg, scale_x, scale_y) =
        crate::profile_transform::extract_transform(object);
    let theta = rotation_deg.to_radians();
    let matrix = [
        theta.cos() * scale_x,
        theta.sin() * scale_x,
        -theta.sin() * scale_y,
        theta.cos() * scale_y,
        x,
        y,
    ];
    let bounds = union_command_bounds(commands);
    let hit_geometry = if bounds.width > 0.0 && bounds.height > 0.0 {
        rect_quad(bounds)
    } else {
        [[x, y], [x, y], [x, y], [x, y]]
    };
    LayerSource {
        id: element.layer_id,
        parent_id: None,
        kind,
        authored_kind: element.kind,
        authored_index: element.source_index as u32,
        game_layer: object.layer,
        z: object.layer,
        authored_visible: object.visible,
        source_content,
        resolved_parameters,
        bounds,
        quad: hit_geometry,
        matrix,
        hit_geometry,
        line_indent,
    }
}

pub fn default_profile_interaction_region(
    element: &NormalizedProfileElement<'_>,
    layer: &LayerSource,
    resolved_data: BTreeMap<String, ParameterValue>,
) -> InteractionRegionSource {
    InteractionRegionSource {
        id: interaction_region_id(&element.source_key, "primary", 0),
        layer_id: layer.id,
        role: "primary".into(),
        bounds: layer.bounds,
        quad: layer.quad,
        matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        hit_geometry: layer.hit_geometry,
        clip: None,
        control_bindings: Vec::new(),
        resolved_data,
        capabilities: vec!["inspect".into(), "select_layer".into()],
    }
}

pub fn rect_quad(rect: Rect) -> Quad {
    [
        [rect.x, rect.y],
        [rect.x + rect.width, rect.y],
        [rect.x + rect.width, rect.y + rect.height],
        [rect.x, rect.y + rect.height],
    ]
}

pub fn union_command_bounds(commands: &[SemanticCommandSource]) -> Rect {
    let mut iter = commands
        .iter()
        .map(|command| command.bounds)
        .filter(|bounds| bounds.width > 0.0 && bounds.height > 0.0);
    let Some(first) = iter.next() else {
        return Rect::default();
    };
    iter.fold(first, |union, bounds| {
        let left = union.x.min(bounds.x);
        let top = union.y.min(bounds.y);
        let right = (union.x + union.width).max(bounds.x + bounds.width);
        let bottom = (union.y + union.height).max(bounds.y + bounds.height);
        Rect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    })
}

pub fn lower_identity_general(
    general_type: i32,
    layer_id: StableId,
    source_key: &str,
    snapshot: &ProfileComponentSnapshot,
) -> Result<Option<ProfileComponentLowering>, ProfileResolveError> {
    let Some(recipe) =
        crate::general_recipe::build_general_recipe(general_type, layer_id, source_key, snapshot)?
    else {
        return Ok(None);
    };
    let commands = recipe
        .nodes
        .iter()
        .map(general_recipe_node_to_command)
        .collect();
    let mut interaction_regions = recipe.interaction_regions.clone();
    interaction_regions.extend(
        recipe
            .nodes
            .iter()
            .filter_map(|node| match &node.payload {
                crate::general_recipe::GeneralRecipePayload::Text {
                    source: crate::TextSource::ProfileField { field, .. },
                    ..
                } => Some(profile_text_interaction_region_from_bounds(
                    source_key,
                    node.layer_id,
                    &node.role,
                    node.bounds,
                    field,
                )),
                _ => None,
            })
            .collect::<Vec<_>>(),
    );
    Ok(Some(ProfileComponentLowering {
        commands,
        interaction_regions,
        controls: recipe.controls,
    }))
}

fn general_recipe_node_to_command(
    node: &crate::general_recipe::GeneralRecipeNode,
) -> SemanticCommandSource {
    use crate::general_recipe::{
        GeneralClip, GeneralFill, GeneralGeometry, GeneralImageSampling, GeneralImageSource,
        GeneralImageSourceConstraint, GeneralRecipePayload, GeneralTextAlign,
    };
    match &node.payload {
        GeneralRecipePayload::Shape {
            geometry,
            fill,
            stroke,
        } => {
            let primitive = match geometry {
                GeneralGeometry::Rect => ShapePrimitive::Rect,
                GeneralGeometry::RoundedRect { radius } => {
                    ShapePrimitive::RoundedRect { radius: *radius }
                }
                GeneralGeometry::Ellipse => ShapePrimitive::Ellipse,
            };
            let mut command = SemanticCommandSource::shape(
                node.id,
                node.layer_id,
                node.role.clone(),
                node.bounds,
                primitive,
            );
            if let SemanticCommandPayload::Shape {
                fill: target_fill,
                gradient,
                stroke: target_stroke,
                stroke_width,
                ..
            } = &mut command.payload
            {
                match fill {
                    GeneralFill::Solid(color) => *target_fill = *color,
                    GeneralFill::LinearGradient {
                        start, end, colors, ..
                    } => {
                        *target_fill = colors.first().copied().unwrap_or([0.0; 4]);
                        *gradient = Some(crate::LinearGradient {
                            start: *start,
                            end: *end,
                            start_color: colors.first().copied().unwrap_or([0.0; 4]),
                            end_color: colors.last().copied().unwrap_or([0.0; 4]),
                        });
                    }
                }
                if let Some(value) = stroke {
                    *target_stroke = value.color;
                    *stroke_width = value.width;
                }
            }
            command.matrix = node.local_matrix;
            command.control_bindings = node.control_bindings.clone();
            command.clip = node.clips.iter().find_map(|clip| match clip {
                GeneralClip::Rect { bounds, .. } => Some(rect_quad(*bounds)),
                GeneralClip::RoundedRect { .. } | GeneralClip::Ellipse { .. } => None,
            });
            command
        }
        GeneralRecipePayload::Text {
            source,
            font_role,
            font_size,
            color,
            align,
            line_spacing,
            wrap,
            render_baseline,
        } => {
            let value = match source {
                crate::TextSource::Authored { value }
                | crate::TextSource::ProfileField { value, .. }
                | crate::TextSource::MasterData { value, .. }
                | crate::TextSource::Localized { value, .. } => value,
            };
            let mut command = SemanticCommandSource::text(
                node.id,
                node.layer_id,
                node.role.clone(),
                value,
                font_role.clone(),
            );
            command.bounds = node.bounds;
            command.matrix = node.local_matrix;
            command.matrix[4] = node.bounds.x + node.bounds.width / 2.0;
            command.matrix[5] = node.bounds.y + node.bounds.height / 2.0;
            command.hit_geometry = rect_quad(node.bounds);
            command.clip = node.clips.iter().find_map(|clip| match clip {
                GeneralClip::Rect { bounds, .. } => Some(rect_quad(*bounds)),
                GeneralClip::RoundedRect { .. } | GeneralClip::Ellipse { .. } => None,
            });
            command.control_bindings = node.control_bindings.clone();
            if let SemanticCommandPayload::Text {
                source: target_source,
                size,
                color: target_color,
                line_spacing: target_line_spacing,
                alignment,
                max_width,
                max_height,
                ..
            } = &mut command.payload
            {
                *target_source = source.clone();
                *size = *font_size / 2.0;
                *target_color = *color;
                *target_line_spacing = *line_spacing;
                *alignment = match align {
                    GeneralTextAlign::Left => 1,
                    GeneralTextAlign::Center => 2,
                    GeneralTextAlign::Right => 4,
                };
                *max_width = wrap.then_some(node.bounds.width);
                *max_height = None;
            }
            let anchor_x = match align {
                GeneralTextAlign::Left => -node.bounds.width / 4.0,
                GeneralTextAlign::Center => 0.0,
                GeneralTextAlign::Right => node.bounds.width / 4.0,
            };
            command.render_placement = Some(crate::TextRenderPlacementSource {
                anchor_x,
                baseline: (*render_baseline).or_else(|| (!wrap).then_some(*font_size * 0.35 / 2.0)),
            });
            command
        }
        GeneralRecipePayload::Image {
            resource,
            natural_size,
            source,
            sampling,
            source_constraint,
            tint,
            source_field,
            source_id,
            provenance,
            blend_mode,
        } => {
            let mut command = SemanticCommandSource::image(
                node.id,
                node.layer_id,
                node.role.clone(),
                resource.clone(),
                node.bounds,
            );
            command.matrix = node.local_matrix;
            command.blend_mode = *blend_mode;
            command.control_bindings = node.control_bindings.clone();
            command.hit_geometry = rect_quad(node.bounds);
            command.clip = node.clips.iter().find_map(|clip| match clip {
                GeneralClip::Rect { bounds, .. } => Some(rect_quad(*bounds)),
                GeneralClip::RoundedRect { .. } | GeneralClip::Ellipse { .. } => None,
            });
            if let Some(value) = source_field {
                command
                    .metadata
                    .insert("source_field".into(), ParameterValue::Text(value.clone()));
            }
            if let Some(value) = source_id {
                command
                    .metadata
                    .insert("source_id".into(), ParameterValue::Text(value.clone()));
            }
            if let Some([width, height]) = natural_size {
                command.metadata.insert(
                    "resource_natural_size".into(),
                    ParameterValue::Vec2([*width, *height]),
                );
            }
            command.metadata.insert(
                "sampling".into(),
                ParameterValue::Text(
                    match sampling {
                        GeneralImageSampling::Nearest => "nearest",
                        GeneralImageSampling::Linear => "linear",
                    }
                    .into(),
                ),
            );
            command.metadata.insert(
                "source_constraint".into(),
                ParameterValue::Text(
                    match source_constraint {
                        GeneralImageSourceConstraint::Implicit => "implicit",
                        GeneralImageSourceConstraint::Fast => "fast",
                        GeneralImageSourceConstraint::Strict => "strict",
                    }
                    .into(),
                ),
            );
            for (key, value) in provenance {
                command
                    .metadata
                    .insert(format!("resource_source.{key}"), value.clone());
            }
            if let SemanticCommandPayload::Image {
                uv,
                tint: target_tint,
                clip,
                ..
            } = &mut command.payload
            {
                *target_tint = *tint;
                *uv = match source {
                    GeneralImageSource::WholeImageImplicit
                    | GeneralImageSource::WholeImageExplicit => Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 1.0,
                        height: 1.0,
                    },
                    GeneralImageSource::Pixels(rect) => {
                        let [width, height] = natural_size
                            .expect("pixel image source requires resolved natural dimensions");
                        Rect {
                            x: rect.x / width.max(f32::EPSILON),
                            y: rect.y / height.max(f32::EPSILON),
                            width: rect.width / width.max(f32::EPSILON),
                            height: rect.height / height.max(f32::EPSILON),
                        }
                    }
                };
                if let GeneralImageSource::Pixels(rect) = source {
                    command.metadata.insert(
                        "source_rect_origin".into(),
                        ParameterValue::Vec2([rect.x, rect.y]),
                    );
                    command.metadata.insert(
                        "source_rect_size".into(),
                        ParameterValue::Vec2([rect.width, rect.height]),
                    );
                }
                if let Some(intrinsic) = node.clips.iter().find(|clip| {
                    matches!(
                        clip,
                        GeneralClip::Ellipse { .. } | GeneralClip::RoundedRect { .. }
                    )
                }) {
                    match intrinsic {
                        GeneralClip::Ellipse { .. } => *clip = Some(crate::ImageClip::Ellipse),
                        GeneralClip::RoundedRect { radius, .. } => {
                            *clip = Some(crate::ImageClip::RoundedRect { radius: *radius })
                        }
                        GeneralClip::Rect { .. } => unreachable!(),
                    }
                }
            }
            command
        }
        GeneralRecipePayload::Group { phase, .. } => {
            let mut command = SemanticCommandSource::composite(
                node.id,
                node.layer_id,
                node.role.clone(),
                node.bounds,
            );
            command.matrix = node.local_matrix;
            command.control_bindings = node.control_bindings.clone();
            if let SemanticCommandPayload::Composite { operation, .. } = &mut command.payload {
                *operation = match phase {
                    crate::general_recipe::GeneralGroupPhase::Begin => {
                        crate::CompositeOperation::BeginIsolation
                    }
                    crate::general_recipe::GeneralGroupPhase::End => {
                        crate::CompositeOperation::EndIsolation
                    }
                };
            }
            command
        }
    }
}

fn profile_text_interaction_region_from_bounds(
    source_key: &str,
    layer_id: StableId,
    role: &str,
    bounds: Rect,
    field: &str,
) -> InteractionRegionSource {
    InteractionRegionSource {
        id: interaction_region_id(source_key, role, 0),
        layer_id,
        role: role.into(),
        bounds,
        quad: rect_quad(bounds),
        matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        hit_geometry: rect_quad(bounds),
        clip: Some(rect_quad(bounds)),
        control_bindings: Vec::new(),
        resolved_data: BTreeMap::from([("field".into(), ParameterValue::Text(field.into()))]),
        capabilities: vec!["inspect".into(), "select_text".into(), "edit_text".into()],
    }
}

fn lower_honor_visual(
    source_key: &str,
    layer_id: StableId,
    honor: &HonorVisualSnapshot,
    layout: crate::profile_layout::ElementLayout,
    ordinal: &mut u32,
    commands: &mut Vec<SemanticCommandSource>,
) -> Result<(), ProfileResolveError> {
    let (w, h) = (layout.w, layout.h);
    let origin_x = layout.cx;
    let origin_y = -layout.cy;
    let full_bounds = Rect {
        x: origin_x - w / 2.0,
        y: origin_y - h / 2.0,
        width: w,
        height: h,
    };
    match &honor.visual {
        HonorVisualKind::Standard {
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
        } => {
            if let Some(background) = background {
                commands.push(descriptor_image_command(
                    source_key,
                    layer_id,
                    &format!("honor-{}-background", honor.honor_id),
                    *ordinal,
                    background,
                    full_bounds,
                    None,
                    None,
                ));
                *ordinal += 1;
            }
            if let Some(frame) = frame_candidates.iter().flatten().next() {
                let frame_bounds = Rect {
                    x: origin_x - frame.natural_width / 2.0,
                    y: origin_y - h / 2.0,
                    width: frame.natural_width,
                    height: frame.natural_height,
                };
                commands.push(descriptor_image_command(
                    source_key,
                    layer_id,
                    &format!("honor-{}-frame", honor.honor_id),
                    *ordinal,
                    frame,
                    frame_bounds,
                    None,
                    None,
                ));
                *ordinal += 1;
            }
            if let Some(overlay) = overlay {
                let (dx, dy) = if *is_live_master {
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
                } else if (honor.full_size && overlay.natural_width == 380.0)
                    || (!honor.full_size && overlay.natural_height == 80.0)
                {
                    (0.0, 0.0)
                } else if honor.full_size {
                    (190.0, 0.0)
                } else {
                    (34.0, 42.0)
                };
                let overlay_bounds = Rect {
                    x: origin_x - w / 2.0 + dx,
                    y: origin_y - h / 2.0 + dy,
                    width: overlay.natural_width,
                    height: overlay.natural_height,
                };
                commands.push(descriptor_image_command(
                    source_key,
                    layer_id,
                    &format!("honor-{}-overlay", honor.honor_id),
                    *ordinal,
                    overlay,
                    overlay_bounds,
                    None,
                    None,
                ));
                *ordinal += 1;
            }
            if *is_live_master {
                let text_layout = crate::profile_layout::ElementLayout {
                    cx: origin_x - w / 2.0 + if honor.full_size { 270.0 } else { 90.0 },
                    cy: -(origin_y - h / 2.0 + 60.0),
                    w: 80.0,
                    h: 20.0,
                };
                commands.push(profile_value_text(
                    source_key,
                    layer_id,
                    &format!("honor-{}-progress", honor.honor_id),
                    *ordinal,
                    &format!("userHonorMissions.{}", honor.honor_id),
                    progress.to_string(),
                    text_layout,
                    2,
                    20.0,
                    [1.0; 4],
                ));
                *ordinal += 1;
            } else if *has_star && matches!(honor_type.as_str(), "character" | "achievement") {
                if let (Some(star), Some(star_high)) = (star, star_high) {
                    lower_honor_stars(
                        source_key, layer_id, honor, layout, star, star_high, ordinal, commands,
                    );
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
            let group_bounds = Rect {
                x: origin_x - w / 2.0,
                y: origin_y - h / 2.0,
                width: w,
                height: h,
            };
            commands.push(composite_command(
                source_key,
                layer_id,
                &format!("bonds-{}-isolation-begin", honor.honor_id),
                *ordinal,
                group_bounds,
                crate::CompositeOperation::BeginIsolation,
            ));
            *ordinal += 1;
            for (index, background) in backgrounds.iter().enumerate() {
                let Some(background) = background else {
                    continue;
                };
                let bounds = Rect {
                    x: origin_x - w / 2.0 + index as f32 * w / 2.0,
                    y: origin_y - h / 2.0,
                    width: w / 2.0,
                    height: h,
                };
                let uv = if index == 0 {
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 0.5,
                        height: 1.0,
                    }
                } else {
                    Rect {
                        x: 0.5,
                        y: 0.0,
                        width: 0.5,
                        height: 1.0,
                    }
                };
                commands.push(descriptor_image_command(
                    source_key,
                    layer_id,
                    &format!("bonds-{}-background-{index}", honor.honor_id),
                    *ordinal,
                    background,
                    bounds,
                    Some(uv),
                    None,
                ));
                *ordinal += 1;
            }
            let offset = if honor.full_size { 120.0 } else { 30.0 };
            for (index, character) in characters.iter().enumerate() {
                let Some(character) = character else { continue };
                let sw = character.natural_width * 0.8;
                let sh = character.natural_height * 0.8;
                let center = origin_x + if index == 0 { -offset } else { offset };
                let raw_left = center - sw / 2.0;
                let clipped_left = if index == 0 {
                    raw_left
                } else {
                    raw_left.max(origin_x)
                };
                let clipped_right = if index == 0 {
                    (raw_left + sw).min(origin_x)
                } else {
                    raw_left + sw
                };
                let width = (clipped_right - clipped_left).max(0.0);
                let uv_x = (clipped_left - raw_left) / sw.max(f32::EPSILON);
                let uv = Rect {
                    x: uv_x,
                    y: 0.0,
                    width: width / sw.max(f32::EPSILON),
                    height: 1.0,
                };
                let bounds = Rect {
                    x: clipped_left,
                    y: origin_y + h / 2.0 - sh,
                    width,
                    height: sh,
                };
                commands.push(descriptor_image_command(
                    source_key,
                    layer_id,
                    &format!("bonds-{}-character-{index}", honor.honor_id),
                    *ordinal,
                    character,
                    bounds,
                    Some(uv),
                    None,
                ));
                *ordinal += 1;
            }
            if let Some(mask) = mask {
                let mut command = descriptor_image_command(
                    source_key,
                    layer_id,
                    &format!("bonds-{}-mask", honor.honor_id),
                    *ordinal,
                    mask,
                    group_bounds,
                    None,
                    None,
                );
                command.blend_mode = crate::BlendMode::DstIn;
                commands.push(command);
                *ordinal += 1;
            }
            commands.push(composite_command(
                source_key,
                layer_id,
                &format!("bonds-{}-isolation-end", honor.honor_id),
                *ordinal,
                group_bounds,
                crate::CompositeOperation::EndIsolation,
            ));
            *ordinal += 1;
            if let Some(frame) = frame {
                let frame_bounds = Rect {
                    x: origin_x - frame.natural_width / 2.0,
                    y: origin_y - h / 2.0,
                    width: frame.natural_width,
                    height: frame.natural_height,
                };
                commands.push(descriptor_image_command(
                    source_key,
                    layer_id,
                    &format!("bonds-{}-frame", honor.honor_id),
                    *ordinal,
                    frame,
                    frame_bounds,
                    None,
                    None,
                ));
                *ordinal += 1;
            }
            if let Some(word) = word {
                let bounds = Rect {
                    x: origin_x - word.natural_width / 2.0,
                    y: origin_y - word.natural_height / 2.0,
                    width: word.natural_width,
                    height: word.natural_height,
                };
                commands.push(descriptor_image_command(
                    source_key,
                    layer_id,
                    &format!("bonds-{}-word", honor.honor_id),
                    *ordinal,
                    word,
                    bounds,
                    None,
                    None,
                ));
                *ordinal += 1;
            }
            if let (Some(star), Some(star_high)) = (star, star_high) {
                lower_honor_stars(
                    source_key, layer_id, honor, layout, star, star_high, ordinal, commands,
                );
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn lower_honor_stars(
    source_key: &str,
    layer_id: StableId,
    honor: &HonorVisualSnapshot,
    layout: crate::profile_layout::ElementLayout,
    star: &ResourceDescriptor,
    star_high: &ResourceDescriptor,
    ordinal: &mut u32,
    commands: &mut Vec<SemanticCommandSource>,
) {
    let mut level = honor.honor_level % 10;
    if level == 0 && honor.honor_level > 0 {
        level = 10;
    }
    let origin_x = layout.cx;
    let origin_y = -layout.cy;
    let base_x = if honor.full_size {
        origin_x - layout.w / 2.0 + 54.0
    } else {
        origin_x - 40.0
    };
    let base_y = origin_y - layout.h / 2.0 + 63.0;
    for index in 0..level.min(5) {
        let bounds = Rect {
            x: base_x + index as f32 * 16.0,
            y: base_y,
            width: star.natural_width,
            height: star.natural_height,
        };
        commands.push(descriptor_image_command(
            source_key,
            layer_id,
            &format!("honor-{}-star-{index}", honor.honor_id),
            *ordinal,
            star,
            bounds,
            None,
            None,
        ));
        *ordinal += 1;
    }
    for index in 0..(level - 5).max(0) {
        let bounds = Rect {
            x: base_x + index as f32 * 16.0,
            y: base_y,
            width: star_high.natural_width,
            height: star_high.natural_height,
        };
        commands.push(descriptor_image_command(
            source_key,
            layer_id,
            &format!("honor-{}-star-high-{index}", honor.honor_id),
            *ordinal,
            star_high,
            bounds,
            None,
            None,
        ));
        *ordinal += 1;
    }
}

fn honor_interaction_region(
    source_key: &str,
    layer_id: StableId,
    ordinal: u32,
    layout: crate::profile_layout::ElementLayout,
    honor: &HonorVisualSnapshot,
) -> InteractionRegionSource {
    component_interaction_region(
        source_key,
        layer_id,
        &format!("honor-{}", honor.honor_id),
        ordinal,
        layout,
        BTreeMap::from([
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
        ]),
        &["inspect", "select_item"],
    )
}

#[allow(clippy::too_many_arguments)]
pub fn resource_lookup_key(kind: &str, id: i32, variant: &str) -> String {
    format!("{kind}\0{id}\0{variant}")
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
        let (layer_kind, source_content, primary) = lower_primary_command(
            element.value,
            element.layer_id,
            command_id,
            snapshot,
            &mut parameters,
        )?;
        let mut layer_commands = vec![primary];
        let mut layer_regions = Vec::new();
        let mut layer_controls = Vec::new();
        if matches!(
            element.value,
            ProfileElementRef::Honor(_) | ProfileElementRef::BondsHonor(_)
        ) {
            if let Some(visual) = snapshot.honor_visuals.get(&element.source_key) {
                let layout = crate::profile_layout::ElementLayout {
                    cx: 0.0,
                    cy: 0.0,
                    w: if visual.full_size { 380.0 } else { 180.0 },
                    h: 80.0,
                };
                let mut ordinal = 0;
                layer_commands.clear();
                lower_honor_visual(
                    &element.source_key,
                    element.layer_id,
                    visual,
                    layout,
                    &mut ordinal,
                    &mut layer_commands,
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
        if let ProfileElementRef::CardMember(card_member) = element.value {
            if card_member.show_master_rank.unwrap_or(false) {
                if let Some(visual) = snapshot.card_member_visuals.get(&element.source_key) {
                    let bounds = layer_commands[0].bounds;
                    let overlay = crate::general_recipe::build_card_member_overlay_recipe(
                        card_member.member_type.unwrap_or(2),
                        element.layer_id,
                        &element.source_key,
                        bounds,
                        visual,
                    );
                    layer_commands.extend(overlay.iter().map(general_recipe_node_to_command));
                }
            }
        }
        if let (ProfileElementRef::General(general), Some(component)) =
            (element.value, snapshot.component.as_ref())
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

fn lower_primary_command(
    element: ProfileElementRef<'_>,
    layer_id: StableId,
    command_id: StableId,
    snapshot: &ProfileResolveSnapshot,
    parameters: &mut BTreeMap<String, ParameterValue>,
) -> Result<(LayerKind, String, SemanticCommandSource), ProfileResolveError> {
    Ok(match element {
        ProfileElementRef::Text(text) => {
            let family = snapshot
                .fonts
                .get(&text.font_id)
                .cloned()
                .ok_or(ProfileResolveError::MissingRegionFont(text.font_id))?;
            parameters.insert("font_id".into(), ParameterValue::I64(text.font_id.into()));
            parameters.insert("font_family".into(), ParameterValue::Text(family));
            let mut command = SemanticCommandSource::text(
                command_id,
                layer_id,
                "text",
                text.text.clone(),
                FontRole::RegionFontId(text.font_id),
            );
            if let SemanticCommandPayload::Text {
                size,
                line_spacing,
                outline_size,
                alignment,
                color,
                outline_color,
                ..
            } = &mut command.payload
            {
                *size = text.size;
                *line_spacing = text.line_spacing;
                *outline_size = text.outline_size;
                *alignment = (text.text_type & 0x07) as u8;
                *color = snapshot
                    .colors
                    .get(&text.color_id)
                    .copied()
                    .unwrap_or([1.0; 4]);
                *outline_color = snapshot
                    .colors
                    .get(&text.outline_color_id)
                    .copied()
                    .unwrap_or([0.0; 4]);
            }
            (LayerKind::Text, text.text.clone(), command)
        }
        ProfileElementRef::Shape(shape) => {
            parameters.insert("resource_id".into(), ParameterValue::I64(shape.id.into()));
            let lookup_key = resource_lookup_key("shape", shape.id, "");
            let descriptor = snapshot
                .resources
                .get(&lookup_key)
                .ok_or_else(|| ProfileResolveError::MissingResource(lookup_key.clone()))?;
            record_resource_parameters(parameters, descriptor);
            let mut command = SemanticCommandSource::shape(
                command_id,
                layer_id,
                "shape",
                descriptor.centered_bounds(),
                ShapePrimitive::AssetMask {
                    resource: descriptor.resource.clone(),
                },
            );
            if let SemanticCommandPayload::Shape {
                fill,
                stroke,
                stroke_width,
                ..
            } = &mut command.payload
            {
                *fill = with_alpha(
                    snapshot
                        .colors
                        .get(&shape.color_id)
                        .copied()
                        .unwrap_or([1.0; 4]),
                    shape.alpha,
                );
                *stroke = with_alpha(
                    snapshot
                        .colors
                        .get(&shape.outline_color_id)
                        .copied()
                        .unwrap_or([0.0; 4]),
                    shape.outline_alpha,
                );
                *stroke_width = shape.outline_size;
            }
            (LayerKind::Shape, String::new(), command)
        }
        ProfileElementRef::CardMember(value) => {
            parameters.insert("resource_id".into(), ParameterValue::I64(value.id.into()));
            parameters.insert(
                "show_master_rank".into(),
                ParameterValue::Bool(value.show_master_rank.unwrap_or(false)),
            );
            let member_type = value.member_type.unwrap_or(2);
            let training = if value.use_after_special_training.unwrap_or(false) {
                "after_training"
            } else {
                "normal"
            };
            let mut resolved = resolved_image(
                snapshot,
                "card-member",
                value.id,
                &format!("{member_type}:{training}"),
                layer_id,
                command_id,
                "card-member",
                parameters,
            )?;
            if member_type == 1 {
                let descriptor = snapshot
                    .resources
                    .get(&resource_lookup_key(
                        "card-member",
                        value.id,
                        &format!("{member_type}:{training}"),
                    ))
                    .ok_or_else(|| {
                        ProfileResolveError::MissingResource(format!("card-member:{}", value.id))
                    })?;
                resolved.2.bounds = Rect {
                    x: -156.0,
                    y: -256.0,
                    width: 312.0,
                    height: 512.0,
                };
                if let SemanticCommandPayload::Image { uv, .. } = &mut resolved.2.payload {
                    let crop_width = descriptor.natural_width.min(312.0);
                    let crop_height = descriptor.natural_height.min(512.0);
                    *uv = Rect {
                        x: ((descriptor.natural_width - crop_width).max(0.0) * 0.5)
                            / descriptor.natural_width.max(f32::EPSILON),
                        y: 0.0,
                        width: crop_width / descriptor.natural_width.max(f32::EPSILON),
                        height: crop_height / descriptor.natural_height.max(f32::EPSILON),
                    };
                }
                record_resource_parameters(parameters, descriptor);
                parameters.insert(
                    "image_fit".into(),
                    ParameterValue::Text("native-center-top-crop".into()),
                );
            }
            resolved
        }
        ProfileElementRef::Stamp(value) => resolved_image_with_parameter(
            snapshot, "stamp", value.id, "", layer_id, command_id, "stamp", parameters,
        )?,
        ProfileElementRef::Other(value) => resolved_image_with_parameter(
            snapshot, "other", value.id, "", layer_id, command_id, "other", parameters,
        )?,
        ProfileElementRef::Collection(value) => resolved_image_with_parameter(
            snapshot,
            "collection",
            value.id,
            "",
            layer_id,
            command_id,
            "collection",
            parameters,
        )?,
        ProfileElementRef::StandMember(value) => resolved_image_with_parameter(
            snapshot,
            "stand-member",
            value.id,
            "",
            layer_id,
            command_id,
            "stand-member",
            parameters,
        )?,
        ProfileElementRef::GeneralBackground(value) => resolved_image_with_parameter(
            snapshot,
            "general-background",
            value.id,
            "",
            layer_id,
            command_id,
            "general-background",
            parameters,
        )?,
        ProfileElementRef::StoryBackground(value) => resolved_image_with_parameter(
            snapshot,
            "story-background",
            value.id,
            "",
            layer_id,
            command_id,
            "story-background",
            parameters,
        )?,
        ProfileElementRef::BondsHonor(value) => {
            composite_primary(layer_id, command_id, "bonds-honor", value.id, parameters)
        }
        ProfileElementRef::Honor(value) => {
            composite_primary(layer_id, command_id, "honor", value.id, parameters)
        }
        ProfileElementRef::General(value) => {
            let general_type = value.general_type.unwrap_or_default();
            parameters.insert(
                "general_type".into(),
                ParameterValue::I64(general_type.into()),
            );
            composite_primary(layer_id, command_id, "general", general_type, parameters)
        }
    })
}

fn resolved_image(
    snapshot: &ProfileResolveSnapshot,
    kind: &str,
    id: i32,
    variant: &str,
    layer_id: StableId,
    command_id: StableId,
    role: &str,
    parameters: &mut BTreeMap<String, ParameterValue>,
) -> Result<(LayerKind, String, SemanticCommandSource), ProfileResolveError> {
    let lookup_key = resource_lookup_key(kind, id, variant);
    let descriptor = snapshot
        .resources
        .get(&lookup_key)
        .ok_or_else(|| ProfileResolveError::MissingResource(lookup_key.clone()))?;
    record_resource_parameters(parameters, descriptor);
    Ok((
        LayerKind::Image,
        String::new(),
        SemanticCommandSource::image(
            command_id,
            layer_id,
            role,
            descriptor.resource.clone(),
            descriptor.centered_bounds(),
        ),
    ))
}

#[allow(clippy::too_many_arguments)]
fn resolved_image_with_parameter(
    snapshot: &ProfileResolveSnapshot,
    kind: &str,
    id: i32,
    variant: &str,
    layer_id: StableId,
    command_id: StableId,
    role: &str,
    parameters: &mut BTreeMap<String, ParameterValue>,
) -> Result<(LayerKind, String, SemanticCommandSource), ProfileResolveError> {
    parameters.insert("resource_id".into(), ParameterValue::I64(id.into()));
    resolved_image(
        snapshot, kind, id, variant, layer_id, command_id, role, parameters,
    )
}

fn record_resource_parameters(
    parameters: &mut BTreeMap<String, ParameterValue>,
    descriptor: &ResourceDescriptor,
) {
    parameters.insert(
        "resource_namespace".into(),
        ParameterValue::Text(descriptor.resource.namespace.clone()),
    );
    parameters.insert(
        "resource_key".into(),
        ParameterValue::Text(descriptor.resource.key.clone()),
    );
    parameters.insert(
        "resource_width".into(),
        ParameterValue::F64(descriptor.natural_width.into()),
    );
    parameters.insert(
        "resource_height".into(),
        ParameterValue::F64(descriptor.natural_height.into()),
    );
    for (key, value) in &descriptor.provenance {
        parameters.insert(format!("resource_source.{key}"), value.clone());
    }
}

fn composite_primary(
    layer_id: StableId,
    command_id: StableId,
    role: &str,
    id: i32,
    parameters: &mut BTreeMap<String, ParameterValue>,
) -> (LayerKind, String, SemanticCommandSource) {
    parameters.insert("resource_id".into(), ParameterValue::I64(id.into()));
    (
        LayerKind::Composite,
        String::new(),
        SemanticCommandSource::composite(command_id, layer_id, role, Rect::default()),
    )
}

fn with_alpha(mut color: [f32; 4], alpha: f32) -> [f32; 4] {
    color[3] = alpha.clamp(0.0, 1.0);
    color
}

fn configure_text_command(
    command: &mut SemanticCommandSource,
    layout: crate::profile_layout::ElementLayout,
    alignment: u8,
    size: f32,
    line_spacing: f32,
) {
    command.bounds = layout_bounds(layout);
    command.matrix[4] = match alignment {
        2 => layout.cx,
        3 => layout.cx + layout.w / 2.0,
        _ => layout.cx - layout.w / 2.0,
    };
    command.matrix[5] = -layout.cy;
    command.hit_geometry = rect_quad(command.bounds);
    if let SemanticCommandPayload::Text {
        size: command_size,
        line_spacing: command_line_spacing,
        alignment: command_alignment,
        max_width,
        max_height,
        ..
    } = &mut command.payload
    {
        *command_size = size;
        *command_line_spacing = line_spacing;
        *command_alignment = alignment;
        *max_width = Some(layout.w);
        *max_height = Some(layout.h);
    }
}

fn set_text_color(command: &mut SemanticCommandSource, color: [f32; 4]) {
    if let SemanticCommandPayload::Text { color: target, .. } = &mut command.payload {
        *target = color;
    }
}

#[allow(clippy::too_many_arguments)]
fn profile_value_text(
    source_key: &str,
    layer_id: StableId,
    role: &str,
    ordinal: u32,
    field: &str,
    value: String,
    layout: crate::profile_layout::ElementLayout,
    alignment: u8,
    size: f32,
    color: [f32; 4],
) -> SemanticCommandSource {
    let mut command = SemanticCommandSource::profile_text(
        semantic_command_id(source_key, role, ordinal),
        layer_id,
        role,
        field,
        value,
        FontRole::RegionFontId(1),
    );
    configure_text_command(&mut command, layout, alignment, size, 0.0);
    set_text_color(&mut command, color);
    command
}

fn descriptor_image_command(
    source_key: &str,
    layer_id: StableId,
    role: &str,
    ordinal: u32,
    descriptor: &ResourceDescriptor,
    bounds: Rect,
    uv: Option<Rect>,
    alpha_mask: Option<&crate::ResourceKey>,
) -> SemanticCommandSource {
    let mut command = SemanticCommandSource::image(
        semantic_command_id(source_key, role, ordinal),
        layer_id,
        role,
        descriptor.resource.clone(),
        bounds,
    );
    command.hit_geometry = rect_quad(bounds);
    for (key, value) in &descriptor.provenance {
        command
            .metadata
            .insert(format!("resource_source.{key}"), value.clone());
    }
    if let SemanticCommandPayload::Image {
        uv: target_uv,
        alpha_mask: target_mask,
        ..
    } = &mut command.payload
    {
        if let Some(uv) = uv {
            *target_uv = uv;
        }
        *target_mask = alpha_mask.cloned();
    }
    command
}

fn composite_command(
    source_key: &str,
    layer_id: StableId,
    role: &str,
    ordinal: u32,
    bounds: Rect,
    operation: crate::CompositeOperation,
) -> SemanticCommandSource {
    let mut command = SemanticCommandSource::composite(
        semantic_command_id(source_key, role, ordinal),
        layer_id,
        role,
        bounds,
    );
    if let SemanticCommandPayload::Composite {
        operation: target, ..
    } = &mut command.payload
    {
        *target = operation;
    }
    command
}

fn component_interaction_region(
    source_key: &str,
    layer_id: StableId,
    role: &str,
    ordinal: u32,
    layout: crate::profile_layout::ElementLayout,
    resolved_data: BTreeMap<String, ParameterValue>,
    capabilities: &[&str],
) -> InteractionRegionSource {
    let bounds = layout_bounds(layout);
    InteractionRegionSource {
        id: interaction_region_id(source_key, role, ordinal),
        layer_id,
        role: role.into(),
        bounds,
        quad: rect_quad(bounds),
        matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        hit_geometry: rect_quad(bounds),
        clip: None,
        control_bindings: Vec::new(),
        resolved_data,
        capabilities: capabilities.iter().map(|value| (*value).into()).collect(),
    }
}

fn layout_bounds(layout: crate::profile_layout::ElementLayout) -> Rect {
    Rect {
        x: layout.cx - layout.w / 2.0,
        y: -layout.cy - layout.h / 2.0,
        width: layout.w,
        height: layout.h,
    }
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
