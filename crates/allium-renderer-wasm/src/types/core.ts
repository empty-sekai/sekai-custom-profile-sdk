export type StableId = string;

export type CoreControlBinding =
  | { kind: "tab_option"; control_id: StableId; value: string }
  | { kind: "scroll_content"; control_id: StableId }
  | { kind: "scroll_thumb"; control_id: StableId }
  | { kind: "scroll_viewport"; control_id: StableId };

export type CoreLayerPatch = {
  layer_id: StableId;
  render_mask: boolean | null;
  transform: { dx: number; dy: number } | null;
};

export type CoreCommandPatch = {
  command_id: StableId;
  slot: number;
  render_mask: boolean | null;
  transform: { dx: number; dy: number } | null;
};

export type CoreSceneDelta = {
  schema_major: number;
  schema_minor: number;
  scene_id: StableId;
  base_revision: number;
  revision: number;
  tick: number;
  dirty: {
    mask: boolean;
    transform: boolean;
    material: boolean;
    layout: boolean;
    command: boolean;
    atlas: boolean;
    control: boolean;
  };
  patches: CoreLayerPatch[];
  command_patches: CoreCommandPatch[];
  error?: string;
};

export type CoreInteractionRegion = {
  id: StableId;
  layer_id: StableId;
  role: string;
  bounds: { x: number; y: number; width: number; height: number };
  quad: Array<[number, number]>;
  matrix: [number, number, number, number, number, number];
  hit_geometry: Array<[number, number]>;
  clip: Array<[number, number]> | null;
  control_bindings: CoreControlBinding[];
  resolved_data: Record<string, unknown>;
  capabilities: string[];
  render_mask: boolean;
};

export type CoreLayerTableEntry = {
  layer_id: StableId;
  parent_id: StableId | null;
  slot: number;
  subtree_start: number;
  subtree_end: number;
};

export type CoreSceneDump = {
  schema: "allium.scene-dump";
  schema_major: number;
  schema_minor: number;
  coordinate_space: "card-device-v1";
  scene_id: StableId;
  tick: number;
  revisions: Record<string, number>;
  layer_table: CoreLayerTableEntry[];
  layer_tree: Array<[StableId, StableId | null]>;
  layers: Array<Record<string, unknown>>;
  interaction_regions: CoreInteractionRegion[];
  component_controls: Array<Record<string, unknown>>;
  command_states: Array<Record<string, unknown>>;
  telemetry: Record<string, number>;
  error?: string;
};

export type CoreSceneCreateResponse = {
  handle: number;
  layer_bindings: Array<{ source_key: string; layer_id: StableId }>;
  snapshot: {
    schema_major: number;
    schema_minor: number;
    scene_id: StableId;
    revisions: Record<string, number>;
    layer_table: CoreLayerTableEntry[];
    layer_sources: Array<{
      id: StableId;
      matrix: [number, number, number, number, number, number];
    } & Record<string, unknown>>;
    commands: Array<{
      layer_id: StableId;
      render_mask: boolean;
      transform: { dx: number; dy: number };
      command_start: number;
      command_count: number;
    }>;
    semantic_commands: Array<Record<string, unknown>>;
    interaction_regions: CoreInteractionRegion[];
    component_controls: Array<Record<string, unknown>>;
    command_states: Array<Record<string, unknown>>;
  };
  error?: string;
};
