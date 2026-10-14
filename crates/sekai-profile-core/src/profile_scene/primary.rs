//! Lowering of an element's primary command from its resolved resources.

use std::collections::BTreeMap;

use super::keys::{card_member_lookup_key, collection_normal_map_lookup_key, resource_lookup_key};
use super::snapshot::{ProfileElementRef, ProfileResolveSnapshot, ResourceDescriptor};
use super::ProfileResolveError;
use crate::profile_source::{
    CardMemberElement, CollectionElement, ShapeElement, TextElement, UserInterfaceIconElement,
};
use crate::{
    FontRole, ImageMaterial, LayerKind, ParameterValue, Rect, SemanticCommandPayload,
    SemanticCommandSource, ShapePrimitive, StableId,
};

/// Layer kind, layer source content and primary command of one element layer.
type LoweredPrimary = (LayerKind, String, SemanticCommandSource);

/// What the primary command of one element is built from: the resolved
/// snapshot, the element's layer and the id of the command.
#[derive(Clone, Copy)]
pub(super) struct PrimaryContext<'a> {
    pub(super) snapshot: &'a ProfileResolveSnapshot,
    pub(super) layer_id: StableId,
    pub(super) command_id: StableId,
}

pub(super) fn lower_primary_command(
    element: ProfileElementRef<'_>,
    context: PrimaryContext<'_>,
    parameters: &mut BTreeMap<String, ParameterValue>,
) -> Result<LoweredPrimary, ProfileResolveError> {
    Ok(match element {
        ProfileElementRef::Text(text) => text_primary(text, context, parameters)?,
        ProfileElementRef::Shape(shape) => shape_primary(shape, context, parameters)?,
        ProfileElementRef::CardMember(value) => card_member_primary(value, context, parameters)?,
        ProfileElementRef::Stamp(value) => {
            resolved_image_with_parameter(context, "stamp", value.id, "", "stamp", parameters)?
        }
        ProfileElementRef::Other(value) => {
            resolved_image_with_parameter(context, "other", value.id, "", "other", parameters)?
        }
        ProfileElementRef::Collection(value) => collection_primary(value, context, parameters)?,
        ProfileElementRef::StandMember(value) => resolved_image_with_parameter(
            context,
            "stand-member",
            value.id,
            "",
            "stand-member",
            parameters,
        )?,
        ProfileElementRef::GeneralBackground(value) => resolved_image_with_parameter(
            context,
            "general-background",
            value.id,
            "",
            "general-background",
            parameters,
        )?,
        ProfileElementRef::StoryBackground(value) => resolved_image_with_parameter(
            context,
            "story-background",
            value.id,
            "",
            "story-background",
            parameters,
        )?,
        ProfileElementRef::CharacterIcon(value) => resolved_image_with_parameter(
            context,
            "character-icon",
            value.id,
            "",
            "character-icon",
            parameters,
        )?,
        ProfileElementRef::Material(value) => resolved_image_with_parameter(
            context, "material", value.id, "", "material", parameters,
        )?,
        ProfileElementRef::UserInterfaceIcon(value) => {
            user_interface_icon_primary(value, context, parameters)?
        }
        ProfileElementRef::BondsHonor(value) => {
            composite_primary(context, "bonds-honor", value.id, parameters)
        }
        ProfileElementRef::Honor(value) => {
            composite_primary(context, "honor", value.id, parameters)
        }
        ProfileElementRef::General(value) => {
            let general_type = value.general_type.unwrap_or_default();
            parameters.insert(
                "general_type".into(),
                ParameterValue::I64(general_type.into()),
            );
            composite_primary(context, "general", general_type, parameters)
        }
    })
}

fn text_primary(
    text: &TextElement,
    context: PrimaryContext<'_>,
    parameters: &mut BTreeMap<String, ParameterValue>,
) -> Result<LoweredPrimary, ProfileResolveError> {
    let PrimaryContext {
        snapshot,
        layer_id,
        command_id,
    } = context;
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
        // A missing horizontal flag uses the text layout's left default.
        *alignment = match text.text_type & 0x07 {
            0 => 1,
            value => value as u8,
        };
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
    Ok((LayerKind::Text, text.text.clone(), command))
}

fn shape_primary(
    shape: &ShapeElement,
    context: PrimaryContext<'_>,
    parameters: &mut BTreeMap<String, ParameterValue>,
) -> Result<LoweredPrimary, ProfileResolveError> {
    let PrimaryContext {
        snapshot,
        layer_id,
        command_id,
    } = context;
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
    Ok((LayerKind::Shape, String::new(), command))
}

/// Card artwork of a card-member element. A member of type 1 shows the
/// top-center crop of the artwork at its native scale.
fn card_member_primary(
    value: &CardMemberElement,
    context: PrimaryContext<'_>,
    parameters: &mut BTreeMap<String, ParameterValue>,
) -> Result<LoweredPrimary, ProfileResolveError> {
    let PrimaryContext {
        snapshot,
        layer_id,
        command_id,
    } = context;
    parameters.insert("resource_id".into(), ParameterValue::I64(value.id.into()));
    parameters.insert(
        "show_master_rank".into(),
        ParameterValue::Bool(value.show_master_rank.unwrap_or(false)),
    );
    let member_type = value.member_type.unwrap_or(2);
    let lookup_key = card_member_lookup_key(value);
    let descriptor = snapshot
        .resources
        .get(&lookup_key)
        .ok_or_else(|| ProfileResolveError::MissingResource(lookup_key.clone()))?;
    record_resource_parameters(parameters, descriptor);
    let mut resolved = (
        LayerKind::Image,
        String::new(),
        SemanticCommandSource::image(
            command_id,
            layer_id,
            "card-member",
            descriptor.resource.clone(),
            descriptor.centered_bounds(),
        ),
    );
    if member_type == 1 {
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
    Ok(resolved)
}

/// Collection image, drawn with the lit can-badge material when the
/// collection has a normal map.
fn collection_primary(
    value: &CollectionElement,
    context: PrimaryContext<'_>,
    parameters: &mut BTreeMap<String, ParameterValue>,
) -> Result<LoweredPrimary, ProfileResolveError> {
    let mut resolved = resolved_image_with_parameter(
        context,
        "collection",
        value.id,
        "",
        "collection",
        parameters,
    )?;
    if let (Some(normal_map), SemanticCommandPayload::Image { material, .. }) = (
        context
            .snapshot
            .resources
            .get(&collection_normal_map_lookup_key(value.id)),
        &mut resolved.2.payload,
    ) {
        *material = ImageMaterial::LitBadge {
            normal_map: normal_map.resource.clone(),
        };
    }
    Ok(resolved)
}

fn user_interface_icon_primary(
    value: &UserInterfaceIconElement,
    context: PrimaryContext<'_>,
    parameters: &mut BTreeMap<String, ParameterValue>,
) -> Result<LoweredPrimary, ProfileResolveError> {
    let mut resolved = resolved_image_with_parameter(
        context,
        "user-interface-icon",
        value.id,
        "",
        "user-interface-icon",
        parameters,
    )?;
    if let SemanticCommandPayload::Image { tint, .. } = &mut resolved.2.payload {
        // The icon is tinted like a shape face: the colour row with the
        // element alpha, reaching the image as an 8-bit vertex colour.
        *tint = with_alpha(
            context
                .snapshot
                .colors
                .get(&value.color_id)
                .copied()
                .unwrap_or([1.0; 4]),
            value.alpha,
        )
        .map(crate::sdf_material::color32_unit);
    }
    Ok(resolved)
}

/// Image command of the resource `kind`/`id`/`variant`, drawn at its natural
/// size centered on the layer origin.
fn resolved_image(
    context: PrimaryContext<'_>,
    kind: &str,
    id: i32,
    variant: &str,
    role: &str,
    parameters: &mut BTreeMap<String, ParameterValue>,
) -> Result<LoweredPrimary, ProfileResolveError> {
    let lookup_key = resource_lookup_key(kind, id, variant);
    let descriptor = context
        .snapshot
        .resources
        .get(&lookup_key)
        .ok_or_else(|| ProfileResolveError::MissingResource(lookup_key.clone()))?;
    record_resource_parameters(parameters, descriptor);
    Ok((
        LayerKind::Image,
        String::new(),
        SemanticCommandSource::image(
            context.command_id,
            context.layer_id,
            role,
            descriptor.resource.clone(),
            descriptor.centered_bounds(),
        ),
    ))
}

/// [`resolved_image`], recording `id` as the layer's resource id.
fn resolved_image_with_parameter(
    context: PrimaryContext<'_>,
    kind: &str,
    id: i32,
    variant: &str,
    role: &str,
    parameters: &mut BTreeMap<String, ParameterValue>,
) -> Result<LoweredPrimary, ProfileResolveError> {
    parameters.insert("resource_id".into(), ParameterValue::I64(id.into()));
    resolved_image(context, kind, id, variant, role, parameters)
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

/// Empty composite command of an element whose content is drawn by other
/// commands, recording `id` as the layer's resource id.
pub(super) fn composite_primary(
    context: PrimaryContext<'_>,
    role: &str,
    id: i32,
    parameters: &mut BTreeMap<String, ParameterValue>,
) -> LoweredPrimary {
    parameters.insert("resource_id".into(), ParameterValue::I64(id.into()));
    (
        LayerKind::Composite,
        String::new(),
        SemanticCommandSource::composite(
            context.command_id,
            context.layer_id,
            role,
            Rect::default(),
        ),
    )
}

fn with_alpha(mut color: [f32; 4], alpha: f32) -> [f32; 4] {
    color[3] = alpha.clamp(0.0, 1.0);
    color
}
