//! SDF capture: the text and shape primitives of a card, mapped to draw commands
//! that sample larger atlas tiers or realtime EDT glyphs when magnified.

use super::elapsed_ns;
#[cfg(feature = "parallel")]
use super::realtime_edt::realtime_edt_pool;
use super::realtime_edt::{
    active_realtime_edt_batch, realtime_edt_request_key, realtime_edt_sampling_spread,
    RealtimeEdtPreparedEntry, RealtimeEdtRequestKey,
};
use crate::assets::AssetStore;
use crate::masterdata::MasterData;
use crate::types::CustomProfileCard;

pub(super) enum CapturedSdfPrimitive {
    Text(crate::text::ResolvedTextSdfGlyph),
    Shape(crate::elements::shape::ResolvedShapeSdfCommand),
}

#[derive(Clone, Copy)]
pub(super) struct SdfCaptureKinds {
    text: bool,
    shape: bool,
}

impl SdfCaptureKinds {
    pub(super) const TEXT: Self = Self {
        text: true,
        shape: false,
    };

    #[cfg(feature = "animation-export")]
    pub(super) const TEXT_AND_SHAPE: Self = Self {
        text: true,
        shape: true,
    };
}

pub(super) struct CapturedSdfPrimitives {
    pub(super) primitives: Vec<CapturedSdfPrimitive>,
    #[cfg_attr(not(feature = "animation-export"), allow(dead_code))]
    pub(super) text_count: u64,
    #[cfg_attr(not(feature = "animation-export"), allow(dead_code))]
    pub(super) shape_count: u64,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn capture_sdf_primitives(
    card: &CustomProfileCard,
    md: &MasterData,
    assets: Option<&AssetStore>,
    text_atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
    profile: Option<&crate::profile::ProfileData>,
    canvas_width: u32,
    canvas_height: u32,
    kinds: SdfCaptureKinds,
) -> Result<CapturedSdfPrimitives, String> {
    let _ = profile;
    let mut primitives = Vec::new();
    let mut failures = Vec::new();
    let mut text_count = 0u64;
    let mut shape_count = 0u64;

    for element in crate::elements::flatten_and_sort(card) {
        if !element.visible() {
            continue;
        }
        match element {
            crate::elements::RenderElement::Text(_) if kinds.text => {
                let mut observer = |result: Result<
                    crate::text::ResolvedTextSdfGlyph,
                    crate::text::TextSdfCaptureError,
                >| match result {
                    Ok(glyph) => {
                        text_count = text_count.saturating_add(1);
                        primitives.push(CapturedSdfPrimitive::Text(glyph));
                    }
                    Err(error) => failures.push(format!("Text: {error:?}")),
                };
                crate::elements::capture_element_sdf(
                    &element,
                    md,
                    assets,
                    canvas_width as f32,
                    canvas_height as f32,
                    [0.0, 0.0],
                    text_atlases,
                    Some(&mut observer),
                    None,
                );
            }
            crate::elements::RenderElement::Shape(_) if kinds.shape => {
                let mut observer = |result: Result<
                    crate::elements::shape::ResolvedShapeSdfCommand,
                    crate::elements::shape::ShapeSdfCaptureError,
                >| match result {
                    Ok(shape) => {
                        shape_count = shape_count.saturating_add(1);
                        primitives.push(CapturedSdfPrimitive::Shape(shape));
                    }
                    Err(error) => failures.push(format!("Shape: {error}")),
                };
                crate::elements::capture_element_sdf(
                    &element,
                    md,
                    assets,
                    canvas_width as f32,
                    canvas_height as f32,
                    [0.0, 0.0],
                    text_atlases,
                    None,
                    Some(&mut observer),
                );
            }
            _ => {}
        }
    }
    if !failures.is_empty() {
        return Err(format!(
            "SDF command capture rejected {} operation(s): {}",
            failures.len(),
            failures.join("; ")
        ));
    }
    Ok(CapturedSdfPrimitives {
        primitives,
        text_count,
        shape_count,
    })
}

pub(super) fn captured_text_sdf_glyph_is_invisible(
    glyph: &crate::text::ResolvedTextSdfGlyph,
) -> bool {
    glyph.font_size.is_finite() && glyph.font_size <= 0.0
}

#[derive(Clone, Copy)]
pub(super) struct TextSdfSamplingGrid {
    command: crate::sdf::tile::SdfDrawCommand,
    point_size: f32,
    left_center: f32,
    top_center: f32,
}

impl TextSdfSamplingGrid {
    pub(super) fn new(
        command: crate::sdf::tile::SdfDrawCommand,
        glyph: &crate::sdf::atlas::SdfAtlasGlyphManifest,
        point_size: f32,
        spread: f32,
    ) -> Option<Self> {
        let spread = spread.ceil();
        let left_center = glyph.plane_bearing[0].floor() - spread + 0.5;
        let top_center = glyph.plane_bearing[1].ceil() + spread - 0.5;
        [point_size, left_center, top_center]
            .into_iter()
            .all(f32::is_finite)
            .then_some(Self {
                command,
                point_size,
                left_center,
                top_center,
            })
            .filter(|grid| grid.point_size > 0.0)
    }
}

pub(super) fn align_substituted_text_sdf_command(
    primary: TextSdfSamplingGrid,
    mut substitution: crate::sdf::tile::SdfDrawCommand,
    glyph: &crate::sdf::atlas::SdfAtlasGlyphManifest,
    point_size: f32,
    spread: f32,
) -> Option<crate::sdf::tile::SdfDrawCommand> {
    use crate::sdf::tile::Point2;

    let target = TextSdfSamplingGrid::new(substitution, glyph, point_size, spread)?;
    let scale = target.point_size / primary.point_size;
    let primary_width = primary.command.atlas_rect[2] as f32;
    let primary_height = primary.command.atlas_rect[3] as f32;
    let target_width = target.command.atlas_rect[2] as f32;
    let target_height = target.command.atlas_rect[3] as f32;
    if !scale.is_finite()
        || scale <= 0.0
        || primary_width <= 0.0
        || primary_height <= 0.0
        || target_width <= 0.0
        || target_height <= 0.0
    {
        return None;
    }

    // Atlas pixels are sampled at half-integer font coordinates. Preserve the
    // primary command's screen-to-sample mapping when switching point-size
    // tiers; rect dimensions alone include floor/ceil padding and are not a
    // scale-invariant UV ratio.
    let offset_x = scale.mul_add(primary.left_center, -target.left_center);
    let offset_y = target.top_center - scale * primary.top_center;
    let axis_range = |primary_extent: f32, target_extent: f32, offset: f32| {
        let slope = scale * primary_extent / target_extent;
        let intercept = (offset + 0.5 * (1.0 - scale)) / target_extent;
        if !slope.is_finite() || !intercept.is_finite() || slope.abs() <= f32::EPSILON {
            return None;
        }
        let start = -intercept / slope;
        let end = (1.0 - intercept) / slope;
        (start.is_finite() && end.is_finite()).then_some((start, end))
    };
    let (u0, u1) = axis_range(primary_width, target_width, offset_x)?;
    let (v0, v1) = axis_range(primary_height, target_height, offset_y)?;

    let [top_left, top_right, _, bottom_left] = primary.command.quad;
    let ex = Point2::new(top_right.x - top_left.x, top_right.y - top_left.y);
    let ey = Point2::new(bottom_left.x - top_left.x, bottom_left.y - top_left.y);
    let map = |u: f32, v: f32| {
        Point2::new(
            ex.x.mul_add(u, ey.x.mul_add(v, top_left.x)),
            ex.y.mul_add(u, ey.y.mul_add(v, top_left.y)),
        )
    };
    substitution.quad = [map(u0, v0), map(u1, v0), map(u1, v1), map(u0, v1)];
    Some(substitution)
}

pub(super) fn map_captured_sdf_commands(
    captured: &[CapturedSdfPrimitive],
    text_atlases: &crate::sdf::atlas::MappedSdfAtlasSet,
    shape_atlas: Option<&crate::sdf::shape_atlas::MappedShapeSdfAtlas>,
    source: &crate::sdf::tile::MixedSdfAtlasSource<'_>,
    mut runtime_text_pages: Option<&mut Vec<crate::sdf::tile::RuntimeTextSdfPage>>,
    mut realtime_edt_telemetry: Option<&mut Vec<crate::telemetry::RealtimeEdtGlyphTelemetry>>,
    mut realtime_edt_batch_telemetry: Option<&mut crate::telemetry::RealtimeEdtBatchTelemetry>,
    mut prepared_entries_out: Option<
        &mut std::collections::BTreeMap<RealtimeEdtRequestKey, RealtimeEdtPreparedEntry>,
    >,
) -> Result<Vec<crate::sdf::tile::SdfDrawCommand>, String> {
    use std::collections::BTreeMap;

    #[derive(Clone)]
    enum PreparedCommand {
        Atlas(crate::sdf::tile::SdfDrawCommand),
        Realtime {
            atlas: crate::sdf::tile::SdfDrawCommand,
            glyph: crate::text::ResolvedTextSdfGlyph,
            primary_grid: TextSdfSamplingGrid,
            request_index: usize,
        },
    }

    #[derive(Clone)]
    struct Request {
        key: RealtimeEdtRequestKey,
        character: char,
        magnification: f32,
        substitution_count: u64,
    }

    struct GeneratedRequest {
        glyph: Result<crate::sdf::outline::OutlineSdfGlyph, String>,
        generation_ns: u64,
    }

    struct RuntimeRequest {
        atlas_set: u16,
        manifest: crate::sdf::atlas::SdfAtlasGlyphManifest,
        generation_ns: u64,
        page_bytes: u64,
    }

    let mut batch = crate::telemetry::RealtimeEdtBatchTelemetry::default();
    let active_batch = active_realtime_edt_batch();
    let mut prepared = Vec::with_capacity(captured.len());
    let mut request_indices = BTreeMap::<RealtimeEdtRequestKey, usize>::new();
    let mut requests = Vec::<Request>::new();

    for (primitive_index, primitive) in captured.iter().enumerate() {
        match primitive {
            CapturedSdfPrimitive::Text(glyph) => {
                // TMP does not emit visible geometry for a finite non-positive
                // rich-text size. Keeping such a captured operation in the SDF
                // stream turns an invisible glyph into a fatal metrics error.
                if captured_text_sdf_glyph_is_invisible(glyph) {
                    if active_batch.is_none() {
                        batch.skipped_non_positive_font_size_count =
                            batch.skipped_non_positive_font_size_count.saturating_add(1);
                    }
                    continue;
                }
                let atlas_command = glyph.to_sdf_command(text_atlases).map_err(|error| {
                    format!(
                        "primitive {primitive_index} Text {:?} family={:?} font_size={}: {error}",
                        glyph.text, glyph.font_family, glyph.font_size
                    )
                })?;
                let mut chars = glyph.text.chars();
                let codepoint = match (chars.next(), chars.next()) {
                    (Some(ch), None) => Some(ch),
                    _ => None,
                };
                let Some(final_magnification) = sdf_command_device_magnification(&atlas_command)
                    .filter(|magnification| *magnification > 3.0)
                else {
                    prepared.push(PreparedCommand::Atlas(atlas_command));
                    continue;
                };
                let Some(codepoint) = codepoint else {
                    prepared.push(PreparedCommand::Atlas(atlas_command));
                    continue;
                };
                // The already-built command is the source of truth for the
                // atlas selected by the normal primary/fallback resolution.
                let Some(atlas) = text_atlases.atlas(atlas_command.atlas_set) else {
                    prepared.push(PreparedCommand::Atlas(atlas_command));
                    continue;
                };
                let Some(primary_glyph) = atlas.glyph(codepoint as u32) else {
                    prepared.push(PreparedCommand::Atlas(atlas_command));
                    continue;
                };
                let Some(primary_grid) = TextSdfSamplingGrid::new(
                    atlas_command,
                    primary_glyph,
                    atlas.manifest().point_size,
                    atlas.manifest().spread,
                ) else {
                    prepared.push(PreparedCommand::Atlas(atlas_command));
                    continue;
                };
                let resolved_family = atlas.manifest().font_family.as_str();
                let target_point_size = (atlas.manifest().point_size * final_magnification)
                    .ceil()
                    .clamp(atlas.manifest().point_size, 4096.0);
                if let Some((precomputed_set, precomputed_atlas, precomputed_glyph)) = text_atlases
                    .glyph_for_font_family_at_least(
                        resolved_family,
                        codepoint as u32,
                        target_point_size,
                    )
                {
                    if let Ok(command) = glyph.to_sdf_command_from_manifest(
                        precomputed_set,
                        precomputed_glyph,
                        precomputed_atlas.manifest().point_size,
                        precomputed_atlas.manifest().spread,
                    ) {
                        let command = align_substituted_text_sdf_command(
                            primary_grid,
                            command,
                            precomputed_glyph,
                            precomputed_atlas.manifest().point_size,
                            precomputed_atlas.manifest().spread,
                        )
                        .unwrap_or(command);
                        if active_batch.is_none() {
                            record_precomputed_tier_substitution(
                                &mut batch,
                                precomputed_atlas.manifest().point_size,
                                atlas.manifest().point_size,
                            );
                        }
                        prepared.push(PreparedCommand::Atlas(command));
                        continue;
                    }
                }
                if active_batch.is_none() {
                    batch.precomputed_tier_miss_count =
                        batch.precomputed_tier_miss_count.saturating_add(1);
                }
                let mut fallback_command = atlas_command;
                let mut fallback_point_size = None;
                if let Some((fallback_set, fallback_atlas, fallback_glyph)) = text_atlases
                    .highest_glyph_for_font_family_above(
                        resolved_family,
                        codepoint as u32,
                        atlas.manifest().point_size,
                    )
                {
                    if let Ok(command) = glyph.to_sdf_command_from_manifest(
                        fallback_set,
                        fallback_glyph,
                        fallback_atlas.manifest().point_size,
                        fallback_atlas.manifest().spread,
                    ) {
                        fallback_command = align_substituted_text_sdf_command(
                            primary_grid,
                            command,
                            fallback_glyph,
                            fallback_atlas.manifest().point_size,
                            fallback_atlas.manifest().spread,
                        )
                        .unwrap_or(command);
                        fallback_point_size = Some(fallback_atlas.manifest().point_size);
                    }
                }
                if runtime_text_pages.is_none() && active_batch.is_none() {
                    batch.realtime_generation_disabled_fallback_count = batch
                        .realtime_generation_disabled_fallback_count
                        .saturating_add(1);
                    if let Some(point_size) = fallback_point_size {
                        let count = batch
                            .realtime_generation_disabled_fallbacks_by_point_size_milli
                            .entry(point_size_milli(point_size))
                            .or_default();
                        *count = count.saturating_add(1);
                    } else {
                        batch.precomputed_3x_miss_count =
                            batch.precomputed_3x_miss_count.saturating_add(1);
                    }
                    prepared.push(PreparedCommand::Atlas(fallback_command));
                    continue;
                }
                if crate::sdf::outline::resolve_font_path(resolved_family).is_none() {
                    batch.font_unavailable_fallback_count =
                        batch.font_unavailable_fallback_count.saturating_add(1);
                    prepared.push(PreparedCommand::Atlas(fallback_command));
                    continue;
                }
                let runtime_spread = realtime_edt_sampling_spread(
                    atlas.manifest().point_size,
                    atlas.manifest().spread,
                    target_point_size,
                );
                let Some(runtime_spread) = runtime_spread else {
                    prepared.push(PreparedCommand::Atlas(fallback_command));
                    continue;
                };
                let key = realtime_edt_request_key(
                    resolved_family,
                    codepoint,
                    target_point_size,
                    runtime_spread,
                );
                if let Some(active) = active_batch.as_deref() {
                    let Some(entry) = active.entries.get(&key) else {
                        prepared.push(PreparedCommand::Atlas(fallback_command));
                        continue;
                    };
                    match glyph.to_sdf_command_from_manifest(
                        entry.atlas_set,
                        &entry.manifest,
                        entry.point_size,
                        entry.spread,
                    ) {
                        Ok(command) => prepared.push(PreparedCommand::Atlas(
                            align_substituted_text_sdf_command(
                                primary_grid,
                                command,
                                &entry.manifest,
                                entry.point_size,
                                entry.spread,
                            )
                            .unwrap_or(command),
                        )),
                        Err(_) => prepared.push(PreparedCommand::Atlas(fallback_command)),
                    }
                    continue;
                }
                batch.collected_glyph_count = batch.collected_glyph_count.saturating_add(1);
                let request_index = if let Some(index) = request_indices.get(&key).copied() {
                    requests[index].substitution_count =
                        requests[index].substitution_count.saturating_add(1);
                    batch.reused_glyph_count = batch.reused_glyph_count.saturating_add(1);
                    index
                } else {
                    let index = requests.len();
                    request_indices.insert(key.clone(), index);
                    requests.push(Request {
                        key,
                        character: codepoint,
                        magnification: final_magnification,
                        substitution_count: 1,
                    });
                    index
                };
                prepared.push(PreparedCommand::Realtime {
                    atlas: fallback_command,
                    glyph: glyph.clone(),
                    primary_grid,
                    request_index,
                });
            }
            CapturedSdfPrimitive::Shape(shape) => {
                let atlas =
                    shape_atlas.ok_or_else(|| "Shape atlas is not installed".to_string())?;
                let atlas_set = source
                    .shape_atlas_set()
                    .ok_or_else(|| "Shape atlas set is unavailable".to_string())?;
                let command = shape
                    .to_sdf_command(atlas, atlas_set)
                    .map_err(|error| format!("primitive {primitive_index} Shape: {error}"))?;
                prepared.push(PreparedCommand::Atlas(command));
            }
        }
    }

    batch.unique_request_count = requests.len() as u64;
    if requests.is_empty() {
        if let Some(report) = realtime_edt_batch_telemetry.as_deref_mut() {
            report.accumulate(&batch);
        }
        return Ok(prepared
            .into_iter()
            .map(|command| match command {
                PreparedCommand::Atlas(command) => command,
                PreparedCommand::Realtime { atlas, .. } => atlas,
            })
            .collect());
    }

    let batch_started = std::time::Instant::now();
    let generate = |request: &Request| {
        let started = std::time::Instant::now();
        let point_size = f32::from_bits(request.key.point_size_bits);
        let spread = f32::from_bits(request.key.spread_bits);
        let glyph = crate::sdf::outline::generate_realtime_edt(
            &request.key.font_family,
            request.character,
            point_size,
            spread,
            2,
        );
        GeneratedRequest {
            glyph,
            generation_ns: elapsed_ns(started),
        }
    };
    // Order-preserving in both forms, so the runtime page layout is identical.
    #[cfg(feature = "parallel")]
    let generated = {
        use rayon::prelude::*;
        realtime_edt_pool().install(|| requests.par_iter().map(generate).collect::<Vec<_>>())
    };
    #[cfg(not(feature = "parallel"))]
    let generated = requests.iter().map(generate).collect::<Vec<_>>();

    let runtime_set = u16::try_from(text_atlases.len() + usize::from(shape_atlas.is_some())).ok();
    let pages = runtime_text_pages.expect("runtime pages were checked before collecting requests");
    let mut runtime_requests = (0..requests.len())
        .map(|_| None)
        .collect::<Vec<Option<RuntimeRequest>>>();
    for (request_index, result) in generated.into_iter().enumerate() {
        batch.worker_generation_ns = batch
            .worker_generation_ns
            .saturating_add(result.generation_ns);
        let Ok(outline) = result.glyph else {
            batch.generation_failed_fallback_count = batch
                .generation_failed_fallback_count
                .saturating_add(requests[request_index].substitution_count);
            continue;
        };
        let (Some(runtime_set), Ok(page)) = (runtime_set, u16::try_from(pages.len())) else {
            batch.capacity_fallback_count = batch
                .capacity_fallback_count
                .saturating_add(requests[request_index].substitution_count);
            continue;
        };
        let Ok((runtime_page, manifest)) = crate::sdf::tile::RuntimeTextSdfPage::from_outline(
            &outline,
            requests[request_index].key.codepoint,
            page,
        ) else {
            batch.runtime_page_failed_fallback_count = batch
                .runtime_page_failed_fallback_count
                .saturating_add(requests[request_index].substitution_count);
            continue;
        };
        let page_bytes = runtime_page.resident_bytes();
        pages.push(runtime_page);
        runtime_requests[request_index] = Some(RuntimeRequest {
            atlas_set: runtime_set,
            manifest,
            generation_ns: result.generation_ns,
            page_bytes,
        });
        if let Some(entries) = prepared_entries_out.as_deref_mut() {
            let request = &requests[request_index];
            entries.insert(
                request.key.clone(),
                RealtimeEdtPreparedEntry {
                    atlas_set: runtime_set,
                    manifest: runtime_requests[request_index]
                        .as_ref()
                        .expect("runtime request was just installed")
                        .manifest
                        .clone(),
                    point_size: f32::from_bits(request.key.point_size_bits),
                    spread: f32::from_bits(request.key.spread_bits),
                },
            );
        }
        batch.generated_request_count = batch.generated_request_count.saturating_add(1);
    }

    let mut commands = Vec::with_capacity(prepared.len());
    let mut successful_substitutions = vec![0u64; requests.len()];
    for command in prepared {
        match command {
            PreparedCommand::Atlas(command) => commands.push(command),
            PreparedCommand::Realtime {
                atlas,
                glyph,
                primary_grid,
                request_index,
            } => {
                let Some(runtime) = runtime_requests[request_index].as_ref() else {
                    commands.push(atlas);
                    continue;
                };
                let request = &requests[request_index];
                match glyph.to_sdf_command_from_manifest(
                    runtime.atlas_set,
                    &runtime.manifest,
                    f32::from_bits(request.key.point_size_bits),
                    f32::from_bits(request.key.spread_bits),
                ) {
                    Ok(command) => {
                        let command = align_substituted_text_sdf_command(
                            primary_grid,
                            command,
                            &runtime.manifest,
                            f32::from_bits(request.key.point_size_bits),
                            f32::from_bits(request.key.spread_bits),
                        )
                        .unwrap_or(command);
                        successful_substitutions[request_index] =
                            successful_substitutions[request_index].saturating_add(1);
                        commands.push(command);
                    }
                    Err(_) => {
                        batch.runtime_command_failed_fallback_count = batch
                            .runtime_command_failed_fallback_count
                            .saturating_add(1);
                        commands.push(atlas);
                    }
                }
            }
        }
    }
    if let Some(records) = realtime_edt_telemetry {
        for (request_index, runtime) in runtime_requests.iter().enumerate() {
            let Some(runtime) = runtime else {
                continue;
            };
            let request = &requests[request_index];
            records.push(crate::telemetry::RealtimeEdtGlyphTelemetry {
                character: request.character.to_string(),
                codepoint: request.key.codepoint,
                font_family: request.key.font_family.clone(),
                device_magnification_milli: (request.magnification * 1000.0)
                    .round()
                    .clamp(0.0, u32::MAX as f32) as u32,
                target_point_size_milli: (f32::from_bits(request.key.point_size_bits) * 1000.0)
                    .round()
                    .clamp(0.0, u32::MAX as f32) as u32,
                generation_ns: runtime.generation_ns,
                page_bytes: runtime.page_bytes,
                substitution_count: successful_substitutions[request_index],
            });
        }
    }
    batch.batch_wall_ns = elapsed_ns(batch_started);
    if let Some(report) = realtime_edt_batch_telemetry {
        report.accumulate(&batch);
    }
    Ok(commands)
}

pub(super) fn sdf_command_device_magnification(
    command: &crate::sdf::tile::SdfDrawCommand,
) -> Option<f32> {
    fn edge_length(a: crate::sdf::tile::Point2, b: crate::sdf::tile::Point2) -> f32 {
        (b.x - a.x).hypot(b.y - a.y)
    }

    let atlas_width = command.atlas_rect[2] as f32;
    let atlas_height = command.atlas_rect[3] as f32;
    if atlas_width <= 0.0 || atlas_height <= 0.0 {
        return None;
    }
    let horizontal = edge_length(command.quad[0], command.quad[1]) / atlas_width;
    let vertical = edge_length(command.quad[0], command.quad[3]) / atlas_height;
    let magnification = horizontal.max(vertical);
    magnification.is_finite().then_some(magnification)
}

fn point_size_milli(point_size: f32) -> u32 {
    (point_size * 1000.0).round().clamp(0.0, u32::MAX as f32) as u32
}

fn record_precomputed_tier_substitution(
    telemetry: &mut crate::telemetry::RealtimeEdtBatchTelemetry,
    selected_point_size: f32,
    primary_point_size: f32,
) {
    telemetry.precomputed_tier_substitution_count = telemetry
        .precomputed_tier_substitution_count
        .saturating_add(1);
    let count = telemetry
        .precomputed_tier_substitutions_by_point_size_milli
        .entry(point_size_milli(selected_point_size))
        .or_default();
    *count = count.saturating_add(1);
    if selected_point_size.to_bits() == (primary_point_size * 3.0).to_bits() {
        telemetry.precomputed_3x_substitution_count = telemetry
            .precomputed_3x_substitution_count
            .saturating_add(1);
    }
}
