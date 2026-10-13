//! Records a scene keeps and reports: revisions, layer and command state, patches
//! and deltas, snapshots, dumps, the animation preflight and `CoreError`.

use std::collections::BTreeMap;

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    profile_scene, AuthoredElementKind, CommandControlBinding, CommandId, DynamicLayerState,
    GlyphId, GlyphSource, LayerId, LayerKind, LayerSource, LineIndentSource, Matrix2d,
    ParameterValue, Quad, Rect, SceneId, SemanticCommandSource, StableId, TransformDelta,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct InteractionRegionSnapshot {
    pub id: StableId,
    pub layer_id: StableId,
    pub role: String,
    pub bounds: Rect,
    pub quad: Quad,
    pub matrix: Matrix2d,
    pub hit_geometry: Quad,
    pub clip: Option<Quad>,
    #[serde(default)]
    pub control_bindings: Vec<CommandControlBinding>,
    pub resolved_data: BTreeMap<String, ParameterValue>,
    pub capabilities: Vec<String>,
    pub render_mask: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnimationPreflight {
    pub sampled_through_tick: u64,
    pub compiled_program_count: u32,
    pub observable_program_count: u32,
    pub observable_layer_ids: Vec<LayerId>,
    pub animated: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct Revisions {
    pub scene: u64,
    pub source: u64,
    pub resolve: u64,
    pub layer_table: u64,
    pub tree: u64,
    pub order: u64,
    pub local_layout: u64,
    pub command: u64,
    pub material: u64,
    pub transform: u64,
    pub resource: u64,
    pub atlas: u64,
    pub timeline: u64,
    pub dynamic_state: u64,
    pub mask: u64,
    pub snapshot: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerState {
    pub mask_override: RenderMaskOverride,
    pub render_mask: bool,
    pub dynamic: Option<DynamicLayerState>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderMaskOverride {
    #[default]
    InheritAuthored,
    ForceVisible,
    ForceHidden,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerPatch {
    pub layer_id: LayerId,
    pub render_mask: Option<bool>,
    pub transform: Option<TransformDelta>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct CommandState {
    pub command_id: CommandId,
    pub slot: u32,
    pub render_mask: bool,
    pub transform: TransformDelta,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandPatch {
    pub command_id: CommandId,
    pub slot: u32,
    pub render_mask: Option<bool>,
    pub transform: Option<TransformDelta>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DirtyKinds {
    pub mask: bool,
    pub transform: bool,
    pub material: bool,
    pub layout: bool,
    pub command: bool,
    pub atlas: bool,
    pub control: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneDelta {
    pub schema_major: u16,
    pub schema_minor: u16,
    pub scene_id: SceneId,
    pub base_revision: u64,
    pub revision: u64,
    pub tick: u64,
    pub dirty: DirtyKinds,
    pub patches: Vec<LayerPatch>,
    pub command_patches: Vec<CommandPatch>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct LayerCommand {
    pub command_id: CommandId,
    pub layer_id: LayerId,
    pub render_mask: bool,
    pub transform: TransformDelta,
    pub command_start: u32,
    pub command_count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct GlyphCommand {
    pub command_id: CommandId,
    pub glyph_id: GlyphId,
    pub layer_id: LayerId,
    pub bounds: Rect,
    pub quad: Quad,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct CoreTelemetry {
    pub dynamic_evaluations: u64,
    pub dirty_layers: u32,
    pub layout_runs: u64,
    pub command_rebuilds: u64,
    pub atlas_generations: u64,
    pub serialized_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct SceneSnapshot {
    pub schema_major: u16,
    pub schema_minor: u16,
    pub scene_id: SceneId,
    pub tick: u64,
    pub revisions: Revisions,
    pub layer_table: Vec<LayerTableEntry>,
    pub layer_sources: Vec<LayerSource>,
    pub layer_tree: Vec<(LayerId, Option<LayerId>)>,
    pub commands: Vec<LayerCommand>,
    pub semantic_commands: Vec<SemanticCommandSource>,
    pub component_controls: Vec<profile_scene::ComponentControlSource>,
    pub command_states: Vec<CommandState>,
    pub interaction_regions: Vec<InteractionRegionSnapshot>,
    pub glyph_commands: Vec<GlyphCommand>,
    pub telemetry: CoreTelemetry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct LayerTableEntry {
    pub layer_id: LayerId,
    pub parent_id: Option<LayerId>,
    pub slot: u32,
    pub subtree_start: u32,
    pub subtree_end: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerDump {
    pub layer_id: LayerId,
    pub parent_id: Option<LayerId>,
    pub kind: LayerKind,
    pub authored_kind: AuthoredElementKind,
    pub authored_index: u32,
    pub game_layer: i32,
    pub source_content: String,
    pub resolved_parameters: BTreeMap<String, ParameterValue>,
    pub bounds: Rect,
    pub quad: Quad,
    pub matrix: Matrix2d,
    pub hit_geometry: Quad,
    pub authored_visible: bool,
    pub mask_override: RenderMaskOverride,
    pub render_mask: bool,
    pub dynamic: Option<DynamicLayerState>,
    pub line_indent: Option<LineIndentSource>,
    pub glyphs: Vec<GlyphSource>,
    pub commands: Vec<SemanticCommandSource>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneDump {
    pub schema: String,
    pub schema_major: u16,
    pub schema_minor: u16,
    pub coordinate_space: String,
    pub scene_id: SceneId,
    pub tick: u64,
    pub revisions: Revisions,
    pub layer_table: Vec<LayerTableEntry>,
    pub layer_tree: Vec<(LayerId, Option<LayerId>)>,
    pub layers: Vec<LayerDump>,
    pub interaction_regions: Vec<InteractionRegionSnapshot>,
    pub component_controls: Vec<profile_scene::ComponentControlSource>,
    pub command_states: Vec<CommandState>,
    pub telemetry: CoreTelemetry,
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("duplicate layer id {0:?}")]
    DuplicateLayer(LayerId),
    #[error("duplicate glyph id {0:?}")]
    DuplicateGlyph(GlyphId),
    #[error("unknown layer id {0:?}")]
    UnknownLayer(LayerId),
    #[error("layer {layer:?} references unknown parent {parent:?}")]
    UnknownParent { layer: LayerId, parent: LayerId },
    #[error("layer tree contains a cycle at {0:?}")]
    LayerTreeCycle(LayerId),
    #[error("unsupported schema major {actual}, expected {expected}")]
    IncompatibleSchema { actual: u16, expected: u16 },
    #[error("stale layer table revision {actual}, expected {expected}")]
    StaleLayerTableRevision { actual: u64, expected: u64 },
    #[error("semantic command {command_id:?} references unknown layer {layer_id:?}")]
    UnknownCommandLayer {
        command_id: CommandId,
        layer_id: LayerId,
    },
    #[error("duplicate semantic command {0:?}")]
    DuplicateCommand(CommandId),
    #[error("interaction region {region_id:?} references unknown layer {layer_id:?}")]
    UnknownInteractionLayer {
        region_id: StableId,
        layer_id: LayerId,
    },
    #[error("duplicate interaction region {0:?}")]
    DuplicateInteractionRegion(StableId),
    #[error("duplicate component control {0:?}")]
    DuplicateComponentControl(StableId),
    #[error("unknown component control {0:?}")]
    UnknownComponentControl(StableId),
    #[error("invalid component control {control_id:?}: {reason}")]
    InvalidComponentControl {
        control_id: StableId,
        reason: String,
    },
    #[error("binary codec failed: {0}")]
    Codec(String),
}
