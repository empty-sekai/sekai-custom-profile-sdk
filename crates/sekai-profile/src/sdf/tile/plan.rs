//! Tile plan construction: command validation, affine planning, scanline span binning,
//! occlusion measurement and the direct axis-aligned Shape spans.

use std::sync::Arc;

use sekai_profile_core::pixel_sampling::{self, PIXEL_CENTRE, TEXEL_CENTRE};

use super::{
    build_axis_aligned_shape_program_from_atlas, for_each_axis_shape_span, rgba_f32_bits_eq,
    AxisAlignedShapeProgram, DirectAxisShapePlan, DirectAxisShapeSpan, PixelOcclusionMask,
    PlannedCommand, Point2, SdfAtlasSource, SdfCommandMaterial, SdfDeviceClip, SdfDrawCommand,
    SdfOcclusionStats, SdfPlanStats, SdfPrimitiveKind, SdfTileError, SdfTilePlan,
    ShapeRowProgramCache, TileGrid, TileSpan,
};
use crate::sdf::shape::coverage_terms;

const MAX_PARALLELOGRAM_ERROR: f32 = 0.01;

impl SdfTilePlan {
    pub fn build(
        grid: TileGrid,
        commands: &[SdfDrawCommand],
        atlas: &impl SdfAtlasSource,
    ) -> Result<Self, SdfTileError> {
        Self::build_impl(grid, commands, atlas, None, true, true)
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
        let had_direct_axis_shape = self.direct_axis_shape.is_some();
        let visible = Self {
            grid: self.grid,
            commands: self.commands.clone(),
            axis_shape_programs: self.axis_shape_programs.clone(),
            direct_axis_shape: None,
            spans,
            tile_offsets,
            stats,
        };
        let visible = if had_direct_axis_shape {
            visible.prepare_direct_axis_shape()?
        } else {
            visible
        };
        Ok((visible, occlusion))
    }

    pub(super) fn axis_shape_program(&self, command: usize) -> Option<&AxisAlignedShapeProgram> {
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
            let (face_offset, scale) = coverage_terms(material.face_threshold, material.sharpness);
            let (outline_offset, _) =
                coverage_terms(material.outline_threshold, material.sharpness);
            (face_offset, outline_offset, scale)
        }
        SdfCommandMaterial::Text(_) => (0.0, 0.0, 0.0),
    };
    Ok(PlannedCommand {
        source,
        tx_dx: atlas_width * a_dx,
        tx_dy: atlas_width * a_dy,
        tx_c: atlas_width.mul_add(a_c, atlas_x - TEXEL_CENTRE),
        ty_dx: atlas_height * b_dx,
        ty_dy: atlas_height * b_dy,
        ty_c: atlas_height.mul_add(b_c, atlas_y - TEXEL_CENTRE),
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
    let (clip_x0, clip_x1) = device_clip.map_or((0.0, grid.canvas_width as f32), |clip| {
        pixel_sampling::pixel_span(clip.min_x, clip.max_x)
    });
    let (clip_y0, clip_y1) = device_clip.map_or((0.0, grid.canvas_height as f32), |clip| {
        pixel_sampling::pixel_span(clip.min_y, clip.max_y)
    });
    let clip_x0 = clip_x0.clamp(0.0, grid.canvas_width as f32) as u32;
    let clip_y0 = clip_y0.clamp(0.0, grid.canvas_height as f32) as u32;
    let clip_x1 = clip_x1.clamp(0.0, grid.canvas_width as f32) as u32;
    let clip_y1 = clip_y1.clamp(0.0, grid.canvas_height as f32) as u32;
    if clip_x0 >= clip_x1 || clip_y0 >= clip_y1 {
        return Ok(());
    }
    if let Some((min_x, min_y, max_x, max_y)) = axis_aligned_quad_bounds(quad) {
        let (first_y, end_y) = pixel_sampling::pixel_span(min_y, max_y);
        let first_y = (first_y.clamp(0.0, grid.canvas_height as f32) as u32).max(clip_y0);
        let end_y = (end_y.clamp(0.0, grid.canvas_height as f32) as u32).min(clip_y1);
        let (x0, x1) = pixel_sampling::pixel_span(min_x, max_x);
        let x0 = (x0.clamp(0.0, grid.canvas_width as f32) as u32).max(clip_x0);
        let x1 = (x1.clamp(0.0, grid.canvas_width as f32) as u32).min(clip_x1);
        if x0 < x1 {
            for y in first_y..end_y {
                push_scanline_spans(
                    grid,
                    command_index,
                    kind,
                    ScanlineSpan { y, x0, x1 },
                    per_tile,
                    stats,
                )?;
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
    let (first_y, end_y) = pixel_sampling::pixel_span(min_y, max_y);
    let first_y = (first_y.clamp(0.0, grid.canvas_height as f32) as u32).max(clip_y0);
    let end_y = (end_y.clamp(0.0, grid.canvas_height as f32) as u32).min(clip_y1);
    for y in first_y..end_y {
        let py = y as f32 + PIXEL_CENTRE;
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
        let (x0, x1) = pixel_sampling::pixel_span(min_x, max_x);
        let x0 = (x0.clamp(0.0, grid.canvas_width as f32) as u32).max(clip_x0);
        let x1 = (x1.clamp(0.0, grid.canvas_width as f32) as u32).min(clip_x1);
        if x0 >= x1 {
            continue;
        }
        push_scanline_spans(
            grid,
            command_index,
            kind,
            ScanlineSpan { y, x0, x1 },
            per_tile,
            stats,
        )?;
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

/// Covered pixels `x0..x1` of device row `y`, already clipped to the canvas
/// and the command's device clip; `x0 < x1`.
#[derive(Clone, Copy)]
struct ScanlineSpan {
    y: u32,
    x0: u32,
    x1: u32,
}

/// Splits one covered scanline into the tile spans of every tile it crosses
/// and counts its fragments and spans.
fn push_scanline_spans(
    grid: TileGrid,
    command_index: u32,
    kind: SdfPrimitiveKind,
    span: ScanlineSpan,
    per_tile: &mut [Vec<TileSpan>],
    stats: &mut SdfPlanStats,
) -> Result<(), SdfTileError> {
    let ScanlineSpan { y, x0, x1 } = span;
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
