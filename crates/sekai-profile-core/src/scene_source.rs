//! The input a scene is built from: semantic commands with their payloads and
//! control bindings, interaction regions, and layer, glyph and line-indent sources.

use std::collections::BTreeMap;

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{
    profile_scene, tmp_text, ugui_text, AuthoredElementKind, BlendMode, CommandId,
    CompositeOperation, FontRole, GlyphId, ImageClip, ImageMaterial, LayerId, LayerKind,
    LinearGradient, Matrix2d, ParameterValue, Quad, Rect, ResourceKey, SceneId, ShapePrimitive,
    StableId, TextSource,
};

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
