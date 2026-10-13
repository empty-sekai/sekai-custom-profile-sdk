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

mod glyph_cache;
mod line_indent;
mod primitives;
mod scene;
mod scene_source;
mod scene_state;

pub use glyph_cache::{GlyphCacheEntry, GlyphCacheKey, GlyphCacheStats, SessionGlyphCache};
pub use line_indent::{
    materialize_line_indent, DynamicLayerState, DynamicStatus, LineIndentFrameLocal,
    LineIndentMaterialization, TimelineDescriptor, TransformDelta,
};
pub use primitives::{
    AuthoredElementKind, BlendMode, CommandId, CompositeOperation, FontRole, GlyphId, ImageClip,
    ImageMaterial, LayerId, LayerKind, LinearGradient, Matrix2d, ParameterValue, Quad, Rect,
    ResourceKey, SceneId, ShapePrimitive, StableId, TextSource,
};
pub use scene::Scene;
pub use scene_source::{
    CommandControlBinding, GlyphSource, InteractionRegionSource, LayerSource, LineIndentSource,
    SceneSource, SemanticCommandPayload, SemanticCommandSource, TextRenderPlacementSource,
};
pub use scene_state::{
    AnimationPreflight, CommandPatch, CommandState, CoreError, CoreTelemetry, DirtyKinds,
    GlyphCommand, InteractionRegionSnapshot, LayerCommand, LayerDump, LayerPatch, LayerState,
    LayerTableEntry, RenderMaskOverride, Revisions, SceneDelta, SceneDump, SceneSnapshot,
};
pub use tmp_text::{wrap_tmp_markup, MeasuredTextUnit};

pub const SCHEMA_MAJOR: u16 = 1;
pub const SCHEMA_MINOR: u16 = 17;
pub const TICKS_PER_SECOND: u32 = 60;
pub const TMP_PAD: f32 = 64.0;
pub const TMP_SEED_WIDTH: f32 = 8.46;
pub const WARMUP_TICKS: u64 = 3;
pub const CONVERGENCE_EPSILON: f32 = 0.05;
pub const STATIC_FINAL_ANALYSIS_TICKS: u64 = 20_000;
pub const TWO_CYCLE_EPSILON: f32 = 0.02;

#[cfg(test)]
mod tests;
