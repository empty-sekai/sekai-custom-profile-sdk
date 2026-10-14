//! Software raster of uGUI text commands.
//!
//! The text is laid out by [`sekai_profile_core::ugui_text::layout`]
//! with FreeType glyphs from its face chain at
//! [`ugui_text::CARD_PIXELS_PER_UNIT`]; a family of the chain without an
//! installed font file fails the command. Its backdrop is drawn first, as a solid
//! rectangle the way shapes are drawn. Each glyph quad then maps its padded
//! glyph cell onto the canvas: a pixel is covered when its centre lies inside
//! the quad, it takes the cell's coverage at its centre
//! ([`ugui_text::cell_coverage`]), and it blends the vertex colour times that
//! coverage.

use sekai_profile_core::pixel_sampling::{pixel_span, PIXEL_CENTRE};
use sekai_profile_core::ugui_text::{self, UguiGlyph, UguiTextSource};
use sekai_profile_core::{BlendMode, LayerSource, Matrix2d, SemanticCommandSource, ShapePrimitive};

use super::{
    axis_aligned_command_clip, blend_pixel, compose_matrix, invert_matrix, quantize,
    raster_semantic_shape_command, transform_point, AxisAlignedClip, ImageExecutor,
    ProfileCompositorError, RasterImageStats, RgbaSurface, SemanticShape,
};
use crate::text::ugui_glyphs::UguiGlyphs;

pub(super) fn raster_ugui_text_command(
    mut surface: RgbaSurface<'_>,
    command: &SemanticCommandSource,
    layer: &LayerSource,
    source: &UguiTextSource,
    translate_y: f32,
    executor: ImageExecutor,
) -> Result<RasterImageStats, ProfileCompositorError> {
    // Every face of the chain is required, whether or not the text needs it.
    let mut glyphs = UguiGlyphs::for_chain(&source.face_chain()).map_err(|family| {
        ProfileCompositorError::MissingFont {
            role: command.role.clone(),
            family,
        }
    })?;
    let mesh = ugui_text::layout(source, ugui_text::CARD_PIXELS_PER_UNIT, &mut glyphs);
    if let Some(reason) = glyphs.error() {
        return Err(ProfileCompositorError::GlyphRaster {
            role: command.role.clone(),
            reason: reason.into(),
        });
    }
    let mut stats = RasterImageStats::default();
    if let Some(backdrop) = &source.backdrop {
        let mut backdrop_command = command.clone();
        backdrop_command.bounds = backdrop.resolved_rect(mesh.preferred_width);
        let raster = raster_semantic_shape_command(
            surface.reborrow(),
            &backdrop_command,
            layer,
            SemanticShape {
                primitive: &ShapePrimitive::Rect,
                fill: backdrop.color,
                gradient: None,
                stroke: [0.0; 4],
                stroke_width: 0.0,
            },
            translate_y,
            executor,
        )?;
        stats.fragments = stats.fragments.saturating_add(raster.fragments);
        stats.simd_packets = stats.simd_packets.saturating_add(raster.simd_packets);
        stats.scalar_fragments = stats
            .scalar_fragments
            .saturating_add(raster.scalar_fragments);
    }

    let mut command_matrix = command.matrix;
    command_matrix[5] += translate_y;
    let device = compose_matrix(
        compose_matrix(layer.matrix, command_matrix),
        source.node_matrix,
    );
    let clip = command
        .clip
        .as_ref()
        .map(|clip| axis_aligned_command_clip(clip, layer.matrix))
        .transpose()
        .map_err(|()| ProfileCompositorError::UnsupportedFeature {
            role: command.role.clone(),
            feature: "non-axis text clip".into(),
        })?;
    let alpha = source.color[3].clamp(0.0, 1.0);
    let color = [
        source.color[0].clamp(0.0, 1.0) * alpha,
        source.color[1].clamp(0.0, 1.0) * alpha,
        source.color[2].clamp(0.0, 1.0) * alpha,
        alpha,
    ];
    let mut target = GlyphTarget {
        pixels: surface.pixels,
        width: surface.width,
        height: surface.height,
        clip,
        blend_mode: command.blend_mode,
    };
    for quad in &mesh.quads {
        let Some(glyph) = &quad.glyph else {
            continue;
        };
        let corners = quad
            .corners
            .map(|[x, y]| transform_point(device, x, y))
            .map(|(x, y)| [x, y]);
        let fragments = target.draw(corners, glyph, mesh.cell_padding, color)?;
        stats.fragments = stats.fragments.saturating_add(fragments);
        stats.scalar_fragments = stats.scalar_fragments.saturating_add(fragments);
    }
    Ok(stats)
}

struct GlyphTarget<'a> {
    pixels: &'a mut [u8],
    width: u32,
    height: u32,
    clip: Option<AxisAlignedClip>,
    blend_mode: BlendMode,
}

impl GlyphTarget<'_> {
    /// Draws one glyph cell whose top-left, top-right, bottom-right and
    /// bottom-left corners land on `corners` (canvas pixels). `color` is
    /// premultiplied. Returns the number of pixels blended.
    fn draw(
        &mut self,
        corners: [[f32; 2]; 4],
        glyph: &UguiGlyph,
        padding: u32,
        color: [f32; 4],
    ) -> Result<u64, ProfileCompositorError> {
        let cell_width = (glyph.width + 2 * padding) as f32;
        let cell_height = (glyph.rows + 2 * padding) as f32;
        let [origin, right, _, down] = corners;
        // Cell texels to canvas pixels: corner 0 is the cell origin, corner 1
        // the far end of its first row and corner 3 of its first column.
        let to_canvas: Matrix2d = [
            (right[0] - origin[0]) / cell_width,
            (right[1] - origin[1]) / cell_width,
            (down[0] - origin[0]) / cell_height,
            (down[1] - origin[1]) / cell_height,
            origin[0],
            origin[1],
        ];
        let Some(to_cell) = invert_matrix(to_canvas) else {
            return Ok(0);
        };
        let min = |axis: usize| {
            corners
                .iter()
                .map(|c| c[axis])
                .fold(f32::INFINITY, f32::min)
        };
        let max = |axis: usize| {
            corners
                .iter()
                .map(|c| c[axis])
                .fold(f32::NEG_INFINITY, f32::max)
        };
        let (clip_x0, clip_x1) = self.clip.map_or((0.0, self.width as f32), |clip| {
            pixel_span(clip.min_x, clip.max_x)
        });
        let (clip_y0, clip_y1) = self.clip.map_or((0.0, self.height as f32), |clip| {
            pixel_span(clip.min_y, clip.max_y)
        });
        let x0 = min(0).floor().max(clip_x0).clamp(0.0, self.width as f32) as u32;
        let y0 = min(1).floor().max(clip_y0).clamp(0.0, self.height as f32) as u32;
        let x1 = max(0).ceil().min(clip_x1).clamp(0.0, self.width as f32) as u32;
        let y1 = max(1).ceil().min(clip_y1).clamp(0.0, self.height as f32) as u32;
        let mut fragments = 0u64;
        for y in y0..y1 {
            for x in x0..x1 {
                let (u, v) =
                    transform_point(to_cell, x as f32 + PIXEL_CENTRE, y as f32 + PIXEL_CENTRE);
                if !(0.0..cell_width).contains(&u) || !(0.0..cell_height).contains(&v) {
                    continue;
                }
                let coverage = ugui_text::cell_coverage(glyph, padding, u, v);
                if coverage <= 0.0 {
                    continue;
                }
                let source = color.map(|channel| quantize(channel * coverage));
                let offset = (y as usize * self.width as usize + x as usize) * 4;
                let pixel = self.pixels.get_mut(offset..offset + 4).ok_or(
                    ProfileCompositorError::InvalidCanvas {
                        width: self.width,
                        height: self.height,
                    },
                )?;
                blend_pixel(pixel, source, self.blend_mode);
                fragments += 1;
            }
        }
        Ok(fragments)
    }
}

#[cfg(test)]
mod tests;
