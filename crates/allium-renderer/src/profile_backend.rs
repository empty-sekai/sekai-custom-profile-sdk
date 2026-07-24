//! Backend contract and telemetry for custom-profile rendering.
//!
//! Text layout is intentionally outside the pixel executor selection: every
//! surface adapter consumes the same completed TMP glyph operations. The
//! surface backend only controls image/composite submission, while the SDF
//! executor controls text and, when enabled, shape coverage generation.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROFILE_RENDER_TELEMETRY_SCHEMA: &str = "allium.profile-render.telemetry.v3";
pub const PROFILE_RENDER_CONTRACT_LEGACY_SKIA: &str = "allium.profile-render.legacy-skia.v1";
pub const PROFILE_RENDER_CONTRACT_ORDERED_SDF_RUNS: &str =
    "allium.profile-render.ordered-sdf-runs.v1";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileSurfaceBackend {
    #[default]
    SkiaRasterCpu,
    SkiaOpenGlLlvmPipe,
    SkiaVulkanLavaPipe,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextSdfExecutor {
    /// Existing per-glyph Skia-compatible software path.
    #[default]
    LegacySkia,
    /// Pre-generated R8 atlas and the runtime-dispatched SIMD tile executor.
    Simd,
    /// Slow correctness oracle used by tests and candidate analysis only.
    ScalarOracle,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShapeSdfExecutor {
    /// Keep the current Skia implementation.
    #[default]
    Skia,
    /// Use the typed RG8 Shape atlas through the same ordered tile compositor
    /// as Text; the distance and alpha-gate channels remain independent.
    Simd,
    /// Measure the page and select SIMD only when its validated classifier
    /// says the work is large enough. This mode must never use player IDs.
    Auto,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileJpegEncoder {
    #[default]
    Skia,
    LibJpegTurbo,
    LibJpegTurboAvx512Yuv420,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendFallbackPolicy {
    /// Reject the request if any requested executor or surface is unavailable.
    FailClosed,
    /// Fall back the entire page to the validated legacy path. A future SIMD
    /// executor may still record per-command fallback events while remaining
    /// the primary page executor.
    #[default]
    Page,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileBackendConfig {
    pub surface: ProfileSurfaceBackend,
    pub text_sdf: TextSdfExecutor,
    pub shape_sdf: ShapeSdfExecutor,
    #[serde(default)]
    pub jpeg_encoder: ProfileJpegEncoder,
    pub tile_width: u16,
    pub tile_height: u16,
    pub collect_telemetry: bool,
    /// Measurement-only reverse pixel-occlusion pass. It never changes the
    /// rendered output and remains opt-in until server evidence justifies an
    /// executable culling path.
    #[serde(default)]
    pub pixel_occlusion_dry_run: bool,
    /// Experimental exact executor that consumes the same reverse mask and
    /// removes covered SDF destination pixels before atlas sampling.
    #[serde(default)]
    pub pixel_occlusion_execute: bool,
    pub fallback_policy: BackendFallbackPolicy,
}

impl Default for ProfileBackendConfig {
    fn default() -> Self {
        Self {
            surface: ProfileSurfaceBackend::SkiaRasterCpu,
            text_sdf: TextSdfExecutor::LegacySkia,
            shape_sdf: ShapeSdfExecutor::Skia,
            jpeg_encoder: ProfileJpegEncoder::Skia,
            tile_width: 32,
            tile_height: 32,
            collect_telemetry: true,
            pixel_occlusion_dry_run: false,
            pixel_occlusion_execute: false,
            fallback_policy: BackendFallbackPolicy::Page,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileBackendCapabilities {
    pub skia_raster_cpu: bool,
    pub skia_opengl_llvmpipe: bool,
    pub skia_vulkan_lavapipe: bool,
    pub text_legacy_skia: bool,
    pub text_simd: bool,
    pub text_scalar_oracle: bool,
    pub shape_skia: bool,
    pub shape_simd: bool,
}

impl ProfileBackendCapabilities {
    /// Capabilities implemented by the existing production renderer. New
    /// executors must only flip a flag after their runtime resources exist.
    pub const fn legacy_skia_only() -> Self {
        Self {
            skia_raster_cpu: true,
            skia_opengl_llvmpipe: false,
            skia_vulkan_lavapipe: false,
            text_legacy_skia: true,
            text_simd: false,
            text_scalar_oracle: false,
            shape_skia: true,
            shape_simd: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendFallbackCode {
    SurfaceUnavailable,
    TextExecutorUnavailable,
    ShapeExecutorUnavailable,
    RenderObjectMissing,
    ExecutorRuntimeFailure,
    EncoderRuntimeFailure,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedProfileBackend {
    pub surface: ProfileSurfaceBackend,
    pub text_sdf: TextSdfExecutor,
    pub shape_sdf: ShapeSdfExecutor,
    pub fallbacks: Vec<BackendFallbackEvent>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ProfileBackendSelectionError {
    #[error(
        "invalid profile backend tile size {width}x{height}; expected powers of two in 8..=128"
    )]
    InvalidTileSize { width: u16, height: u16 },
    #[error("requested profile backend stage {stage} is unavailable: {reason}")]
    Unavailable {
        stage: &'static str,
        reason: &'static str,
    },
    #[error("validated legacy fallback stage {stage} is unavailable")]
    MissingLegacyFallback { stage: &'static str },
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ProfileBackendRenderError {
    #[error(transparent)]
    Selection(#[from] ProfileBackendSelectionError),
    #[error("profile backend render failed: {0}")]
    Render(String),
}

impl ProfileBackendConfig {
    pub fn resolve(
        &self,
        capabilities: ProfileBackendCapabilities,
    ) -> Result<ResolvedProfileBackend, ProfileBackendSelectionError> {
        if !(8..=128).contains(&self.tile_width)
            || !(8..=128).contains(&self.tile_height)
            || !self.tile_width.is_power_of_two()
            || !self.tile_height.is_power_of_two()
        {
            return Err(ProfileBackendSelectionError::InvalidTileSize {
                width: self.tile_width,
                height: self.tile_height,
            });
        }

        let mut resolved = ResolvedProfileBackend {
            surface: self.surface,
            text_sdf: self.text_sdf,
            shape_sdf: self.shape_sdf,
            fallbacks: Vec::new(),
        };
        if !surface_available(self.surface, capabilities) {
            resolve_unavailable(
                self.fallback_policy,
                capabilities.skia_raster_cpu,
                "surface",
                "requested-surface-unavailable",
                BackendFallbackCode::SurfaceUnavailable,
                &mut resolved.fallbacks,
            )?;
            resolved.surface = ProfileSurfaceBackend::SkiaRasterCpu;
        }
        if !text_available(self.text_sdf, capabilities) {
            resolve_unavailable(
                self.fallback_policy,
                capabilities.text_legacy_skia,
                "text-sdf",
                "requested-text-executor-unavailable",
                BackendFallbackCode::TextExecutorUnavailable,
                &mut resolved.fallbacks,
            )?;
            resolved.text_sdf = TextSdfExecutor::LegacySkia;
        }
        if !shape_available(self.shape_sdf, capabilities) {
            resolve_unavailable(
                self.fallback_policy,
                capabilities.shape_skia,
                "shape-sdf",
                "requested-shape-executor-unavailable",
                BackendFallbackCode::ShapeExecutorUnavailable,
                &mut resolved.fallbacks,
            )?;
            resolved.shape_sdf = ShapeSdfExecutor::Skia;
        } else if resolved.shape_sdf == ShapeSdfExecutor::Auto {
            // Auto currently has one deterministic, resource-based choice.
            // A workload classifier may replace this only after it has its own
            // validated threshold corpus; player/profile identity is never an
            // input to backend selection.
            resolved.shape_sdf = ShapeSdfExecutor::Simd;
        }
        Ok(resolved)
    }
}

fn surface_available(
    requested: ProfileSurfaceBackend,
    capabilities: ProfileBackendCapabilities,
) -> bool {
    match requested {
        ProfileSurfaceBackend::SkiaRasterCpu => capabilities.skia_raster_cpu,
        ProfileSurfaceBackend::SkiaOpenGlLlvmPipe => capabilities.skia_opengl_llvmpipe,
        ProfileSurfaceBackend::SkiaVulkanLavaPipe => capabilities.skia_vulkan_lavapipe,
    }
}

fn text_available(requested: TextSdfExecutor, capabilities: ProfileBackendCapabilities) -> bool {
    match requested {
        TextSdfExecutor::LegacySkia => capabilities.text_legacy_skia,
        TextSdfExecutor::Simd => capabilities.text_simd,
        TextSdfExecutor::ScalarOracle => capabilities.text_scalar_oracle,
    }
}

fn shape_available(requested: ShapeSdfExecutor, capabilities: ProfileBackendCapabilities) -> bool {
    match requested {
        ShapeSdfExecutor::Skia => capabilities.shape_skia,
        ShapeSdfExecutor::Simd | ShapeSdfExecutor::Auto => capabilities.shape_simd,
    }
}

fn resolve_unavailable(
    policy: BackendFallbackPolicy,
    legacy_available: bool,
    stage: &'static str,
    reason: &'static str,
    code: BackendFallbackCode,
    fallbacks: &mut Vec<BackendFallbackEvent>,
) -> Result<(), ProfileBackendSelectionError> {
    if policy == BackendFallbackPolicy::FailClosed {
        return Err(ProfileBackendSelectionError::Unavailable { stage, reason });
    }
    if !legacy_available {
        return Err(ProfileBackendSelectionError::MissingLegacyFallback { stage });
    }
    fallbacks.push(BackendFallbackEvent {
        code,
        stage: stage.into(),
        reason: reason.into(),
        layer_id: None,
        command_id: None,
    });
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileCommandKind {
    #[default]
    Text,
    Shape,
    Image,
    Composite,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandTelemetry {
    pub kind: ProfileCommandKind,
    pub command_count: u64,
    pub covered_fragments: u64,
    pub blended_fragments: u64,
    pub cpu_ns: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileRenderTimings {
    pub profile_fetch_ns: u64,
    pub parse_ns: u64,
    pub semantic_resolve_ns: u64,
    pub tmp_layout_ns: u64,
    pub plan_build_ns: u64,
    pub surface_create_ns: u64,
    pub surface_clear_ns: u64,
    pub atlas_open_ns: u64,
    pub atlas_mmap_ns: u64,
    pub atlas_prefetch_ns: u64,
    pub tile_binning_ns: u64,
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
    /// Time spent building ordered per-tile spans for executable SDF runs.
    pub sdf_plan_build_ns: u64,
    /// Combined Text+Shape executor time. This is intentionally not split by
    /// primitive kind because mixed runs are executed together.
    pub sdf_execute_ns: u64,
    /// Time spent drawing non-SDF elements between ordered SDF runs.
    pub legacy_element_draw_ns: u64,
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
    /// Dynamic candidate layers: alpha-bounds scan over executor-owned RGBA.
    #[serde(default)]
    pub dynamic_layer_bounds_ns: u64,
    /// Dynamic candidate layers: tight-row copy and final raster construction.
    #[serde(default)]
    pub dynamic_layer_crop_ns: u64,
    pub text_sdf_ns: u64,
    pub shape_sdf_ns: u64,
    pub image_draw_ns: u64,
    pub composite_ns: u64,
    pub tile_upload_ns: u64,
    pub gpu_submit_ns: u64,
    pub gpu_flush_ns: u64,
    pub readback_ns: u64,
    pub dynamic_evaluate_ns: u64,
    pub dynamic_frame_composite_ns: u64,
    pub rgb_to_yuv_ns: u64,
    pub encode_ns: u64,
    pub artifact_upload_ns: u64,
    pub total_ns: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileRenderWork {
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
    pub shape_count: u64,
    pub element_run_count: u64,
    pub ordered_span_count: u64,
    pub tile_count: u64,
    pub touched_tile_count: u64,
    pub dirty_tile_count: u64,
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
    pub dynamic_frame_count: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileRenderBytes {
    pub atlas_file_bytes: u64,
    pub atlas_mapped_bytes: u64,
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
    pub tile_upload_bytes: u64,
    pub readback_bytes: u64,
    pub encoder_input_bytes: u64,
    pub encoded_output_bytes: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CacheCounter {
    pub hits: u64,
    pub misses: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileRenderCaches {
    pub plan: CacheCounter,
    pub atlas: CacheCounter,
    pub glyph: CacheCounter,
    pub static_span: CacheCounter,
    pub layer: CacheCounter,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BackendFallbackEvent {
    pub code: BackendFallbackCode,
    pub stage: String,
    pub reason: String,
    pub layer_id: Option<String>,
    pub command_id: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileSurfaceIdentity {
    pub pixel_format: String,
    pub alpha_type: String,
    pub color_space: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileAtlasIdentity {
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
pub struct ProfileRenderTelemetry {
    pub schema: String,
    pub requested: ProfileBackendConfig,
    pub actual_surface: ProfileSurfaceBackend,
    pub actual_text_sdf: TextSdfExecutor,
    pub actual_shape_sdf: ShapeSdfExecutor,
    #[serde(default)]
    pub actual_jpeg_encoder: ProfileJpegEncoder,
    pub render_contract: String,
    pub atlas_contract: Option<String>,
    #[serde(default)]
    pub surface_identity: ProfileSurfaceIdentity,
    #[serde(default)]
    pub atlas_identities: Vec<ProfileAtlasIdentity>,
    pub timings: ProfileRenderTimings,
    pub work: ProfileRenderWork,
    pub bytes: ProfileRenderBytes,
    pub caches: ProfileRenderCaches,
    pub commands: Vec<CommandTelemetry>,
    pub fallbacks: Vec<BackendFallbackEvent>,
    #[cfg(feature = "skia-core")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_glyph_cache: Option<crate::sdf::fallback_cache::PersistentFallbackSdfCacheReport>,
}

impl ProfileRenderTelemetry {
    pub fn new(requested: ProfileBackendConfig, render_contract: impl Into<String>) -> Self {
        Self {
            schema: PROFILE_RENDER_TELEMETRY_SCHEMA.into(),
            actual_surface: requested.surface,
            actual_text_sdf: requested.text_sdf,
            actual_shape_sdf: requested.shape_sdf,
            actual_jpeg_encoder: requested.jpeg_encoder,
            requested,
            render_contract: render_contract.into(),
            atlas_contract: None,
            surface_identity: ProfileSurfaceIdentity::default(),
            atlas_identities: Vec::new(),
            timings: ProfileRenderTimings::default(),
            work: ProfileRenderWork::default(),
            bytes: ProfileRenderBytes::default(),
            caches: ProfileRenderCaches::default(),
            commands: Vec::new(),
            fallbacks: Vec::new(),
            #[cfg(feature = "skia-core")]
            fallback_glyph_cache: None,
        }
    }

    pub fn record_fallback(
        &mut self,
        code: BackendFallbackCode,
        stage: impl Into<String>,
        reason: impl Into<String>,
        layer_id: Option<String>,
        command_id: Option<String>,
    ) {
        self.fallbacks.push(BackendFallbackEvent {
            code,
            stage: stage.into(),
            reason: reason.into(),
            layer_id,
            command_id,
        });
    }

    pub fn apply_selection(&mut self, selection: ResolvedProfileBackend) {
        self.actual_surface = selection.surface;
        self.actual_text_sdf = selection.text_sdf;
        self.actual_shape_sdf = selection.shape_sdf;
        self.fallbacks.extend(selection.fallbacks);
    }

    #[cfg(feature = "skia-core")]
    pub fn record_sdf_plan(
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

        self.record_sdf_command_plan(ProfileCommandKind::Text, stats.text_covered_fragment_count);
        self.record_sdf_command_plan(
            ProfileCommandKind::Shape,
            stats.shape_covered_fragment_count,
        );
    }

    #[cfg(feature = "skia-core")]
    pub fn record_sdf_execution(&mut self, stats: crate::sdf::tile::SdfExecutionStats) {
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
        self.record_sdf_command_execution(
            ProfileCommandKind::Text,
            stats.text_blended_fragment_count,
        );
        self.record_sdf_command_execution(
            ProfileCommandKind::Shape,
            stats.shape_blended_fragment_count,
        );
    }

    /// Records commands that were actually executed by the SDF backend. This
    /// is deliberately separate from `record_sdf_plan`: the legacy renderer
    /// may build a shadow plan for telemetry without executing those commands.
    #[cfg(feature = "skia-core")]
    pub fn record_executed_sdf_commands(
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
        let text = self.command_telemetry_mut(ProfileCommandKind::Text);
        text.command_count = text.command_count.saturating_add(text_element_count);
        let shape = self.command_telemetry_mut(ProfileCommandKind::Shape);
        shape.command_count = shape.command_count.saturating_add(shape_element_count);
    }

    #[cfg(feature = "skia-core")]
    pub fn record_legacy_commands(
        &mut self,
        kind: ProfileCommandKind,
        command_count: u64,
        cpu_ns: u64,
    ) {
        if command_count == 0 {
            return;
        }
        self.work.command_count = self.work.command_count.saturating_add(command_count);
        match kind {
            ProfileCommandKind::Text => {
                self.work.text_command_count =
                    self.work.text_command_count.saturating_add(command_count);
            }
            ProfileCommandKind::Shape => {
                self.work.shape_command_count =
                    self.work.shape_command_count.saturating_add(command_count);
                self.work.shape_count = self.work.shape_count.saturating_add(command_count);
            }
            ProfileCommandKind::Image => {
                self.work.image_command_count =
                    self.work.image_command_count.saturating_add(command_count);
            }
            ProfileCommandKind::Composite => {
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

    #[cfg(feature = "skia-core")]
    fn record_sdf_command_plan(&mut self, kind: ProfileCommandKind, covered_fragments: u64) {
        if covered_fragments == 0 {
            return;
        }
        let command = self.command_telemetry_mut(kind);
        command.covered_fragments = command.covered_fragments.saturating_add(covered_fragments);
    }

    #[cfg(feature = "skia-core")]
    fn record_sdf_command_execution(&mut self, kind: ProfileCommandKind, blended_fragments: u64) {
        if blended_fragments == 0 {
            return;
        }
        let command = self.command_telemetry_mut(kind);
        command.blended_fragments = command.blended_fragments.saturating_add(blended_fragments);
    }

    #[cfg(feature = "skia-core")]
    fn command_telemetry_mut(&mut self, kind: ProfileCommandKind) -> &mut CommandTelemetry {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileBackendRenderOutput {
    pub encoded: Vec<u8>,
    pub telemetry: ProfileRenderTelemetry,
}

pub struct ProfileBackendBatchRenderOutput {
    pub compiled: crate::compiled_profile::CompiledProfileBatch,
    pub prepared_render_objects: Option<crate::compiled_profile::PreparedRenderObjectBatch>,
    pub pages: Vec<(i32, ProfileBackendRenderOutput)>,
    pub fallback_glyph_cache: Option<crate::sdf::fallback_cache::PersistentFallbackSdfCacheReport>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_the_existing_production_path() {
        let config = ProfileBackendConfig::default();
        assert_eq!(config.surface, ProfileSurfaceBackend::SkiaRasterCpu);
        assert_eq!(config.text_sdf, TextSdfExecutor::LegacySkia);
        assert_eq!(config.shape_sdf, ShapeSdfExecutor::Skia);
        assert_eq!(config.jpeg_encoder, ProfileJpegEncoder::Skia);
        assert_eq!((config.tile_width, config.tile_height), (32, 32));
        let telemetry = ProfileRenderTelemetry::new(config, PROFILE_RENDER_CONTRACT_LEGACY_SKIA);
        assert_eq!(telemetry.actual_jpeg_encoder, ProfileJpegEncoder::Skia);
    }

    #[test]
    fn telemetry_serializes_backend_identity_and_fallback_reason() {
        let requested = ProfileBackendConfig {
            text_sdf: TextSdfExecutor::Simd,
            shape_sdf: ShapeSdfExecutor::Auto,
            jpeg_encoder: ProfileJpegEncoder::LibJpegTurbo,
            ..ProfileBackendConfig::default()
        };
        let mut telemetry = ProfileRenderTelemetry::new(requested, "profile-sdf-v1");
        telemetry.surface_identity = ProfileSurfaceIdentity {
            pixel_format: "rgba8888".into(),
            alpha_type: "premultiplied".into(),
            color_space: "none".into(),
        };
        telemetry.atlas_identities.push(ProfileAtlasIdentity {
            family: "text".into(),
            usage: "executed".into(),
            schema: "allium.sdf-atlas-manifest.v1".into(),
            manifest_sha256: "11".repeat(32),
            generator_contract: "outline-edt-v1:ss=2:fallback=analytic-v1".into(),
            pixel_format: "r8-distance".into(),
            font_family: Some("test-font".into()),
            font_sha256: Some("22".repeat(32)),
            page_count: 1,
            entry_count: 42,
            mapped_bytes: 4096,
        });
        telemetry.record_fallback(
            BackendFallbackCode::ShapeExecutorUnavailable,
            "shape-sdf",
            "unsupported-shape-resource",
            Some("layer-1".into()),
            Some("command-2".into()),
        );
        let value = serde_json::to_value(telemetry).expect("telemetry must serialize");
        assert_eq!(value["schema"], PROFILE_RENDER_TELEMETRY_SCHEMA);
        assert_eq!(value["requested"]["text_sdf"], "simd");
        assert_eq!(value["requested"]["jpeg_encoder"], "lib-jpeg-turbo");
        assert_eq!(value["actual_jpeg_encoder"], "lib-jpeg-turbo");
        assert_eq!(value["surface_identity"]["pixel_format"], "rgba8888");
        assert_eq!(value["atlas_identities"][0]["usage"], "executed");
        assert_eq!(
            value["atlas_identities"][0]["generator_contract"],
            "outline-edt-v1:ss=2:fallback=analytic-v1"
        );
        assert_eq!(
            value["fallbacks"][0]["reason"],
            "unsupported-shape-resource"
        );
    }

    #[test]
    fn legacy_capabilities_resolve_default_without_fallback() {
        let resolved = ProfileBackendConfig::default()
            .resolve(ProfileBackendCapabilities::legacy_skia_only())
            .expect("default backend must resolve");
        assert_eq!(resolved.surface, ProfileSurfaceBackend::SkiaRasterCpu);
        assert_eq!(resolved.text_sdf, TextSdfExecutor::LegacySkia);
        assert_eq!(resolved.shape_sdf, ShapeSdfExecutor::Skia);
        assert!(resolved.fallbacks.is_empty());
    }

    #[test]
    fn unavailable_candidate_falls_back_whole_page_with_stable_codes() {
        let config = ProfileBackendConfig {
            surface: ProfileSurfaceBackend::SkiaVulkanLavaPipe,
            text_sdf: TextSdfExecutor::Simd,
            shape_sdf: ShapeSdfExecutor::Auto,
            ..ProfileBackendConfig::default()
        };
        let resolved = config
            .resolve(ProfileBackendCapabilities::legacy_skia_only())
            .expect("page fallback must resolve");
        assert_eq!(resolved.surface, ProfileSurfaceBackend::SkiaRasterCpu);
        assert_eq!(resolved.text_sdf, TextSdfExecutor::LegacySkia);
        assert_eq!(resolved.shape_sdf, ShapeSdfExecutor::Skia);
        assert_eq!(
            resolved
                .fallbacks
                .iter()
                .map(|fallback| fallback.code)
                .collect::<Vec<_>>(),
            vec![
                BackendFallbackCode::SurfaceUnavailable,
                BackendFallbackCode::TextExecutorUnavailable,
                BackendFallbackCode::ShapeExecutorUnavailable,
            ]
        );
    }

    #[test]
    fn fail_closed_rejects_unavailable_candidate() {
        let config = ProfileBackendConfig {
            text_sdf: TextSdfExecutor::Simd,
            fallback_policy: BackendFallbackPolicy::FailClosed,
            ..ProfileBackendConfig::default()
        };
        assert!(matches!(
            config.resolve(ProfileBackendCapabilities::legacy_skia_only()),
            Err(ProfileBackendSelectionError::Unavailable {
                stage: "text-sdf",
                ..
            })
        ));
    }

    #[test]
    fn invalid_tile_size_is_rejected_before_selection() {
        let config = ProfileBackendConfig {
            tile_width: 24,
            ..ProfileBackendConfig::default()
        };
        assert_eq!(
            config.resolve(ProfileBackendCapabilities::legacy_skia_only()),
            Err(ProfileBackendSelectionError::InvalidTileSize {
                width: 24,
                height: 32,
            })
        );
    }

    #[test]
    fn shape_auto_resolves_to_concrete_simd_without_identity_input() {
        let config = ProfileBackendConfig {
            shape_sdf: ShapeSdfExecutor::Auto,
            ..ProfileBackendConfig::default()
        };
        let resolved = config
            .resolve(ProfileBackendCapabilities {
                skia_raster_cpu: true,
                skia_opengl_llvmpipe: false,
                skia_vulkan_lavapipe: false,
                text_legacy_skia: true,
                text_simd: false,
                text_scalar_oracle: false,
                shape_skia: true,
                shape_simd: true,
            })
            .expect("available Shape Auto must resolve");
        assert_eq!(resolved.shape_sdf, ShapeSdfExecutor::Simd);
        assert!(resolved.fallbacks.is_empty());
    }

    #[cfg(feature = "skia-core")]
    #[test]
    fn shadow_plan_does_not_claim_commands_were_executed() {
        let mut telemetry = ProfileRenderTelemetry::new(
            ProfileBackendConfig::default(),
            PROFILE_RENDER_CONTRACT_LEGACY_SKIA,
        );
        telemetry.record_sdf_plan(
            crate::sdf::tile::SdfPlanStats {
                command_count: 3,
                text_command_count: 2,
                shape_command_count: 1,
                span_count: 4,
                text_span_count: 3,
                shape_span_count: 1,
                tile_count: 2,
                nonempty_tile_count: 1,
                covered_fragment_count: 8,
                text_covered_fragment_count: 6,
                shape_covered_fragment_count: 2,
            },
            128,
            64,
        );
        assert_eq!(telemetry.work.command_count, 0);
        assert_eq!(telemetry.work.glyph_count, 0);
        assert_eq!(telemetry.work.shape_count, 0);
        assert_eq!(telemetry.work.ordered_span_count, 4);
        assert_eq!(telemetry.commands[0].command_count, 0);
    }

    #[cfg(feature = "skia-core")]
    #[test]
    fn sdf_stats_accumulate_without_losing_command_kind() {
        let mut telemetry = ProfileRenderTelemetry::new(
            ProfileBackendConfig::default(),
            PROFILE_RENDER_CONTRACT_LEGACY_SKIA,
        );
        let plan_stats = crate::sdf::tile::SdfPlanStats {
            command_count: 5,
            text_command_count: 4,
            shape_command_count: 1,
            span_count: 20,
            text_span_count: 16,
            shape_span_count: 4,
            tile_count: 8,
            nonempty_tile_count: 6,
            covered_fragment_count: 100,
            text_covered_fragment_count: 80,
            shape_covered_fragment_count: 20,
        };
        telemetry.record_executed_sdf_commands(2, 1, 4);
        telemetry.record_sdf_plan(plan_stats, 2_048, 320);
        telemetry.record_sdf_execution(crate::sdf::tile::SdfExecutionStats {
            shaded_fragment_count: 100,
            text_shaded_fragment_count: 80,
            shape_shaded_fragment_count: 20,
            sampled_texel_count: 400,
            blended_fragment_count: 100,
            text_blended_fragment_count: 80,
            shape_blended_fragment_count: 20,
            simd_packet_count: 7,
            swizzled_packet_count: 3,
            gather_fallback_packet_count: 4,
            precomputed_shape_fragment_count: 12,
            precomputed_shape_span_count: 3,
            direct_output_run_count: 2,
            direct_output_packet_count: 5,
        });

        assert_eq!(telemetry.work.glyph_count, 4);
        assert_eq!(telemetry.work.shape_count, 1);
        assert_eq!(telemetry.work.command_count, 3);
        assert_eq!(telemetry.work.ordered_span_count, 20);
        assert_eq!(telemetry.work.touched_tile_count, 6);
        assert_eq!(telemetry.work.sampled_texel_count, 400);
        assert_eq!(telemetry.work.simd_packet_count, 7);
        assert_eq!(telemetry.work.swizzled_packet_count, 3);
        assert_eq!(telemetry.work.gather_fallback_packet_count, 4);
        assert_eq!(telemetry.work.precomputed_shape_fragment_count, 12);
        assert_eq!(telemetry.work.precomputed_shape_span_count, 3);
        assert_eq!(telemetry.work.direct_output_run_count, 2);
        assert_eq!(telemetry.work.direct_output_packet_count, 5);
        assert_eq!(telemetry.bytes.plan_bytes, 2_048);
        assert_eq!(telemetry.bytes.static_span_bytes, 320);
        assert_eq!(
            telemetry
                .commands
                .iter()
                .find(|command| command.kind == ProfileCommandKind::Text)
                .map(|command| (
                    command.command_count,
                    command.covered_fragments,
                    command.blended_fragments
                )),
            Some((2, 80, 80))
        );
        assert_eq!(
            telemetry
                .commands
                .iter()
                .find(|command| command.kind == ProfileCommandKind::Shape)
                .map(|command| (
                    command.command_count,
                    command.covered_fragments,
                    command.blended_fragments
                )),
            Some((1, 20, 20))
        );
    }
}
