//! Backend-neutral renderer state and command contract shared by native and WASM.

pub mod authoring_document;
pub mod authoring_session;
pub mod general_recipe;
pub mod locale;
pub mod masterdata;
pub mod profile_data;
pub mod profile_layout;
pub mod profile_resolve;
pub mod profile_scene;
pub mod profile_source;
pub mod profile_transform;
pub mod sdf_geometry;
pub mod tmp_text;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const SCHEMA_MAJOR: u16 = 1;
pub const SCHEMA_MINOR: u16 = 14;
pub const TICKS_PER_SECOND: u32 = 60;
pub const TMP_PAD: f32 = 64.0;
pub const TMP_SEED_WIDTH: f32 = 8.46;
pub const WARMUP_TICKS: u64 = 3;
pub const CONVERGENCE_EPSILON: f32 = 0.05;
pub const STATIC_FINAL_ANALYSIS_TICKS: u64 = 20_000;
pub const TWO_CYCLE_EPSILON: f32 = 0.02;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StableId(pub u64);

pub type SceneId = StableId;
pub type LayerId = StableId;
pub type GlyphId = StableId;
pub type CommandId = StableId;

impl StableId {
    pub fn derive(namespace: &str, source_key: &[u8]) -> Self {
        let mut hash = Sha256::new();
        hash.update((namespace.len() as u64).to_le_bytes());
        hash.update(namespace.as_bytes());
        hash.update((source_key.len() as u64).to_le_bytes());
        hash.update(source_key);
        let digest = hash.finalize();
        Self(u64::from_le_bytes(
            digest[..8].try_into().expect("sha256 prefix"),
        ))
    }
}

impl Serialize for StableId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            serializer.serialize_str(&format!("{:016x}", self.0))
        } else {
            serializer.serialize_u64(self.0)
        }
    }
}

impl<'de> Deserialize<'de> for StableId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if !deserializer.is_human_readable() {
            return u64::deserialize(deserializer).map(Self);
        }
        let raw = String::deserialize(deserializer)?;
        if raw.len() != 16 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(de::Error::custom(
                "stable id must be 16 lowercase/uppercase hex digits",
            ));
        }
        u64::from_str_radix(&raw, 16)
            .map(Self)
            .map_err(de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

pub type Matrix2d = [f32; 6];
pub type Quad = [[f32; 2]; 4];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ParameterValue {
    Bool(bool),
    I64(i64),
    F64(f64),
    Text(String),
    Vec2([f32; 2]),
    Color([f32; 4]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    Text,
    Image,
    Shape,
    Composite,
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoredElementKind {
    #[default]
    Text,
    Shape,
    CardMember,
    Stamp,
    Other,
    BondsHonor,
    Honor,
    Collection,
    General,
    StandMember,
    GeneralBackground,
    StoryBackground,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontRole {
    RegionFontId(i32),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TextSource {
    Authored {
        value: String,
    },
    ProfileField {
        field: String,
        value: String,
    },
    MasterData {
        table: String,
        key: String,
        value: String,
    },
    Localized {
        key: String,
        locale: String,
        value: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResourceKey {
    pub namespace: String,
    pub key: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    SrcOver,
    SrcIn,
    DstIn,
    Multiply,
    Screen,
    Add,
}

impl Default for BlendMode {
    fn default() -> Self {
        Self::SrcOver
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompositeOperation {
    #[default]
    Marker,
    BeginIsolation,
    EndIsolation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapePrimitive {
    Rect,
    RoundedRect { radius: [f32; 2] },
    Ellipse,
    AssetMask { resource: ResourceKey },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinearGradient {
    /// Normalized shape-space coordinates.
    pub start: [f32; 2],
    pub end: [f32; 2],
    pub start_color: [f32; 4],
    pub end_color: [f32; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageClip {
    RoundedRect { radius: [f32; 2] },
    Ellipse,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandControlBinding {
    TabOption { control_id: StableId, value: String },
    ScrollContent { control_id: StableId },
    ScrollThumb { control_id: StableId },
    ScrollViewport { control_id: StableId },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SemanticCommandPayload {
    Text {
        source: TextSource,
        font_role: FontRole,
        size: f32,
        color: [f32; 4],
        outline_color: [f32; 4],
        outline_size: f32,
        line_spacing: f32,
        alignment: u8,
        max_width: Option<f32>,
        max_height: Option<f32>,
    },
    Image {
        resource: ResourceKey,
        uv: Rect,
        tint: [f32; 4],
        #[serde(default)]
        clip: Option<ImageClip>,
        #[serde(default)]
        alpha_mask: Option<ResourceKey>,
    },
    Shape {
        primitive: ShapePrimitive,
        fill: [f32; 4],
        #[serde(default)]
        gradient: Option<LinearGradient>,
        stroke: [f32; 4],
        stroke_width: f32,
    },
    Composite {
        #[serde(default)]
        operation: CompositeOperation,
        opacity: f32,
        clip: Option<Quad>,
    },
}

/// Backend-neutral post-layout placement. The TMP layout output remains
/// unchanged; backends translate completed glyph geometry to this anchor.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextRenderPlacementSource {
    pub anchor_x: f32,
    pub baseline: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticCommandSource {
    pub id: CommandId,
    pub layer_id: LayerId,
    pub role: String,
    pub bounds: Rect,
    pub matrix: Matrix2d,
    pub hit_geometry: Quad,
    pub blend_mode: BlendMode,
    #[serde(default)]
    pub clip: Option<Quad>,
    #[serde(default)]
    pub control_bindings: Vec<CommandControlBinding>,
    #[serde(default)]
    pub metadata: BTreeMap<String, ParameterValue>,
    #[serde(default)]
    pub numeric_text_runs: Vec<tmp_text::NumericTextRun>,
    #[serde(default)]
    pub render_placement: Option<TextRenderPlacementSource>,
    pub payload: SemanticCommandPayload,
}

impl SemanticCommandSource {
    pub fn text(
        id: CommandId,
        layer_id: LayerId,
        role: impl Into<String>,
        source_content: impl Into<String>,
        font_role: FontRole,
    ) -> Self {
        let source_content = source_content.into();
        Self {
            id,
            layer_id,
            role: role.into(),
            bounds: Rect::default(),
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            hit_geometry: [[0.0; 2]; 4],
            blend_mode: BlendMode::SrcOver,
            clip: None,
            control_bindings: Vec::new(),
            metadata: BTreeMap::new(),
            numeric_text_runs: tmp_text::numeric_text_runs(&source_content),
            render_placement: None,
            payload: SemanticCommandPayload::Text {
                source: TextSource::Authored {
                    value: source_content,
                },
                font_role,
                size: 0.0,
                color: [1.0; 4],
                outline_color: [0.0; 4],
                outline_size: 0.0,
                line_spacing: 0.0,
                alignment: 1,
                max_width: None,
                max_height: None,
            },
        }
    }

    pub fn shape(
        id: CommandId,
        layer_id: LayerId,
        role: impl Into<String>,
        bounds: Rect,
        primitive: ShapePrimitive,
    ) -> Self {
        Self {
            id,
            layer_id,
            role: role.into(),
            bounds,
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            hit_geometry: [[0.0; 2]; 4],
            blend_mode: BlendMode::SrcOver,
            clip: None,
            control_bindings: Vec::new(),
            metadata: BTreeMap::new(),
            numeric_text_runs: Vec::new(),
            render_placement: None,
            payload: SemanticCommandPayload::Shape {
                primitive,
                fill: [1.0; 4],
                gradient: None,
                stroke: [0.0; 4],
                stroke_width: 0.0,
            },
        }
    }

    pub fn image(
        id: CommandId,
        layer_id: LayerId,
        role: impl Into<String>,
        resource: ResourceKey,
        bounds: Rect,
    ) -> Self {
        Self {
            id,
            layer_id,
            role: role.into(),
            bounds,
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            hit_geometry: [[0.0; 2]; 4],
            blend_mode: BlendMode::SrcOver,
            clip: None,
            control_bindings: Vec::new(),
            metadata: BTreeMap::new(),
            numeric_text_runs: Vec::new(),
            render_placement: None,
            payload: SemanticCommandPayload::Image {
                resource,
                uv: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                },
                tint: [1.0; 4],
                clip: None,
                alpha_mask: None,
            },
        }
    }

    pub fn composite(
        id: CommandId,
        layer_id: LayerId,
        role: impl Into<String>,
        bounds: Rect,
    ) -> Self {
        Self {
            id,
            layer_id,
            role: role.into(),
            bounds,
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            hit_geometry: [[0.0; 2]; 4],
            blend_mode: BlendMode::SrcOver,
            clip: None,
            control_bindings: Vec::new(),
            metadata: BTreeMap::new(),
            numeric_text_runs: Vec::new(),
            render_placement: None,
            payload: SemanticCommandPayload::Composite {
                operation: CompositeOperation::Marker,
                opacity: 1.0,
                clip: None,
            },
        }
    }

    pub fn profile_text(
        id: CommandId,
        layer_id: LayerId,
        role: impl Into<String>,
        field: impl Into<String>,
        value: impl Into<String>,
        font_role: FontRole,
    ) -> Self {
        let value = value.into();
        let mut command = Self::text(id, layer_id, role, &value, font_role);
        if let SemanticCommandPayload::Text { source, .. } = &mut command.payload {
            *source = TextSource::ProfileField {
                field: field.into(),
                value,
            };
        }
        command
    }

    pub fn localized_text(
        id: CommandId,
        layer_id: LayerId,
        role: impl Into<String>,
        key: impl Into<String>,
        locale: impl Into<String>,
        value: impl Into<String>,
        font_role: FontRole,
    ) -> Self {
        let value = value.into();
        let mut command = Self::text(id, layer_id, role, &value, font_role);
        if let SemanticCommandPayload::Text { source, .. } = &mut command.payload {
            *source = TextSource::Localized {
                key: key.into(),
                locale: locale.into(),
                value,
            };
        }
        command
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InteractionRegionSource {
    pub id: StableId,
    pub layer_id: LayerId,
    pub role: String,
    pub bounds: Rect,
    pub quad: Quad,
    pub matrix: Matrix2d,
    pub hit_geometry: Quad,
    #[serde(default)]
    pub clip: Option<Quad>,
    #[serde(default)]
    pub control_bindings: Vec<CommandControlBinding>,
    pub resolved_data: BTreeMap<String, ParameterValue>,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineIndentSource {
    pub percent: f32,
    pub line_advances_tmp: Vec<Vec<f32>>,
    pub rotation_deg: f32,
    pub scale_x: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerSource {
    pub id: LayerId,
    pub parent_id: Option<LayerId>,
    pub kind: LayerKind,
    pub authored_kind: AuthoredElementKind,
    pub authored_index: u32,
    pub game_layer: i32,
    pub z: i32,
    pub authored_visible: bool,
    pub source_content: String,
    pub resolved_parameters: BTreeMap<String, ParameterValue>,
    pub bounds: Rect,
    pub quad: Quad,
    pub matrix: Matrix2d,
    pub hit_geometry: Quad,
    pub line_indent: Option<LineIndentSource>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GlyphSource {
    pub id: GlyphId,
    pub layer_id: LayerId,
    pub output_ordinal: u32,
    pub source_span: [u32; 2],
    pub bounds: Rect,
    pub quad: Quad,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneSource {
    pub scene_id: SceneId,
    pub region: String,
    pub font_engine_fingerprint: String,
    pub raster_contract: String,
    pub layers: Vec<LayerSource>,
    pub glyphs: Vec<GlyphSource>,
    #[serde(default)]
    pub semantic_commands: Vec<SemanticCommandSource>,
    #[serde(default)]
    pub interaction_regions: Vec<InteractionRegionSource>,
    #[serde(default)]
    pub component_controls: Vec<profile_scene::ComponentControlSource>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicStatus {
    Running,
    Settled,
    Periodic,
    Held,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnimationPreflight {
    pub sampled_through_tick: u64,
    pub compiled_program_count: u32,
    pub observable_program_count: u32,
    pub observable_layer_ids: Vec<LayerId>,
    pub animated: bool,
}

#[derive(Clone, Debug)]
struct LineIndentRuntime {
    source: LineIndentSource,
    lines: Vec<LineIndentMetrics>,
    static_width_tmp: f32,
    width_tmp: f32,
    produced_ticks: u64,
    samples: Vec<f32>,
    state: DynamicLayerState,
}

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
    fn new(source: LineIndentSource) -> Option<Self> {
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
            samples: Vec::with_capacity(32),
            state: DynamicLayerState {
                status: DynamicStatus::Running,
                transform: TransformDelta::default(),
                timeline: None,
            },
        };
        runtime.prime_visible_tick_zero();
        Some(runtime)
    }

    fn reset(&mut self) {
        self.width_tmp = TMP_SEED_WIDTH;
        self.produced_ticks = 0;
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

    fn advance_one(&mut self) {
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
            let local = (pct - 0.5) * (self.width_tmp - self.static_width_tmp);
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
            if self.samples.len() < 32 {
                self.samples.push(local);
            }
            if (0.0..1.0).contains(&pct)
                && self.samples.len() >= 2
                && local.abs() <= CONVERGENCE_EPSILON
            {
                self.state.status = DynamicStatus::Settled;
            } else if self.samples.len() >= 4 && is_two_cycle(&self.samples) {
                self.state.status = DynamicStatus::Periodic;
                self.state.timeline = Some(TimelineDescriptor {
                    loop_start_tick: 0,
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeasuredTextUnit {
    pub advance: f32,
    pub hard_break: bool,
}

/// Inserts soft line breaks into TMP markup without splitting tags. `units` must contain one
/// entry per visible Unicode scalar (including existing newlines as `hard_break=true`).
pub fn wrap_tmp_markup(
    raw: &str,
    units: &[MeasuredTextUnit],
    max_width: f32,
) -> Result<String, &'static str> {
    if !max_width.is_finite() || max_width <= 0.0 {
        return Err("max_width must be finite and positive");
    }
    let mut breaks = BTreeSet::new();
    let mut width = 0.0f32;
    let mut line_units = 0usize;
    for (index, unit) in units.iter().enumerate() {
        if unit.hard_break {
            width = 0.0;
            line_units = 0;
            continue;
        }
        let advance = unit.advance.max(0.0);
        if line_units > 0 && width + advance > max_width {
            breaks.insert(index);
            width = 0.0;
            line_units = 0;
        }
        width += advance;
        line_units += 1;
    }

    let mut output = String::with_capacity(raw.len() + breaks.len());
    let mut visible_index = 0usize;
    let mut cursor = 0usize;
    while cursor < raw.len() {
        let rest = &raw[cursor..];
        if rest.starts_with('<') {
            if let Some(end) = rest.find('>') {
                let end = cursor + end + 1;
                output.push_str(&raw[cursor..end]);
                cursor = end;
                continue;
            }
        }
        let ch = rest.chars().next().ok_or("invalid utf-8 cursor")?;
        if breaks.contains(&visible_index) && ch != '\n' {
            output.push('\n');
        }
        output.push(ch);
        cursor += ch.len_utf8();
        visible_index += 1;
    }
    if visible_index != units.len() {
        return Err("measured unit count does not match visible markup text");
    }
    Ok(output)
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerCommand {
    pub command_id: CommandId,
    pub layer_id: LayerId,
    pub render_mask: bool,
    pub transform: TransformDelta,
    pub command_start: u32,
    pub command_count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GlyphCommand {
    pub command_id: CommandId,
    pub glyph_id: GlyphId,
    pub layer_id: LayerId,
    pub bounds: Rect,
    pub quad: Quad,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CoreTelemetry {
    pub dynamic_evaluations: u64,
    pub dirty_layers: u32,
    pub layout_runs: u64,
    pub command_rebuilds: u64,
    pub atlas_generations: u64,
    pub serialized_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
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

pub struct Scene {
    source: SceneSource,
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

    pub fn encode_snapshot(&mut self) -> Result<Vec<u8>, CoreError> {
        let bytes = bincode::serde::encode_to_vec(self.snapshot(), bincode::config::standard())
            .map_err(|error| CoreError::Codec(error.to_string()))?;
        self.telemetry.serialized_bytes += bytes.len() as u64;
        Ok(bytes)
    }

    pub fn decode_snapshot(bytes: &[u8]) -> Result<SceneSnapshot, CoreError> {
        let (snapshot, _): (SceneSnapshot, usize) =
            bincode::serde::decode_from_slice(bytes, bincode::config::standard())
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

fn validate_component_control(
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

fn command_control_id(binding: &CommandControlBinding) -> StableId {
    match binding {
        CommandControlBinding::TabOption { control_id, .. }
        | CommandControlBinding::ScrollContent { control_id }
        | CommandControlBinding::ScrollThumb { control_id }
        | CommandControlBinding::ScrollViewport { control_id } => *control_id,
    }
}

fn evaluate_command_state(
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

fn evaluate_control_bindings(
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

struct LayerStateFragment {
    render_mask: bool,
    transform: TransformDelta,
}

fn transform_quad(matrix: Matrix2d, quad: Quad) -> Quad {
    quad.map(|[x, y]| {
        [
            matrix[0] * x + matrix[2] * y + matrix[4],
            matrix[1] * x + matrix[3] * y + matrix[5],
        ]
    })
}

fn multiply_matrix(parent: Matrix2d, local: Matrix2d) -> Matrix2d {
    [
        parent[0] * local[0] + parent[2] * local[1],
        parent[1] * local[0] + parent[3] * local[1],
        parent[0] * local[2] + parent[2] * local[3],
        parent[1] * local[2] + parent[3] * local[3],
        parent[0] * local[4] + parent[2] * local[5] + parent[4],
        parent[1] * local[4] + parent[3] * local[5] + parent[5],
    ]
}

fn quad_bounds(quad: Quad) -> Rect {
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

fn resolve_render_mask(
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

fn semantic_command_spans(
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

fn build_dfs_layer_table(
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

fn merge_intervals(mut intervals: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GlyphCacheKey {
    pub region: String,
    pub font_digest: [u8; 32],
    pub font_engine_fingerprint: String,
    pub raster_contract: String,
    pub glyph_index: u32,
}

#[derive(Clone, Debug)]
pub struct GlyphCacheEntry {
    pub metrics: [i32; 6],
    pub pixels: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlyphCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub generations: u64,
    pub bytes: usize,
}

#[derive(Default)]
pub struct SessionGlyphCache {
    entries: HashMap<GlyphCacheKey, GlyphCacheEntry>,
    stats: GlyphCacheStats,
}

impl SessionGlyphCache {
    pub fn get_or_insert_with<F>(&mut self, key: GlyphCacheKey, generate: F) -> &GlyphCacheEntry
    where
        F: FnOnce() -> GlyphCacheEntry,
    {
        if self.entries.contains_key(&key) {
            self.stats.hits += 1;
        } else {
            self.stats.misses += 1;
            self.stats.generations += 1;
            let entry = generate();
            self.stats.bytes += entry.pixels.len();
            self.entries.insert(key.clone(), entry);
        }
        &self.entries[&key]
    }

    pub fn stats(&self) -> &GlyphCacheStats {
        &self.stats
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.stats.bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(id: u64, dynamic: bool) -> LayerSource {
        LayerSource {
            id: StableId(id),
            parent_id: None,
            kind: LayerKind::Text,
            authored_kind: AuthoredElementKind::Text,
            authored_index: id as u32,
            game_layer: id as i32,
            z: id as i32,
            authored_visible: true,
            source_content: "<line-indent=25%>abc".into(),
            resolved_parameters: BTreeMap::new(),
            bounds: Rect::default(),
            quad: [[0.0; 2]; 4],
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            hit_geometry: [[0.0; 2]; 4],
            line_indent: dynamic.then(|| LineIndentSource {
                percent: 25.0,
                line_advances_tmp: vec![vec![20.0, 20.0, 20.0]],
                rotation_deg: 0.0,
                scale_x: 1.0,
            }),
        }
    }

    fn scene() -> Scene {
        Scene::new(SceneSource {
            scene_id: StableId(9),
            region: "cn".into(),
            font_engine_fingerprint: "ft".into(),
            raster_contract: "sdf".into(),
            layers: vec![layer(1, true), layer(2, false)],
            glyphs: Vec::new(),
            semantic_commands: Vec::new(),
            interaction_regions: Vec::new(),
            component_controls: Vec::new(),
        })
        .unwrap()
    }

    #[test]
    fn animation_preflight_uses_compiled_visible_programs_not_source_text() {
        let dynamic_scene = scene();
        let descriptor = dynamic_scene.animation_preflight(480).unwrap();
        assert_eq!(descriptor.compiled_program_count, 1);
        assert_eq!(descriptor.observable_program_count, 1);
        assert_eq!(descriptor.observable_layer_ids, vec![StableId(1)]);
        assert!(descriptor.animated);

        let mut hidden_source = dynamic_scene.source.clone();
        hidden_source.layers[0].authored_visible = false;
        let hidden = Scene::new(hidden_source)
            .unwrap()
            .animation_preflight(480)
            .unwrap();
        assert_eq!(hidden.compiled_program_count, 1);
        assert_eq!(hidden.observable_program_count, 0);
        assert!(hidden.observable_layer_ids.is_empty());
        assert!(!hidden.animated);
    }

    #[test]
    fn periodic_program_exposes_a_proven_timeline_descriptor() {
        let mut source = scene().source.clone();
        source.layers[0].line_indent.as_mut().unwrap().percent = -5.0;
        let mut scene = Scene::new(source).unwrap();
        scene.advance_to_tick(8);
        let dynamic = scene.state(StableId(1)).unwrap().dynamic.as_ref().unwrap();
        assert_eq!(dynamic.status, DynamicStatus::Periodic);
        assert_eq!(
            dynamic.timeline,
            Some(TimelineDescriptor {
                loop_start_tick: 0,
                period_ticks: 2,
            })
        );
    }

    #[test]
    fn negative_full_percent_keeps_emitting_the_two_cycle_after_long_runtime() {
        let mut source = scene().source.clone();
        let dynamic = source.layers[0].line_indent.as_mut().unwrap();
        dynamic.percent = -100.0;
        dynamic.line_advances_tmp = vec![vec![48.0; 6]];
        let mut scene = Scene::new(source).unwrap();

        scene.advance_to_tick(1_000);
        let first = scene
            .state(StableId(1))
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .clone();
        let next = scene.advance_to_tick(1_001);
        let second = scene
            .state(StableId(1))
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .clone();
        let repeated = scene.advance_to_tick(1_002);
        let third = scene.state(StableId(1)).unwrap().dynamic.as_ref().unwrap();

        assert_eq!(first.status, DynamicStatus::Periodic);
        assert_eq!(second.status, DynamicStatus::Periodic);
        assert_ne!(first.transform, second.transform);
        assert_eq!(first.transform, third.transform);
        assert_eq!(next.patches.len(), 1);
        assert_eq!(repeated.patches.len(), 1);
    }

    #[test]
    fn positive_percent_emits_initial_patches_then_settles() {
        let mut source = scene().source.clone();
        let dynamic = source.layers[0].line_indent.as_mut().unwrap();
        dynamic.percent = 97.0;
        dynamic.line_advances_tmp = vec![vec![24.0; 6]];
        let mut scene = Scene::new(source).unwrap();

        let initial = scene.advance_to_tick(1);
        assert_eq!(initial.patches.len(), 1);
        assert_eq!(
            scene
                .state(StableId(1))
                .unwrap()
                .dynamic
                .as_ref()
                .unwrap()
                .status,
            DynamicStatus::Running
        );

        scene.advance_to_tick(2_000);
        assert_eq!(
            scene
                .state(StableId(1))
                .unwrap()
                .dynamic
                .as_ref()
                .unwrap()
                .status,
            DynamicStatus::Settled
        );
        assert!(scene.advance_to_tick(2_001).patches.is_empty());
    }

    #[test]
    fn static_final_bootstrap_leaves_no_dynamic_program_running() {
        let mut scene = scene();
        let delta = scene.advance_to_static_final();
        let dynamic = scene.state(StableId(1)).unwrap().dynamic.as_ref().unwrap();

        assert!(scene.tick() > 0);
        assert!(matches!(
            dynamic.status,
            DynamicStatus::Settled | DynamicStatus::Periodic | DynamicStatus::Held
        ));
        assert!(delta.dirty.transform);

        scene.advance_to_tick(0);
        assert_eq!(scene.tick(), 0);
        assert_eq!(
            scene
                .state(StableId(1))
                .unwrap()
                .dynamic
                .as_ref()
                .unwrap()
                .status,
            DynamicStatus::Running
        );
    }

    #[test]
    fn text_command_exposes_tmp_stripped_numeric_runs_in_schema() {
        let command = SemanticCommandSource::text(
            StableId(1),
            StableId(2),
            "value",
            "<color=#fff>12</color><b>34</b>-05",
            FontRole::RegionFontId(1),
        );
        assert_eq!(command.numeric_text_runs.len(), 2);
        assert_eq!(command.numeric_text_runs[0].text, "1234");
        assert_eq!(command.numeric_text_runs[0].plain_start, 0);
        assert_eq!(command.numeric_text_runs[0].plain_end, 4);
        assert_eq!(command.numeric_text_runs[1].text, "05");
    }

    #[test]
    fn mask_toggle_does_not_reset_timeline_layout_command_or_atlas() {
        let mut scene = scene();
        scene.advance_to_tick(120);
        let before = scene.revisions();
        let state_before = scene.state(StableId(1)).unwrap().dynamic.clone();
        for index in 0..10_000 {
            scene.set_render_mask(StableId(1), index % 2 == 0).unwrap();
        }
        let after = scene.revisions();
        assert_eq!(scene.tick(), 120);
        assert_eq!(before.timeline, after.timeline);
        assert_eq!(before.local_layout, after.local_layout);
        assert_eq!(before.command, after.command);
        assert_eq!(before.atlas, after.atlas);
        assert_eq!(state_before, scene.state(StableId(1)).unwrap().dynamic);
    }

    #[test]
    fn parent_mask_preserves_child_override_and_emits_only_resolved_changes() {
        let mut parent = layer(1, false);
        let mut child = layer(2, false);
        child.parent_id = Some(parent.id);
        parent.authored_visible = true;
        let mut scene = Scene::new(SceneSource {
            scene_id: StableId(10),
            region: "cn".into(),
            font_engine_fingerprint: "ft".into(),
            raster_contract: "sdf".into(),
            layers: vec![child, parent],
            glyphs: Vec::new(),
            semantic_commands: Vec::new(),
            interaction_regions: Vec::new(),
            component_controls: Vec::new(),
        })
        .unwrap();
        scene.set_render_mask(StableId(2), true).unwrap();
        let hidden = scene.set_render_mask(StableId(1), false).unwrap();
        assert_eq!(hidden.patches.len(), 2);
        let while_hidden = scene.set_render_mask(StableId(2), false).unwrap();
        assert!(while_hidden.dirty.mask && while_hidden.patches.is_empty());
        scene.set_render_mask(StableId(2), true).unwrap();
        let shown = scene.set_render_mask(StableId(1), true).unwrap();
        assert_eq!(shown.patches.len(), 2);
        assert!(scene.state(StableId(2)).unwrap().render_mask);
    }

    #[test]
    fn layer_table_uses_dfs_slots_and_contiguous_subtree_intervals() {
        let mut root = layer(1, false);
        let mut child_a = layer(2, false);
        let mut grandchild = layer(3, false);
        let mut child_b = layer(4, false);
        let other_root = layer(5, false);
        root.authored_visible = true;
        child_a.parent_id = Some(root.id);
        grandchild.parent_id = Some(child_a.id);
        child_b.parent_id = Some(root.id);
        let scene = Scene::new(SceneSource {
            scene_id: StableId(13),
            region: "cn".into(),
            font_engine_fingerprint: "ft".into(),
            raster_contract: "sdf".into(),
            // Deliberately not in tree order.
            layers: vec![grandchild, child_b, other_root, child_a, root],
            glyphs: Vec::new(),
            semantic_commands: Vec::new(),
            interaction_regions: Vec::new(),
            component_controls: Vec::new(),
        })
        .unwrap();

        let table = scene.snapshot().layer_table;
        assert_eq!(
            table.iter().map(|entry| entry.layer_id).collect::<Vec<_>>(),
            vec![
                StableId(5),
                StableId(1),
                StableId(4),
                StableId(2),
                StableId(3)
            ]
        );
        let root_entry = table
            .iter()
            .find(|entry| entry.layer_id == StableId(1))
            .unwrap();
        assert_eq!((root_entry.subtree_start, root_entry.subtree_end), (1, 5));
        let child_entry = table
            .iter()
            .find(|entry| entry.layer_id == StableId(2))
            .unwrap();
        assert_eq!((child_entry.subtree_start, child_entry.subtree_end), (3, 5));
    }

    #[test]
    fn bulk_mask_rejects_stale_layer_table_and_preserves_non_mask_revisions() {
        let mut root = layer(1, true);
        let mut child = layer(2, true);
        child.parent_id = Some(root.id);
        root.authored_visible = true;
        let mut scene = Scene::new(SceneSource {
            scene_id: StableId(14),
            region: "cn".into(),
            font_engine_fingerprint: "ft".into(),
            raster_contract: "sdf".into(),
            layers: vec![child, root],
            glyphs: Vec::new(),
            semantic_commands: Vec::new(),
            interaction_regions: Vec::new(),
            component_controls: Vec::new(),
        })
        .unwrap();
        scene.advance_to_tick(120);
        let before = scene.revisions();
        let stale = scene.set_render_mask_overrides(
            before.layer_table + 1,
            &[(StableId(1), RenderMaskOverride::ForceHidden)],
        );
        assert!(matches!(
            stale,
            Err(CoreError::StaleLayerTableRevision { .. })
        ));
        assert_eq!(scene.revisions(), before);

        let delta = scene
            .set_render_mask_overrides(
                before.layer_table,
                &[(StableId(1), RenderMaskOverride::ForceHidden)],
            )
            .unwrap();
        assert_eq!(delta.patches.len(), 2);
        let after = scene.revisions();
        assert_eq!(before.timeline, after.timeline);
        assert_eq!(before.local_layout, after.local_layout);
        assert_eq!(before.command, after.command);
        assert_eq!(before.atlas, after.atlas);
        assert_eq!(before.layer_table, after.layer_table);
    }

    #[test]
    fn analytic_distance_field_preserves_curve_distance_and_winding() {
        use crate::sdf_geometry::{AnalyticDistanceField, QuadSeg, Segment, Vec2};

        let contour = vec![
            Segment::Quad(QuadSeg {
                p0: Vec2::new(0.0, 0.0),
                p1: Vec2::new(5.0, 10.0),
                p2: Vec2::new(10.0, 0.0),
            }),
            Segment::line(Vec2::new(10.0, 0.0), Vec2::new(0.0, 0.0)),
        ];
        let field = AnalyticDistanceField::new(&[contour]);
        assert!(field.signed_distance(Vec2::new(5.0, 2.0)) < 0.0);
        assert!(field.signed_distance(Vec2::new(5.0, 8.0)) > 0.0);
        assert!((field.signed_distance(Vec2::new(5.0, 5.0))).abs() <= 1e-4);
    }

    #[test]
    fn invalid_layer_trees_fail_closed() {
        let mut orphan = layer(1, false);
        orphan.parent_id = Some(StableId(99));
        assert!(matches!(
            Scene::new(SceneSource {
                scene_id: StableId(11),
                region: "cn".into(),
                font_engine_fingerprint: "ft".into(),
                raster_contract: "sdf".into(),
                layers: vec![orphan],
                glyphs: Vec::new(),
                semantic_commands: Vec::new(),
                interaction_regions: Vec::new(),
                component_controls: Vec::new(),
            }),
            Err(CoreError::UnknownParent { .. })
        ));
        let mut first = layer(1, false);
        let mut second = layer(2, false);
        first.parent_id = Some(second.id);
        second.parent_id = Some(first.id);
        assert!(matches!(
            Scene::new(SceneSource {
                scene_id: StableId(12),
                region: "cn".into(),
                font_engine_fingerprint: "ft".into(),
                raster_contract: "sdf".into(),
                layers: vec![first, second],
                glyphs: Vec::new(),
                semantic_commands: Vec::new(),
                interaction_regions: Vec::new(),
                component_controls: Vec::new(),
            }),
            Err(CoreError::LayerTreeCycle(_))
        ));
    }

    #[test]
    fn dynamic_updates_are_transform_only_and_rewind_is_deterministic() {
        let mut scene = scene();
        let initial = scene
            .state(StableId(1))
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .transform;
        let first = scene.advance_to_tick(30);
        assert!(first.dirty.transform && !first.dirty.layout && !first.dirty.atlas);
        let transform = scene
            .state(StableId(1))
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .transform;
        let reset = scene.advance_to_tick(0);
        assert_eq!(reset.patches[0].transform, Some(initial));
        scene.advance_to_tick(30);
        assert_eq!(
            transform,
            scene
                .state(StableId(1))
                .unwrap()
                .dynamic
                .as_ref()
                .unwrap()
                .transform
        );
    }

    #[test]
    fn batched_advance_returns_latest_transform_and_same_tick_is_noop() {
        let mut scene = scene();
        let delta = scene.advance_to_tick(8);
        let current = scene
            .state(StableId(1))
            .unwrap()
            .dynamic
            .as_ref()
            .unwrap()
            .transform;
        assert_eq!(delta.patches.len(), 1);
        assert_eq!(delta.patches[0].transform, Some(current));
        let revisions = scene.revisions();
        let noop = scene.advance_to_tick(8);
        assert!(noop.patches.is_empty());
        assert_eq!(scene.revisions(), revisions);
    }

    #[test]
    fn snapshot_round_trip_and_major_reject() {
        let mut scene = scene();
        let bytes = scene.encode_snapshot().unwrap();
        let decoded = Scene::decode_snapshot(&bytes).unwrap();
        assert_eq!(decoded.scene_id, StableId(9));
        let mut bad = decoded;
        bad.schema_major += 1;
        let bytes = bincode::serde::encode_to_vec(bad, bincode::config::standard()).unwrap();
        assert!(matches!(
            Scene::decode_snapshot(&bytes),
            Err(CoreError::IncompatibleSchema { .. })
        ));
    }

    #[test]
    fn dump_exposes_source_parameters_geometry_and_mask() {
        let scene = scene();
        let dump = scene.dump();
        assert_eq!(dump.layers[0].source_content, "<line-indent=25%>abc");
        assert!(dump.layers[0].render_mask);
        assert_eq!(dump.coordinate_space, "card-device-v1");
    }

    #[test]
    fn authored_layer_owns_ordered_commands_without_component_layers() {
        let layer_id = StableId(41);
        let commands = vec![
            SemanticCommandSource::shape(
                StableId(4101),
                layer_id,
                "panel-background",
                Rect::default(),
                ShapePrimitive::RoundedRect { radius: [8.0, 8.0] },
            ),
            SemanticCommandSource::text(
                StableId(4102),
                layer_id,
                "player-name",
                "<color=#ffffff>Player</color>",
                FontRole::RegionFontId(1),
            ),
            SemanticCommandSource::localized_text(
                StableId(4103),
                layer_id,
                "comment-title",
                "custom_profile.general.comment.title",
                "ja-JP",
                "ひとこと",
                FontRole::RegionFontId(1),
            ),
        ];
        let mut source = SceneSource {
            scene_id: StableId(40),
            region: "jp".into(),
            font_engine_fingerprint: "ft".into(),
            raster_contract: "sdf".into(),
            layers: vec![layer(41, false)],
            glyphs: Vec::new(),
            semantic_commands: commands,
            interaction_regions: vec![InteractionRegionSource {
                id: StableId(4199),
                layer_id,
                role: "player-name".into(),
                bounds: Rect::default(),
                quad: [[0.0; 2]; 4],
                matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                hit_geometry: [[0.0; 2]; 4],
                clip: None,
                control_bindings: Vec::new(),
                resolved_data: BTreeMap::new(),
                capabilities: vec!["inspect".into(), "select_text".into()],
            }],
            component_controls: Vec::new(),
        };
        source.layers[0].kind = LayerKind::Composite;
        let scene = Scene::new(source).unwrap();
        let snapshot = scene.snapshot();
        assert_eq!(snapshot.layer_table.len(), 1);
        assert_eq!(snapshot.layer_tree.len(), 1);
        assert_eq!(snapshot.commands[0].command_start, 0);
        assert_eq!(snapshot.commands[0].command_count, 3);
        assert_eq!(snapshot.semantic_commands.len(), 3);
        assert_eq!(snapshot.interaction_regions.len(), 1);
        assert_eq!(snapshot.interaction_regions[0].layer_id, layer_id);
        assert!(snapshot
            .semantic_commands
            .iter()
            .all(|command| command.layer_id == layer_id));
    }

    #[test]
    fn component_controls_patch_command_state_without_rebuilding_layout_atlas_or_timeline() {
        let layer_id = StableId(61);
        let tab_id = StableId(6100);
        let scroll_id = StableId(6200);
        let mut first = SemanticCommandSource::text(
            StableId(6101),
            layer_id,
            "rank",
            "1",
            FontRole::RegionFontId(1),
        );
        first
            .control_bindings
            .push(CommandControlBinding::TabOption {
                control_id: tab_id,
                value: "rank".into(),
            });
        let mut challenge = SemanticCommandSource::text(
            StableId(6102),
            layer_id,
            "challenge",
            "2",
            FontRole::RegionFontId(1),
        );
        challenge
            .control_bindings
            .push(CommandControlBinding::TabOption {
                control_id: tab_id,
                value: "challenge".into(),
            });
        let mut content = SemanticCommandSource::shape(
            StableId(6201),
            layer_id,
            "scroll-content",
            Rect::default(),
            ShapePrimitive::Rect,
        );
        content
            .control_bindings
            .push(CommandControlBinding::ScrollContent {
                control_id: scroll_id,
            });
        let mut thumb = SemanticCommandSource::shape(
            StableId(6202),
            layer_id,
            "scroll-thumb",
            Rect {
                x: 0.0,
                y: 0.0,
                width: 8.0,
                height: 25.0,
            },
            ShapePrimitive::Rect,
        );
        thumb
            .control_bindings
            .push(CommandControlBinding::ScrollThumb {
                control_id: scroll_id,
            });
        let mut scene = Scene::new(SceneSource {
            scene_id: StableId(60),
            region: "cn".into(),
            font_engine_fingerprint: "ft".into(),
            raster_contract: "sdf".into(),
            layers: vec![layer(61, false)],
            glyphs: Vec::new(),
            semantic_commands: vec![first, challenge, content, thumb],
            interaction_regions: vec![
                InteractionRegionSource {
                    id: StableId(6299),
                    layer_id,
                    role: "item".into(),
                    bounds: Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 20.0,
                        height: 20.0,
                    },
                    quad: crate::profile_scene::rect_quad(Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 20.0,
                        height: 20.0,
                    }),
                    matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                    hit_geometry: crate::profile_scene::rect_quad(Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 20.0,
                        height: 20.0,
                    }),
                    clip: Some(crate::profile_scene::rect_quad(Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 20.0,
                        height: 100.0,
                    })),
                    control_bindings: vec![CommandControlBinding::ScrollContent {
                        control_id: scroll_id,
                    }],
                    resolved_data: BTreeMap::new(),
                    capabilities: vec!["inspect".into()],
                },
                InteractionRegionSource {
                    id: StableId(6199),
                    layer_id,
                    role: "rank-tab".into(),
                    bounds: Rect {
                        x: 30.0,
                        y: 0.0,
                        width: 20.0,
                        height: 20.0,
                    },
                    quad: crate::profile_scene::rect_quad(Rect {
                        x: 30.0,
                        y: 0.0,
                        width: 20.0,
                        height: 20.0,
                    }),
                    matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                    hit_geometry: crate::profile_scene::rect_quad(Rect {
                        x: 30.0,
                        y: 0.0,
                        width: 20.0,
                        height: 20.0,
                    }),
                    clip: None,
                    control_bindings: vec![CommandControlBinding::TabOption {
                        control_id: tab_id,
                        value: "rank".into(),
                    }],
                    resolved_data: BTreeMap::new(),
                    capabilities: vec!["activate".into()],
                },
                InteractionRegionSource {
                    id: StableId(6298),
                    layer_id,
                    role: "scroll-viewport".into(),
                    bounds: Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 20.0,
                        height: 100.0,
                    },
                    quad: crate::profile_scene::rect_quad(Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 20.0,
                        height: 100.0,
                    }),
                    matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                    hit_geometry: crate::profile_scene::rect_quad(Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 20.0,
                        height: 100.0,
                    }),
                    clip: None,
                    control_bindings: vec![CommandControlBinding::ScrollViewport {
                        control_id: scroll_id,
                    }],
                    resolved_data: BTreeMap::new(),
                    capabilities: vec!["scroll".into()],
                },
            ],
            component_controls: vec![
                crate::profile_scene::ComponentControlSource {
                    id: tab_id,
                    layer_id,
                    role: "mode".into(),
                    state: crate::profile_scene::ComponentControlState::Tabs {
                        options: vec!["rank".into(), "challenge".into()],
                        active: "rank".into(),
                    },
                },
                crate::profile_scene::ComponentControlSource {
                    id: scroll_id,
                    layer_id,
                    role: "list".into(),
                    state: crate::profile_scene::ComponentControlState::Scroll {
                        offset: 0.0,
                        min: 0.0,
                        max: 100.0,
                        viewport_extent: 100.0,
                        content_extent: 200.0,
                        step: 20.0,
                    },
                },
            ],
        })
        .unwrap();
        let baseline = scene.revisions();
        let initial = scene.snapshot();
        let item_region = initial
            .interaction_regions
            .iter()
            .find(|region| region.role == "item")
            .unwrap();
        assert_eq!(
            item_region.control_bindings,
            vec![CommandControlBinding::ScrollContent {
                control_id: scroll_id,
            }]
        );
        let tab_region = initial
            .interaction_regions
            .iter()
            .find(|region| region.role == "rank-tab")
            .unwrap();
        assert_eq!(
            tab_region.control_bindings,
            vec![CommandControlBinding::TabOption {
                control_id: tab_id,
                value: "rank".into(),
            }]
        );
        assert_eq!(
            scene.dump().interaction_regions,
            initial.interaction_regions
        );
        assert_eq!(
            initial
                .command_states
                .iter()
                .map(|state| state.render_mask)
                .collect::<Vec<_>>(),
            vec![true, false, true, true]
        );

        let tab = scene.set_tab(tab_id, "challenge").unwrap();
        assert_eq!(tab.command_patches.len(), 2);
        assert!(tab.dirty.material && !tab.dirty.layout && !tab.dirty.command && !tab.dirty.atlas);
        let scroll = scene.scroll_by(scroll_id, 30.0).unwrap();
        assert_eq!(scroll.command_patches.len(), 2);
        assert_eq!(scene.snapshot().command_states[2].transform.dy, -30.0);
        assert_eq!(scene.snapshot().command_states[3].transform.dy, 22.5);
        let after_controls = scene.snapshot();
        let region = after_controls
            .interaction_regions
            .iter()
            .find(|region| region.role == "item")
            .unwrap();
        assert_eq!(region.bounds.y, -30.0);
        assert_eq!(region.clip.unwrap()[0][1], 0.0);
        let viewport = after_controls
            .interaction_regions
            .iter()
            .find(|region| region.role == "scroll-viewport")
            .unwrap();
        assert_eq!(viewport.bounds.y, 0.0);
        assert_eq!(viewport.hit_geometry[0][1], 0.0);
        assert_eq!(
            viewport.control_bindings,
            vec![CommandControlBinding::ScrollViewport {
                control_id: scroll_id,
            }]
        );
        assert!(
            !after_controls
                .interaction_regions
                .iter()
                .find(|region| region.role == "rank-tab")
                .unwrap()
                .render_mask
        );
        let after = scene.revisions();
        assert_eq!(baseline.local_layout, after.local_layout);
        assert_eq!(baseline.command, after.command);
        assert_eq!(baseline.atlas, after.atlas);
        assert_eq!(baseline.timeline, after.timeline);

        scene.set_render_mask(layer_id, false).unwrap();
        scene.set_render_mask(layer_id, true).unwrap();
        assert_eq!(scene.snapshot().command_states[2].transform.dy, -30.0);
    }

    #[test]
    fn interaction_snapshot_applies_authored_layer_transform_and_mask() {
        let layer_id = StableId(51);
        let mut authored_layer = layer(51, false);
        authored_layer.matrix = [2.0, 0.0, 0.0, 3.0, 10.0, 20.0];
        let local_quad = [[1.0, 2.0], [5.0, 2.0], [5.0, 6.0], [1.0, 6.0]];
        let source = SceneSource {
            scene_id: StableId(50),
            region: "cn".into(),
            font_engine_fingerprint: "ft".into(),
            raster_contract: "sdf".into(),
            layers: vec![authored_layer],
            glyphs: Vec::new(),
            semantic_commands: Vec::new(),
            interaction_regions: vec![InteractionRegionSource {
                id: StableId(5199),
                layer_id,
                role: "content".into(),
                bounds: Rect {
                    x: 1.0,
                    y: 2.0,
                    width: 4.0,
                    height: 4.0,
                },
                quad: local_quad,
                matrix: [1.0, 0.0, 0.0, 1.0, 1.0, 2.0],
                hit_geometry: local_quad,
                clip: None,
                control_bindings: Vec::new(),
                resolved_data: BTreeMap::new(),
                capabilities: vec!["inspect".into()],
            }],
            component_controls: Vec::new(),
        };
        let mut scene = Scene::new(source).unwrap();
        scene.set_render_mask(layer_id, false).unwrap();
        let region = &scene.snapshot().interaction_regions[0];
        assert_eq!(
            region.quad,
            [[12.0, 26.0], [20.0, 26.0], [20.0, 38.0], [12.0, 38.0]]
        );
        assert_eq!(
            region.bounds,
            Rect {
                x: 12.0,
                y: 26.0,
                width: 8.0,
                height: 12.0
            }
        );
        assert_eq!(region.matrix, [2.0, 0.0, 0.0, 3.0, 12.0, 26.0]);
        assert!(!region.render_mask);
    }

    #[test]
    fn general_profile_layouts_are_shared_core_contracts() {
        use crate::profile_layout::*;
        let panels = [
            (2, &TOTAL_POWER, 5usize),
            (3, &DECK, 1),
            (4, &COMMENT, 3),
            (5, &LEADER_MEMBER, 4),
            (6, &HONORS, 4),
            (9, &MVP_SUPERSTAR, 6),
            (10, &CHALLENGE_LIVE, 5),
            (11, &CHAR_RANK, 6),
            (12, &MUSIC_CLEAR, 6),
            (13, &PLAYER_NAME, 2),
            (14, &STORY_FAVORITE, 10),
            (15, &CHAR_RANK, 6),
            (16, &MUSIC_CLEAR_TAB, 14),
        ];
        for (general_type, panel, elements) in panels {
            assert!(
                panel.w > 0.0 && panel.h > 0.0,
                "general type={general_type}"
            );
            assert_eq!(
                panel.elements.len(),
                elements,
                "general type={general_type}"
            );
        }
        assert_eq!((PLAYER_NAME.w, PLAYER_NAME.h), (610.0, 127.0));
        assert_eq!((COMMENT.w, COMMENT.h), (700.0, 251.0));
        assert_eq!((CHAR_RANK.w, CHAR_RANK.h), (967.0, 872.0));
        assert_eq!(
            (CHALLENGE_LIVE.elements[2].w, CHALLENGE_LIVE.elements[2].h),
            (92.0, 92.0),
            "the challenge avatar is a true circle; the former 87px height was a measurement error"
        );
    }

    #[test]
    fn complete_profile_card_source_schema_lives_in_shared_core() {
        let object = serde_json::json!({
            "layer": 1, "lock": false,
            "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
        });
        let card: crate::profile_source::CustomProfileCard = serde_json::from_value(serde_json::json!({
            "texts": [{ "objectData": object, "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 1, "outlineSize": 0.0, "size": 24.0, "text": "A", "type": 1 }],
            "shapes": [{ "objectData": object, "alpha": 1.0, "colorId": 1, "id": 1, "outlineAlpha": 1.0, "outlineColorId": 1, "outlineSize": 0.0 }],
            "cardMembers": [{ "objectData": object, "id": 1 }], "stamps": [{ "objectData": object, "id": 1 }],
            "others": [{ "objectData": object, "id": 1 }],
            "bondsHonors": [{ "objectData": object, "id": 1, "wordId": 1, "fullSize": true, "inverse": false, "useUnitVirtualSinger": false }],
            "honors": [{ "objectData": object, "id": 1, "fullSize": true }],
            "collections": [{ "objectData": object, "id": 1 }], "generals": [{ "objectData": object, "type": 4 }],
            "standMembers": [{ "objectData": object, "id": 1 }], "generalBackgrounds": [{ "objectData": object, "id": 1 }],
            "storyBackgrounds": [{ "objectData": object, "id": 1 }]
        })).unwrap();
        assert_eq!(card.element_count(), 12);
        assert_eq!(
            serde_json::to_value(&card).unwrap()["generals"][0]["type"],
            4
        );
    }

    #[test]
    fn profile_transform_is_shared_and_uses_canonical_canvas() {
        let object = crate::profile_source::ObjectData {
            layer: 1,
            lock: false,
            position: crate::profile_source::Vec3 {
                x: 10.0,
                y: 20.0,
                z: 0.0,
            },
            rotation: crate::profile_source::Quaternion {
                w: 1.0,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            scale: crate::profile_source::Vec3 {
                x: 2.0,
                y: 3.0,
                z: 1.0,
            },
            visible: true,
        };
        assert_eq!(
            crate::profile_transform::extract_transform(&object),
            (925.0, 386.0, -0.0, 2.0, 3.0)
        );
    }

    #[test]
    fn profile_scene_normalization_preserves_authored_identity_and_game_order() {
        let object = |layer: i32| {
            serde_json::json!({
                "layer": layer, "lock": false,
                "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
            })
        };
        let card: crate::profile_source::CustomProfileCard = serde_json::from_value(serde_json::json!({
            "texts": [
                { "objectData": object(9), "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 1, "outlineSize": 0.0, "size": 24.0, "text": "A", "type": 1 },
                { "objectData": object(2), "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 1, "outlineSize": 0.0, "size": 24.0, "text": "B", "type": 1 }
            ],
            "generals": [{ "objectData": object(2), "type": 4 }]
        })).unwrap();
        let normalized = crate::profile_scene::ordered_profile_elements(&card, "document");
        assert_eq!(normalized.len(), 3);
        assert_eq!(
            normalized
                .iter()
                .map(|element| element.object().layer)
                .collect::<Vec<_>>(),
            vec![2, 2, 9]
        );
        assert_eq!(normalized[0].kind, AuthoredElementKind::Text);
        assert_eq!(normalized[0].source_index, 1);
        assert_eq!(normalized[1].kind, AuthoredElementKind::General);
        assert_ne!(normalized[0].layer_id, normalized[1].layer_id);
        assert_eq!(
            normalized,
            crate::profile_scene::ordered_profile_elements(&card, "document")
        );
    }

    #[test]
    fn shared_profile_layer_assembly_owns_commands_and_geometry() {
        let card: crate::profile_source::CustomProfileCard =
            serde_json::from_value(serde_json::json!({
                "generals": [{
                    "objectData": {
                        "layer": 7, "lock": false,
                        "position": { "x": 10.0, "y": 20.0, "z": 0.0 },
                        "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                        "scale": { "x": 2.0, "y": 3.0, "z": 1.0 }, "visible": true
                    }, "type": 4
                }]
            }))
            .unwrap();
        let element = crate::profile_scene::ordered_profile_elements(&card, "document").remove(0);
        let command = SemanticCommandSource::shape(
            crate::profile_scene::semantic_command_id(&element.source_key, "background", 0),
            element.layer_id,
            "background",
            Rect {
                x: -10.0,
                y: -5.0,
                width: 20.0,
                height: 10.0,
            },
            ShapePrimitive::Rect,
        );
        let layer = crate::profile_scene::assemble_profile_layer(
            &element,
            LayerKind::Composite,
            String::new(),
            BTreeMap::new(),
            &[command],
            None,
        );
        assert_eq!(layer.game_layer, 7);
        assert_eq!(
            layer.bounds,
            Rect {
                x: -10.0,
                y: -5.0,
                width: 20.0,
                height: 10.0
            }
        );
        assert_eq!(layer.matrix, [2.0, -0.0, 0.0, 3.0, 925.0, 386.0]);
        assert_eq!(layer.authored_kind, AuthoredElementKind::General);
    }

    #[test]
    fn identity_general_lowering_is_shared_and_region_font_one_only() {
        let snapshot = crate::profile_scene::ProfileComponentSnapshot {
            locale: "jp".into(),
            region_fonts: BTreeMap::from([(1, "FOT-Rodin".into())]),
            localized_text: BTreeMap::from([(
                "custom_profile.general.comment.title".into(),
                "ひとこと".into(),
            )]),
            user_name: "<b>Player</b>".into(),
            word: "<color=#ff0000>Bio</color>".into(),
            user_rank: 0,
            total_power: 0,
            mvp: 0,
            superstar: 0,
            challenge_score: 0,
            challenge_character_id: 0,
            challenge_avatar: None,
            music_results: None,
            story_favorites: Vec::new(),
            player_avatar: None,
            character_ranks: Vec::new(),
            deck_members: Vec::new(),
            leader_card: None,
            honor_slots: Vec::new(),
        };
        let layer_id = StableId(81);
        let name =
            crate::profile_scene::lower_identity_general(13, layer_id, "general-name", &snapshot)
                .unwrap()
                .unwrap();
        let comment =
            crate::profile_scene::lower_identity_general(4, layer_id, "general-comment", &snapshot)
                .unwrap()
                .unwrap();
        assert_eq!(name.commands.len(), 2);
        assert_eq!(comment.commands.len(), 3);
        assert_eq!(name.interaction_regions.len(), 1);
        assert_eq!(comment.interaction_regions.len(), 1);
        for command in name.commands.iter().chain(&comment.commands) {
            if let SemanticCommandPayload::Text { font_role, .. } = &command.payload {
                assert_eq!(font_role, &FontRole::RegionFontId(1));
            }
        }
        assert!(comment.commands.iter().any(|command| matches!(
            &command.payload,
            SemanticCommandPayload::Text {
                source: TextSource::Localized { key, locale, value }, ..
            } if key == "custom_profile.general.comment.title" && locale == "jp" && value == "ひとこと"
        )));
        let mut missing_font = snapshot.clone();
        missing_font.region_fonts.clear();
        assert!(matches!(
            crate::profile_scene::lower_identity_general(13, layer_id, "x", &missing_font),
            Err(crate::profile_scene::ProfileResolveError::MissingRegionFont(1))
        ));
    }

    #[test]
    fn basic_general_lowering_has_profile_and_locale_provenance_without_component_layers() {
        let snapshot = crate::profile_scene::ProfileComponentSnapshot {
            locale: "ja-JP".into(),
            region_fonts: BTreeMap::from([(1, "FOT-Rodin".into())]),
            localized_text: crate::locale::GENERAL_LOCALIZATION_KEYS
                .iter()
                .map(|key| ((*key).into(), crate::locale::resolve("ja-JP", key).unwrap()))
                .collect(),
            user_name: String::new(),
            word: String::new(),
            user_rank: 100,
            total_power: 123_456,
            mvp: 12,
            superstar: 3,
            challenge_score: 765_432,
            challenge_character_id: 9,
            challenge_avatar: None,
            music_results: None,
            story_favorites: Vec::new(),
            player_avatar: None,
            character_ranks: Vec::new(),
            deck_members: Vec::new(),
            leader_card: None,
            honor_slots: Vec::new(),
        };
        for general_type in [2, 9, 10, 17] {
            let lowered = crate::profile_scene::lower_identity_general(
                general_type,
                StableId(91),
                &format!("general-{general_type}"),
                &snapshot,
            )
            .unwrap()
            .unwrap();
            assert!(lowered.commands.len() > 2);
            assert!(lowered
                .commands
                .iter()
                .all(|command| command.layer_id == StableId(91)));
            assert!(!lowered.commands.iter().any(|command| matches!(
                command.payload,
                SemanticCommandPayload::Composite { .. }
            )));
            assert!(lowered
                .commands
                .iter()
                .filter_map(|command| match &command.payload {
                    SemanticCommandPayload::Text { font_role, .. } => Some(font_role),
                    _ => None,
                })
                .all(|font| font == &FontRole::RegionFontId(1)));
        }
    }

    #[test]
    fn music_general_lowering_is_typed_localized_and_preserves_append_gradient() {
        let difficulty = crate::profile_scene::MusicDifficultySnapshot {
            clear: 10,
            full_combo: 8,
            all_perfect: 3,
        };
        let snapshot = crate::profile_scene::ProfileComponentSnapshot {
            locale: "ko-KR".into(),
            region_fonts: BTreeMap::from([(1, "KoreanRegionFont".into())]),
            localized_text: crate::locale::GENERAL_LOCALIZATION_KEYS
                .iter()
                .map(|key| ((*key).into(), crate::locale::resolve("ko-KR", key).unwrap()))
                .collect(),
            user_name: String::new(),
            word: String::new(),
            user_rank: 0,
            total_power: 0,
            mvp: 0,
            superstar: 0,
            challenge_score: 0,
            challenge_character_id: 0,
            challenge_avatar: None,
            music_results: Some(crate::profile_scene::MusicResultsSnapshot {
                easy: difficulty,
                normal: difficulty,
                hard: difficulty,
                expert: difficulty,
                master: difficulty,
                append: difficulty,
            }),
            story_favorites: Vec::new(),
            player_avatar: None,
            character_ranks: Vec::new(),
            deck_members: Vec::new(),
            leader_card: None,
            honor_slots: Vec::new(),
        };
        for general_type in [12, 16] {
            let lowered = crate::profile_scene::lower_identity_general(
                general_type,
                StableId(92),
                &format!("music-{general_type}"),
                &snapshot,
            )
            .unwrap()
            .unwrap();
            assert!(lowered.commands.len() >= 24);
            assert!(!lowered.commands.iter().any(|command| matches!(
                command.payload,
                SemanticCommandPayload::Composite { .. }
            )));
            assert!(lowered
                .commands
                .iter()
                .filter_map(|command| match &command.payload {
                    SemanticCommandPayload::Text { source, .. } => Some(source),
                    _ => None,
                })
                .all(|source| !matches!(source, TextSource::Authored { .. })));
            if general_type == 16 {
                assert!(lowered.commands.iter().any(|command| matches!(
                    command.payload,
                    SemanticCommandPayload::Shape {
                        gradient: Some(_),
                        ..
                    }
                )));
                assert!(matches!(
                    &lowered.controls[0].state,
                    crate::profile_scene::ComponentControlState::Tabs { options, active }
                        if options == &["clear", "full_combo", "all_perfect"] && active == "clear"
                ));
                assert_eq!(lowered.interaction_regions.len(), 21);
                assert_eq!(
                    lowered
                        .interaction_regions
                        .iter()
                        .filter(|region| region.capabilities.contains(&"activate".into()))
                        .count(),
                    3
                );
                assert_eq!(
                    lowered
                        .interaction_regions
                        .iter()
                        .filter(|region| region.capabilities.contains(&"select_text".into()))
                        .count(),
                    18
                );
                assert_eq!(
                    lowered
                        .commands
                        .iter()
                        .filter(|command| !command.control_bindings.is_empty())
                        .count(),
                    30
                );
            }
        }
    }

    #[test]
    fn image_general_lowering_exposes_source_metadata_cover_uv_and_clip() {
        let descriptor = crate::profile_scene::ResourceDescriptor {
            resource: ResourceKey {
                namespace: "assets".into(),
                key: "fixture/wide".into(),
            },
            natural_width: 360.0,
            natural_height: 180.0,
            provenance: BTreeMap::from([("table".into(), ParameterValue::Text("cards".into()))]),
        };
        let image = crate::profile_scene::ComponentImageSnapshot {
            source_field: "userProfile.leaderCard".into(),
            source_id: "42".into(),
            descriptor: Some(descriptor.clone()),
        };
        let snapshot = crate::profile_scene::ProfileComponentSnapshot {
            locale: "en-US".into(),
            region_fonts: BTreeMap::from([(1, "RegionFont".into())]),
            localized_text: crate::locale::GENERAL_LOCALIZATION_KEYS
                .iter()
                .map(|key| ((*key).into(), crate::locale::resolve("en-US", key).unwrap()))
                .collect(),
            user_name: String::new(),
            word: String::new(),
            user_rank: 0,
            total_power: 0,
            mvp: 0,
            superstar: 0,
            challenge_score: 0,
            challenge_character_id: 0,
            challenge_avatar: None,
            music_results: None,
            story_favorites: vec![crate::profile_scene::StoryFavoriteSnapshot {
                story_id: 7,
                story_type: "event".into(),
                image: crate::profile_scene::ComponentImageSnapshot {
                    source_field: "userProfile.storyFavorites".into(),
                    source_id: "event:7".into(),
                    descriptor: Some(descriptor.clone()),
                },
            }],
            player_avatar: Some(image),
            character_ranks: vec![crate::profile_scene::CharacterRankSnapshot {
                character_id: 21,
                rank: 50,
                challenge_rank: Some(12),
                avatar: crate::profile_scene::ComponentImageSnapshot {
                    source_field: "userProfile.characterRanks".into(),
                    source_id: "21".into(),
                    descriptor: Some(descriptor.clone()),
                },
            }],
            deck_members: vec![crate::profile_scene::CardVisualSnapshot {
                card_id: 42,
                after_training: true,
                master_rank: 5,
                level: 60,
                rarity: "rarity_4".into(),
                attribute: "cool".into(),
                image: crate::profile_scene::ComponentImageSnapshot {
                    source_field: "userProfile.deckMembers".into(),
                    source_id: "42".into(),
                    descriptor: Some(descriptor.clone()),
                },
            }],
            leader_card: Some(crate::profile_scene::CardVisualSnapshot {
                card_id: 42,
                after_training: true,
                master_rank: 5,
                level: 60,
                rarity: "rarity_4".into(),
                attribute: "cool".into(),
                image: crate::profile_scene::ComponentImageSnapshot {
                    source_field: "userProfile.leaderCard".into(),
                    source_id: "42".into(),
                    descriptor: Some(descriptor.clone()),
                },
            }),
            honor_slots: vec![
                crate::profile_scene::HonorVisualSnapshot {
                    source_field: "userProfile.honorSlots".into(),
                    source_id: "1".into(),
                    honor_id: 1,
                    honor_level: 6,
                    full_size: true,
                    visual: crate::profile_scene::HonorVisualKind::Standard {
                        honor_type: "character".into(),
                        has_star: true,
                        is_live_master: false,
                        progress: 0,
                        background: Some(descriptor.clone()),
                        frame_candidates: vec![Some(descriptor.clone())],
                        overlay: Some(descriptor.clone()),
                        star: Some(descriptor.clone()),
                        star_high: Some(descriptor.clone()),
                        live_star_on: Some(descriptor.clone()),
                        live_star_off: Some(descriptor.clone()),
                    },
                },
                crate::profile_scene::HonorVisualSnapshot {
                    source_field: "userProfile.honorSlots".into(),
                    source_id: "2".into(),
                    honor_id: 2,
                    honor_level: 3,
                    full_size: false,
                    visual: crate::profile_scene::HonorVisualKind::Bonds {
                        character_ids: [1, 2],
                        backgrounds: [Some(descriptor.clone()), Some(descriptor.clone())],
                        characters: [Some(descriptor.clone()), Some(descriptor.clone())],
                        mask: Some(descriptor.clone()),
                        frame: Some(descriptor.clone()),
                        word: Some(descriptor.clone()),
                        star: Some(descriptor.clone()),
                        star_high: Some(descriptor.clone()),
                    },
                },
            ],
        };
        let story =
            crate::profile_scene::lower_identity_general(14, StableId(93), "story", &snapshot)
                .unwrap()
                .unwrap();
        let avatar =
            crate::profile_scene::lower_identity_general(18, StableId(94), "avatar", &snapshot)
                .unwrap()
                .unwrap();
        assert!(!story
            .commands
            .iter()
            .any(|command| matches!(command.payload, SemanticCommandPayload::Composite { .. })));
        let avatar_image = avatar
            .commands
            .iter()
            .find(|command| command.role == "avatar")
            .unwrap();
        assert_eq!(
            avatar_image.metadata.get("source_field"),
            Some(&ParameterValue::Text("userProfile.leaderCard".into()))
        );
        assert!(matches!(
            &avatar_image.payload,
            SemanticCommandPayload::Image {
                uv: Rect { x, width, .. },
                clip: Some(crate::ImageClip::Ellipse),
                ..
            } if (*x - 0.25).abs() < 1e-6 && (*width - 0.5).abs() < 1e-6
        ));
        let ranks =
            crate::profile_scene::lower_identity_general(11, StableId(95), "ranks", &snapshot)
                .unwrap()
                .unwrap();
        assert!(ranks
            .commands
            .iter()
            .any(|command| command.role == "character-21-avatar"));
        assert!(ranks.commands.iter().any(|command| matches!(
            &command.payload,
            SemanticCommandPayload::Text { source: TextSource::ProfileField { field, value }, .. }
                if field == "userProfile.characterRanks.21.rank" && value == "50"
        )));
        assert!(ranks.commands.iter().any(|command| matches!(
            &command.payload,
            SemanticCommandPayload::Text { source: TextSource::ProfileField { field, value }, .. }
                if field == "userProfile.challengeLiveSoloStages.21.rank" && value == "12"
        )));
        assert_eq!(
            ranks
                .commands
                .iter()
                .filter(|command| !command.control_bindings.is_empty())
                .count(),
            8
        );
        assert!(matches!(
            &ranks.controls[0].state,
            crate::profile_scene::ComponentControlState::Tabs { options, active }
                if options == &["character_rank", "challenge_live_rank"] && active == "character_rank"
        ));
        assert_eq!(ranks.interaction_regions.len(), 5);
        assert_eq!(
            ranks
                .interaction_regions
                .iter()
                .filter(|region| region.capabilities.contains(&"select_text".into()))
                .count(),
            2
        );
        for general_type in [3, 5] {
            let cards = crate::profile_scene::lower_identity_general(
                general_type,
                StableId(98),
                &format!("cards-{general_type}"),
                &snapshot,
            )
            .unwrap()
            .unwrap();
            assert!(cards.commands.len() >= 5);
            assert!(cards.commands.iter().any(|command| matches!(
                &command.payload,
                SemanticCommandPayload::Image { resource, .. } if resource.namespace == "static"
            )));
            assert!(cards
                .interaction_regions
                .iter()
                .all(|region| region.capabilities.contains(&"select_item".into())));
            assert!(!cards.commands.iter().any(|command| matches!(
                command.payload,
                SemanticCommandPayload::Composite { .. }
            )));
        }
        let honors =
            crate::profile_scene::lower_identity_general(6, StableId(99), "honors", &snapshot)
                .unwrap()
                .unwrap();
        assert_eq!(honors.interaction_regions.len(), 2);
        assert!(honors.commands.iter().any(|command| {
            command.blend_mode == crate::BlendMode::DstIn
                && matches!(
                    &command.payload,
                    SemanticCommandPayload::Image { resource, .. } if resource.key == "fixture/wide"
                )
        }));
        assert!(honors
            .commands
            .iter()
            .any(|command| matches!(command.payload, SemanticCommandPayload::Composite { .. })));

        let compact = crate::profile_scene::lower_identity_general(
            15,
            StableId(96),
            "compact-ranks",
            &snapshot,
        )
        .unwrap()
        .unwrap();
        assert!(compact.controls.iter().any(|control| matches!(
            control.state,
            crate::profile_scene::ComponentControlState::Scroll { .. }
        )));
        assert!(compact.commands.iter().any(|command| command
            .control_bindings
            .iter()
            .any(|binding| matches!(binding, CommandControlBinding::ScrollContent { .. }))
            && command.clip.is_some()));
        let compact_background = compact
            .commands
            .iter()
            .find(|command| command.role == "character-21-background")
            .unwrap();
        assert!(compact_background.clip.is_some());
        let compact_avatar = compact
            .commands
            .iter()
            .find(|command| command.role == "character-21-avatar")
            .unwrap();
        assert!(compact_avatar.clip.is_some());
        assert!(matches!(
            compact_avatar.payload,
            SemanticCommandPayload::Image {
                clip: Some(crate::ImageClip::Ellipse),
                ..
            }
        ));
        assert!(compact
            .commands
            .iter()
            .find(|command| command.role == "character-21-character_rank")
            .unwrap()
            .clip
            .is_some());

        let mut scrolling_story_snapshot = snapshot.clone();
        let favorite = scrolling_story_snapshot.story_favorites[0].clone();
        scrolling_story_snapshot.story_favorites = (0..12)
            .map(|mut index| {
                let mut value = favorite.clone();
                index += 1;
                value.story_id = index;
                value.image.source_id = format!("event:{index}");
                value
            })
            .collect();
        let scrolling_story = crate::profile_scene::lower_identity_general(
            14,
            StableId(97),
            "scrolling-story",
            &scrolling_story_snapshot,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            scrolling_story
                .commands
                .iter()
                .filter(|command| command.role.starts_with("story-")
                    && !command.role.contains("scroll"))
                .count(),
            12
        );
        assert!(scrolling_story.controls.iter().any(|control| matches!(
            control.state,
            crate::profile_scene::ComponentControlState::Scroll { max, .. } if max > 0.0
        )));
        assert!(scrolling_story.interaction_regions.iter().any(|region| {
            region.resolved_data.get("action") == Some(&ParameterValue::Text("scroll_by".into()))
        }));
    }

    #[test]
    fn shared_profile_resolver_lowers_all_twelve_authored_types() {
        let object = serde_json::json!({
            "layer": 7, "lock": false,
            "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
        });
        let card: crate::profile_source::CustomProfileCard = serde_json::from_value(serde_json::json!({
            "texts": [{ "objectData": object, "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 2, "outlineSize": 1.0, "size": 24.0, "text": "A", "type": 1 }],
            "shapes": [{ "objectData": object, "alpha": 0.5, "colorId": 1, "id": 1, "outlineAlpha": 1.0, "outlineColorId": 2, "outlineSize": 2.0 }],
            "cardMembers": [{ "objectData": object, "id": 1 }], "stamps": [{ "objectData": object, "id": 1 }],
            "others": [{ "objectData": object, "id": 1 }],
            "bondsHonors": [{ "objectData": object, "id": 1, "wordId": 1, "fullSize": true, "inverse": false, "useUnitVirtualSinger": false }],
            "honors": [{ "objectData": object, "id": 1, "fullSize": true }],
            "collections": [{ "objectData": object, "id": 1 }], "generals": [{ "objectData": object, "type": 9 }],
            "standMembers": [{ "objectData": object, "id": 1 }], "generalBackgrounds": [{ "objectData": object, "id": 1 }],
            "storyBackgrounds": [{ "objectData": object, "id": 1 }]
        })).unwrap();
        let mut snapshot = crate::profile_scene::ProfileResolveSnapshot {
            fonts: BTreeMap::from([(1, "RegionFont".into())]),
            colors: BTreeMap::from([(1, [1.0, 0.0, 0.0, 1.0]), (2, [0.0, 0.0, 0.0, 1.0])]),
            resources: [
                ("shape", "", "shape/resolved", 1024.0, 1024.0),
                ("card-member", "2:normal", "card/resolved", 156.0, 156.0),
                ("stamp", "", "stamp/resolved", 100.0, 100.0),
                ("other", "", "other/resolved", 120.0, 80.0),
                ("collection", "", "collection/resolved", 100.0, 100.0),
                ("stand-member", "", "standing/resolved", 300.0, 700.0),
                (
                    "general-background",
                    "",
                    "general-bg/resolved",
                    1830.0,
                    812.0,
                ),
                ("story-background", "", "story-bg/resolved", 1830.0, 812.0),
            ]
            .into_iter()
            .map(|(kind, variant, key, natural_width, natural_height)| {
                (
                    crate::profile_scene::resource_lookup_key(kind, 1, variant),
                    crate::profile_scene::ResourceDescriptor {
                        resource: ResourceKey {
                            namespace: "assets".into(),
                            key: key.into(),
                        },
                        natural_width,
                        natural_height,
                        provenance: BTreeMap::from([(
                            "table".into(),
                            ParameterValue::Text(kind.into()),
                        )]),
                    },
                )
            })
            .collect(),
            ..Default::default()
        };
        let honor_asset = crate::profile_scene::ResourceDescriptor {
            resource: ResourceKey {
                namespace: "static".into(),
                key: "honor/fixture".into(),
            },
            natural_width: 380.0,
            natural_height: 80.0,
            provenance: BTreeMap::new(),
        };
        for element in crate::profile_scene::ordered_profile_elements(&card, "document") {
            let visual = match element.value {
                crate::profile_scene::ProfileElementRef::Honor(value) => {
                    Some(crate::profile_scene::HonorVisualSnapshot {
                        source_field: "customProfile.honors".into(),
                        source_id: value.id.to_string(),
                        honor_id: value.id,
                        honor_level: value.honor_level,
                        full_size: value.full_size,
                        visual: crate::profile_scene::HonorVisualKind::Standard {
                            honor_type: "achievement".into(),
                            has_star: true,
                            is_live_master: false,
                            progress: 0,
                            background: Some(honor_asset.clone()),
                            frame_candidates: vec![Some(honor_asset.clone())],
                            overlay: Some(honor_asset.clone()),
                            star: Some(honor_asset.clone()),
                            star_high: Some(honor_asset.clone()),
                            live_star_on: Some(honor_asset.clone()),
                            live_star_off: Some(honor_asset.clone()),
                        },
                    })
                }
                crate::profile_scene::ProfileElementRef::BondsHonor(value) => {
                    Some(crate::profile_scene::HonorVisualSnapshot {
                        source_field: "customProfile.bondsHonors".into(),
                        source_id: value.id.to_string(),
                        honor_id: value.id,
                        honor_level: value.honor_level,
                        full_size: value.full_size,
                        visual: crate::profile_scene::HonorVisualKind::Bonds {
                            character_ids: [1, 2],
                            backgrounds: [Some(honor_asset.clone()), Some(honor_asset.clone())],
                            characters: [Some(honor_asset.clone()), Some(honor_asset.clone())],
                            mask: Some(honor_asset.clone()),
                            frame: Some(honor_asset.clone()),
                            word: Some(honor_asset.clone()),
                            star: Some(honor_asset.clone()),
                            star_high: Some(honor_asset.clone()),
                        },
                    })
                }
                _ => None,
            };
            if let Some(visual) = visual {
                snapshot.honor_visuals.insert(element.source_key, visual);
            }
        }
        let resolved =
            crate::profile_scene::resolve_profile_scene(&card, "document", &snapshot).unwrap();
        assert_eq!(resolved.layers.len(), 12);
        assert!(resolved.commands.len() > 12);
        assert!(resolved.interaction_regions.len() > 12);
        assert!(resolved.layers.iter().all(|layer| layer.game_layer == 7));
        assert!(resolved.commands.iter().all(|command| resolved
            .layers
            .iter()
            .any(|layer| layer.id == command.layer_id)));
        assert!(resolved.commands.iter().any(|command| matches!(
            &command.payload,
            SemanticCommandPayload::Image { resource, .. } if resource.key == "other/resolved"
        )));
        for kind in [AuthoredElementKind::Honor, AuthoredElementKind::BondsHonor] {
            let layer = resolved
                .layers
                .iter()
                .find(|layer| layer.authored_kind == kind)
                .unwrap();
            assert!(resolved
                .commands
                .iter()
                .filter(|command| command.layer_id == layer.id)
                .all(|command| !matches!(
                    command.payload,
                    SemanticCommandPayload::Composite { .. }
                )));
        }
        let other_layer = resolved
            .layers
            .iter()
            .find(|layer| layer.authored_kind == AuthoredElementKind::Other)
            .unwrap();
        assert_eq!(other_layer.bounds.width, 120.0);
        assert_eq!(
            other_layer.resolved_parameters.get("resource_source.table"),
            Some(&ParameterValue::Text("other".into()))
        );
        let text = resolved
            .commands
            .iter()
            .find(|command| matches!(command.payload, SemanticCommandPayload::Text { .. }))
            .unwrap();
        assert!(matches!(
            &text.payload,
            SemanticCommandPayload::Text { font_role: FontRole::RegionFontId(1), color, outline_color, .. }
                if *color == [1.0, 0.0, 0.0, 1.0] && *outline_color == [0.0, 0.0, 0.0, 1.0]
        ));
    }

    #[test]
    fn semantic_command_for_unknown_layer_fails_closed() {
        let source = SceneSource {
            scene_id: StableId(42),
            region: "cn".into(),
            font_engine_fingerprint: "ft".into(),
            raster_contract: "sdf".into(),
            layers: vec![layer(1, false)],
            glyphs: Vec::new(),
            semantic_commands: vec![SemanticCommandSource::text(
                StableId(4201),
                StableId(99),
                "orphan",
                "text",
                FontRole::RegionFontId(1),
            )],
            interaction_regions: Vec::new(),
            component_controls: Vec::new(),
        };
        assert!(matches!(
            Scene::new(source),
            Err(CoreError::UnknownCommandLayer { .. })
        ));
    }

    #[test]
    fn warm_glyph_cache_does_not_regenerate() {
        let mut cache = SessionGlyphCache::default();
        let key = GlyphCacheKey {
            region: "cn".into(),
            font_digest: [7; 32],
            font_engine_fingerprint: "ft".into(),
            raster_contract: "sdf".into(),
            glyph_index: 12,
        };
        cache.get_or_insert_with(key.clone(), || GlyphCacheEntry {
            metrics: [0; 6],
            pixels: vec![1, 2, 3],
        });
        cache.get_or_insert_with(key, || panic!("warm glyph regenerated"));
        assert_eq!(
            cache.stats(),
            &GlyphCacheStats {
                hits: 1,
                misses: 1,
                generations: 1,
                bytes: 3
            }
        );
    }

    #[test]
    fn materialized_dynamic_matches_pre_core_production_formula() {
        for percent in [-50.0, 0.0, 25.0, 50.0, 99.0, 100.0] {
            let source = LineIndentSource {
                percent,
                line_advances_tmp: vec![vec![17.5, 31.25, 22.0, 19.75]],
                rotation_deg: 0.0,
                scale_x: 1.0,
            };
            let max_frames = if (0.0..100.0).contains(&percent) {
                20_000
            } else {
                1_800
            };
            let actual = materialize_line_indent(source.clone(), max_frames).unwrap();
            let expected = legacy_materialize(&source, max_frames);
            assert_eq!(actual.looped, expected.0, "percent={percent}");
            assert_eq!(actual.frames.len(), expected.1.len(), "percent={percent}");
            for (actual, expected) in actual.frames.iter().zip(expected.1) {
                assert!(
                    (actual.dx_local - expected).abs() <= 1e-5,
                    "percent={percent}"
                );
            }
        }
        let divergent = materialize_line_indent(
            LineIndentSource {
                percent: 150.0,
                line_advances_tmp: vec![vec![17.5, 31.25, 22.0, 19.75]],
                rotation_deg: 0.0,
                scale_x: 1.0,
            },
            1_800,
        )
        .unwrap();
        assert!(divergent
            .frames
            .iter()
            .all(|frame| frame.dx_local.is_finite()));
        assert!(divergent.frames.len() < 1_800);
    }

    #[test]
    fn multiline_line_indent_feedback_uses_global_preferred_width() {
        let multiline = materialize_line_indent(
            LineIndentSource {
                percent: 50.0,
                line_advances_tmp: vec![vec![10.0], vec![20.0, 20.0]],
                rotation_deg: 0.0,
                scale_x: 1.0,
            },
            512,
        )
        .unwrap();
        let widest_line = materialize_line_indent(
            LineIndentSource {
                percent: 50.0,
                line_advances_tmp: vec![vec![20.0, 20.0]],
                rotation_deg: 0.0,
                scale_x: 1.0,
            },
            512,
        )
        .unwrap();

        assert_eq!(multiline, widest_line);
    }

    #[test]
    fn rich_text_wrapping_inserts_breaks_without_splitting_markup() {
        let units = "ABCD"
            .chars()
            .map(|_| MeasuredTextUnit {
                advance: 60.0,
                hard_break: false,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            wrap_tmp_markup("<color=#ff0000>ABCD</color>", &units, 130.0).unwrap(),
            "<color=#ff0000>AB\nCD</color>"
        );
    }

    #[test]
    fn rich_text_wrapping_preserves_authored_newlines_and_fails_on_unit_drift() {
        let units = vec![
            MeasuredTextUnit {
                advance: 40.0,
                hard_break: false,
            },
            MeasuredTextUnit {
                advance: 0.0,
                hard_break: true,
            },
            MeasuredTextUnit {
                advance: 40.0,
                hard_break: false,
            },
        ];
        assert_eq!(wrap_tmp_markup("A\nB", &units, 50.0).unwrap(), "A\nB");
        assert!(wrap_tmp_markup("AB", &units, 50.0).is_err());
    }

    fn legacy_materialize(source: &LineIndentSource, max_frames: usize) -> (bool, Vec<f32>) {
        let pct = source.percent / 100.0;
        let natural = source.line_advances_tmp[0].iter().sum::<f32>();
        let last = *source.line_advances_tmp[0].last().unwrap();
        let static_width = if pct < 1.0 {
            (natural + TMP_PAD) / (1.0 - pct)
        } else {
            natural + TMP_PAD
        };
        let mut width = TMP_SEED_WIDTH;
        let mut frames = Vec::new();
        for frame in 0..(WARMUP_TICKS as usize + max_frames) {
            let x_advance = natural + pct * width;
            let preferred = if x_advance >= last {
                x_advance
            } else {
                2.0 * last - x_advance
            };
            if frame >= WARMUP_TICKS as usize {
                let local = (pct - 0.5) * (width - static_width);
                frames.push(local);
                if (0.0..1.0).contains(&pct)
                    && frames.len() >= 2
                    && local.abs() <= CONVERGENCE_EPSILON
                {
                    break;
                }
            }
            width = preferred + TMP_PAD;
        }
        if frames.len() >= 4
            && frames
                .iter()
                .take(32)
                .enumerate()
                .all(|(index, value)| (*value - frames[index % 2]).abs() <= TWO_CYCLE_EPSILON)
        {
            return (true, vec![frames[0], frames[1]]);
        }
        (false, frames)
    }
}
