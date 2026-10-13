//! `Scene`: the validated layer tree with its render masks, component controls and
//! dynamic programs, and the deltas, snapshots and dumps it produces.

mod control;
mod layer_table;

use std::collections::{BTreeMap, BTreeSet};

use crate::line_indent::LineIndentRuntime;
use crate::{
    profile_scene, AnimationPreflight, CommandControlBinding, CommandId, CommandPatch,
    CommandState, CoreError, CoreTelemetry, DirtyKinds, DynamicStatus, GlyphCommand,
    InteractionRegionSnapshot, LayerCommand, LayerDump, LayerId, LayerPatch, LayerState,
    LayerTableEntry, RenderMaskOverride, Revisions, SceneDelta, SceneDump, SceneSnapshot,
    SceneSource, StableId, CONVERGENCE_EPSILON, SCHEMA_MAJOR, SCHEMA_MINOR,
    STATIC_FINAL_ANALYSIS_TICKS, TICKS_PER_SECOND,
};

use control::{
    command_control_id, evaluate_command_state, evaluate_control_bindings,
    validate_component_control,
};
use layer_table::{
    build_dfs_layer_table, merge_intervals, multiply_matrix, quad_bounds, resolve_render_mask,
    semantic_command_spans, transform_quad,
};

pub struct Scene {
    pub(crate) source: SceneSource,
    order: Vec<LayerId>,
    subtree_intervals: BTreeMap<LayerId, (usize, usize)>,
    states: BTreeMap<LayerId, LayerState>,
    programs: BTreeMap<LayerId, LineIndentRuntime>,
    command_states: BTreeMap<CommandId, CommandState>,
    tick: u64,
    revisions: Revisions,
    telemetry: CoreTelemetry,
}

impl Scene {
    pub fn animation_preflight(&self, maximum_tick: u64) -> Result<AnimationPreflight, CoreError> {
        if maximum_tick > TICKS_PER_SECOND as u64 * 60 * 60 {
            return Err(CoreError::Codec(
                "animation preflight exceeds one hour".into(),
            ));
        }
        let mut observable_layer_ids = Vec::new();
        for (layer_id, program) in &self.programs {
            if !self
                .states
                .get(layer_id)
                .is_some_and(|state| state.render_mask)
            {
                continue;
            }
            let mut runtime = program.clone();
            let initial = runtime.state.transform;
            let mut observable = false;
            for _ in 0..maximum_tick {
                runtime.advance_one();
                let current = runtime.state.transform;
                if (current.dx - initial.dx).abs() > CONVERGENCE_EPSILON
                    || (current.dy - initial.dy).abs() > CONVERGENCE_EPSILON
                {
                    observable = true;
                    break;
                }
            }
            if observable {
                observable_layer_ids.push(*layer_id);
            }
        }
        let observable_program_count = observable_layer_ids.len() as u32;
        Ok(AnimationPreflight {
            sampled_through_tick: maximum_tick,
            compiled_program_count: self.programs.len() as u32,
            observable_program_count,
            observable_layer_ids,
            animated: observable_program_count > 0,
        })
    }

    pub fn new(mut source: SceneSource) -> Result<Self, CoreError> {
        let mut states = BTreeMap::new();
        let mut programs = BTreeMap::new();
        let mut parents = BTreeMap::new();
        for layer in &source.layers {
            if states.contains_key(&layer.id) {
                return Err(CoreError::DuplicateLayer(layer.id));
            }
            parents.insert(layer.id, layer.parent_id);
            let dynamic = layer
                .line_indent
                .clone()
                .and_then(LineIndentRuntime::new)
                .map(|runtime| {
                    let state = runtime.state.clone();
                    programs.insert(layer.id, runtime);
                    state
                });
            states.insert(
                layer.id,
                LayerState {
                    mask_override: RenderMaskOverride::InheritAuthored,
                    render_mask: layer.authored_visible,
                    dynamic,
                },
            );
        }
        for layer in &source.layers {
            if let Some(parent) = layer.parent_id {
                if !states.contains_key(&parent) {
                    return Err(CoreError::UnknownParent {
                        layer: layer.id,
                        parent,
                    });
                }
            }
            let mut seen = BTreeSet::new();
            let mut current = Some(layer.id);
            while let Some(id) = current {
                if !seen.insert(id) {
                    return Err(CoreError::LayerTreeCycle(id));
                }
                current = parents.get(&id).copied().flatten();
            }
        }
        let mut glyph_ids = BTreeMap::new();
        for glyph in &source.glyphs {
            if glyph_ids.insert(glyph.id, ()).is_some() {
                return Err(CoreError::DuplicateGlyph(glyph.id));
            }
            if !states.contains_key(&glyph.layer_id) {
                return Err(CoreError::UnknownLayer(glyph.layer_id));
            }
        }
        let (order, subtree_intervals) = build_dfs_layer_table(&source.layers);
        let order_slots = order
            .iter()
            .enumerate()
            .map(|(slot, layer_id)| (*layer_id, slot))
            .collect::<BTreeMap<_, _>>();
        let mut command_ids = BTreeSet::new();
        for command in &source.semantic_commands {
            if !states.contains_key(&command.layer_id) {
                return Err(CoreError::UnknownCommandLayer {
                    command_id: command.id,
                    layer_id: command.layer_id,
                });
            }
            if !command_ids.insert(command.id) {
                return Err(CoreError::DuplicateCommand(command.id));
            }
        }
        for region in &source.interaction_regions {
            for binding in &region.control_bindings {
                let control_id = command_control_id(binding);
                let control = source
                    .component_controls
                    .iter()
                    .find(|value| value.id == control_id)
                    .ok_or(CoreError::UnknownComponentControl(control_id))?;
                if control.layer_id != region.layer_id {
                    return Err(CoreError::InvalidComponentControl {
                        control_id,
                        reason: "interaction binding crosses authored layer boundary".into(),
                    });
                }
                match (binding, &control.state) {
                    (
                        CommandControlBinding::TabOption { value, .. },
                        profile_scene::ComponentControlState::Tabs { options, .. },
                    ) if options.contains(value) => {}
                    (
                        CommandControlBinding::ScrollContent { .. }
                        | CommandControlBinding::ScrollThumb { .. }
                        | CommandControlBinding::ScrollViewport { .. },
                        profile_scene::ComponentControlState::Scroll { .. },
                    ) => {}
                    _ => {
                        return Err(CoreError::InvalidComponentControl {
                            control_id,
                            reason: "interaction binding is incompatible with control state".into(),
                        })
                    }
                }
            }
        }
        let mut interaction_ids = BTreeSet::new();
        for region in &source.interaction_regions {
            if !states.contains_key(&region.layer_id) {
                return Err(CoreError::UnknownInteractionLayer {
                    region_id: region.id,
                    layer_id: region.layer_id,
                });
            }
            if !interaction_ids.insert(region.id) {
                return Err(CoreError::DuplicateInteractionRegion(region.id));
            }
        }
        let mut control_ids = BTreeSet::new();
        for control in &source.component_controls {
            if !states.contains_key(&control.layer_id) {
                return Err(CoreError::UnknownLayer(control.layer_id));
            }
            if !control_ids.insert(control.id) {
                return Err(CoreError::DuplicateComponentControl(control.id));
            }
            validate_component_control(control)?;
        }
        for command in &source.semantic_commands {
            for binding in &command.control_bindings {
                let control_id = command_control_id(binding);
                let control = source
                    .component_controls
                    .iter()
                    .find(|value| value.id == control_id)
                    .ok_or(CoreError::UnknownComponentControl(control_id))?;
                if control.layer_id != command.layer_id {
                    return Err(CoreError::InvalidComponentControl {
                        control_id,
                        reason: "binding crosses authored layer boundary".into(),
                    });
                }
                match (binding, &control.state) {
                    (
                        CommandControlBinding::TabOption { value, .. },
                        profile_scene::ComponentControlState::Tabs { options, .. },
                    ) if options.contains(value) => {}
                    (
                        CommandControlBinding::ScrollContent { .. }
                        | CommandControlBinding::ScrollThumb { .. }
                        | CommandControlBinding::ScrollViewport { .. },
                        profile_scene::ComponentControlState::Scroll { .. },
                    ) => {}
                    _ => {
                        return Err(CoreError::InvalidComponentControl {
                            control_id,
                            reason: "command binding is incompatible with control state".into(),
                        })
                    }
                }
            }
        }
        source
            .semantic_commands
            .sort_by_key(|command| order_slots[&command.layer_id]);
        source
            .interaction_regions
            .sort_by_key(|region| order_slots[&region.layer_id]);
        let command_states = source
            .semantic_commands
            .iter()
            .enumerate()
            .map(|(slot, command)| {
                (
                    command.id,
                    evaluate_command_state(command, slot as u32, &source.component_controls),
                )
            })
            .collect();
        let mut scene = Self {
            source,
            order,
            subtree_intervals,
            states,
            programs,
            command_states,
            tick: 0,
            revisions: Revisions {
                scene: 1,
                source: 1,
                resolve: 1,
                layer_table: 1,
                tree: 1,
                order: 1,
                local_layout: 1,
                command: 1,
                material: 1,
                transform: 1,
                resource: 1,
                atlas: 1,
                timeline: 1,
                dynamic_state: 1,
                mask: 1,
                snapshot: 1,
            },
            telemetry: CoreTelemetry::default(),
        };
        scene.recompute_render_masks();
        Ok(scene)
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn layer_count(&self) -> usize {
        self.order.len()
    }
    pub fn revisions(&self) -> Revisions {
        self.revisions
    }
    pub fn state(&self, layer_id: LayerId) -> Option<&LayerState> {
        self.states.get(&layer_id)
    }

    pub fn set_tab(&mut self, control_id: StableId, value: &str) -> Result<SceneDelta, CoreError> {
        let control = self
            .source
            .component_controls
            .iter_mut()
            .find(|control| control.id == control_id)
            .ok_or(CoreError::UnknownComponentControl(control_id))?;
        let profile_scene::ComponentControlState::Tabs { options, active } = &mut control.state
        else {
            return Err(CoreError::InvalidComponentControl {
                control_id,
                reason: "set_tab requires a tabs control".into(),
            });
        };
        if !options.iter().any(|option| option == value) {
            return Err(CoreError::InvalidComponentControl {
                control_id,
                reason: format!("unknown tab option {value}"),
            });
        }
        if active == value {
            return Ok(self.delta(
                self.revisions.snapshot,
                DirtyKinds::default(),
                Vec::new(),
                Vec::new(),
            ));
        }
        *active = value.into();
        Ok(self.finish_control_change(control_id))
    }

    pub fn scroll_by(&mut self, control_id: StableId, delta: f32) -> Result<SceneDelta, CoreError> {
        let current = self
            .source
            .component_controls
            .iter()
            .find(|control| control.id == control_id)
            .ok_or(CoreError::UnknownComponentControl(control_id))?;
        let profile_scene::ComponentControlState::Scroll { offset, .. } = current.state else {
            return Err(CoreError::InvalidComponentControl {
                control_id,
                reason: "scroll_by requires a scroll control".into(),
            });
        };
        self.set_scroll_offset(control_id, offset + delta)
    }

    pub fn set_scroll_offset(
        &mut self,
        control_id: StableId,
        requested: f32,
    ) -> Result<SceneDelta, CoreError> {
        let control = self
            .source
            .component_controls
            .iter_mut()
            .find(|control| control.id == control_id)
            .ok_or(CoreError::UnknownComponentControl(control_id))?;
        let profile_scene::ComponentControlState::Scroll {
            offset, min, max, ..
        } = &mut control.state
        else {
            return Err(CoreError::InvalidComponentControl {
                control_id,
                reason: "set_scroll_offset requires a scroll control".into(),
            });
        };
        if !requested.is_finite() {
            return Err(CoreError::InvalidComponentControl {
                control_id,
                reason: "scroll offset must be finite".into(),
            });
        }
        let next = requested.clamp(*min, *max);
        if *offset == next {
            return Ok(self.delta(
                self.revisions.snapshot,
                DirtyKinds::default(),
                Vec::new(),
                Vec::new(),
            ));
        }
        *offset = next;
        Ok(self.finish_control_change(control_id))
    }

    fn finish_control_change(&mut self, control_id: StableId) -> SceneDelta {
        let base_revision = self.revisions.snapshot;
        let mut patches = Vec::new();
        for (slot, command) in self.source.semantic_commands.iter().enumerate() {
            if !command
                .control_bindings
                .iter()
                .any(|binding| command_control_id(binding) == control_id)
            {
                continue;
            }
            let next =
                evaluate_command_state(command, slot as u32, &self.source.component_controls);
            let previous = &self.command_states[&command.id];
            if previous.render_mask != next.render_mask || previous.transform != next.transform {
                patches.push(CommandPatch {
                    command_id: command.id,
                    slot: slot as u32,
                    render_mask: (previous.render_mask != next.render_mask)
                        .then_some(next.render_mask),
                    transform: (previous.transform != next.transform).then_some(next.transform),
                });
                self.command_states.insert(command.id, next);
            }
        }
        if patches.is_empty() {
            return self.delta(base_revision, DirtyKinds::default(), Vec::new(), patches);
        }
        let has_mask = patches.iter().any(|patch| patch.render_mask.is_some());
        let has_transform = patches.iter().any(|patch| patch.transform.is_some());
        self.revisions.scene += 1;
        self.revisions.dynamic_state += 1;
        self.revisions.snapshot += 1;
        if has_mask {
            self.revisions.material += 1;
        }
        if has_transform {
            self.revisions.transform += 1;
        }
        self.telemetry.dirty_layers = 1;
        self.delta(
            base_revision,
            DirtyKinds {
                material: has_mask,
                transform: has_transform,
                control: true,
                ..DirtyKinds::default()
            },
            Vec::new(),
            patches,
        )
    }

    pub fn set_render_mask(
        &mut self,
        layer_id: LayerId,
        visible: bool,
    ) -> Result<SceneDelta, CoreError> {
        self.set_render_mask_override(
            layer_id,
            if visible {
                RenderMaskOverride::ForceVisible
            } else {
                RenderMaskOverride::ForceHidden
            },
        )
    }

    pub fn clear_render_mask(&mut self, layer_id: LayerId) -> Result<SceneDelta, CoreError> {
        self.set_render_mask_override(layer_id, RenderMaskOverride::InheritAuthored)
    }

    pub fn set_render_mask_override(
        &mut self,
        layer_id: LayerId,
        mask_override: RenderMaskOverride,
    ) -> Result<SceneDelta, CoreError> {
        self.apply_render_mask_overrides(&[(layer_id, mask_override)])
    }

    pub fn set_render_mask_overrides(
        &mut self,
        expected_layer_table_revision: u64,
        overrides: &[(LayerId, RenderMaskOverride)],
    ) -> Result<SceneDelta, CoreError> {
        if expected_layer_table_revision != self.revisions.layer_table {
            return Err(CoreError::StaleLayerTableRevision {
                actual: expected_layer_table_revision,
                expected: self.revisions.layer_table,
            });
        }
        self.apply_render_mask_overrides(overrides)
    }

    fn apply_render_mask_overrides(
        &mut self,
        overrides: &[(LayerId, RenderMaskOverride)],
    ) -> Result<SceneDelta, CoreError> {
        let base_revision = self.revisions.snapshot;
        let mut requested = BTreeMap::new();
        for (layer_id, mask_override) in overrides {
            if !self.states.contains_key(layer_id) {
                return Err(CoreError::UnknownLayer(*layer_id));
            }
            requested.insert(*layer_id, *mask_override);
        }
        let changed = requested
            .into_iter()
            .filter(|(layer_id, mask_override)| {
                self.states[layer_id].mask_override != *mask_override
            })
            .collect::<Vec<_>>();
        if changed.is_empty() {
            return Ok(self.delta(base_revision, DirtyKinds::default(), Vec::new(), Vec::new()));
        }
        let before = self
            .states
            .iter()
            .map(|(id, state)| (*id, state.render_mask))
            .collect::<BTreeMap<_, _>>();
        for (layer_id, mask_override) in &changed {
            self.states
                .get_mut(layer_id)
                .expect("validated layer state")
                .mask_override = *mask_override;
        }
        let intervals = merge_intervals(
            changed
                .iter()
                .map(|(layer_id, _)| self.subtree_intervals[layer_id])
                .collect(),
        );
        self.recompute_render_mask_intervals(&intervals);
        let patches = intervals
            .iter()
            .flat_map(|(start, end)| self.order[*start..*end].iter())
            .filter_map(|id| {
                let visible = self.states[id].render_mask;
                (before[id] != visible).then_some(LayerPatch {
                    layer_id: *id,
                    render_mask: Some(visible),
                    transform: None,
                })
            })
            .collect::<Vec<_>>();
        self.revisions.scene += 1;
        self.revisions.mask += 1;
        self.revisions.snapshot += 1;
        self.telemetry.dirty_layers = patches.len() as u32;
        Ok(self.delta(
            base_revision,
            DirtyKinds {
                mask: true,
                ..DirtyKinds::default()
            },
            patches,
            Vec::new(),
        ))
    }

    pub fn advance_to_tick(&mut self, tick: u64) -> SceneDelta {
        let base_revision = self.revisions.snapshot;
        if tick == self.tick {
            self.telemetry.dirty_layers = 0;
            return self.delta(base_revision, DirtyKinds::default(), Vec::new(), Vec::new());
        }
        let mut latest_patches = BTreeMap::new();
        let mut dynamic_state_changed = false;
        if tick < self.tick {
            dynamic_state_changed = !self.programs.is_empty();
            for (layer_id, runtime) in &mut self.programs {
                runtime.reset();
                if let Some(state) = self.states.get_mut(layer_id) {
                    state.dynamic = Some(runtime.state.clone());
                }
                latest_patches.insert(
                    *layer_id,
                    LayerPatch {
                        layer_id: *layer_id,
                        render_mask: None,
                        transform: Some(runtime.state.transform),
                    },
                );
            }
            self.tick = 0;
        }
        while self.tick < tick {
            self.tick += 1;
            for (layer_id, runtime) in &mut self.programs {
                let before = runtime.state.clone();
                runtime.advance_one();
                self.telemetry.dynamic_evaluations += 1;
                if runtime.state != before {
                    dynamic_state_changed = true;
                    if let Some(state) = self.states.get_mut(layer_id) {
                        state.dynamic = Some(runtime.state.clone());
                    }
                }
                if runtime.state.transform != before.transform {
                    latest_patches.insert(
                        *layer_id,
                        LayerPatch {
                            layer_id: *layer_id,
                            render_mask: None,
                            transform: Some(runtime.state.transform),
                        },
                    );
                }
            }
        }
        let patches = latest_patches.into_values().collect::<Vec<_>>();
        self.revisions.timeline += 1;
        self.revisions.snapshot += 1;
        if dynamic_state_changed {
            self.revisions.scene += 1;
            self.revisions.dynamic_state += 1;
        }
        if !patches.is_empty() {
            self.revisions.transform += 1;
        }
        self.telemetry.dirty_layers = patches.len() as u32;
        self.delta(
            base_revision,
            DirtyKinds {
                transform: !patches.is_empty(),
                ..DirtyKinds::default()
            },
            patches,
            Vec::new(),
        )
    }

    /// Advances renderer-owned dynamic programs to a deterministic static snapshot.
    ///
    /// This is a scene bootstrap operation for static consumers. It keeps local layout and
    /// command buffers immutable, stops once every program is settled, periodic, or held, and
    /// bounds pathological programs with the shared analysis limit.
    pub fn advance_to_static_final(&mut self) -> SceneDelta {
        let base_revision = self.revisions.snapshot;
        if self.programs.is_empty()
            || self
                .programs
                .values()
                .all(|runtime| !matches!(runtime.state.status, DynamicStatus::Running))
        {
            self.telemetry.dirty_layers = 0;
            return self.delta(base_revision, DirtyKinds::default(), Vec::new(), Vec::new());
        }

        let initial = self
            .programs
            .iter()
            .map(|(layer_id, runtime)| (*layer_id, runtime.state.clone()))
            .collect::<BTreeMap<_, _>>();
        let maximum_tick = self.tick.saturating_add(STATIC_FINAL_ANALYSIS_TICKS);
        while self.tick < maximum_tick
            && self
                .programs
                .values()
                .any(|runtime| matches!(runtime.state.status, DynamicStatus::Running))
        {
            self.tick += 1;
            for runtime in self.programs.values_mut() {
                if matches!(runtime.state.status, DynamicStatus::Running) {
                    runtime.advance_one();
                    self.telemetry.dynamic_evaluations += 1;
                }
            }
        }
        for runtime in self.programs.values_mut() {
            if matches!(runtime.state.status, DynamicStatus::Running) {
                runtime.state.status = DynamicStatus::Held;
            }
        }

        let mut patches = Vec::new();
        let mut dynamic_state_changed = false;
        for (layer_id, runtime) in &self.programs {
            let before = &initial[layer_id];
            dynamic_state_changed |= runtime.state != *before;
            if let Some(state) = self.states.get_mut(layer_id) {
                state.dynamic = Some(runtime.state.clone());
            }
            if runtime.state.transform != before.transform {
                patches.push(LayerPatch {
                    layer_id: *layer_id,
                    render_mask: None,
                    transform: Some(runtime.state.transform),
                });
            }
        }

        self.revisions.timeline += 1;
        self.revisions.snapshot += 1;
        if dynamic_state_changed {
            self.revisions.scene += 1;
            self.revisions.dynamic_state += 1;
        }
        if !patches.is_empty() {
            self.revisions.transform += 1;
        }
        self.telemetry.dirty_layers = patches.len() as u32;
        self.delta(
            base_revision,
            DirtyKinds {
                transform: !patches.is_empty(),
                ..DirtyKinds::default()
            },
            patches,
            Vec::new(),
        )
    }

    pub fn snapshot(&self) -> SceneSnapshot {
        let command_spans = semantic_command_spans(&self.order, &self.source.semantic_commands);
        let commands = self
            .order
            .iter()
            .filter_map(|layer_id| self.states.get(layer_id).map(|state| (*layer_id, state)))
            .map(|(layer_id, state)| LayerCommand {
                command_id: StableId::derive("layer-command-v1", &layer_id.0.to_le_bytes()),
                layer_id,
                render_mask: state.render_mask,
                transform: state
                    .dynamic
                    .as_ref()
                    .map(|value| value.transform)
                    .unwrap_or_default(),
                command_start: command_spans[&layer_id].0,
                command_count: command_spans[&layer_id].1,
            })
            .collect();
        SceneSnapshot {
            schema_major: SCHEMA_MAJOR,
            schema_minor: SCHEMA_MINOR,
            scene_id: self.source.scene_id,
            tick: self.tick,
            revisions: self.revisions,
            layer_table: self.layer_table(),
            layer_sources: self.source.layers.clone(),
            layer_tree: self
                .source
                .layers
                .iter()
                .map(|layer| (layer.id, layer.parent_id))
                .collect(),
            commands,
            semantic_commands: self.source.semantic_commands.clone(),
            component_controls: self.source.component_controls.clone(),
            command_states: self
                .source
                .semantic_commands
                .iter()
                .map(|command| self.command_states[&command.id].clone())
                .collect(),
            interaction_regions: self.evaluated_interaction_regions(),
            glyph_commands: self
                .source
                .glyphs
                .iter()
                .map(|glyph| GlyphCommand {
                    command_id: StableId::derive("glyph-command-v1", &glyph.id.0.to_le_bytes()),
                    glyph_id: glyph.id,
                    layer_id: glyph.layer_id,
                    bounds: glyph.bounds,
                    quad: glyph.quad,
                })
                .collect(),
            telemetry: self.telemetry.clone(),
        }
    }

    pub fn dump(&self) -> SceneDump {
        SceneDump {
            schema: "allium.scene-dump".into(),
            schema_major: SCHEMA_MAJOR,
            schema_minor: SCHEMA_MINOR,
            coordinate_space: "card-device-v1".into(),
            scene_id: self.source.scene_id,
            tick: self.tick,
            revisions: self.revisions,
            layer_table: self.layer_table(),
            layer_tree: self
                .source
                .layers
                .iter()
                .map(|layer| (layer.id, layer.parent_id))
                .collect(),
            layers: self
                .source
                .layers
                .iter()
                .map(|layer| {
                    let state = &self.states[&layer.id];
                    LayerDump {
                        layer_id: layer.id,
                        parent_id: layer.parent_id,
                        kind: layer.kind,
                        authored_kind: layer.authored_kind,
                        authored_index: layer.authored_index,
                        game_layer: layer.game_layer,
                        source_content: layer.source_content.clone(),
                        resolved_parameters: layer.resolved_parameters.clone(),
                        bounds: layer.bounds,
                        quad: layer.quad,
                        matrix: layer.matrix,
                        hit_geometry: layer.hit_geometry,
                        authored_visible: layer.authored_visible,
                        mask_override: state.mask_override,
                        render_mask: state.render_mask,
                        dynamic: state.dynamic.clone(),
                        line_indent: layer.line_indent.clone(),
                        glyphs: self
                            .source
                            .glyphs
                            .iter()
                            .filter(|glyph| glyph.layer_id == layer.id)
                            .cloned()
                            .collect(),
                        commands: self
                            .source
                            .semantic_commands
                            .iter()
                            .filter(|command| command.layer_id == layer.id)
                            .cloned()
                            .collect(),
                    }
                })
                .collect(),
            interaction_regions: self.evaluated_interaction_regions(),
            component_controls: self.source.component_controls.clone(),
            command_states: self
                .source
                .semantic_commands
                .iter()
                .map(|command| self.command_states[&command.id].clone())
                .collect(),
            telemetry: self.telemetry.clone(),
        }
    }

    /// Encodes the current [`SceneSnapshot`] into the compact binary form.
    ///
    /// The binary form is positional: enums are written by variant index
    /// rather than by the `kind` tags the JSON form uses, so every command
    /// payload, text source, control binding and control state reads back.
    pub fn encode_snapshot(&mut self) -> Result<Vec<u8>, CoreError> {
        let bytes = bincode::encode_to_vec(self.snapshot(), bincode::config::standard())
            .map_err(|error| CoreError::Codec(error.to_string()))?;
        self.telemetry.serialized_bytes += bytes.len() as u64;
        Ok(bytes)
    }

    /// Decodes bytes produced by [`Scene::encode_snapshot`], rejecting a
    /// snapshot from another schema major version.
    pub fn decode_snapshot(bytes: &[u8]) -> Result<SceneSnapshot, CoreError> {
        let (snapshot, _): (SceneSnapshot, usize) =
            bincode::decode_from_slice(bytes, bincode::config::standard())
                .map_err(|error| CoreError::Codec(error.to_string()))?;
        if snapshot.schema_major != SCHEMA_MAJOR {
            return Err(CoreError::IncompatibleSchema {
                actual: snapshot.schema_major,
                expected: SCHEMA_MAJOR,
            });
        }
        Ok(snapshot)
    }

    fn evaluated_interaction_regions(&self) -> Vec<InteractionRegionSnapshot> {
        let layers = self
            .source
            .layers
            .iter()
            .map(|layer| (layer.id, layer))
            .collect::<BTreeMap<_, _>>();
        self.source
            .interaction_regions
            .iter()
            .filter_map(|region| {
                let layer = layers.get(&region.layer_id)?;
                let state = self.states.get(&region.layer_id)?;
                let dynamic = state
                    .dynamic
                    .as_ref()
                    .map(|value| value.transform)
                    .unwrap_or_default();
                let control_state = evaluate_control_bindings(
                    &region.control_bindings,
                    region.bounds.height,
                    &self.source.component_controls,
                );
                let mut layer_matrix = layer.matrix;
                layer_matrix[4] += dynamic.dx;
                layer_matrix[5] += dynamic.dy;
                let mut content_matrix = layer_matrix;
                content_matrix[4] += control_state.transform.dx;
                content_matrix[5] += control_state.transform.dy;
                let quad = transform_quad(content_matrix, region.quad);
                let hit_geometry = transform_quad(content_matrix, region.hit_geometry);
                Some(InteractionRegionSnapshot {
                    id: region.id,
                    layer_id: region.layer_id,
                    role: region.role.clone(),
                    bounds: quad_bounds(hit_geometry),
                    quad,
                    matrix: multiply_matrix(content_matrix, region.matrix),
                    hit_geometry,
                    clip: region.clip.map(|clip| transform_quad(layer_matrix, clip)),
                    control_bindings: region.control_bindings.clone(),
                    resolved_data: region.resolved_data.clone(),
                    capabilities: region.capabilities.clone(),
                    render_mask: state.render_mask && control_state.render_mask,
                })
            })
            .collect()
    }

    fn delta(
        &self,
        base_revision: u64,
        dirty: DirtyKinds,
        patches: Vec<LayerPatch>,
        command_patches: Vec<CommandPatch>,
    ) -> SceneDelta {
        SceneDelta {
            schema_major: SCHEMA_MAJOR,
            schema_minor: SCHEMA_MINOR,
            scene_id: self.source.scene_id,
            base_revision,
            revision: self.revisions.snapshot,
            tick: self.tick,
            dirty,
            patches,
            command_patches,
        }
    }

    fn recompute_render_masks(&mut self) {
        let source = self
            .source
            .layers
            .iter()
            .map(|layer| (layer.id, (layer.parent_id, layer.authored_visible)))
            .collect::<BTreeMap<_, _>>();
        for layer_id in &self.order {
            let (parent_id, authored_visible) = source[layer_id];
            let parent_visible = parent_id
                .map(|parent| self.states[&parent].render_mask)
                .unwrap_or(true);
            let state = self
                .states
                .get_mut(layer_id)
                .expect("validated layer state");
            state.render_mask =
                resolve_render_mask(state.mask_override, authored_visible, parent_visible);
        }
    }

    fn recompute_render_mask_intervals(&mut self, intervals: &[(usize, usize)]) {
        let source = self
            .source
            .layers
            .iter()
            .map(|layer| (layer.id, (layer.parent_id, layer.authored_visible)))
            .collect::<BTreeMap<_, _>>();
        for (start, end) in intervals {
            for layer_id in &self.order[*start..*end] {
                let (parent_id, authored_visible) = source[layer_id];
                let parent_visible = parent_id
                    .map(|parent| self.states[&parent].render_mask)
                    .unwrap_or(true);
                let state = self
                    .states
                    .get_mut(layer_id)
                    .expect("validated layer state");
                state.render_mask =
                    resolve_render_mask(state.mask_override, authored_visible, parent_visible);
            }
        }
    }

    fn layer_table(&self) -> Vec<LayerTableEntry> {
        let parents = self
            .source
            .layers
            .iter()
            .map(|layer| (layer.id, layer.parent_id))
            .collect::<BTreeMap<_, _>>();
        self.order
            .iter()
            .enumerate()
            .map(|(slot, layer_id)| {
                let (subtree_start, subtree_end) = self.subtree_intervals[layer_id];
                LayerTableEntry {
                    layer_id: *layer_id,
                    parent_id: parents[layer_id],
                    slot: slot as u32,
                    subtree_start: subtree_start as u32,
                    subtree_end: subtree_end as u32,
                }
            })
            .collect()
    }
}
