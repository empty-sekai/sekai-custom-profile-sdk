export type SemanticResourceKey = { namespace: string; key: string };

export type SemanticCommand = {
  id: string;
  layer_id: string;
  role: string;
  payload: Record<string, unknown> & { kind: string };
  [key: string]: unknown;
};

export type SemanticLayerPatch = {
  layer_id: string;
  render_mask: boolean | null;
  transform: { dx: number; dy: number } | null;
};

export type SemanticCommandStatePatch = {
  slot: number;
  render_mask: boolean | null;
  transform: { dx: number; dy: number } | null;
};

export type SemanticCommandPlanInput = {
  layerTableRevision: number;
  layerTable: Array<{
    layer_id: string;
    parent_id: string | null;
    slot: number;
    subtree_start: number;
    subtree_end: number;
  }>;
  layerSources: Array<{
    layer_id: string;
    matrix: [number, number, number, number, number, number];
  }>;
  layerCommands: Array<{
    layer_id: string;
    render_mask: boolean;
    transform: { dx: number; dy: number };
    command_start: number;
    command_count: number;
  }>;
  semanticCommands: SemanticCommand[];
  commandStates?: Array<{
    command_id: string;
    slot: number;
    render_mask: boolean;
    transform: { dx: number; dy: number };
  }>;
};

export type SemanticDrawOperation = {
  command: SemanticCommand;
  layerId: string;
  layerSlot: number;
  baseMatrix: [number, number, number, number, number, number];
  visible: boolean;
  transform: { dx: number; dy: number };
  commandSlot: number;
  commandVisible: boolean;
  commandTransform: { dx: number; dy: number };
};

type LayerRuntimeState = {
  slot: number;
  baseMatrix: [number, number, number, number, number, number];
  visible: boolean;
  transform: { dx: number; dy: number };
};

/**
 * Immutable authored-layer command ownership plus mutable render state.
 * Visibility and dynamic deltas update only the dense state table; command
 * order and resource identity remain stable for the plan lifetime.
 */
export class SemanticCommandPlan {
  readonly layerTableRevision: number;
  readonly commandRevision = 1;
  readonly resourceRevision = 1;
  private readonly commands: SemanticCommand[];
  private readonly operationLayers: Array<{ layerId: string; slot: number }>;
  private readonly stateByLayer = new Map<string, LayerRuntimeState>();
  private readonly resources: SemanticResourceKey[];
  private readonly commandVisible: boolean[];
  private readonly commandTransforms: Array<{ dx: number; dy: number }>;

  constructor(input: SemanticCommandPlanInput) {
    this.layerTableRevision = input.layerTableRevision;
    this.commands = [...input.semanticCommands];
    const stateBySlot = input.commandStates ?? this.commands.map((command, slot) => ({
      command_id: command.id,
      slot,
      render_mask: true,
      transform: { dx: 0, dy: 0 },
    }));
    if (stateBySlot.length !== this.commands.length) throw new Error("command state table length mismatch");
    this.commandVisible = new Array(this.commands.length);
    this.commandTransforms = new Array(this.commands.length);
    for (const state of stateBySlot) {
      if (!Number.isInteger(state.slot) || state.slot < 0 || state.slot >= this.commands.length || this.commands[state.slot].id !== state.command_id) {
        throw new Error(`command state identity mismatch at ${state.slot}`);
      }
      this.commandVisible[state.slot] = state.render_mask;
      this.commandTransforms[state.slot] = { ...state.transform };
    }
    const tableByLayer = new Map<string, number>();
    const matrixByLayer = new Map(input.layerSources.map((layer) => [layer.layer_id, layer.matrix] as const));
    for (const entry of input.layerTable) {
      if (tableByLayer.has(entry.layer_id)) throw new Error(`duplicate layer table id ${entry.layer_id}`);
      if (!Number.isInteger(entry.slot) || entry.slot < 0) throw new Error(`invalid layer slot ${entry.slot}`);
      tableByLayer.set(entry.layer_id, entry.slot);
    }
    const coverage = new Uint8Array(this.commands.length);
    this.operationLayers = new Array(this.commands.length);
    for (const layerCommand of input.layerCommands) {
      const slot = tableByLayer.get(layerCommand.layer_id);
      if (slot == null) throw new Error(`layer command references unknown layer ${layerCommand.layer_id}`);
      if (this.stateByLayer.has(layerCommand.layer_id)) {
        throw new Error(`duplicate layer command ${layerCommand.layer_id}`);
      }
      const baseMatrix = matrixByLayer.get(layerCommand.layer_id);
      if (!baseMatrix) throw new Error(`missing authored layer matrix ${layerCommand.layer_id}`);
      const start = layerCommand.command_start;
      const end = start + layerCommand.command_count;
      if (!Number.isInteger(start) || !Number.isInteger(end) || start < 0 || end > this.commands.length) {
        throw new Error(`invalid command span for layer ${layerCommand.layer_id}`);
      }
      for (let index = start; index < end; index += 1) {
        const command = this.commands[index];
        if (coverage[index] !== 0 || command.layer_id !== layerCommand.layer_id) {
          throw new Error(`command span ownership mismatch at ${index}`);
        }
        coverage[index] = 1;
        this.operationLayers[index] = { layerId: layerCommand.layer_id, slot };
      }
      this.stateByLayer.set(layerCommand.layer_id, {
        slot,
        baseMatrix: [...baseMatrix] as LayerRuntimeState["baseMatrix"],
        visible: layerCommand.render_mask,
        transform: { ...layerCommand.transform },
      });
    }
    const uncovered = coverage.findIndex((value) => value === 0);
    if (uncovered >= 0) throw new Error(`semantic command ${uncovered} is outside every authored layer span`);
    this.resources = collectResourceRequests(this.commands);
  }

  operations(): SemanticDrawOperation[] {
    return this.commands.map((command, index) => {
      const ownership = this.operationLayers[index];
      const state = this.stateByLayer.get(ownership.layerId);
      if (!state) throw new Error(`missing runtime layer state ${ownership.layerId}`);
      return {
        command,
        layerId: ownership.layerId,
        layerSlot: ownership.slot,
        baseMatrix: [...state.baseMatrix] as SemanticDrawOperation["baseMatrix"],
        visible: state.visible,
        transform: { ...state.transform },
        commandSlot: index,
        commandVisible: this.commandVisible[index],
        commandTransform: { ...this.commandTransforms[index] },
      };
    });
  }

  resourceRequests(): SemanticResourceKey[] {
    return this.resources.map((resource) => ({ ...resource }));
  }

  applyLayerPatches(patches: SemanticLayerPatch[]): void {
    for (const patch of patches) {
      const state = this.stateByLayer.get(patch.layer_id);
      if (!state) throw new Error(`unknown layer patch ${patch.layer_id}`);
      if (patch.render_mask != null) state.visible = patch.render_mask;
      if (patch.transform != null) state.transform = { ...patch.transform };
    }
  }

  applyCommandPatches(patches: SemanticCommandStatePatch[]): void {
    for (const patch of patches) {
      if (!Number.isInteger(patch.slot) || patch.slot < 0 || patch.slot >= this.commands.length) throw new Error(`invalid command slot ${patch.slot}`);
      if (patch.render_mask != null) this.commandVisible[patch.slot] = patch.render_mask;
      if (patch.transform != null) this.commandTransforms[patch.slot] = { ...patch.transform };
    }
  }
}

export function semanticCommandPlanFromCoreSnapshot(snapshot: {
  revisions: { layer_table: number } & Record<string, number>;
  layer_table: SemanticCommandPlanInput["layerTable"];
  layer_sources: Array<{ id: string; matrix: [number, number, number, number, number, number] }>;
  commands: SemanticCommandPlanInput["layerCommands"];
  semantic_commands: SemanticCommand[];
  command_states: NonNullable<SemanticCommandPlanInput["commandStates"]>;
}): SemanticCommandPlan {
  return new SemanticCommandPlan({
    layerTableRevision: snapshot.revisions.layer_table,
    layerTable: snapshot.layer_table,
    layerSources: snapshot.layer_sources.map((layer) => ({ layer_id: layer.id, matrix: layer.matrix })),
    layerCommands: snapshot.commands,
    semanticCommands: snapshot.semantic_commands,
    commandStates: snapshot.command_states,
  });
}

export type CoreSceneDumpInput = {
  revisions: { layer_table: number } & Record<string, number>;
  layer_table: SemanticCommandPlanInput["layerTable"];
  layers: Array<{
    layer_id: string;
    matrix: [number, number, number, number, number, number];
    render_mask: boolean;
    commands: SemanticCommand[];
  }>;
  command_states: NonNullable<SemanticCommandPlanInput["commandStates"]>;
};

/** Adapts the native/server debug dump to the exact same immutable browser
 * command plan used by a live WASM Scene. This is intentionally strict so a
 * dump cannot silently reorder authored layers or command ownership. */
export function semanticCommandPlanFromCoreDump(dump: CoreSceneDumpInput): SemanticCommandPlan {
  const layerById = new Map(dump.layers.map((layer) => [layer.layer_id, layer] as const));
  const semanticCommands: SemanticCommand[] = [];
  const layerCommands: SemanticCommandPlanInput["layerCommands"] = [];
  const layerSources: SemanticCommandPlanInput["layerSources"] = [];
  for (const entry of [...dump.layer_table].sort((left, right) => left.slot - right.slot)) {
    const layer = layerById.get(entry.layer_id);
    if (!layer) throw new Error(`core dump missing layer ${entry.layer_id}`);
    const commandStart = semanticCommands.length;
    semanticCommands.push(...layer.commands);
    layerSources.push({ layer_id: layer.layer_id, matrix: layer.matrix });
    layerCommands.push({
      layer_id: layer.layer_id,
      render_mask: layer.render_mask,
      transform: { dx: 0, dy: 0 },
      command_start: commandStart,
      command_count: layer.commands.length,
    });
  }
  if (layerById.size !== dump.layer_table.length) throw new Error("core dump contains layers outside the layer table");
  return new SemanticCommandPlan({
    layerTableRevision: dump.revisions.layer_table,
    layerTable: dump.layer_table,
    layerSources,
    layerCommands,
    semanticCommands,
    commandStates: dump.command_states,
  });
}

function collectResourceRequests(commands: SemanticCommand[]): SemanticResourceKey[] {
  const deduplicated = new Map<string, SemanticResourceKey>();
  for (const command of commands) {
    const resource = commandResource(command.payload);
    if (!resource) continue;
    deduplicated.set(`${resource.namespace}\0${resource.key}`, resource);
    if (command.payload.kind === "image") {
      const mask = asResourceKey(command.payload.alpha_mask);
      if (mask) deduplicated.set(`${mask.namespace}\0${mask.key}`, mask);
    }
  }
  return [...deduplicated.values()].map((resource) => ({ ...resource }));
}

function commandResource(payload: SemanticCommand["payload"]): SemanticResourceKey | null {
  if (payload.kind === "image") return asResourceKey(payload.resource);
  if (payload.kind !== "shape") return null;
  const primitive = payload.primitive;
  if (!primitive || typeof primitive !== "object" || Array.isArray(primitive)) return null;
  const assetMask = (primitive as Record<string, unknown>).asset_mask;
  if (!assetMask || typeof assetMask !== "object" || Array.isArray(assetMask)) return null;
  return asResourceKey((assetMask as Record<string, unknown>).resource);
}

function asResourceKey(value: unknown): SemanticResourceKey | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const { namespace, key } = value as Record<string, unknown>;
  return typeof namespace === "string" && typeof key === "string" ? { namespace, key } : null;
}
