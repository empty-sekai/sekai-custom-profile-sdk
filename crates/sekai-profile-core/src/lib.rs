//! Backend-neutral renderer state and command contract shared by native and WASM.
//!
//! This crate holds everything about a custom profile card that does not depend
//! on how pixels are produced: the input model, the resolved snapshot, the
//! panel recipes and the layout constants. Both the native CPU backend and the
//! browser WebGL2 backend consume it, so a rule written here is the single
//! source of truth for both rather than something each backend re-derives.
//!
//! # Pipeline
//!
//! Game payload ([`profile_source`]) is lowered into a resolved snapshot
//! ([`profile_scene`]) by [`profile_resolve`], and each panel is then turned
//! into an ordered node list by [`general_recipe`]. Nodes carry layout
//! coordinates and resource keys; turning them into draw calls is the backend's
//! job.
//!
//! # Modules
//!
//! | Module | Responsibility |
//! | --- | --- |
//! | [`profile_source`] | Card and element shapes as the game API sends them |
//! | [`profile_scene`] | Resolved snapshot a recipe builder reads |
//! | [`profile_resolve`] | Lowering from source to snapshot |
//! | [`general_recipe`] | Per-panel node lists, backend-neutral |
//! | [`profile_layout`] | Measured panel geometry |
//! | [`profile_transform`] | Transform and matrix helpers shared by backends |
//! | [`masterdata`] | Masterdata lookups the resolve step needs |
//! | [`locale`] | Region-specific strings for fixed panel labels |
//! | [`tmp_text`] | TextMesh Pro markup, line breaking and glyph choice |
//! | [`sdf_geometry`] | Path segments and their analytic distance field |
//! | [`sdf_glyph`] | Glyph SDF rasterization shared by the native and browser backends |
//! | [`sdf_material`] | Text and shape SDF material parameters |
//! | [`badge_material`] | Lit material of can-badge collection images |
//! | [`pixel_sampling`] | Where a device pixel is sampled, and what a shape or image covers there |
//! | [`ugui_text`] | uGUI text layout drawn from FreeType bitmaps |
//! | `ugui_freetype` | FreeType glyphs for [`ugui_text`] (feature `freetype`) |
//! | [`omikuji`] | The fortune slip of omikuji collections |
//! | [`authoring_document`] | Editable document model |
//! | [`authoring_session`] | Editing session state and command handling |

pub mod authoring_document;
pub mod authoring_session;
pub mod badge_material;
pub mod general_recipe;
pub mod locale;
pub mod masterdata;
pub mod omikuji;
pub mod pixel_sampling;
pub mod profile_data;
pub mod profile_layout;
pub mod profile_resolve;
pub mod profile_scene;
pub mod profile_source;
pub mod profile_transform;
pub mod sdf_geometry;
pub mod sdf_glyph;
pub mod sdf_material;
pub mod tmp_text;
#[cfg(feature = "freetype")]
pub mod ugui_freetype;
pub mod ugui_text;

pub use tmp_text::{wrap_tmp_markup, MeasuredTextUnit};

use std::collections::{BTreeMap, BTreeSet, HashMap};

use bincode::{Decode, Encode};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const SCHEMA_MAJOR: u16 = 1;
pub const SCHEMA_MINOR: u16 = 17;
pub const TICKS_PER_SECOND: u32 = 60;
pub const TMP_PAD: f32 = 64.0;
pub const TMP_SEED_WIDTH: f32 = 8.46;
pub const WARMUP_TICKS: u64 = 3;
pub const CONVERGENCE_EPSILON: f32 = 0.05;
pub const STATIC_FINAL_ANALYSIS_TICKS: u64 = 20_000;
pub const TWO_CYCLE_EPSILON: f32 = 0.02;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Encode, Decode)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

pub type Matrix2d = [f32; 6];
pub type Quad = [[f32; 2]; 4];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ParameterValue {
    Bool(bool),
    I64(i64),
    F64(f64),
    Text(String),
    Vec2([f32; 2]),
    Color([f32; 4]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    Text,
    Image,
    Shape,
    Composite,
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    Deserialize,
    Encode,
    Decode,
)]
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
    CharacterIcon,
    Material,
    UserInterfaceIcon,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum FontRole {
    RegionFontId(i32),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Encode, Decode)]
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct ResourceKey {
    pub namespace: String,
    pub key: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Encode, Decode)]
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum CompositeOperation {
    #[default]
    Marker,
    BeginIsolation,
    EndIsolation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum ShapePrimitive {
    Rect,
    RoundedRect { radius: [f32; 2] },
    Ellipse,
    AssetMask { resource: ResourceKey },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct LinearGradient {
    /// Normalized shape-space coordinates.
    pub start: [f32; 2],
    pub end: [f32; 2],
    pub start_color: [f32; 4],
    pub end_color: [f32; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum ImageClip {
    RoundedRect { radius: [f32; 2] },
    Ellipse,
}

/// How an image command shades its texels.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImageMaterial {
    /// The texel colour multiplied by the tint.
    #[default]
    Plain,
    /// The lit material of [`badge_material`], with `normal_map` sampled at
    /// the image's UV. The tint's alpha is the element alpha; the material
    /// applies no tint colour.
    LitBadge { normal_map: ResourceKey },
}

impl ImageMaterial {
    pub fn is_plain(&self) -> bool {
        matches!(self, Self::Plain)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandControlBinding {
    TabOption { control_id: StableId, value: String },
    ScrollContent { control_id: StableId },
    ScrollThumb { control_id: StableId },
    ScrollViewport { control_id: StableId },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
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
        /// Omitted from JSON when [`ImageMaterial::Plain`].
        #[serde(default, skip_serializing_if = "ImageMaterial::is_plain")]
        material: ImageMaterial,
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
    /// A uGUI text node drawn from FreeType bitmaps; see [`ugui_text`]. Its
    /// geometry is laid out by the renderer, which owns the font file.
    UguiText(ugui_text::UguiTextSource),
}

/// Backend-neutral post-layout placement. The TMP layout output remains
/// unchanged; backends translate completed glyph geometry to this anchor.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct TextRenderPlacementSource {
    pub anchor_x: f32,
    pub baseline: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
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
    #[bincode(with_serde)]
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
                material: ImageMaterial::Plain,
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

    /// A uGUI text node whose node rect covers `bounds` in the layer.
    pub fn ugui_text(
        id: CommandId,
        layer_id: LayerId,
        role: impl Into<String>,
        bounds: Rect,
        source: ugui_text::UguiTextSource,
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
            payload: SemanticCommandPayload::UguiText(source),
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct LineIndentSource {
    pub percent: f32,
    pub line_advances_tmp: Vec<Vec<f32>>,
    pub rotation_deg: f32,
    pub scale_x: f32,
    /// Horizontal alignment of the text, encoded like the text command's
    /// `alignment`: 2 centres, 4 right-aligns, and any other value lays the
    /// lines out from the left edge.
    #[serde(default)]
    pub alignment: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
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
    sample_count: u64,
    /// The most recent samples, oldest first, at most [`LINE_INDENT_SAMPLE_WINDOW`].
    samples: Vec<f32>,
    state: DynamicLayerState,
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

    fn reset(&mut self) {
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
mod tests;
