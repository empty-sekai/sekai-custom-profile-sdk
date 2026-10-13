//! Plan execution: the scalar FP32 reference executor with its bilinear sampling,
//! shading and source-over blending, and the AVX-512 entry points.

use sekai_profile_core::pixel_sampling::PIXEL_CENTRE;
use sekai_profile_core::sdf_material::TMP_MIN_SHADER_SCALE;

#[cfg(target_arch = "x86_64")]
use super::simd_x86;
use super::{
    for_each_axis_shape_span, SdfAccumulationMode, SdfAtlasSource, SdfAtlasTexel,
    SdfCommandMaterial, SdfDestination, SdfDrawCommand, SdfExecutionStats, SdfMaterial,
    SdfPrimitiveKind, SdfTileError, SdfTilePlan,
};
use crate::sdf::shape::{shade_shape, ShapeSdfTexel};

impl SdfTilePlan {
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
                let py = y as f32 + PIXEL_CENTRE;
                for local_x in span.x0..span.x1 {
                    let x = origin_x + u32::from(local_x);
                    let px = x as f32 + PIXEL_CENTRE;
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
                    // Re-quantizing the whole tile needlessly perturbs destination
                    // bytes other elements drew and makes interleaved image/SDF
                    // composition depend on tile dimensions. Duplicate span writes are harmless:
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
    /// pages directly. The executor is intentionally hardware-specific and
    /// fails closed when AVX-512F/FMA/BW/VBMI or physical atlas views are absent.
    pub fn execute_simd(
        &self,
        atlas: &impl SdfAtlasSource,
        clear: [u8; 4],
        output: &mut [u8],
        accumulation: SdfAccumulationMode,
    ) -> Result<SdfExecutionStats, SdfTileError> {
        #[cfg(target_arch = "x86_64")]
        {
            if !(std::arch::is_x86_feature_detected!("avx512f")
                && std::arch::is_x86_feature_detected!("avx512bw")
                && std::arch::is_x86_feature_detected!("avx512vbmi")
                && std::arch::is_x86_feature_detected!("fma"))
            {
                return Err(SdfTileError::SimdUnavailable);
            }
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
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = (atlas, clear, output, accumulation);
            Err(SdfTileError::SimdUnavailable)
        }
    }

    /// AVX-512 equivalent of [`Self::execute_scalar_f32_over`]. The executor
    /// loads only touched destination tiles and preserves all untouched bytes.
    pub fn execute_simd_f32_over(
        &self,
        atlas: &impl SdfAtlasSource,
        output: &mut [u8],
    ) -> Result<SdfExecutionStats, SdfTileError> {
        #[cfg(target_arch = "x86_64")]
        {
            if !(std::arch::is_x86_feature_detected!("avx512f")
                && std::arch::is_x86_feature_detected!("avx512bw")
                && std::arch::is_x86_feature_detected!("avx512vbmi")
                && std::arch::is_x86_feature_detected!("fma"))
            {
                return Err(SdfTileError::SimdUnavailable);
            }
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
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = (atlas, output);
            Err(SdfTileError::SimdUnavailable)
        }
    }
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
            Ok(shade_shape(texel, material))
        }
        _ => Err(SdfTileError::MaterialKindMismatch { command: 0 }),
    }
}

pub(super) fn shade_text(material: SdfMaterial, sdf: f32) -> [f32; 4] {
    let face_t = material
        .face_scale
        .max(TMP_MIN_SHADER_SCALE)
        .mul_add(sdf, -material.face_bias)
        .clamp(0.0, 1.0);
    let outline_t = material
        .outline_scale
        .max(TMP_MIN_SHADER_SCALE)
        .mul_add(sdf, -material.outline_bias)
        .clamp(0.0, 1.0);
    let outline_weight = outline_t * (1.0 - material.face[3] * face_t);
    let vertex_alpha = material.vertex_alpha.clamp(0.0, 1.0);
    std::array::from_fn(|channel| {
        material.outline[channel].mul_add(outline_weight, material.face[channel] * face_t)
            * vertex_alpha
    })
}

pub(super) fn source_over_rgba8(destination: &mut [f32; 4], source: [f32; 4]) {
    // Floating-point source-over, then the RGBA8 surface writeback between
    // ordered draws.
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

pub(super) fn quantize(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
