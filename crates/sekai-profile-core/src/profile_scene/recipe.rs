//! Lowering of general recipes to semantic commands: identity generals and
//! the card-member overlay.

use std::collections::BTreeMap;

use super::keys::interaction_region_id;
use super::layer::rect_quad;
use super::snapshot::{
    CardVisualSnapshot, ProfileComponentLowering, ProfileComponentSnapshot, ProfileResolveSnapshot,
};
use super::ProfileResolveError;
use crate::general_recipe::{
    GeneralClip, GeneralFill, GeneralGeometry, GeneralGroupPhase, GeneralImageSampling,
    GeneralImageSource, GeneralImageSourceConstraint, GeneralRecipeNode, GeneralRecipePayload,
    GeneralStroke, GeneralTextAlign,
};
use crate::{
    InteractionRegionSource, ParameterValue, Quad, Rect, SemanticCommandPayload,
    SemanticCommandSource, ShapePrimitive, StableId,
};

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
        .collect::<Result<Vec<_>, ProfileResolveError>>()?;
    let mut interaction_regions = recipe.interaction_regions.clone();
    interaction_regions.extend(
        recipe
            .nodes
            .iter()
            .zip(&commands)
            .filter_map(|(node, command)| match &node.payload {
                crate::general_recipe::GeneralRecipePayload::Text {
                    source: crate::TextSource::ProfileField { field, .. },
                    ..
                } => Some(profile_text_interaction_region(source_key, command, field)),
                _ => None,
            }),
    );
    Ok(Some(ProfileComponentLowering {
        commands,
        interaction_regions,
        controls: recipe.controls,
    }))
}

pub(super) fn general_recipe_node_to_command(
    node: &GeneralRecipeNode,
) -> Result<SemanticCommandSource, ProfileResolveError> {
    match &node.payload {
        GeneralRecipePayload::Shape {
            geometry,
            fill,
            stroke,
        } => Ok(shape_node_command(node, geometry, fill, stroke.as_ref())),
        GeneralRecipePayload::Text { .. } => Ok(text_node_command(node)),
        GeneralRecipePayload::Image { .. } => image_node_command(node),
        GeneralRecipePayload::Group { phase, .. } => Ok(group_node_command(node, *phase)),
    }
}

/// Shape command of a recipe shape node, filled and stroked as the recipe
/// paints it.
fn shape_node_command(
    node: &GeneralRecipeNode,
    geometry: &GeneralGeometry,
    fill: &GeneralFill,
    stroke: Option<&GeneralStroke>,
) -> SemanticCommandSource {
    let primitive = match geometry {
        GeneralGeometry::Rect => ShapePrimitive::Rect,
        GeneralGeometry::RoundedRect { radius } => ShapePrimitive::RoundedRect { radius: *radius },
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
    command.clip = rect_clip(&node.clips);
    command
}

/// Text command of a recipe text node, placed at the center of the node
/// bounds.
fn text_node_command(node: &GeneralRecipeNode) -> SemanticCommandSource {
    let GeneralRecipePayload::Text {
        source,
        font_role,
        font_size,
        color,
        align,
        line_spacing,
        wrap,
        render_baseline,
    } = &node.payload
    else {
        unreachable!("text node lowering received a non-text node");
    };
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
    command.clip = rect_clip(&node.clips);
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

/// Image command of a recipe image node. Its metadata records where the image
/// comes from and how the backend samples it.
fn image_node_command(
    node: &GeneralRecipeNode,
) -> Result<SemanticCommandSource, ProfileResolveError> {
    let GeneralRecipePayload::Image {
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
    } = &node.payload
    else {
        unreachable!("image node lowering received a non-image node");
    };
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
    command.clip = rect_clip(&node.clips);
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
        *uv = image_source_uv(node, source, *natural_size)?;
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
        if let Some(intrinsic) = intrinsic_image_clip(&node.clips) {
            *clip = Some(intrinsic);
        }
    }
    Ok(command)
}

/// Texture rectangle of an image source, normalized to the image size.
fn image_source_uv(
    node: &GeneralRecipeNode,
    source: &GeneralImageSource,
    natural_size: Option<[f32; 2]>,
) -> Result<Rect, ProfileResolveError> {
    Ok(match source {
        GeneralImageSource::WholeImageImplicit | GeneralImageSource::WholeImageExplicit => Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        GeneralImageSource::Pixels(rect) => {
            // Recipes are serializable; a restored payload can carry
            // `pixels` without natural_size. That is a contract
            // violation and must fail explicitly, not panic.
            let [width, height] = natural_size.ok_or_else(|| {
                ProfileResolveError::InvalidRecipe(format!(
                    "pixel image source {:?} requires resolved natural dimensions",
                    node.id
                ))
            })?;
            Rect {
                x: rect.x / width.max(f32::EPSILON),
                y: rect.y / height.max(f32::EPSILON),
                width: rect.width / width.max(f32::EPSILON),
                height: rect.height / height.max(f32::EPSILON),
            }
        }
    })
}

/// Image clip of the first ellipse or rounded-rectangle clip of a node.
fn intrinsic_image_clip(clips: &[GeneralClip]) -> Option<crate::ImageClip> {
    clips.iter().find_map(|clip| match clip {
        GeneralClip::Ellipse { .. } => Some(crate::ImageClip::Ellipse),
        GeneralClip::RoundedRect { radius, .. } => {
            Some(crate::ImageClip::RoundedRect { radius: *radius })
        }
        GeneralClip::Rect { .. } => None,
    })
}

/// Clip quad of the first rectangle clip of a node.
fn rect_clip(clips: &[GeneralClip]) -> Option<Quad> {
    clips.iter().find_map(|clip| match clip {
        GeneralClip::Rect { bounds, .. } => Some(rect_quad(*bounds)),
        GeneralClip::RoundedRect { .. } | GeneralClip::Ellipse { .. } => None,
    })
}

/// Composite command that opens or closes the isolated group of a recipe
/// group node.
fn group_node_command(node: &GeneralRecipeNode, phase: GeneralGroupPhase) -> SemanticCommandSource {
    let mut command =
        SemanticCommandSource::composite(node.id, node.layer_id, node.role.clone(), node.bounds);
    command.matrix = node.local_matrix;
    command.control_bindings = node.control_bindings.clone();
    if let SemanticCommandPayload::Composite { operation, .. } = &mut command.payload {
        *operation = match phase {
            GeneralGroupPhase::Begin => crate::CompositeOperation::BeginIsolation,
            GeneralGroupPhase::End => crate::CompositeOperation::EndIsolation,
        };
    }
    command
}

/// Selection region for a profile-field text command. It shares the command's
/// tab and scroll bindings and its viewport clip, so it is hit-testable exactly
/// where and when the text is drawn.
fn profile_text_interaction_region(
    source_key: &str,
    command: &SemanticCommandSource,
    field: &str,
) -> InteractionRegionSource {
    InteractionRegionSource {
        id: interaction_region_id(source_key, &command.role, 0),
        layer_id: command.layer_id,
        role: command.role.clone(),
        bounds: command.bounds,
        quad: rect_quad(command.bounds),
        matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        hit_geometry: rect_quad(command.bounds),
        clip: command.clip,
        control_bindings: command.control_bindings.clone(),
        resolved_data: BTreeMap::from([("field".into(), ParameterValue::Text(field.into()))]),
        capabilities: vec!["inspect".into(), "select_text".into(), "edit_text".into()],
    }
}

/// Level caption of an authored card-member overlay, in the profile's
/// localized card-level wording. Without player data there is no locale to
/// follow, and the caption uses the game's Japanese wording.
pub(super) fn card_member_level_source(
    snapshot: &ProfileResolveSnapshot,
    visual: &CardVisualSnapshot,
) -> crate::TextSource {
    let key = crate::general_recipe::CARD_LEVEL_LOCALIZATION_KEY;
    let (locale, template) = snapshot
        .component
        .as_ref()
        .and_then(|component| {
            component
                .localized_text
                .get(key)
                .map(|template| (component.locale.clone(), template.clone()))
        })
        .unwrap_or_else(|| {
            (
                "ja-JP".into(),
                crate::locale::resolve("jp", key).unwrap_or_else(|| "{level}".into()),
            )
        });
    crate::general_recipe::card_level_source(&locale, &template, visual.level)
}
