//! Telemetry of one render: the config it ran with, the atlases it sampled,
//! and what each stage cost.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::render_config::{RenderConfig, SdfExecutor};

pub const PROFILE_RENDER_TELEMETRY_SCHEMA: &str = "allium.profile-render.telemetry.v4";
/// Schema of the identity [`crate::CustomProfileRenderer::cache_identity`]
/// returns.
pub const PROFILE_RENDER_CACHE_SCHEMA: &str = "allium.profile-render.cache.v8";
pub const PROFILE_RENDER_CONTRACT_ORDERED_SDF_RUNS: &str =
    "allium.profile-render.ordered-sdf-runs.v1";

/// Why part of a render took a slower path than the installed resources
/// would normally allow.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FallbackCode {
    /// The pinned render-object store lacks objects the page needs; they were
    /// decoded from the asset store instead.
    RenderObjectMissing,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    #[default]
    Text,
    Shape,
    Image,
    Composite,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandTelemetry {
    pub kind: CommandKind,
    pub command_count: u64,
    pub covered_fragments: u64,
    pub blended_fragments: u64,
    pub cpu_ns: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RenderTimings {
    pub semantic_resolve_ns: u64,
    pub surface_create_ns: u64,
    pub surface_clear_ns: u64,
    /// Time spent resolving Text/Shape elements into shared SDF primitives.
    pub sdf_capture_ns: u64,
    /// Rich-text parsing performed while capturing Text SDF commands.
    pub sdf_capture_rich_parse_ns: u64,
    /// Font-family and typeface resolution performed during Text capture.
    pub sdf_capture_font_resolve_ns: u64,
    /// Per-element line/segment setup before TMP measurement.
    pub sdf_capture_layout_setup_ns: u64,
    /// TMP preferred-size and line measurement pass.
    pub sdf_capture_measure_ns: u64,
    /// Final glyph operation construction after measurement.
    pub sdf_capture_command_build_ns: u64,
    /// Conversion of completed glyph operations to ordered SDF primitives.
    pub sdf_capture_emit_ns: u64,
    /// Time spent mapping resolved primitives to immutable atlas commands.
    pub sdf_command_mapping_ns: u64,
    #[serde(default)]
    pub realtime_edt_generation_ns: u64,
    /// Time spent building ordered per-tile spans for executable SDF runs.
    pub sdf_plan_build_ns: u64,
    /// Combined Text+Shape executor time. This is intentionally not split by
    /// primitive kind because mixed runs are executed together.
    pub sdf_execute_ns: u64,
    /// Time spent compositing mmap-backed Image/Composite semantic commands.
    #[serde(default)]
    pub image_composite_ns: u64,
    #[serde(default)]
    pub general_base_composite_ns: u64,
    #[serde(default)]
    pub occlusion_mask_build_ns: u64,
    #[serde(default)]
    pub occlusion_intersection_ns: u64,
    /// Time spent taking the tight premultiplied RGBA snapshot.
    pub rgba_snapshot_ns: u64,
    /// Animation layers: alpha-bounds scan over the executor-owned RGBA.
    #[serde(default)]
    pub dynamic_layer_bounds_ns: u64,
    /// Animation layers: tight-row copy and final raster construction.
    #[serde(default)]
    pub dynamic_layer_crop_ns: u64,
    pub encode_ns: u64,
    pub total_ns: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RenderWork {
    pub page_count: u64,
    pub command_count: u64,
    pub text_command_count: u64,
    pub shape_command_count: u64,
    pub image_command_count: u64,
    pub composite_command_count: u64,
    #[serde(default)]
    pub software_skipped_text_command_count: u64,
    #[serde(default)]
    pub software_skipped_shape_command_count: u64,
    #[serde(default)]
    pub general_base_hit_count: u64,
    #[serde(default)]
    pub general_base_miss_count: u64,
    #[serde(default)]
    pub general_base_baked_command_count: u64,
    #[serde(default)]
    pub general_base_overlay_command_count: u64,
    #[serde(default)]
    pub deck_art_variant_hit_count: u64,
    #[serde(default)]
    pub deck_art_variant_miss_count: u64,
    pub glyph_count: u64,
    #[serde(default)]
    pub realtime_edt_glyph_count: u64,
    pub shape_count: u64,
    pub element_run_count: u64,
    pub ordered_span_count: u64,
    pub tile_count: u64,
    pub touched_tile_count: u64,
    pub covered_fragments: u64,
    pub blended_fragments: u64,
    pub sampled_texel_count: u64,
    pub simd_packet_count: u64,
    pub swizzled_packet_count: u64,
    pub gather_fallback_packet_count: u64,
    #[serde(default)]
    pub precomputed_shape_fragment_count: u64,
    #[serde(default)]
    pub precomputed_shape_span_count: u64,
    #[serde(default)]
    pub direct_output_run_count: u64,
    #[serde(default)]
    pub direct_output_packet_count: u64,
    #[serde(default)]
    pub occlusion_eligible_image_count: u64,
    #[serde(default)]
    pub occlusion_mask_snapshot_count: u64,
    #[serde(default)]
    pub occluded_fragment_count: u64,
    #[serde(default)]
    pub visible_fragment_count: u64,
    #[serde(default)]
    pub occluded_text_fragment_count: u64,
    #[serde(default)]
    pub occluded_shape_fragment_count: u64,
    #[serde(default)]
    pub fully_occluded_sdf_command_count: u64,
    pub dynamic_layer_count: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RenderBytes {
    pub atlas_mapped_bytes: u64,
    #[serde(default)]
    pub realtime_edt_page_bytes: u64,
    #[serde(default)]
    pub render_object_mapped_bytes: u64,
    #[serde(default)]
    pub general_base_bytes: u64,
    #[serde(default)]
    pub general_base_avoided_source_bytes: u64,
    #[serde(default)]
    pub deck_art_variant_bytes: u64,
    #[serde(default)]
    pub deck_art_variant_avoided_source_bytes: u64,
    pub plan_bytes: u64,
    pub static_span_bytes: u64,
    #[serde(default)]
    pub occlusion_mask_bytes: u64,
    pub layer_cache_bytes: u64,
    pub scratch_peak_bytes: u64,
    pub encoder_input_bytes: u64,
    pub encoded_output_bytes: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CacheCounter {
    pub hits: u64,
    pub misses: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RenderCaches {
    pub layer: CacheCounter,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FallbackEvent {
    pub code: FallbackCode,
    pub stage: String,
    pub reason: String,
    pub layer_id: Option<String>,
    pub command_id: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SurfaceIdentity {
    pub pixel_format: String,
    pub alpha_type: String,
    pub color_space: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AtlasIdentity {
    pub family: String,
    pub usage: String,
    pub schema: String,
    pub manifest_sha256: String,
    pub generator_contract: String,
    pub pixel_format: String,
    pub font_family: Option<String>,
    pub font_sha256: Option<String>,
    pub page_count: u64,
    pub entry_count: u64,
    pub mapped_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RealtimeEdtGlyphTelemetry {
    pub character: String,
    pub codepoint: u32,
    pub font_family: String,
    pub device_magnification_milli: u32,
    pub target_point_size_milli: u32,
    pub generation_ns: u64,
    pub page_bytes: u64,
    /// Number of captured glyph placements that reused this one generated page.
    #[serde(default)]
    pub substitution_count: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RealtimeEdtBatchTelemetry {
    /// Oversized placements served directly from the immutable 3x atlas tier.
    #[serde(default)]
    pub precomputed_3x_substitution_count: u64,
    /// Oversized placements with request-local generation disabled whose
    /// resolved glyph was absent from every sampling tier above the base
    /// atlas.
    #[serde(default)]
    pub precomputed_3x_miss_count: u64,
    /// Oversized placements served from any adequate precomputed sampling
    /// tier, including sparse analytic tiers above 3x.
    #[serde(default)]
    pub precomputed_tier_substitution_count: u64,
    /// Adequate precomputed tier lookups that missed the resolved glyph.
    #[serde(default)]
    pub precomputed_tier_miss_count: u64,
    /// Substitutions grouped by the selected atlas point size in milli-points.
    /// This is sufficient to prove which tier executed without recording
    /// additional text content.
    #[serde(default)]
    pub precomputed_tier_substitutions_by_point_size_milli: BTreeMap<u32, u64>,
    /// Oversized placements which deliberately kept an installed lower tier
    /// because request-local generation was disabled.
    #[serde(default)]
    pub realtime_generation_disabled_fallback_count: u64,
    /// Disabled-generation fallbacks grouped by the point size of the
    /// installed tier that served the placement, in milli-points. Kept apart
    /// from `precomputed_tier_substitutions_by_point_size_milli` so adequate
    /// selections stay distinguishable from degraded fallbacks.
    #[serde(default)]
    pub realtime_generation_disabled_fallbacks_by_point_size_milli: BTreeMap<u32, u64>,
    /// Oversized glyph placements collected before any request-local EDT work starts.
    pub collected_glyph_count: u64,
    /// Exact `(family, codepoint, point size, spread)` requests after deduplication.
    pub unique_request_count: u64,
    /// Placements served by an already-collected request identity.
    pub reused_glyph_count: u64,
    /// Unique requests that produced a runtime page.
    pub generated_request_count: u64,
    /// Captured glyphs with a finite non-positive resolved size; TMP treats these as invisible.
    pub skipped_non_positive_font_size_count: u64,
    pub font_unavailable_fallback_count: u64,
    pub generation_failed_fallback_count: u64,
    pub runtime_page_failed_fallback_count: u64,
    pub runtime_command_failed_fallback_count: u64,
    pub capacity_fallback_count: u64,
    /// Wall time for the bounded parallel generation batch, including stable page assembly.
    pub batch_wall_ns: u64,
    /// Sum of the individual worker generation durations.
    pub worker_generation_ns: u64,
}

impl RealtimeEdtBatchTelemetry {
    pub(crate) fn accumulate(&mut self, other: &Self) {
        self.precomputed_3x_substitution_count = self
            .precomputed_3x_substitution_count
            .saturating_add(other.precomputed_3x_substitution_count);
        self.precomputed_3x_miss_count = self
            .precomputed_3x_miss_count
            .saturating_add(other.precomputed_3x_miss_count);
        self.precomputed_tier_substitution_count = self
            .precomputed_tier_substitution_count
            .saturating_add(other.precomputed_tier_substitution_count);
        self.precomputed_tier_miss_count = self
            .precomputed_tier_miss_count
            .saturating_add(other.precomputed_tier_miss_count);
        for (point_size_milli, count) in &other.precomputed_tier_substitutions_by_point_size_milli {
            let accumulated = self
                .precomputed_tier_substitutions_by_point_size_milli
                .entry(*point_size_milli)
                .or_default();
            *accumulated = accumulated.saturating_add(*count);
        }
        self.realtime_generation_disabled_fallback_count = self
            .realtime_generation_disabled_fallback_count
            .saturating_add(other.realtime_generation_disabled_fallback_count);
        for (point_size_milli, count) in
            &other.realtime_generation_disabled_fallbacks_by_point_size_milli
        {
            let accumulated = self
                .realtime_generation_disabled_fallbacks_by_point_size_milli
                .entry(*point_size_milli)
                .or_default();
            *accumulated = accumulated.saturating_add(*count);
        }
        self.collected_glyph_count = self
            .collected_glyph_count
            .saturating_add(other.collected_glyph_count);
        self.unique_request_count = self
            .unique_request_count
            .saturating_add(other.unique_request_count);
        self.reused_glyph_count = self
            .reused_glyph_count
            .saturating_add(other.reused_glyph_count);
        self.generated_request_count = self
            .generated_request_count
            .saturating_add(other.generated_request_count);
        self.skipped_non_positive_font_size_count = self
            .skipped_non_positive_font_size_count
            .saturating_add(other.skipped_non_positive_font_size_count);
        self.font_unavailable_fallback_count = self
            .font_unavailable_fallback_count
            .saturating_add(other.font_unavailable_fallback_count);
        self.generation_failed_fallback_count = self
            .generation_failed_fallback_count
            .saturating_add(other.generation_failed_fallback_count);
        self.runtime_page_failed_fallback_count = self
            .runtime_page_failed_fallback_count
            .saturating_add(other.runtime_page_failed_fallback_count);
        self.runtime_command_failed_fallback_count = self
            .runtime_command_failed_fallback_count
            .saturating_add(other.runtime_command_failed_fallback_count);
        self.capacity_fallback_count = self
            .capacity_fallback_count
            .saturating_add(other.capacity_fallback_count);
        self.batch_wall_ns = self.batch_wall_ns.saturating_add(other.batch_wall_ns);
        self.worker_generation_ns = self
            .worker_generation_ns
            .saturating_add(other.worker_generation_ns);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RenderTelemetry {
    pub schema: String,
    pub requested: RenderConfig,
    /// The executor that ran, with [`crate::SdfExecutor::Auto`] resolved.
    pub executor: SdfExecutor,
    /// The JPEG encoding applied to the page, when it was encoded as JPEG.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jpeg: Option<crate::output::JpegOptions>,
    pub render_contract: String,
    pub atlas_contract: Option<String>,
    #[serde(default)]
    pub surface_identity: SurfaceIdentity,
    #[serde(default)]
    pub atlas_identities: Vec<AtlasIdentity>,
    #[serde(default)]
    pub realtime_edt_glyphs: Vec<RealtimeEdtGlyphTelemetry>,
    #[serde(default)]
    pub realtime_edt_batch: RealtimeEdtBatchTelemetry,
    pub timings: RenderTimings,
    pub work: RenderWork,
    pub bytes: RenderBytes,
    pub caches: RenderCaches,
    pub commands: Vec<CommandTelemetry>,
    pub fallbacks: Vec<FallbackEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_glyph_cache: Option<crate::sdf::fallback_cache::PersistentFallbackSdfCacheReport>,
}

impl RenderTelemetry {
    pub(crate) fn new(requested: RenderConfig, executor: SdfExecutor) -> Self {
        Self {
            schema: PROFILE_RENDER_TELEMETRY_SCHEMA.into(),
            requested,
            executor,
            jpeg: None,
            render_contract: PROFILE_RENDER_CONTRACT_ORDERED_SDF_RUNS.into(),
            atlas_contract: None,
            surface_identity: SurfaceIdentity::default(),
            atlas_identities: Vec::new(),
            realtime_edt_glyphs: Vec::new(),
            realtime_edt_batch: RealtimeEdtBatchTelemetry::default(),
            timings: RenderTimings::default(),
            work: RenderWork::default(),
            bytes: RenderBytes::default(),
            caches: RenderCaches::default(),
            commands: Vec::new(),
            fallbacks: Vec::new(),
            fallback_glyph_cache: None,
        }
    }

    pub(crate) fn record_fallback(
        &mut self,
        code: FallbackCode,
        stage: impl Into<String>,
        reason: impl Into<String>,
        layer_id: Option<String>,
        command_id: Option<String>,
    ) {
        self.fallbacks.push(FallbackEvent {
            code,
            stage: stage.into(),
            reason: reason.into(),
            layer_id,
            command_id,
        });
    }

    pub(crate) fn record_sdf_plan(
        &mut self,
        stats: crate::sdf::tile::SdfPlanStats,
        resident_bytes: u64,
        span_bytes: u64,
    ) {
        self.work.ordered_span_count = self
            .work
            .ordered_span_count
            .saturating_add(stats.span_count);
        self.work.tile_count = self.work.tile_count.saturating_add(stats.tile_count);
        self.work.touched_tile_count = self
            .work
            .touched_tile_count
            .saturating_add(stats.nonempty_tile_count);
        self.work.covered_fragments = self
            .work
            .covered_fragments
            .saturating_add(stats.covered_fragment_count);
        self.bytes.plan_bytes = self.bytes.plan_bytes.saturating_add(resident_bytes);
        self.bytes.static_span_bytes = self.bytes.static_span_bytes.saturating_add(span_bytes);

        self.record_sdf_command_plan(CommandKind::Text, stats.text_covered_fragment_count);
        self.record_sdf_command_plan(CommandKind::Shape, stats.shape_covered_fragment_count);
    }

    pub(crate) fn record_sdf_execution(&mut self, stats: crate::sdf::tile::SdfExecutionStats) {
        self.work.blended_fragments = self
            .work
            .blended_fragments
            .saturating_add(stats.blended_fragment_count);
        self.work.sampled_texel_count = self
            .work
            .sampled_texel_count
            .saturating_add(stats.sampled_texel_count);
        self.work.simd_packet_count = self
            .work
            .simd_packet_count
            .saturating_add(stats.simd_packet_count);
        self.work.swizzled_packet_count = self
            .work
            .swizzled_packet_count
            .saturating_add(stats.swizzled_packet_count);
        self.work.gather_fallback_packet_count = self
            .work
            .gather_fallback_packet_count
            .saturating_add(stats.gather_fallback_packet_count);
        self.work.precomputed_shape_fragment_count = self
            .work
            .precomputed_shape_fragment_count
            .saturating_add(stats.precomputed_shape_fragment_count);
        self.work.precomputed_shape_span_count = self
            .work
            .precomputed_shape_span_count
            .saturating_add(stats.precomputed_shape_span_count);
        self.work.direct_output_run_count = self
            .work
            .direct_output_run_count
            .saturating_add(stats.direct_output_run_count);
        self.work.direct_output_packet_count = self
            .work
            .direct_output_packet_count
            .saturating_add(stats.direct_output_packet_count);
        self.record_sdf_command_execution(CommandKind::Text, stats.text_blended_fragment_count);
        self.record_sdf_command_execution(CommandKind::Shape, stats.shape_blended_fragment_count);
    }

    /// Records the commands the SDF executor ran. Planned spans and fragments
    /// are counted separately by `record_sdf_plan`, so a plan that is built
    /// but not executed adds no commands.
    pub(crate) fn record_executed_sdf_commands(
        &mut self,
        text_element_count: u64,
        shape_element_count: u64,
        glyph_count: u64,
    ) {
        self.work.command_count = self
            .work
            .command_count
            .saturating_add(text_element_count.saturating_add(shape_element_count));
        self.work.text_command_count = self
            .work
            .text_command_count
            .saturating_add(text_element_count);
        self.work.shape_command_count = self
            .work
            .shape_command_count
            .saturating_add(shape_element_count);
        self.work.shape_count = self.work.shape_count.saturating_add(shape_element_count);
        self.work.glyph_count = self.work.glyph_count.saturating_add(glyph_count);
        let text = self.command_telemetry_mut(CommandKind::Text);
        text.command_count = text.command_count.saturating_add(text_element_count);
        let shape = self.command_telemetry_mut(CommandKind::Shape);
        shape.command_count = shape.command_count.saturating_add(shape_element_count);
    }

    pub(crate) fn record_commands(&mut self, kind: CommandKind, command_count: u64, cpu_ns: u64) {
        if command_count == 0 {
            return;
        }
        self.work.command_count = self.work.command_count.saturating_add(command_count);
        match kind {
            CommandKind::Text => {
                self.work.text_command_count =
                    self.work.text_command_count.saturating_add(command_count);
            }
            CommandKind::Shape => {
                self.work.shape_command_count =
                    self.work.shape_command_count.saturating_add(command_count);
                self.work.shape_count = self.work.shape_count.saturating_add(command_count);
            }
            CommandKind::Image => {
                self.work.image_command_count =
                    self.work.image_command_count.saturating_add(command_count);
            }
            CommandKind::Composite => {
                self.work.composite_command_count = self
                    .work
                    .composite_command_count
                    .saturating_add(command_count);
            }
        }
        let command = self.command_telemetry_mut(kind);
        command.command_count = command.command_count.saturating_add(command_count);
        command.cpu_ns = command.cpu_ns.saturating_add(cpu_ns);
    }

    fn record_sdf_command_plan(&mut self, kind: CommandKind, covered_fragments: u64) {
        if covered_fragments == 0 {
            return;
        }
        let command = self.command_telemetry_mut(kind);
        command.covered_fragments = command.covered_fragments.saturating_add(covered_fragments);
    }

    fn record_sdf_command_execution(&mut self, kind: CommandKind, blended_fragments: u64) {
        if blended_fragments == 0 {
            return;
        }
        let command = self.command_telemetry_mut(kind);
        command.blended_fragments = command.blended_fragments.saturating_add(blended_fragments);
    }

    fn command_telemetry_mut(&mut self, kind: CommandKind) -> &mut CommandTelemetry {
        if let Some(index) = self.commands.iter().position(|entry| entry.kind == kind) {
            return &mut self.commands[index];
        }
        self.commands.push(CommandTelemetry {
            kind,
            ..CommandTelemetry::default()
        });
        let index = self.commands.len() - 1;
        &mut self.commands[index]
    }
}

#[cfg(test)]
mod tests;
