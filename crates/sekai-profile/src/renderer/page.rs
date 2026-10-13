//! Page rendering: single pages and batches, their telemetry, and PNG and
//! JPEG encoding.

use super::authored::resolve_native_profile_scene;
use super::surface::{SdfPass, SurfaceOutput};
use super::{elapsed_ns, CustomProfileRenderer, RenderObjectGenerationPin};
#[cfg(feature = "jpeg-turbo")]
use crate::output::JpegEncoder;
use crate::output::{EncodedPage, JpegOptions, RenderedBatch, RenderedPage};
use crate::render_config::{sdf_simd_available, RenderConfig, RenderError};
use crate::telemetry::{CommandKind, FallbackCode, RenderTelemetry, SurfaceIdentity};
use crate::types::CustomProfileCard;

impl CustomProfileRenderer {
    /// Renders one page against the render objects the renderer currently
    /// publishes.
    ///
    /// Images the render-object store lacks, or every image when no store is
    /// installed, are decoded from the asset store.
    pub fn render(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        config: &RenderConfig,
    ) -> Result<RenderedPage, RenderError> {
        let generation = self.pin_render_object_generation();
        self.render_pinned(card, profile, config, &generation)
    }

    /// [`Self::render`] against the render objects `generation` pins.
    pub fn render_pinned(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        config: &RenderConfig,
        generation: &RenderObjectGenerationPin,
    ) -> Result<RenderedPage, RenderError> {
        self.render_page_scene(card, profile, config, None, true, generation.store())
    }

    fn render_page_scene(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        config: &RenderConfig,
        pre_resolved_scene: Option<&sekai_profile_core::profile_scene::ResolvedProfileScene>,
        prepare_fallback_glyphs: bool,
        render_object_store: Option<&crate::render_object::MappedRenderObjectStore>,
    ) -> Result<RenderedPage, RenderError> {
        let total_started = std::time::Instant::now();
        let executor = config.resolve(sdf_simd_available())?;
        let store = render_object_store
            .unwrap_or_else(|| crate::render_object::MappedRenderObjectStore::empty());
        let mut telemetry = RenderTelemetry::new(config.clone(), executor.public());

        // A single-page request arrives without a resolved scene. Resolve it
        // once here when fallback glyphs are prepared, so the glyph demand
        // and the drawing read the same scene.
        let owned_scene = if prepare_fallback_glyphs
            && self.profile_fallback_sdf_cache.is_some()
            && pre_resolved_scene.is_none()
        {
            let md = self.snapshot();
            Some(
                resolve_native_profile_scene(
                    card,
                    &md,
                    profile,
                    self.assets.as_deref(),
                    Some(store),
                )
                .map_err(|error| RenderError::Render(error.to_string()))?,
            )
        } else {
            None
        };
        let scene = pre_resolved_scene.or(owned_scene.as_ref());
        let fallback_glyph_cache = match scene {
            Some(scene) if prepare_fallback_glyphs => {
                let md = self.snapshot();
                self.ensure_profile_fallback_for_scenes(std::iter::once(scene), &md)?
            }
            _ => None,
        };

        let mut output = self.render_full_card(
            card,
            profile,
            SdfPass::new(config, executor),
            config.background.clear_rgba(),
            scene,
            store,
        )?;
        self.record_atlas_identities(&mut telemetry, "executed");
        telemetry.atlas_contract = Some(crate::sdf::tile::MIXED_SDF_ATLAS_CONTRACT.into());
        Self::record_surface(&mut telemetry, &output);
        telemetry.timings.total_ns = elapsed_ns(total_started);
        telemetry.fallback_glyph_cache = fallback_glyph_cache;
        Ok(RenderedPage {
            width: output.width,
            height: output.height,
            rgba: std::mem::take(&mut output.rgba),
            background: config.background,
            telemetry,
        })
    }

    /// Folds what drawing one page took into its telemetry.
    fn record_surface(telemetry: &mut RenderTelemetry, output: &SurfaceOutput) {
        telemetry.surface_identity = SurfaceIdentity {
            pixel_format: "rgba8888".into(),
            alpha_type: "premultiplied".into(),
            color_space: "none".into(),
        };
        telemetry.work.page_count = 1;
        telemetry.work.element_run_count = output.sdf_run_count;
        telemetry.bytes.atlas_mapped_bytes = output.atlas_mapped_bytes;
        telemetry.bytes.render_object_mapped_bytes = output.render_object_mapped_bytes;
        telemetry.bytes.encoder_input_bytes = output.rgba.len() as u64;
        telemetry.timings.sdf_capture_ns = output.timings.capture_ns;
        telemetry.timings.semantic_resolve_ns = output.timings.semantic_resolve_ns;
        telemetry.timings.sdf_capture_rich_parse_ns = output.timings.capture_rich_parse_ns;
        telemetry.timings.sdf_capture_font_resolve_ns = output.timings.capture_font_resolve_ns;
        telemetry.timings.sdf_capture_layout_setup_ns = output.timings.capture_layout_setup_ns;
        telemetry.timings.sdf_capture_measure_ns = output.timings.capture_measure_ns;
        telemetry.timings.sdf_capture_command_build_ns = output.timings.capture_command_build_ns;
        telemetry.timings.sdf_capture_emit_ns = output.timings.capture_emit_ns;
        telemetry.timings.surface_create_ns = output.timings.surface_create_ns;
        telemetry.timings.surface_clear_ns = output.timings.surface_clear_ns;
        telemetry.timings.sdf_command_mapping_ns = output.timings.command_mapping_ns;
        telemetry.work.realtime_edt_glyph_count = output
            .realtime_edt_glyphs
            .iter()
            .map(|glyph| glyph.substitution_count)
            .sum();
        telemetry.bytes.realtime_edt_page_bytes = output
            .realtime_edt_glyphs
            .iter()
            .map(|glyph| glyph.page_bytes)
            .sum();
        telemetry.timings.realtime_edt_generation_ns = output
            .realtime_edt_glyphs
            .iter()
            .map(|glyph| glyph.generation_ns)
            .sum();
        telemetry.realtime_edt_glyphs = output.realtime_edt_glyphs.clone();
        telemetry.realtime_edt_batch = output.realtime_edt_batch.clone();
        telemetry.timings.sdf_plan_build_ns = output.timings.plan_build_ns;
        telemetry.timings.sdf_execute_ns = output.timings.execute_ns;
        telemetry.timings.image_composite_ns = output.timings.image_composite_ns;
        telemetry.timings.general_base_composite_ns = output.timings.general_base_composite_ns;
        telemetry.timings.occlusion_mask_build_ns = output.timings.occlusion_mask_build_ns;
        telemetry.timings.occlusion_intersection_ns = output.timings.occlusion_intersection_ns;
        telemetry.timings.rgba_snapshot_ns = output.timings.rgba_snapshot_ns;
        telemetry.record_executed_sdf_commands(
            output.sdf_text_element_count,
            output.sdf_shape_element_count,
            output.captured_text_count,
        );
        telemetry.record_sdf_plan(output.plan_stats, output.plan_bytes, output.span_bytes);
        telemetry.record_sdf_execution(output.execution_stats);
        telemetry.work.occlusion_eligible_image_count = output.occlusion_eligible_image_count;
        telemetry.work.software_skipped_text_command_count =
            output.software_skipped_text_command_count;
        telemetry.work.software_skipped_shape_command_count =
            output.software_skipped_shape_command_count;
        telemetry.work.general_base_hit_count = output.general_base_hit_count;
        telemetry.work.general_base_miss_count = output.general_base_miss_count;
        telemetry.work.general_base_baked_command_count = output.general_base_baked_command_count;
        telemetry.work.general_base_overlay_command_count =
            output.general_base_overlay_command_count;
        telemetry.bytes.general_base_bytes = output.general_base_bytes;
        telemetry.bytes.general_base_avoided_source_bytes =
            output.general_base_avoided_source_bytes;
        telemetry.work.deck_art_variant_hit_count = output.deck_art_variant_hit_count;
        telemetry.work.deck_art_variant_miss_count = output.deck_art_variant_miss_count;
        telemetry.bytes.deck_art_variant_bytes = output.deck_art_variant_bytes;
        telemetry.bytes.deck_art_variant_avoided_source_bytes =
            output.deck_art_variant_avoided_source_bytes;
        telemetry.work.text_command_count = telemetry
            .work
            .text_command_count
            .saturating_add(output.software_text_command_count);
        telemetry.work.shape_command_count = telemetry
            .work
            .shape_command_count
            .saturating_add(output.software_shape_command_count);
        telemetry.work.command_count = telemetry.work.command_count.saturating_add(
            output
                .software_text_command_count
                .saturating_add(output.software_shape_command_count),
        );
        telemetry.work.occlusion_mask_snapshot_count = output.occlusion_mask_snapshot_count;
        telemetry.work.occluded_fragment_count = output.occlusion.occluded_fragment_count;
        telemetry.work.visible_fragment_count = output.occlusion.visible_fragment_count;
        telemetry.work.occluded_text_fragment_count = output.occlusion.occluded_text_fragment_count;
        telemetry.work.occluded_shape_fragment_count =
            output.occlusion.occluded_shape_fragment_count;
        telemetry.work.fully_occluded_sdf_command_count =
            output.occlusion.fully_occluded_command_count;
        telemetry.bytes.occlusion_mask_bytes = output.occlusion_mask_bytes;
        telemetry.record_commands(
            CommandKind::Image,
            output.software_image_count,
            output.timings.image_composite_ns,
        );
        telemetry.work.blended_fragments = telemetry
            .work
            .blended_fragments
            .saturating_add(output.image_fragment_count);
        telemetry.work.sampled_texel_count = telemetry
            .work
            .sampled_texel_count
            .saturating_add(output.image_fragment_count);
        telemetry.work.simd_packet_count = telemetry
            .work
            .simd_packet_count
            .saturating_add(output.image_simd_packet_count);
        if let Some(image) = telemetry
            .commands
            .iter_mut()
            .find(|command| command.kind == CommandKind::Image)
        {
            image.covered_fragments = image
                .covered_fragments
                .saturating_add(output.image_fragment_count);
            image.blended_fragments = image
                .blended_fragments
                .saturating_add(output.image_fragment_count);
        }
    }

    /// Encodes `page` as PNG, keeping its pixel buffer for the next render.
    pub fn encode_png(&self, page: RenderedPage) -> Result<EncodedPage, RenderError> {
        let started = std::time::Instant::now();
        let bytes = page.to_png()?;
        Ok(self.finish_encoding(page, bytes, started, None))
    }

    /// Encodes `page` as JPEG, keeping its pixel buffer and the encoder's
    /// scratch for the next render.
    #[cfg(feature = "jpeg-turbo")]
    pub fn encode_jpeg(
        &self,
        page: RenderedPage,
        options: JpegOptions,
    ) -> Result<EncodedPage, RenderError> {
        let started = std::time::Instant::now();
        options.validate()?;
        page.jpeg_input()?;
        let quality = u32::from(options.quality);
        let bytes = match options.encoder {
            JpegEncoder::Standard => {
                crate::jpeg_turbo::encode_rgba(&page.rgba, page.width, page.height, quality)
            }
            JpegEncoder::Avx512Yuv420 => {
                let mut scratch = self.take_jpeg_yuv420_scratch();
                let encoded = crate::jpeg_turbo::encode_rgba_avx512_yuv420_with_scratch(
                    &page.rgba,
                    page.width,
                    page.height,
                    quality,
                    &mut scratch,
                );
                self.recycle_jpeg_yuv420_scratch(scratch);
                encoded
            }
        }?;
        Ok(self.finish_encoding(page, bytes, started, Some(options)))
    }

    fn finish_encoding(
        &self,
        page: RenderedPage,
        bytes: Vec<u8>,
        started: std::time::Instant,
        jpeg: Option<JpegOptions>,
    ) -> EncodedPage {
        let RenderedPage {
            width,
            height,
            rgba,
            mut telemetry,
            ..
        } = page;
        let encode_ns = elapsed_ns(started);
        telemetry.timings.encode_ns = encode_ns;
        telemetry.timings.total_ns = telemetry.timings.total_ns.saturating_add(encode_ns);
        telemetry.bytes.encoded_output_bytes = bytes.len() as u64;
        telemetry.bytes.scratch_peak_bytes = (rgba.len() as u64).saturating_add(bytes.len() as u64);
        telemetry.jpeg = jpeg;
        self.recycle_profile_rgba_scratch(rgba);
        EncodedPage {
            width,
            height,
            bytes,
            telemetry,
        }
    }

    /// Renders the pages of one profile together: their scenes are resolved
    /// in one pass and missing fallback glyphs are generated once for all of
    /// them.
    ///
    /// `batch_key` and `locale` identify the batch in its compiled form.
    pub fn render_batch<'a>(
        &self,
        batch_key: &str,
        locale: &str,
        pages: impl IntoIterator<Item = (i32, &'a CustomProfileCard)>,
        profile: Option<&crate::profile::ProfileData>,
        config: &RenderConfig,
    ) -> Result<RenderedBatch, RenderError> {
        let generation = self.pin_render_object_generation();
        self.render_batch_pinned(batch_key, locale, pages, profile, config, &generation)
    }

    /// [`Self::render_batch`] against the render objects `generation` pins.
    pub fn render_batch_pinned<'a>(
        &self,
        batch_key: &str,
        locale: &str,
        pages: impl IntoIterator<Item = (i32, &'a CustomProfileCard)>,
        profile: Option<&crate::profile::ProfileData>,
        config: &RenderConfig,
        generation: &RenderObjectGenerationPin,
    ) -> Result<RenderedBatch, RenderError> {
        config.resolve(sdf_simd_available())?;
        let inputs = pages.into_iter().collect::<Vec<_>>();
        let cards_by_seq = inputs
            .iter()
            .map(|(seq, card)| (*seq, *card))
            .collect::<std::collections::BTreeMap<_, _>>();
        let md = self.snapshot();
        let compiled = crate::compiled_profile::compile_profile_batch_with_store(
            batch_key,
            locale,
            inputs.iter().map(|(seq, card)| (*seq, *card)),
            &md,
            profile,
            self.assets.as_deref(),
            generation.store(),
        )
        .map_err(|error| RenderError::Render(error.to_string()))?;
        let prepared_render_objects = generation.store().map(|store| {
            let prepared = compiled.prepare_render_objects(store);
            if !prepared.missing_object_keys.is_empty() {
                tracing::warn!(
                    generation = generation.source_identity().unwrap_or("none"),
                    missing_count = prepared.missing_object_keys.len(),
                    missing_keys = ?prepared.missing_object_keys.iter().take(16).collect::<Vec<_>>(),
                    "the render-object store lacks objects these pages use; they are decoded from the asset store"
                );
            }
            prepared
        });
        let missing_page_object_keys = prepared_render_objects
            .as_ref()
            .map(|prepared| prepared.missing_page_object_keys.clone())
            .unwrap_or_default();
        let fallback_glyph_cache = self.ensure_profile_fallback_for_scenes(
            compiled.pages.iter().map(|page| &page.scene),
            &md,
        )?;
        let mut rendered = Vec::with_capacity(compiled.pages.len());
        for page in &compiled.pages {
            let card = cards_by_seq.get(&page.seq).copied().ok_or_else(|| {
                RenderError::Render(format!("compiled batch lost page seq={}", page.seq))
            })?;
            let mut output = self.render_page_scene(
                card,
                profile,
                config,
                Some(&page.scene),
                false,
                generation.store(),
            )?;
            let missing_for_page = missing_page_object_keys
                .get(&page.seq)
                .map(Vec::as_slice)
                .unwrap_or_default();
            if !missing_for_page.is_empty() {
                output.telemetry.record_fallback(
                    FallbackCode::RenderObjectMissing,
                    "render-objects",
                    format!(
                        "page seq={} decoded {} object(s) from the asset store: {}",
                        page.seq,
                        missing_for_page.len(),
                        missing_for_page
                            .iter()
                            .take(8)
                            .map(|key| key.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    None,
                    None,
                );
            }
            rendered.push((page.seq, output));
        }
        Ok(RenderedBatch {
            compiled,
            prepared_render_objects,
            pages: rendered,
            fallback_glyph_cache,
        })
    }

    /// Records the identity of every installed atlas a render may sample, so
    /// a telemetry consumer can tell which atlas build produced the pixels.
    pub(crate) fn record_atlas_identities(&self, telemetry: &mut RenderTelemetry, usage: &str) {
        let sdf_atlases = self.sdf_atlases.load_full();
        record_profile_atlas_identities(
            telemetry,
            &sdf_atlases,
            self.shape_sdf_atlas.as_deref(),
            usage,
        );
    }
}

fn record_profile_atlas_identities(
    telemetry: &mut RenderTelemetry,
    text_atlases: &crate::sdf::atlas::MappedSdfAtlasSet,
    shape_atlas: Option<&crate::sdf::shape_atlas::MappedShapeSdfAtlas>,
    usage: &str,
) {
    telemetry.atlas_identities.clear();
    {
        telemetry
            .atlas_identities
            .extend(text_atlases.iter().map(|atlas| {
                let manifest = atlas.manifest();
                crate::telemetry::AtlasIdentity {
                    family: "text".into(),
                    usage: usage.into(),
                    schema: manifest.schema.clone(),
                    manifest_sha256: atlas.manifest_sha256().into(),
                    generator_contract: manifest.generator_contract.clone(),
                    pixel_format: "r8-distance".into(),
                    font_family: Some(manifest.font_family.clone()),
                    font_sha256: Some(manifest.font_sha256.clone()),
                    page_count: manifest.pages.len() as u64,
                    entry_count: manifest.glyphs.len() as u64,
                    mapped_bytes: atlas
                        .pages()
                        .iter()
                        .map(|page| page.mapped_bytes() as u64)
                        .fold(0, u64::saturating_add),
                }
            }));
    }
    if let Some(atlas) = shape_atlas {
        let manifest = atlas.manifest();
        telemetry
            .atlas_identities
            .push(crate::telemetry::AtlasIdentity {
                family: "shape".into(),
                usage: usage.into(),
                schema: manifest.schema.clone(),
                manifest_sha256: atlas.manifest_sha256().into(),
                generator_contract: manifest.generator_contract.clone(),
                pixel_format: manifest.pixel_format.clone(),
                font_family: None,
                font_sha256: None,
                page_count: manifest.pages.len() as u64,
                entry_count: manifest.shapes.len() as u64,
                mapped_bytes: atlas.mapped_bytes(),
            });
    }
}
