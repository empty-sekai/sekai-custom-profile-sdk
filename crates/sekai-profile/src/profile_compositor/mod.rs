//! Ordered software composition for compiled custom-profile image commands.
//!
//! The scalar path is the correctness reference for the packet executor. It
//! consumes mmap-backed premultiplied RGBA8 objects directly and deliberately
//! fails closed on command features whose pixel contract is not implemented.

use std::collections::BTreeSet;
use std::time::Instant;

use sekai_profile_core::{
    pixel_sampling, BlendMode, LinearGradient, Matrix2d, Rect, ShapePrimitive, StableId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::assets::AssetStore;
use crate::masterdata::MasterData;
use crate::render_object::{MappedRenderObject, MappedRenderObjectStore, RenderObjectEntry};

mod blend;
mod cache_variants;
/// Reusable image and shape drawing on the same compositor.
pub mod canvas;
mod commands;
mod raster;
mod scene;
mod semantic_text;
#[cfg(target_arch = "x86_64")]
mod simd_x86;
pub(crate) mod theme;
mod ugui_text;

pub use self::cache_variants::{
    build_deck_art_variant_simd, build_general_base_objects_simd, deck_art_variant_identity,
    deck_art_variant_identity_from_source,
};
pub use self::scene::{
    authored_image_is_exact_opaque_mask_source, render_authored_image_into_simd,
    render_authored_profile_into_scalar, render_authored_profile_into_simd,
    render_image_scene_scalar, render_image_scene_simd, visible_scene_device_bounds,
};

use self::blend::*;
use self::cache_variants::*;
use self::commands::*;
use self::raster::*;
use self::semantic_text::*;

pub const PROFILE_COMPOSITOR_SCHEMA: &str = "allium.profile-compositor.v1";
pub const GENERAL_BASE_CONTRACT: &str =
    "allium.general-base.sdk-6b0dae58.rgba8-premul-canonical-type11.v4";
pub const DECK_ART_VARIANT_CONTRACT: &str =
    "allium.deck-art-variant.sdk-6b0dae58.crop312x512.slot148x243.v1";

/// Pixel size of a baked deck art variant.
const DECK_ART_VARIANT_WIDTH: u32 = 149;
const DECK_ART_VARIANT_HEIGHT: u32 = 243;
/// Deck slot a variant is baked for, in layer units; the variant holds it at
/// one unit per pixel, so its last column is only partially covered.
const DECK_ART_SLOT_WIDTH: f32 = 148.078_13;
const DECK_ART_SLOT_HEIGHT: f32 = 243.0;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileCompositorStats {
    pub text_command_count: u64,
    pub shape_command_count: u64,
    pub image_command_count: u64,
    pub composite_command_count: u64,
    pub skipped_text_command_count: u64,
    pub skipped_shape_command_count: u64,
    pub sampled_fragment_count: u64,
    pub blended_fragment_count: u64,
    pub simd_packet_count: u64,
    pub scalar_fragment_count: u64,
    pub isolation_count: u64,
    pub maximum_isolation_depth: u64,
    pub scratch_peak_bytes: u64,
    pub general_base_hit_count: u64,
    pub general_base_miss_count: u64,
    pub general_base_baked_command_count: u64,
    pub general_base_overlay_command_count: u64,
    pub general_base_bytes: u64,
    pub general_base_avoided_source_bytes: u64,
    pub general_base_composite_ns: u64,
    pub deck_art_variant_hit_count: u64,
    pub deck_art_variant_miss_count: u64,
    pub deck_art_variant_bytes: u64,
    pub deck_art_variant_avoided_source_bytes: u64,
    pub source_fallback_object_count: u64,
    pub source_fallback_bytes: u64,
    pub output_bytes: u64,
    pub render_ns: u64,
}

#[derive(Clone, Debug)]
pub struct GeneralBaseBuildOutput {
    pub object_key: String,
    pub source_sha256: String,
    pub general_type: i32,
    pub group: String,
    pub bounds: Rect,
    pub width: u32,
    pub height: u32,
    pub row_bytes: u32,
    pub pixels: Vec<u8>,
    pub baked_command_count: u64,
    pub overlay_command_count: u64,
    pub avoided_source_bytes: u64,
    pub stats: ProfileCompositorStats,
}

#[derive(Clone, Debug)]
pub struct DeckArtVariantBuildOutput {
    pub object_key: String,
    pub source_sha256: String,
    pub width: u32,
    pub height: u32,
    pub row_bytes: u32,
    pub pixels: Vec<u8>,
    pub source_bytes: u64,
    pub stats: ProfileCompositorStats,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileCompositorOutput {
    pub schema: String,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub stats: ProfileCompositorStats,
}

#[derive(Debug, Error)]
pub enum ProfileCompositorError {
    #[error("profile compositor canvas dimensions are invalid: {width}x{height}")]
    InvalidCanvas { width: u32, height: u32 },
    #[error("profile compositor surface of {width}x{height} needs {expected} bytes, got {actual}")]
    SurfaceLength {
        width: u32,
        height: u32,
        expected: usize,
        actual: usize,
    },
    #[error("profile compositor scene contains duplicate layer {0}")]
    DuplicateLayer(String),
    #[error("profile compositor command references missing layer {0}")]
    MissingLayer(String),
    #[error("profile compositor is missing render object {0}")]
    MissingObject(String),
    #[error("profile compositor render object {0} has an invalid pixel layout")]
    InvalidObject(String),
    #[error("profile compositor command {role} has unsupported feature {feature}")]
    UnsupportedFeature { role: String, feature: String },
    #[error("profile compositor command {role} has invalid geometry")]
    InvalidGeometry { role: String },
    #[error("profile compositor isolation stack underflow at command {0}")]
    IsolationUnderflow(String),
    #[error("profile compositor finished with {0} unclosed isolation layer(s)")]
    UnclosedIsolation(usize),
    #[error("profile compositor semantic SDF command {role} failed: {reason}")]
    SemanticSdf { role: String, reason: String },
    #[error("profile compositor command {role} needs font {family}, which is not installed")]
    MissingFont { role: String, family: String },
    #[error("profile compositor command {role} could not render its glyphs: {reason}")]
    GlyphRaster { role: String, reason: String },
}

/// A tight, row-major premultiplied RGBA8 surface the compositor draws into.
///
/// It holds `width * height` pixels of four bytes each, with rows `width * 4`
/// bytes apart. [`RgbaSurface::new`] checks that length, so a surface always
/// covers its full extent.
pub struct RgbaSurface<'a> {
    pub(crate) pixels: &'a mut [u8],
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl<'a> RgbaSurface<'a> {
    /// Views `pixels` as a `width` by `height` surface.
    ///
    /// Fails when either dimension is zero or `pixels` is not exactly
    /// `width * height * 4` bytes long.
    pub fn new(
        pixels: &'a mut [u8],
        width: u32,
        height: u32,
    ) -> Result<Self, ProfileCompositorError> {
        let expected = canvas_bytes(width, height)?;
        if pixels.len() != expected {
            return Err(ProfileCompositorError::SurfaceLength {
                width,
                height,
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            pixels,
            width,
            height,
        })
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Borrows the surface for one draw, keeping `self` usable once that draw
    /// is done.
    pub(crate) fn reborrow(&mut self) -> RgbaSurface<'_> {
        RgbaSurface {
            pixels: self.pixels,
            width: self.width,
            height: self.height,
        }
    }
}

/// Read-only resources that scene commands draw from.
#[derive(Clone, Copy)]
pub struct CompositorSources<'a> {
    /// Baked render objects that image commands sample.
    pub store: &'a MappedRenderObjectStore,
    /// SDF text resources. Without them, SDF text commands are skipped and no
    /// baked General base applies.
    pub text: Option<SemanticSdfContext<'a>>,
    /// Asset store that supplies the source image of an object `store` does
    /// not hold.
    pub assets: Option<&'a AssetStore>,
}

impl<'a> CompositorSources<'a> {
    /// Sources with only the render-object store: no text resources and no
    /// source images.
    pub fn new(store: &'a MappedRenderObjectStore) -> Self {
        Self {
            store,
            text: None,
            assets: None,
        }
    }
}

struct CompositionTarget {
    pixels: Vec<u8>,
    bounds: CompositionBounds,
}

/// Half-open device pixel rectangle: columns `x0..x1` of rows `y0..y1`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompositionBounds {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ImageExecutor {
    Scalar,
    Simd,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RasterImageStats {
    fragments: u64,
    simd_packets: u64,
    scalar_fragments: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AxisAlignedClip {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ImageClipGeometry {
    RoundedRect { radius: [f32; 2] },
    Ellipse,
}

/// Inputs of the lit badge material for one image command.
#[derive(Clone, Copy)]
struct BadgeShading<'a> {
    normal_map: MappedRenderObject<'a>,
    tangent: [f32; 2],
}

/// One image command resolved for rasterization: the texels it samples, where
/// they land on the canvas and how they blend.
#[derive(Clone, Copy)]
struct ImageDraw<'a> {
    /// Command role, named in errors.
    role: &'a str,
    /// Premultiplied texels, sampled nearest-neighbour.
    source: MappedRenderObject<'a>,
    /// Local rectangle the image covers.
    bounds: Rect,
    /// Normalised region of `source` mapped onto `bounds`.
    uv: Rect,
    /// Vertex colour multiplied into every texel.
    tint: [f32; 4],
    /// Local-to-canvas transform.
    matrix: Matrix2d,
    blend_mode: BlendMode,
    /// Canvas-space clip rectangle.
    command_clip: Option<AxisAlignedClip>,
    /// Clip shape inscribed in `bounds`.
    image_clip: Option<ImageClipGeometry>,
    /// Mask whose alpha scales the source across `bounds`.
    alpha_mask: Option<MappedRenderObject<'a>>,
    badge: Option<BadgeShading<'a>>,
}

impl<'a> ImageDraw<'a> {
    /// An untinted source-over draw of `uv` of `source` onto `bounds`, with no
    /// clip, mask or material.
    fn new(
        role: &'a str,
        source: MappedRenderObject<'a>,
        bounds: Rect,
        uv: Rect,
        matrix: Matrix2d,
    ) -> Self {
        Self {
            role,
            source,
            bounds,
            uv,
            tint: [1.0; 4],
            matrix,
            blend_mode: BlendMode::SrcOver,
            command_clip: None,
            image_clip: None,
            alpha_mask: None,
            badge: None,
        }
    }
}

/// The primitive and paint of a shape command.
#[derive(Clone, Copy)]
struct SemanticShape<'a> {
    primitive: &'a ShapePrimitive,
    fill: [f32; 4],
    /// Replaces `fill` when present.
    gradient: Option<&'a LinearGradient>,
    stroke: [f32; 4],
    /// Width of the stroke band inside the outline; `0` draws no stroke.
    stroke_width: f32,
}

struct OwnedRenderObject {
    entry: RenderObjectEntry,
    pixels: Vec<u8>,
}

impl OwnedRenderObject {
    fn mapped(&self) -> MappedRenderObject<'_> {
        MappedRenderObject {
            entry: &self.entry,
            pixels: &self.pixels,
        }
    }
}

/// SDF text atlases with the master data that resolves each text command's
/// font to an atlas family.
#[derive(Clone, Copy)]
pub struct SemanticSdfContext<'a> {
    /// SDF atlases of the text font families.
    pub text_atlases: &'a crate::sdf::atlas::MappedSdfAtlasSet,
    /// Master data that maps a text command's font id to its family.
    pub md: &'a MasterData,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct EvaluatedCommandControlState {
    visible: bool,
    translate_y: f32,
}

#[derive(Clone, Debug)]
struct GeneralBaseTilePlan {
    group: String,
    object_key: String,
    source_sha256: String,
    bounds: Rect,
    command_ids: BTreeSet<StableId>,
    avoided_source_bytes: u64,
}

#[derive(Clone, Debug)]
struct GeneralBasePlan {
    layer_id: StableId,
    general_type: i32,
    tiles: Vec<GeneralBaseTilePlan>,
    overlay_command_ids: BTreeSet<StableId>,
}

enum GeneralBaseAttempt {
    NotEligible,
    Miss,
    Hit(Box<ProfileCompositorStats>),
}

fn axis_aligned_command_clip(
    clip: &sekai_profile_core::Quad,
    matrix: Matrix2d,
) -> Result<AxisAlignedClip, ()> {
    let corners = clip.map(|point| {
        let (x, y) = transform_point(matrix, point[0], point[1]);
        [x, y]
    });
    let [min_x, min_y, max_x, max_y] =
        pixel_sampling::axis_aligned_clip_bounds(corners).ok_or(())?;
    Ok(AxisAlignedClip {
        min_x,
        min_y,
        max_x,
        max_y,
    })
}

fn unsupported<T>(role: &str, feature: &str) -> Result<T, ProfileCompositorError> {
    Err(ProfileCompositorError::UnsupportedFeature {
        role: role.into(),
        feature: feature.into(),
    })
}

fn compose_matrix(parent: Matrix2d, child: Matrix2d) -> Matrix2d {
    [
        parent[0] * child[0] + parent[2] * child[1],
        parent[1] * child[0] + parent[3] * child[1],
        parent[0] * child[2] + parent[2] * child[3],
        parent[1] * child[2] + parent[3] * child[3],
        parent[0] * child[4] + parent[2] * child[5] + parent[4],
        parent[1] * child[4] + parent[3] * child[5] + parent[5],
    ]
}

fn invert_matrix(matrix: Matrix2d) -> Option<Matrix2d> {
    let determinant = matrix[0] * matrix[3] - matrix[1] * matrix[2];
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return None;
    }
    Some([
        matrix[3] / determinant,
        -matrix[1] / determinant,
        -matrix[2] / determinant,
        matrix[0] / determinant,
        (matrix[2] * matrix[5] - matrix[3] * matrix[4]) / determinant,
        (matrix[1] * matrix[4] - matrix[0] * matrix[5]) / determinant,
    ])
}

fn transform_point(matrix: Matrix2d, x: f32, y: f32) -> (f32, f32) {
    (
        matrix[0].mul_add(x, matrix[2].mul_add(y, matrix[4])),
        matrix[1].mul_add(x, matrix[3].mul_add(y, matrix[5])),
    )
}

fn canvas_bytes(width: u32, height: u32) -> Result<usize, ProfileCompositorError> {
    if width == 0 || height == 0 {
        return Err(ProfileCompositorError::InvalidCanvas { width, height });
    }
    usize::try_from(width)
        .ok()
        .and_then(|value| value.checked_mul(height as usize))
        .and_then(|value| value.checked_mul(4))
        .ok_or(ProfileCompositorError::InvalidCanvas { width, height })
}

fn quantize(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod software_tests;
#[cfg(test)]
mod tests;
