//! Backend-neutral ordered tile plan, scalar oracle and Turin-class SIMD executor.
//!
//! Layout produces [`SdfDrawCommand`] values once. The scalar oracle and the
//! AVX-512 executor consume the same plan, so optimization cannot silently
//! change glyph geometry, material order, or atlas addressing.

mod atlas_source;
mod command;
mod execute;
mod plan;
mod shape_program;
#[cfg(target_arch = "x86_64")]
mod simd_x86;

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use lru::LruCache;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::shape::ShapeSdfTexel;

pub(crate) use self::atlas_source::RuntimeTextSdfPage;
pub use self::atlas_source::{
    MixedSdfAtlasSource, SdfAtlasSource, SdfAtlasTexel, SdfSwizzledFormat, SdfSwizzledPage,
};
pub use self::command::{
    Affine2, Point2, SdfCommandBuildError, SdfCommandMaterial, SdfDeviceClip, SdfDrawCommand,
    SdfMaterial, SdfPrimitiveKind,
};

#[cfg(test)]
use self::execute::{quantize, shade_text, source_over_rgba8};
use self::shape_program::*;

pub const DEFAULT_TILE_WIDTH: u16 = 64;
pub const DEFAULT_TILE_HEIGHT: u16 = 16;
pub const MIXED_SDF_ATLAS_CONTRACT: &str = "allium.mixed-sdf-atlas.v1(text-r8-set-v1,shape-rg8-v2)";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SdfAccumulationMode {
    /// Quantize the premultiplied tile after every ordered draw, as an RGBA8
    /// surface stores it.
    #[default]
    Rgba8Writeback,
    /// Keep the ordered tile in FP32 until final output. This is the retained
    /// throughput candidate used by the native AVX-512 executor.
    F32Tile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SdfDestination {
    Clear([u8; 4]),
    LoadExisting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TileGrid {
    pub canvas_width: u32,
    pub canvas_height: u32,
    pub tile_width: u16,
    pub tile_height: u16,
}

impl TileGrid {
    pub const fn new(canvas_width: u32, canvas_height: u32) -> Self {
        Self {
            canvas_width,
            canvas_height,
            tile_width: DEFAULT_TILE_WIDTH,
            tile_height: DEFAULT_TILE_HEIGHT,
        }
    }

    fn tiles_x(self) -> u32 {
        self.canvas_width.div_ceil(u32::from(self.tile_width))
    }

    fn tiles_y(self) -> u32 {
        self.canvas_height.div_ceil(u32::from(self.tile_height))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TileSpan {
    pub command: u32,
    pub row: u16,
    pub x0: u16,
    pub x1: u16,
}

/// Row-major 1-bit destination-pixel mask used by the exact occlusion
/// experiment. A set bit means a later SrcOver draw is proven to produce
/// alpha 255 at that destination pixel.
#[derive(Clone, Debug)]
pub struct PixelOcclusionMask {
    width: u32,
    height: u32,
    words_per_row: usize,
    words: Vec<u64>,
}

impl PixelOcclusionMask {
    pub fn new(width: u32, height: u32) -> Result<Self, SdfTileError> {
        let words_per_row =
            usize::try_from(width.div_ceil(u64::BITS)).map_err(|_| SdfTileError::SizeOverflow)?;
        let word_count = words_per_row
            .checked_mul(usize::try_from(height).map_err(|_| SdfTileError::SizeOverflow)?)
            .ok_or(SdfTileError::SizeOverflow)?;
        Ok(Self {
            width,
            height,
            words_per_row,
            words: vec![0; word_count],
        })
    }

    pub fn union_opaque_rgba8(&mut self, pixels: &[u8]) -> Result<u64, SdfTileError> {
        let pixel_count = usize::try_from(self.width)
            .ok()
            .and_then(|width| {
                usize::try_from(self.height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(SdfTileError::SizeOverflow)?;
        if pixels.len()
            != pixel_count
                .checked_mul(4)
                .ok_or(SdfTileError::SizeOverflow)?
        {
            return Err(SdfTileError::OutputLength {
                expected: pixel_count.saturating_mul(4),
                actual: pixels.len(),
            });
        }

        let mut newly_opaque = 0u64;
        for (index, pixel) in pixels.chunks_exact(4).enumerate() {
            if pixel[3] != u8::MAX {
                continue;
            }
            let y = index / self.width as usize;
            let x = index - y * self.width as usize;
            let word = y * self.words_per_row + x / u64::BITS as usize;
            let bit = 1u64 << (x % u64::BITS as usize);
            newly_opaque = newly_opaque.saturating_add(u64::from(self.words[word] & bit == 0));
            self.words[word] |= bit;
        }
        Ok(newly_opaque)
    }

    pub fn resident_bytes(&self) -> u64 {
        self.words.len().saturating_mul(std::mem::size_of::<u64>()) as u64
    }

    fn count_opaque_span(&self, y: u32, x0: u32, x1: u32) -> u64 {
        debug_assert!(y < self.height);
        debug_assert!(x0 <= x1 && x1 <= self.width);
        if x0 == x1 {
            return 0;
        }
        let row = y as usize * self.words_per_row;
        let first_word = x0 as usize / u64::BITS as usize;
        let last_word = (x1 - 1) as usize / u64::BITS as usize;
        if first_word == last_word {
            let left = u64::MAX << (x0 % u64::BITS);
            let right = u64::MAX >> ((u64::BITS - (x1 % u64::BITS)) % u64::BITS);
            return u64::from((self.words[row + first_word] & left & right).count_ones());
        }

        let mut count =
            u64::from((self.words[row + first_word] & (u64::MAX << (x0 % u64::BITS))).count_ones());
        for word in first_word + 1..last_word {
            count = count.saturating_add(u64::from(self.words[row + word].count_ones()));
        }
        let tail_bits = x1 % u64::BITS;
        let tail_mask = if tail_bits == 0 {
            u64::MAX
        } else {
            u64::MAX >> (u64::BITS - tail_bits)
        };
        count.saturating_add(u64::from(
            (self.words[row + last_word] & tail_mask).count_ones(),
        ))
    }

    fn is_opaque(&self, x: u32, y: u32) -> bool {
        debug_assert!(x < self.width && y < self.height);
        let word = y as usize * self.words_per_row + x as usize / u64::BITS as usize;
        self.words[word] & (1u64 << (x % u64::BITS)) != 0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SdfOcclusionStats {
    pub occluded_fragment_count: u64,
    pub visible_fragment_count: u64,
    pub occluded_text_fragment_count: u64,
    pub occluded_shape_fragment_count: u64,
    pub fully_occluded_command_count: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SdfPlanStats {
    pub command_count: u64,
    pub text_command_count: u64,
    pub shape_command_count: u64,
    pub span_count: u64,
    pub text_span_count: u64,
    pub shape_span_count: u64,
    pub tile_count: u64,
    pub nonempty_tile_count: u64,
    pub covered_fragment_count: u64,
    pub text_covered_fragment_count: u64,
    pub shape_covered_fragment_count: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SdfExecutionStats {
    pub shaded_fragment_count: u64,
    pub text_shaded_fragment_count: u64,
    pub shape_shaded_fragment_count: u64,
    pub sampled_texel_count: u64,
    pub blended_fragment_count: u64,
    pub text_blended_fragment_count: u64,
    pub shape_blended_fragment_count: u64,
    pub simd_packet_count: u64,
    pub swizzled_packet_count: u64,
    pub gather_fallback_packet_count: u64,
    pub precomputed_shape_fragment_count: u64,
    pub precomputed_shape_span_count: u64,
    pub direct_output_run_count: u64,
    pub direct_output_packet_count: u64,
}

#[derive(Clone, Copy, Debug)]
struct PlannedCommand {
    source: SdfDrawCommand,
    tx_dx: f32,
    tx_dy: f32,
    tx_c: f32,
    ty_dx: f32,
    ty_dy: f32,
    ty_c: f32,
    shape_face_offset: f32,
    shape_outline_offset: f32,
    shape_coverage_scale: f32,
}

#[derive(Clone, Copy, Debug)]
struct ShapeCoverageRun {
    x0: u32,
    x1: u32,
    face_coverage: f32,
    outline_coverage: f32,
}

#[derive(Clone, Copy, Debug)]
struct ShapeTexelRun {
    x0: u32,
    x1: u32,
    texel: ShapeSdfTexel,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct DirectAxisShapeSpan {
    pub(super) y: u32,
    pub(super) x0: u32,
    pub(super) x1: u32,
    pub(super) source: [f32; 4],
}

#[derive(Debug)]
struct DirectAxisShapePlan {
    spans: Vec<DirectAxisShapeSpan>,
}

#[derive(Debug)]
struct AxisAlignedShapeProgram {
    rect: [u32; 4],
    row_offsets: Vec<u32>,
    runs: Vec<ShapeCoverageRun>,
}

#[derive(Debug)]
struct ShapeSourceProgram {
    rect: [u32; 4],
    row_offsets: Vec<u32>,
    runs: Vec<ShapeTexelRun>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ShapeSourceProgramKey {
    atlas_page: u16,
    atlas_rect: [u32; 4],
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ShapeRowProgramKey {
    atlas_page: u16,
    atlas_rect: [u32; 4],
    coverage: [u32; 3],
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ShapeRowProgramPrewarmReport {
    pub(crate) program_count: u64,
    pub(crate) run_count: u64,
    pub(crate) resident_bytes: u64,
}

pub(crate) struct ShapeRowProgramCache {
    source_programs: Mutex<HashMap<ShapeSourceProgramKey, Arc<ShapeSourceProgram>>>,
    programs: Mutex<LruCache<ShapeRowProgramKey, Arc<AxisAlignedShapeProgram>>>,
}

impl Default for ShapeRowProgramCache {
    fn default() -> Self {
        Self {
            source_programs: Mutex::new(HashMap::new()),
            programs: Mutex::new(LruCache::new(
                NonZeroUsize::new(256).expect("non-zero Shape row-program cache capacity"),
            )),
        }
    }
}

#[derive(Debug)]
pub struct SdfTilePlan {
    grid: TileGrid,
    commands: Vec<PlannedCommand>,
    axis_shape_programs: Vec<Option<Arc<AxisAlignedShapeProgram>>>,
    direct_axis_shape: Option<DirectAxisShapePlan>,
    spans: Vec<TileSpan>,
    tile_offsets: Vec<u32>,
    stats: SdfPlanStats,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum SdfTileError {
    #[error("canvas and tile dimensions must be non-zero")]
    InvalidGrid,
    #[error("canvas, plan or output size overflow")]
    SizeOverflow,
    #[error("more than u32::MAX commands are not supported")]
    TooManyCommands,
    #[error("more than u32::MAX spans are not supported")]
    TooManySpans,
    #[error("more than u16::MAX atlas sets are not supported")]
    TooManyAtlasSets,
    #[error("command {command} references missing atlas set {atlas_set} page {page}")]
    MissingAtlasPage {
        command: u32,
        atlas_set: u16,
        page: u16,
    },
    #[error("command {command} has a non-finite field")]
    NonFinite { command: u32 },
    #[error("command {command} primitive kind and material kind disagree")]
    MaterialKindMismatch { command: u32 },
    #[error("command {command} has invalid atlas rect")]
    InvalidAtlasRect { command: u32 },
    #[error("command {command} has a degenerate quad")]
    DegenerateQuad { command: u32 },
    #[error("command {command} is not an affine parallelogram")]
    NonAffineQuad { command: u32 },
    #[error("output length mismatch: expected {expected}, got {actual}")]
    OutputLength { expected: usize, actual: usize },
    #[error("atlas lookup failed for set {atlas_set} page {page} at ({x}, {y})")]
    AtlasLookup {
        atlas_set: u16,
        page: u16,
        x: u32,
        y: u32,
    },
    #[error("atlas texel format does not match command {kind:?}")]
    AtlasFormatMismatch { kind: SdfPrimitiveKind },
    #[error("tile plan is internally inconsistent")]
    CorruptPlan,
    #[error("Turin SIMD executor requires x86-64 AVX-512F/FMA/BW/VBMI")]
    SimdUnavailable,
    #[error("SIMD executor cannot access physical swizzled atlas set {atlas_set} page {page}")]
    SimdAtlasUnavailable { atlas_set: u16, page: u16 },
}

#[cfg(test)]
mod tests;
