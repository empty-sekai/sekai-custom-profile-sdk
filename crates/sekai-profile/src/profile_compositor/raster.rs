//! Shape and image command rasterizers on the scalar and packet executors, with
//! texel sampling, alpha masks and the lit badge material.

use sekai_profile_core::{
    pixel_sampling, BlendMode, LayerSource, Matrix2d, Rect, SemanticCommandSource, ShapePrimitive,
};

#[cfg(target_arch = "x86_64")]
use super::simd_x86;
use super::{
    axis_aligned_command_clip, blend_pixel, compose_matrix, invert_matrix, mul_div_255_round,
    quantize, transform_point, AxisAlignedClip, BadgeShading, ImageClipGeometry, ImageExecutor,
    ProfileCompositorError, RasterImageStats,
};
use crate::render_object::MappedRenderObject;

/// Samples an alpha mask's coverage at a normalised position within the
/// command's bounds, with the same nearest-neighbour rule as the source.
fn mask_alpha_at(
    mask: MappedRenderObject<'_>,
    u: f32,
    v: f32,
    role: &str,
) -> Result<u8, ProfileCompositorError> {
    let mx = pixel_sampling::nearest_texel(u, mask.entry.width);
    let my = pixel_sampling::nearest_texel(v, mask.entry.height);
    let pixel = object_pixel(mask, mx, my)
        .ok_or_else(|| ProfileCompositorError::InvalidObject(role.into()))?;
    Ok(pixel[3])
}

#[inline]
fn image_clip_contains(
    clip: Option<ImageClipGeometry>,
    bounds: Rect,
    local_x: f32,
    local_y: f32,
) -> bool {
    match clip {
        None => true,
        Some(ImageClipGeometry::Ellipse) => {
            pixel_sampling::ellipse_clip_contains(bounds, local_x, local_y)
        }
        Some(ImageClipGeometry::RoundedRect { radius }) => {
            pixel_sampling::rounded_rect_clip_contains(radius, bounds, local_x, local_y)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn raster_semantic_shape_command(
    destination: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    command: &SemanticCommandSource,
    layer: &LayerSource,
    primitive: &ShapePrimitive,
    fill: [f32; 4],
    gradient: Option<&sekai_profile_core::LinearGradient>,
    stroke: [f32; 4],
    stroke_width: f32,
    translate_y: f32,
    executor: ImageExecutor,
) -> Result<RasterImageStats, ProfileCompositorError> {
    let bounds = command.bounds;
    if !bounds.x.is_finite()
        || !bounds.y.is_finite()
        || !bounds.width.is_finite()
        || !bounds.height.is_finite()
        || bounds.width <= 0.0
        || bounds.height <= 0.0
        || !stroke_width.is_finite()
        || stroke_width < 0.0
    {
        return Err(ProfileCompositorError::InvalidGeometry {
            role: command.role.clone(),
        });
    }
    let mut translated_matrix = command.matrix;
    translated_matrix[5] += translate_y;
    let matrix = compose_matrix(layer.matrix, translated_matrix);
    let inverse = invert_matrix(matrix).ok_or_else(|| ProfileCompositorError::InvalidGeometry {
        role: command.role.clone(),
    })?;
    let command_clip = command
        .clip
        .as_ref()
        .map(|clip| axis_aligned_command_clip(clip, layer.matrix))
        .transpose()
        .map_err(|()| ProfileCompositorError::UnsupportedFeature {
            role: command.role.clone(),
            feature: "non-axis shape clip".into(),
        })?;
    let corners = [
        transform_point(matrix, bounds.x, bounds.y),
        transform_point(matrix, bounds.x + bounds.width, bounds.y),
        transform_point(matrix, bounds.x + bounds.width, bounds.y + bounds.height),
        transform_point(matrix, bounds.x, bounds.y + bounds.height),
    ];
    let min_x = corners
        .iter()
        .map(|value| value.0)
        .fold(f32::INFINITY, f32::min);
    let min_y = corners
        .iter()
        .map(|value| value.1)
        .fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|value| value.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = corners
        .iter()
        .map(|value| value.1)
        .fold(f32::NEG_INFINITY, f32::max);
    let (clip_x0, clip_x1) = command_clip.map_or((0.0, canvas_width as f32), |clip| {
        pixel_sampling::pixel_span(clip.min_x, clip.max_x)
    });
    let (clip_y0, clip_y1) = command_clip.map_or((0.0, canvas_height as f32), |clip| {
        pixel_sampling::pixel_span(clip.min_y, clip.max_y)
    });
    let x0 = min_x.floor().max(clip_x0).clamp(0.0, canvas_width as f32) as u32;
    let y0 = min_y.floor().max(clip_y0).clamp(0.0, canvas_height as f32) as u32;
    let x1 = max_x.ceil().min(clip_x1).clamp(0.0, canvas_width as f32) as u32;
    let y1 = max_y.ceil().min(clip_y1).clamp(0.0, canvas_height as f32) as u32;
    let mut stats = RasterImageStats::default();
    for y in y0..y1 {
        #[cfg(target_arch = "x86_64")]
        if executor == ImageExecutor::Simd && std::arch::is_x86_feature_detected!("fma") {
            let mut x = x0;
            while x < x1 {
                let packet = (x1 - x).min(16);
                let destination_offset = ((y as usize * canvas_width as usize) + x as usize) * 4;
                let active = unsafe {
                    simd_x86::raster_semantic_shape_packet(
                        destination.as_mut_ptr().add(destination_offset),
                        x,
                        y,
                        packet,
                        &inverse,
                        bounds,
                        primitive,
                        fill,
                        gradient,
                        stroke,
                        stroke_width,
                        command.blend_mode,
                    )
                };
                if active != 0 {
                    stats.fragments = stats
                        .fragments
                        .saturating_add(u64::from(active.count_ones()));
                    stats.simd_packets = stats.simd_packets.saturating_add(1);
                }
                x += packet;
            }
            continue;
        }
        for x in x0..x1 {
            let mut accumulated = [0.0f32; 4];
            let mut covered = false;
            for [offset_x, offset_y] in pixel_sampling::SHAPE_SAMPLES {
                let (local_x, local_y) =
                    transform_point(inverse, x as f32 + offset_x, y as f32 + offset_y);
                if !pixel_sampling::shape_contains(primitive, bounds, local_x, local_y, 0.0) {
                    continue;
                }
                covered = true;
                let use_stroke = stroke_width > 0.0
                    && !pixel_sampling::shape_contains(
                        primitive,
                        bounds,
                        local_x,
                        local_y,
                        stroke_width,
                    );
                let color = if use_stroke {
                    stroke
                } else {
                    pixel_sampling::shape_fill(fill, gradient, bounds, local_x, local_y)
                };
                let alpha = color[3].clamp(0.0, 1.0);
                let weight = pixel_sampling::SHAPE_SAMPLE_WEIGHT;
                accumulated[0] += color[0].clamp(0.0, 1.0) * alpha * weight;
                accumulated[1] += color[1].clamp(0.0, 1.0) * alpha * weight;
                accumulated[2] += color[2].clamp(0.0, 1.0) * alpha * weight;
                accumulated[3] += alpha * weight;
            }
            if !covered {
                continue;
            }
            let source = accumulated.map(quantize);
            let offset = ((y as usize * canvas_width as usize) + x as usize) * 4;
            let pixel = destination.get_mut(offset..offset + 4).ok_or(
                ProfileCompositorError::InvalidCanvas {
                    width: canvas_width,
                    height: canvas_height,
                },
            )?;
            blend_pixel(pixel, source, command.blend_mode);
            stats.fragments = stats.fragments.saturating_add(1);
            stats.scalar_fragments = stats.scalar_fragments.saturating_add(1);
        }
    }
    Ok(stats)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn raster_image_command(
    destination: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    source: MappedRenderObject<'_>,
    bounds: Rect,
    uv: Rect,
    tint: [f32; 4],
    matrix: Matrix2d,
    blend_mode: BlendMode,
    role: &str,
    command_clip: Option<AxisAlignedClip>,
    image_clip: Option<ImageClipGeometry>,
    alpha_mask: Option<MappedRenderObject<'_>>,
    badge: Option<BadgeShading<'_>>,
    executor: ImageExecutor,
) -> Result<RasterImageStats, ProfileCompositorError> {
    validate_geometry(bounds, uv, tint, matrix, role)?;
    validate_object(source, role)?;
    if let Some(mask) = alpha_mask {
        validate_object(mask, role)?;
    }
    if let Some(badge) = badge {
        validate_object(badge.normal_map, role)?;
    }
    // The badge material applies the tint while shading, so its output
    // reaches the blender untinted.
    let blend_tint = if badge.is_some() { [1.0; 4] } else { tint };
    if bounds.width == 0.0 || bounds.height == 0.0 {
        return Ok(RasterImageStats::default());
    }
    let inverse = invert_matrix(matrix)
        .ok_or_else(|| ProfileCompositorError::InvalidGeometry { role: role.into() })?;
    let corners = [
        transform_point(matrix, bounds.x, bounds.y),
        transform_point(matrix, bounds.x + bounds.width, bounds.y),
        transform_point(matrix, bounds.x + bounds.width, bounds.y + bounds.height),
        transform_point(matrix, bounds.x, bounds.y + bounds.height),
    ];
    let min_x = corners
        .iter()
        .map(|value| value.0)
        .fold(f32::INFINITY, f32::min);
    let min_y = corners
        .iter()
        .map(|value| value.1)
        .fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|value| value.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = corners
        .iter()
        .map(|value| value.1)
        .fold(f32::NEG_INFINITY, f32::max);
    let (clip_x0, clip_x1) = command_clip.map_or((0.0, canvas_width as f32), |clip| {
        pixel_sampling::pixel_span(clip.min_x, clip.max_x)
    });
    let (clip_y0, clip_y1) = command_clip.map_or((0.0, canvas_height as f32), |clip| {
        pixel_sampling::pixel_span(clip.min_y, clip.max_y)
    });
    let x0 = min_x.floor().max(clip_x0).max(0.0).min(canvas_width as f32) as u32;
    let y0 = min_y
        .floor()
        .max(clip_y0)
        .max(0.0)
        .min(canvas_height as f32) as u32;
    let x1 = max_x.ceil().min(clip_x1).max(0.0).min(canvas_width as f32) as u32;
    let y1 = max_y.ceil().min(clip_y1).max(0.0).min(canvas_height as f32) as u32;
    // A clip that misses the image leaves an empty, possibly inverted, span.
    if x0 >= x1 || y0 >= y1 {
        return Ok(RasterImageStats::default());
    }
    let mut stats = RasterImageStats::default();
    let packet_blend = executor == ImageExecutor::Simd
        && blend_tint
            .iter()
            .all(|value| value.to_bits() == 1.0f32.to_bits());
    // The two fast paths below read the source straight out of the mapped
    // object, so they cannot fold a second texture into the sample. A masked
    // or lit command gathers through the loop instead, where the mask and the
    // material are applied before either blender sees the source.
    let direct_source_sampling = alpha_mask.is_none() && badge.is_none();
    #[cfg(target_arch = "x86_64")]
    let affine_packet_blend = packet_blend
        && direct_source_sampling
        && source.pixels.len() <= i32::MAX as usize
        && source.entry.row_bytes <= i32::MAX as u32;

    if packet_blend
        && direct_source_sampling
        && matrix[0] > 0.0
        && matrix[3] > 0.0
        && matrix[1].to_bits() == 0.0f32.to_bits()
        && matrix[2].to_bits() == 0.0f32.to_bits()
    {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            return Ok(raster_axis_aligned_image_command(
                destination,
                canvas_width,
                source,
                bounds,
                uv,
                inverse,
                blend_mode,
                image_clip,
                x0,
                y0,
                x1,
                y1,
            ));
        }
    }

    for y in y0..y1 {
        let py = y as f32 + pixel_sampling::PIXEL_CENTRE;
        let mut x = x0;
        while x < x1 {
            let packet_end = x.saturating_add(16).min(x1);
            #[cfg(target_arch = "x86_64")]
            if affine_packet_blend {
                let destination_offset = ((y as usize * canvas_width as usize) + x as usize) * 4;
                let destination_packet = destination.get_mut(destination_offset..).ok_or({
                    ProfileCompositorError::InvalidCanvas {
                        width: canvas_width,
                        height: canvas_height,
                    }
                })?;
                let active_mask = unsafe {
                    simd_x86::sample_affine_and_blend_rgba8_packet(
                        destination_packet.as_mut_ptr(),
                        source.pixels.as_ptr(),
                        source.entry.row_bytes as i32,
                        source.entry.width,
                        source.entry.height,
                        &inverse,
                        bounds,
                        uv,
                        x,
                        y,
                        packet_end - x,
                        blend_mode,
                        image_clip,
                    )
                };
                let active_fragments = u64::from(active_mask.count_ones());
                stats.fragments = stats.fragments.saturating_add(active_fragments);
                if active_mask != 0 {
                    stats.simd_packets = stats.simd_packets.saturating_add(1);
                }
                x = packet_end;
                continue;
            }
            let mut source_pixels = [0u32; 16];
            let mut active_mask = 0u16;
            for packet_x in x..packet_end {
                let px = packet_x as f32 + pixel_sampling::PIXEL_CENTRE;
                let (local_x, local_y) = transform_point(inverse, px, py);
                let u = (local_x - bounds.x) / bounds.width;
                let v = (local_y - bounds.y) / bounds.height;
                if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                    continue;
                }
                if !image_clip_contains(image_clip, bounds, local_x, local_y) {
                    continue;
                }
                let image_u = uv.x + u * uv.width;
                let image_v = uv.y + v * uv.height;
                let sx = pixel_sampling::nearest_texel(image_u, source.entry.width);
                let sy = pixel_sampling::nearest_texel(image_v, source.entry.height);
                let mut source_pixel = object_pixel(source, sx, sy)
                    .ok_or_else(|| ProfileCompositorError::InvalidObject(role.into()))?;
                if let Some(badge) = badge {
                    source_pixel =
                        badge_fragment(source_pixel, badge, image_u, image_v, tint, role)?;
                }
                if let Some(mask) = alpha_mask {
                    // The mask spans the command bounds, so it is sampled at the
                    // same normalised position as the source. Its alpha scales
                    // the premultiplied source, which is what a `DstIn` draw of
                    // the mask over the same bounds produces.
                    let coverage = mask_alpha_at(mask, u, v, role)?;
                    if coverage == 0 {
                        continue;
                    }
                    if coverage != u8::MAX {
                        source_pixel = std::array::from_fn(|channel| {
                            mul_div_255_round(source_pixel[channel], coverage)
                        });
                    }
                }
                let lane = usize::try_from(packet_x - x).unwrap_or_default();
                source_pixels[lane] = u32::from_le_bytes(source_pixel);
                active_mask |= 1u16 << lane;
            }

            let active_fragments = u64::from(active_mask.count_ones());
            if active_mask != 0 && packet_blend {
                let destination_offset = ((y as usize * canvas_width as usize) + x as usize) * 4;
                let destination_packet = destination.get_mut(destination_offset..).ok_or({
                    ProfileCompositorError::InvalidCanvas {
                        width: canvas_width,
                        height: canvas_height,
                    }
                })?;
                #[cfg(target_arch = "x86_64")]
                unsafe {
                    simd_x86::blend_rgba8_packet(
                        destination_packet.as_mut_ptr(),
                        source_pixels.as_ptr(),
                        active_mask,
                        blend_mode,
                    );
                }
                stats.simd_packets = stats.simd_packets.saturating_add(1);
            } else {
                for lane in 0..usize::try_from(packet_end - x).unwrap_or_default() {
                    if active_mask & (1u16 << lane) == 0 {
                        continue;
                    }
                    let packet_x = x + lane as u32;
                    let destination_offset =
                        ((y as usize * canvas_width as usize) + packet_x as usize) * 4;
                    let destination_pixel = destination
                        .get_mut(destination_offset..destination_offset + 4)
                        .ok_or(ProfileCompositorError::InvalidCanvas {
                            width: canvas_width,
                            height: canvas_height,
                        })?;
                    let source_pixel = apply_tint(source_pixels[lane].to_le_bytes(), blend_tint);
                    blend_pixel(destination_pixel, source_pixel, blend_mode);
                }
                stats.scalar_fragments = stats.scalar_fragments.saturating_add(active_fragments);
            }
            stats.fragments = stats.fragments.saturating_add(active_fragments);
            x = packet_end;
        }
    }
    Ok(stats)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
#[allow(clippy::too_many_arguments)]
unsafe fn raster_axis_aligned_image_command(
    destination: &mut [u8],
    canvas_width: u32,
    source: MappedRenderObject<'_>,
    bounds: Rect,
    uv: Rect,
    inverse: Matrix2d,
    blend_mode: BlendMode,
    image_clip: Option<ImageClipGeometry>,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
) -> RasterImageStats {
    const INACTIVE_SOURCE_COLUMN: u32 = u32::MAX;

    let source_row_bytes = source.entry.row_bytes as usize;
    let canvas_row_bytes = canvas_width as usize * 4;
    let mut source_columns = Vec::with_capacity((x1 - x0) as usize);
    let mut local_columns = Vec::with_capacity((x1 - x0) as usize);
    for x in x0..x1 {
        let px = x as f32 + pixel_sampling::PIXEL_CENTRE;
        let local_x = inverse[0].mul_add(px, inverse[4]);
        local_columns.push(local_x);
        let u = (local_x - bounds.x) / bounds.width;
        if !(0.0..1.0).contains(&u) {
            source_columns.push(INACTIVE_SOURCE_COLUMN);
            continue;
        }
        source_columns.push(pixel_sampling::nearest_texel(
            uv.x + u * uv.width,
            source.entry.width,
        ));
    }

    struct AxisPacket {
        destination_x: u32,
        column_offset: usize,
        active_mask: u16,
        first_source_x: u32,
        contiguous: bool,
    }
    let mut packets = Vec::with_capacity((x1 - x0).div_ceil(16) as usize);
    let mut packet_x = x0;
    while packet_x < x1 {
        let packet_end = packet_x.saturating_add(16).min(x1);
        let packet_length = (packet_end - packet_x) as usize;
        let column_offset = (packet_x - x0) as usize;
        let columns = &source_columns[column_offset..column_offset + packet_length];
        let mut active_mask = 0u16;
        let mut contiguous = true;
        let mut first_source_x = 0u32;
        let mut active_count = 0usize;
        for (lane, &source_x) in columns.iter().enumerate() {
            if source_x == INACTIVE_SOURCE_COLUMN {
                contiguous = false;
                continue;
            }
            if active_count == 0 {
                first_source_x = source_x;
                contiguous &= lane == 0;
            } else {
                contiguous &= source_x == first_source_x + active_count as u32;
            }
            active_mask |= 1u16 << lane;
            active_count += 1;
        }
        let expected_mask = if active_count == 16 {
            u16::MAX
        } else {
            (1u16 << active_count) - 1
        };
        packets.push(AxisPacket {
            destination_x: packet_x,
            column_offset,
            active_mask,
            first_source_x,
            contiguous: contiguous && active_mask == expected_mask,
        });
        packet_x = packet_end;
    }

    let destination_base = destination.as_mut_ptr();
    let source_base = source.pixels.as_ptr();
    let mut stats = RasterImageStats::default();
    for y in y0..y1 {
        let py = y as f32 + pixel_sampling::PIXEL_CENTRE;
        let (_, local_y) = transform_point(inverse, pixel_sampling::PIXEL_CENTRE, py);
        let v = (local_y - bounds.y) / bounds.height;
        if !(0.0..1.0).contains(&v) {
            continue;
        }
        let source_y =
            pixel_sampling::nearest_texel(uv.y + v * uv.height, source.entry.height) as usize;
        let source_row = source_base.add(source_y * source_row_bytes);
        let destination_row = destination_base.add(y as usize * canvas_row_bytes);

        for packet in &packets {
            let active_mask = if let Some(clip) = image_clip {
                simd_x86::axis_aligned_image_clip_mask(
                    local_columns.as_ptr().add(packet.column_offset),
                    local_y,
                    bounds,
                    clip,
                    packet.active_mask,
                )
            } else {
                packet.active_mask
            };
            if active_mask != 0 {
                let destination_packet = destination_row.add(packet.destination_x as usize * 4);
                if packet.contiguous {
                    let source_packet = source_row
                        .add(packet.first_source_x as usize * 4)
                        .cast::<u32>();
                    simd_x86::blend_rgba8_packet(
                        destination_packet,
                        source_packet,
                        active_mask,
                        blend_mode,
                    );
                } else {
                    simd_x86::gather_and_blend_rgba8_packet(
                        destination_packet,
                        source_row,
                        source_columns.as_ptr().add(packet.column_offset),
                        active_mask,
                        blend_mode,
                    );
                }
                stats.fragments = stats
                    .fragments
                    .saturating_add(u64::from(active_mask.count_ones()));
                stats.simd_packets = stats.simd_packets.saturating_add(1);
            }
        }
    }
    stats
}

fn validate_geometry(
    bounds: Rect,
    uv: Rect,
    tint: [f32; 4],
    matrix: Matrix2d,
    role: &str,
) -> Result<(), ProfileCompositorError> {
    let finite = [
        bounds.x,
        bounds.y,
        bounds.width,
        bounds.height,
        uv.x,
        uv.y,
        uv.width,
        uv.height,
    ]
    .into_iter()
    .chain(tint)
    .chain(matrix)
    .all(f32::is_finite);
    if !finite || bounds.width < 0.0 || bounds.height < 0.0 || uv.width <= 0.0 || uv.height <= 0.0 {
        return Err(ProfileCompositorError::InvalidGeometry { role: role.into() });
    }
    Ok(())
}

fn validate_object(
    object: MappedRenderObject<'_>,
    role: &str,
) -> Result<(), ProfileCompositorError> {
    let expected = u64::from(object.entry.row_bytes).checked_mul(u64::from(object.entry.height));
    if object.entry.width == 0
        || object.entry.height == 0
        || object.entry.row_bytes < object.entry.width.saturating_mul(4)
        || expected.and_then(|value| usize::try_from(value).ok()) != Some(object.pixels.len())
    {
        return Err(ProfileCompositorError::InvalidObject(role.into()));
    }
    Ok(())
}

fn object_pixel(object: MappedRenderObject<'_>, x: u32, y: u32) -> Option<[u8; 4]> {
    let offset = usize::try_from(y)
        .ok()?
        .checked_mul(usize::try_from(object.entry.row_bytes).ok()?)?
        .checked_add(usize::try_from(x).ok()?.checked_mul(4)?)?;
    let pixel = object.pixels.get(offset..offset.checked_add(4)?)?;
    Some([pixel[0], pixel[1], pixel[2], pixel[3]])
}

/// One lit badge fragment: the image texel `albedo` (premultiplied, as the
/// compositor stores it) shaded with the normal map at the same image UV.
fn badge_fragment(
    albedo: [u8; 4],
    badge: BadgeShading<'_>,
    image_u: f32,
    image_v: f32,
    vertex_color: [f32; 4],
    role: &str,
) -> Result<[u8; 4], ProfileCompositorError> {
    let normal = sample_straight_bilinear(badge.normal_map, image_u, image_v, role)?;
    Ok(sekai_profile_core::badge_material::shade(
        straight_texel(albedo),
        normal,
        badge.tangent,
        vertex_color,
    )
    .map(quantize))
}

/// A premultiplied texel as straight RGBA in `0..=1`; a transparent texel has
/// no colour.
fn straight_texel(texel: [u8; 4]) -> [f32; 4] {
    if texel[3] == 0 {
        return [0.0; 4];
    }
    let alpha = f32::from(texel[3]);
    [
        (f32::from(texel[0]) / alpha).min(1.0),
        (f32::from(texel[1]) / alpha).min(1.0),
        (f32::from(texel[2]) / alpha).min(1.0),
        alpha / 255.0,
    ]
}

/// Bilinear, edge-clamped sample of an object's straight texels at a UV in
/// `0..=1` with v down, as a GPU samples a straight-alpha texture with linear
/// filtering.
fn sample_straight_bilinear(
    object: MappedRenderObject<'_>,
    u: f32,
    v: f32,
    role: &str,
) -> Result<[f32; 4], ProfileCompositorError> {
    let (x0, x1, fx) = pixel_sampling::bilinear_taps(u, object.entry.width);
    let (y0, y1, fy) = pixel_sampling::bilinear_taps(v, object.entry.height);
    let texel = |x: u32, y: u32| {
        object_pixel(object, x, y)
            .map(straight_texel)
            .ok_or_else(|| ProfileCompositorError::InvalidObject(role.into()))
    };
    let lerp = |a: [f32; 4], b: [f32; 4], t: f32| -> [f32; 4] {
        std::array::from_fn(|channel| a[channel] * (1.0 - t) + b[channel] * t)
    };
    let top = lerp(texel(x0, y0)?, texel(x1, y0)?, fx);
    let bottom = lerp(texel(x0, y1)?, texel(x1, y1)?, fx);
    Ok(lerp(top, bottom, fy))
}

fn apply_tint(source: [u8; 4], tint: [f32; 4]) -> [u8; 4] {
    if tint.iter().all(|value| value.to_bits() == 1.0f32.to_bits()) {
        return source;
    }
    let tint_alpha = tint[3].clamp(0.0, 1.0);
    [
        quantize(f32::from(source[0]) / 255.0 * tint[0].clamp(0.0, 1.0) * tint_alpha),
        quantize(f32::from(source[1]) / 255.0 * tint[1].clamp(0.0, 1.0) * tint_alpha),
        quantize(f32::from(source[2]) / 255.0 * tint[2].clamp(0.0, 1.0) * tint_alpha),
        quantize(f32::from(source[3]) / 255.0 * tint_alpha),
    ]
}
