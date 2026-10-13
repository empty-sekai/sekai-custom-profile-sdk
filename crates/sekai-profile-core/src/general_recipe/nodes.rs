//! Node constructors, item regions, layout rectangles and text sources the panel
//! builders share.

use super::{
    recipe_node_id, GeneralClip, GeneralFill, GeneralGeometry, GeneralGroupPhase,
    GeneralImageSampling, GeneralImageSource, GeneralImageSourceConstraint, GeneralRecipeNode,
    GeneralRecipePayload, GeneralTextAlign,
};
use crate::profile_scene::{ProfileComponentSnapshot, ProfileResolveError};
use crate::{
    BlendMode, FontRole, InteractionRegionSource, ParameterValue, Rect, ResourceKey, StableId,
    TextSource,
};

pub(super) fn shape_node(
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

pub(super) fn group_node(
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
pub(super) fn text_node(
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
pub(super) fn image_node(
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

pub(super) fn image_node_without_source(
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
