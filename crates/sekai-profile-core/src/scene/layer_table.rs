//! Depth-first layer order, subtree intervals, render-mask resolution and the quad
//! transforms a scene applies to interaction regions.

use std::collections::BTreeMap;

use crate::{
    LayerId, LayerSource, Matrix2d, Quad, Rect, RenderMaskOverride, SemanticCommandSource,
};

pub(super) fn transform_quad(matrix: Matrix2d, quad: Quad) -> Quad {
    quad.map(|[x, y]| {
        [
            matrix[0] * x + matrix[2] * y + matrix[4],
            matrix[1] * x + matrix[3] * y + matrix[5],
        ]
    })
}

pub(super) fn multiply_matrix(parent: Matrix2d, local: Matrix2d) -> Matrix2d {
    [
        parent[0] * local[0] + parent[2] * local[1],
        parent[1] * local[0] + parent[3] * local[1],
        parent[0] * local[2] + parent[2] * local[3],
        parent[1] * local[2] + parent[3] * local[3],
        parent[0] * local[4] + parent[2] * local[5] + parent[4],
        parent[1] * local[4] + parent[3] * local[5] + parent[5],
    ]
}

pub(super) fn quad_bounds(quad: Quad) -> Rect {
    let left = quad
        .iter()
        .map(|point| point[0])
        .fold(f32::INFINITY, f32::min);
    let top = quad
        .iter()
        .map(|point| point[1])
        .fold(f32::INFINITY, f32::min);
    let right = quad
        .iter()
        .map(|point| point[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = quad
        .iter()
        .map(|point| point[1])
        .fold(f32::NEG_INFINITY, f32::max);
    Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

pub(super) fn resolve_render_mask(
    mask_override: RenderMaskOverride,
    authored_visible: bool,
    parent_visible: bool,
) -> bool {
    let local = match mask_override {
        RenderMaskOverride::InheritAuthored => authored_visible,
        RenderMaskOverride::ForceVisible => true,
        RenderMaskOverride::ForceHidden => false,
    };
    local && parent_visible
}

pub(super) fn semantic_command_spans(
    order: &[LayerId],
    commands: &[SemanticCommandSource],
) -> BTreeMap<LayerId, (u32, u32)> {
    let mut spans = BTreeMap::new();
    let mut cursor = 0usize;
    for layer_id in order {
        let start = cursor;
        while cursor < commands.len() && commands[cursor].layer_id == *layer_id {
            cursor += 1;
        }
        spans.insert(*layer_id, (start as u32, (cursor - start) as u32));
    }
    spans
}

pub(super) fn build_dfs_layer_table(
    layers: &[LayerSource],
) -> (Vec<LayerId>, BTreeMap<LayerId, (usize, usize)>) {
    let mut children = BTreeMap::<Option<LayerId>, Vec<LayerId>>::new();
    for layer in layers {
        children.entry(layer.parent_id).or_default().push(layer.id);
    }
    let mut order = Vec::with_capacity(layers.len());
    let mut starts = BTreeMap::new();
    let mut intervals = BTreeMap::new();
    let mut stack = children
        .get(&None)
        .into_iter()
        .flatten()
        .rev()
        .map(|layer_id| (*layer_id, false))
        .collect::<Vec<_>>();
    while let Some((layer_id, exiting)) = stack.pop() {
        if exiting {
            intervals.insert(layer_id, (starts[&layer_id], order.len()));
            continue;
        }
        starts.insert(layer_id, order.len());
        order.push(layer_id);
        stack.push((layer_id, true));
        if let Some(layer_children) = children.get(&Some(layer_id)) {
            stack.extend(layer_children.iter().rev().map(|child| (*child, false)));
        }
    }
    (order, intervals)
}

pub(super) fn merge_intervals(mut intervals: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    intervals.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(intervals.len());
    for (start, end) in intervals {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}
