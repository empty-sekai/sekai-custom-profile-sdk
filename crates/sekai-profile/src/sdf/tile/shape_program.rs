//! Axis-aligned Shape row programs: per-row coverage runs built from Shape atlas
//! texels, their cache, and the span iteration the executors share.

use std::sync::Arc;

use super::{
    AxisAlignedShapeProgram, MixedSdfAtlasSource, PlannedCommand, SdfAtlasSource, SdfAtlasTexel,
    SdfCommandMaterial, SdfPrimitiveKind, SdfTileError, ShapeCoverageRun, ShapeRowProgramCache,
    ShapeRowProgramKey, ShapeRowProgramPrewarmReport, ShapeSourceProgram, ShapeSourceProgramKey,
    ShapeTexelRun,
};
use crate::sdf::atlas::MappedSdfAtlasSet;
use crate::sdf::shape::{shade_shape_coverages, texel_coverage};
use crate::sdf::shape_atlas::MappedShapeSdfAtlas;

impl AxisAlignedShapeProgram {
    pub(super) fn resident_bytes(&self) -> usize {
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

    pub(super) fn get_or_build(
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

pub(super) fn build_axis_aligned_shape_program_from_atlas(
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

pub(super) fn rgba_f32_bits_eq(left: [f32; 4], right: [f32; 4]) -> bool {
    left.into_iter()
        .zip(right)
        .all(|(left, right)| left.to_bits() == right.to_bits())
}

pub(super) fn for_each_axis_shape_span(
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
