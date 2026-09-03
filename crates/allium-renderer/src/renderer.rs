//! 场景图渲染器高层 API。

use crate::assets::AssetStore;
use crate::masterdata::{MasterData, MasterDataProvider};
use crate::types::CustomProfileCard;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::sync::{Arc, RwLock};

/// Immutable render-object generation pinned for one external render request.
/// Cloning this token clones only the `Arc`, never the mapped pages.
#[derive(Clone, Default)]
pub struct RenderObjectGenerationPin {
    store: Option<Arc<crate::render_object::MappedRenderObjectStore>>,
}

impl RenderObjectGenerationPin {
    pub fn store(&self) -> Option<&crate::render_object::MappedRenderObjectStore> {
        self.store.as_deref()
    }

    pub fn manifest_sha256(&self) -> Option<&str> {
        self.store.as_deref().map(|store| store.manifest_sha256())
    }

    pub fn source_identity(&self) -> Option<&str> {
        self.store
            .as_deref()
            .map(|store| store.manifest().source_identity.as_str())
    }
}

/// 自定义名片渲染器。
pub struct CustomProfileRenderer {
    md_source: RwLock<Arc<dyn MasterDataProvider>>,
    assets: Option<Arc<AssetStore>>,
    /// Hot-swappable so a generated fallback atlas can be published without
    /// rebuilding the renderer.
    sdf_atlases: arc_swap::ArcSwap<crate::sdf::atlas::MappedSdfAtlasSet>,
    profile_fallback_sdf_cache: Option<Arc<crate::sdf::fallback_cache::PersistentFallbackSdfCache>>,
    shape_sdf_atlas: Option<Arc<crate::sdf::shape_atlas::MappedShapeSdfAtlas>>,
    render_object_generations: Option<Arc<crate::render_object::RenderObjectGenerationManager>>,
    shape_row_program_cache: crate::sdf::tile::ShapeRowProgramCache,
    /// Reused between requests so a full-page surface is not reallocated per
    /// render. Recycled buffers are always returned empty.
    profile_rgba_scratch: Mutex<Vec<u8>>,
    jpeg_yuv420_scratch: Mutex<Vec<u8>>,
    /// Whether a glyph too large for any installed atlas tier may be generated
    /// during the request instead of falling back to an installed tier.
    realtime_oversized_glyph_generation: bool,
}

impl CustomProfileRenderer {
    /// 用 MasterData provider 初始化渲染器。
    pub fn new(provider: Arc<dyn MasterDataProvider>) -> Self {
        let md = MasterData::new(Arc::clone(&provider));
        tracing::info!(
            colors = md.color_count(),
            fonts = md.font_count(),
            "自定义名片渲染器初始化完成"
        );
        Self {
            md_source: RwLock::new(provider),
            assets: None,
            sdf_atlases: arc_swap::ArcSwap::from_pointee(
                crate::sdf::atlas::MappedSdfAtlasSet::new(),
            ),
            profile_fallback_sdf_cache: None,
            shape_sdf_atlas: None,
            render_object_generations: None,
            shape_row_program_cache: crate::sdf::tile::ShapeRowProgramCache::default(),
            profile_rgba_scratch: Mutex::new(Vec::new()),
            jpeg_yuv420_scratch: Mutex::new(Vec::new()),
            realtime_oversized_glyph_generation: true,
        }
    }

    /// 设置素材缓存。
    pub fn with_assets(mut self, assets: Arc<AssetStore>) -> Self {
        self.assets = Some(assets);
        self
    }

    pub fn with_sdf_atlases(self, atlases: Arc<crate::sdf::atlas::MappedSdfAtlasSet>) -> Self {
        self.sdf_atlases.store(atlases);
        self
    }

    /// Installs the persistent FreeType fallback glyph cache.
    ///
    /// Codepoints missing from the declared font's atlas are generated from the
    /// fallback face and published into the atlas set, so layout and rendering
    /// stay on FreeType instead of substituting another engine's metrics.
    pub fn with_profile_fallback_sdf_cache(
        mut self,
        cache: Arc<crate::sdf::fallback_cache::PersistentFallbackSdfCache>,
    ) -> Self {
        self.profile_fallback_sdf_cache = Some(cache);
        self
    }

    #[cfg(feature = "animation-export")]
    pub(crate) fn mapped_text_sdf_atlases(
        &self,
    ) -> Option<Arc<crate::sdf::atlas::MappedSdfAtlasSet>> {
        let atlases = self.sdf_atlases.load_full();
        (!atlases.is_empty()).then_some(atlases)
    }

    pub fn with_shape_sdf_atlas(
        mut self,
        atlas: Arc<crate::sdf::shape_atlas::MappedShapeSdfAtlas>,
    ) -> Self {
        self.shape_sdf_atlas = Some(atlas);
        self
    }

    pub fn with_render_object_store(
        mut self,
        store: Arc<crate::render_object::MappedRenderObjectStore>,
    ) -> Self {
        self.render_object_generations = Some(Arc::new(
            crate::render_object::RenderObjectGenerationManager::new(store),
        ));
        self
    }

    pub fn profile_backend_capabilities(
        &self,
    ) -> crate::profile_backend::ProfileBackendCapabilities {
        let simd = turin_sdf_simd_available();
        let sdf_atlases_available = !self.sdf_atlases.load().is_empty();
        crate::profile_backend::ProfileBackendCapabilities {
            skia_raster_cpu: true,
            skia_opengl_llvmpipe: false,
            skia_vulkan_lavapipe: false,
            // The native surface renders every element through the software
            // image path, so it needs the packet executor, both atlas kinds,
            // and the pre-decoded render-object store to all be installed.
            native_raster_cpu: simd
                && sdf_atlases_available
                && self.shape_sdf_atlas.is_some()
                && self.render_object_generations.is_some(),
            text_legacy_skia: true,
            text_simd: simd && sdf_atlases_available,
            text_scalar_oracle: sdf_atlases_available,
            shape_skia: true,
            shape_simd: simd && self.shape_sdf_atlas.is_some(),
            shape_scalar_oracle: self.shape_sdf_atlas.is_some(),
        }
    }

    /// Pre-warms the FreeType fallback glyphs this card needs.
    ///
    /// A no-op unless a fallback cache is installed, so callers that supply
    /// complete atlases pay nothing. Resolving the card here keeps glyph
    /// generation off the compositing path.
    /// and compatibility callers may retain the bounded realtime EDT path.
    pub fn with_realtime_oversized_glyph_generation(mut self, enabled: bool) -> Self {
        self.realtime_oversized_glyph_generation = enabled;
        self
    }

    /// Installs an immutable font atlas into this renderer instance. Conflicting
    /// identities for the same family are rejected before any request runs.
    pub fn with_sdf_atlas(
        self,
        atlas: Arc<crate::sdf::atlas::MappedSdfAtlas>,
    ) -> Result<Self, crate::sdf::atlas::SdfAtlasError> {
        let mut atlases = (*self.sdf_atlases.load_full()).clone();
        atlases.insert(atlas)?;
        self.sdf_atlases.store(Arc::new(atlases));
        Ok(self)
    }

    pub fn with_render_object_generation_manager(
        mut self,
        manager: Arc<crate::render_object::RenderObjectGenerationManager>,
    ) -> Self {
        self.render_object_generations = Some(manager);
        self
    }

    pub fn render_object_generation_manager(
        &self,
    ) -> Option<Arc<crate::render_object::RenderObjectGenerationManager>> {
        self.render_object_generations.clone()
    }

    pub fn pin_render_object_generation(&self) -> RenderObjectGenerationPin {
        RenderObjectGenerationPin {
            store: self
                .render_object_generations
                .as_ref()
                .map(|manager| manager.current()),
        }
    }

    /// Builds one canonical server-side General base from an already resolved
    /// semantic scene for the standalone resource-pipeline builder.
    pub fn build_general_base_objects(
        &self,
        scene: &allium_renderer_core::profile_scene::ResolvedProfileScene,
        authored_index: u32,
    ) -> Result<Vec<crate::profile_compositor::GeneralBaseBuildOutput>, String> {
        let generation = self.pin_render_object_generation();
        let store = generation
            .store()
            .ok_or_else(|| "General base builder requires a render-object store".to_string())?;
        self.build_general_base_objects_for_store(scene, authored_index, store)
    }

    pub fn build_general_base_objects_for_store(
        &self,
        scene: &allium_renderer_core::profile_scene::ResolvedProfileScene,
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

    fn take_profile_rgba_scratch(&self, len: usize, clear_rgba: [u8; 4]) -> Vec<u8> {
        let mut scratch = self
            .profile_rgba_scratch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut rgba: Vec<u8> = std::mem::take(&mut *scratch);
        drop(scratch);
        debug_assert!(rgba.is_empty());
        // Recycled buffers always have length zero. resize() initializes every
        // byte in the requested surface, so a second fill(0) only doubles the
        // memory bandwidth cost for large animation expansions.
        rgba.resize(len, 0);
        let _ = clear_rgba;
        rgba
    }

    fn recycle_profile_rgba_scratch(&self, mut rgba: Vec<u8>) {
        let mut scratch = self
            .profile_rgba_scratch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if scratch.is_empty() {
            rgba.clear();
            *scratch = rgba;
        }
    }

    fn take_jpeg_yuv420_scratch(&self) -> Vec<u8> {
        let mut scratch = self
            .jpeg_yuv420_scratch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *scratch)
    }

    fn recycle_jpeg_yuv420_scratch(&self, mut buffer: Vec<u8>) {
        let mut scratch = self
            .jpeg_yuv420_scratch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if scratch.is_empty() {
            buffer.clear();
            *scratch = buffer;
        }
    }

    /// immutable atlas identities actually installed in this process.
    pub fn profile_backend_cache_identity(
        &self,
        config: &crate::profile_backend::ProfileBackendConfig,
    ) -> Result<String, String> {
        let generation = self.pin_render_object_generation();
        self.profile_backend_cache_identity_for_generation(config, &generation)
    }

    pub fn profile_backend_cache_identity_for_generation(
        &self,
        config: &crate::profile_backend::ProfileBackendConfig,
        generation: &RenderObjectGenerationPin,
    ) -> Result<String, String> {
        use sha2::{Digest, Sha256};

        let mut digest = Sha256::new();
        digest.update(crate::profile_backend::PROFILE_RENDER_CACHE_SCHEMA.as_bytes());
        digest.update(
            serde_json::to_vec(config)
                .map_err(|error| format!("serialize profile backend config: {error}"))?,
        );
        let sdf_atlases = self.sdf_atlases.load_full();
        for atlas in sdf_atlases.iter() {
            digest.update(atlas.manifest().font_family.as_bytes());
            digest.update(atlas.manifest_sha256().as_bytes());
        }
        if let Some(atlas) = self.shape_sdf_atlas.as_deref() {
            digest.update(atlas.manifest_sha256().as_bytes());
        }
        if let Some(store) = generation.store() {
            digest.update(store.manifest().source_identity.as_bytes());
            digest.update(store.manifest_sha256().as_bytes());
        }
        let identity = hex::encode(digest.finalize());
        Ok(format!("profile-backend-{}", &identity[..16]))
    }

    /// Generates any codepoints the declared fonts' atlases lack, from the
    /// FreeType fallback face, and publishes the result into the atlas set.
    ///
    /// Runs before rendering so the request path only ever reads atlases; it
    /// never generates glyphs while compositing.
    fn ensure_profile_fallback_for_scenes<'a>(
        &self,
        scenes: impl IntoIterator<Item = &'a allium_renderer_core::profile_scene::ResolvedProfileScene>,
        md: &MasterData,
    ) -> Result<Option<crate::sdf::fallback_cache::PersistentFallbackSdfCacheReport>, String> {
        use allium_renderer_core::{FontRole, SemanticCommandPayload, TextSource};
        use std::collections::BTreeSet;

        let Some(cache) = self.profile_fallback_sdf_cache.as_deref() else {
            return Ok(None);
        };
        let atlases = self.sdf_atlases.load_full();
        let mut requested = BTreeSet::new();
        for scene in scenes {
            for command in &scene.commands {
                // Live-master progress text is player state, not authored glyphs.
                if command.role.starts_with("honor-") && command.role.ends_with("-progress") {
                    continue;
                }
                let SemanticCommandPayload::Text {
                    source, font_role, ..
                } = &command.payload
                else {
                    continue;
                };
                let font_id = match font_role {
                    FontRole::RegionFontId(font_id) => *font_id,
                };
                let Some(primary_family) = md.resolve_font(font_id) else {
                    continue;
                };
                if primary_family == crate::sdf::atlas::PROFILE_TEXT_FALLBACK_FONT_FAMILY {
                    continue;
                }
                let Some((_, primary_atlas)) = atlases.atlas_for_font_family(&primary_family)
                else {
                    continue;
                };
                let value = match source {
                    TextSource::Authored { value }
                    | TextSource::ProfileField { value, .. }
                    | TextSource::MasterData { value, .. }
                    | TextSource::Localized { value, .. } => value,
                };
                requested.extend(value.chars().filter_map(|ch| {
                    (!ch.is_whitespace()
                        && !ch.is_control()
                        && primary_atlas.glyph(u32::from(ch)).is_none())
                    .then_some(u32::from(ch))
                }));
            }
        }
        self.ensure_profile_fallback_codepoints(requested, cache, &atlases)
    }

    /// Generates the fallback glyphs an animation export needs from a
    /// resolved scene, installing the resulting atlas the same way a page
    /// render does.
    #[cfg(feature = "animation-export")]
    pub(crate) fn prepare_profile_fallback_for_animation_scene(
        &self,
        scene: &allium_renderer_core::profile_scene::ResolvedProfileScene,
        md: &MasterData,
    ) -> Result<Option<crate::sdf::fallback_cache::PersistentFallbackSdfCacheReport>, String> {
        if self.profile_fallback_sdf_cache.is_none() {
            return Ok(None);
        }
        self.ensure_profile_fallback_for_scenes(std::iter::once(scene), md)
    }

    /// The same preparation for a card that resolved to no scene: the
    /// codepoints come straight off the authored text elements.
    #[cfg(feature = "animation-export")]
    pub(crate) fn prepare_profile_fallback_for_animation_card(
        &self,
        card: &CustomProfileCard,
        md: &MasterData,
    ) -> Result<Option<crate::sdf::fallback_cache::PersistentFallbackSdfCacheReport>, String> {
        use std::collections::BTreeSet;

        let Some(cache) = self.profile_fallback_sdf_cache.as_deref() else {
            return Ok(None);
        };
        let atlases = self.sdf_atlases.load_full();
        let mut requested = BTreeSet::new();
        for text in &card.texts {
            let Some(primary_family) = md.resolve_font(text.font_id) else {
                continue;
            };
            if primary_family == crate::sdf::atlas::PROFILE_TEXT_FALLBACK_FONT_FAMILY {
                continue;
            }
            let Some((_, primary_atlas)) = atlases.atlas_for_font_family(&primary_family) else {
                continue;
            };
            requested.extend(text.text.chars().filter_map(|ch| {
                (!ch.is_whitespace()
                    && !ch.is_control()
                    && primary_atlas.glyph(u32::from(ch)).is_none())
                .then_some(u32::from(ch))
            }));
        }
        self.ensure_profile_fallback_codepoints(requested, cache, &atlases)
    }

    fn ensure_profile_fallback_codepoints(
        &self,
        requested: std::collections::BTreeSet<u32>,
        cache: &crate::sdf::fallback_cache::PersistentFallbackSdfCache,
        atlases: &crate::sdf::atlas::MappedSdfAtlasSet,
    ) -> Result<Option<crate::sdf::fallback_cache::PersistentFallbackSdfCacheReport>, String> {
        if requested.is_empty() {
            return Ok(None);
        }

        let (atlas, report) = cache.ensure_codepoints(&requested)?;
        if let Some(atlas) = atlas {
            let already_published = atlases
                .atlas_for_font_family(cache.font_family())
                .is_some_and(|(_, installed)| {
                    installed.manifest_sha256() == atlas.manifest_sha256()
                });
            if !already_published {
                let mut updated = (*self.sdf_atlases.load_full()).clone();
                updated
                    .replace_or_insert(atlas)
                    .map_err(|error| error.to_string())?;
                self.sdf_atlases.store(Arc::new(updated));
            }
        }
        Ok(Some(report))
    }

    /// 热替换 MasterData provider。
    pub fn swap_masterdata(&self, new_provider: Arc<dyn MasterDataProvider>) {
        let md = MasterData::new(Arc::clone(&new_provider));
        tracing::info!(
            colors = md.color_count(),
            fonts = md.font_count(),
            "MasterData 热替换完成"
        );
        *self.md_source.write().unwrap_or_else(|e| e.into_inner()) = new_provider;
    }

    fn snapshot(&self) -> MasterData {
        let provider = self
            .md_source
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        MasterData::new(provider)
    }

    pub fn snapshot_masterdata(&self) -> MasterData {
        self.snapshot()
    }

    /// 获取 MasterData 快照。
    pub fn masterdata(&self) -> MasterData {
        self.snapshot()
    }

    /// 获取内部 AssetStore。
    pub fn assets(&self) -> Option<&Arc<AssetStore>> {
        self.assets.as_ref()
    }

    /// 预校验名片数据。
    pub fn validate_card(&self, card: &CustomProfileCard) -> Vec<String> {
        let md = self.snapshot();
        let mut warnings = Vec::new();
        for (i, text) in card.texts.iter().enumerate() {
            if md.resolve_color(text.color_id).is_none() {
                warnings.push(format!(
                    "texts[{i}]: colorId={} 不在映射表中",
                    text.color_id
                ));
            }
            if md.resolve_font(text.font_id).is_none() {
                warnings.push(format!("texts[{i}]: fontId={} 不在映射表中", text.font_id));
            }
        }
        for (i, shape) in card.shapes.iter().enumerate() {
            if md.resolve_color(shape.color_id).is_none() {
                warnings.push(format!(
                    "shapes[{i}]: colorId={} 不在映射表中",
                    shape.color_id
                ));
            }
            if md.resolve_resource("shape", shape.id).is_none() {
                warnings.push(format!("shapes[{i}]: shapeId={} 不在映射表中", shape.id));
            }
        }
        if !warnings.is_empty() {
            tracing::warn!(count = warnings.len(), "名片数据校验发现缺失映射");
        }
        warnings
    }

    /// 填充名片中的 Honor/BondsHonor 等级。
    pub fn enrich_honor_levels(
        &self,
        card: &mut CustomProfileCard,
        honor_levels: &std::collections::HashMap<i32, i32>,
        bonds_levels: &std::collections::HashMap<i32, i32>,
        char_ranks: &std::collections::HashMap<i32, i32>,
    ) {
        let md = self.snapshot();
        for honor in &mut card.honors {
            if let Some(&level) = honor_levels.get(&honor.id) {
                honor.honor_level = level;
            } else if let Some(res) = md.resolve_honor(honor.id, 1) {
                if res.honor_type == "character" {
                    if let Some(entry) = md.get_honor(honor.id) {
                        if let Some(group_id) = entry.group_id {
                            if let Some(&rank) = char_ranks.get(&group_id) {
                                honor.honor_level = rank;
                            }
                        }
                    }
                }
            }
        }
        for bond in &mut card.bonds_honors {
            if let Some(&level) = bonds_levels.get(&bond.id) {
                bond.honor_level = level;
            }
        }
    }

    /// Decode and validate the fixed game Shape sources before the worker is
    /// declared ready. This keeps source RG8 hashing out of the first unseen
    /// profile request while preserving the atlas/source identity gate.
    pub fn prewarm_profile_backend_resources(&self) -> Result<ProfileBackendPrewarmReport, String> {
        let started = std::time::Instant::now();
        let mut report = ProfileBackendPrewarmReport::default();
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

    /// Executes only asset-backed Shape elements through the typed RG8 scalar
    /// oracle. This is a comparison primitive, not a production surface: any
    /// missing resource, asset, identity or affine transform fails closed.
    pub fn render_shape_sdf_scalar_oracle(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
    ) -> Result<ShapeSdfScalarOracleOutput, String> {
        self.render_shape_sdf_candidate(card, profile, ShapeSdfCandidateExecutor::ScalarRgba8)
    }

    /// Executes Shape commands with the Turin AVX-512 FP32 tile candidate.
    /// Unsupported CPUs and non-swizzled atlases fail closed.
    pub fn render_shape_sdf_simd_candidate(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
    ) -> Result<ShapeSdfExecutionOutput, String> {
        self.render_shape_sdf_candidate(card, profile, ShapeSdfCandidateExecutor::SimdF32)
    }

    /// Executes the ordered Text+Shape layer through the scalar FP32 oracle.
    /// Missing atlas entries and unsupported transforms fail closed.
    pub fn render_sdf_layer_scalar_f32_candidate(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
    ) -> Result<SdfLayerExecutionOutput, String> {
        self.render_sdf_layer_candidate(card, profile, SdfLayerCandidateExecutor::ScalarF32)
    }

    /// Executes the exact same ordered Text+Shape plan through the Turin
    /// AVX-512 FP32 executor. Unsupported CPUs fail closed.
    pub fn render_sdf_layer_simd_candidate(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
    ) -> Result<SdfLayerExecutionOutput, String> {
        self.render_sdf_layer_candidate(card, profile, SdfLayerCandidateExecutor::SimdF32)
    }

    fn render_sdf_layer_candidate(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        executor: SdfLayerCandidateExecutor,
    ) -> Result<SdfLayerExecutionOutput, String> {
        let total_started = std::time::Instant::now();
        let width = crate::transform::CANVAS_WIDTH as u32;
        let height = crate::transform::CANVAS_HEIGHT as u32;
        let md = self.snapshot();
        let sdf_atlases = self.sdf_atlases.load_full();

        let capture_started = std::time::Instant::now();
        let captured = capture_sdf_primitives(
            card,
            &md,
            self.assets.as_deref(),
            Some(&sdf_atlases),
            profile,
            width,
            height,
            SdfCaptureKinds::TEXT_AND_SHAPE,
        )?;
        let capture_ns = elapsed_ns(capture_started);

        let source = crate::sdf::tile::MixedSdfAtlasSource::new(
            &sdf_atlases,
            self.shape_sdf_atlas.as_deref(),
        )
        .map_err(|error| error.to_string())?;
        let mapping_started = std::time::Instant::now();
        let commands = map_captured_sdf_commands(
            &captured.primitives,
            &sdf_atlases,
            self.shape_sdf_atlas.as_deref(),
            &source,
            None,
            None,
            None,
            None,
        )?;
        let command_mapping_ns = elapsed_ns(mapping_started);

        let plan_started = std::time::Instant::now();
        let plan = crate::sdf::tile::SdfTilePlan::build(
            crate::sdf::tile::TileGrid::new(width, height),
            &commands,
            &source,
        )
        .map_err(|error| error.to_string())?;
        let plan_build_ns = elapsed_ns(plan_started);

        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        let execute_started = std::time::Instant::now();
        let execution_stats = match executor {
            SdfLayerCandidateExecutor::ScalarF32 => plan
                .execute_scalar_f32(&source, [0, 0, 0, 0], &mut rgba)
                .map_err(|error| error.to_string())?,
            SdfLayerCandidateExecutor::SimdF32 => plan
                .execute_simd(
                    &source,
                    [0, 0, 0, 0],
                    &mut rgba,
                    crate::sdf::tile::SdfAccumulationMode::F32Tile,
                )
                .map_err(|error| error.to_string())?,
        };
        let execute_ns = elapsed_ns(execute_started);
        Ok(SdfLayerExecutionOutput {
            rgba,
            width,
            height,
            captured_text_count: captured.text_count,
            captured_shape_count: captured.shape_count,
            plan_stats: plan.stats(),
            execution_stats,
            atlas_mapped_bytes: sdf_atlases.mapped_bytes().saturating_add(
                self.shape_sdf_atlas.as_deref().map_or(
                    0,
                    crate::sdf::shape_atlas::MappedShapeSdfAtlas::mapped_bytes,
                ),
            ),
            plan_bytes: plan.resident_bytes(),
            span_bytes: plan.span_bytes(),
            timings: SdfLayerExecutionTimings {
                capture_ns,
                command_mapping_ns,
                plan_build_ns,
                execute_ns,
                total_ns: elapsed_ns(total_started),
            },
        })
    }

    /// Renders a complete card while replacing every contiguous Text/Shape
    /// run with the scalar FP32 SDF oracle. Non-SDF elements continue through
    /// the existing Skia raster path at their authored positions.
    pub fn render_full_card_sdf_scalar_f32_candidate(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
    ) -> Result<FullCardSdfExecutionOutput, String> {
        self.render_full_card_sdf_candidate(
            card,
            profile,
            Some(FullCardSdfCandidateExecutor::ScalarF32),
            Some(FullCardSdfCandidateExecutor::ScalarF32),
            32,
            32,
            false,
            false,
            [255, 255, 255, 255],
        )
    }

    /// Formal mixed path used by the first opt-in stage: Text uses the scalar
    /// atlas oracle while Shape and all other elements remain on legacy Skia.
    pub fn render_full_card_text_sdf_scalar_f32_candidate(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
    ) -> Result<FullCardSdfExecutionOutput, String> {
        self.render_full_card_sdf_candidate(
            card,
            profile,
            Some(FullCardSdfCandidateExecutor::ScalarF32),
            None,
            32,
            32,
            false,
            false,
            [255, 255, 255, 255],
        )
    }

    /// Renders the same ordered full-card plan through the Turin AVX-512
    /// executor. Unsupported CPUs and missing atlas entries fail closed.
    pub fn render_full_card_sdf_simd_candidate(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
    ) -> Result<FullCardSdfExecutionOutput, String> {
        self.render_full_card_sdf_candidate(
            card,
            profile,
            Some(FullCardSdfCandidateExecutor::SimdF32),
            Some(FullCardSdfCandidateExecutor::SimdF32),
            32,
            32,
            false,
            false,
            [255, 255, 255, 255],
        )
    }

    /// Production-shape-preserving Text-only Turin candidate. Shape and all
    /// non-Text elements remain on legacy Skia so the approved Text oracle can
    /// be inherited independently from the still-unapproved Shape backend.
    pub fn render_full_card_text_sdf_simd_candidate(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
    ) -> Result<FullCardSdfExecutionOutput, String> {
        self.render_full_card_sdf_candidate(
            card,
            profile,
            Some(FullCardSdfCandidateExecutor::SimdF32),
            None,
            32,
            32,
            false,
            false,
            [255, 255, 255, 255],
        )
    }

    /// Transparent counterpart used to quantify alpha coverage and bounds.
    pub fn render_full_card_sdf_scalar_f32_transparent_candidate(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
    ) -> Result<FullCardSdfExecutionOutput, String> {
        self.render_full_card_sdf_candidate(
            card,
            profile,
            Some(FullCardSdfCandidateExecutor::ScalarF32),
            Some(FullCardSdfCandidateExecutor::ScalarF32),
            32,
            32,
            false,
            false,
            [0, 0, 0, 0],
        )
    }

    pub fn render_full_card_text_sdf_scalar_f32_transparent_candidate(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
    ) -> Result<FullCardSdfExecutionOutput, String> {
        self.render_full_card_sdf_candidate(
            card,
            profile,
            Some(FullCardSdfCandidateExecutor::ScalarF32),
            None,
            32,
            32,
            false,
            false,
            [0, 0, 0, 0],
        )
    }

    /// Transparent AVX-512 counterpart. Unsupported CPUs still fail closed.
    pub fn render_full_card_sdf_simd_transparent_candidate(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
    ) -> Result<FullCardSdfExecutionOutput, String> {
        self.render_full_card_sdf_candidate(
            card,
            profile,
            Some(FullCardSdfCandidateExecutor::SimdF32),
            Some(FullCardSdfCandidateExecutor::SimdF32),
            32,
            32,
            false,
            false,
            [0, 0, 0, 0],
        )
    }

    fn render_full_card_sdf_candidate(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        text_executor: Option<FullCardSdfCandidateExecutor>,
        shape_executor: Option<FullCardSdfCandidateExecutor>,
        tile_width: u16,
        tile_height: u16,
        pixel_occlusion_dry_run: bool,
        pixel_occlusion_execute: bool,
        clear_rgba: [u8; 4],
    ) -> Result<FullCardSdfExecutionOutput, String> {
        let generation = self.pin_render_object_generation();
        self.render_full_card_sdf_candidate_with_scene(
            card,
            profile,
            text_executor,
            shape_executor,
            tile_width,
            tile_height,
            pixel_occlusion_dry_run,
            pixel_occlusion_execute,
            clear_rgba,
            false,
            None,
            generation.store(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_full_card_sdf_candidate_with_scene(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        text_executor: Option<FullCardSdfCandidateExecutor>,
        shape_executor: Option<FullCardSdfCandidateExecutor>,
        tile_width: u16,
        tile_height: u16,
        pixel_occlusion_dry_run: bool,
        pixel_occlusion_execute: bool,
        clear_rgba: [u8; 4],
        forbid_legacy_elements: bool,
        pre_resolved_scene: Option<&allium_renderer_core::profile_scene::ResolvedProfileScene>,
        render_object_store: Option<&crate::render_object::MappedRenderObjectStore>,
    ) -> Result<FullCardSdfExecutionOutput, String> {
        let md = self.snapshot();
        let render = || {
            self.render_ordered_sdf_surface_candidate(
                card,
                profile,
                text_executor,
                shape_executor,
                tile_width,
                tile_height,
                pixel_occlusion_dry_run,
                pixel_occlusion_execute,
                OrderedSdfSurfaceSpec::full_card(clear_rgba)
                    .with_forbid_legacy_elements(forbid_legacy_elements),
                &md,
                self.assets.as_deref(),
                pre_resolved_scene,
                render_object_store,
            )
        };
        if text_executor.is_none()
            || !self.realtime_oversized_glyph_generation
            || active_realtime_edt_batch().is_some()
        {
            return render();
        }
        let prepared = self.prepare_realtime_edt_batch(
            std::slice::from_ref(card),
            &md,
            self.assets.as_deref(),
            profile,
        )?;
        let prepared_for_telemetry = Arc::clone(&prepared);
        let mut output = self.with_realtime_edt_batch(prepared, render)?;
        output
            .realtime_edt_glyphs
            .extend(prepared_for_telemetry.glyphs.iter().cloned());
        output
            .realtime_edt_batch
            .accumulate(&prepared_for_telemetry.telemetry);
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_ordered_sdf_surface_candidate(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        text_executor: Option<FullCardSdfCandidateExecutor>,
        shape_executor: Option<FullCardSdfCandidateExecutor>,
        tile_width: u16,
        tile_height: u16,
        pixel_occlusion_dry_run: bool,
        pixel_occlusion_execute: bool,
        spec: OrderedSdfSurfaceSpec,
        md: &MasterData,
        assets: Option<&AssetStore>,
        pre_resolved_scene: Option<&allium_renderer_core::profile_scene::ResolvedProfileScene>,
        render_object_store: Option<&crate::render_object::MappedRenderObjectStore>,
    ) -> Result<FullCardSdfExecutionOutput, String> {
        let total_started = std::time::Instant::now();
        let semantic_resolve_started = std::time::Instant::now();
        let resolved_scene = if render_object_store.is_some() && pre_resolved_scene.is_none() {
            Some(
                crate::semantic_resolve::resolve_card_commands_with_resources(
                    card,
                    md,
                    "native:ordered-profile-backend",
                    profile,
                    "cn",
                    crate::semantic_resolve::ResolveResourceContext {
                        assets,
                        render_objects: render_object_store,
                        catalog_lookup_ns: None,
                    },
                )
                .map_err(|error| format!("resolve ordered profile scene: {error}"))?,
            )
        } else {
            None
        };
        let semantic_scene = pre_resolved_scene.or(resolved_scene.as_ref());
        let semantic_resolve_ns = elapsed_ns(semantic_resolve_started);
        let surface_started = std::time::Instant::now();
        let width = spec.surface_width;
        let height = spec.surface_height;
        let row_bytes = width as usize * 4;
        let mut rgba = self.take_profile_rgba_scratch(row_bytes * height as usize, spec.clear_rgba);
        let surface_create_ns = elapsed_ns(surface_started);

        let sdf_atlases = self.sdf_atlases.load_full();
        let empty_text_atlases = crate::sdf::atlas::MappedSdfAtlasSet::new();
        let text_atlases = if text_executor.is_some() {
            &sdf_atlases
        } else {
            &empty_text_atlases
        };
        let shape_atlas = shape_executor
            .is_some()
            .then_some(self.shape_sdf_atlas.as_deref())
            .flatten();
        let source = crate::sdf::tile::MixedSdfAtlasSource::new(text_atlases, shape_atlas)
            .map_err(|error| error.to_string())?;
        let fallback_assets = crate::assets::AssetStore::new(1);
        let grid = crate::sdf::tile::TileGrid {
            canvas_width: width,
            canvas_height: height,
            tile_width,
            tile_height,
        };
        let elements = crate::elements::flatten_and_sort(card);
        let occlusion_dry_run = if pixel_occlusion_dry_run || pixel_occlusion_execute {
            match (semantic_scene, render_object_store) {
                (Some(scene), Some(store)) => Some(build_pixel_occlusion_dry_run(
                    card,
                    &elements,
                    scene,
                    store,
                    text_executor.is_some(),
                    shape_executor.is_some(),
                    width,
                    height,
                )?),
                _ => None,
            }
        } else {
            None
        };
        let mut pending = Vec::new();
        let mut pending_executor = None;
        let mut pending_occlusion_mask = None;
        let mut aggregate = FullCardSdfRunAggregate::default();
        aggregate.prepare_direct_axis_shape = spec.prepare_direct_axis_shape;
        aggregate.realtime_oversized_glyph_generation = self.realtime_oversized_glyph_generation;
        aggregate.render_object_mapped_bytes = render_object_store
            .map(crate::render_object::MappedRenderObjectStore::mapped_bytes)
            .unwrap_or_default();
        aggregate.timings.semantic_resolve_ns = semantic_resolve_ns;
        aggregate.timings.surface_create_ns = surface_create_ns;
        if let Some(dry_run) = occlusion_dry_run.as_ref() {
            aggregate.occlusion_eligible_image_count = dry_run.eligible_image_count;
            aggregate.occlusion_mask_snapshot_count = dry_run.mask_snapshot_count;
            aggregate.occlusion_mask_bytes = dry_run.mask_bytes;
            aggregate.timings.occlusion_mask_build_ns = dry_run.build_ns;
        }
        let clear_started = std::time::Instant::now();
        // `rgba` was allocated with `vec![0; len]`, which is already the exact
        // premultiplied transparent-black surface required by animation
        // layers, so only a non-transparent clear touches the buffer.
        if spec.clear_rgba != [0, 0, 0, 0] {
            fill_premultiplied_clear(&mut rgba, spec.clear_rgba);
        }
        aggregate.timings.surface_clear_ns = elapsed_ns(clear_started);
        let origin = [spec.canvas_origin_x, spec.canvas_origin_y];
        let mut last_element_was_legacy = false;

        for (element_index, element) in elements.into_iter().enumerate() {
            if !element.visible() {
                continue;
            }
            match element {
                crate::elements::RenderElement::Text(_) if text_executor.is_some() => {
                    last_element_was_legacy = false;
                    aggregate.sdf_text_element_count =
                        aggregate.sdf_text_element_count.saturating_add(1);
                    select_full_card_sdf_executor(
                        rgba.as_mut_slice(),
                        &mut pending,
                        &mut pending_executor,
                        &mut pending_occlusion_mask,
                        text_executor.expect("guarded Text SDF executor"),
                        occlusion_dry_run
                            .as_ref()
                            .and_then(|dry_run| dry_run.masks_by_element[element_index].clone()),
                        pixel_occlusion_execute,
                        text_atlases,
                        shape_atlas,
                        &source,
                        grid,
                        &self.shape_row_program_cache,
                        &mut aggregate,
                    )?;
                    let capture_started = std::time::Instant::now();
                    let mut failure = None;
                    let mut observer = |result: Result<
                        crate::text::ResolvedTextSdfGlyph,
                        crate::text::TextSdfCaptureError,
                    >| match result {
                        Ok(glyph) => {
                            aggregate.captured_text_count =
                                aggregate.captured_text_count.saturating_add(1);
                            pending.push(CapturedSdfPrimitive::Text(glyph));
                        }
                        Err(error) => failure = Some(format!("Text: {error:?}")),
                    };
                    let observation_timings = crate::elements::capture_element_sdf(
                        &element,
                        md,
                        assets,
                        spec.canvas_width as f32,
                        spec.canvas_height as f32,
                        origin,
                        Some(text_atlases),
                        Some(&mut observer),
                        None,
                    );
                    aggregate.timings.capture_rich_parse_ns = aggregate
                        .timings
                        .capture_rich_parse_ns
                        .saturating_add(observation_timings.text_capture.rich_parse_ns);
                    aggregate.timings.capture_font_resolve_ns = aggregate
                        .timings
                        .capture_font_resolve_ns
                        .saturating_add(observation_timings.text_capture.font_resolve_ns);
                    aggregate.timings.capture_layout_setup_ns = aggregate
                        .timings
                        .capture_layout_setup_ns
                        .saturating_add(observation_timings.text_capture.layout_setup_ns);
                    aggregate.timings.capture_measure_ns = aggregate
                        .timings
                        .capture_measure_ns
                        .saturating_add(observation_timings.text_capture.measure_ns);
                    aggregate.timings.capture_command_build_ns = aggregate
                        .timings
                        .capture_command_build_ns
                        .saturating_add(observation_timings.text_capture.command_build_ns);
                    aggregate.timings.capture_emit_ns = aggregate
                        .timings
                        .capture_emit_ns
                        .saturating_add(observation_timings.text_capture.emit_ns);
                    aggregate.timings.capture_ns = aggregate
                        .timings
                        .capture_ns
                        .saturating_add(elapsed_ns(capture_started));
                    if let Some(error) = failure {
                        return Err(error);
                    }
                }
                crate::elements::RenderElement::Shape(_) if shape_executor.is_some() => {
                    last_element_was_legacy = false;
                    aggregate.sdf_shape_element_count =
                        aggregate.sdf_shape_element_count.saturating_add(1);
                    select_full_card_sdf_executor(
                        rgba.as_mut_slice(),
                        &mut pending,
                        &mut pending_executor,
                        &mut pending_occlusion_mask,
                        shape_executor.expect("guarded Shape SDF executor"),
                        occlusion_dry_run
                            .as_ref()
                            .and_then(|dry_run| dry_run.masks_by_element[element_index].clone()),
                        pixel_occlusion_execute,
                        text_atlases,
                        shape_atlas,
                        &source,
                        grid,
                        &self.shape_row_program_cache,
                        &mut aggregate,
                    )?;
                    let capture_started = std::time::Instant::now();
                    let mut failure = None;
                    let mut observer = |result: Result<
                        crate::elements::shape::ResolvedShapeSdfCommand,
                        crate::elements::shape::ShapeSdfCaptureError,
                    >| match result {
                        Ok(shape) => {
                            aggregate.captured_shape_count =
                                aggregate.captured_shape_count.saturating_add(1);
                            pending.push(CapturedSdfPrimitive::Shape(shape));
                        }
                        Err(error) => failure = Some(format!("Shape: {error}")),
                    };
                    crate::elements::capture_element_sdf(
                        &element,
                        md,
                        assets,
                        spec.canvas_width as f32,
                        spec.canvas_height as f32,
                        origin,
                        Some(text_atlases),
                        None,
                        Some(&mut observer),
                    );
                    aggregate.timings.capture_ns = aggregate
                        .timings
                        .capture_ns
                        .saturating_add(elapsed_ns(capture_started));
                    if let Some(error) = failure {
                        return Err(error);
                    }
                }
                _ => {
                    flush_active_full_card_sdf_run(
                        rgba.as_mut_slice(),
                        &mut pending,
                        &mut pending_executor,
                        &mut pending_occlusion_mask,
                        pixel_occlusion_execute,
                        text_atlases,
                        shape_atlas,
                        &source,
                        grid,
                        &self.shape_row_program_cache,
                        &mut aggregate,
                    )?;
                    let software_identity = match (semantic_scene, render_object_store) {
                        (Some(_), Some(_))
                            if !matches!(
                                element,
                                crate::elements::RenderElement::Text(_)
                                    | crate::elements::RenderElement::Shape(_)
                            ) =>
                        {
                            render_element_authored_identity(card, &element)
                        }
                        _ => None,
                    };
                    if let (Some(scene), Some(store), Some((kind, index))) =
                        (semantic_scene, render_object_store, software_identity)
                    {
                        last_element_was_legacy = false;
                        let draw_started = std::time::Instant::now();
                        let stats = render_authored_image_into_pixels(
                            rgba.as_mut_slice(),
                            scene,
                            store,
                            text_executor.is_some().then_some(text_atlases),
                            md,
                            assets,
                            kind,
                            index,
                            spec.surface_width,
                            spec.surface_height,
                            spec.canvas_origin_x,
                            spec.canvas_origin_y,
                            // The image compositor follows the page's executor
                            // choice so a scalar host runs a scalar pipeline.
                            text_executor
                                .or(shape_executor)
                                .unwrap_or(FullCardSdfCandidateExecutor::SimdF32),
                        )?;
                        aggregate.software_image_count =
                            aggregate.software_image_count.saturating_add(1);
                        aggregate.software_text_command_count = aggregate
                            .software_text_command_count
                            .saturating_add(stats.text_command_count);
                        aggregate.software_shape_command_count = aggregate
                            .software_shape_command_count
                            .saturating_add(stats.shape_command_count);
                        aggregate.software_skipped_text_command_count = aggregate
                            .software_skipped_text_command_count
                            .saturating_add(stats.skipped_text_command_count);
                        aggregate.software_skipped_shape_command_count = aggregate
                            .software_skipped_shape_command_count
                            .saturating_add(stats.skipped_shape_command_count);
                        aggregate.image_fragment_count = aggregate
                            .image_fragment_count
                            .saturating_add(stats.blended_fragment_count);
                        aggregate.image_simd_packet_count = aggregate
                            .image_simd_packet_count
                            .saturating_add(stats.simd_packet_count);
                        aggregate.general_base_hit_count = aggregate
                            .general_base_hit_count
                            .saturating_add(stats.general_base_hit_count);
                        aggregate.general_base_miss_count = aggregate
                            .general_base_miss_count
                            .saturating_add(stats.general_base_miss_count);
                        aggregate.general_base_baked_command_count = aggregate
                            .general_base_baked_command_count
                            .saturating_add(stats.general_base_baked_command_count);
                        aggregate.general_base_overlay_command_count = aggregate
                            .general_base_overlay_command_count
                            .saturating_add(stats.general_base_overlay_command_count);
                        aggregate.general_base_bytes = aggregate
                            .general_base_bytes
                            .saturating_add(stats.general_base_bytes);
                        aggregate.general_base_avoided_source_bytes = aggregate
                            .general_base_avoided_source_bytes
                            .saturating_add(stats.general_base_avoided_source_bytes);
                        aggregate.deck_art_variant_hit_count = aggregate
                            .deck_art_variant_hit_count
                            .saturating_add(stats.deck_art_variant_hit_count);
                        aggregate.deck_art_variant_miss_count = aggregate
                            .deck_art_variant_miss_count
                            .saturating_add(stats.deck_art_variant_miss_count);
                        aggregate.deck_art_variant_bytes = aggregate
                            .deck_art_variant_bytes
                            .saturating_add(stats.deck_art_variant_bytes);
                        aggregate.deck_art_variant_avoided_source_bytes = aggregate
                            .deck_art_variant_avoided_source_bytes
                            .saturating_add(stats.deck_art_variant_avoided_source_bytes);
                        aggregate.timings.general_base_composite_ns = aggregate
                            .timings
                            .general_base_composite_ns
                            .saturating_add(stats.general_base_composite_ns);
                        aggregate.timings.image_composite_ns = aggregate
                            .timings
                            .image_composite_ns
                            .saturating_add(elapsed_ns(draw_started));
                    } else {
                        if spec.forbid_legacy_elements {
                            return Err(format!(
                                "native surface requires the software image path; {:?} element has no authored identity",
                                profile_command_kind(&element)
                            ));
                        }
                        let _ = (&fallback_assets, last_element_was_legacy);
                        return Err(format!(
                            "the legacy element renderer was retired; {:?} element has no authored identity",
                            profile_command_kind(&element)
                        ));
                    }
                }
            }
        }
        flush_active_full_card_sdf_run(
            rgba.as_mut_slice(),
            &mut pending,
            &mut pending_executor,
            &mut pending_occlusion_mask,
            pixel_occlusion_execute,
            text_atlases,
            shape_atlas,
            &source,
            grid,
            &self.shape_row_program_cache,
            &mut aggregate,
        )?;

        let snapshot_started = std::time::Instant::now();
        if rgba.len() != row_bytes * height as usize {
            return Err(format!(
                "ordered SDF raster surface length mismatch: expected {}, got {}",
                row_bytes * height as usize,
                rgba.len()
            ));
        }
        aggregate.timings.rgba_snapshot_ns = elapsed_ns(snapshot_started);
        aggregate.timings.total_ns = elapsed_ns(total_started);
        Ok(FullCardSdfExecutionOutput {
            rgba,
            width,
            height,
            clear_rgba: spec.clear_rgba,
            sdf_run_count: aggregate.sdf_run_count,
            legacy_run_count: aggregate.legacy_run_count,
            legacy_element_count: aggregate.legacy_element_count,
            legacy_text_count: aggregate.legacy_text_count,
            legacy_shape_count: aggregate.legacy_shape_count,
            legacy_image_count: aggregate.legacy_image_count,
            software_image_count: aggregate.software_image_count,
            software_text_command_count: aggregate.software_text_command_count,
            software_shape_command_count: aggregate.software_shape_command_count,
            software_skipped_text_command_count: aggregate.software_skipped_text_command_count,
            software_skipped_shape_command_count: aggregate.software_skipped_shape_command_count,
            image_fragment_count: aggregate.image_fragment_count,
            image_simd_packet_count: aggregate.image_simd_packet_count,
            general_base_hit_count: aggregate.general_base_hit_count,
            general_base_miss_count: aggregate.general_base_miss_count,
            general_base_baked_command_count: aggregate.general_base_baked_command_count,
            general_base_overlay_command_count: aggregate.general_base_overlay_command_count,
            general_base_bytes: aggregate.general_base_bytes,
            general_base_avoided_source_bytes: aggregate.general_base_avoided_source_bytes,
            deck_art_variant_hit_count: aggregate.deck_art_variant_hit_count,
            deck_art_variant_miss_count: aggregate.deck_art_variant_miss_count,
            deck_art_variant_bytes: aggregate.deck_art_variant_bytes,
            deck_art_variant_avoided_source_bytes: aggregate.deck_art_variant_avoided_source_bytes,
            render_object_mapped_bytes: aggregate.render_object_mapped_bytes,
            sdf_text_element_count: aggregate.sdf_text_element_count,
            sdf_shape_element_count: aggregate.sdf_shape_element_count,
            captured_text_count: aggregate.captured_text_count,
            captured_shape_count: aggregate.captured_shape_count,
            realtime_edt_glyphs: aggregate.realtime_edt_glyphs,
            realtime_edt_batch: aggregate.realtime_edt_batch,
            plan_stats: aggregate.plan_stats,
            execution_stats: aggregate.execution_stats,
            atlas_mapped_bytes: text_atlases
                .mapped_bytes()
                .saturating_add(shape_atlas.map_or(
                    0,
                    crate::sdf::shape_atlas::MappedShapeSdfAtlas::mapped_bytes,
                )),
            plan_bytes: aggregate.plan_bytes,
            span_bytes: aggregate.span_bytes,
            occlusion: aggregate.occlusion,
            occlusion_eligible_image_count: aggregate.occlusion_eligible_image_count,
            occlusion_mask_snapshot_count: aggregate.occlusion_mask_snapshot_count,
            occlusion_mask_bytes: aggregate.occlusion_mask_bytes,
            timings: aggregate.timings,
        })
    }

    fn render_shape_sdf_candidate(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        executor: ShapeSdfCandidateExecutor,
    ) -> Result<ShapeSdfExecutionOutput, String> {
        let atlas = self
            .shape_sdf_atlas
            .as_deref()
            .ok_or_else(|| "Shape RG8 atlas is not installed".to_string())?;
        let width = crate::transform::CANVAS_WIDTH as u32;
        let height = crate::transform::CANVAS_HEIGHT as u32;
        let md = self.snapshot();
        let captured = capture_sdf_primitives(
            card,
            &md,
            self.assets.as_deref(),
            None,
            profile,
            width,
            height,
            SdfCaptureKinds::SHAPE,
        )?;

        let empty_text = crate::sdf::atlas::MappedSdfAtlasSet::new();
        let source = crate::sdf::tile::MixedSdfAtlasSource::new(&empty_text, Some(atlas))
            .map_err(|error| error.to_string())?;
        let commands = map_captured_sdf_commands(
            &captured.primitives,
            &empty_text,
            Some(atlas),
            &source,
            None,
            None,
            None,
            None,
        )?;
        let grid = crate::sdf::tile::TileGrid::new(width, height);
        let plan = crate::sdf::tile::SdfTilePlan::build(grid, &commands, &source)
            .map_err(|error| error.to_string())?;
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        let execution_stats = match executor {
            ShapeSdfCandidateExecutor::ScalarRgba8 => plan
                .execute_scalar(&source, [0, 0, 0, 0], &mut rgba)
                .map_err(|error| error.to_string())?,
            ShapeSdfCandidateExecutor::SimdF32 => plan
                .execute_simd(
                    &source,
                    [0, 0, 0, 0],
                    &mut rgba,
                    crate::sdf::tile::SdfAccumulationMode::F32Tile,
                )
                .map_err(|error| error.to_string())?,
        };
        Ok(ShapeSdfExecutionOutput {
            rgba,
            width,
            height,
            plan_stats: plan.stats(),
            execution_stats,
            atlas_mapped_bytes: atlas.mapped_bytes(),
            plan_bytes: plan.resident_bytes(),
            span_bytes: plan.span_bytes(),
        })
    }

    /// Explicit custom-profile backend entry point. The existing `render_page`
    /// API remains pinned to the production legacy path. Candidate requests are
    /// resolved against resources that are actually installed in this process;
    /// unavailable stages either fail closed or emit a page-fallback event.
    pub fn render_page_with_backend(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        config: crate::profile_backend::ProfileBackendConfig,
    ) -> Result<
        crate::profile_backend::ProfileBackendRenderOutput,
        crate::profile_backend::ProfileBackendRenderError,
    > {
        let generation = self.pin_render_object_generation();
        self.render_page_with_backend_generation(card, profile, config, &generation)
    }

    pub fn render_page_with_backend_generation(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        config: crate::profile_backend::ProfileBackendConfig,
        generation: &RenderObjectGenerationPin,
    ) -> Result<
        crate::profile_backend::ProfileBackendRenderOutput,
        crate::profile_backend::ProfileBackendRenderError,
    > {
        self.render_page_with_backend_scene(card, profile, config, None, true, generation.store())
    }

    fn render_page_with_backend_scene(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        config: crate::profile_backend::ProfileBackendConfig,
        pre_resolved_scene: Option<&allium_renderer_core::profile_scene::ResolvedProfileScene>,
        prepare_fallback_glyphs: bool,
        render_object_store: Option<&crate::render_object::MappedRenderObjectStore>,
    ) -> Result<
        crate::profile_backend::ProfileBackendRenderOutput,
        crate::profile_backend::ProfileBackendRenderError,
    > {
        use crate::profile_backend::{
            ProfileBackendRenderError, ProfileRenderTelemetry, ShapeSdfExecutor, TextSdfExecutor,
            PROFILE_RENDER_CONTRACT_LEGACY_SKIA, PROFILE_RENDER_CONTRACT_ORDERED_SDF_RUNS,
        };

        let total_started = std::time::Instant::now();
        let selection = config.resolve(self.profile_backend_capabilities())?;
        let _collect_telemetry = config.collect_telemetry;
        let actual_text = selection.text_sdf;
        let actual_shape = selection.shape_sdf;
        let mut telemetry = ProfileRenderTelemetry::new(
            config.clone(),
            if actual_text == TextSdfExecutor::LegacySkia && actual_shape == ShapeSdfExecutor::Skia
            {
                PROFILE_RENDER_CONTRACT_LEGACY_SKIA
            } else {
                PROFILE_RENDER_CONTRACT_ORDERED_SDF_RUNS
            },
        );
        let native_surface =
            selection.surface == crate::profile_backend::ProfileSurfaceBackend::NativeRasterCpu;
        telemetry.apply_selection(selection);

        let text_executor = match actual_text {
            TextSdfExecutor::LegacySkia => None,
            TextSdfExecutor::Simd => Some(FullCardSdfCandidateExecutor::SimdF32),
            TextSdfExecutor::ScalarOracle => Some(FullCardSdfCandidateExecutor::ScalarF32),
        };
        let shape_executor = match actual_shape {
            ShapeSdfExecutor::Skia => None,
            ShapeSdfExecutor::Simd => Some(FullCardSdfCandidateExecutor::SimdF32),
            ShapeSdfExecutor::ScalarOracle => Some(FullCardSdfCandidateExecutor::ScalarF32),
            ShapeSdfExecutor::Auto => unreachable!("Auto is resolved to a concrete executor"),
        };

        // A direct single-page request does not arrive with a compiled scene.
        // Resolve it once here when sparse fallback preparation is enabled, so
        // the demand scan and the renderer consume the same semantic payload.
        let owned_scene = if prepare_fallback_glyphs
            && text_executor.is_some()
            && self.profile_fallback_sdf_cache.is_some()
            && pre_resolved_scene.is_none()
        {
            let md = self.snapshot();
            Some(
                crate::semantic_resolve::resolve_card_commands_with_resources(
                    card,
                    &md,
                    "native:ordered-profile-backend",
                    profile,
                    "cn",
                    crate::semantic_resolve::ResolveResourceContext {
                        assets: self.assets.as_deref(),
                        render_objects: render_object_store,
                        catalog_lookup_ns: None,
                    },
                )
                .map_err(|error| ProfileBackendRenderError::Render(error.to_string()))?,
            )
        } else {
            None
        };
        let effective_scene = pre_resolved_scene.or(owned_scene.as_ref());
        let fallback_glyph_cache = if prepare_fallback_glyphs && text_executor.is_some() {
            if let Some(scene) = effective_scene {
                let md = self.snapshot();
                self.ensure_profile_fallback_for_scenes(std::iter::once(scene), &md)
                    .map_err(ProfileBackendRenderError::Render)?
            } else {
                None
            }
        } else {
            None
        };

        let encoded = if text_executor.is_some() || shape_executor.is_some() {
            match self.render_full_card_sdf_candidate_with_scene(
                card,
                profile,
                text_executor,
                shape_executor,
                config.tile_width,
                config.tile_height,
                config.pixel_occlusion_dry_run,
                config.pixel_occlusion_execute,
                [255, 255, 255, 255],
                native_surface,
                effective_scene,
                render_object_store,
            ) {
                Ok(mut output) => {
                    telemetry.surface_identity = crate::profile_backend::ProfileSurfaceIdentity {
                        pixel_format: "rgba8888".into(),
                        alpha_type: "premultiplied".into(),
                        color_space: "none".into(),
                    };
                    let sdf_atlases = self.sdf_atlases.load_full();
                    record_profile_atlas_identities(
                        &mut telemetry,
                        text_executor.is_some().then_some(sdf_atlases.as_ref()),
                        shape_executor
                            .is_some()
                            .then_some(self.shape_sdf_atlas.as_deref())
                            .flatten(),
                        "executed",
                    );
                    telemetry.atlas_contract = Some(
                        match (text_executor.is_some(), shape_executor.is_some()) {
                            (true, true) => crate::sdf::tile::MIXED_SDF_ATLAS_CONTRACT,
                            (true, false) => crate::sdf::atlas::ATLAS_SET_CONTRACT,
                            (false, true) => crate::sdf::shape_atlas::SHAPE_ATLAS_MANIFEST_SCHEMA,
                            (false, false) => unreachable!("candidate branch requires SDF work"),
                        }
                        .into(),
                    );
                    telemetry.work.page_count = 1;
                    telemetry.work.element_run_count =
                        output.sdf_run_count.saturating_add(output.legacy_run_count);
                    telemetry.bytes.atlas_mapped_bytes = output.atlas_mapped_bytes;
                    telemetry.bytes.render_object_mapped_bytes = output.render_object_mapped_bytes;
                    telemetry.bytes.readback_bytes = output.rgba.len() as u64;
                    telemetry.bytes.encoder_input_bytes = output.rgba.len() as u64;
                    telemetry.timings.sdf_capture_ns = output.timings.capture_ns;
                    telemetry.timings.semantic_resolve_ns = output.timings.semantic_resolve_ns;
                    telemetry.timings.sdf_capture_rich_parse_ns =
                        output.timings.capture_rich_parse_ns;
                    telemetry.timings.sdf_capture_font_resolve_ns =
                        output.timings.capture_font_resolve_ns;
                    telemetry.timings.sdf_capture_layout_setup_ns =
                        output.timings.capture_layout_setup_ns;
                    telemetry.timings.sdf_capture_measure_ns = output.timings.capture_measure_ns;
                    telemetry.timings.sdf_capture_command_build_ns =
                        output.timings.capture_command_build_ns;
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
                    telemetry.timings.legacy_element_draw_ns = output.timings.legacy_draw_ns;
                    telemetry.timings.image_composite_ns = output.timings.image_composite_ns;
                    telemetry.timings.general_base_composite_ns =
                        output.timings.general_base_composite_ns;
                    telemetry.timings.occlusion_mask_build_ns =
                        output.timings.occlusion_mask_build_ns;
                    telemetry.timings.occlusion_intersection_ns =
                        output.timings.occlusion_intersection_ns;
                    telemetry.timings.image_draw_ns = output.timings.image_composite_ns;
                    telemetry.timings.rgba_snapshot_ns = output.timings.rgba_snapshot_ns;
                    telemetry.timings.readback_ns = output.timings.rgba_snapshot_ns;
                    telemetry.record_executed_sdf_commands(
                        output.sdf_text_element_count,
                        output.sdf_shape_element_count,
                        output.captured_text_count,
                    );
                    telemetry.record_sdf_plan(
                        output.plan_stats,
                        output.plan_bytes,
                        output.span_bytes,
                    );
                    telemetry.record_sdf_execution(output.execution_stats);
                    telemetry.work.occlusion_eligible_image_count =
                        output.occlusion_eligible_image_count;
                    telemetry.work.software_skipped_text_command_count =
                        output.software_skipped_text_command_count;
                    telemetry.work.software_skipped_shape_command_count =
                        output.software_skipped_shape_command_count;
                    telemetry.work.general_base_hit_count = output.general_base_hit_count;
                    telemetry.work.general_base_miss_count = output.general_base_miss_count;
                    telemetry.work.general_base_baked_command_count =
                        output.general_base_baked_command_count;
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
                    telemetry.work.occlusion_mask_snapshot_count =
                        output.occlusion_mask_snapshot_count;
                    telemetry.work.occluded_fragment_count =
                        output.occlusion.occluded_fragment_count;
                    telemetry.work.visible_fragment_count = output.occlusion.visible_fragment_count;
                    telemetry.work.occluded_text_fragment_count =
                        output.occlusion.occluded_text_fragment_count;
                    telemetry.work.occluded_shape_fragment_count =
                        output.occlusion.occluded_shape_fragment_count;
                    telemetry.work.fully_occluded_sdf_command_count =
                        output.occlusion.fully_occluded_command_count;
                    telemetry.bytes.occlusion_mask_bytes = output.occlusion_mask_bytes;
                    telemetry.record_legacy_commands(
                        crate::profile_backend::ProfileCommandKind::Text,
                        output.legacy_text_count,
                        0,
                    );
                    telemetry.record_legacy_commands(
                        crate::profile_backend::ProfileCommandKind::Shape,
                        output.legacy_shape_count,
                        0,
                    );
                    telemetry.record_legacy_commands(
                        crate::profile_backend::ProfileCommandKind::Image,
                        output.legacy_image_count,
                        output.timings.legacy_draw_ns,
                    );
                    telemetry.record_legacy_commands(
                        crate::profile_backend::ProfileCommandKind::Image,
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
                    if let Some(image) = telemetry.commands.iter_mut().find(|command| {
                        command.kind == crate::profile_backend::ProfileCommandKind::Image
                    }) {
                        image.covered_fragments = image
                            .covered_fragments
                            .saturating_add(output.image_fragment_count);
                        image.blended_fragments = image
                            .blended_fragments
                            .saturating_add(output.image_fragment_count);
                    }

                    let encode_started = std::time::Instant::now();
                    let encoded: Result<Vec<u8>, String> = match config.jpeg_encoder {
                        crate::profile_backend::ProfileJpegEncoder::Skia => {
                            Err("the Skia JPEG encoder was retired; use libjpeg-turbo".to_string())
                        }
                        #[cfg(feature = "jpeg-turbo")]
                        crate::profile_backend::ProfileJpegEncoder::LibJpegTurbo => {
                            crate::jpeg_turbo::encode_rgba(
                                &output.rgba,
                                output.width,
                                output.height,
                                90,
                            )
                        }
                        #[cfg(feature = "jpeg-turbo")]
                        crate::profile_backend::ProfileJpegEncoder::LibJpegTurboAvx512Yuv420 => {
                            let mut scratch = self.take_jpeg_yuv420_scratch();
                            let encoded = crate::jpeg_turbo::encode_rgba_avx512_yuv420_with_scratch(
                                &output.rgba,
                                output.width,
                                output.height,
                                90,
                                &mut scratch,
                            );
                            self.recycle_jpeg_yuv420_scratch(scratch);
                            encoded
                        }
                        #[cfg(not(feature = "jpeg-turbo"))]
                        crate::profile_backend::ProfileJpegEncoder::LibJpegTurbo
                        | crate::profile_backend::ProfileJpegEncoder::LibJpegTurboAvx512Yuv420 => {
                            Err("libjpeg-turbo support is not compiled in".to_string())
                        }
                    };
                    telemetry.timings.encode_ns = elapsed_ns(encode_started);
                    let rgba_len = output.rgba.len();
                    self.recycle_profile_rgba_scratch(std::mem::take(&mut output.rgba));
                    match encoded {
                        Ok(encoded) => {
                            telemetry.bytes.encoded_output_bytes = encoded.len() as u64;
                            telemetry.bytes.scratch_peak_bytes =
                                (rgba_len as u64).saturating_add(encoded.len() as u64);
                            encoded
                        }

                        Err(error) => return Err(ProfileBackendRenderError::Render(error)),
                    }
                }
                Err(error) if error.contains("profile compositor is missing render object") => {
                    return Err(ProfileBackendRenderError::Render(error));
                }

                Err(error) => return Err(ProfileBackendRenderError::Render(error)),
            }
        } else {
            return Err(ProfileBackendRenderError::Render(
                "the legacy page renderer was retired; select SDF executors".into(),
            ));
        };
        telemetry.timings.total_ns = elapsed_ns(total_started);
        telemetry.fallback_glyph_cache = fallback_glyph_cache;
        Ok(crate::profile_backend::ProfileBackendRenderOutput { encoded, telemetry })
    }

    pub fn render_profile_batch_with_backend<'a>(
        &self,
        batch_key: &str,
        locale: &str,
        pages: impl IntoIterator<Item = (i32, &'a CustomProfileCard)>,
        profile: Option<&crate::profile::ProfileData>,
        config: crate::profile_backend::ProfileBackendConfig,
    ) -> Result<
        crate::profile_backend::ProfileBackendBatchRenderOutput,
        crate::profile_backend::ProfileBackendRenderError,
    > {
        let generation = self.pin_render_object_generation();
        self.render_profile_batch_with_backend_generation(
            batch_key,
            locale,
            pages,
            profile,
            config,
            &generation,
        )
    }

    pub fn render_profile_batch_with_backend_generation<'a>(
        &self,
        batch_key: &str,
        locale: &str,
        pages: impl IntoIterator<Item = (i32, &'a CustomProfileCard)>,
        profile: Option<&crate::profile::ProfileData>,
        config: crate::profile_backend::ProfileBackendConfig,
        generation: &RenderObjectGenerationPin,
    ) -> Result<
        crate::profile_backend::ProfileBackendBatchRenderOutput,
        crate::profile_backend::ProfileBackendRenderError,
    > {
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
        .map_err(|error| {
            crate::profile_backend::ProfileBackendRenderError::Render(error.to_string())
        })?;
        let selection = config.resolve(self.profile_backend_capabilities())?;
        let prepared_render_objects = if selection.text_sdf
            != crate::profile_backend::TextSdfExecutor::LegacySkia
            || selection.shape_sdf != crate::profile_backend::ShapeSdfExecutor::Skia
        {
            let store = generation.store().ok_or_else(|| {
                crate::profile_backend::ProfileBackendRenderError::Render(
                    "compiled profile backend requires a render-object store".into(),
                )
            })?;
            let prepared = compiled.prepare_render_objects(store);
            if !prepared.missing_object_keys.is_empty() {
                tracing::warn!(
                    stage = "render_object_miss",
                    generation = generation.source_identity().unwrap_or("none"),
                    missing_count = prepared.missing_object_keys.len(),
                    missing_keys = ?prepared.missing_object_keys.iter().take(16).collect::<Vec<_>>(),
                    "render-object generation is incomplete; resource pipeline must publish a complete replacement"
                );
            }
            Some(prepared)
        } else {
            None
        };
        let missing_page_object_keys = prepared_render_objects
            .as_ref()
            .map(|prepared| prepared.missing_page_object_keys.clone())
            .unwrap_or_default();
        let fallback_glyph_cache =
            if selection.text_sdf != crate::profile_backend::TextSdfExecutor::LegacySkia {
                self.ensure_profile_fallback_for_scenes(
                    compiled.pages.iter().map(|page| &page.scene),
                    &md,
                )
                .map_err(crate::profile_backend::ProfileBackendRenderError::Render)?
            } else {
                None
            };
        let mut rendered = Vec::with_capacity(compiled.pages.len());
        for page in &compiled.pages {
            let card = cards_by_seq.get(&page.seq).copied().ok_or_else(|| {
                crate::profile_backend::ProfileBackendRenderError::Render(format!(
                    "compiled batch lost page seq={}",
                    page.seq
                ))
            })?;
            let missing_for_page = missing_page_object_keys
                .get(&page.seq)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mut output = self.render_page_with_backend_scene(
                card,
                profile,
                config.clone(),
                Some(&page.scene),
                false,
                generation.store(),
            )?;
            if !missing_for_page.is_empty() {
                output.telemetry.record_fallback(
                    crate::profile_backend::BackendFallbackCode::RenderObjectMissing,
                    "render-object-source-cos",
                    format!(
                        "page seq={} loaded {} source object(s) from AssetStore/COS: {}",
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
        Ok(crate::profile_backend::ProfileBackendBatchRenderOutput {
            compiled,
            prepared_render_objects,
            pages: rendered,
            fallback_glyph_cache,
        })
    }

    /// Builds the v0.2 text scene dump without changing or replacing the production pixel path.
    pub fn dump_text_scene(
        &self,
        card: &CustomProfileCard,
        document_key: &str,
        tick: u64,
    ) -> Result<allium_renderer_core::SceneDump, String> {
        let started = std::time::Instant::now();
        let md = self.snapshot();
        let mut scene = crate::core_shadow::build_text_scene(card, &md, document_key)
            .map_err(|error| error.to_string())?;
        scene.advance_to_tick(tick);
        let dump = scene.dump();
        tracing::debug!(
            document_key,
            tick,
            layers = dump.layers.len(),
            dynamic_layers = dump
                .layers
                .iter()
                .filter(|layer| layer.dynamic.is_some())
                .count(),
            dynamic_evaluations = dump.telemetry.dynamic_evaluations,
            elapsed_us = started.elapsed().as_micros(),
            "renderer core native text shadow dump"
        );
        Ok(dump)
    }

    /// Builds the v0.2 full authored-layer semantic scene without changing the pixel path.
    pub fn dump_semantic_scene(
        &self,
        card: &CustomProfileCard,
        document_key: &str,
        region: &str,
        tick: u64,
    ) -> Result<allium_renderer_core::SceneDump, String> {
        self.dump_semantic_scene_with_profile(card, None, document_key, region, region, tick)
    }

    pub fn dump_semantic_scene_with_profile(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        document_key: &str,
        region: &str,
        locale: &str,
        tick: u64,
    ) -> Result<allium_renderer_core::SceneDump, String> {
        let md = self.snapshot();
        let mut scene = crate::core_shadow::build_scene(
            card,
            &md,
            document_key,
            region,
            profile,
            locale,
            self.assets.as_deref(),
        )?;
        scene.advance_to_tick(tick);
        Ok(scene.dump())
    }

    pub(crate) fn prepare_realtime_edt_batch(
        &self,
        cards: &[CustomProfileCard],
        md: &MasterData,
        assets: Option<&AssetStore>,
        profile: Option<&crate::profile::ProfileData>,
    ) -> Result<Arc<RealtimeEdtPreparedBatch>, String> {
        let text_atlases = self.sdf_atlases.load_full();
        let source = crate::sdf::tile::MixedSdfAtlasSource::new(
            &text_atlases,
            self.shape_sdf_atlas.as_deref(),
        )
        .map_err(|error| error.to_string())?;
        let mut primitives = Vec::new();
        for card in cards {
            let captured = capture_sdf_primitives(
                card,
                md,
                assets,
                Some(&text_atlases),
                profile,
                crate::transform::CANVAS_WIDTH as u32,
                crate::transform::CANVAS_HEIGHT as u32,
                SdfCaptureKinds::TEXT,
            )?;
            primitives.extend(captured.primitives);
        }
        let mut pages = Vec::new();
        let mut entries = std::collections::BTreeMap::new();
        let mut glyphs = Vec::new();
        let mut telemetry = crate::profile_backend::RealtimeEdtBatchTelemetry::default();
        let runtime_pages = self
            .realtime_oversized_glyph_generation
            .then_some(&mut pages);
        let _ = map_captured_sdf_commands(
            &primitives,
            &text_atlases,
            self.shape_sdf_atlas.as_deref(),
            &source,
            runtime_pages,
            Some(&mut glyphs),
            Some(&mut telemetry),
            Some(&mut entries),
        )?;
        Ok(Arc::new(RealtimeEdtPreparedBatch {
            pages,
            entries,
            glyphs,
            telemetry,
        }))
    }

    pub(crate) fn with_realtime_edt_batch<T>(
        &self,
        batch: Arc<RealtimeEdtPreparedBatch>,
        run: impl FnOnce() -> T,
    ) -> T {
        struct Restore(Option<Arc<RealtimeEdtPreparedBatch>>);
        impl Drop for Restore {
            fn drop(&mut self) {
                let previous = self.0.take();
                ACTIVE_REALTIME_EDT_BATCH.with(|slot| *slot.borrow_mut() = previous);
            }
        }

        let previous = ACTIVE_REALTIME_EDT_BATCH.with(|slot| slot.borrow_mut().replace(batch));
        let _restore = Restore(previous);
        run()
    }
}

#[cfg(feature = "animation-export")]
#[derive(Default)]
struct AnimationCanvasExpansion {
    left: i32,
    right: i32,
    top: i32,
    bottom: i32,
}

#[cfg(feature = "animation-export")]
fn animation_canvas_expansion(
    elements: &[crate::elements::RenderElement<'_>],
    md: &MasterData,
) -> AnimationCanvasExpansion {
    const PAD: i32 = 8;
    let Some(text) = elements.iter().find_map(|element| match element {
        crate::elements::RenderElement::Text(text) if text.object_data.visible => Some(*text),
        _ => None,
    }) else {
        return AnimationCanvasExpansion::default();
    };
    let Some(animation) = crate::text::line_indent_x_animation(text, md) else {
        return AnimationCanvasExpansion::default();
    };
    let (_, _, angle_deg, scale_x, _) = crate::transform::extract_transform(&text.object_data);
    let radians = angle_deg.to_radians();
    let cos = radians.cos();
    let sin = radians.sin();
    let mut min_dx = 0.0_f32;
    let mut max_dx = 0.0_f32;
    let mut min_dy = 0.0_f32;
    let mut max_dy = 0.0_f32;
    for frame in animation.frames {
        let local_x = frame.dx_local * scale_x;
        let dx = local_x * cos;
        let dy = local_x * sin;
        min_dx = min_dx.min(dx);
        max_dx = max_dx.max(dx);
        min_dy = min_dy.min(dy);
        max_dy = max_dy.max(dy);
    }
    AnimationCanvasExpansion {
        left: max_dx.max(0.0).ceil() as i32 + PAD,
        right: (-min_dx).max(0.0).ceil() as i32 + PAD,
        top: max_dy.max(0.0).ceil() as i32 + PAD,
        bottom: (-min_dy).max(0.0).ceil() as i32 + PAD,
    }
}

/// 扫描像素缓冲区，找到所有 alpha > 0 像素的最小包围矩形。
fn find_opaque_bounds(
    pixels: &[u8],
    width: u32,
    height: u32,
    row_bytes: usize,
) -> (u32, u32, u32, u32) {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
        && std::arch::is_x86_feature_detected!("bmi2")
    {
        // SAFETY: the feature checks above satisfy the implementation's
        // target-feature contract. Callers already provide a complete RGBA8
        // surface, exactly as required by the scalar implementation.
        return unsafe { find_opaque_bounds_avx512(pixels, width, height, row_bytes) };
    }

    find_opaque_bounds_scalar(pixels, width, height, row_bytes)
}

// ─────────────────────────────────────────────────────────────────────────
// 批量分层裁剪渲染：把名片拆成「每个可见元素一张裁剪 WebP」的统一原语。
//
// 当前实现循环调 `render_element_layer_cropped`，每层输出与单方法逐字节一致。
// 后续可在不破坏逐字节一致性的前提下做内部优化（单画布复用等）。
// ─────────────────────────────────────────────────────────────────────────

/// Runtime probe for the Turin AVX-512 SDF tile executor.
fn turin_sdf_simd_available() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
            && std::arch::is_x86_feature_detected!("avx512vbmi")
            && std::arch::is_x86_feature_detected!("fma")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

pub struct ShapeSdfExecutionOutput {
    /// Premultiplied row-major RGBA8.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub plan_stats: crate::sdf::tile::SdfPlanStats,
    pub execution_stats: crate::sdf::tile::SdfExecutionStats,
    pub atlas_mapped_bytes: u64,
    pub plan_bytes: u64,
    pub span_bytes: u64,
}

pub type ShapeSdfScalarOracleOutput = ShapeSdfExecutionOutput;

/// Timings for the SDF-only layer candidate. Capture runs through the same
/// ordered element draw path as the legacy renderer; mapping, binning and
/// execution are reported separately so a server benchmark can distinguish
/// layout/capture cost from the Turin executor itself.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SdfLayerExecutionTimings {
    pub capture_ns: u64,
    pub command_mapping_ns: u64,
    pub plan_build_ns: u64,
    pub execute_ns: u64,
    pub total_ns: u64,
}

/// Ordered Text+Shape SDF layer output. This deliberately excludes images and
/// other Skia-only elements; it is a numerical/performance comparison surface
/// for the shared SDF backend, not a complete card renderer.
pub struct SdfLayerExecutionOutput {
    /// Premultiplied row-major RGBA8, quantized once after FP32 tile blending.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub captured_text_count: u64,
    pub captured_shape_count: u64,
    pub plan_stats: crate::sdf::tile::SdfPlanStats,
    pub execution_stats: crate::sdf::tile::SdfExecutionStats,
    pub atlas_mapped_bytes: u64,
    pub plan_bytes: u64,
    pub span_bytes: u64,
    pub timings: SdfLayerExecutionTimings,
}

/// Full-card candidate output with ordered SDF runs interleaved with the
/// existing Skia-only element path. The pixels remain premultiplied RGBA8 so
/// callers can compare the compositor result before any image encoding.
#[derive(Default)]
pub struct FullCardSdfExecutionOutput {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub clear_rgba: [u8; 4],
    pub sdf_run_count: u64,
    pub legacy_run_count: u64,
    pub legacy_element_count: u64,
    pub legacy_text_count: u64,
    pub legacy_shape_count: u64,
    pub legacy_image_count: u64,
    pub software_image_count: u64,
    pub software_text_command_count: u64,
    pub software_shape_command_count: u64,
    pub software_skipped_text_command_count: u64,
    pub software_skipped_shape_command_count: u64,
    pub image_fragment_count: u64,
    pub image_simd_packet_count: u64,
    pub general_base_hit_count: u64,
    pub general_base_miss_count: u64,
    pub general_base_baked_command_count: u64,
    pub general_base_overlay_command_count: u64,
    pub general_base_bytes: u64,
    pub general_base_avoided_source_bytes: u64,
    pub deck_art_variant_hit_count: u64,
    pub deck_art_variant_miss_count: u64,
    pub deck_art_variant_bytes: u64,
    pub deck_art_variant_avoided_source_bytes: u64,
    pub render_object_mapped_bytes: u64,
    pub sdf_text_element_count: u64,
    pub sdf_shape_element_count: u64,
    pub captured_text_count: u64,
    pub captured_shape_count: u64,
    pub realtime_edt_glyphs: Vec<crate::profile_backend::RealtimeEdtGlyphTelemetry>,
    pub realtime_edt_batch: crate::profile_backend::RealtimeEdtBatchTelemetry,
    pub plan_stats: crate::sdf::tile::SdfPlanStats,
    pub execution_stats: crate::sdf::tile::SdfExecutionStats,
    pub atlas_mapped_bytes: u64,
    pub plan_bytes: u64,
    pub span_bytes: u64,
    pub occlusion: crate::sdf::tile::SdfOcclusionStats,
    pub occlusion_eligible_image_count: u64,
    pub occlusion_mask_snapshot_count: u64,
    pub occlusion_mask_bytes: u64,
    pub timings: FullCardSdfExecutionTimings,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize)]
pub struct ProfileBackendPrewarmReport {
    pub shape_source_count: u64,
    pub shape_decoded_bytes: u64,
    pub render_object_count: u64,
    pub render_object_bytes: u64,
    pub render_object_page_touch_count: u64,
    pub render_object_checksum: u64,
    pub text_atlas_count: u64,
    pub text_atlas_page_count: u64,
    pub text_atlas_mapped_bytes: u64,
    pub text_atlas_page_touch_count: u64,
    pub text_atlas_checksum: u64,
    pub font_family_count: u64,
    pub font_resolve_ns: u64,
    pub shape_atlas_count: u64,
    pub shape_atlas_page_count: u64,
    pub shape_atlas_mapped_bytes: u64,
    pub shape_atlas_page_touch_count: u64,
    pub shape_atlas_checksum: u64,
    pub shape_row_program_count: u64,
    pub shape_row_program_run_count: u64,
    pub shape_row_program_bytes: u64,
    pub shape_row_program_build_ns: u64,
    pub profile_surface_scratch_bytes: u64,
    pub profile_surface_page_touch_count: u64,
    pub profile_surface_init_ns: u64,
    pub jpeg_encoder_prewarm_bytes: u64,
    pub jpeg_yuv420_prewarm_bytes: u64,
    pub jpeg_yuv420_scratch_bytes: u64,
    pub jpeg_yuv420_scratch_page_touch_count: u64,
    pub elapsed_ns: u64,
}

#[derive(Clone, Copy, Debug)]
struct OrderedSdfSurfaceSpec {
    surface_width: u32,
    surface_height: u32,
    canvas_width: u32,
    canvas_height: u32,
    canvas_origin_x: f32,
    canvas_origin_y: f32,
    clear_rgba: [u8; 4],
    prepare_direct_axis_shape: bool,
    /// Native-surface discipline: an element that cannot take the software
    /// image path fails the request instead of drawing through the legacy
    /// element renderer.
    forbid_legacy_elements: bool,
}

impl OrderedSdfSurfaceSpec {
    fn full_card(clear_rgba: [u8; 4]) -> Self {
        Self {
            surface_width: crate::transform::CANVAS_WIDTH as u32,
            surface_height: crate::transform::CANVAS_HEIGHT as u32,
            canvas_width: crate::transform::CANVAS_WIDTH as u32,
            canvas_height: crate::transform::CANVAS_HEIGHT as u32,
            canvas_origin_x: 0.0,
            canvas_origin_y: 0.0,
            clear_rgba,
            prepare_direct_axis_shape: true,
            forbid_legacy_elements: false,
        }
    }

    fn with_forbid_legacy_elements(mut self, forbid: bool) -> Self {
        self.forbid_legacy_elements = forbid;
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct FullCardSdfExecutionTimings {
    #[serde(default)]
    pub semantic_resolve_ns: u64,
    pub surface_create_ns: u64,
    pub surface_clear_ns: u64,
    pub capture_ns: u64,
    pub capture_rich_parse_ns: u64,
    pub capture_font_resolve_ns: u64,
    pub capture_layout_setup_ns: u64,
    pub capture_measure_ns: u64,
    pub capture_command_build_ns: u64,
    pub capture_emit_ns: u64,
    pub command_mapping_ns: u64,
    pub plan_build_ns: u64,
    pub execute_ns: u64,
    pub legacy_draw_ns: u64,
    pub image_composite_ns: u64,
    pub general_base_composite_ns: u64,
    pub occlusion_mask_build_ns: u64,
    pub occlusion_intersection_ns: u64,
    pub rgba_snapshot_ns: u64,
    /// Dynamic-layer post-processing: scan the executor-owned RGBA buffer for
    /// its non-transparent bounds without a Skia readback.
    #[serde(default)]
    pub postprocess_bounds_ns: u64,
    /// Dynamic-layer post-processing: copy the tight RGBA rows and construct
    /// the final cropped raster image.
    #[serde(default)]
    pub postprocess_crop_ns: u64,
    pub total_ns: u64,
}

#[derive(Clone, Copy)]
enum ShapeSdfCandidateExecutor {
    ScalarRgba8,
    SimdF32,
}

#[derive(Clone, Copy)]
pub(crate) enum SdfLayerCandidateExecutor {
    ScalarF32,
    SimdF32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FullCardSdfCandidateExecutor {
    ScalarF32,
    SimdF32,
}

fn record_profile_atlas_identities(
    telemetry: &mut crate::profile_backend::ProfileRenderTelemetry,
    text_atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
    shape_atlas: Option<&crate::sdf::shape_atlas::MappedShapeSdfAtlas>,
    usage: &str,
) {
    telemetry.atlas_identities.clear();
    if let Some(text_atlases) = text_atlases {
        telemetry
            .atlas_identities
            .extend(text_atlases.iter().map(|atlas| {
                let manifest = atlas.manifest();
                crate::profile_backend::ProfileAtlasIdentity {
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
            .push(crate::profile_backend::ProfileAtlasIdentity {
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

#[derive(Clone, Copy)]
#[allow(dead_code)]
struct SdfShadowPlanConfig<'a> {
    text_atlases: &'a crate::sdf::atlas::MappedSdfAtlasSet,
    shape_atlas: Option<&'a crate::sdf::shape_atlas::MappedShapeSdfAtlas>,
    tile_width: u16,
    tile_height: u16,
}

enum CapturedSdfPrimitive {
    Text(crate::text::ResolvedTextSdfGlyph),
    Shape(crate::elements::shape::ResolvedShapeSdfCommand),
}

#[derive(Clone, Copy)]
struct SdfCaptureKinds {
    text: bool,
    shape: bool,
}

impl SdfCaptureKinds {
    const TEXT: Self = Self {
        text: true,
        shape: false,
    };

    const SHAPE: Self = Self {
        text: false,
        shape: true,
    };

    const TEXT_AND_SHAPE: Self = Self {
        text: true,
        shape: true,
    };
}

struct CapturedSdfPrimitives {
    primitives: Vec<CapturedSdfPrimitive>,
    text_count: u64,
    shape_count: u64,
}

#[allow(clippy::too_many_arguments)]
fn capture_sdf_primitives(
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

fn captured_text_sdf_glyph_is_invisible(glyph: &crate::text::ResolvedTextSdfGlyph) -> bool {
    glyph.font_size.is_finite() && glyph.font_size <= 0.0
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RealtimeEdtRequestKey {
    font_family: String,
    codepoint: u32,
    point_size_bits: u32,
    spread_bits: u32,
}

#[derive(Clone)]
struct RealtimeEdtPreparedEntry {
    atlas_set: u16,
    manifest: crate::sdf::atlas::SdfAtlasGlyphManifest,
    point_size: f32,
    spread: f32,
}

pub(crate) struct RealtimeEdtPreparedBatch {
    pages: Vec<crate::sdf::tile::RuntimeTextSdfPage>,
    entries: std::collections::BTreeMap<RealtimeEdtRequestKey, RealtimeEdtPreparedEntry>,
    pub(crate) glyphs: Vec<crate::profile_backend::RealtimeEdtGlyphTelemetry>,
    pub(crate) telemetry: crate::profile_backend::RealtimeEdtBatchTelemetry,
}

thread_local! {
    static ACTIVE_REALTIME_EDT_BATCH: std::cell::RefCell<Option<Arc<RealtimeEdtPreparedBatch>>> =
        const { std::cell::RefCell::new(None) };
}

fn active_realtime_edt_batch() -> Option<Arc<RealtimeEdtPreparedBatch>> {
    ACTIVE_REALTIME_EDT_BATCH.with(|slot| slot.borrow().clone())
}

fn realtime_edt_request_key(
    font_family: &str,
    codepoint: char,
    point_size: f32,
    spread: f32,
) -> RealtimeEdtRequestKey {
    RealtimeEdtRequestKey {
        font_family: font_family.to_string(),
        codepoint: u32::from(codepoint),
        point_size_bits: point_size.to_bits(),
        spread_bits: spread.to_bits(),
    }
}

#[derive(Clone, Copy)]
struct TextSdfSamplingGrid {
    command: crate::sdf::tile::SdfDrawCommand,
    point_size: f32,
    left_center: f32,
    top_center: f32,
}

impl TextSdfSamplingGrid {
    fn new(
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

fn align_substituted_text_sdf_command(
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

fn map_captured_sdf_commands(
    captured: &[CapturedSdfPrimitive],
    text_atlases: &crate::sdf::atlas::MappedSdfAtlasSet,
    shape_atlas: Option<&crate::sdf::shape_atlas::MappedShapeSdfAtlas>,
    source: &crate::sdf::tile::MixedSdfAtlasSource<'_>,
    mut runtime_text_pages: Option<&mut Vec<crate::sdf::tile::RuntimeTextSdfPage>>,
    mut realtime_edt_telemetry: Option<&mut Vec<crate::profile_backend::RealtimeEdtGlyphTelemetry>>,
    mut realtime_edt_batch_telemetry: Option<
        &mut crate::profile_backend::RealtimeEdtBatchTelemetry,
    >,
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

    let mut batch = crate::profile_backend::RealtimeEdtBatchTelemetry::default();
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
    let pages = runtime_text_pages
        .as_deref_mut()
        .expect("runtime pages were checked before collecting requests");
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
    if let Some(records) = realtime_edt_telemetry.as_deref_mut() {
        for (request_index, runtime) in runtime_requests.iter().enumerate() {
            let Some(runtime) = runtime else {
                continue;
            };
            let request = &requests[request_index];
            records.push(crate::profile_backend::RealtimeEdtGlyphTelemetry {
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
    if let Some(report) = realtime_edt_batch_telemetry.as_deref_mut() {
        report.accumulate(&batch);
    }
    Ok(commands)
}

#[cfg(feature = "parallel")]
fn realtime_edt_pool() -> &'static rayon::ThreadPool {
    use std::sync::OnceLock;
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::env::var("SCAPUS_REALTIME_EDT_THREADS")
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|threads| (1..=4).contains(threads))
            .unwrap_or(2);
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|index| format!("realtime-edt-{index}"))
            .build()
            .expect("realtime EDT thread pool creation failed")
    })
}

fn sdf_command_device_magnification(command: &crate::sdf::tile::SdfDrawCommand) -> Option<f32> {
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
    telemetry: &mut crate::profile_backend::RealtimeEdtBatchTelemetry,
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

fn realtime_edt_sampling_spread(
    atlas_point_size: f32,
    atlas_spread: f32,
    target_point_size: f32,
) -> Option<f32> {
    if !atlas_point_size.is_finite()
        || !atlas_spread.is_finite()
        || !target_point_size.is_finite()
        || atlas_point_size <= 0.0
        || atlas_spread <= 0.0
        || target_point_size < atlas_point_size
    {
        return None;
    }
    let spread = atlas_spread * (target_point_size / atlas_point_size);
    (spread.is_finite() && spread > 0.0).then_some(spread)
}

struct PixelOcclusionDryRun {
    masks_by_element: Vec<Option<Arc<crate::sdf::tile::PixelOcclusionMask>>>,
    eligible_image_count: u64,
    mask_snapshot_count: u64,
    mask_bytes: u64,
    build_ns: u64,
}

#[allow(clippy::too_many_arguments)]
fn build_pixel_occlusion_dry_run(
    card: &CustomProfileCard,
    elements: &[crate::elements::RenderElement<'_>],
    scene: &allium_renderer_core::profile_scene::ResolvedProfileScene,
    store: &crate::render_object::MappedRenderObjectStore,
    text_sdf: bool,
    shape_sdf: bool,
    width: u32,
    height: u32,
) -> Result<PixelOcclusionDryRun, String> {
    let started = std::time::Instant::now();
    let pixel_bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|count| count.checked_mul(4))
        .ok_or_else(|| "pixel occlusion scratch size overflow".to_string())?;
    let mut masks_by_element = vec![None; elements.len()];
    let mut opaque_above = Arc::new(
        crate::sdf::tile::PixelOcclusionMask::new(width, height)
            .map_err(|error| error.to_string())?,
    );
    let mut scratch = vec![0u8; pixel_bytes];
    let mut eligible_image_count = 0u64;

    for (element_index, element) in elements.iter().enumerate().rev() {
        if !element.visible() {
            continue;
        }
        let is_sdf = matches!(element, crate::elements::RenderElement::Text(_)) && text_sdf
            || matches!(element, crate::elements::RenderElement::Shape(_)) && shape_sdf;
        if is_sdf {
            masks_by_element[element_index] = Some(Arc::clone(&opaque_above));
            continue;
        }
        if matches!(
            element,
            crate::elements::RenderElement::Text(_) | crate::elements::RenderElement::Shape(_)
        ) {
            continue;
        }
        let Some((kind, index)) = render_element_authored_identity(card, element) else {
            continue;
        };
        if !crate::profile_compositor::authored_image_is_exact_opaque_mask_source(
            scene, kind, index,
        ) {
            continue;
        }
        scratch.fill(0);
        if crate::profile_compositor::render_authored_image_into_simd(
            scene,
            store,
            kind,
            index,
            &mut scratch,
            width,
            height,
        )
        .is_err()
        {
            continue;
        }
        eligible_image_count = eligible_image_count.saturating_add(1);
        if scratch.chunks_exact(4).any(|pixel| pixel[3] == u8::MAX) {
            Arc::make_mut(&mut opaque_above)
                .union_opaque_rgba8(&scratch)
                .map_err(|error| error.to_string())?;
        }
    }

    let mut unique_masks = std::collections::BTreeSet::new();
    let mut mask_bytes = 0u64;
    for mask in masks_by_element.iter().flatten() {
        if unique_masks.insert(Arc::as_ptr(mask) as usize) {
            mask_bytes = mask_bytes.saturating_add(mask.resident_bytes());
        }
    }
    Ok(PixelOcclusionDryRun {
        masks_by_element,
        eligible_image_count,
        mask_snapshot_count: unique_masks.len() as u64,
        mask_bytes,
        build_ns: elapsed_ns(started),
    })
}

#[derive(Default)]
struct FullCardSdfRunAggregate {
    prepare_direct_axis_shape: bool,
    realtime_oversized_glyph_generation: bool,
    sdf_run_count: u64,
    legacy_run_count: u64,
    legacy_element_count: u64,
    legacy_text_count: u64,
    legacy_shape_count: u64,
    legacy_image_count: u64,
    software_image_count: u64,
    software_text_command_count: u64,
    software_shape_command_count: u64,
    software_skipped_text_command_count: u64,
    software_skipped_shape_command_count: u64,
    image_fragment_count: u64,
    image_simd_packet_count: u64,
    general_base_hit_count: u64,
    general_base_miss_count: u64,
    general_base_baked_command_count: u64,
    general_base_overlay_command_count: u64,
    general_base_bytes: u64,
    general_base_avoided_source_bytes: u64,
    deck_art_variant_hit_count: u64,
    deck_art_variant_miss_count: u64,
    deck_art_variant_bytes: u64,
    deck_art_variant_avoided_source_bytes: u64,
    render_object_mapped_bytes: u64,
    sdf_text_element_count: u64,
    sdf_shape_element_count: u64,
    captured_text_count: u64,
    captured_shape_count: u64,
    realtime_edt_glyphs: Vec<crate::profile_backend::RealtimeEdtGlyphTelemetry>,
    realtime_edt_batch: crate::profile_backend::RealtimeEdtBatchTelemetry,
    plan_stats: crate::sdf::tile::SdfPlanStats,
    execution_stats: crate::sdf::tile::SdfExecutionStats,
    plan_bytes: u64,
    span_bytes: u64,
    occlusion: crate::sdf::tile::SdfOcclusionStats,
    occlusion_eligible_image_count: u64,
    occlusion_mask_snapshot_count: u64,
    occlusion_mask_bytes: u64,
    timings: FullCardSdfExecutionTimings,
}

/// Fills a premultiplied RGBA8 buffer with a clear colour, matching the
/// backend clear it replaces: the alpha endpoints — the only clear colours the
/// profile paths pass — premultiply exactly, and any other alpha follows the
/// engine's premultiply rounding.
fn fill_premultiplied_clear(pixels: &mut [u8], clear: [u8; 4]) {
    let alpha = clear[3];
    let premultiplied = [
        crate::codec::premultiply_channel(clear[0], alpha),
        crate::codec::premultiply_channel(clear[1], alpha),
        crate::codec::premultiply_channel(clear[2], alpha),
        alpha,
    ];
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.copy_from_slice(&premultiplied);
    }
}

fn select_full_card_sdf_executor(
    pixels: &mut [u8],
    pending: &mut Vec<CapturedSdfPrimitive>,
    active_executor: &mut Option<FullCardSdfCandidateExecutor>,
    active_occlusion_mask: &mut Option<Arc<crate::sdf::tile::PixelOcclusionMask>>,
    next_executor: FullCardSdfCandidateExecutor,
    next_occlusion_mask: Option<Arc<crate::sdf::tile::PixelOcclusionMask>>,
    execute_occlusion: bool,
    text_atlases: &crate::sdf::atlas::MappedSdfAtlasSet,
    shape_atlas: Option<&crate::sdf::shape_atlas::MappedShapeSdfAtlas>,
    source: &crate::sdf::tile::MixedSdfAtlasSource<'_>,
    grid: crate::sdf::tile::TileGrid,
    shape_program_cache: &crate::sdf::tile::ShapeRowProgramCache,
    aggregate: &mut FullCardSdfRunAggregate,
) -> Result<(), String> {
    if active_executor.is_some_and(|active| active != next_executor) {
        flush_active_full_card_sdf_run(
            pixels,
            pending,
            active_executor,
            active_occlusion_mask,
            execute_occlusion,
            text_atlases,
            shape_atlas,
            source,
            grid,
            shape_program_cache,
            aggregate,
        )?;
    }
    *active_executor = Some(next_executor);
    if active_occlusion_mask.is_none() {
        *active_occlusion_mask = next_occlusion_mask;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn flush_active_full_card_sdf_run(
    pixels: &mut [u8],
    pending: &mut Vec<CapturedSdfPrimitive>,
    active_executor: &mut Option<FullCardSdfCandidateExecutor>,
    active_occlusion_mask: &mut Option<Arc<crate::sdf::tile::PixelOcclusionMask>>,
    execute_occlusion: bool,
    text_atlases: &crate::sdf::atlas::MappedSdfAtlasSet,
    shape_atlas: Option<&crate::sdf::shape_atlas::MappedShapeSdfAtlas>,
    source: &crate::sdf::tile::MixedSdfAtlasSource<'_>,
    grid: crate::sdf::tile::TileGrid,
    shape_program_cache: &crate::sdf::tile::ShapeRowProgramCache,
    aggregate: &mut FullCardSdfRunAggregate,
) -> Result<(), String> {
    let Some(executor) = active_executor.take() else {
        debug_assert!(pending.is_empty());
        *active_occlusion_mask = None;
        return Ok(());
    };
    let occlusion_mask = active_occlusion_mask.take();
    if pending.is_empty() {
        return Ok(());
    }
    flush_full_card_sdf_run(
        pixels,
        pending,
        text_atlases,
        shape_atlas,
        source,
        grid,
        shape_program_cache,
        executor,
        occlusion_mask.as_deref(),
        execute_occlusion,
        aggregate,
    )
}

#[allow(clippy::too_many_arguments)]
fn flush_full_card_sdf_run(
    pixels: &mut [u8],
    pending: &mut Vec<CapturedSdfPrimitive>,
    text_atlases: &crate::sdf::atlas::MappedSdfAtlasSet,
    shape_atlas: Option<&crate::sdf::shape_atlas::MappedShapeSdfAtlas>,
    source: &crate::sdf::tile::MixedSdfAtlasSource<'_>,
    grid: crate::sdf::tile::TileGrid,
    shape_program_cache: &crate::sdf::tile::ShapeRowProgramCache,
    executor: FullCardSdfCandidateExecutor,
    occlusion_mask: Option<&crate::sdf::tile::PixelOcclusionMask>,
    execute_occlusion: bool,
    aggregate: &mut FullCardSdfRunAggregate,
) -> Result<(), String> {
    if pending.is_empty() {
        return Ok(());
    }

    let mapping_started = std::time::Instant::now();
    let mut runtime_text_pages = Vec::new();
    let prepared_batch = active_realtime_edt_batch();
    let runtime_pages = aggregate
        .realtime_oversized_glyph_generation
        .then_some(&mut runtime_text_pages);
    let commands = map_captured_sdf_commands(
        pending,
        text_atlases,
        shape_atlas,
        source,
        runtime_pages,
        Some(&mut aggregate.realtime_edt_glyphs),
        Some(&mut aggregate.realtime_edt_batch),
        None,
    )
    .map_err(|error| format!("full-card SDF command mapping: {error}"))?;
    let runtime_pages = prepared_batch
        .as_deref()
        .map(|batch| batch.pages.as_slice())
        .unwrap_or(runtime_text_pages.as_slice());
    let execution_source = crate::sdf::tile::MixedSdfAtlasSource::with_runtime_text(
        text_atlases,
        shape_atlas,
        runtime_pages,
    )
    .map_err(|error| format!("full-card SDF runtime source: {error}"))?;
    aggregate.timings.command_mapping_ns = aggregate
        .timings
        .command_mapping_ns
        .saturating_add(elapsed_ns(mapping_started));

    let plan_started = std::time::Instant::now();
    let plan = if aggregate.prepare_direct_axis_shape {
        crate::sdf::tile::SdfTilePlan::build_static_one_shot_with_shape_program_cache(
            grid,
            &commands,
            &execution_source,
            shape_program_cache,
        )
    } else {
        crate::sdf::tile::SdfTilePlan::build_for_one_shot_dynamic_layer(
            grid,
            &commands,
            &execution_source,
        )
    }
    .map_err(|error| format!("full-card SDF plan build: {error}"))?;
    aggregate.timings.plan_build_ns = aggregate
        .timings
        .plan_build_ns
        .saturating_add(elapsed_ns(plan_started));

    let mut visible_plan = None;
    if let Some(mask) = occlusion_mask {
        let intersection_started = std::time::Instant::now();
        let measured = if execute_occlusion {
            let (filtered, measured) = plan
                .visible_plan(mask)
                .map_err(|error| format!("full-card SDF visible plan: {error}"))?;
            visible_plan = Some(filtered);
            measured
        } else {
            plan.measure_occlusion(mask)
                .map_err(|error| format!("full-card SDF occlusion measure: {error}"))?
        };
        aggregate.timings.occlusion_intersection_ns = aggregate
            .timings
            .occlusion_intersection_ns
            .saturating_add(elapsed_ns(intersection_started));
        add_sdf_occlusion_stats(&mut aggregate.occlusion, measured);
    }

    let expected_len = grid.canvas_width as usize * grid.canvas_height as usize * 4;
    if pixels.len() != expected_len {
        return Err(format!(
            "ordered SDF compositor requires a tight RGBA8888 premul buffer of {expected_len} bytes, got {}",
            pixels.len()
        ));
    }
    let execution_plan = visible_plan.as_ref().unwrap_or(&plan);
    let execute_started = std::time::Instant::now();
    let execution = match executor {
        FullCardSdfCandidateExecutor::ScalarF32 => execution_plan
            .execute_scalar_f32_over(&execution_source, pixels)
            .map_err(|error| format!("full-card scalar SDF execute: {error}"))?,
        FullCardSdfCandidateExecutor::SimdF32 => execution_plan
            .execute_simd_f32_over(&execution_source, pixels)
            .map_err(|error| format!("full-card SIMD SDF execute: {error}"))?,
    };
    aggregate.timings.execute_ns = aggregate
        .timings
        .execute_ns
        .saturating_add(elapsed_ns(execute_started));
    add_sdf_plan_stats(&mut aggregate.plan_stats, plan.stats());
    add_sdf_execution_stats(&mut aggregate.execution_stats, execution);
    aggregate.plan_bytes = aggregate.plan_bytes.saturating_add(plan.resident_bytes());
    aggregate.span_bytes = aggregate.span_bytes.saturating_add(plan.span_bytes());
    if let Some(visible_plan) = visible_plan.as_ref() {
        aggregate.plan_bytes = aggregate
            .plan_bytes
            .saturating_add(visible_plan.resident_bytes());
        aggregate.span_bytes = aggregate
            .span_bytes
            .saturating_add(visible_plan.span_bytes());
    }
    aggregate.sdf_run_count = aggregate.sdf_run_count.saturating_add(1);
    pending.clear();
    Ok(())
}

fn add_sdf_occlusion_stats(
    total: &mut crate::sdf::tile::SdfOcclusionStats,
    value: crate::sdf::tile::SdfOcclusionStats,
) {
    total.occluded_fragment_count = total
        .occluded_fragment_count
        .saturating_add(value.occluded_fragment_count);
    total.visible_fragment_count = total
        .visible_fragment_count
        .saturating_add(value.visible_fragment_count);
    total.occluded_text_fragment_count = total
        .occluded_text_fragment_count
        .saturating_add(value.occluded_text_fragment_count);
    total.occluded_shape_fragment_count = total
        .occluded_shape_fragment_count
        .saturating_add(value.occluded_shape_fragment_count);
    total.fully_occluded_command_count = total
        .fully_occluded_command_count
        .saturating_add(value.fully_occluded_command_count);
}

fn add_sdf_plan_stats(
    total: &mut crate::sdf::tile::SdfPlanStats,
    value: crate::sdf::tile::SdfPlanStats,
) {
    total.command_count = total.command_count.saturating_add(value.command_count);
    total.text_command_count = total
        .text_command_count
        .saturating_add(value.text_command_count);
    total.shape_command_count = total
        .shape_command_count
        .saturating_add(value.shape_command_count);
    total.span_count = total.span_count.saturating_add(value.span_count);
    total.text_span_count = total.text_span_count.saturating_add(value.text_span_count);
    total.shape_span_count = total
        .shape_span_count
        .saturating_add(value.shape_span_count);
    total.tile_count = total.tile_count.saturating_add(value.tile_count);
    total.nonempty_tile_count = total
        .nonempty_tile_count
        .saturating_add(value.nonempty_tile_count);
    total.covered_fragment_count = total
        .covered_fragment_count
        .saturating_add(value.covered_fragment_count);
    total.text_covered_fragment_count = total
        .text_covered_fragment_count
        .saturating_add(value.text_covered_fragment_count);
    total.shape_covered_fragment_count = total
        .shape_covered_fragment_count
        .saturating_add(value.shape_covered_fragment_count);
}

fn add_sdf_execution_stats(
    total: &mut crate::sdf::tile::SdfExecutionStats,
    value: crate::sdf::tile::SdfExecutionStats,
) {
    total.shaded_fragment_count = total
        .shaded_fragment_count
        .saturating_add(value.shaded_fragment_count);
    total.text_shaded_fragment_count = total
        .text_shaded_fragment_count
        .saturating_add(value.text_shaded_fragment_count);
    total.shape_shaded_fragment_count = total
        .shape_shaded_fragment_count
        .saturating_add(value.shape_shaded_fragment_count);
    total.sampled_texel_count = total
        .sampled_texel_count
        .saturating_add(value.sampled_texel_count);
    total.blended_fragment_count = total
        .blended_fragment_count
        .saturating_add(value.blended_fragment_count);
    total.text_blended_fragment_count = total
        .text_blended_fragment_count
        .saturating_add(value.text_blended_fragment_count);
    total.shape_blended_fragment_count = total
        .shape_blended_fragment_count
        .saturating_add(value.shape_blended_fragment_count);
    total.simd_packet_count = total
        .simd_packet_count
        .saturating_add(value.simd_packet_count);
    total.swizzled_packet_count = total
        .swizzled_packet_count
        .saturating_add(value.swizzled_packet_count);
    total.gather_fallback_packet_count = total
        .gather_fallback_packet_count
        .saturating_add(value.gather_fallback_packet_count);
    total.precomputed_shape_fragment_count = total
        .precomputed_shape_fragment_count
        .saturating_add(value.precomputed_shape_fragment_count);
    total.precomputed_shape_span_count = total
        .precomputed_shape_span_count
        .saturating_add(value.precomputed_shape_span_count);
    total.direct_output_run_count = total
        .direct_output_run_count
        .saturating_add(value.direct_output_run_count);
    total.direct_output_packet_count = total
        .direct_output_packet_count
        .saturating_add(value.direct_output_packet_count);
}

fn elapsed_ns(started: std::time::Instant) -> u64 {
    started.elapsed().as_nanos().min(u64::MAX as u128) as u64
}

fn render_authored_image_into_pixels(
    pixels: &mut [u8],
    scene: &allium_renderer_core::profile_scene::ResolvedProfileScene,
    store: &crate::render_object::MappedRenderObjectStore,
    text_atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
    md: &MasterData,
    assets: Option<&AssetStore>,
    authored_kind: allium_renderer_core::AuthoredElementKind,
    authored_index: u32,
    width: u32,
    height: u32,
    canvas_origin_x: f32,
    canvas_origin_y: f32,
    image_executor: FullCardSdfCandidateExecutor,
) -> Result<crate::profile_compositor::ProfileCompositorStats, String> {
    let expected_len = width as usize * height as usize * 4;
    if pixels.len() != expected_len {
        return Err(format!(
            "ordered Image compositor requires a tight RGBA8888 premul buffer of {expected_len} bytes, got {}",
            pixels.len()
        ));
    }
    let translated_scene;
    let scene = if canvas_origin_x != 0.0 || canvas_origin_y != 0.0 {
        translated_scene = {
            let mut scene = scene.clone();
            for layer in &mut scene.layers {
                if layer.authored_kind == authored_kind && layer.authored_index == authored_index {
                    layer.matrix[4] += canvas_origin_x;
                    layer.matrix[5] += canvas_origin_y;
                }
            }
            scene
        };
        &translated_scene
    } else {
        scene
    };
    match image_executor {
        FullCardSdfCandidateExecutor::SimdF32 => {
            crate::profile_compositor::render_authored_profile_into_simd(
                scene,
                store,
                text_atlases,
                md,
                assets,
                authored_kind,
                authored_index,
                pixels,
                width,
                height,
            )
        }
        FullCardSdfCandidateExecutor::ScalarF32 => {
            crate::profile_compositor::render_authored_profile_into_scalar(
                scene,
                store,
                text_atlases,
                md,
                assets,
                authored_kind,
                authored_index,
                pixels,
                width,
                height,
            )
        }
    }
    .map_err(|error| error.to_string())
}

fn render_element_authored_identity(
    card: &CustomProfileCard,
    element: &crate::elements::RenderElement<'_>,
) -> Option<(allium_renderer_core::AuthoredElementKind, u32)> {
    use allium_renderer_core::AuthoredElementKind;

    fn index_of<T>(items: &[T], target: &T) -> Option<u32> {
        items
            .iter()
            .position(|candidate| std::ptr::eq(candidate, target))
            .and_then(|index| u32::try_from(index).ok())
    }

    match element {
        crate::elements::RenderElement::Text(value) => {
            index_of(&card.texts, value).map(|index| (AuthoredElementKind::Text, index))
        }
        crate::elements::RenderElement::Shape(value) => {
            index_of(&card.shapes, value).map(|index| (AuthoredElementKind::Shape, index))
        }
        crate::elements::RenderElement::CardMember(value) => index_of(&card.card_members, value)
            .map(|index| (AuthoredElementKind::CardMember, index)),
        crate::elements::RenderElement::Stamp(value) => {
            index_of(&card.stamps, value).map(|index| (AuthoredElementKind::Stamp, index))
        }
        crate::elements::RenderElement::Other(value) => {
            index_of(&card.others, value).map(|index| (AuthoredElementKind::Other, index))
        }
        crate::elements::RenderElement::BondsHonor(value) => index_of(&card.bonds_honors, value)
            .map(|index| (AuthoredElementKind::BondsHonor, index)),
        crate::elements::RenderElement::Honor(value) => {
            index_of(&card.honors, value).map(|index| (AuthoredElementKind::Honor, index))
        }
        crate::elements::RenderElement::Collection(value) => {
            index_of(&card.collections, value).map(|index| (AuthoredElementKind::Collection, index))
        }
        crate::elements::RenderElement::General(value) => {
            index_of(&card.generals, value).map(|index| (AuthoredElementKind::General, index))
        }
        crate::elements::RenderElement::StandMember(value) => index_of(&card.stand_members, value)
            .map(|index| (AuthoredElementKind::StandMember, index)),
        crate::elements::RenderElement::GeneralBackground(value) => {
            index_of(&card.general_backgrounds, value)
                .map(|index| (AuthoredElementKind::GeneralBackground, index))
        }
        crate::elements::RenderElement::StoryBackground(value) => {
            index_of(&card.story_backgrounds, value)
                .map(|index| (AuthoredElementKind::StoryBackground, index))
        }
    }
}

fn profile_command_kind(
    element: &crate::elements::RenderElement<'_>,
) -> crate::profile_backend::ProfileCommandKind {
    match element {
        crate::elements::RenderElement::Text(_) => crate::profile_backend::ProfileCommandKind::Text,
        crate::elements::RenderElement::Shape(_) => {
            crate::profile_backend::ProfileCommandKind::Shape
        }
        _ => crate::profile_backend::ProfileCommandKind::Image,
    }
}

#[allow(dead_code)]
fn record_command_telemetry(
    telemetry: &mut crate::profile_backend::ProfileRenderTelemetry,
    kind: crate::profile_backend::ProfileCommandKind,
    cpu_ns: u64,
) {
    if let Some(command) = telemetry
        .commands
        .iter_mut()
        .find(|command| command.kind == kind)
    {
        command.command_count = command.command_count.saturating_add(1);
        command.cpu_ns = command.cpu_ns.saturating_add(cpu_ns);
    } else {
        telemetry
            .commands
            .push(crate::profile_backend::CommandTelemetry {
                kind,
                command_count: 1,
                covered_fragments: 0,
                blended_fragments: 0,
                cpu_ns,
            });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum LayerDynamic {
    TmpLineIndent {
        fps: u32,
        #[serde(rename = "loop")]
        looped: bool,
        frames: Vec<LayerDynamicFrame>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerDynamicFrame {
    pub frame: u32,
    pub dx: f32,
    pub dy: f32,
}

/// Premultiplied RGBA8 layer raster produced by the ordered SDF path,
/// cropped to the tight non-transparent bounds.
/// What a rasterized animation layer carries: either the pixels themselves,
/// or the commands needed to produce them on demand.
pub enum AnimationLayerRaster {
    /// Pixels held for the whole export.
    Ordered(OrderedLayerRaster),
    /// Commands held instead of pixels, for a layer whose surface exceeds the
    /// bytes an export may retain. Windows of it are rasterized per frame.
    DeferredSdf(DeferredAnimationSdfLayer),
}

/// A rasterized animation layer together with the execution telemetry of the
/// SDF run that produced it.
///
/// `execution` is `None` when no run took place: the layer covered no pixels,
/// or its commands were retained in place of its pixels.
pub struct CroppedLayerBackendRaster {
    pub raster: AnimationLayerRaster,
    pub execution: Option<FullCardSdfExecutionOutput>,
}

/// An animation layer kept as commands rather than pixels.
///
/// The commands are in canvas coordinates — un-shifted — so a window of any
/// size can be rasterized from them by translating into that window's origin.
pub struct DeferredAnimationSdfLayer {
    pub(crate) commands: Vec<crate::sdf::tile::SdfDrawCommand>,
    pub(crate) text_atlases: std::sync::Arc<crate::sdf::atlas::MappedSdfAtlasSet>,
    pub(crate) shape_atlas: Option<std::sync::Arc<crate::sdf::shape_atlas::MappedShapeSdfAtlas>>,
    pub(crate) runtime_text: Vec<crate::sdf::tile::RuntimeTextSdfPage>,
    pub(crate) executor: SdfLayerCandidateExecutor,
    pub(crate) tile_width: u16,
    pub(crate) tile_height: u16,
    /// Layer bounds in canvas coordinates.
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Dynamic timeline metadata for the layer, when requested and present.
    pub dynamic: Option<LayerDynamic>,
}

/// One rasterized window of a deferred SDF animation layer: the pixels of a
/// source-space sub-rectangle, regenerated from the retained commands.
pub(crate) struct RenderedDeferredAnimationWindow {
    /// Tight premultiplied RGBA8, `width * 4` bytes per row.
    pub(crate) pixels: Vec<u8>,
    /// Window origin in canvas coordinates.
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Peak transient bytes: the window surface plus the cropped copy.
    pub(crate) scratch_peak_bytes: usize,
    /// Execution telemetry for the window's SDF run.
    pub(crate) execution: FullCardSdfExecutionOutput,
}

pub struct OrderedLayerRaster {
    /// Tight premultiplied RGBA8, `width * 4` bytes per row.
    pub pixels: Vec<u8>,
    /// Crop offset in canvas coordinates (any dynamic expansion removed).
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Dynamic timeline metadata for the layer, when requested and present.
    pub dynamic: Option<LayerDynamic>,
    /// Elements the run drew through the legacy renderer; zero when the
    /// SDF executors and the software image path covered everything.
    pub legacy_element_count: u64,
    /// Peak transient bytes: the full surface plus the cropped copy.
    pub scratch_peak_bytes: usize,
}

impl CustomProfileRenderer {
    /// Reports whether a card animates, without rendering it: builds the
    /// scene the export would render and preflights its compiled programs up
    /// to `maximum_tick`.
    #[cfg(feature = "animation-export")]
    pub fn animation_preflight_with_profile(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        document_key: &str,
        region: &str,
        maximum_tick: u64,
    ) -> Result<allium_renderer_core::AnimationPreflight, String> {
        let md = self.snapshot();
        let scene = crate::core_shadow::build_scene(
            card,
            &md,
            document_key,
            region,
            profile,
            region,
            self.assets.as_deref(),
        )?;
        scene
            .animation_preflight(maximum_tick)
            .map_err(|error| error.to_string())
    }

    /// Renders a card's animation export: preflights the compiled scene,
    /// rasterizes each element layer through the ordered SDF path with the
    /// executors the backend selection grants, composites the frames, and
    /// encodes them with the preset's format. Static cards return
    /// `animated: false` without an artifact.
    #[cfg(feature = "animation-export")]
    pub fn render_animation_with_profile_backend(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        document_key: &str,
        region: &str,
        preset: &crate::animation::ResolvedAnimationPreset,
        backend: Option<crate::profile_backend::ProfileBackendConfig>,
    ) -> Result<crate::animation::ProfileAnimationExport, String> {
        let generation = self.pin_render_object_generation();
        let md = self.snapshot();
        crate::animation::export_profile_animation(
            self,
            card,
            profile,
            document_key,
            region,
            &md,
            self.assets.as_deref(),
            preset,
            backend,
            generation.store(),
        )
    }

    /// Resolves a backend config against this renderer's capabilities for the
    /// animation layer raster.
    #[cfg(feature = "animation-export")]
    pub(crate) fn resolve_animation_profile_backend(
        &self,
        config: &crate::profile_backend::ProfileBackendConfig,
    ) -> Result<crate::profile_backend::ResolvedProfileBackend, String> {
        config
            .resolve(self.profile_backend_capabilities())
            .map_err(|error| error.to_string())
    }

    /// Records the identity of every atlas the animation layer raster may
    /// sample, so a telemetry consumer can tell which atlas build produced the
    /// pixels. Atlases the selection did not enable are left out.
    #[cfg(feature = "animation-export")]
    pub(crate) fn record_animation_profile_atlas_identities(
        &self,
        telemetry: &mut crate::profile_backend::ProfileRenderTelemetry,
        text_enabled: bool,
        shape_enabled: bool,
        usage: &str,
    ) {
        let sdf_atlases = self.sdf_atlases.load_full();
        record_profile_atlas_identities(
            telemetry,
            text_enabled.then_some(sdf_atlases.as_ref()),
            shape_enabled
                .then_some(self.shape_sdf_atlas.as_deref())
                .flatten(),
            usage,
        );
    }

    /// Renders one element layer through the ordered SDF path — the same
    /// pipeline the profile backend uses for full pages — onto a transparent
    /// surface expanded for the layer's dynamic travel, then crops to the
    /// tight non-transparent bounds. Executor choices follow the backend
    /// vocabulary; `ShapeSdfExecutor::Auto` must be resolved by the caller.
    #[allow(clippy::too_many_arguments)]
    pub fn render_ordered_element_layer_cropped(
        &self,
        card: &CustomProfileCard,
        md: &MasterData,
        assets: Option<&AssetStore>,
        profile: Option<&crate::profile::ProfileData>,
        text_sdf: crate::profile_backend::TextSdfExecutor,
        shape_sdf: crate::profile_backend::ShapeSdfExecutor,
        include_dynamic_bounds: bool,
        forbid_legacy_elements: bool,
        tile_width: u16,
        tile_height: u16,
        render_object_store: Option<&crate::render_object::MappedRenderObjectStore>,
        maximum_retained_layer_bytes: usize,
    ) -> Result<CroppedLayerBackendRaster, String> {
        let w = crate::transform::CANVAS_WIDTH as i32;
        let h = crate::transform::CANVAS_HEIGHT as i32;
        let elements = crate::elements::flatten_and_sort(card);
        let dynamic = if include_dynamic_bounds {
            layer_dynamic_for_elements(&elements, md)
        } else {
            None
        };
        let text_executor = match text_sdf {
            crate::profile_backend::TextSdfExecutor::LegacySkia => None,
            crate::profile_backend::TextSdfExecutor::Simd => {
                Some(FullCardSdfCandidateExecutor::SimdF32)
            }
            crate::profile_backend::TextSdfExecutor::ScalarOracle => {
                Some(FullCardSdfCandidateExecutor::ScalarF32)
            }
        };
        let shape_executor = match shape_sdf {
            crate::profile_backend::ShapeSdfExecutor::Skia => None,
            crate::profile_backend::ShapeSdfExecutor::Simd => {
                Some(FullCardSdfCandidateExecutor::SimdF32)
            }
            crate::profile_backend::ShapeSdfExecutor::ScalarOracle => {
                Some(FullCardSdfCandidateExecutor::ScalarF32)
            }
            crate::profile_backend::ShapeSdfExecutor::Auto => {
                return Err("shape executor Auto must be resolved by the caller".into());
            }
        };
        let expansion = dynamic
            .as_ref()
            .map(dynamic_canvas_expansion)
            .unwrap_or_default();
        let tight_executor = tight_animation_sdf_executor(&elements, text_executor, shape_executor);
        if let Some(executor) = tight_executor {
            return self.render_animation_sdf_layer_tight(
                card,
                md,
                assets,
                profile,
                executor,
                tile_width,
                tile_height,
                dynamic,
                expansion,
                maximum_retained_layer_bytes,
            );
        }
        let surface_w = w + expansion.left + expansion.right;
        let surface_h = h + expansion.top + expansion.bottom;
        let spec = OrderedSdfSurfaceSpec {
            surface_width: u32::try_from(surface_w)
                .map_err(|_| "layer surface width overflow".to_string())?,
            surface_height: u32::try_from(surface_h)
                .map_err(|_| "layer surface height overflow".to_string())?,
            canvas_width: w as u32,
            canvas_height: h as u32,
            canvas_origin_x: expansion.left as f32,
            canvas_origin_y: expansion.top as f32,
            clear_rgba: [0, 0, 0, 0],
            prepare_direct_axis_shape: false,
            forbid_legacy_elements,
        };
        let mut output = self.render_ordered_sdf_surface_candidate(
            card,
            profile,
            text_executor,
            shape_executor,
            tile_width,
            tile_height,
            false,
            false,
            spec,
            md,
            assets,
            None,
            render_object_store,
        )?;
        let rgba = std::mem::take(&mut output.rgba);
        let row_bytes = surface_w as usize * 4;
        let surface_bytes = row_bytes * surface_h as usize;
        let bounds_started = std::time::Instant::now();
        let (bx, by, bw, bh) = opaque_bounds_for_pixels(&rgba, surface_w, surface_h, row_bytes)?;
        output.timings.postprocess_bounds_ns = elapsed_ns(bounds_started);
        let crop_started = std::time::Instant::now();
        let raster = if bw == 0 || bh == 0 {
            OrderedLayerRaster {
                pixels: Vec::new(),
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                dynamic,
                legacy_element_count: output.legacy_element_count,
                scratch_peak_bytes: surface_bytes,
            }
        } else {
            OrderedLayerRaster {
                pixels: crop_pixels_lossless(
                    &rgba,
                    row_bytes,
                    surface_w as u32,
                    surface_h as u32,
                    bx,
                    by,
                    bw,
                    bh,
                )?,
                x: bx as i32 - expansion.left,
                y: by as i32 - expansion.top,
                width: bw,
                height: bh,
                dynamic,
                legacy_element_count: output.legacy_element_count,
                scratch_peak_bytes: surface_bytes.saturating_add(bw as usize * bh as usize * 4),
            }
        };
        output.timings.postprocess_crop_ns = elapsed_ns(crop_started);
        output.timings.total_ns = output
            .timings
            .total_ns
            .saturating_add(output.timings.postprocess_bounds_ns)
            .saturating_add(output.timings.postprocess_crop_ns);
        let raster = AnimationLayerRaster::Ordered(raster);
        self.recycle_profile_rgba_scratch(rgba);
        Ok(CroppedLayerBackendRaster {
            raster,
            execution: Some(output),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn render_animation_sdf_layer_tight(
        &self,
        card: &CustomProfileCard,
        md: &MasterData,
        assets: Option<&AssetStore>,
        profile: Option<&crate::profile::ProfileData>,
        executor: SdfLayerCandidateExecutor,
        tile_width: u16,
        tile_height: u16,
        dynamic: Option<LayerDynamic>,
        reachable: CanvasExpansion,
        maximum_retained_layer_bytes: usize,
    ) -> Result<CroppedLayerBackendRaster, String> {
        let total_started = std::time::Instant::now();
        let sdf_atlases = self.sdf_atlases.load_full();
        let capture_started = std::time::Instant::now();
        let captured = capture_sdf_primitives(
            card,
            md,
            assets,
            Some(&sdf_atlases),
            profile,
            crate::transform::CANVAS_WIDTH as u32,
            crate::transform::CANVAS_HEIGHT as u32,
            SdfCaptureKinds::TEXT_AND_SHAPE,
        )?;
        let capture_ns = elapsed_ns(capture_started);

        let base_source = crate::sdf::tile::MixedSdfAtlasSource::new(
            &sdf_atlases,
            self.shape_sdf_atlas.as_deref(),
        )
        .map_err(|error| error.to_string())?;

        let mut runtime_text_pages = Vec::new();
        let mut realtime_edt_glyphs = Vec::new();
        let mut realtime_edt_batch = crate::profile_backend::RealtimeEdtBatchTelemetry::default();
        let prepared_batch = active_realtime_edt_batch();
        let runtime_pages = self
            .realtime_oversized_glyph_generation
            .then_some(&mut runtime_text_pages);
        let mapping_started = std::time::Instant::now();
        let mut commands = map_captured_sdf_commands(
            &captured.primitives,
            &sdf_atlases,
            self.shape_sdf_atlas.as_deref(),
            &base_source,
            runtime_pages,
            Some(&mut realtime_edt_glyphs),
            Some(&mut realtime_edt_batch),
            None,
        )?;
        let command_mapping_ns = elapsed_ns(mapping_started);

        let empty = CroppedLayerBackendRaster {
            raster: AnimationLayerRaster::Ordered(OrderedLayerRaster {
                pixels: Vec::new(),
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                dynamic: dynamic.clone(),
                legacy_element_count: 0,
                scratch_peak_bytes: 0,
            }),
            execution: None,
        };
        let Some((content_x, content_y, content_w, content_h)) =
            tight_sdf_command_bounds(&commands)
        else {
            return Ok(empty);
        };
        // Keep only the part of the content a frame can ever show. The layer's
        // travel bounds the canvas positions it can occupy, so intersecting the
        // command extent with the travelled canvas drops pixels no frame can
        // reach while keeping every pixel some frame can.
        let canvas_w = crate::transform::CANVAS_WIDTH as i32;
        let canvas_h = crate::transform::CANVAS_HEIGHT as i32;
        let left = content_x.max(-reachable.left);
        let top = content_y.max(-reachable.top);
        let right = content_x
            .checked_add(i32::try_from(content_w).map_err(|_| "layer width overflow".to_string())?)
            .ok_or_else(|| "layer right edge overflow".to_string())?
            .min(canvas_w + reachable.right);
        let bottom = content_y
            .checked_add(i32::try_from(content_h).map_err(|_| "layer height overflow".to_string())?)
            .ok_or_else(|| "layer bottom edge overflow".to_string())?
            .min(canvas_h + reachable.bottom);
        if right <= left || bottom <= top {
            return Ok(empty);
        }
        let (origin_x, origin_y) = (left, top);
        let width = u32::try_from(right - left).map_err(|_| "layer width overflow".to_string())?;
        let height =
            u32::try_from(bottom - top).map_err(|_| "layer height overflow".to_string())?;
        let (surface_bytes, retained_peak_bytes) =
            animation_sdf_retained_surface_bytes(width, height)?;

        // A layer whose surface would outlive the bytes this export may retain
        // keeps its commands instead of its pixels; windows of it are
        // rasterized per frame. The commands stay in canvas coordinates here,
        // so any window origin can be applied later.
        if retained_peak_bytes > maximum_retained_layer_bytes {
            return Ok(CroppedLayerBackendRaster {
                raster: AnimationLayerRaster::DeferredSdf(DeferredAnimationSdfLayer {
                    commands,
                    text_atlases: sdf_atlases,
                    shape_atlas: self.shape_sdf_atlas.clone(),
                    runtime_text: prepared_batch
                        .as_deref()
                        .map(|batch| batch.pages.clone())
                        .unwrap_or(runtime_text_pages),
                    executor,
                    tile_width,
                    tile_height,
                    x: origin_x,
                    y: origin_y,
                    width,
                    height,
                    dynamic,
                }),
                execution: None,
            });
        }

        shift_sdf_commands(&mut commands, -(origin_x as f32), -(origin_y as f32));
        let runtime_pages = prepared_batch
            .as_deref()
            .map(|batch| batch.pages.as_slice())
            .unwrap_or(runtime_text_pages.as_slice());
        let source = crate::sdf::tile::MixedSdfAtlasSource::with_runtime_text(
            &sdf_atlases,
            self.shape_sdf_atlas.as_deref(),
            runtime_pages,
        )
        .map_err(|error| error.to_string())?;

        let plan_started = std::time::Instant::now();
        let plan = crate::sdf::tile::SdfTilePlan::build_for_one_shot_dynamic_layer(
            crate::sdf::tile::TileGrid {
                canvas_width: width,
                canvas_height: height,
                tile_width,
                tile_height,
            },
            &commands,
            &source,
        )
        .map_err(|error| error.to_string())?;
        let plan_build_ns = elapsed_ns(plan_started);

        let mut rgba = self.take_profile_rgba_scratch(surface_bytes, [0, 0, 0, 0]);
        let execute_started = std::time::Instant::now();
        let execution_stats = match executor {
            SdfLayerCandidateExecutor::ScalarF32 => plan
                .execute_scalar_f32(&source, [0, 0, 0, 0], &mut rgba)
                .map_err(|error| error.to_string())?,
            SdfLayerCandidateExecutor::SimdF32 => plan
                .execute_simd(
                    &source,
                    [0, 0, 0, 0],
                    &mut rgba,
                    crate::sdf::tile::SdfAccumulationMode::F32Tile,
                )
                .map_err(|error| error.to_string())?,
        };
        let execute_ns = elapsed_ns(execute_started);

        let crop_started = std::time::Instant::now();
        let pixels = crop_pixels_lossless(
            &rgba,
            width as usize * 4,
            width,
            height,
            0,
            0,
            width,
            height,
        )?;
        let crop_ns = elapsed_ns(crop_started);
        // The surface is already the intersection of the content bounds and
        // the reachable canvas, so the full surface is the layer raster; the
        // accounting matches the production backend (surface plus the full
        // surface copy taken during the crop).
        let scratch_peak_bytes = surface_bytes.saturating_mul(2);
        self.recycle_profile_rgba_scratch(rgba);
        let mut execution = FullCardSdfExecutionOutput {
            width,
            height,
            clear_rgba: [0, 0, 0, 0],
            ..FullCardSdfExecutionOutput::default()
        };
        execution.sdf_run_count = 1;
        execution.sdf_text_element_count = captured.text_count;
        execution.sdf_shape_element_count = captured.shape_count;
        execution.captured_text_count = captured.text_count;
        execution.captured_shape_count = captured.shape_count;
        execution.realtime_edt_glyphs = realtime_edt_glyphs;
        execution.realtime_edt_batch = realtime_edt_batch;
        execution.plan_stats = plan.stats();
        execution.execution_stats = execution_stats;
        execution.atlas_mapped_bytes =
            sdf_atlases
                .mapped_bytes()
                .saturating_add(self.shape_sdf_atlas.as_deref().map_or(
                    0,
                    crate::sdf::shape_atlas::MappedShapeSdfAtlas::mapped_bytes,
                ));
        execution.plan_bytes = plan.resident_bytes();
        execution.span_bytes = plan.span_bytes();
        execution.timings.capture_ns = capture_ns;
        execution.timings.command_mapping_ns = command_mapping_ns;
        execution.timings.plan_build_ns = plan_build_ns;
        execution.timings.execute_ns = execute_ns;
        execution.timings.postprocess_crop_ns = crop_ns;
        execution.timings.total_ns = elapsed_ns(total_started);
        Ok(CroppedLayerBackendRaster {
            raster: AnimationLayerRaster::Ordered(OrderedLayerRaster {
                pixels,
                x: origin_x,
                y: origin_y,
                width,
                height,
                dynamic,
                legacy_element_count: 0,
                scratch_peak_bytes,
            }),
            execution: Some(execution),
        })
    }

    /// Rasterizes one source-space window of a deferred SDF animation layer
    /// from its retained commands, returning the window's pixels.
    ///
    /// `x`/`y`/`width`/`height` select a sub-rectangle of the layer's bounds in
    /// canvas coordinates. The commands are cloned and translated into that
    /// window's origin before the plan is built, so the window is self-contained
    /// and any frame can rasterize any window of the same layer.
    #[cfg(feature = "animation-export")]
    pub(crate) fn render_deferred_animation_sdf_window(
        &self,
        layer: &DeferredAnimationSdfLayer,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Result<RenderedDeferredAnimationWindow, String> {
        if width == 0 || height == 0 {
            return Err("deferred animation SDF window must be non-empty".into());
        }
        let right = x
            .checked_add(i32::try_from(width).map_err(|_| "deferred window width overflow")?)
            .ok_or("deferred window right overflow")?;
        let bottom = y
            .checked_add(i32::try_from(height).map_err(|_| "deferred window height overflow")?)
            .ok_or("deferred window bottom overflow")?;
        let layer_right = layer
            .x
            .checked_add(i32::try_from(layer.width).map_err(|_| "deferred layer width overflow")?)
            .ok_or("deferred layer right overflow")?;
        let layer_bottom = layer
            .y
            .checked_add(i32::try_from(layer.height).map_err(|_| "deferred layer height overflow")?)
            .ok_or("deferred layer bottom overflow")?;
        if x < layer.x || y < layer.y || right > layer_right || bottom > layer_bottom {
            return Err("deferred animation SDF window is outside the source layer".into());
        }

        let mut commands = layer.commands.clone();
        shift_sdf_commands(&mut commands, -(x as f32), -(y as f32));
        let source = crate::sdf::tile::MixedSdfAtlasSource::with_runtime_text(
            &layer.text_atlases,
            layer.shape_atlas.as_deref(),
            &layer.runtime_text,
        )
        .map_err(|error| error.to_string())?;
        let plan = crate::sdf::tile::SdfTilePlan::build_for_one_shot_dynamic_layer(
            crate::sdf::tile::TileGrid {
                canvas_width: width,
                canvas_height: height,
                tile_width: layer.tile_width,
                tile_height: layer.tile_height,
            },
            &commands,
            &source,
        )
        .map_err(|error| error.to_string())?;
        let surface_bytes = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "deferred animation SDF surface byte length overflow".to_string())?;
        let mut rgba = self.take_profile_rgba_scratch(surface_bytes, [0, 0, 0, 0]);
        let execution_stats = match layer.executor {
            SdfLayerCandidateExecutor::ScalarF32 => plan
                .execute_scalar_f32(&source, [0, 0, 0, 0], &mut rgba)
                .map_err(|error| error.to_string())?,
            SdfLayerCandidateExecutor::SimdF32 => plan
                .execute_simd(
                    &source,
                    [0, 0, 0, 0],
                    &mut rgba,
                    crate::sdf::tile::SdfAccumulationMode::F32Tile,
                )
                .map_err(|error| error.to_string())?,
        };
        let pixels = crop_pixels_lossless(
            &rgba,
            width as usize * 4,
            width,
            height,
            0,
            0,
            width,
            height,
        )?;
        self.recycle_profile_rgba_scratch(rgba);
        let stats = plan.stats();
        let mut execution = FullCardSdfExecutionOutput {
            width,
            height,
            clear_rgba: [0, 0, 0, 0],
            ..FullCardSdfExecutionOutput::default()
        };
        execution.sdf_run_count = 1;
        execution.sdf_text_element_count = stats.text_command_count;
        execution.sdf_shape_element_count = stats.shape_command_count;
        execution.captured_text_count = stats.text_command_count;
        execution.captured_shape_count = stats.shape_command_count;
        execution.plan_stats = stats;
        execution.execution_stats = execution_stats;
        execution.atlas_mapped_bytes =
            layer
                .text_atlases
                .mapped_bytes()
                .saturating_add(layer.shape_atlas.as_deref().map_or(
                    0,
                    crate::sdf::shape_atlas::MappedShapeSdfAtlas::mapped_bytes,
                ));
        execution.plan_bytes = plan.resident_bytes();
        execution.span_bytes = plan.span_bytes();
        Ok(RenderedDeferredAnimationWindow {
            pixels,
            x,
            y,
            width,
            height,
            scratch_peak_bytes: surface_bytes.saturating_mul(2),
            execution,
        })
    }
}

fn tight_animation_sdf_executor(
    elements: &[crate::elements::RenderElement<'_>],
    text_executor: Option<FullCardSdfCandidateExecutor>,
    shape_executor: Option<FullCardSdfCandidateExecutor>,
) -> Option<SdfLayerCandidateExecutor> {
    let mut selected = None;
    for element in elements.iter().filter(|element| element.visible()) {
        let candidate = match element {
            crate::elements::RenderElement::Text(_) => text_executor?,
            crate::elements::RenderElement::Shape(_) => shape_executor?,
            _ => return None,
        };
        if selected.is_some_and(|value| value != candidate) {
            return None;
        }
        selected = Some(candidate);
    }
    selected.map(|value| match value {
        FullCardSdfCandidateExecutor::ScalarF32 => SdfLayerCandidateExecutor::ScalarF32,
        FullCardSdfCandidateExecutor::SimdF32 => SdfLayerCandidateExecutor::SimdF32,
    })
}

fn tight_sdf_command_bounds(
    commands: &[crate::sdf::tile::SdfDrawCommand],
) -> Option<(i32, i32, u32, u32)> {
    const GUARD: f32 = 2.0;
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    for command in commands {
        let mut command_min_x = command
            .quad
            .iter()
            .map(|point| point.x)
            .fold(f32::INFINITY, f32::min);
        let mut command_min_y = command
            .quad
            .iter()
            .map(|point| point.y)
            .fold(f32::INFINITY, f32::min);
        let mut command_max_x = command
            .quad
            .iter()
            .map(|point| point.x)
            .fold(f32::NEG_INFINITY, f32::max);
        let mut command_max_y = command
            .quad
            .iter()
            .map(|point| point.y)
            .fold(f32::NEG_INFINITY, f32::max);

        if let Some(clip) = command.device_clip {
            command_min_x = command_min_x.max(clip.min_x);
            command_min_y = command_min_y.max(clip.min_y);
            command_max_x = command_max_x.min(clip.max_x);
            command_max_y = command_max_y.min(clip.max_y);
        }

        if command_min_x >= command_max_x || command_min_y >= command_max_y {
            continue;
        }

        min_x = min_x.min(command_min_x);
        min_y = min_y.min(command_min_y);
        max_x = max_x.max(command_max_x);
        max_y = max_y.max(command_max_y);
    }

    if ![min_x, min_y, max_x, max_y].into_iter().all(f32::is_finite) {
        return None;
    }

    let origin_x = (min_x - GUARD).floor() as i32;
    let origin_y = (min_y - GUARD).floor() as i32;
    let right = (max_x + GUARD).ceil() as i32;
    let bottom = (max_y + GUARD).ceil() as i32;
    let width = u32::try_from(right.checked_sub(origin_x)?).ok()?;
    let height = u32::try_from(bottom.checked_sub(origin_y)?).ok()?;
    (width != 0 && height != 0).then_some((origin_x, origin_y, width, height))
}

fn animation_sdf_retained_surface_bytes(width: u32, height: u32) -> Result<(usize, usize), String> {
    let surface_bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "animation SDF layer surface byte length overflow".to_string())?;
    Ok((surface_bytes, surface_bytes.saturating_mul(2)))
}

fn shift_sdf_commands(commands: &mut [crate::sdf::tile::SdfDrawCommand], dx: f32, dy: f32) {
    for command in commands {
        for point in &mut command.quad {
            point.x += dx;
            point.y += dy;
        }
        if let Some(clip) = &mut command.device_clip {
            clip.min_x += dx;
            clip.max_x += dx;
            clip.min_y += dy;
            clip.max_y += dy;
        }
    }
}

/// Copies a tight sub-rectangle out of a premultiplied RGBA8 buffer.
fn crop_pixels_lossless(
    pixels: &[u8],
    source_row_bytes: usize,
    surface_width: u32,
    surface_height: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let required_row_bytes = (surface_width as usize)
        .checked_mul(4)
        .ok_or_else(|| "layer surface row stride overflow".to_string())?;
    if source_row_bytes < required_row_bytes {
        return Err("layer surface row stride is shorter than one row".to_string());
    }
    let required_bytes = source_row_bytes
        .checked_mul(surface_height as usize)
        .ok_or_else(|| "layer surface byte length overflow".to_string())?;
    if pixels.len() < required_bytes {
        return Err("layer surface buffer is truncated".to_string());
    }
    let right = x
        .checked_add(width)
        .ok_or_else(|| "crop x overflow".to_string())?;
    let bottom = y
        .checked_add(height)
        .ok_or_else(|| "crop y overflow".to_string())?;
    if width == 0 || height == 0 || right > surface_width || bottom > surface_height {
        return Err("crop rect is empty or outside the layer surface".to_string());
    }
    let row_bytes = (width as usize)
        .checked_mul(4)
        .ok_or_else(|| "crop row stride overflow".to_string())?;
    let start_of = |row: usize| (y as usize + row) * source_row_bytes + x as usize * 4;
    if pixels.len() < start_of(height as usize - 1) + row_bytes {
        return Err("crop rect exceeds the layer pixel buffer".to_string());
    }
    let mut cropped = vec![0u8; row_bytes * height as usize];
    for row in 0..height as usize {
        let source = start_of(row);
        cropped[row * row_bytes..(row + 1) * row_bytes]
            .copy_from_slice(&pixels[source..source + row_bytes]);
    }
    Ok(cropped)
}

fn layer_dynamic_for_elements(
    elements: &[crate::elements::RenderElement<'_>],
    md: &MasterData,
) -> Option<LayerDynamic> {
    let visible_text = elements.iter().find_map(|elem| match elem {
        crate::elements::RenderElement::Text(text) if text.object_data.visible => Some(*text),
        _ => None,
    })?;
    let animation = crate::text::line_indent_x_animation(visible_text, md)?;
    let (_, _, angle_deg, sx, _) = crate::transform::extract_transform(&visible_text.object_data);
    let radians = angle_deg.to_radians();
    let cos = radians.cos();
    let sin = radians.sin();
    let frames = animation
        .frames
        .into_iter()
        .map(|frame| {
            let local_x = frame.dx_local * sx;
            LayerDynamicFrame {
                frame: frame.frame,
                dx: local_x * cos,
                dy: local_x * sin,
            }
        })
        .collect();

    Some(LayerDynamic::TmpLineIndent {
        fps: animation.fps,
        looped: animation.looped,
        frames,
    })
}

#[derive(Default)]
struct CanvasExpansion {
    left: i32,
    right: i32,
    top: i32,
    bottom: i32,
}

fn dynamic_canvas_expansion(dynamic: &LayerDynamic) -> CanvasExpansion {
    const PAD: i32 = 8;
    let mut min_dx = 0.0_f32;
    let mut max_dx = 0.0_f32;
    let mut min_dy = 0.0_f32;
    let mut max_dy = 0.0_f32;

    match dynamic {
        LayerDynamic::TmpLineIndent { frames, .. } => {
            for frame in frames {
                min_dx = min_dx.min(frame.dx);
                max_dx = max_dx.max(frame.dx);
                min_dy = min_dy.min(frame.dy);
                max_dy = max_dy.max(frame.dy);
            }
        }
    }

    CanvasExpansion {
        left: max_dx.max(0.0).ceil() as i32 + PAD,
        right: (-min_dx).max(0.0).ceil() as i32 + PAD,
        top: max_dy.max(0.0).ceil() as i32 + PAD,
        bottom: (-min_dy).max(0.0).ceil() as i32 + PAD,
    }
}

/// Tight non-transparent bounds of a premultiplied RGBA8 layer buffer.
///
/// Only the alpha channel takes part, so premultiplication does not affect the
/// result.
fn opaque_bounds_for_pixels(
    pixels: &[u8],
    w: i32,
    h: i32,
    row_bytes: usize,
) -> Result<(u32, u32, u32, u32), String> {
    if pixels.len() < row_bytes * h as usize {
        return Err("layer surface buffer is truncated".to_string());
    }
    Ok(find_opaque_bounds(pixels, w as u32, h as u32, row_bytes))
}

fn find_opaque_bounds_scalar(
    pixels: &[u8],
    width: u32,
    height: u32,
    row_bytes: usize,
) -> (u32, u32, u32, u32) {
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x: u32 = 0;
    let mut max_y: u32 = 0;

    for y in 0..height {
        let row_start = y as usize * row_bytes;
        for x in 0..width {
            // RGBA8888, alpha is byte offset 3
            let pixel_offset = row_start + (x as usize) * 4;
            let alpha = pixels[pixel_offset + 3];
            if alpha > 0 {
                if x < min_x {
                    min_x = x;
                }
                if x > max_x {
                    max_x = x;
                }
                if y < min_y {
                    min_y = y;
                }
                if y > max_y {
                    max_y = y;
                }
            }
        }
    }

    if max_x < min_x || max_y < min_y {
        return (0, 0, 0, 0);
    }

    (min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
}

/// Checks 16 RGBA8 pixels per ZMM load. The compare produces one bit per byte;
/// BMI2 PEXT compacts byte positions 3, 7, ... 63 into a 16-bit pixel mask.
/// The tail remains scalar, avoiding masked-load setup once per scanline.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw,bmi2")]
unsafe fn find_opaque_bounds_avx512(
    pixels: &[u8],
    width: u32,
    height: u32,
    row_bytes: usize,
) -> (u32, u32, u32, u32) {
    use std::arch::x86_64::*;

    const ALPHA_BYTE_BITS: u64 = 0x8888_8888_8888_8888;
    const PIXELS_PER_ZMM: u32 = 16;

    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let zero = _mm512_setzero_si512();

    for y in 0..height {
        let row = pixels.as_ptr().add(y as usize * row_bytes);
        let mut x = 0u32;
        while x + PIXELS_PER_ZMM <= width {
            let rgba = _mm512_loadu_si512(row.add(x as usize * 4).cast());
            let nonzero_bytes = _mm512_cmpneq_epi8_mask(rgba, zero);
            let alpha_pixels = _pext_u64(nonzero_bytes, ALPHA_BYTE_BITS) as u16;
            if alpha_pixels != 0 {
                min_y = min_y.min(y);
                max_y = max_y.max(y);
                min_x = min_x.min(x + alpha_pixels.trailing_zeros());
                max_x = max_x.max(x + (u16::BITS - 1 - alpha_pixels.leading_zeros()));
            }
            x += PIXELS_PER_ZMM;
        }
        while x < width {
            if *row.add(x as usize * 4 + 3) != 0 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
            x += 1;
        }
    }

    if min_x == width || min_y == height {
        (0, 0, 0, 0)
    } else {
        (min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
    }
}

#[cfg(test)]
mod expansion_tests {
    use super::{dynamic_canvas_expansion, LayerDynamic, LayerDynamicFrame};

    fn line_indent(offsets: &[(f32, f32)]) -> LayerDynamic {
        LayerDynamic::TmpLineIndent {
            fps: 60,
            looped: false,
            frames: offsets
                .iter()
                .enumerate()
                .map(|(index, &(dx, dy))| LayerDynamicFrame {
                    frame: index as u32,
                    dx,
                    dy,
                })
                .collect(),
        }
    }

    #[test]
    fn expansion_covers_travel_that_only_enters_the_canvas_later() {
        // The layer sits outside the canvas at frame 0 and travels inward. An
        // expansion derived from the frame-0 position alone would crop the
        // content away before it ever becomes visible.
        let expansion =
            dynamic_canvas_expansion(&line_indent(&[(0.0, 0.0), (-400.0, 0.0), (-900.0, 0.0)]));
        assert!(
            expansion.right >= 900,
            "travel toward the canvas must widen the surface, got {}",
            expansion.right,
        );
    }

    #[test]
    fn expansion_covers_travel_that_only_crosses_the_canvas_midway() {
        // Both endpoints sit far on the same side of the canvas while the path
        // between them crosses it. Sampling only the first and last frame would
        // report no travel at all and blank every frame in between.
        let expansion =
            dynamic_canvas_expansion(&line_indent(&[(0.0, 0.0), (-1200.0, 0.0), (-40.0, 0.0)]));
        assert!(
            expansion.right >= 1200,
            "a midway crossing must widen the surface, got {}",
            expansion.right,
        );
    }

    #[test]
    fn dynamic_canvas_expansion_covers_the_complete_motion_range() {
        let expansion = dynamic_canvas_expansion(&LayerDynamic::TmpLineIndent {
            fps: 60,
            looped: false,
            frames: vec![
                LayerDynamicFrame {
                    frame: 0,
                    dx: -6400.25,
                    dy: 120.5,
                },
                LayerDynamicFrame {
                    frame: 1,
                    dx: 310.1,
                    dy: -90.75,
                },
            ],
        });
        assert_eq!(expansion.left, 319);
        assert_eq!(expansion.right, 6409);
        assert_eq!(expansion.top, 129);
        assert_eq!(expansion.bottom, 99);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::masterdata::{MasterDataProvider, ResolvedColor, ResolvedHonor, ResourceInfo};
    use crate::types::{BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry};
    #[cfg(feature = "skia-oracle")]
    use crate::types::{
        CustomProfileCard, ObjectData, Quaternion, StampElement, TextElement, Vec3,
    };

    fn magnification_command(
        atlas_width: u32,
        atlas_height: u32,
        quad: [crate::sdf::tile::Point2; 4],
    ) -> crate::sdf::tile::SdfDrawCommand {
        crate::sdf::tile::SdfDrawCommand {
            kind: crate::sdf::tile::SdfPrimitiveKind::Text,
            atlas_set: 0,
            atlas_page: 0,
            atlas_rect: [0, 0, atlas_width, atlas_height],
            quad,
            device_clip: None,
            material: crate::sdf::tile::SdfCommandMaterial::Text(
                crate::sdf::tile::SdfMaterial::default(),
            ),
        }
    }

    #[test]
    fn realtime_edt_threshold_uses_final_command_geometry() {
        use crate::sdf::tile::Point2;

        let exactly_three = magnification_command(
            10,
            20,
            [
                Point2::new(0.0, 0.0),
                Point2::new(30.0, 0.0),
                Point2::new(30.0, 60.0),
                Point2::new(0.0, 60.0),
            ],
        );
        let above_three = magnification_command(
            10,
            20,
            [
                Point2::new(0.0, 0.0),
                Point2::new(30.1, 0.0),
                Point2::new(30.1, 60.0),
                Point2::new(0.0, 60.0),
            ],
        );
        assert_eq!(sdf_command_device_magnification(&exactly_three), Some(3.0));
        assert!(!(sdf_command_device_magnification(&exactly_three).unwrap() > 3.0));
        assert!(sdf_command_device_magnification(&above_three).unwrap() > 3.0);
    }

    #[test]
    fn final_command_magnification_handles_rotation_and_non_uniform_scale() {
        use crate::sdf::tile::Point2;

        let command = magnification_command(
            10,
            20,
            [
                Point2::new(5.0, 7.0),
                Point2::new(5.0, 37.0),
                Point2::new(-75.0, 37.0),
                Point2::new(-75.0, 7.0),
            ],
        );
        assert_eq!(sdf_command_device_magnification(&command), Some(4.0));
    }

    #[test]
    fn realtime_edt_spread_preserves_the_atlas_logical_distance_range() {
        assert_eq!(realtime_edt_sampling_spread(75.0, 6.0, 75.0), Some(6.0));
        assert_eq!(realtime_edt_sampling_spread(75.0, 6.0, 300.0), Some(24.0));
        assert_eq!(
            realtime_edt_sampling_spread(75.0, 6.0, 4096.0),
            Some(327.68)
        );
        assert_eq!(realtime_edt_sampling_spread(0.0, 6.0, 300.0), None);
        assert_eq!(realtime_edt_sampling_spread(75.0, 0.0, 300.0), None);
    }

    #[test]
    fn non_positive_resolved_text_size_is_an_invisible_sdf_operation() {
        fn glyph(font_size: f32) -> crate::text::ResolvedTextSdfGlyph {
            crate::text::ResolvedTextSdfGlyph {
                text: "(".into(),
                font_family: Some("test-family".into()),
                baseline_origin: crate::sdf::tile::Point2::new(0.0, 0.0),
                font_size,
                local_to_device: crate::sdf::tile::Affine2::IDENTITY,
                material: crate::sdf::tile::SdfMaterial::default(),
            }
        }

        assert!(captured_text_sdf_glyph_is_invisible(&glyph(0.0)));
        assert!(captured_text_sdf_glyph_is_invisible(&glyph(-147.636_35)));
        assert!(!captured_text_sdf_glyph_is_invisible(&glyph(0.001)));
        assert!(!captured_text_sdf_glyph_is_invisible(&glyph(f32::NAN)));
    }

    #[test]
    fn realtime_edt_request_identity_deduplicates_only_exact_sampling_requests() {
        use std::collections::BTreeSet;

        let duplicate_a = realtime_edt_request_key("test-family", '(', 300.0, 24.0);
        let duplicate_b = realtime_edt_request_key("test-family", '(', 300.0, 24.0);
        let different_size = realtime_edt_request_key("test-family", '(', 300.001, 24.0);
        let different_spread = realtime_edt_request_key("test-family", '(', 300.0, 24.001);
        let different_font = realtime_edt_request_key("other-test-family", '(', 300.0, 24.0);

        let unique = [
            duplicate_a,
            duplicate_b,
            different_size,
            different_spread,
            different_font,
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), 4);
    }

    #[test]
    fn substituted_text_tier_preserves_primary_sample_grid() {
        use crate::sdf::atlas::SdfAtlasGlyphManifest;
        use crate::sdf::tile::{Point2, SdfCommandMaterial, SdfDrawCommand, SdfMaterial};

        let primary_glyph = SdfAtlasGlyphManifest {
            codepoint: 0x25a0,
            page: 0,
            rect: [0, 0, 73, 73],
            plane_bearing: [7.5, 55.203_125],
            plane_size: [60.0, 60.0],
            plane_advance_x: 75.0,
        };
        let target_glyph = SdfAtlasGlyphManifest {
            codepoint: 0x25a0,
            page: 1,
            rect: [0, 0, 217, 217],
            plane_bearing: [22.5, 165.593_75],
            plane_size: [180.0, 180.0],
            plane_advance_x: 225.0,
        };
        let command = |atlas_set, atlas_page, atlas_rect| SdfDrawCommand {
            kind: crate::sdf::tile::SdfPrimitiveKind::Text,
            atlas_set,
            atlas_page,
            atlas_rect,
            quad: [
                Point2::new(0.0, 0.0),
                Point2::new(144_000.0, 0.0),
                Point2::new(144_000.0, 2_400.0),
                Point2::new(0.0, 2_400.0),
            ],
            device_clip: None,
            material: SdfCommandMaterial::Text(SdfMaterial::default()),
        };
        let primary_command = command(0, 0, primary_glyph.rect);
        let target_command = command(1, 1, target_glyph.rect);
        let primary_grid = TextSdfSamplingGrid::new(primary_command, &primary_glyph, 75.0, 6.0)
            .expect("valid primary grid");
        let aligned = align_substituted_text_sdf_command(
            primary_grid,
            target_command,
            &target_glyph,
            225.0,
            18.0,
        )
        .expect("valid substituted grid");

        let close = |actual: f32, expected: f32| {
            assert!((actual - expected).abs() < 0.01, "{actual} != {expected}");
        };
        close(aligned.quad[0].x, 144_000.0 / 219.0);
        close(aligned.quad[1].x, 144_000.0 * 218.0 / 219.0);
        close(aligned.quad[0].y, 2_400.0 * 2.0 / 219.0);
        close(aligned.quad[3].y, 2_400.0);
        assert_eq!(aligned.atlas_set, 1);
        assert_eq!(aligned.atlas_page, 1);
        assert_eq!(aligned.atlas_rect, target_glyph.rect);
    }

    #[test]
    fn lossless_crop_rejects_short_row_stride() {
        let result = crop_pixels_lossless(&[0; 16], 12, 4, 1, 0, 0, 1, 1);
        assert!(matches!(result, Err(_)));
    }

    #[test]
    fn lossless_crop_rejects_out_of_bounds_rect() {
        let result = crop_pixels_lossless(&[0; 16], 16, 4, 1, 4, 0, 1, 1);
        assert!(matches!(result, Err(_)));
    }

    #[test]
    fn lossless_crop_rejects_zero_sized_rect() {
        let result = crop_pixels_lossless(&[0; 16], 16, 4, 1, 0, 0, 0, 1);
        assert!(matches!(result, Err(_)));
    }

    #[test]
    fn direct_opaque_bounds_match_scalar_and_vector_paths() {
        let surface_width = 4u32;
        let surface_height = 3u32;
        let row_bytes = 20usize;
        let mut source = vec![0u8; row_bytes * surface_height as usize];
        let samples = [
            (1u32, 1u32, [10u8, 20, 30, 40]),
            (2, 1, [50, 60, 70, 80]),
            (1, 2, [90, 100, 110, 120]),
            (2, 2, [130, 140, 150, 160]),
        ];
        for (x, y, rgba) in samples {
            let offset = y as usize * row_bytes + x as usize * 4;
            source[offset..offset + 4].copy_from_slice(&rgba);
        }
        assert_eq!(
            find_opaque_bounds(&source, surface_width, surface_height, row_bytes),
            (1, 1, 2, 2)
        );
        assert_eq!(
            find_opaque_bounds_scalar(&source, surface_width, surface_height, row_bytes),
            (1, 1, 2, 2)
        );
        #[cfg(target_arch = "x86_64")]
        if std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
            && std::arch::is_x86_feature_detected!("bmi2")
        {
            assert_eq!(
                unsafe {
                    find_opaque_bounds_avx512(&source, surface_width, surface_height, row_bytes)
                },
                (1, 1, 2, 2)
            );
        }
    }

    #[test]
    fn backend_cache_identity_is_stable_and_config_sensitive() {
        use std::sync::Arc;

        let renderer = CustomProfileRenderer::new(Arc::new(NullProvider));
        let default = crate::profile_backend::ProfileBackendConfig::default();
        let first = renderer
            .profile_backend_cache_identity(&default)
            .expect("default backend identity");
        let repeated = renderer
            .profile_backend_cache_identity(&default)
            .expect("repeated backend identity");
        let changed = renderer
            .profile_backend_cache_identity(&crate::profile_backend::ProfileBackendConfig {
                tile_width: 64,
                ..default
            })
            .expect("changed backend identity");

        assert_eq!(first, repeated);
        assert_ne!(first, changed);
        assert!(first.starts_with("profile-backend-"));
    }

    #[cfg(feature = "animation-export")]
    #[test]
    fn oversized_animation_sdf_surface_is_detected_before_plan_or_surface_build() {
        let (surface_bytes, retained_peak_bytes) =
            animation_sdf_retained_surface_bytes(46_290, 29_984).unwrap();
        assert_eq!(surface_bytes, 5_551_837_440);
        assert_eq!(retained_peak_bytes, 11_103_674_880);
        assert!(retained_peak_bytes > 64 * 1024 * 1024);

        let (_, ordinary_peak_bytes) = animation_sdf_retained_surface_bytes(1_830, 812).unwrap();
        assert!(ordinary_peak_bytes < 64 * 1024 * 1024);
    }

    #[cfg(feature = "animation-export")]
    #[test]
    fn deferred_animation_window_keeps_complete_source_across_viewport_changes() {
        use sha2::Digest as _;

        let root = tempfile::tempdir().unwrap();
        let page_path = root.path().join("page-000.r8swz");
        let mut page = vec![0u8; crate::sdf::atlas::SWIZZLED_PAGE_HEADER_BYTES + 8 * 8];
        page[..crate::sdf::atlas::SWIZZLED_PAGE_MAGIC.len()]
            .copy_from_slice(crate::sdf::atlas::SWIZZLED_PAGE_MAGIC);
        page[12..16].copy_from_slice(&crate::sdf::atlas::SWIZZLED_PAGE_VERSION.to_le_bytes());
        page[16..20].copy_from_slice(&8u32.to_le_bytes());
        page[20..24].copy_from_slice(&8u32.to_le_bytes());
        page[24..28].copy_from_slice(&8u32.to_le_bytes());
        page[28..32].copy_from_slice(&8u32.to_le_bytes());
        page[crate::sdf::atlas::SWIZZLED_PAGE_HEADER_BYTES..].fill(128);
        std::fs::write(&page_path, &page).unwrap();
        let manifest = crate::sdf::atlas::SdfAtlasManifest {
            schema: crate::sdf::atlas::ATLAS_MANIFEST_SCHEMA.into(),
            generator_contract: "deferred-window-test".into(),
            font_family: "deferred-window-test".into(),
            font_sha256: "00".repeat(32),
            point_size: 8.0,
            spread: 1.0,
            pages: vec![crate::sdf::atlas::SdfAtlasPageManifest {
                file: "page-000.r8swz".into(),
                width: 8,
                height: 8,
                file_sha256: hex::encode(sha2::Sha256::digest(&page)),
            }],
            glyphs: Vec::new(),
            generation: crate::sdf::atlas::SdfAtlasGenerationReport {
                cmap_codepoint_count: 0,
                requested_codepoint_count: 0,
                generated_glyph_count: 0,
                failed_glyph_count: 0,
                analytic_fallback_count: 0,
                page_width: 8,
                page_height: 8,
                gutter: 0,
                failures: Vec::new(),
                analytic_fallback_codepoints: Vec::new(),
            },
        };
        let manifest_path = root.path().join("manifest.json");
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let atlas =
            std::sync::Arc::new(crate::sdf::atlas::MappedSdfAtlas::open(manifest_path).unwrap());
        let mut atlases = crate::sdf::atlas::MappedSdfAtlasSet::new();
        let atlas_set = atlases.insert(atlas).unwrap();
        let command = crate::sdf::tile::SdfDrawCommand {
            kind: crate::sdf::tile::SdfPrimitiveKind::Text,
            atlas_set,
            atlas_page: 0,
            atlas_rect: [0, 0, 8, 8],
            quad: [
                crate::sdf::tile::Point2 { x: 0.0, y: 0.0 },
                crate::sdf::tile::Point2 {
                    x: 10_000.0,
                    y: 0.0,
                },
                crate::sdf::tile::Point2 {
                    x: 10_000.0,
                    y: 10_000.0,
                },
                crate::sdf::tile::Point2 {
                    x: 0.0,
                    y: 10_000.0,
                },
            ],
            device_clip: None,
            material: crate::sdf::tile::SdfCommandMaterial::Text(
                crate::sdf::tile::SdfMaterial::default(),
            ),
        };
        let deferred = DeferredAnimationSdfLayer {
            commands: vec![command],
            text_atlases: std::sync::Arc::new(atlases),
            shape_atlas: None,
            runtime_text: Vec::new(),
            executor: SdfLayerCandidateExecutor::ScalarF32,
            tile_width: 32,
            tile_height: 32,
            x: 0,
            y: 0,
            width: 10_000,
            height: 10_000,
            dynamic: None,
        };
        let renderer = CustomProfileRenderer::new(std::sync::Arc::new(NullProvider));
        let first = renderer
            .render_deferred_animation_sdf_window(&deferred, 0, 0, 64, 64)
            .unwrap();
        let shifted = renderer
            .render_deferred_animation_sdf_window(&deferred, 9_000, 9_000, 64, 64)
            .unwrap();
        let restored = renderer
            .render_deferred_animation_sdf_window(&deferred, 0, 0, 64, 64)
            .unwrap();

        let first_rgba = first.pixels.clone();
        assert!(first_rgba.chunks_exact(4).any(|pixel| pixel[3] != 0));
        assert_eq!(shifted.pixels, first_rgba);
        assert_eq!(restored.pixels, first_rgba);
        assert_eq!(first.scratch_peak_bytes, 64 * 64 * 8);
        assert!(first.execution.plan_stats.tile_count <= 4);
    }

    /// A provider with no optional master-data entries keeps renderer setup independent of assets.
    struct NullProvider;

    impl MasterDataProvider for NullProvider {
        fn resolve_story_banner(&self, _story_type: &str, _story_id: i32) -> Option<String> {
            None
        }
        fn get_card(&self, _card_id: i32) -> Option<CardEntry> {
            None
        }
        fn resolve_color(&self, _color_id: i32) -> Option<ResolvedColor> {
            None
        }
        fn resolve_font(&self, _font_id: i32) -> Option<String> {
            None
        }
        fn resolve_stamp(&self, _stamp_id: i32) -> Option<String> {
            None
        }
        fn resolve_resource(&self, _res_type: &str, _id: i32) -> Option<ResourceInfo> {
            None
        }
        fn resolve_honor(&self, _honor_id: i32, _honor_level: i32) -> Option<ResolvedHonor> {
            None
        }
        fn get_bonds_honor(&self, _id: i32) -> Option<BondsHonorEntry> {
            None
        }
        fn get_bonds_honor_word(&self, _word_id: i64) -> Option<BondsHonorWordEntry> {
            None
        }
        fn get_honor(&self, _honor_id: i32) -> Option<HonorEntry> {
            None
        }
        fn resolve_unit_vs_sd(&self, _self_id: i32, _partner_id: i32) -> i32 {
            0
        }
        fn font_count(&self) -> usize {
            0
        }
        fn color_count(&self) -> usize {
            0
        }
    }

    #[cfg(feature = "skia-oracle")]
    fn default_object_data(layer: i32, visible: bool) -> ObjectData {
        ObjectData {
            layer,
            lock: false,
            position: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            rotation: Quaternion {
                w: 1.0,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            scale: Vec3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            },
            visible,
        }
    }

    /// 最小名片结构：
    /// 4 个 invisible shapes + 4 个 visible stamps + 4 个 invisible texts = 12 层
    #[cfg(feature = "skia-oracle")]
    fn issue5_simple_card() -> CustomProfileCard {
        CustomProfileCard {
            shapes: vec![
                crate::types::ShapeElement {
                    object_data: default_object_data(0, false),
                    alpha: 0.0,
                    color_id: 23,
                    id: 3,
                    outline_alpha: 1.0,
                    outline_color_id: 23,
                    outline_size: 0.37,
                },
                crate::types::ShapeElement {
                    object_data: default_object_data(1, false),
                    alpha: 1.0,
                    color_id: 23,
                    id: 3,
                    outline_alpha: 1.0,
                    outline_color_id: 23,
                    outline_size: 0.0,
                },
                crate::types::ShapeElement {
                    object_data: default_object_data(2, false),
                    alpha: 0.0,
                    color_id: 23,
                    id: 3,
                    outline_alpha: 1.0,
                    outline_color_id: 23,
                    outline_size: 0.37,
                },
                crate::types::ShapeElement {
                    object_data: default_object_data(3, false),
                    alpha: 0.0,
                    color_id: 23,
                    id: 3,
                    outline_alpha: 1.0,
                    outline_color_id: 23,
                    outline_size: 0.37,
                },
            ],
            stamps: vec![
                StampElement {
                    object_data: default_object_data(4, true),
                    id: 609,
                },
                StampElement {
                    object_data: default_object_data(5, true),
                    id: 179,
                },
                StampElement {
                    object_data: default_object_data(6, true),
                    id: 631,
                },
                StampElement {
                    object_data: default_object_data(7, true),
                    id: 514,
                },
            ],
            texts: vec![
                TextElement {
                    object_data: default_object_data(8, false),
                    color_id: 18,
                    font_id: 2,
                    line_spacing: 0.0,
                    outline_color_id: 18,
                    outline_size: 0.0,
                    size: 24.0,
                    text: "5.9-5.12".to_string(),
                    text_type: 513,
                },
                TextElement {
                    object_data: default_object_data(9, false),
                    color_id: 15,
                    font_id: 2,
                    line_spacing: 0.0,
                    outline_color_id: 15,
                    outline_size: 0.0,
                    size: 24.0,
                    text: "5.12-5.15".to_string(),
                    text_type: 513,
                },
                TextElement {
                    object_data: default_object_data(10, false),
                    color_id: 17,
                    font_id: 2,
                    line_spacing: 0.0,
                    outline_color_id: 17,
                    outline_size: 0.0,
                    size: 24.0,
                    text: "5.15-5.18".to_string(),
                    text_type: 513,
                },
                TextElement {
                    object_data: default_object_data(11, false),
                    color_id: 16,
                    font_id: 2,
                    line_spacing: 0.0,
                    outline_color_id: 16,
                    outline_size: 0.0,
                    size: 24.0,
                    text: "5.18-5.21".to_string(),
                    text_type: 513,
                },
            ],
            card_members: vec![],
            others: vec![],
            bonds_honors: vec![],
            honors: vec![],
            collections: vec![],
            generals: vec![],
            stand_members: vec![],
            general_backgrounds: vec![],
            story_backgrounds: vec![],
        }
    }
}
