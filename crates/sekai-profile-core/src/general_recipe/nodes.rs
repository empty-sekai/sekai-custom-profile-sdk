//! Node constructors, item regions, layout rectangles and text sources the panel
//! builders share.

use super::{
    recipe_node_id, GeneralClip, GeneralFill, GeneralGeometry, GeneralGroupPhase,
    GeneralImageSampling, GeneralImageSource, GeneralImageSourceConstraint, GeneralRecipeNode,
    GeneralRecipePayload, GeneralTextAlign,
};
use crate::profile_scene::{
    ComponentImageSnapshot, ProfileComponentSnapshot, ProfileResolveError, ResourceDescriptor,
};
use crate::{
    BlendMode, FontRole, InteractionRegionSource, ParameterValue, Rect, ResourceKey, StableId,
    TextSource,
};

/// The panel a recipe builds nodes for: the layer its nodes are drawn on and
/// the source key their ids derive from.
#[derive(Clone, Copy)]
pub(super) struct RecipeLayer<'a> {
    pub(super) layer_id: StableId,
    pub(super) source_key: &'a str,
}

impl<'a> RecipeLayer<'a> {
    /// Key of the node with `role`. `ordinal` tells apart the ids of nodes
    /// that share a role.
    pub(super) fn key(self, role: &'a str, ordinal: u32) -> NodeKey<'a> {
        NodeKey {
            layer: self,
            role,
            ordinal,
        }
    }
}

/// Identity of one recipe node: the panel it belongs to, its role, and the
/// ordinal its id derives from together with the role.
#[derive(Clone, Copy)]
pub(super) struct NodeKey<'a> {
    layer: RecipeLayer<'a>,
    role: &'a str,
    ordinal: u32,
}

impl NodeKey<'_> {
    /// Node drawing `payload` in `bounds`, untransformed, with no clip and
    /// no control binding.
    pub(super) fn node(self, bounds: Rect, payload: GeneralRecipePayload) -> GeneralRecipeNode {
        GeneralRecipeNode {
            id: recipe_node_id(self.layer.source_key, self.role, self.ordinal),
            layer_id: self.layer.layer_id,
            role: self.role.into(),
            parent_id: None,
            bounds,
            local_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            clips: Vec::new(),
            control_bindings: Vec::new(),
            payload,
        }
    }
}

pub(super) fn shape_node(
    key: NodeKey<'_>,
    bounds: Rect,
    geometry: GeneralGeometry,
    color: [f32; 4],
) -> GeneralRecipeNode {
    key.node(
        bounds,
        GeneralRecipePayload::Shape {
            geometry,
            fill: GeneralFill::Solid(color),
            stroke: None,
        },
    )
}

pub(super) fn group_node(
    key: NodeKey<'_>,
    bounds: Rect,
    phase: GeneralGroupPhase,
) -> GeneralRecipeNode {
    key.node(
        bounds,
        GeneralRecipePayload::Group {
            isolation: true,
            phase,
        },
    )
}

/// Text in the region font 1 that does not wrap and adds no line spacing. A
/// paragraph sets `wrap` and `line_spacing` on the payload of the returned
/// node.
pub(super) fn text_node(
    key: NodeKey<'_>,
    bounds: Rect,
    source: TextSource,
    font_size: f32,
    color: [f32; 4],
    align: GeneralTextAlign,
) -> GeneralRecipeNode {
    key.node(
        bounds,
        GeneralRecipePayload::Text {
            source,
            font_role: FontRole::RegionFontId(1),
            font_size,
            color,
            align,
            line_spacing: 0.0,
            wrap: false,
            render_baseline: None,
        },
    )
}

/// Player-data image, sampled with nearest filtering. `descriptor` is the
/// resolved descriptor of `image`.
pub(super) fn image_node(
    key: NodeKey<'_>,
    bounds: Rect,
    image: &ComponentImageSnapshot,
    descriptor: &ResourceDescriptor,
    source: GeneralImageSource,
    source_constraint: GeneralImageSourceConstraint,
    clips: Vec<GeneralClip>,
) -> GeneralRecipeNode {
    GeneralRecipeNode {
        clips,
        ..key.node(
            bounds,
            GeneralRecipePayload::Image {
                resource: descriptor.resource.clone(),
                natural_size: Some([descriptor.natural_width, descriptor.natural_height]),
                source,
                sampling: GeneralImageSampling::Nearest,
                source_constraint,
                tint: [1.0; 4],
                source_field: Some(image.source_field.clone()),
                source_id: Some(image.source_id.clone()),
                provenance: descriptor.provenance.clone(),
                blend_mode: BlendMode::SrcOver,
            },
        )
    }
}

pub(super) fn image_node_without_source(
    key: NodeKey<'_>,
    bounds: Rect,
    resource: ResourceKey,
) -> GeneralRecipeNode {
    key.node(
        bounds,
        GeneralRecipePayload::Image {
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
    )
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

pub(super) fn item_region(
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

pub(super) fn layout_rect(layout: crate::profile_layout::ElementLayout) -> Rect {
    Rect {
        x: layout.cx - layout.w / 2.0,
        y: -layout.cy - layout.h / 2.0,
        width: layout.w,
        height: layout.h,
    }
}

pub(super) fn localized_source(
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

pub(super) fn localized_two_line_sources(
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

pub(super) fn count_source(
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
