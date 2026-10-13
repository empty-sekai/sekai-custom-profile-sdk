//! Frame timeline: the ticks a clip samples and the period it loops on.

pub(super) fn animation_loop_period_ticks(
    timelines: impl IntoIterator<Item = sekai_profile_core::TimelineDescriptor>,
    maximum_tick: u64,
) -> Option<u64> {
    let mut timelines = timelines.into_iter();
    let first = timelines.next()?;
    if first.loop_start_tick != 0 || first.period_ticks == 0 {
        return None;
    }
    let mut common_period = first.period_ticks;
    for timeline in timelines {
        if timeline.loop_start_tick != first.loop_start_tick || timeline.period_ticks == 0 {
            return None;
        }
        let divisor = gcd(common_period, timeline.period_ticks);
        common_period = common_period
            .checked_div(divisor)?
            .checked_mul(timeline.period_ticks)?;
    }
    (common_period <= maximum_tick && maximum_tick.is_multiple_of(common_period))
        .then_some(common_period)
}

fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

pub(super) fn plan_running_ticks(maximum_tick: u64, fps: u32) -> Vec<u64> {
    let frame_count = maximum_tick.saturating_mul(fps as u64).div_ceil(60);
    (0..frame_count)
        .map(|index| index.saturating_mul(60) / fps as u64)
        .collect()
}

pub(super) fn plan_ticks(
    scene: &mut sekai_profile_core::Scene,
    dynamic_layer_ids: &[sekai_profile_core::LayerId],
    maximum_tick: u64,
    fps: u32,
) -> Vec<u64> {
    let running_ticks = plan_running_ticks(maximum_tick, fps);
    let mut ticks = Vec::new();
    let hold_frames = (fps / 2).max(1) as usize;
    for (index, tick) in running_ticks.iter().copied().enumerate() {
        scene.advance_to_tick(tick);
        ticks.push(tick);
        let settled = !dynamic_layer_ids.is_empty()
            && dynamic_layer_ids.iter().all(|id| {
                scene
                    .state(*id)
                    .and_then(|state| state.dynamic.as_ref())
                    .is_some_and(|state| {
                        matches!(
                            state.status,
                            sekai_profile_core::DynamicStatus::Settled
                                | sekai_profile_core::DynamicStatus::Held
                        )
                    })
            });
        if settled && ticks.len() > 1 {
            ticks.extend(
                running_ticks
                    .iter()
                    .skip(index + 1)
                    .take(hold_frames)
                    .copied(),
            );
            break;
        }
    }
    ticks
}
