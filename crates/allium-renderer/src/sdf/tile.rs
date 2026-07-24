//! Backend-neutral ordered tile plan, scalar oracle and Turin-class SIMD executor.
//!
//! Layout produces [`SdfDrawCommand`] values once. The scalar oracle and the
//! AVX-512 executor consume the same plan, so optimization cannot silently
//! change glyph geometry, material order, or atlas addressing.

#[cfg(target_arch = "x86_64")]
mod simd_x86;

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use lru::LruCache;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::atlas::{MappedSdfAtlas, MappedSdfAtlasSet, SdfAtlasGlyphManifest};
use super::shape::{
    shade_shape, shade_shape_coverages, texel_coverage, ShapeSdfMaterial, ShapeSdfTexel,
};
use super::shape_atlas::MappedShapeSdfAtlas;

pub const DEFAULT_TILE_WIDTH: u16 = 64;
pub const DEFAULT_TILE_HEIGHT: u16 = 16;
pub const MIXED_SDF_ATLAS_CONTRACT: &str = "allium.mixed-sdf-atlas.v1(text-r8-set-v1,shape-rg8-v2)";
const MAX_PARALLELOGRAM_ERROR: f32 = 0.01;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SdfPrimitiveKind {
    Text,
    Shape,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SdfAccumulationMode {
    /// Quantize the premultiplied tile after every ordered draw, matching the
    /// current Skia RGBA8 raster surface as closely as possible.
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point2 {
    pub x: f32,
    pub y: f32,
}

impl Point2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine2 {
    pub scale_x: f32,
    pub skew_x: f32,
    pub translate_x: f32,
    pub skew_y: f32,
    pub scale_y: f32,
    pub translate_y: f32,
}

impl Affine2 {
    pub const IDENTITY: Self = Self {
        scale_x: 1.0,
        skew_x: 0.0,
        translate_x: 0.0,
        skew_y: 0.0,
        scale_y: 1.0,
        translate_y: 0.0,
    };

    pub fn map_point(self, point: Point2) -> Point2 {
        Point2::new(
            self.skew_x
                .mul_add(point.y, self.scale_x.mul_add(point.x, self.translate_x)),
            self.scale_y
                .mul_add(point.y, self.skew_y.mul_add(point.x, self.translate_y)),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SdfMaterial {
    /// Premultiplied RGBA face color.
    pub face: [f32; 4],
    /// Premultiplied RGBA outline color.
    pub outline: [f32; 4],
    pub face_scale: f32,
    pub face_bias: f32,
    pub outline_scale: f32,
    pub outline_bias: f32,
    pub vertex_alpha: f32,
}

/// Axis-aligned half-open clip in final device coordinates. Geometry and
/// inverse atlas mapping remain unchanged; the planner only emits fragments
/// whose pixel centers lie inside this rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SdfDeviceClip {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SdfCommandMaterial {
    Text(SdfMaterial),
    Shape(ShapeSdfMaterial),
}

impl Default for SdfMaterial {
    fn default() -> Self {
        Self {
            face: [1.0; 4],
            outline: [0.0; 4],
            face_scale: 1.0,
            face_bias: 0.0,
            outline_scale: 1.0,
            outline_bias: 0.0,
            vertex_alpha: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SdfDrawCommand {
    pub kind: SdfPrimitiveKind,
    pub atlas_set: u16,
    pub atlas_page: u16,
    /// Linear atlas texel coordinates `[x, y, width, height]`.
    pub atlas_rect: [u32; 4],
    /// Device-space `[top-left, top-right, bottom-right, bottom-left]`.
    pub quad: [Point2; 4],
    #[serde(default)]
    pub device_clip: Option<SdfDeviceClip>,
    pub material: SdfCommandMaterial,
}

impl SdfDrawCommand {
    /// Builds the same padded glyph plane used by the existing Skia SDF path,
    /// then maps it to device space through the already-resolved layout matrix.
    pub fn from_atlas_glyph(
        kind: SdfPrimitiveKind,
        atlas_set: u16,
        glyph: &SdfAtlasGlyphManifest,
        atlas_point_size: f32,
        atlas_spread: f32,
        baseline_origin: Point2,
        font_size: f32,
        local_to_device: Affine2,
        material: SdfMaterial,
    ) -> Result<Self, SdfCommandBuildError> {
        let finite = [
            atlas_point_size,
            atlas_spread,
            baseline_origin.x,
            baseline_origin.y,
            font_size,
            local_to_device.scale_x,
            local_to_device.skew_x,
            local_to_device.translate_x,
            local_to_device.skew_y,
            local_to_device.scale_y,
            local_to_device.translate_y,
        ]
        .into_iter()
        .chain(glyph.plane_bearing)
        .chain(glyph.plane_size)
        .all(f32::is_finite);
        if !finite {
            return Err(SdfCommandBuildError::NonFinite);
        }
        if atlas_point_size <= 0.0
            || atlas_spread < 0.0
            || font_size <= 0.0
            || glyph.rect[2] == 0
            || glyph.rect[3] == 0
            || glyph.plane_size[0] <= 0.0
            || glyph.plane_size[1] <= 0.0
        {
            return Err(SdfCommandBuildError::InvalidMetrics);
        }
        let logical_scale = font_size / atlas_point_size;
        let left =
            (glyph.plane_bearing[0] - atlas_spread).mul_add(logical_scale, baseline_origin.x);
        let top =
            (-(glyph.plane_bearing[1] + atlas_spread)).mul_add(logical_scale, baseline_origin.y);
        let width = (glyph.plane_size[0] + atlas_spread * 2.0) * logical_scale;
        let height = (glyph.plane_size[1] + atlas_spread * 2.0) * logical_scale;
        let local_quad = [
            Point2::new(left, top),
            Point2::new(left + width, top),
            Point2::new(left + width, top + height),
            Point2::new(left, top + height),
        ];
        Ok(Self {
            kind,
            atlas_set,
            atlas_page: glyph.page,
            atlas_rect: glyph.rect,
            quad: local_quad.map(|point| local_to_device.map_point(point)),
            device_clip: None,
            material: SdfCommandMaterial::Text(material),
        })
    }

    pub fn from_shape_atlas(
        atlas_set: u16,
        shape: &super::shape_atlas::ShapeSdfAtlasEntry,
        quad: [Point2; 4],
        material: ShapeSdfMaterial,
    ) -> Result<Self, SdfCommandBuildError> {
        if shape.rect[2] == 0 || shape.rect[3] == 0 {
            return Err(SdfCommandBuildError::InvalidMetrics);
        }
        if quad
            .iter()
            .flat_map(|point| [point.x, point.y])
            .chain(material.face)
            .chain(material.outline)
            .chain([
                material.face_threshold,
                material.outline_threshold,
                material.sharpness,
            ])
            .any(|value| !value.is_finite())
        {
            return Err(SdfCommandBuildError::NonFinite);
        }
        if material.sharpness <= 0.0 {
            return Err(SdfCommandBuildError::InvalidMetrics);
        }
        Ok(Self {
            kind: SdfPrimitiveKind::Shape,
            atlas_set,
            atlas_page: shape.page,
            atlas_rect: shape.rect,
            quad,
            device_clip: None,
            material: SdfCommandMaterial::Shape(material),
        })
    }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum SdfCommandBuildError {
    #[error("glyph placement contains a non-finite value")]
    NonFinite,
    #[error("glyph placement has invalid atlas or font metrics")]
    InvalidMetrics,
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

pub const HIGHWAY_SDF_ABI_VERSION: u32 = 1;
pub const HIGHWAY_SDF_KIND_TEXT: u32 = 1;
pub const HIGHWAY_SDF_KIND_SHAPE: u32 = 2;

/// Stable command record consumed by the experimental C++/Highway executor.
/// The Rust planner remains authoritative; native executors receive only this
/// resolved inverse-affine/material form and cannot redo layout or geometry.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HighwaySdfCommand {
    pub kind: u32,
    pub atlas_set: u16,
    pub atlas_page: u16,
    pub atlas_rect: [u32; 4],
    pub inverse_affine: [f32; 6],
    pub face: [f32; 4],
    pub outline: [f32; 4],
    /// Text: face scale/bias, outline scale/bias, vertex alpha, reserved×3.
    /// Shape: face threshold, outline threshold, sharpness, reserved×5.
    pub params: [f32; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HighwaySdfSpan {
    pub command: u32,
    pub row: u16,
    pub x0: u16,
    pub x1: u16,
    pub reserved: u16,
}

#[derive(Debug)]
pub struct HighwaySdfPlan {
    pub abi_version: u32,
    pub grid: TileGrid,
    pub commands: Vec<HighwaySdfCommand>,
    pub spans: Vec<HighwaySdfSpan>,
    pub tile_offsets: Vec<u32>,
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

impl SdfTilePlan {
    pub fn build(
        grid: TileGrid,
        commands: &[SdfDrawCommand],
        atlas: &impl SdfAtlasSource,
    ) -> Result<Self, SdfTileError> {
        Self::build_impl(grid, commands, atlas, None, true, true)
    }

    pub(crate) fn build_with_shape_program_cache(
        grid: TileGrid,
        commands: &[SdfDrawCommand],
        atlas: &impl SdfAtlasSource,
        cache: &ShapeRowProgramCache,
    ) -> Result<Self, SdfTileError> {
        Self::build_impl(grid, commands, atlas, Some(cache), true, true)
    }

    /// A static profile page also executes each run once. Reuse cached Shape
    /// row programs, but do not eagerly expand a single Shape into a second
    /// destination-sized span buffer before that one execution.
    pub(crate) fn build_static_one_shot_with_shape_program_cache(
        grid: TileGrid,
        commands: &[SdfDrawCommand],
        atlas: &impl SdfAtlasSource,
        cache: &ShapeRowProgramCache,
    ) -> Result<Self, SdfTileError> {
        Self::build_impl(grid, commands, atlas, Some(cache), true, false)
    }

    /// Dynamic layer rasters execute each plan only once before the cropped
    /// layer is cached. Avoid both the Shape row-program build and its eager
    /// destination-span expansion when their one-shot preparation costs more
    /// than sampling the layer directly.
    pub(crate) fn build_for_one_shot_dynamic_layer(
        grid: TileGrid,
        commands: &[SdfDrawCommand],
        atlas: &impl SdfAtlasSource,
    ) -> Result<Self, SdfTileError> {
        Self::build_impl(grid, commands, atlas, None, false, false)
    }

    fn build_impl(
        grid: TileGrid,
        commands: &[SdfDrawCommand],
        atlas: &impl SdfAtlasSource,
        shape_program_cache: Option<&ShapeRowProgramCache>,
        build_shape_programs: bool,
        prepare_direct_axis_shape: bool,
    ) -> Result<Self, SdfTileError> {
        validate_grid(grid)?;
        let tile_count_u32 = grid
            .tiles_x()
            .checked_mul(grid.tiles_y())
            .ok_or(SdfTileError::SizeOverflow)?;
        let tile_count = usize::try_from(tile_count_u32).map_err(|_| SdfTileError::SizeOverflow)?;
        let mut per_tile = vec![Vec::new(); tile_count];
        let mut planned = Vec::with_capacity(commands.len());
        let mut axis_shape_programs = Vec::with_capacity(commands.len());
        let mut stats = SdfPlanStats {
            command_count: commands.len() as u64,
            tile_count: tile_count as u64,
            ..SdfPlanStats::default()
        };

        for (command_index, command) in commands.iter().copied().enumerate() {
            let command_u32 =
                u32::try_from(command_index).map_err(|_| SdfTileError::TooManyCommands)?;
            let page_dimensions = atlas
                .page_dimensions(command.atlas_set, command.atlas_page)
                .ok_or(SdfTileError::MissingAtlasPage {
                    command: command_u32,
                    atlas_set: command.atlas_set,
                    page: command.atlas_page,
                })?;
            validate_command(command_u32, command, page_dimensions)?;
            let affine = plan_command(command_u32, command)?;
            let shape_program = if build_shape_programs {
                match shape_program_cache {
                    Some(cache) => cache.get_or_build(&affine, atlas)?,
                    None => {
                        build_axis_aligned_shape_program_from_atlas(&affine, atlas)?.map(Arc::new)
                    }
                }
            } else {
                None
            };
            match command.kind {
                SdfPrimitiveKind::Text => stats.text_command_count += 1,
                SdfPrimitiveKind::Shape => stats.shape_command_count += 1,
            }
            scan_command(
                grid,
                command_u32,
                command.kind,
                command.quad,
                command.device_clip,
                &mut per_tile,
                &mut stats,
            )?;
            planned.push(affine);
            axis_shape_programs.push(shape_program);
        }

        let mut spans = Vec::with_capacity(
            usize::try_from(stats.span_count).map_err(|_| SdfTileError::SizeOverflow)?,
        );
        let mut tile_offsets = Vec::with_capacity(tile_count + 1);
        tile_offsets.push(0);
        for tile in per_tile {
            stats.nonempty_tile_count += u64::from(!tile.is_empty());
            spans.extend(tile);
            tile_offsets.push(u32::try_from(spans.len()).map_err(|_| SdfTileError::TooManySpans)?);
        }

        let plan = Self {
            grid,
            commands: planned,
            axis_shape_programs,
            direct_axis_shape: None,
            spans,
            tile_offsets,
            stats,
        };
        if prepare_direct_axis_shape {
            plan.prepare_direct_axis_shape()
        } else {
            Ok(plan)
        }
    }

    pub const fn grid(&self) -> TileGrid {
        self.grid
    }

    pub fn commands(&self) -> impl ExactSizeIterator<Item = &SdfDrawCommand> {
        self.commands.iter().map(|command| &command.source)
    }

    pub fn spans_for_tile(&self, tile: u32) -> Option<&[TileSpan]> {
        let index = usize::try_from(tile).ok()?;
        let begin = usize::try_from(*self.tile_offsets.get(index)?).ok()?;
        let end = usize::try_from(*self.tile_offsets.get(index + 1)?).ok()?;
        self.spans.get(begin..end)
    }

    pub const fn stats(&self) -> SdfPlanStats {
        self.stats
    }

    pub fn resident_bytes(&self) -> u64 {
        std::mem::size_of::<Self>()
            .saturating_add(
                self.commands
                    .len()
                    .saturating_mul(std::mem::size_of::<PlannedCommand>()),
            )
            .saturating_add(
                self.axis_shape_programs
                    .iter()
                    .flatten()
                    .map(|program| program.resident_bytes())
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(self.direct_axis_shape.as_ref().map_or(0, |plan| {
                plan.spans
                    .len()
                    .saturating_mul(std::mem::size_of::<DirectAxisShapeSpan>())
            }))
            .saturating_add(
                self.spans
                    .len()
                    .saturating_mul(std::mem::size_of::<TileSpan>()),
            )
            .saturating_add(
                self.tile_offsets
                    .len()
                    .saturating_mul(std::mem::size_of::<u32>()),
            ) as u64
    }

    pub fn span_bytes(&self) -> u64 {
        self.spans
            .len()
            .saturating_mul(std::mem::size_of::<TileSpan>()) as u64
    }

    /// Counts work that a later proven-opaque pixel mask can eliminate. This
    /// is measurement-only: neither the plan nor output pixels are changed.
    pub fn measure_occlusion(
        &self,
        mask: &PixelOcclusionMask,
    ) -> Result<SdfOcclusionStats, SdfTileError> {
        if mask.width != self.grid.canvas_width || mask.height != self.grid.canvas_height {
            return Err(SdfTileError::OutputLength {
                expected: self.grid.canvas_width as usize * self.grid.canvas_height as usize,
                actual: mask.width as usize * mask.height as usize,
            });
        }
        let mut stats = SdfOcclusionStats::default();
        let mut covered_by_command = vec![0u64; self.commands.len()];
        let mut occluded_by_command = vec![0u64; self.commands.len()];
        let tiles_x = self.grid.tiles_x();
        for tile in 0..self.grid.tiles_x().saturating_mul(self.grid.tiles_y()) {
            let tile_x = tile % tiles_x;
            let tile_y = tile / tiles_x;
            let origin_x = tile_x * u32::from(self.grid.tile_width);
            let origin_y = tile_y * u32::from(self.grid.tile_height);
            for span in self.spans_for_tile(tile).unwrap_or_default() {
                let command_index = span.command as usize;
                let covered = u64::from(span.x1 - span.x0);
                let occluded = mask.count_opaque_span(
                    origin_y + u32::from(span.row),
                    origin_x + u32::from(span.x0),
                    origin_x + u32::from(span.x1),
                );
                covered_by_command[command_index] =
                    covered_by_command[command_index].saturating_add(covered);
                occluded_by_command[command_index] =
                    occluded_by_command[command_index].saturating_add(occluded);
                stats.occluded_fragment_count =
                    stats.occluded_fragment_count.saturating_add(occluded);
                match self.commands[command_index].source.kind {
                    SdfPrimitiveKind::Text => {
                        stats.occluded_text_fragment_count =
                            stats.occluded_text_fragment_count.saturating_add(occluded);
                    }
                    SdfPrimitiveKind::Shape => {
                        stats.occluded_shape_fragment_count =
                            stats.occluded_shape_fragment_count.saturating_add(occluded);
                    }
                }
            }
        }
        stats.visible_fragment_count = self
            .stats
            .covered_fragment_count
            .saturating_sub(stats.occluded_fragment_count);
        stats.fully_occluded_command_count = covered_by_command
            .iter()
            .zip(occluded_by_command.iter())
            .filter(|(covered, occluded)| **covered != 0 && covered == occluded)
            .count() as u64;
        Ok(stats)
    }

    /// Builds an execution plan containing only pixels not proven opaque by a
    /// later draw. The returned statistics are relative to the original plan.
    /// Command/material order is unchanged; individual spans may be split.
    pub fn visible_plan(
        &self,
        mask: &PixelOcclusionMask,
    ) -> Result<(Self, SdfOcclusionStats), SdfTileError> {
        if mask.width != self.grid.canvas_width || mask.height != self.grid.canvas_height {
            return Err(SdfTileError::OutputLength {
                expected: self.grid.canvas_width as usize * self.grid.canvas_height as usize,
                actual: mask.width as usize * mask.height as usize,
            });
        }
        let tile_count = self.grid.tiles_x().saturating_mul(self.grid.tiles_y());
        let mut spans = Vec::with_capacity(self.spans.len());
        let mut tile_offsets = Vec::with_capacity(tile_count as usize + 1);
        let mut stats = SdfPlanStats {
            command_count: self.stats.command_count,
            text_command_count: self.stats.text_command_count,
            shape_command_count: self.stats.shape_command_count,
            tile_count: self.stats.tile_count,
            ..SdfPlanStats::default()
        };
        let mut original_by_command = vec![0u64; self.commands.len()];
        let mut visible_by_command = vec![0u64; self.commands.len()];
        tile_offsets.push(0);
        let tiles_x = self.grid.tiles_x();

        for tile in 0..tile_count {
            let before = spans.len();
            let tile_x = tile % tiles_x;
            let tile_y = tile / tiles_x;
            let origin_x = tile_x * u32::from(self.grid.tile_width);
            let origin_y = tile_y * u32::from(self.grid.tile_height);
            for span in self.spans_for_tile(tile).unwrap_or_default() {
                let command_index = span.command as usize;
                original_by_command[command_index] =
                    original_by_command[command_index].saturating_add(u64::from(span.x1 - span.x0));
                let y = origin_y + u32::from(span.row);
                let mut x = u32::from(span.x0);
                let end = u32::from(span.x1);
                while x < end {
                    while x < end && mask.is_opaque(origin_x + x, y) {
                        x += 1;
                    }
                    let run_start = x;
                    while x < end && !mask.is_opaque(origin_x + x, y) {
                        x += 1;
                    }
                    if run_start == x {
                        continue;
                    }
                    let covered = u64::from(x - run_start);
                    spans.push(TileSpan {
                        command: span.command,
                        row: span.row,
                        x0: u16::try_from(run_start).map_err(|_| SdfTileError::SizeOverflow)?,
                        x1: u16::try_from(x).map_err(|_| SdfTileError::SizeOverflow)?,
                    });
                    visible_by_command[command_index] =
                        visible_by_command[command_index].saturating_add(covered);
                    stats.span_count = stats.span_count.saturating_add(1);
                    stats.covered_fragment_count =
                        stats.covered_fragment_count.saturating_add(covered);
                    match self.commands[command_index].source.kind {
                        SdfPrimitiveKind::Text => {
                            stats.text_span_count = stats.text_span_count.saturating_add(1);
                            stats.text_covered_fragment_count =
                                stats.text_covered_fragment_count.saturating_add(covered);
                        }
                        SdfPrimitiveKind::Shape => {
                            stats.shape_span_count = stats.shape_span_count.saturating_add(1);
                            stats.shape_covered_fragment_count =
                                stats.shape_covered_fragment_count.saturating_add(covered);
                        }
                    }
                }
            }
            stats.nonempty_tile_count = stats
                .nonempty_tile_count
                .saturating_add(u64::from(spans.len() != before));
            tile_offsets.push(u32::try_from(spans.len()).map_err(|_| SdfTileError::TooManySpans)?);
        }

        let occlusion = SdfOcclusionStats {
            occluded_fragment_count: self
                .stats
                .covered_fragment_count
                .saturating_sub(stats.covered_fragment_count),
            visible_fragment_count: stats.covered_fragment_count,
            occluded_text_fragment_count: self
                .stats
                .text_covered_fragment_count
                .saturating_sub(stats.text_covered_fragment_count),
            occluded_shape_fragment_count: self
                .stats
                .shape_covered_fragment_count
                .saturating_sub(stats.shape_covered_fragment_count),
            fully_occluded_command_count: original_by_command
                .iter()
                .zip(visible_by_command.iter())
                .filter(|(original, visible)| **original != 0 && **visible == 0)
                .count() as u64,
        };
        Ok((
            Self {
                grid: self.grid,
                commands: self.commands.clone(),
                axis_shape_programs: self.axis_shape_programs.clone(),
                direct_axis_shape: None,
                spans,
                tile_offsets,
                stats,
            }
            .prepare_direct_axis_shape()?,
            occlusion,
        ))
    }

    fn axis_shape_program(&self, command: usize) -> Option<&AxisAlignedShapeProgram> {
        self.axis_shape_programs.get(command)?.as_deref()
    }

    pub(super) fn direct_axis_shape_spans(&self) -> Option<&[DirectAxisShapeSpan]> {
        Some(&self.direct_axis_shape.as_ref()?.spans)
    }

    fn prepare_direct_axis_shape(mut self) -> Result<Self, SdfTileError> {
        if self.commands.len() != 1 || self.axis_shape_program(0).is_none() {
            return Ok(self);
        }
        let command = self.commands[0];
        let program = self
            .axis_shape_program(0)
            .ok_or(SdfTileError::CorruptPlan)?;
        let mut rows = (0..self.grid.canvas_height)
            .map(|_| Vec::<DirectAxisShapeSpan>::new())
            .collect::<Vec<_>>();
        let tiles_x = self.grid.tiles_x();
        let tile_count =
            u32::try_from(self.tile_offsets.len() - 1).map_err(|_| SdfTileError::SizeOverflow)?;
        for tile_index in 0..tile_count {
            let tile_x = tile_index % tiles_x;
            let tile_y = tile_index / tiles_x;
            let origin_x = tile_x * u32::from(self.grid.tile_width);
            let origin_y = tile_y * u32::from(self.grid.tile_height);
            for span in self
                .spans_for_tile(tile_index)
                .ok_or(SdfTileError::CorruptPlan)?
            {
                if span.command != 0 {
                    return Err(SdfTileError::CorruptPlan);
                }
                let y = origin_y + u32::from(span.row);
                let row = rows
                    .get_mut(usize::try_from(y).map_err(|_| SdfTileError::SizeOverflow)?)
                    .ok_or(SdfTileError::CorruptPlan)?;
                for_each_axis_shape_span(
                    &command,
                    program,
                    y,
                    origin_x + u32::from(span.x0),
                    origin_x + u32::from(span.x1),
                    |x0, x1, source| row.push(DirectAxisShapeSpan { y, x0, x1, source }),
                )?;
            }
        }

        let mut spans = Vec::<DirectAxisShapeSpan>::new();
        for row in &mut rows {
            row.sort_unstable_by_key(|span| span.x0);
            for span in row.drain(..) {
                if let Some(previous) = spans.last_mut() {
                    if previous.y == span.y
                        && previous.x1 == span.x0
                        && rgba_f32_bits_eq(previous.source, span.source)
                    {
                        previous.x1 = span.x1;
                        continue;
                    }
                    if previous.y == span.y && previous.x1 > span.x0 {
                        return Err(SdfTileError::CorruptPlan);
                    }
                }
                spans.push(span);
            }
        }
        self.direct_axis_shape = Some(DirectAxisShapePlan { spans });
        Ok(self)
    }

    pub fn export_highway_abi(&self) -> HighwaySdfPlan {
        let commands = self
            .commands
            .iter()
            .map(|command| {
                let (kind, face, outline, params) = match command.source.material {
                    SdfCommandMaterial::Text(material) => (
                        HIGHWAY_SDF_KIND_TEXT,
                        material.face,
                        material.outline,
                        [
                            material.face_scale,
                            material.face_bias,
                            material.outline_scale,
                            material.outline_bias,
                            material.vertex_alpha,
                            0.0,
                            0.0,
                            0.0,
                        ],
                    ),
                    SdfCommandMaterial::Shape(material) => (
                        HIGHWAY_SDF_KIND_SHAPE,
                        material.face,
                        material.outline,
                        [
                            material.face_threshold,
                            material.outline_threshold,
                            material.sharpness,
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                            0.0,
                        ],
                    ),
                };
                HighwaySdfCommand {
                    kind,
                    atlas_set: command.source.atlas_set,
                    atlas_page: command.source.atlas_page,
                    atlas_rect: command.source.atlas_rect,
                    inverse_affine: [
                        command.tx_dx,
                        command.tx_dy,
                        command.tx_c,
                        command.ty_dx,
                        command.ty_dy,
                        command.ty_c,
                    ],
                    face,
                    outline,
                    params,
                }
            })
            .collect();
        let spans = self
            .spans
            .iter()
            .map(|span| HighwaySdfSpan {
                command: span.command,
                row: span.row,
                x0: span.x0,
                x1: span.x1,
                reserved: 0,
            })
            .collect();
        HighwaySdfPlan {
            abi_version: HIGHWAY_SDF_ABI_VERSION,
            grid: self.grid,
            commands,
            spans,
            tile_offsets: self.tile_offsets.clone(),
        }
    }

    /// Executes the reference FP32 sampler/shader into premultiplied RGBA8.
    /// `clear` must itself be premultiplied.
    pub fn execute_scalar(
        &self,
        atlas: &impl SdfAtlasSource,
        clear: [u8; 4],
        output: &mut [u8],
    ) -> Result<SdfExecutionStats, SdfTileError> {
        self.execute_scalar_with_mode(
            atlas,
            SdfDestination::Clear(clear),
            output,
            SdfAccumulationMode::Rgba8Writeback,
            false,
        )
    }

    /// Executes the scalar reference with the same FP32 tile accumulation
    /// contract used by [`Self::execute_simd`].
    pub fn execute_scalar_f32(
        &self,
        atlas: &impl SdfAtlasSource,
        clear: [u8; 4],
        output: &mut [u8],
    ) -> Result<SdfExecutionStats, SdfTileError> {
        self.execute_scalar_with_mode(
            atlas,
            SdfDestination::Clear(clear),
            output,
            SdfAccumulationMode::F32Tile,
            false,
        )
    }

    /// Blends this ordered plan over an existing premultiplied RGBA8 target.
    /// This is the compositor contract used by SDF runs interleaved with
    /// images and other legacy elements; untouched tiles remain byte-identical.
    pub fn execute_scalar_f32_over(
        &self,
        atlas: &impl SdfAtlasSource,
        output: &mut [u8],
    ) -> Result<SdfExecutionStats, SdfTileError> {
        self.execute_scalar_with_mode(
            atlas,
            SdfDestination::LoadExisting,
            output,
            SdfAccumulationMode::F32Tile,
            false,
        )
    }

    /// Portable oracle for the axis-aligned Shape row-program fast path.
    /// Text and non-axis-aligned Shape commands retain the regular sampler.
    pub fn execute_scalar_f32_over_precomputed_shapes(
        &self,
        atlas: &impl SdfAtlasSource,
        output: &mut [u8],
    ) -> Result<SdfExecutionStats, SdfTileError> {
        self.execute_scalar_with_mode(
            atlas,
            SdfDestination::LoadExisting,
            output,
            SdfAccumulationMode::F32Tile,
            true,
        )
    }

    fn execute_scalar_with_mode(
        &self,
        atlas: &impl SdfAtlasSource,
        destination: SdfDestination,
        output: &mut [u8],
        accumulation: SdfAccumulationMode,
        use_precomputed_shapes: bool,
    ) -> Result<SdfExecutionStats, SdfTileError> {
        let pixel_count = usize::try_from(self.grid.canvas_width)
            .ok()
            .and_then(|width| {
                usize::try_from(self.grid.canvas_height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(SdfTileError::SizeOverflow)?;
        let expected = pixel_count
            .checked_mul(4)
            .ok_or(SdfTileError::SizeOverflow)?;
        if output.len() != expected {
            return Err(SdfTileError::OutputLength {
                expected,
                actual: output.len(),
            });
        }
        if let SdfDestination::Clear(clear) = destination {
            for pixel in output.chunks_exact_mut(4) {
                pixel.copy_from_slice(&clear);
            }
        }

        let tile_width = usize::from(self.grid.tile_width);
        let tile_height = usize::from(self.grid.tile_height);
        let tile_pixels = tile_width
            .checked_mul(tile_height)
            .ok_or(SdfTileError::SizeOverflow)?;
        let mut tile = vec![[0.0f32; 4]; tile_pixels];
        let clear_f32 = match destination {
            SdfDestination::Clear(clear) => clear.map(|channel| f32::from(channel) / 255.0),
            SdfDestination::LoadExisting => [0.0; 4],
        };
        let mut stats = SdfExecutionStats::default();
        let tiles_x = self.grid.tiles_x();

        for tile_index in
            0..u32::try_from(self.tile_offsets.len() - 1).map_err(|_| SdfTileError::SizeOverflow)?
        {
            let spans = self
                .spans_for_tile(tile_index)
                .ok_or(SdfTileError::CorruptPlan)?;
            if spans.is_empty() {
                continue;
            }
            let tile_x = tile_index % tiles_x;
            let tile_y = tile_index / tiles_x;
            let origin_x = tile_x * u32::from(self.grid.tile_width);
            let origin_y = tile_y * u32::from(self.grid.tile_height);
            let valid_width =
                (self.grid.canvas_width - origin_x).min(u32::from(self.grid.tile_width));
            let valid_height =
                (self.grid.canvas_height - origin_y).min(u32::from(self.grid.tile_height));
            match destination {
                SdfDestination::Clear(_) => tile.fill(clear_f32),
                SdfDestination::LoadExisting => {
                    tile.fill([0.0; 4]);
                    for row in 0..valid_height {
                        for x in 0..valid_width {
                            let tile_pixel = row as usize * tile_width + x as usize;
                            let output_pixel = ((origin_y + row) as usize
                                * self.grid.canvas_width as usize
                                + (origin_x + x) as usize)
                                * 4;
                            for channel in 0..4 {
                                tile[tile_pixel][channel] =
                                    f32::from(output[output_pixel + channel]) / 255.0;
                            }
                        }
                    }
                }
            }

            for span in spans {
                let command_index =
                    usize::try_from(span.command).map_err(|_| SdfTileError::CorruptPlan)?;
                let command = self
                    .commands
                    .get(command_index)
                    .ok_or(SdfTileError::CorruptPlan)?;
                let y = origin_y + u32::from(span.row);
                if use_precomputed_shapes {
                    if let Some(program) = self.axis_shape_program(command_index) {
                        let mut fast_fragments = 0u64;
                        let mut fast_spans = 0u64;
                        for_each_axis_shape_span(
                            command,
                            program,
                            y,
                            origin_x + u32::from(span.x0),
                            origin_x + u32::from(span.x1),
                            |begin, end, source| {
                                fast_spans += 1;
                                fast_fragments += u64::from(end - begin);
                                for x in begin..end {
                                    let pixel_index = usize::from(span.row) * tile_width
                                        + usize::try_from(x - origin_x).expect("tile-local x");
                                    match accumulation {
                                        SdfAccumulationMode::Rgba8Writeback => {
                                            source_over_rgba8(&mut tile[pixel_index], source)
                                        }
                                        SdfAccumulationMode::F32Tile => {
                                            source_over_f32(&mut tile[pixel_index], source)
                                        }
                                    }
                                }
                            },
                        )?;
                        stats.shaded_fragment_count =
                            stats.shaded_fragment_count.saturating_add(fast_fragments);
                        stats.shape_shaded_fragment_count = stats
                            .shape_shaded_fragment_count
                            .saturating_add(fast_fragments);
                        stats.blended_fragment_count =
                            stats.blended_fragment_count.saturating_add(fast_fragments);
                        stats.shape_blended_fragment_count = stats
                            .shape_blended_fragment_count
                            .saturating_add(fast_fragments);
                        stats.precomputed_shape_fragment_count = stats
                            .precomputed_shape_fragment_count
                            .saturating_add(fast_fragments);
                        stats.precomputed_shape_span_count = stats
                            .precomputed_shape_span_count
                            .saturating_add(fast_spans);
                        continue;
                    }
                }
                let py = y as f32 + 0.5;
                for local_x in span.x0..span.x1 {
                    let x = origin_x + u32::from(local_x);
                    let px = x as f32 + 0.5;
                    let tx = command
                        .tx_dx
                        .mul_add(px, command.tx_dy.mul_add(py, command.tx_c));
                    let ty = command
                        .ty_dx
                        .mul_add(px, command.ty_dy.mul_add(py, command.ty_c));
                    let sample = sample_bilinear(
                        atlas,
                        command.source.atlas_set,
                        command.source.atlas_page,
                        command.source.atlas_rect,
                        tx,
                        ty,
                    )?;
                    let source = shade_sample(command.source, sample)?;
                    let pixel_index = usize::from(span.row) * tile_width + usize::from(local_x);
                    match accumulation {
                        SdfAccumulationMode::Rgba8Writeback => {
                            source_over_rgba8(&mut tile[pixel_index], source);
                        }
                        SdfAccumulationMode::F32Tile => {
                            source_over_f32(&mut tile[pixel_index], source);
                        }
                    }
                    stats.shaded_fragment_count += 1;
                    stats.sampled_texel_count += 4;
                    stats.blended_fragment_count += 1;
                    match command.source.kind {
                        SdfPrimitiveKind::Text => {
                            stats.text_shaded_fragment_count += 1;
                            stats.text_blended_fragment_count += 1;
                        }
                        SdfPrimitiveKind::Shape => {
                            stats.shape_shaded_fragment_count += 1;
                            stats.shape_blended_fragment_count += 1;
                        }
                    }
                }
            }

            let write_span = |row: u32, x0: u32, x1: u32, output: &mut [u8]| {
                for x in x0..x1 {
                    let tile_pixel = row as usize * tile_width + x as usize;
                    let output_pixel = ((origin_y + row) as usize
                        * self.grid.canvas_width as usize
                        + (origin_x + x) as usize)
                        * 4;
                    for channel in 0..4 {
                        output[output_pixel + channel] = quantize(tile[tile_pixel][channel]);
                    }
                }
            };
            match destination {
                SdfDestination::Clear(_) => {
                    for row in 0..valid_height {
                        write_span(row, 0, valid_width, output);
                    }
                }
                SdfDestination::LoadExisting => {
                    // A run may touch only a few pixels in an otherwise large tile.
                    // Re-quantizing the whole tile needlessly perturbs Skia-owned
                    // destination bytes and makes interleaved legacy/SDF composition
                    // depend on tile dimensions. Duplicate span writes are harmless:
                    // all commands have already accumulated into the final tile.
                    for span in spans {
                        write_span(
                            u32::from(span.row),
                            u32::from(span.x0),
                            u32::from(span.x1),
                            output,
                        );
                    }
                }
            }
        }
        Ok(stats)
    }

    /// Executes 16 fragments per ZMM packet using the immutable swizzled mmap
    /// pages directly. Falls back to the scalar FP32 oracle when AVX-512F/FMA/BW/VBMI
    /// or physical atlas views are absent, preserving byte-identical output.
    pub fn execute_simd(
        &self,
        atlas: &impl SdfAtlasSource,
        clear: [u8; 4],
        output: &mut [u8],
        accumulation: SdfAccumulationMode,
    ) -> Result<SdfExecutionStats, SdfTileError> {
        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("avx512f")
                && std::arch::is_x86_feature_detected!("avx512bw")
                && std::arch::is_x86_feature_detected!("avx512vbmi")
                && std::arch::is_x86_feature_detected!("fma")
            {
                // SAFETY: every target feature required by the implementation was
                // checked immediately above; page bounds remain checked in Rust.
                return unsafe {
                    simd_x86::execute(
                        self,
                        atlas,
                        SdfDestination::Clear(clear),
                        output,
                        accumulation,
                    )
                };
            }
        }
        match accumulation {
            SdfAccumulationMode::Rgba8Writeback => self.execute_scalar(atlas, clear, output),
            SdfAccumulationMode::F32Tile => self.execute_scalar_f32(atlas, clear, output),
        }
    }

    /// AVX-512 equivalent of [`Self::execute_scalar_f32_over`]. The executor
    /// loads only touched destination tiles and preserves all untouched bytes.
    /// Falls back to the scalar FP32 oracle when AVX-512 is unavailable.
    pub fn execute_simd_f32_over(
        &self,
        atlas: &impl SdfAtlasSource,
        output: &mut [u8],
    ) -> Result<SdfExecutionStats, SdfTileError> {
        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("avx512f")
                && std::arch::is_x86_feature_detected!("avx512bw")
                && std::arch::is_x86_feature_detected!("avx512vbmi")
                && std::arch::is_x86_feature_detected!("fma")
            {
                return unsafe {
                    simd_x86::execute(
                        self,
                        atlas,
                        SdfDestination::LoadExisting,
                        output,
                        SdfAccumulationMode::F32Tile,
                    )
                };
            }
        }
        self.execute_scalar_f32_over(atlas, output)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdfAtlasTexel {
    Text(u8),
    Shape(ShapeSdfTexel),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdfSwizzledFormat {
    TextR8,
    ShapeRg8,
}

#[derive(Clone, Copy, Debug)]
pub struct SdfSwizzledPage<'a> {
    pub width: u32,
    pub height: u32,
    pub format: SdfSwizzledFormat,
    pub payload: &'a [u8],
}

pub trait SdfAtlasSource {
    fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)>;
    fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel>;

    fn swizzled_page(&self, _atlas_set: u16, _page: u16) -> Option<SdfSwizzledPage<'_>> {
        None
    }
}

impl SdfAtlasSource for MappedSdfAtlas {
    fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
        if atlas_set != 0 {
            return None;
        }
        self.pages()
            .get(usize::from(page))
            .map(|page| (page.width(), page.height()))
    }

    fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
        if atlas_set != 0 {
            return None;
        }
        self.pages()
            .get(usize::from(page))?
            .texel(x, y)
            .map(SdfAtlasTexel::Text)
    }

    fn swizzled_page(&self, atlas_set: u16, page: u16) -> Option<SdfSwizzledPage<'_>> {
        if atlas_set != 0 {
            return None;
        }
        let page = self.pages().get(usize::from(page))?;
        Some(SdfSwizzledPage {
            width: page.width(),
            height: page.height(),
            format: SdfSwizzledFormat::TextR8,
            payload: page.swizzled_payload(),
        })
    }
}

impl SdfAtlasSource for MappedSdfAtlasSet {
    fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
        self.atlas(atlas_set)?
            .pages()
            .get(usize::from(page))
            .map(|page| (page.width(), page.height()))
    }

    fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
        self.atlas(atlas_set)?
            .pages()
            .get(usize::from(page))?
            .texel(x, y)
            .map(SdfAtlasTexel::Text)
    }

    fn swizzled_page(&self, atlas_set: u16, page: u16) -> Option<SdfSwizzledPage<'_>> {
        let page = self.atlas(atlas_set)?.pages().get(usize::from(page))?;
        Some(SdfSwizzledPage {
            width: page.width(),
            height: page.height(),
            format: SdfSwizzledFormat::TextR8,
            payload: page.swizzled_payload(),
        })
    }
}

/// Presents the immutable Text and Shape atlas families as one typed address
/// space without widening Text texels to RG8. Shape receives the first atlas
/// set id after all installed Text families.
pub struct MixedSdfAtlasSource<'a> {
    text: &'a MappedSdfAtlasSet,
    shape: Option<&'a MappedShapeSdfAtlas>,
    shape_atlas_set: Option<u16>,
}

impl<'a> MixedSdfAtlasSource<'a> {
    pub fn new(
        text: &'a MappedSdfAtlasSet,
        shape: Option<&'a MappedShapeSdfAtlas>,
    ) -> Result<Self, SdfTileError> {
        let shape_atlas_set = shape
            .map(|_| u16::try_from(text.len()).map_err(|_| SdfTileError::TooManyAtlasSets))
            .transpose()?;
        Ok(Self {
            text,
            shape,
            shape_atlas_set,
        })
    }

    pub const fn shape_atlas_set(&self) -> Option<u16> {
        self.shape_atlas_set
    }
}

impl SdfAtlasSource for MixedSdfAtlasSource<'_> {
    fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
        if Some(atlas_set) == self.shape_atlas_set {
            return self
                .shape?
                .pages()
                .get(usize::from(page))
                .map(|page| (page.width(), page.height()));
        }
        self.text
            .atlas(atlas_set)?
            .pages()
            .get(usize::from(page))
            .map(|page| (page.width(), page.height()))
    }

    fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
        if Some(atlas_set) == self.shape_atlas_set {
            return self
                .shape?
                .pages()
                .get(usize::from(page))?
                .texel(x, y)
                .map(SdfAtlasTexel::Shape);
        }
        self.text
            .atlas(atlas_set)?
            .pages()
            .get(usize::from(page))?
            .texel(x, y)
            .map(SdfAtlasTexel::Text)
    }

    fn swizzled_page(&self, atlas_set: u16, page: u16) -> Option<SdfSwizzledPage<'_>> {
        if Some(atlas_set) == self.shape_atlas_set {
            let page = self.shape?.pages().get(usize::from(page))?;
            return Some(SdfSwizzledPage {
                width: page.width(),
                height: page.height(),
                format: SdfSwizzledFormat::ShapeRg8,
                payload: page.swizzled_payload(),
            });
        }
        let page = self.text.atlas(atlas_set)?.pages().get(usize::from(page))?;
        Some(SdfSwizzledPage {
            width: page.width(),
            height: page.height(),
            format: SdfSwizzledFormat::TextR8,
            payload: page.swizzled_payload(),
        })
    }
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

impl AxisAlignedShapeProgram {
    fn resident_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(
                self.row_offsets
                    .len()
                    .saturating_mul(std::mem::size_of::<u32>()),
            )
            .saturating_add(
                self.runs
                    .len()
                    .saturating_mul(std::mem::size_of::<ShapeCoverageRun>()),
            )
    }

    fn runs_for_row(&self, row: u32) -> Option<&[ShapeCoverageRun]> {
        let row = usize::try_from(row).ok()?;
        let begin = usize::try_from(*self.row_offsets.get(row)?).ok()?;
        let end = usize::try_from(*self.row_offsets.get(row + 1)?).ok()?;
        self.runs.get(begin..end)
    }
}

impl ShapeSourceProgram {
    fn resident_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(
                self.row_offsets
                    .len()
                    .saturating_mul(std::mem::size_of::<u32>()),
            )
            .saturating_add(
                self.runs
                    .len()
                    .saturating_mul(std::mem::size_of::<ShapeTexelRun>()),
            )
    }

    fn runs_for_row(&self, row: u32) -> Option<&[ShapeTexelRun]> {
        let row = usize::try_from(row).ok()?;
        let begin = usize::try_from(*self.row_offsets.get(row)?).ok()?;
        let end = usize::try_from(*self.row_offsets.get(row + 1)?).ok()?;
        self.runs.get(begin..end)
    }
}

impl ShapeRowProgramCache {
    pub(crate) fn prewarm_shape_atlas(
        &self,
        atlas: &MappedShapeSdfAtlas,
    ) -> Result<ShapeRowProgramPrewarmReport, SdfTileError> {
        let empty_text = MappedSdfAtlasSet::new();
        let source = MixedSdfAtlasSource::new(&empty_text, Some(atlas))?;
        let atlas_set = source
            .shape_atlas_set()
            .ok_or(SdfTileError::TooManyAtlasSets)?;
        let mut sources = self
            .source_programs
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for entry in &atlas.manifest().shapes {
            let key = ShapeSourceProgramKey {
                atlas_page: entry.page,
                atlas_rect: entry.rect,
            };
            if sources.contains_key(&key) {
                continue;
            }
            let source_program = Arc::new(
                build_shape_texel_program(atlas_set, entry.page, entry.rect, &source)?.ok_or(
                    SdfTileError::AtlasFormatMismatch {
                        kind: SdfPrimitiveKind::Shape,
                    },
                )?,
            );
            sources.insert(key, source_program);
        }
        Ok(ShapeRowProgramPrewarmReport {
            program_count: sources.len() as u64,
            run_count: sources
                .values()
                .map(|program| program.runs.len() as u64)
                .sum(),
            resident_bytes: sources
                .values()
                .map(|program| program.resident_bytes() as u64)
                .sum(),
        })
    }

    fn get_or_build(
        &self,
        command: &PlannedCommand,
        atlas: &impl SdfAtlasSource,
    ) -> Result<Option<Arc<AxisAlignedShapeProgram>>, SdfTileError> {
        let Some(key) = ShapeRowProgramKey::from_command(command) else {
            return Ok(None);
        };
        if let Some(program) = self
            .programs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&key)
            .cloned()
        {
            return Ok(Some(program));
        }
        let source_key = ShapeSourceProgramKey {
            atlas_page: command.source.atlas_page,
            atlas_rect: command.source.atlas_rect,
        };
        let cached_source = {
            let sources = self
                .source_programs
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            sources.get(&source_key).cloned()
        };
        let source_program = if let Some(source) = cached_source {
            source
        } else {
            let Some(source) = build_shape_texel_program(
                command.source.atlas_set,
                command.source.atlas_page,
                command.source.atlas_rect,
                atlas,
            )?
            else {
                return Ok(None);
            };
            let source = Arc::new(source);
            let mut sources = self
                .source_programs
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            Arc::clone(
                sources
                    .entry(source_key)
                    .or_insert_with(|| Arc::clone(&source)),
            )
        };
        let Some(program) = build_axis_aligned_shape_program(command, &source_program)? else {
            return Ok(None);
        };
        let program = Arc::new(program);
        let mut programs = self
            .programs
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(existing) = programs.get(&key).cloned() {
            return Ok(Some(existing));
        }
        programs.put(key, Arc::clone(&program));
        Ok(Some(program))
    }
}

impl ShapeRowProgramKey {
    fn from_command(command: &PlannedCommand) -> Option<Self> {
        let material = match command.source.material {
            SdfCommandMaterial::Shape(material) => material,
            SdfCommandMaterial::Text(_) => return None,
        };
        if command.tx_dy != 0.0
            || command.ty_dx != 0.0
            || command.tx_dx <= 0.0
            || command.ty_dy <= 0.0
        {
            return None;
        }
        Some(Self {
            atlas_page: command.source.atlas_page,
            atlas_rect: command.source.atlas_rect,
            coverage: [
                material.face_threshold.to_bits(),
                material.outline_threshold.to_bits(),
                material.sharpness.to_bits(),
            ],
        })
    }
}

fn build_axis_aligned_shape_program(
    command: &PlannedCommand,
    source: &ShapeSourceProgram,
) -> Result<Option<AxisAlignedShapeProgram>, SdfTileError> {
    let material = match command.source.material {
        SdfCommandMaterial::Shape(material) => material,
        SdfCommandMaterial::Text(_) => return Ok(None),
    };
    if command.tx_dy != 0.0 || command.ty_dx != 0.0 || command.tx_dx <= 0.0 || command.ty_dy <= 0.0
    {
        return Ok(None);
    }

    let row_capacity = usize::try_from(source.rect[3])
        .ok()
        .and_then(|height| height.checked_add(1))
        .ok_or(SdfTileError::SizeOverflow)?;
    let mut row_offsets = Vec::with_capacity(row_capacity);
    let mut runs = Vec::new();
    row_offsets.push(0);
    for row in 0..source.rect[3] {
        let mut active: Option<ShapeCoverageRun> = None;
        for source_run in source.runs_for_row(row).ok_or(SdfTileError::CorruptPlan)? {
            let face_coverage = texel_coverage(
                source_run.texel,
                material.face_threshold,
                material.sharpness,
            );
            let outline_coverage = texel_coverage(
                source_run.texel,
                material.outline_threshold,
                material.sharpness,
            ) * (1.0 - face_coverage);
            if face_coverage == 0.0 && outline_coverage == 0.0 {
                if let Some(run) = active.take() {
                    runs.push(run);
                }
                continue;
            }
            match active.as_mut() {
                Some(run)
                    if run.x1 == source_run.x0
                        && run.face_coverage.to_bits() == face_coverage.to_bits()
                        && run.outline_coverage.to_bits() == outline_coverage.to_bits() =>
                {
                    run.x1 = source_run.x1;
                }
                Some(_) => {
                    runs.push(
                        active
                            .replace(ShapeCoverageRun {
                                x0: source_run.x0,
                                x1: source_run.x1,
                                face_coverage,
                                outline_coverage,
                            })
                            .expect("active shape coverage run"),
                    );
                }
                None => {
                    active = Some(ShapeCoverageRun {
                        x0: source_run.x0,
                        x1: source_run.x1,
                        face_coverage,
                        outline_coverage,
                    });
                }
            }
        }
        if let Some(run) = active {
            runs.push(run);
        }
        row_offsets.push(u32::try_from(runs.len()).map_err(|_| SdfTileError::TooManySpans)?);
    }
    Ok(Some(AxisAlignedShapeProgram {
        rect: source.rect,
        row_offsets,
        runs,
    }))
}

fn build_axis_aligned_shape_program_from_atlas(
    command: &PlannedCommand,
    atlas: &impl SdfAtlasSource,
) -> Result<Option<AxisAlignedShapeProgram>, SdfTileError> {
    if ShapeRowProgramKey::from_command(command).is_none() {
        return Ok(None);
    }
    let Some(source) = build_shape_texel_program(
        command.source.atlas_set,
        command.source.atlas_page,
        command.source.atlas_rect,
        atlas,
    )?
    else {
        return Ok(None);
    };
    build_axis_aligned_shape_program(command, &source)
}

fn build_shape_texel_program(
    atlas_set: u16,
    atlas_page: u16,
    rect: [u32; 4],
    atlas: &impl SdfAtlasSource,
) -> Result<Option<ShapeSourceProgram>, SdfTileError> {
    let [rect_x, rect_y, rect_width, rect_height] = rect;
    let row_capacity = usize::try_from(rect_height)
        .ok()
        .and_then(|height| height.checked_add(1))
        .ok_or(SdfTileError::SizeOverflow)?;
    let mut row_offsets = Vec::with_capacity(row_capacity);
    let mut runs = Vec::new();
    row_offsets.push(0);
    for y in rect_y..rect_y + rect_height {
        let mut active: Option<ShapeTexelRun> = None;
        for x in rect_x..rect_x + rect_width {
            let texel =
                match atlas
                    .texel(atlas_set, atlas_page, x, y)
                    .ok_or(SdfTileError::AtlasLookup {
                        atlas_set,
                        page: atlas_page,
                        x,
                        y,
                    })? {
                    SdfAtlasTexel::Shape(texel) => texel,
                    SdfAtlasTexel::Text(_) => return Ok(None),
                };
            match active.as_mut() {
                Some(run) if run.texel == texel => run.x1 = x + 1,
                Some(_) => {
                    runs.push(
                        active
                            .replace(ShapeTexelRun {
                                x0: x,
                                x1: x + 1,
                                texel,
                            })
                            .expect("active shape run"),
                    );
                }
                None => {
                    active = Some(ShapeTexelRun {
                        x0: x,
                        x1: x + 1,
                        texel,
                    });
                }
            }
        }
        if let Some(run) = active {
            runs.push(run);
        }
        row_offsets.push(u32::try_from(runs.len()).map_err(|_| SdfTileError::TooManySpans)?);
    }
    Ok(Some(ShapeSourceProgram {
        rect,
        row_offsets,
        runs,
    }))
}

fn rgba_f32_bits_eq(left: [f32; 4], right: [f32; 4]) -> bool {
    left.into_iter()
        .zip(right)
        .all(|(left, right)| left.to_bits() == right.to_bits())
}

fn for_each_axis_shape_span(
    command: &PlannedCommand,
    program: &AxisAlignedShapeProgram,
    y: u32,
    x0: u32,
    x1: u32,
    mut visit: impl FnMut(u32, u32, [f32; 4]),
) -> Result<(), SdfTileError> {
    let [rect_x, rect_y, rect_width, rect_height] = program.rect;
    let ty = command.ty_dy.mul_add(y as f32 + 0.5, command.ty_c);
    let source_y = ((ty + 0.5).floor() as i64)
        .clamp(i64::from(rect_y), i64::from(rect_y + rect_height) - 1) as u32;
    let runs = program
        .runs_for_row(source_y - rect_y)
        .ok_or(SdfTileError::CorruptPlan)?;
    let material = match command.source.material {
        SdfCommandMaterial::Shape(material) => material,
        SdfCommandMaterial::Text(_) => return Err(SdfTileError::CorruptPlan),
    };
    for run in runs {
        let begin = lower_bound_shape_x(command, x0, x1, run.x0, rect_x, rect_width);
        let end = lower_bound_shape_x(command, begin, x1, run.x1, rect_x, rect_width);
        if begin < end {
            visit(
                begin,
                end,
                shade_shape_coverages(run.face_coverage, run.outline_coverage, material),
            );
        }
    }
    Ok(())
}

fn lower_bound_shape_x(
    command: &PlannedCommand,
    mut begin: u32,
    mut end: u32,
    target: u32,
    rect_x: u32,
    rect_width: u32,
) -> u32 {
    while begin < end {
        let middle = begin + (end - begin) / 2;
        let tx = command.tx_dx.mul_add(middle as f32 + 0.5, command.tx_c);
        let source_x = ((tx + 0.5).floor() as i64)
            .clamp(i64::from(rect_x), i64::from(rect_x + rect_width) - 1)
            as u32;
        if source_x < target {
            begin = middle + 1;
        } else {
            end = middle;
        }
    }
    begin
}

fn validate_grid(grid: TileGrid) -> Result<(), SdfTileError> {
    if grid.canvas_width == 0
        || grid.canvas_height == 0
        || grid.tile_width == 0
        || grid.tile_height == 0
    {
        return Err(SdfTileError::InvalidGrid);
    }
    Ok(())
}

fn validate_command(
    command_index: u32,
    command: SdfDrawCommand,
    page_dimensions: (u32, u32),
) -> Result<(), SdfTileError> {
    let material_finite = match command.material {
        SdfCommandMaterial::Text(material) => material
            .face
            .into_iter()
            .chain(material.outline)
            .chain([
                material.face_scale,
                material.face_bias,
                material.outline_scale,
                material.outline_bias,
                material.vertex_alpha,
            ])
            .all(f32::is_finite),
        SdfCommandMaterial::Shape(material) => material
            .face
            .into_iter()
            .chain(material.outline)
            .chain([
                material.face_threshold,
                material.outline_threshold,
                material.sharpness,
            ])
            .all(f32::is_finite),
    };
    let finite = command
        .quad
        .iter()
        .flat_map(|point| [point.x, point.y])
        .all(f32::is_finite)
        && command.device_clip.is_none_or(|clip| {
            [clip.min_x, clip.min_y, clip.max_x, clip.max_y]
                .into_iter()
                .all(f32::is_finite)
                && clip.min_x <= clip.max_x
                && clip.min_y <= clip.max_y
        })
        && material_finite;
    if !finite {
        return Err(SdfTileError::NonFinite {
            command: command_index,
        });
    }
    if !matches!(
        (command.kind, command.material),
        (SdfPrimitiveKind::Text, SdfCommandMaterial::Text(_))
            | (SdfPrimitiveKind::Shape, SdfCommandMaterial::Shape(_))
    ) {
        return Err(SdfTileError::MaterialKindMismatch {
            command: command_index,
        });
    }
    let [x, y, width, height] = command.atlas_rect;
    if width == 0
        || height == 0
        || x.checked_add(width)
            .is_none_or(|right| right > page_dimensions.0)
        || y.checked_add(height)
            .is_none_or(|bottom| bottom > page_dimensions.1)
    {
        return Err(SdfTileError::InvalidAtlasRect {
            command: command_index,
        });
    }
    Ok(())
}

fn plan_command(
    command_index: u32,
    source: SdfDrawCommand,
) -> Result<PlannedCommand, SdfTileError> {
    let [top_left, top_right, bottom_right, bottom_left] = source.quad;
    let expected_bottom_right = Point2::new(
        top_right.x + bottom_left.x - top_left.x,
        top_right.y + bottom_left.y - top_left.y,
    );
    if (bottom_right.x - expected_bottom_right.x).hypot(bottom_right.y - expected_bottom_right.y)
        > MAX_PARALLELOGRAM_ERROR
    {
        return Err(SdfTileError::NonAffineQuad {
            command: command_index,
        });
    }
    let ex = Point2::new(top_right.x - top_left.x, top_right.y - top_left.y);
    let ey = Point2::new(bottom_left.x - top_left.x, bottom_left.y - top_left.y);
    let determinant = ex.x * ey.y - ex.y * ey.x;
    if determinant.abs() <= 1.0e-6 {
        return Err(SdfTileError::DegenerateQuad {
            command: command_index,
        });
    }
    let inverse = determinant.recip();
    let a_dx = ey.y * inverse;
    let a_dy = -ey.x * inverse;
    let b_dx = -ex.y * inverse;
    let b_dy = ex.x * inverse;
    let a_c = -(a_dx * top_left.x + a_dy * top_left.y);
    let b_c = -(b_dx * top_left.x + b_dy * top_left.y);
    let [atlas_x, atlas_y, atlas_width, atlas_height] = source.atlas_rect;
    let atlas_x = atlas_x as f32;
    let atlas_y = atlas_y as f32;
    let atlas_width = atlas_width as f32;
    let atlas_height = atlas_height as f32;
    let (shape_face_offset, shape_outline_offset, shape_coverage_scale) = match source.material {
        SdfCommandMaterial::Shape(material) => {
            let sharpness = material.sharpness.max(f32::EPSILON);
            (
                sharpness - material.face_threshold * 255.0,
                sharpness - material.outline_threshold * 255.0,
                (2.0 * sharpness).recip(),
            )
        }
        SdfCommandMaterial::Text(_) => (0.0, 0.0, 0.0),
    };
    Ok(PlannedCommand {
        source,
        tx_dx: atlas_width * a_dx,
        tx_dy: atlas_width * a_dy,
        tx_c: atlas_width.mul_add(a_c, atlas_x - 0.5),
        ty_dx: atlas_height * b_dx,
        ty_dy: atlas_height * b_dy,
        ty_c: atlas_height.mul_add(b_c, atlas_y - 0.5),
        shape_face_offset,
        shape_outline_offset,
        shape_coverage_scale,
    })
}

fn scan_command(
    grid: TileGrid,
    command_index: u32,
    kind: SdfPrimitiveKind,
    quad: [Point2; 4],
    device_clip: Option<SdfDeviceClip>,
    per_tile: &mut [Vec<TileSpan>],
    stats: &mut SdfPlanStats,
) -> Result<(), SdfTileError> {
    let clip_x0 = device_clip
        .map_or(0.0, |clip| (clip.min_x - 0.5).ceil())
        .clamp(0.0, grid.canvas_width as f32) as u32;
    let clip_y0 = device_clip
        .map_or(0.0, |clip| (clip.min_y - 0.5).ceil())
        .clamp(0.0, grid.canvas_height as f32) as u32;
    let clip_x1 = device_clip
        .map_or(grid.canvas_width as f32, |clip| (clip.max_x - 0.5).ceil())
        .clamp(0.0, grid.canvas_width as f32) as u32;
    let clip_y1 = device_clip
        .map_or(grid.canvas_height as f32, |clip| (clip.max_y - 0.5).ceil())
        .clamp(0.0, grid.canvas_height as f32) as u32;
    if clip_x0 >= clip_x1 || clip_y0 >= clip_y1 {
        return Ok(());
    }
    if let Some((min_x, min_y, max_x, max_y)) = axis_aligned_quad_bounds(quad) {
        let first_y =
            ((min_y - 0.5).ceil().clamp(0.0, grid.canvas_height as f32) as u32).max(clip_y0);
        let end_y =
            ((max_y - 0.5).ceil().clamp(0.0, grid.canvas_height as f32) as u32).min(clip_y1);
        let x0 = ((min_x - 0.5).ceil().clamp(0.0, grid.canvas_width as f32) as u32).max(clip_x0);
        let x1 = ((max_x - 0.5).ceil().clamp(0.0, grid.canvas_width as f32) as u32).min(clip_x1);
        if x0 < x1 {
            for y in first_y..end_y {
                push_scanline_spans(grid, command_index, kind, y, x0, x1, per_tile, stats)?;
            }
        }
        return Ok(());
    }

    let min_y = quad
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min);
    let max_y = quad
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let first_y = ((min_y - 0.5).ceil().clamp(0.0, grid.canvas_height as f32) as u32).max(clip_y0);
    let end_y = ((max_y - 0.5).ceil().clamp(0.0, grid.canvas_height as f32) as u32).min(clip_y1);
    for y in first_y..end_y {
        let py = y as f32 + 0.5;
        let mut intersections = [0.0f32; 4];
        let mut count = 0usize;
        for edge in 0..4 {
            let p0 = quad[edge];
            let p1 = quad[(edge + 1) & 3];
            let edge_min_y = p0.y.min(p1.y);
            let edge_max_y = p0.y.max(p1.y);
            if py < edge_min_y || py >= edge_max_y || edge_max_y == edge_min_y {
                continue;
            }
            let t = (py - p0.y) / (p1.y - p0.y);
            intersections[count] = (p1.x - p0.x).mul_add(t, p0.x);
            count += 1;
        }
        if count < 2 {
            continue;
        }
        let (min_x, max_x) = intersections[..count].iter().fold(
            (f32::INFINITY, f32::NEG_INFINITY),
            |(min_x, max_x), value| (min_x.min(*value), max_x.max(*value)),
        );
        let x0 = ((min_x - 0.5).ceil().clamp(0.0, grid.canvas_width as f32) as u32).max(clip_x0);
        let x1 = ((max_x - 0.5).ceil().clamp(0.0, grid.canvas_width as f32) as u32).min(clip_x1);
        if x0 >= x1 {
            continue;
        }
        push_scanline_spans(grid, command_index, kind, y, x0, x1, per_tile, stats)?;
    }
    Ok(())
}

fn axis_aligned_quad_bounds(quad: [Point2; 4]) -> Option<(f32, f32, f32, f32)> {
    let [top_left, top_right, bottom_right, bottom_left] = quad;
    if top_left.y.to_bits() != top_right.y.to_bits()
        || top_right.x.to_bits() != bottom_right.x.to_bits()
        || bottom_right.y.to_bits() != bottom_left.y.to_bits()
        || bottom_left.x.to_bits() != top_left.x.to_bits()
        || top_left.x > top_right.x
        || top_left.y > bottom_left.y
    {
        return None;
    }
    Some((top_left.x, top_left.y, bottom_right.x, bottom_right.y))
}

#[allow(clippy::too_many_arguments)]
fn push_scanline_spans(
    grid: TileGrid,
    command_index: u32,
    kind: SdfPrimitiveKind,
    y: u32,
    x0: u32,
    x1: u32,
    per_tile: &mut [Vec<TileSpan>],
    stats: &mut SdfPlanStats,
) -> Result<(), SdfTileError> {
    let fragments = u64::from(x1 - x0);
    stats.covered_fragment_count += fragments;
    match kind {
        SdfPrimitiveKind::Text => stats.text_covered_fragment_count += fragments,
        SdfPrimitiveKind::Shape => stats.shape_covered_fragment_count += fragments,
    }
    let tile_y = y / u32::from(grid.tile_height);
    let tiles_x = grid.tiles_x();
    for tile_x in x0 / u32::from(grid.tile_width)..=(x1 - 1) / u32::from(grid.tile_width) {
        let origin_x = tile_x * u32::from(grid.tile_width);
        let tile_index = tile_y
            .checked_mul(tiles_x)
            .and_then(|index| index.checked_add(tile_x))
            .ok_or(SdfTileError::SizeOverflow)?;
        per_tile
            .get_mut(usize::try_from(tile_index).map_err(|_| SdfTileError::SizeOverflow)?)
            .ok_or(SdfTileError::SizeOverflow)?
            .push(TileSpan {
                command: command_index,
                row: u16::try_from(y - tile_y * u32::from(grid.tile_height))
                    .map_err(|_| SdfTileError::SizeOverflow)?,
                x0: u16::try_from(x0.max(origin_x) - origin_x)
                    .map_err(|_| SdfTileError::SizeOverflow)?,
                x1: u16::try_from(x1.min(origin_x + u32::from(grid.tile_width)) - origin_x)
                    .map_err(|_| SdfTileError::SizeOverflow)?,
            });
        stats.span_count += 1;
        match kind {
            SdfPrimitiveKind::Text => stats.text_span_count += 1,
            SdfPrimitiveKind::Shape => stats.shape_span_count += 1,
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct BilinearSample {
    texels: [SdfAtlasTexel; 4],
    fx: f32,
    fy: f32,
}

fn sample_bilinear(
    atlas: &impl SdfAtlasSource,
    atlas_set: u16,
    page: u16,
    atlas_rect: [u32; 4],
    tx: f32,
    ty: f32,
) -> Result<BilinearSample, SdfTileError> {
    let (width, height) =
        atlas
            .page_dimensions(atlas_set, page)
            .ok_or(SdfTileError::MissingAtlasPage {
                command: 0,
                atlas_set,
                page,
            })?;
    let ix = tx.floor() as i64;
    let iy = ty.floor() as i64;
    let fx = tx - ix as f32;
    let fy = ty - iy as f32;
    let [rect_x, rect_y, rect_width, rect_height] = atlas_rect;
    let rect_right = rect_x
        .checked_add(rect_width)
        .filter(|right| *right <= width)
        .ok_or(SdfTileError::InvalidAtlasRect { command: 0 })?;
    let rect_bottom = rect_y
        .checked_add(rect_height)
        .filter(|bottom| *bottom <= height)
        .ok_or(SdfTileError::InvalidAtlasRect { command: 0 })?;
    let clamp_x = |x: i64| x.clamp(i64::from(rect_x), i64::from(rect_right) - 1) as u32;
    let clamp_y = |y: i64| y.clamp(i64::from(rect_y), i64::from(rect_bottom) - 1) as u32;
    let x0 = clamp_x(ix);
    let x1 = clamp_x(ix + 1);
    let y0 = clamp_y(iy);
    let y1 = clamp_y(iy + 1);
    let texel = |x, y| {
        atlas
            .texel(atlas_set, page, x, y)
            .ok_or(SdfTileError::AtlasLookup {
                atlas_set,
                page,
                x,
                y,
            })
    };
    Ok(BilinearSample {
        texels: [
            texel(x0, y0)?,
            texel(x1, y0)?,
            texel(x0, y1)?,
            texel(x1, y1)?,
        ],
        fx,
        fy,
    })
}

fn shade_sample(command: SdfDrawCommand, sample: BilinearSample) -> Result<[f32; 4], SdfTileError> {
    match (command.kind, command.material) {
        (SdfPrimitiveKind::Text, SdfCommandMaterial::Text(material)) => {
            let mut values = [0.0f32; 4];
            for (destination, source) in values.iter_mut().zip(sample.texels) {
                *destination = match source {
                    SdfAtlasTexel::Text(value) => f32::from(value) / 255.0,
                    SdfAtlasTexel::Shape(_) => {
                        return Err(SdfTileError::AtlasFormatMismatch { kind: command.kind });
                    }
                };
            }
            let [p00, p10, p01, p11] = values;
            let top = (p10 - p00).mul_add(sample.fx, p00);
            let bottom = (p11 - p01).mul_add(sample.fx, p01);
            Ok(shade_text(material, (bottom - top).mul_add(sample.fy, top)))
        }
        (SdfPrimitiveKind::Shape, SdfCommandMaterial::Shape(material)) => {
            let mut texels = [ShapeSdfTexel::default(); 4];
            for (destination, source) in texels.iter_mut().zip(sample.texels) {
                *destination = match source {
                    SdfAtlasTexel::Shape(texel) => texel,
                    SdfAtlasTexel::Text(_) => {
                        return Err(SdfTileError::AtlasFormatMismatch { kind: command.kind });
                    }
                };
            }
            let nearest_x = usize::from(sample.fx >= 0.5);
            let nearest_y = usize::from(sample.fy >= 0.5);
            let texel = texels[nearest_y * 2 + nearest_x];
            Ok(shade_shape([texel; 4], 0.0, 0.0, material))
        }
        _ => Err(SdfTileError::MaterialKindMismatch { command: 0 }),
    }
}

fn shade_text(material: SdfMaterial, sdf: f32) -> [f32; 4] {
    let face_t = material
        .face_scale
        .max(0.0001)
        .mul_add(sdf, -material.face_bias)
        .clamp(0.0, 1.0);
    let outline_t = material
        .outline_scale
        .max(0.0001)
        .mul_add(sdf, -material.outline_bias)
        .clamp(0.0, 1.0)
        * (sdf * 12.5).clamp(0.0, 1.0);
    let outline_weight = outline_t * (1.0 - material.face[3] * face_t);
    let vertex_alpha = material.vertex_alpha.clamp(0.0, 1.0);
    std::array::from_fn(|channel| {
        material.outline[channel].mul_add(outline_weight, material.face[channel] * face_t)
            * vertex_alpha
    })
}

fn source_over_rgba8(destination: &mut [f32; 4], source: [f32; 4]) {
    // Preserve the floating source-over math used by Skia's raster pipeline,
    // then model the RGBA8 surface writeback between ordered draws.
    let inverse_alpha = 1.0 - source[3];
    for channel in 0..4 {
        let blended = source[channel] + destination[channel] * inverse_alpha;
        destination[channel] = f32::from(quantize(blended)) / 255.0;
    }
}

fn source_over_f32(destination: &mut [f32; 4], source: [f32; 4]) {
    let inverse_alpha = 1.0 - source[3];
    for channel in 0..4 {
        destination[channel] = destination[channel].mul_add(inverse_alpha, source[channel]);
    }
}

fn quantize(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestAtlas {
        width: u32,
        height: u32,
        texels: Vec<u8>,
    }

    struct TestMixedAtlas {
        text: u8,
        shape: ShapeSdfTexel,
    }

    struct PhysicalMixedAtlas {
        text: [u8; 64],
        shape: [u8; 128],
    }

    struct PhysicalTextAtlas {
        payload: [u8; 128],
    }

    struct PhysicalShapeAtlas {
        payload: [u8; 256],
    }

    struct WidePhysicalShapeAtlas {
        payload: [u8; 640],
    }

    impl SdfAtlasSource for PhysicalTextAtlas {
        fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
            (atlas_set == 0 && page == 0).then_some((16, 8))
        }

        fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
            if atlas_set != 0 || page != 0 || x >= 16 || y >= 8 {
                return None;
            }
            let block = x / 8;
            let in_block = y * 8 + x % 8;
            self.payload
                .get((block * 64 + in_block) as usize)
                .copied()
                .map(SdfAtlasTexel::Text)
        }

        fn swizzled_page(&self, atlas_set: u16, page: u16) -> Option<SdfSwizzledPage<'_>> {
            (atlas_set == 0 && page == 0).then_some(SdfSwizzledPage {
                width: 16,
                height: 8,
                format: SdfSwizzledFormat::TextR8,
                payload: &self.payload,
            })
        }
    }

    impl SdfAtlasSource for PhysicalShapeAtlas {
        fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
            (atlas_set == 0 && page == 0).then_some((16, 8))
        }

        fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
            if atlas_set != 0 || page != 0 || x >= 16 || y >= 8 {
                return None;
            }
            let block = x / 8;
            let local = y * 8 + x % 8;
            let offset = (block * 128 + local * 2) as usize;
            Some(SdfAtlasTexel::Shape(ShapeSdfTexel {
                distance: self.payload[offset],
                gate: self.payload[offset + 1],
            }))
        }

        fn swizzled_page(&self, atlas_set: u16, page: u16) -> Option<SdfSwizzledPage<'_>> {
            (atlas_set == 0 && page == 0).then_some(SdfSwizzledPage {
                width: 16,
                height: 8,
                format: SdfSwizzledFormat::ShapeRg8,
                payload: &self.payload,
            })
        }
    }

    impl SdfAtlasSource for WidePhysicalShapeAtlas {
        fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
            (atlas_set == 0 && page == 0).then_some((40, 8))
        }

        fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
            if atlas_set != 0 || page != 0 || x >= 40 || y >= 8 {
                return None;
            }
            let block = x / 8;
            let local = y * 8 + x % 8;
            let offset = (block * 128 + local * 2) as usize;
            Some(SdfAtlasTexel::Shape(ShapeSdfTexel {
                distance: self.payload[offset],
                gate: self.payload[offset + 1],
            }))
        }

        fn swizzled_page(&self, atlas_set: u16, page: u16) -> Option<SdfSwizzledPage<'_>> {
            (atlas_set == 0 && page == 0).then_some(SdfSwizzledPage {
                width: 40,
                height: 8,
                format: SdfSwizzledFormat::ShapeRg8,
                payload: &self.payload,
            })
        }
    }

    impl SdfAtlasSource for PhysicalMixedAtlas {
        fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
            (atlas_set <= 1 && page == 0).then_some((8, 8))
        }

        fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
            if page != 0 || x >= 8 || y >= 8 {
                return None;
            }
            let index = (y * 8 + x) as usize;
            match atlas_set {
                0 => Some(SdfAtlasTexel::Text(self.text[index])),
                1 => Some(SdfAtlasTexel::Shape(ShapeSdfTexel {
                    distance: self.shape[index * 2],
                    gate: self.shape[index * 2 + 1],
                })),
                _ => None,
            }
        }

        fn swizzled_page(&self, atlas_set: u16, page: u16) -> Option<SdfSwizzledPage<'_>> {
            if page != 0 {
                return None;
            }
            match atlas_set {
                0 => Some(SdfSwizzledPage {
                    width: 8,
                    height: 8,
                    format: SdfSwizzledFormat::TextR8,
                    payload: &self.text,
                }),
                1 => Some(SdfSwizzledPage {
                    width: 8,
                    height: 8,
                    format: SdfSwizzledFormat::ShapeRg8,
                    payload: &self.shape,
                }),
                _ => None,
            }
        }
    }

    impl SdfAtlasSource for TestMixedAtlas {
        fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
            (atlas_set <= 1 && page == 0).then_some((2, 2))
        }

        fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
            if page != 0 || x >= 2 || y >= 2 {
                return None;
            }
            match atlas_set {
                0 => Some(SdfAtlasTexel::Text(self.text)),
                1 => Some(SdfAtlasTexel::Shape(self.shape)),
                _ => None,
            }
        }
    }

    impl SdfAtlasSource for TestAtlas {
        fn page_dimensions(&self, atlas_set: u16, page: u16) -> Option<(u32, u32)> {
            (atlas_set == 0 && page == 0).then_some((self.width, self.height))
        }

        fn texel(&self, atlas_set: u16, page: u16, x: u32, y: u32) -> Option<SdfAtlasTexel> {
            if atlas_set != 0 || page != 0 || x >= self.width || y >= self.height {
                return None;
            }
            self.texels
                .get((y as usize) * self.width as usize + x as usize)
                .copied()
                .map(SdfAtlasTexel::Text)
        }
    }

    fn solid_atlas(value: u8) -> TestAtlas {
        TestAtlas {
            width: 8,
            height: 8,
            texels: vec![value; 64],
        }
    }

    fn command(color: [f32; 4], quad: [Point2; 4]) -> SdfDrawCommand {
        SdfDrawCommand {
            kind: SdfPrimitiveKind::Text,
            atlas_set: 0,
            atlas_page: 0,
            atlas_rect: [0, 0, 8, 8],
            quad,
            device_clip: None,
            material: SdfCommandMaterial::Text(SdfMaterial {
                face: color,
                ..SdfMaterial::default()
            }),
        }
    }

    fn rectangle(x0: f32, y0: f32, x1: f32, y1: f32) -> [Point2; 4] {
        [
            Point2::new(x0, y0),
            Point2::new(x1, y0),
            Point2::new(x1, y1),
            Point2::new(x0, y1),
        ]
    }

    #[test]
    fn plan_is_variable_sized_and_preserves_command_order_per_tile() {
        let atlas = solid_atlas(255);
        let commands = [
            command([1.0, 0.0, 0.0, 1.0], rectangle(0.0, 0.0, 3.0, 2.0)),
            command([0.0, 1.0, 0.0, 1.0], rectangle(1.0, 0.0, 4.0, 2.0)),
        ];
        let grid = TileGrid {
            canvas_width: 5,
            canvas_height: 3,
            tile_width: 2,
            tile_height: 2,
        };
        let plan = SdfTilePlan::build(grid, &commands, &atlas).expect("build plan");
        assert_eq!(plan.stats().command_count, 2);
        assert_eq!(plan.stats().span_count, 8);
        assert_eq!(
            plan.spans_for_tile(0)
                .expect("tile zero")
                .iter()
                .map(|span| span.command)
                .collect::<Vec<_>>(),
            vec![0, 0, 1, 1]
        );

        let mut output = vec![0; 5 * 3 * 4];
        let stats = plan
            .execute_scalar(&atlas, [0, 0, 0, 0], &mut output)
            .expect("execute scalar");
        assert_eq!(stats.shaded_fragment_count, 12);
        assert_eq!(&output[(0 * 5 + 0) * 4..][..4], &[255, 0, 0, 255]);
        assert_eq!(&output[(0 * 5 + 1) * 4..][..4], &[0, 255, 0, 255]);
        assert_eq!(&output[(0 * 5 + 4) * 4..][..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn device_clip_limits_text_spans_without_moving_atlas_geometry() {
        let atlas = solid_atlas(255);
        let mut clipped = command([1.0, 0.0, 0.0, 1.0], rectangle(0.0, 0.0, 8.0, 4.0));
        clipped.device_clip = Some(SdfDeviceClip {
            min_x: 2.0,
            min_y: 1.0,
            max_x: 6.0,
            max_y: 3.0,
        });
        let plan =
            SdfTilePlan::build(TileGrid::new(8, 4), &[clipped], &atlas).expect("clipped text plan");
        assert_eq!(plan.stats().covered_fragment_count, 8);

        let mut output = vec![0; 8 * 4 * 4];
        plan.execute_scalar(&atlas, [0, 0, 0, 0], &mut output)
            .expect("execute clipped text");
        for y in 0..4usize {
            for x in 0..8usize {
                let alpha = output[(y * 8 + x) * 4 + 3];
                assert_eq!(alpha != 0, (2..6).contains(&x) && (1..3).contains(&y));
            }
        }
    }

    #[test]
    fn pixel_occlusion_counts_destination_fragments_without_changing_plan() {
        let atlas = solid_atlas(255);
        let commands = [
            command([1.0, 0.0, 0.0, 1.0], rectangle(0.0, 0.0, 3.0, 2.0)),
            command([0.0, 1.0, 0.0, 1.0], rectangle(1.0, 0.0, 4.0, 2.0)),
        ];
        let plan = SdfTilePlan::build(
            TileGrid {
                canvas_width: 5,
                canvas_height: 3,
                tile_width: 2,
                tile_height: 2,
            },
            &commands,
            &atlas,
        )
        .expect("build plan");
        let mut rgba = vec![0u8; 5 * 3 * 4];
        for y in 0..2 {
            for x in 0..3 {
                rgba[(y * 5 + x) * 4 + 3] = 255;
            }
        }
        let mut mask = PixelOcclusionMask::new(5, 3).expect("mask");
        assert_eq!(mask.union_opaque_rgba8(&rgba).expect("union"), 6);
        let measured = plan.measure_occlusion(&mask).expect("measure");
        assert_eq!(measured.occluded_fragment_count, 10);
        assert_eq!(measured.visible_fragment_count, 2);
        assert_eq!(measured.occluded_text_fragment_count, 10);
        assert_eq!(measured.occluded_shape_fragment_count, 0);
        assert_eq!(measured.fully_occluded_command_count, 1);
        assert_eq!(plan.stats().covered_fragment_count, 12);

        let (visible, filtered) = plan.visible_plan(&mask).expect("visible plan");
        assert_eq!(filtered, measured);
        assert_eq!(visible.stats().covered_fragment_count, 2);
        let mut original_pixels = vec![0; 5 * 3 * 4];
        let mut visible_pixels = vec![0; 5 * 3 * 4];
        plan.execute_scalar(&atlas, [0, 0, 0, 0], &mut original_pixels)
            .expect("execute original");
        visible
            .execute_scalar(&atlas, [0, 0, 0, 0], &mut visible_pixels)
            .expect("execute visible");
        for y in 0..2 {
            for x in 0..3 {
                let offset = (y * 5 + x) * 4;
                original_pixels[offset..offset + 4].copy_from_slice(&[0, 0, 255, 255]);
                visible_pixels[offset..offset + 4].copy_from_slice(&[0, 0, 255, 255]);
            }
        }
        assert_eq!(visible_pixels, original_pixels);
    }

    #[test]
    fn pixel_occlusion_mask_counts_across_word_boundaries() {
        let mut rgba = vec![0u8; 130 * 4];
        for x in [0usize, 63, 64, 129] {
            rgba[x * 4 + 3] = 255;
        }
        let mut mask = PixelOcclusionMask::new(130, 1).expect("mask");
        assert_eq!(mask.union_opaque_rgba8(&rgba).expect("union"), 4);
        assert_eq!(mask.count_opaque_span(0, 0, 130), 4);
        assert_eq!(mask.count_opaque_span(0, 1, 129), 2);
        assert_eq!(mask.count_opaque_span(0, 63, 65), 2);
    }

    #[test]
    fn highway_abi_export_preserves_plan_order_and_has_stable_layout() {
        let atlas = solid_atlas(255);
        let commands = [
            command([1.0, 0.0, 0.0, 1.0], rectangle(0.0, 0.0, 3.0, 2.0)),
            command([0.0, 1.0, 0.0, 1.0], rectangle(1.0, 0.0, 4.0, 2.0)),
        ];
        let grid = TileGrid {
            canvas_width: 5,
            canvas_height: 3,
            tile_width: 2,
            tile_height: 2,
        };
        let plan = SdfTilePlan::build(grid, &commands, &atlas).expect("build plan");
        let exported = plan.export_highway_abi();

        assert_eq!(exported.abi_version, 1);
        assert_eq!(exported.commands.len(), 2);
        assert_eq!(exported.commands[0].kind, 1);
        assert_eq!(exported.commands[1].kind, 1);
        assert_eq!(exported.spans.len(), 8);
        assert_eq!(exported.tile_offsets.len(), 7);
        assert_eq!(std::mem::size_of_val(&exported.commands[0]), 112);
        assert_eq!(std::mem::align_of_val(&exported.commands[0]), 16);
        assert_eq!(std::mem::size_of_val(&exported.spans[0]), 12);
        assert_eq!(std::mem::offset_of!(HighwaySdfCommand, kind), 0);
        assert_eq!(std::mem::offset_of!(HighwaySdfCommand, atlas_set), 4);
        assert_eq!(std::mem::offset_of!(HighwaySdfCommand, atlas_page), 6);
        assert_eq!(std::mem::offset_of!(HighwaySdfCommand, atlas_rect), 8);
        assert_eq!(std::mem::offset_of!(HighwaySdfCommand, inverse_affine), 24);
        assert_eq!(std::mem::offset_of!(HighwaySdfCommand, face), 48);
        assert_eq!(std::mem::offset_of!(HighwaySdfCommand, outline), 64);
        assert_eq!(std::mem::offset_of!(HighwaySdfCommand, params), 80);
        assert_eq!(std::mem::offset_of!(HighwaySdfSpan, command), 0);
        assert_eq!(std::mem::offset_of!(HighwaySdfSpan, row), 4);
        assert_eq!(std::mem::offset_of!(HighwaySdfSpan, x0), 6);
        assert_eq!(std::mem::offset_of!(HighwaySdfSpan, x1), 8);
        assert_eq!(std::mem::offset_of!(HighwaySdfSpan, reserved), 10);
    }

    #[test]
    fn bilinear_sampler_and_material_are_locked_by_scalar_oracle() {
        let atlas = TestAtlas {
            width: 2,
            height: 2,
            texels: vec![0, 64, 128, 255],
        };
        let mut draw = command([0.5, 0.25, 0.0, 0.5], rectangle(0.0, 0.0, 1.0, 1.0));
        draw.atlas_rect = [0, 0, 2, 2];
        let SdfCommandMaterial::Text(material) = &mut draw.material else {
            panic!("test command must use text material");
        };
        material.face_scale = 2.0;
        material.face_bias = 0.25;
        let plan = SdfTilePlan::build(TileGrid::new(1, 1), &[draw], &atlas).expect("build plan");
        let mut output = [0; 4];
        plan.execute_scalar(&atlas, [0, 0, 0, 0], &mut output)
            .expect("execute scalar");
        // Center sample = mean(0,64,128,255)/255 = 0.4382353;
        // face_t = center*2-0.25 = 0.6264706.
        assert_eq!(output, [80, 40, 0, 80]);
    }

    #[test]
    fn rejects_non_affine_and_degenerate_commands() {
        let atlas = solid_atlas(255);
        let non_affine = command(
            [1.0; 4],
            [
                Point2::new(0.0, 0.0),
                Point2::new(2.0, 0.0),
                Point2::new(3.0, 2.0),
                Point2::new(0.0, 2.0),
            ],
        );
        assert_eq!(
            SdfTilePlan::build(TileGrid::new(4, 4), &[non_affine], &atlas)
                .expect_err("non-affine quad must fail"),
            SdfTileError::NonAffineQuad { command: 0 }
        );

        let degenerate = command(
            [1.0; 4],
            [
                Point2::new(0.0, 0.0),
                Point2::new(1.0, 0.0),
                Point2::new(2.0, 0.0),
                Point2::new(1.0, 0.0),
            ],
        );
        assert_eq!(
            SdfTilePlan::build(TileGrid::new(4, 4), &[degenerate], &atlas)
                .expect_err("degenerate quad must fail"),
            SdfTileError::DegenerateQuad { command: 0 }
        );
    }

    #[test]
    fn reports_shape_commands_separately() {
        let atlas = solid_atlas(255);
        let mut shape = command([1.0; 4], rectangle(0.0, 0.0, 2.0, 2.0));
        shape.kind = SdfPrimitiveKind::Shape;
        shape.material = SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
            [1.0; 3], 1.0, [0.0; 3], 0.0, 0.0,
        ));
        let plan = SdfTilePlan::build(TileGrid::new(2, 2), &[shape], &atlas).expect("shape plan");
        assert_eq!(plan.stats().text_command_count, 0);
        assert_eq!(plan.stats().shape_command_count, 1);
    }

    #[test]
    fn mixed_text_and_shape_keep_order_and_typed_sampling() {
        let atlas = TestMixedAtlas {
            text: 255,
            shape: ShapeSdfTexel {
                distance: 255,
                gate: 255,
            },
        };
        let mut text = command([1.0, 0.0, 0.0, 1.0], rectangle(0.0, 0.0, 1.0, 1.0));
        text.atlas_rect = [0, 0, 2, 2];
        let shape = SdfDrawCommand {
            kind: SdfPrimitiveKind::Shape,
            atlas_set: 1,
            atlas_page: 0,
            atlas_rect: [0, 0, 2, 2],
            quad: rectangle(0.0, 0.0, 1.0, 1.0),
            device_clip: None,
            material: SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
                [0.0, 1.0, 0.0],
                0.5,
                [0.0; 3],
                0.0,
                0.0,
            )),
        };
        let plan =
            SdfTilePlan::build(TileGrid::new(1, 1), &[text, shape], &atlas).expect("mixed plan");
        let mut output = [0; 4];
        let stats = plan
            .execute_scalar(&atlas, [0, 0, 0, 0], &mut output)
            .expect("mixed scalar execution");
        assert_eq!(output, [128, 128, 0, 255]);
        assert_eq!(stats.text_shaded_fragment_count, 1);
        assert_eq!(stats.shape_shaded_fragment_count, 1);
    }

    #[test]
    fn load_existing_destination_preserves_untouched_pixels_and_blends_touched_pixels() {
        let atlas = PhysicalTextAtlas {
            payload: [255; 128],
        };
        let draw = command([0.5, 0.0, 0.0, 0.5], rectangle(1.0, 0.0, 3.0, 1.0));
        let plan = SdfTilePlan::build(TileGrid::new(4, 2), &[draw], &atlas).expect("build plan");
        // Non-dyadic opaque channels catch accidental conversion/writeback of
        // pixels sharing a tile with the draw but outside its actual spans.
        let background = [255, 239, 168, 255];
        let mut output = background.repeat(8);
        plan.execute_scalar_f32_over(&atlas, &mut output)
            .expect("blend over existing target");
        assert_eq!(&output[0..4], &background);
        assert_eq!(&output[4..8], &[255, 120, 84, 255]);
        assert_eq!(&output[8..12], &[255, 120, 84, 255]);
        assert_eq!(&output[12..16], &background);
        assert_eq!(&output[16..], &background.repeat(4));

        let mut simd = background.repeat(8);
        let result = plan.execute_simd_f32_over(&atlas, &mut simd);
        #[cfg(target_arch = "x86_64")]
        let simd_supported = std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
            && std::arch::is_x86_feature_detected!("avx512vbmi")
            && std::arch::is_x86_feature_detected!("fma");
        #[cfg(not(target_arch = "x86_64"))]
        let simd_supported = false;
        if simd_supported {
            result.expect("supported SIMD over execution");
            assert_eq!(simd, output);
        } else {
            result.expect("scalar fallback execution");
            assert_eq!(simd, output);
        }
    }

    #[test]
    fn simd_dispatch_is_exact_to_f32_oracle_or_fails_closed() {
        let mut text = [0u8; 64];
        for (index, value) in text.iter_mut().enumerate() {
            *value = (index * 4).min(255) as u8;
        }
        let mut shape = [0u8; 128];
        for texel in shape.chunks_exact_mut(2) {
            texel.copy_from_slice(&[220, 200]);
        }
        let atlas = PhysicalMixedAtlas { text, shape };
        let text = command([0.7, 0.2, 0.1, 0.75], rectangle(0.0, 0.0, 32.0, 4.0));
        let shape = SdfDrawCommand {
            kind: SdfPrimitiveKind::Shape,
            atlas_set: 1,
            atlas_page: 0,
            atlas_rect: [0, 0, 8, 8],
            quad: rectangle(4.0, 0.0, 28.0, 4.0),
            device_clip: None,
            material: SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
                [0.1, 0.6, 0.3],
                0.55,
                [0.8, 0.2, 0.5],
                0.4,
                0.25,
            )),
        };
        let plan = SdfTilePlan::build(TileGrid::new(32, 4), &[text, shape], &atlas)
            .expect("physical mixed plan");
        let mut oracle = vec![0; 32 * 4 * 4];
        plan.execute_scalar_f32(&atlas, [3, 5, 7, 11], &mut oracle)
            .expect("scalar f32 oracle");
        let mut simd = vec![0; oracle.len()];
        let result = plan.execute_simd(
            &atlas,
            [3, 5, 7, 11],
            &mut simd,
            SdfAccumulationMode::F32Tile,
        );
        let background = [17, 31, 47, 191].repeat(32 * 4);
        let mut over_oracle = background.clone();
        plan.execute_scalar_f32_over(&atlas, &mut over_oracle)
            .expect("scalar over oracle");
        let mut over_simd = background.clone();
        let over_result = plan.execute_simd_f32_over(&atlas, &mut over_simd);
        #[cfg(target_arch = "x86_64")]
        let simd_supported = std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
            && std::arch::is_x86_feature_detected!("avx512vbmi")
            && std::arch::is_x86_feature_detected!("fma");
        #[cfg(not(target_arch = "x86_64"))]
        let simd_supported = false;
        if simd_supported {
            let stats = result.expect("supported SIMD execution");
            assert_eq!(simd, oracle);
            assert!(stats.simd_packet_count > 0);
            assert!(stats.swizzled_packet_count > 0);
            assert_eq!(
                stats.simd_packet_count,
                stats.swizzled_packet_count + stats.gather_fallback_packet_count
            );
            over_result.expect("supported SIMD over execution");
            assert_eq!(over_simd, over_oracle);
        } else {
            result.expect("scalar fallback execution");
            over_result.expect("scalar fallback over execution");
            assert_eq!(simd, oracle);
            assert_eq!(over_simd, over_oracle);
        }
    }

    #[test]
    fn text_adjacent_blocks_use_swizzled_loads_without_gather() {
        let mut payload = [0u8; 128];
        for (index, value) in payload.iter_mut().enumerate() {
            *value = ((index * 37 + 19) & 0xff) as u8;
        }
        let atlas = PhysicalTextAtlas { payload };
        let mut text = command([0.7, 0.2, 0.1, 0.75], rectangle(0.0, 0.0, 16.0, 1.0));
        text.atlas_rect = [0, 0, 16, 8];
        let plan = SdfTilePlan::build(TileGrid::new(16, 1), &[text], &atlas)
            .expect("cross-block text plan");
        let mut oracle = vec![0; 16 * 4];
        plan.execute_scalar_f32(&atlas, [3, 5, 7, 11], &mut oracle)
            .expect("scalar f32 oracle");
        let mut simd = vec![0; oracle.len()];
        let result = plan.execute_simd(
            &atlas,
            [3, 5, 7, 11],
            &mut simd,
            SdfAccumulationMode::F32Tile,
        );

        #[cfg(target_arch = "x86_64")]
        let simd_supported = std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
            && std::arch::is_x86_feature_detected!("avx512vbmi")
            && std::arch::is_x86_feature_detected!("fma");
        #[cfg(not(target_arch = "x86_64"))]
        let simd_supported = false;

        if simd_supported {
            let stats = result.expect("supported SIMD execution");
            assert_eq!(simd, oracle);
            assert_eq!(stats.swizzled_packet_count, 1);
            assert_eq!(stats.gather_fallback_packet_count, 0);
        } else {
            result.expect("scalar fallback execution");
            assert_eq!(simd, oracle);
        }
    }

    #[test]
    fn shape_five_blocks_use_swizzled_loads_and_match_scalar() {
        let mut payload = [0u8; 640];
        for (index, texel) in payload.chunks_exact_mut(2).enumerate() {
            texel.copy_from_slice(&[
                ((index * 29 + 17) & 0xff) as u8,
                ((index * 11 + 101) & 0xff) as u8,
            ]);
        }
        let atlas = WidePhysicalShapeAtlas { payload };
        let shape = SdfDrawCommand {
            kind: SdfPrimitiveKind::Shape,
            atlas_set: 0,
            atlas_page: 0,
            atlas_rect: [0, 0, 40, 8],
            quad: rectangle(0.0, 0.0, 16.0, 1.0),
            device_clip: None,
            material: SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
                [0.2, 0.4, 0.8],
                0.7,
                [0.8, 0.1, 0.2],
                0.3,
                0.2,
            )),
        };
        let plan =
            SdfTilePlan::build_for_one_shot_dynamic_layer(TileGrid::new(16, 1), &[shape], &atlas)
                .expect("five-block shape plan");
        let mut oracle = vec![0; 16 * 4];
        plan.execute_scalar_f32(&atlas, [3, 5, 7, 11], &mut oracle)
            .expect("scalar f32 oracle");
        let mut simd = vec![0; oracle.len()];
        let result = plan.execute_simd(
            &atlas,
            [3, 5, 7, 11],
            &mut simd,
            SdfAccumulationMode::F32Tile,
        );

        #[cfg(target_arch = "x86_64")]
        let simd_supported = std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
            && std::arch::is_x86_feature_detected!("avx512vbmi")
            && std::arch::is_x86_feature_detected!("fma");
        #[cfg(not(target_arch = "x86_64"))]
        let simd_supported = false;

        if simd_supported {
            let stats = result.expect("supported SIMD execution");
            assert_eq!(simd, oracle);
            assert_eq!(stats.swizzled_packet_count, 1);
            assert_eq!(stats.gather_fallback_packet_count, 0);
        } else {
            result.expect("scalar fallback execution");
            assert_eq!(simd, oracle);
        }
    }

    #[test]
    fn axis_aligned_shape_uses_row_program_without_sampling() {
        let mut payload = [0u8; 256];
        for (index, texel) in payload.chunks_exact_mut(2).enumerate() {
            texel.copy_from_slice(&[
                ((index * 29 + 17) & 0xff) as u8,
                ((index * 11 + 101) & 0xff) as u8,
            ]);
        }
        let atlas = PhysicalShapeAtlas { payload };
        let shape = SdfDrawCommand {
            kind: SdfPrimitiveKind::Shape,
            atlas_set: 0,
            atlas_page: 0,
            atlas_rect: [0, 0, 16, 8],
            quad: rectangle(0.0, 0.0, 16.0, 1.0),
            device_clip: None,
            material: SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
                [0.1, 0.6, 0.3],
                0.55,
                [0.8, 0.2, 0.5],
                0.4,
                0.25,
            )),
        };
        let plan = SdfTilePlan::build(TileGrid::new(16, 1), &[shape], &atlas)
            .expect("cross-block shape plan");
        let mut oracle = vec![0; 16 * 4];
        plan.execute_scalar_f32(&atlas, [3, 5, 7, 11], &mut oracle)
            .expect("scalar f32 oracle");
        let mut simd = vec![0; oracle.len()];
        let result = plan.execute_simd(
            &atlas,
            [3, 5, 7, 11],
            &mut simd,
            SdfAccumulationMode::F32Tile,
        );

        #[cfg(target_arch = "x86_64")]
        let simd_supported = std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
            && std::arch::is_x86_feature_detected!("avx512vbmi")
            && std::arch::is_x86_feature_detected!("fma");
        #[cfg(not(target_arch = "x86_64"))]
        let simd_supported = false;

        if simd_supported {
            let stats = result.expect("supported SIMD execution");
            assert_eq!(simd, oracle);
            assert!(stats.precomputed_shape_fragment_count > 0);
            assert!(stats.precomputed_shape_span_count > 0);
            assert_eq!(stats.sampled_texel_count, 0);
            assert_eq!(stats.swizzled_packet_count, 0);
            assert_eq!(stats.gather_fallback_packet_count, 0);
        } else {
            result.expect("scalar fallback execution");
            assert_eq!(simd, oracle);
        }
    }

    #[test]
    fn atlas_glyph_builder_reuses_legacy_plane_geometry() {
        let glyph = SdfAtlasGlyphManifest {
            codepoint: u32::from('A'),
            page: 3,
            rect: [16, 24, 8, 9],
            plane_bearing: [2.0, 7.0],
            plane_size: [4.0, 5.0],
            plane_advance_x: 4.5,
        };
        let command = SdfDrawCommand::from_atlas_glyph(
            SdfPrimitiveKind::Text,
            0,
            &glyph,
            10.0,
            1.0,
            Point2::new(10.0, 20.0),
            20.0,
            Affine2::IDENTITY,
            SdfMaterial::default(),
        )
        .expect("valid glyph placement");
        assert_eq!(command.atlas_page, 3);
        assert_eq!(command.atlas_rect, [16, 24, 8, 9]);
        assert_eq!(
            command.quad,
            [
                Point2::new(12.0, 4.0),
                Point2::new(24.0, 4.0),
                Point2::new(24.0, 18.0),
                Point2::new(12.0, 18.0),
            ]
        );
    }

    #[test]
    fn atlas_glyph_builder_applies_resolved_affine_without_relayout() {
        let glyph = SdfAtlasGlyphManifest {
            codepoint: u32::from('B'),
            page: 0,
            rect: [0, 0, 8, 8],
            plane_bearing: [0.0, 1.0],
            plane_size: [2.0, 2.0],
            plane_advance_x: 2.0,
        };
        let affine = Affine2 {
            scale_x: 2.0,
            skew_x: 0.5,
            translate_x: 3.0,
            skew_y: -0.25,
            scale_y: 1.5,
            translate_y: 4.0,
        };
        let command = SdfDrawCommand::from_atlas_glyph(
            SdfPrimitiveKind::Text,
            0,
            &glyph,
            1.0,
            0.0,
            Point2::new(0.0, 0.0),
            1.0,
            affine,
            SdfMaterial::default(),
        )
        .expect("affine glyph placement");
        assert_eq!(command.quad[0], affine.map_point(Point2::new(0.0, -1.0)));
        assert_eq!(command.quad[2], affine.map_point(Point2::new(2.0, 1.0)));
        let plan = SdfTilePlan::build(TileGrid::new(16, 16), &[command], &solid_atlas(255))
            .expect("affine command must remain a parallelogram");
        assert_eq!(plan.stats().command_count, 1);
    }

    #[test]
    fn axis_aligned_shape_row_program_is_exact_to_sampler_oracle() {
        let mut atlas = PhysicalShapeAtlas { payload: [0; 256] };
        for y in 0..8u32 {
            for x in 0..16u32 {
                let block = x / 8;
                let local = y * 8 + x % 8;
                let offset = (block * 128 + local * 2) as usize;
                let border = x == 0 || x == 15 || y == 0 || y == 7;
                atlas.payload[offset] = if border {
                    0
                } else {
                    ((x * 19 + y * 31) & 0xff) as u8
                };
                atlas.payload[offset + 1] = if border { 0 } else { 255 };
            }
        }
        let shape = SdfDrawCommand {
            kind: SdfPrimitiveKind::Shape,
            atlas_set: 0,
            atlas_page: 0,
            atlas_rect: [0, 0, 16, 8],
            quad: rectangle(2.25, 1.75, 70.25, 39.75),
            device_clip: None,
            material: SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
                [0.1, 0.6, 0.3],
                0.55,
                [0.8, 0.2, 0.5],
                0.4,
                0.25,
            )),
        };
        let plan = SdfTilePlan::build(TileGrid::new(73, 41), &[shape], &atlas)
            .expect("axis-aligned shape plan");
        let program = plan.axis_shape_program(0).expect("row program");
        assert!(program.runs.len() < 16 * 8);

        let background = [17, 31, 47, 191].repeat(73 * 41);
        let mut oracle = background.clone();
        plan.execute_scalar_f32_over(&atlas, &mut oracle)
            .expect("sampler oracle");
        let mut candidate = background.clone();
        let stats = plan
            .execute_scalar_f32_over_precomputed_shapes(&atlas, &mut candidate)
            .expect("row-program candidate");
        assert_eq!(candidate, oracle);
        assert!(stats.precomputed_shape_fragment_count > 0);
        assert!(stats.precomputed_shape_span_count > 0);
        assert_eq!(stats.sampled_texel_count, 0);
    }

    #[test]
    fn one_shot_shape_plan_skips_row_program_and_eager_direct_spans() {
        let atlas = PhysicalShapeAtlas {
            payload: [255; 256],
        };
        let shape = SdfDrawCommand {
            kind: SdfPrimitiveKind::Shape,
            atlas_set: 0,
            atlas_page: 0,
            atlas_rect: [0, 0, 16, 8],
            quad: rectangle(2.0, 3.0, 70.0, 39.0),
            device_clip: None,
            material: SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
                [0.1, 0.6, 0.3],
                0.55,
                [0.8, 0.2, 0.5],
                0.4,
                0.25,
            )),
        };
        let plan =
            SdfTilePlan::build_for_one_shot_dynamic_layer(TileGrid::new(73, 41), &[shape], &atlas)
                .expect("one-shot Shape plan");

        assert!(plan.axis_shape_program(0).is_none());
        assert!(plan.direct_axis_shape_spans().is_none());
    }

    #[test]
    fn static_one_shot_keeps_row_program_without_eager_direct_spans() {
        let atlas = PhysicalShapeAtlas {
            payload: [255; 256],
        };
        let shape = SdfDrawCommand {
            kind: SdfPrimitiveKind::Shape,
            atlas_set: 0,
            atlas_page: 0,
            atlas_rect: [0, 0, 16, 8],
            quad: rectangle(2.0, 3.0, 70.0, 39.0),
            device_clip: None,
            material: SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
                [0.1, 0.6, 0.3],
                0.55,
                [0.8, 0.2, 0.5],
                0.4,
                0.25,
            )),
        };
        let cache = ShapeRowProgramCache::default();
        let plan = SdfTilePlan::build_static_one_shot_with_shape_program_cache(
            TileGrid::new(73, 41),
            &[shape],
            &atlas,
            &cache,
        )
        .expect("static one-shot Shape plan");

        assert!(plan.axis_shape_program(0).is_some());
        assert!(plan.direct_axis_shape_spans().is_none());
        let background = [17, 31, 47, 191].repeat(73 * 41);
        let mut oracle = background.clone();
        plan.execute_scalar_f32_over(&atlas, &mut oracle)
            .expect("sampler oracle");
        let mut candidate = background.clone();
        let stats = plan
            .execute_scalar_f32_over_precomputed_shapes(&atlas, &mut candidate)
            .expect("row-program candidate");
        assert_eq!(candidate, oracle);
        assert!(stats.precomputed_shape_fragment_count > 0);
        assert_eq!(stats.sampled_texel_count, 0);

        let mut recolored = shape;
        recolored.material = SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
            [0.9, 0.1, 0.7],
            0.8,
            [0.1, 0.8, 0.2],
            0.3,
            0.25,
        ));
        let recolored_plan = SdfTilePlan::build_static_one_shot_with_shape_program_cache(
            TileGrid::new(73, 41),
            &[recolored],
            &atlas,
            &cache,
        )
        .expect("recolored static one-shot Shape plan");
        assert!(Arc::ptr_eq(
            plan.axis_shape_programs[0]
                .as_ref()
                .expect("original row program"),
            recolored_plan.axis_shape_programs[0]
                .as_ref()
                .expect("recolored row program"),
        ));
        let mut recolored_oracle = background.clone();
        recolored_plan
            .execute_scalar_f32_over(&atlas, &mut recolored_oracle)
            .expect("recolored sampler oracle");
        let mut recolored_candidate = background;
        recolored_plan
            .execute_scalar_f32_over_precomputed_shapes(&atlas, &mut recolored_candidate)
            .expect("recolored row-program candidate");
        assert_eq!(recolored_candidate, recolored_oracle);

        let mut reoutlined = recolored;
        reoutlined.material = SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
            [0.9, 0.1, 0.7],
            0.8,
            [0.1, 0.8, 0.2],
            0.3,
            0.75,
        ));
        let reoutlined_plan = SdfTilePlan::build_static_one_shot_with_shape_program_cache(
            TileGrid::new(73, 41),
            &[reoutlined],
            &atlas,
            &cache,
        )
        .expect("reoutlined static one-shot Shape plan");
        assert!(!Arc::ptr_eq(
            recolored_plan.axis_shape_programs[0]
                .as_ref()
                .expect("recolored row program"),
            reoutlined_plan.axis_shape_programs[0]
                .as_ref()
                .expect("reoutlined row program"),
        ));
        assert_eq!(
            cache
                .source_programs
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .len(),
            1,
        );
        let mut reoutlined_oracle = [17, 31, 47, 191].repeat(73 * 41);
        reoutlined_plan
            .execute_scalar_f32_over(&atlas, &mut reoutlined_oracle)
            .expect("reoutlined sampler oracle");
        let mut reoutlined_candidate = [17, 31, 47, 191].repeat(73 * 41);
        reoutlined_plan
            .execute_scalar_f32_over_precomputed_shapes(&atlas, &mut reoutlined_candidate)
            .expect("reoutlined row-program candidate");
        assert_eq!(reoutlined_candidate, reoutlined_oracle);
    }

    #[test]
    fn transformed_shape_without_axis_alignment_falls_back_to_sampler() {
        let atlas = PhysicalShapeAtlas {
            payload: [255; 256],
        };
        let shape = SdfDrawCommand {
            kind: SdfPrimitiveKind::Shape,
            atlas_set: 0,
            atlas_page: 0,
            atlas_rect: [0, 0, 16, 8],
            quad: [
                Point2::new(2.0, 1.0),
                Point2::new(20.0, 3.0),
                Point2::new(24.0, 14.0),
                Point2::new(6.0, 12.0),
            ],
            device_clip: None,
            material: SdfCommandMaterial::Shape(ShapeSdfMaterial::from_profile_values(
                [0.2, 0.4, 0.8],
                0.7,
                [0.8, 0.1, 0.2],
                0.3,
                0.2,
            )),
        };
        let plan = SdfTilePlan::build(TileGrid::new(32, 16), &[shape], &atlas)
            .expect("transformed shape plan");
        assert!(plan.axis_shape_program(0).is_none());
        let background = [5, 7, 11, 191].repeat(32 * 16);
        let mut oracle = background.clone();
        plan.execute_scalar_f32_over(&atlas, &mut oracle)
            .expect("sampler oracle");
        let mut candidate = background;
        let stats = plan
            .execute_scalar_f32_over_precomputed_shapes(&atlas, &mut candidate)
            .expect("fallback candidate");
        assert_eq!(candidate, oracle);
        assert_eq!(stats.precomputed_shape_fragment_count, 0);
        assert!(stats.sampled_texel_count > 0);
    }
}
