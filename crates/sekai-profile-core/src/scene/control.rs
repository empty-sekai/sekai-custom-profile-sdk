//! Component controls inside a scene: their binary encoding, validation, and the
//! visibility and offset they give the commands and regions bound to them.

use bincode::{Decode, Encode};

use crate::{
    profile_scene, CommandControlBinding, CommandState, CoreError, SemanticCommandSource, StableId,
    TransformDelta,
};

// Component controls travel in the binary snapshot next to the commands that
// bind to them. Their JSON form is tagged by `kind`; the binary form writes the
// fields in declaration order behind a variant index, like every other enum in
// the snapshot.
impl Encode for profile_scene::ComponentControlSource {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        self.id.encode(encoder)?;
        self.layer_id.encode(encoder)?;
        self.role.encode(encoder)?;
        self.state.encode(encoder)
    }
}

impl<Context> Decode<Context> for profile_scene::ComponentControlSource {
    fn decode<D: bincode::de::Decoder<Context = Context>>(
        decoder: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        Ok(Self {
            id: Decode::decode(decoder)?,
            layer_id: Decode::decode(decoder)?,
            role: Decode::decode(decoder)?,
            state: Decode::decode(decoder)?,
        })
    }
}

bincode::impl_borrow_decode!(profile_scene::ComponentControlSource);

impl Encode for profile_scene::ComponentControlState {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        match self {
            Self::Tabs { options, active } => {
                0u32.encode(encoder)?;
                options.encode(encoder)?;
                active.encode(encoder)
            }
            Self::Scroll {
                offset,
                min,
                max,
                viewport_extent,
                content_extent,
                step,
            } => {
                1u32.encode(encoder)?;
                offset.encode(encoder)?;
                min.encode(encoder)?;
                max.encode(encoder)?;
                viewport_extent.encode(encoder)?;
                content_extent.encode(encoder)?;
                step.encode(encoder)
            }
        }
    }
}

impl<Context> Decode<Context> for profile_scene::ComponentControlState {
    fn decode<D: bincode::de::Decoder<Context = Context>>(
        decoder: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        match u32::decode(decoder)? {
            0 => Ok(Self::Tabs {
                options: Decode::decode(decoder)?,
                active: Decode::decode(decoder)?,
            }),
            1 => Ok(Self::Scroll {
                offset: Decode::decode(decoder)?,
                min: Decode::decode(decoder)?,
                max: Decode::decode(decoder)?,
                viewport_extent: Decode::decode(decoder)?,
                content_extent: Decode::decode(decoder)?,
                step: Decode::decode(decoder)?,
            }),
            found => Err(bincode::error::DecodeError::UnexpectedVariant {
                type_name: "ComponentControlState",
                allowed: &bincode::error::AllowedEnumVariants::Range { min: 0, max: 1 },
                found,
            }),
        }
    }
}

bincode::impl_borrow_decode!(profile_scene::ComponentControlState);

pub(super) fn validate_component_control(
    control: &profile_scene::ComponentControlSource,
) -> Result<(), CoreError> {
    match &control.state {
        profile_scene::ComponentControlState::Tabs { options, active } => {
            if options.is_empty() || !options.contains(active) {
                return Err(CoreError::InvalidComponentControl {
                    control_id: control.id,
                    reason: "tabs require a non-empty option set containing active".into(),
                });
            }
        }
        profile_scene::ComponentControlState::Scroll {
            offset,
            min,
            max,
            viewport_extent,
            content_extent,
            step,
        } => {
            if ![offset, min, max, viewport_extent, content_extent, step]
                .into_iter()
                .all(|value| value.is_finite())
                || min > max
                || offset < min
                || offset > max
                || *viewport_extent <= 0.0
                || *content_extent < *viewport_extent
                || *step <= 0.0
            {
                return Err(CoreError::InvalidComponentControl {
                    control_id: control.id,
                    reason: "invalid finite scroll range/extents".into(),
                });
            }
        }
    }
    Ok(())
}

pub(super) fn command_control_id(binding: &CommandControlBinding) -> StableId {
    match binding {
        CommandControlBinding::TabOption { control_id, .. }
        | CommandControlBinding::ScrollContent { control_id }
        | CommandControlBinding::ScrollThumb { control_id }
        | CommandControlBinding::ScrollViewport { control_id } => *control_id,
    }
}

pub(super) fn evaluate_command_state(
    command: &SemanticCommandSource,
    slot: u32,
    controls: &[profile_scene::ComponentControlSource],
) -> CommandState {
    let evaluated =
        evaluate_control_bindings(&command.control_bindings, command.bounds.height, controls);
    CommandState {
        command_id: command.id,
        slot,
        render_mask: evaluated.render_mask,
        transform: evaluated.transform,
    }
}

pub(super) fn evaluate_control_bindings(
    bindings: &[CommandControlBinding],
    extent: f32,
    controls: &[profile_scene::ComponentControlSource],
) -> LayerStateFragment {
    let mut state = LayerStateFragment {
        render_mask: true,
        transform: TransformDelta::default(),
    };
    for binding in bindings {
        let control_id = command_control_id(binding);
        let control = controls
            .iter()
            .find(|control| control.id == control_id)
            .expect("validated control binding");
        match (binding, &control.state) {
            (
                CommandControlBinding::TabOption { value, .. },
                profile_scene::ComponentControlState::Tabs { active, .. },
            ) => state.render_mask &= value == active,
            (
                CommandControlBinding::ScrollContent { .. },
                profile_scene::ComponentControlState::Scroll { offset, .. },
            ) => state.transform.dy += -*offset,
            (
                CommandControlBinding::ScrollThumb { .. },
                profile_scene::ComponentControlState::Scroll {
                    offset,
                    min,
                    max,
                    viewport_extent,
                    ..
                },
            ) => {
                let travel = (*viewport_extent - extent).max(0.0);
                let progress = if max > min {
                    (*offset - *min) / (*max - *min)
                } else {
                    0.0
                };
                state.transform.dy += travel * progress;
            }
            (
                CommandControlBinding::ScrollViewport { .. },
                profile_scene::ComponentControlState::Scroll { .. },
            ) => {}
            _ => unreachable!("validated control binding"),
        }
    }
    state
}

pub(super) struct LayerStateFragment {
    pub(super) render_mask: bool,
    pub(super) transform: TransformDelta,
}
