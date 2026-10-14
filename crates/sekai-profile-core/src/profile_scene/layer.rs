//! Layer assembly and the geometry of layers and interaction regions.

use std::collections::BTreeMap;

use super::keys::interaction_region_id;
use super::snapshot::NormalizedProfileElement;
use crate::profile_layout::ElementLayout;
use crate::{
    InteractionRegionSource, LayerKind, LayerSource, LineIndentSource, ParameterValue, Quad, Rect,
    SemanticCommandSource, StableId,
};

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
    // Geometry is layer-local; the layer matrix places it on the canvas. A
    // layer without drawn area collapses to its own origin.
    let hit_geometry = if bounds.width > 0.0 && bounds.height > 0.0 {
        rect_quad(bounds)
    } else {
        [[0.0; 2]; 4]
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

pub(super) fn component_interaction_region(
    source_key: &str,
    layer_id: StableId,
    role: &str,
    ordinal: u32,
    layout: ElementLayout,
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

pub(super) fn layout_bounds(layout: ElementLayout) -> Rect {
    Rect {
        x: layout.cx - layout.w / 2.0,
        y: -layout.cy - layout.h / 2.0,
        width: layout.w,
        height: layout.h,
    }
}
