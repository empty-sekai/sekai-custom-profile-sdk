//! Dynamic layer state and the line-indent program that drives it: the per-tick
//! runtime, its settle and two-cycle rules, and bounded materialization.

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{
    LineIndentSource, CONVERGENCE_EPSILON, TICKS_PER_SECOND, TMP_PAD, TMP_SEED_WIDTH,
    TWO_CYCLE_EPSILON, WARMUP_TICKS,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicStatus {
    Running,
    Settled,
    Periodic,
    Held,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct TransformDelta {
    pub dx: f32,
    pub dy: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DynamicLayerState {
    pub status: DynamicStatus,
    pub transform: TransformDelta,
    pub timeline: Option<TimelineDescriptor>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineDescriptor {
    pub loop_start_tick: u64,
    pub period_ticks: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct LineIndentRuntime {
    source: LineIndentSource,
    lines: Vec<LineIndentMetrics>,
    pub(crate) static_width_tmp: f32,
    pub(crate) width_tmp: f32,
    produced_ticks: u64,
    sample_count: u64,
    /// The most recent samples, oldest first, at most [`LINE_INDENT_SAMPLE_WINDOW`].
    samples: Vec<f32>,
    pub(crate) state: DynamicLayerState,
}

/// Samples a two-cycle has to repeat over before it is reported as periodic.
const LINE_INDENT_SAMPLE_WINDOW: usize = 4;

#[derive(Clone, Copy, Debug)]
struct LineIndentMetrics {
    natural_width_tmp: f32,
    last_advance_tmp: f32,
}

impl LineIndentMetrics {
    fn from_advances(advances: &[f32]) -> Option<Self> {
        let advances = advances
            .iter()
            .copied()
            .filter(|value| value.is_finite() && *value > 0.0)
            .collect::<Vec<_>>();
        Some(Self {
            natural_width_tmp: advances.iter().sum(),
            last_advance_tmp: advances.last().copied()?,
        })
    }

    fn preferred_width_tmp(self, indent_tmp: f32) -> f32 {
        let x_advance_tmp = self.natural_width_tmp + indent_tmp;
        if x_advance_tmp >= self.last_advance_tmp {
            x_advance_tmp
        } else {
            2.0 * self.last_advance_tmp - x_advance_tmp
        }
    }
}

impl LineIndentRuntime {
    pub(crate) fn new(source: LineIndentSource) -> Option<Self> {
        let lines = source
            .line_advances_tmp
            .iter()
            .filter_map(|advances| LineIndentMetrics::from_advances(advances))
            .collect::<Vec<_>>();
        let reference_width_tmp = lines
            .iter()
            .map(|line| line.natural_width_tmp)
            .reduce(f32::max)?;
        if !source.percent.is_finite() || reference_width_tmp <= 0.0 {
            return None;
        }
        let pct = source.percent / 100.0;
        let static_width_tmp = if pct < 1.0 {
            (reference_width_tmp + TMP_PAD) / (1.0 - pct)
        } else {
            reference_width_tmp + TMP_PAD
        };
        let mut runtime = Self {
            source,
            lines,
            static_width_tmp,
            width_tmp: TMP_SEED_WIDTH,
            produced_ticks: 0,
            sample_count: 0,
            samples: Vec::with_capacity(LINE_INDENT_SAMPLE_WINDOW),
            state: DynamicLayerState {
                status: DynamicStatus::Running,
                transform: TransformDelta::default(),
                timeline: None,
            },
        };
        runtime.prime_visible_tick_zero();
        Some(runtime)
    }

    pub(crate) fn reset(&mut self) {
        self.width_tmp = TMP_SEED_WIDTH;
        self.produced_ticks = 0;
        self.sample_count = 0;
        self.samples.clear();
        self.state.status = DynamicStatus::Running;
        self.state.transform = TransformDelta::default();
        self.state.timeline = None;
        self.prime_visible_tick_zero();
    }

    fn prime_visible_tick_zero(&mut self) {
        for _ in 0..=WARMUP_TICKS {
            self.advance_one();
        }
    }

    pub(crate) fn advance_one(&mut self) {
        if matches!(
            self.state.status,
            DynamicStatus::Settled | DynamicStatus::Held
        ) {
            self.produced_ticks += 1;
            return;
        }
        let pct = self.source.percent / 100.0;
        let indent_tmp = pct * self.width_tmp;
        let preferred_width_tmp = self
            .lines
            .iter()
            .map(|line| line.preferred_width_tmp(indent_tmp))
            .fold(0.0f32, f32::max);
        if !preferred_width_tmp.is_finite() {
            self.state.status = DynamicStatus::Held;
            self.produced_ticks += 1;
            return;
        }
        if self.produced_ticks >= WARMUP_TICKS {
            let local = line_indent_shift_per_width(pct, self.source.alignment)
                * (self.width_tmp - self.static_width_tmp);
            if !local.is_finite() {
                self.state.status = DynamicStatus::Held;
                self.produced_ticks += 1;
                return;
            }
            let theta = self.source.rotation_deg.to_radians();
            self.state.transform = TransformDelta {
                dx: local * theta.cos() * self.source.scale_x,
                dy: local * theta.sin() * self.source.scale_x,
            };
            let previous = self.samples.last().copied();
            if self.samples.len() == LINE_INDENT_SAMPLE_WINDOW {
                self.samples.remove(0);
            }
            self.samples.push(local);
            self.sample_count += 1;
            if previous.is_some_and(|previous| line_indent_settled(pct, local, previous)) {
                self.state.status = DynamicStatus::Settled;
            } else if self.state.status == DynamicStatus::Running
                && pct.abs() >= 1.0
                && self.samples.len() == LINE_INDENT_SAMPLE_WINDOW
                && is_two_cycle(&self.samples)
            {
                self.state.status = DynamicStatus::Periodic;
                self.state.timeline = Some(TimelineDescriptor {
                    loop_start_tick: self.sample_count - LINE_INDENT_SAMPLE_WINDOW as u64,
                    period_ticks: 2,
                });
            }
        }
        self.width_tmp = preferred_width_tmp + TMP_PAD;
        self.produced_ticks += 1;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineIndentFrameLocal {
    pub tick: u32,
    pub dx_local: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineIndentMaterialization {
    pub fps: u32,
    pub looped: bool,
    pub frames: Vec<LineIndentFrameLocal>,
}

/// Compatibility-only bounded materialization for native encoders/debuggers.
/// Browser runtime must keep the stateful program instead of playing this list.
pub fn materialize_line_indent(
    mut source: LineIndentSource,
    max_frames: usize,
) -> Option<LineIndentMaterialization> {
    source.rotation_deg = 0.0;
    source.scale_x = 1.0;
    let mut runtime = LineIndentRuntime::new(source)?;
    let mut frames = Vec::with_capacity(max_frames.min(512));
    for tick in 0..max_frames {
        frames.push(LineIndentFrameLocal {
            tick: tick as u32,
            dx_local: runtime.state.transform.dx,
        });
        if runtime.state.status == DynamicStatus::Settled {
            break;
        }
        if runtime.state.status == DynamicStatus::Held {
            break;
        }
        if runtime.state.status == DynamicStatus::Periodic {
            let len = frames.len();
            if len >= 2 {
                frames = vec![
                    LineIndentFrameLocal {
                        tick: 0,
                        dx_local: frames[len - 2].dx_local,
                    },
                    LineIndentFrameLocal {
                        tick: 1,
                        dx_local: frames[len - 1].dx_local,
                    },
                ];
            }
            return Some(LineIndentMaterialization {
                fps: TICKS_PER_SECOND,
                looped: true,
                frames,
            });
        }
        runtime.advance_one();
    }
    Some(LineIndentMaterialization {
        fps: TICKS_PER_SECOND,
        looped: false,
        frames,
    })
}

/// How far the first glyph moves per unit of text-rect width change.
///
/// The rect is centred on the layer. TMP starts a left-aligned line at the
/// left edge, a centred one half the free width in, and a right-aligned one
/// all of the free width in, where the free width is the rect width minus the
/// line advance and the advance includes the `pct * width` indent. Measured
/// from the centre that puts the first glyph at `(pct - 0.5) * w`,
/// `(pct * w - natural) / 2` and `w / 2 - natural`.
fn line_indent_shift_per_width(pct: f32, alignment: u8) -> f32 {
    match alignment {
        2 => pct / 2.0,
        4 => 0.5,
        _ => pct - 0.5,
    }
}

/// Whether a line-indent program has come to rest after producing `local`
/// following `previous`.
///
/// Each frame feeds the preferred width back into the rect, and every branch
/// of that map scales width changes by `|pct|`. With `|pct| < 1` the loop
/// therefore contracts to a single rest point. For a non-negative percentage
/// the rest point is the static layout, so the offset itself has to reach
/// zero. A negative percentage can rest elsewhere, because an indent that
/// pulls a line's advance below its last glyph makes the preferred width grow
/// again; there the remaining travel, at most `|pct| / (1 - |pct|)` times the
/// last step, has to fall under the epsilon.
fn line_indent_settled(pct: f32, local: f32, previous: f32) -> bool {
    if pct >= 0.0 {
        return pct < 1.0 && local.abs() <= CONVERGENCE_EPSILON;
    }
    let rate = -pct;
    rate < 1.0 && (local - previous).abs() * rate <= CONVERGENCE_EPSILON * (1.0 - rate)
}

fn is_two_cycle(samples: &[f32]) -> bool {
    let Some((&a, rest)) = samples.split_first() else {
        return false;
    };
    let Some((&b, _)) = rest.split_first() else {
        return false;
    };
    samples.iter().enumerate().all(|(index, value)| {
        (*value - if index % 2 == 0 { a } else { b }).abs() <= TWO_CYCLE_EPSILON
    })
}
