//! Builders of the image, composite and text commands of component lowering.

use super::keys::semantic_command_id;
use super::layer::{layout_bounds, rect_quad};
use super::snapshot::ResourceDescriptor;
use crate::profile_layout::ElementLayout;
use crate::{
    CompositeOperation, FontRole, Rect, SemanticCommandPayload, SemanticCommandSource, StableId,
};

/// How a component text is set: its alignment flags (1 = left, 2 = center,
/// 4 = right), its size and its colour.
#[derive(Clone, Copy)]
pub(super) struct TextStyle {
    pub(super) alignment: u8,
    pub(super) size: f32,
    pub(super) color: [f32; 4],
}

/// The commands of one element layer, appended in draw order. Each command id
/// derives from the element's source key, the command role and the number of
/// commands appended before it.
pub(super) struct LayerCommands<'a> {
    source_key: &'a str,
    layer_id: StableId,
    ordinal: u32,
    commands: &'a mut Vec<SemanticCommandSource>,
}

impl<'a> LayerCommands<'a> {
    pub(super) fn new(
        source_key: &'a str,
        layer_id: StableId,
        commands: &'a mut Vec<SemanticCommandSource>,
    ) -> Self {
        Self {
            source_key,
            layer_id,
            ordinal: 0,
            commands,
        }
    }

    fn next_id(&mut self, role: &str) -> StableId {
        let id = semantic_command_id(self.source_key, role, self.ordinal);
        self.ordinal += 1;
        id
    }

    fn push(&mut self, command: SemanticCommandSource) -> &mut SemanticCommandSource {
        self.commands.push(command);
        let last = self.commands.len() - 1;
        &mut self.commands[last]
    }

    /// Appends an image command that draws `descriptor` into `bounds`,
    /// sampling the `uv` rectangle of the image, or all of it when `uv` is
    /// `None`. Returns the appended command.
    pub(super) fn push_image(
        &mut self,
        role: &str,
        descriptor: &ResourceDescriptor,
        bounds: Rect,
        uv: Option<Rect>,
    ) -> &mut SemanticCommandSource {
        let mut command = SemanticCommandSource::image(
            self.next_id(role),
            self.layer_id,
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
        if let (Some(uv), SemanticCommandPayload::Image { uv: target_uv, .. }) =
            (uv, &mut command.payload)
        {
            *target_uv = uv;
        }
        self.push(command)
    }

    /// Appends a composite command that applies `operation` over `bounds`.
    pub(super) fn push_composite(
        &mut self,
        role: &str,
        bounds: Rect,
        operation: CompositeOperation,
    ) {
        let mut command =
            SemanticCommandSource::composite(self.next_id(role), self.layer_id, role, bounds);
        if let SemanticCommandPayload::Composite {
            operation: target, ..
        } = &mut command.payload
        {
            *target = operation;
        }
        self.push(command);
    }

    /// Appends a text command showing `value`, the value of the profile field
    /// `field`, laid out in `layout` with the region font 1.
    pub(super) fn push_profile_text(
        &mut self,
        role: &str,
        field: &str,
        value: String,
        layout: ElementLayout,
        style: TextStyle,
    ) {
        let mut command = SemanticCommandSource::profile_text(
            self.next_id(role),
            self.layer_id,
            role,
            field,
            value,
            FontRole::RegionFontId(1),
        );
        configure_text_command(&mut command, layout, style.alignment, style.size, 0.0);
        set_text_color(&mut command, style.color);
        self.push(command);
    }
}

fn configure_text_command(
    command: &mut SemanticCommandSource,
    layout: ElementLayout,
    alignment: u8,
    size: f32,
    line_spacing: f32,
) {
    command.bounds = layout_bounds(layout);
    // Alignment is the low bits of the authored TextAlignmentOptions value:
    // a bitmask where 1 = left, 2 = center, 4 = right.
    command.matrix[4] = match alignment {
        2 => layout.cx,
        4 => layout.cx + layout.w / 2.0,
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
