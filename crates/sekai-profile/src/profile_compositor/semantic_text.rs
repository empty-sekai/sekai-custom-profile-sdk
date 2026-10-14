//! Text commands: semantic SDF text drawn through the tile plan, and the
//! live-master progress digits drawn as plain text.

use sekai_profile_core::{
    FontRole, LayerSource, SemanticCommandPayload, SemanticCommandSource, TextSource,
};

use super::{
    axis_aligned_command_clip, compose_matrix, ImageExecutor, ProfileCompositorError,
    SemanticSdfContext,
};

pub(super) fn semantic_text_glyph<'a>(
    context: SemanticSdfContext<'a>,
    font_id: i32,
    codepoint: u32,
) -> Option<(
    u16,
    &'a crate::sdf::atlas::MappedSdfAtlas,
    &'a crate::sdf::atlas::SdfAtlasGlyphManifest,
)> {
    let primary_family = context.md.resolve_font_or_default(font_id)?;
    context
        .text_atlases
        .profile_glyph_for_font_family(&primary_family, codepoint)
}

pub(super) fn is_live_master_progress_command(command: &SemanticCommandSource) -> bool {
    command.role.starts_with("honor-") && command.role.ends_with("-progress")
}

pub(super) fn render_live_master_progress_text(
    command: &SemanticCommandSource,
    layer: &LayerSource,
    translate_y: f32,
    destination: &mut [u8],
    width: u32,
    height: u32,
) -> Result<(), ProfileCompositorError> {
    let SemanticCommandPayload::Text { source, size, .. } = &command.payload else {
        return Err(ProfileCompositorError::SemanticSdf {
            role: command.role.clone(),
            reason: "live-master progress command is not text".into(),
        });
    };
    let text = match source {
        TextSource::Authored { value }
        | TextSource::ProfileField { value, .. }
        | TextSource::MasterData { value, .. }
        | TextSource::Localized { value, .. } => value,
    };
    let lowered_general = command.render_placement.is_some();
    let font_size = if lowered_general { *size * 2.0 } else { *size };
    let baseline = command
        .render_placement
        .as_ref()
        .and_then(|placement| placement.baseline)
        .map(|value| value * 2.0)
        .unwrap_or(font_size / 2.0);
    let mut local_matrix = command.matrix;
    local_matrix[5] += translate_y;
    // A placed honor may be scaled or rotated; the digits follow its full
    // transform with the baseline in the command's local space.
    let matrix = compose_matrix(layer.matrix, local_matrix);
    let rendered = crate::text::simple_raster::draw_centered_white_text(
        destination,
        width,
        height,
        crate::profile_compositor::theme::fonts::LIVE_MASTER_PROGRESS,
        text,
        20.0,
        matrix,
        baseline,
    )
    .map_err(|reason| ProfileCompositorError::SemanticSdf {
        role: command.role.clone(),
        reason,
    })?;
    if !rendered {
        return Err(ProfileCompositorError::SemanticSdf {
            role: command.role.clone(),
            reason: "live-master progress typeface is unavailable".into(),
        });
    }
    Ok(())
}

pub(super) fn render_semantic_text_command(
    command: &SemanticCommandSource,
    layer: &LayerSource,
    translate_y: f32,
    context: SemanticSdfContext<'_>,
    destination: &mut [u8],
    width: u32,
    height: u32,
    executor: ImageExecutor,
) -> Result<crate::sdf::tile::SdfExecutionStats, ProfileCompositorError> {
    render_semantic_text_commands(
        &[(command, layer, translate_y)],
        context,
        destination,
        width,
        height,
        executor,
    )
}

pub(super) fn render_semantic_text_commands(
    commands: &[(&SemanticCommandSource, &LayerSource, f32)],
    context: SemanticSdfContext<'_>,
    destination: &mut [u8],
    width: u32,
    height: u32,
    executor: ImageExecutor,
) -> Result<crate::sdf::tile::SdfExecutionStats, ProfileCompositorError> {
    let mut draws = Vec::new();
    for &(command, layer, translate_y) in commands {
        append_semantic_text_draws(command, layer, translate_y, context, &mut draws)?;
    }
    if draws.is_empty() {
        return Ok(crate::sdf::tile::SdfExecutionStats::default());
    }
    let source = crate::sdf::tile::MixedSdfAtlasSource::new(context.text_atlases, None).map_err(
        |error| ProfileCompositorError::SemanticSdf {
            role: "semantic-text-batch".into(),
            reason: error.to_string(),
        },
    )?;
    let plan = crate::sdf::tile::SdfTilePlan::build(
        crate::sdf::tile::TileGrid::new(width, height),
        &draws,
        &source,
    )
    .map_err(|error| ProfileCompositorError::SemanticSdf {
        role: "semantic-text-batch".into(),
        reason: error.to_string(),
    })?;
    match executor {
        ImageExecutor::Scalar => plan.execute_scalar_f32_over(&source, destination),
        ImageExecutor::Simd => plan.execute_simd_f32_over(&source, destination),
    }
    .map_err(|error| ProfileCompositorError::SemanticSdf {
        role: "semantic-text-batch".into(),
        reason: error.to_string(),
    })
}

fn append_semantic_text_draws(
    command: &SemanticCommandSource,
    layer: &LayerSource,
    translate_y: f32,
    context: SemanticSdfContext<'_>,
    draws: &mut Vec<crate::sdf::tile::SdfDrawCommand>,
) -> Result<(), ProfileCompositorError> {
    let SemanticCommandPayload::Text {
        source,
        font_role,
        size,
        color,
        outline_color,
        outline_size,
        line_spacing,
        alignment,
        max_width,
        ..
    } = &command.payload
    else {
        unreachable!("semantic text renderer received non-text command");
    };
    // The payload carries the outline already resolved to RGBA. It is the
    // glyph underlay, which card text draws at every width, zero included.
    let outline_override = Some(crate::text::TextOutlineOverride {
        rgba: *outline_color,
        size: *outline_size,
    });
    let font_id = match font_role {
        FontRole::RegionFontId(font_id) => *font_id,
    };
    let family = context.md.resolve_font_or_default(font_id).ok_or_else(|| {
        ProfileCompositorError::SemanticSdf {
            role: command.role.clone(),
            reason: format!("missing region fontId={font_id}"),
        }
    })?;
    if context
        .text_atlases
        .atlas_for_font_family(&family)
        .is_none()
    {
        return Err(ProfileCompositorError::SemanticSdf {
            role: command.role.clone(),
            reason: format!("font atlas not installed for {family}"),
        });
    }
    let value = match source {
        TextSource::Authored { value }
        | TextSource::ProfileField { value, .. }
        | TextSource::MasterData { value, .. }
        | TextSource::Localized { value, .. } => value,
    };
    if !size.is_finite() || *size <= 0.0 {
        return Err(ProfileCompositorError::InvalidGeometry {
            role: command.role.clone(),
        });
    }
    let placement =
        command
            .render_placement
            .unwrap_or(sekai_profile_core::TextRenderPlacementSource {
                anchor_x: 0.0,
                baseline: None,
            });
    let device_clip = command
        .clip
        .as_ref()
        .map(|clip| axis_aligned_command_clip(clip, layer.matrix))
        .transpose()
        .map_err(|()| ProfileCompositorError::UnsupportedFeature {
            role: command.role.clone(),
            feature: "non-axis text clip".into(),
        })?
        .map(|clip| crate::sdf::tile::SdfDeviceClip {
            min_x: clip.min_x,
            min_y: clip.min_y,
            max_x: clip.max_x,
            max_y: clip.max_y,
        });
    let mut content_matrix = command.matrix;
    content_matrix[5] += translate_y;
    let content_matrix = compose_matrix(layer.matrix, content_matrix);
    let mut capture_error = None;
    let mut observer = |result: Result<
        crate::text::ResolvedTextSdfGlyph,
        crate::text::TextSdfCaptureError,
    >| match result {
        Ok(glyph) => match glyph.to_sdf_command(context.text_atlases) {
            Ok(mut draw) => {
                draw.device_clip = device_clip;
                draws.push(draw);
            }
            Err(error) => {
                capture_error = Some(ProfileCompositorError::SemanticSdf {
                    role: command.role.clone(),
                    reason: error.to_string(),
                });
            }
        },
        Err(error) => capture_error = Some(text_capture_error(&command.role, error)),
    };
    let capture = crate::elements::generals::sdf_text::capture_general_sdf_text_from_lowered(
        content_matrix,
        value,
        command.bounds.width,
        *color,
        *alignment,
        *size,
        *line_spacing,
        font_id,
        *max_width,
        crate::text::TextRenderPlacement {
            anchor_x: placement.anchor_x,
            baseline: placement.baseline,
        },
        outline_override,
        context.md,
        context.text_atlases,
        &mut observer,
    );
    capture.map_err(|reason| ProfileCompositorError::SemanticSdf {
        role: command.role.clone(),
        reason,
    })?;
    if let Some(error) = capture_error {
        return Err(error);
    }
    Ok(())
}

fn text_capture_error(
    role: &str,
    error: crate::text::TextSdfCaptureError,
) -> ProfileCompositorError {
    match error {
        crate::text::TextSdfCaptureError::UnsupportedFeature { feature } => {
            ProfileCompositorError::UnsupportedFeature {
                role: role.into(),
                feature: feature.into(),
            }
        }
    }
}

#[cfg(test)]
mod capture_error_tests {
    #[test]
    fn unsupported_text_capture_features_keep_the_public_feature_error() {
        for feature in ["text underline", "text strikethrough", "text mark"] {
            let error = super::text_capture_error(
                "profile-text",
                crate::text::TextSdfCaptureError::UnsupportedFeature { feature },
            );
            assert!(matches!(
                error,
                super::ProfileCompositorError::UnsupportedFeature { role, feature: actual }
                    if role == "profile-text" && actual == feature
            ));
        }
    }
}
