//! Work on the installed resources outside a page render: prewarming them
//! before the first request and building General base render objects.

use super::{elapsed_ns, CustomProfileRenderer};

impl CustomProfileRenderer {
    /// Builds one canonical server-side General base from an already resolved
    /// semantic scene for the standalone resource-pipeline builder.
    pub fn build_general_base_objects(
        &self,
        scene: &sekai_profile_core::profile_scene::ResolvedProfileScene,
        authored_index: u32,
    ) -> Result<Vec<crate::profile_compositor::GeneralBaseBuildOutput>, String> {
        let generation = self.pin_render_object_generation();
        let store = generation
            .store()
            .ok_or_else(|| "General base builder requires a render-object store".to_string())?;
        self.build_general_base_objects_for_store(scene, authored_index, store)
    }

    /// [`Self::build_general_base_objects`] against `store` instead of the
    /// published render-object store.
    pub fn build_general_base_objects_for_store(
        &self,
        scene: &sekai_profile_core::profile_scene::ResolvedProfileScene,
        authored_index: u32,
        store: &crate::render_object::MappedRenderObjectStore,
    ) -> Result<Vec<crate::profile_compositor::GeneralBaseBuildOutput>, String> {
        let atlases = self.sdf_atlases.load_full();
        let md = self.snapshot();
        crate::profile_compositor::build_general_base_objects_simd(
            scene,
            store,
            &atlases,
            &md,
            authored_index,
        )
        .map_err(|error| error.to_string())
    }

    /// Touches every installed resource once so the first render does not
    /// pay for it: decodes and checks each shape source against the shape
    /// atlas, faults in atlas and render-object pages, resolves the atlas
    /// fonts, and allocates the page and encoder scratch buffers.
    ///
    /// # Errors
    ///
    /// Returns a message when a shape atlas is installed without an asset
    /// store, when a shape source is missing or differs from the one the
    /// atlas was built from, when an atlas font cannot be resolved, or when
    /// the shape programs or the JPEG encoder cannot be prepared.
    pub fn prewarm(&self) -> Result<PrewarmReport, String> {
        let started = std::time::Instant::now();
        let mut report = PrewarmReport::default();
        if let Some(atlas) = self.shape_sdf_atlas.as_deref() {
            let assets = self
                .assets
                .as_deref()
                .ok_or_else(|| "Shape SDF atlas is installed without an AssetStore".to_string())?;
            for entry in &atlas.manifest().shapes {
                let identity = assets
                    .shape_sdf_source_identity_for_key(&entry.asset_key)
                    .map_err(|error| {
                        format!(
                            "Shape SDF prewarm could not resolve source {}: {error:?}",
                            entry.asset_key
                        )
                    })?;
                let source_size = [
                    u32::try_from(identity.width).map_err(|_| {
                        format!("Shape SDF prewarm width is invalid: {}", entry.asset_key)
                    })?,
                    u32::try_from(identity.height).map_err(|_| {
                        format!("Shape SDF prewarm height is invalid: {}", entry.asset_key)
                    })?,
                ];
                if source_size != entry.source_size
                    || identity.rg8_sha256 != entry.source_rg8_sha256
                {
                    return Err(format!(
                        "Shape SDF prewarm identity mismatch for {}: size {:?}/{:?}, RG8 {}/{}",
                        entry.asset_key,
                        source_size,
                        entry.source_size,
                        identity.rg8_sha256,
                        entry.source_rg8_sha256
                    ));
                }
                report.shape_source_count = report.shape_source_count.saturating_add(1);
                report.shape_decoded_bytes = report.shape_decoded_bytes.saturating_add(
                    u64::from(source_size[0])
                        .saturating_mul(u64::from(source_size[1]))
                        .saturating_mul(4),
                );
            }
        }
        let generation = self.pin_render_object_generation();
        if let Some(store) = generation.store() {
            let object_report = store.prewarm_profile_hotset();
            report.render_object_count = object_report.object_count;
            report.render_object_bytes = object_report.object_bytes;
            report.render_object_page_touch_count = object_report.page_touch_count;
            report.render_object_checksum = object_report.checksum;
        }
        let sdf_atlases = self.sdf_atlases.load_full();
        let (font_family_count, font_resolve_ns) = crate::text::prewarm_profile_font_families(
            sdf_atlases
                .iter()
                .map(|atlas| atlas.manifest().font_family.as_str())
                .filter(|family| *family != crate::sdf::atlas::PROFILE_TEXT_FALLBACK_FONT_FAMILY),
        )?;
        report.font_family_count = font_family_count;
        report.font_resolve_ns = font_resolve_ns;
        let text_report = sdf_atlases.prewarm_primary_pages();
        report.text_atlas_count = text_report.atlas_count;
        report.text_atlas_page_count = text_report.page_count;
        report.text_atlas_mapped_bytes = text_report.mapped_bytes;
        report.text_atlas_page_touch_count = text_report.page_touch_count;
        report.text_atlas_checksum = text_report.checksum;
        if let Some(atlas) = self.shape_sdf_atlas.as_deref() {
            let shape_report = atlas.prewarm_pages();
            report.shape_atlas_count = shape_report.atlas_count;
            report.shape_atlas_page_count = shape_report.page_count;
            report.shape_atlas_mapped_bytes = shape_report.mapped_bytes;
            report.shape_atlas_page_touch_count = shape_report.page_touch_count;
            report.shape_atlas_checksum = shape_report.checksum;
            let row_program_started = std::time::Instant::now();
            let row_program_report = self
                .shape_row_program_cache
                .prewarm_shape_atlas(atlas)
                .map_err(|error| format!("Shape row-program prewarm failed: {error}"))?;
            report.shape_row_program_count = row_program_report.program_count;
            report.shape_row_program_run_count = row_program_report.run_count;
            report.shape_row_program_bytes = row_program_report.resident_bytes;
            report.shape_row_program_build_ns = elapsed_ns(row_program_started);
        }
        let profile_surface_bytes =
            crate::transform::CANVAS_WIDTH as usize * crate::transform::CANVAS_HEIGHT as usize * 4;
        {
            let mut scratch = self
                .profile_rgba_scratch
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            scratch.resize(profile_surface_bytes, 0);
            // Touch and retain every physical page before READY. A non-zero
            // pass prevents the kernel's shared zero page from satisfying the
            // prewarm without allocating private writable pages.
            scratch.fill(0xa5);
            scratch.fill(0);
            // The pool contract keeps capacity but always publishes an empty
            // Vec. `take_profile_rgba_scratch` resizes and initializes the
            // requested surface on every checkout.
            scratch.clear();
        }
        report.profile_surface_scratch_bytes = profile_surface_bytes as u64;
        report.profile_surface_page_touch_count = profile_surface_bytes.div_ceil(4096) as u64;
        let surface_init_started = std::time::Instant::now();
        let mut rgba = self.take_profile_rgba_scratch(profile_surface_bytes, [255; 4]);
        // Touch every page of the page-sized scratch so the first request does
        // not pay the faults.
        rgba.fill(255);
        self.recycle_profile_rgba_scratch(rgba);
        report.profile_surface_init_ns = elapsed_ns(surface_init_started);
        #[cfg(feature = "jpeg-turbo")]
        {
            let width = crate::transform::CANVAS_WIDTH as u32;
            let height = crate::transform::CANVAS_HEIGHT as u32;
            let jpeg_probe = crate::jpeg_turbo::encode_rgba(&[255u8; 8 * 8 * 4], 8, 8, 90)
                .map_err(|error| format!("JPEG encoder prewarm failed: {error}"))?;
            report.jpeg_encoder_prewarm_bytes = jpeg_probe.len() as u64;
            let mut rgba = self.take_profile_rgba_scratch(profile_surface_bytes, [255; 4]);
            rgba.fill(255);
            let mut yuv_scratch = self.take_jpeg_yuv420_scratch();
            let yuv_scratch_len = crate::jpeg_turbo::yuv420_scratch_len(width, height)?;
            yuv_scratch.resize(yuv_scratch_len, 0);
            yuv_scratch.fill(0xa5);
            yuv_scratch.fill(0);
            let yuv_probe = crate::jpeg_turbo::encode_rgba_avx512_yuv420_with_scratch(
                &rgba,
                width,
                height,
                90,
                &mut yuv_scratch,
            );
            self.recycle_profile_rgba_scratch(rgba);
            match yuv_probe {
                Ok(encoded) => report.jpeg_yuv420_prewarm_bytes = encoded.len() as u64,
                Err(error) if error == "AVX-512 JPEG YUV420 is unavailable" => {}
                Err(error) => return Err(format!("JPEG raw YUV420 prewarm failed: {error}")),
            }
            report.jpeg_yuv420_scratch_bytes = yuv_scratch_len as u64;
            report.jpeg_yuv420_scratch_page_touch_count = yuv_scratch_len.div_ceil(4096) as u64;
            self.recycle_jpeg_yuv420_scratch(yuv_scratch);
        }
        report.elapsed_ns = elapsed_ns(started);
        Ok(report)
    }
}

/// What [`CustomProfileRenderer::prewarm`] touched.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize)]
///
/// Counts are zero for resources that are not installed. Each checksum folds
/// the bytes read while touching pages: it keeps the reads observable and
/// serves as a compact diagnostic, not as a content hash.
pub struct PrewarmReport {
    /// Shape sources decoded and checked against the shape atlas.
    pub shape_source_count: u64,
    /// RGBA bytes of the decoded shape sources.
    pub shape_decoded_bytes: u64,
    /// Render objects touched in the pinned store.
    pub render_object_count: u64,
    /// Bytes of those render objects.
    pub render_object_bytes: u64,
    /// Memory pages touched in the render-object store.
    pub render_object_page_touch_count: u64,
    /// Checksum of the render-object bytes read.
    pub render_object_checksum: u64,
    /// Text atlases touched.
    pub text_atlas_count: u64,
    /// Pages of those atlases.
    pub text_atlas_page_count: u64,
    /// Mapped bytes of those atlases.
    pub text_atlas_mapped_bytes: u64,
    /// Memory pages touched in the text atlases.
    pub text_atlas_page_touch_count: u64,
    /// Checksum of the text atlas bytes read.
    pub text_atlas_checksum: u64,
    /// Font families resolved to font files.
    pub font_family_count: u64,
    /// Time spent resolving those families, in nanoseconds.
    pub font_resolve_ns: u64,
    /// Shape atlases touched (0 or 1).
    pub shape_atlas_count: u64,
    /// Pages of the shape atlas.
    pub shape_atlas_page_count: u64,
    /// Mapped bytes of the shape atlas.
    pub shape_atlas_mapped_bytes: u64,
    /// Memory pages touched in the shape atlas.
    pub shape_atlas_page_touch_count: u64,
    /// Checksum of the shape atlas bytes read.
    pub shape_atlas_checksum: u64,
    /// Shape row programs built.
    pub shape_row_program_count: u64,
    /// Runs in those programs.
    pub shape_row_program_run_count: u64,
    /// Resident bytes of those programs.
    pub shape_row_program_bytes: u64,
    /// Time spent building them, in nanoseconds.
    pub shape_row_program_build_ns: u64,
    /// Bytes of the page surface scratch buffer.
    pub profile_surface_scratch_bytes: u64,
    /// Memory pages touched in that buffer.
    pub profile_surface_page_touch_count: u64,
    /// Time spent initializing the page surface, in nanoseconds.
    pub profile_surface_init_ns: u64,
    /// Size of the probe JPEG the standard encoder produced (feature
    /// `jpeg-turbo`).
    pub jpeg_encoder_prewarm_bytes: u64,
    /// Size of the probe JPEG the AVX-512 YUV 4:2:0 encoder produced; 0 when
    /// that encoder is unavailable.
    pub jpeg_yuv420_prewarm_bytes: u64,
    /// Bytes of the YUV 4:2:0 scratch buffer.
    pub jpeg_yuv420_scratch_bytes: u64,
    /// Memory pages touched in that buffer.
    pub jpeg_yuv420_scratch_page_touch_count: u64,
    /// Total prewarm time, in nanoseconds.
    pub elapsed_ns: u64,
}
