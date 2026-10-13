//! [`CustomProfileRenderer`]: renders custom profile pages and animations.

#[cfg(feature = "animation-export")]
mod animation_layer;
mod authored;
mod capture;
mod debug;
mod fallback;
mod occlusion;
mod page;
mod prewarm;
mod realtime_edt;
mod surface;

#[cfg(feature = "animation-export")]
pub(crate) use animation_layer::{
    AnimationLayerRaster, DeferredAnimationSdfLayer, RenderedDeferredAnimationWindow,
};
pub use prewarm::PrewarmReport;
#[cfg(feature = "animation-export")]
pub(crate) use surface::SurfaceOutput;

use crate::assets::AssetStore;
use crate::masterdata::{MasterData, MasterDataProvider};
use crate::render_config::{sdf_simd_available, RenderCapabilities, RenderConfig};
use crate::types::CustomProfileCard;
use std::sync::Mutex;
use std::sync::{Arc, RwLock};

/// Immutable render-object generation pinned for one external render request.
/// Cloning this token clones only the `Arc`, never the mapped pages.
#[derive(Clone, Default)]
pub struct RenderObjectGenerationPin {
    store: Option<Arc<crate::render_object::MappedRenderObjectStore>>,
}

impl RenderObjectGenerationPin {
    /// Pins a store the caller built for one request, for renderers that are
    /// shared across requests whose render objects differ.
    pub fn new(store: Arc<crate::render_object::MappedRenderObjectStore>) -> Self {
        Self { store: Some(store) }
    }

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

/// Renders custom profile pages and animations.
///
/// A renderer holds the master data, the asset store and the installed
/// atlases and render objects; one renderer serves any number of concurrent
/// renders.
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
    #[cfg(feature = "jpeg-turbo")]
    jpeg_yuv420_scratch: Mutex<Vec<u8>>,
    /// Whether a glyph too large for any installed atlas tier may be generated
    /// during the request instead of falling back to an installed tier.
    realtime_oversized_glyph_generation: bool,
}

impl CustomProfileRenderer {
    /// A renderer over `provider` with nothing installed.
    pub fn new(provider: Arc<dyn MasterDataProvider>) -> Self {
        let md = MasterData::new(Arc::clone(&provider));
        tracing::info!(
            colors = md.color_count(),
            fonts = md.font_count(),
            "custom profile renderer created"
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
            #[cfg(feature = "jpeg-turbo")]
            jpeg_yuv420_scratch: Mutex::new(Vec::new()),
            realtime_oversized_glyph_generation: true,
        }
    }

    /// Installs the asset store images and fonts are read from.
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

    /// Installs every atlas in `dir`, laid out as [`crate::atlas_dir::AtlasDir`]
    /// describes. See [`crate::atlas_dir::AtlasDir::open`] for the checks made.
    pub fn with_atlas_dir(self, dir: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let installed = crate::atlas_dir::AtlasDir::new(dir.as_ref()).open()?;
        self.with_installed_atlases(installed)
    }

    /// Installs atlases opened from an [`crate::atlas_dir::AtlasDir`].
    ///
    /// With tier atlases installed, a glyph larger than every tier samples
    /// the largest tier that holds it; without tiers, such a glyph is
    /// generated from its font during the request.
    pub fn with_installed_atlases(
        mut self,
        atlases: crate::atlas_dir::InstalledAtlases,
    ) -> Result<Self, String> {
        let realtime = atlases.tiers.is_empty();
        for atlas in atlases.text.into_iter().chain(atlases.tiers) {
            self = self
                .with_sdf_atlas(atlas)
                .map_err(|error| error.to_string())?;
        }
        if let Some(shape) = atlases.shape {
            self = self.with_shape_sdf_atlas(shape);
        }
        if let Some(fallback) = atlases.fallback {
            self = self.with_profile_fallback_sdf_cache(fallback);
        }
        Ok(self.with_realtime_oversized_glyph_generation(realtime))
    }

    /// The resources this renderer has installed.
    pub fn capabilities(&self) -> RenderCapabilities {
        RenderCapabilities {
            simd: sdf_simd_available(),
            text_atlases: self.sdf_atlases.load().len(),
            shape_atlas: self.shape_sdf_atlas.is_some(),
            render_objects: self.render_object_generations.is_some(),
            fallback_glyphs: self.profile_fallback_sdf_cache.is_some(),
        }
    }

    /// Whether a glyph drawn larger than every installed atlas tier is
    /// generated from its font during the request (`true`, the default) or
    /// sampled from the largest installed tier that holds it.
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

    #[cfg(feature = "jpeg-turbo")]
    fn take_jpeg_yuv420_scratch(&self) -> Vec<u8> {
        let mut scratch = self
            .jpeg_yuv420_scratch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *scratch)
    }

    #[cfg(feature = "jpeg-turbo")]
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

    /// An identity for caching rendered pages: it changes whenever `config`,
    /// an installed atlas or the published render-object store changes.
    pub fn cache_identity(&self, config: &RenderConfig) -> Result<String, String> {
        let generation = self.pin_render_object_generation();
        self.cache_identity_pinned(config, &generation)
    }

    /// [`Self::cache_identity`] for the render objects `generation` pins.
    pub fn cache_identity_pinned(
        &self,
        config: &RenderConfig,
        generation: &RenderObjectGenerationPin,
    ) -> Result<String, String> {
        use sha2::{Digest, Sha256};

        let mut digest = Sha256::new();
        digest.update(crate::telemetry::PROFILE_RENDER_CACHE_SCHEMA.as_bytes());
        digest.update(
            serde_json::to_vec(config)
                .map_err(|error| format!("serialize render config: {error}"))?,
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
        Ok(format!("profile-render-{}", &identity[..16]))
    }

    /// Replaces the master data. Renders already running keep the master
    /// data they started with.
    pub fn swap_masterdata(&self, new_provider: Arc<dyn MasterDataProvider>) {
        let md = MasterData::new(Arc::clone(&new_provider));
        tracing::info!(
            colors = md.color_count(),
            fonts = md.font_count(),
            "master data replaced"
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

    /// The current master data.
    pub fn masterdata(&self) -> MasterData {
        self.snapshot()
    }

    /// The installed asset store.
    pub fn assets(&self) -> Option<&Arc<AssetStore>> {
        self.assets.as_ref()
    }

    /// Lists the colours, fonts, shapes and icons `card` refers to that the
    /// master data does not define. Such references draw with defaults or not
    /// at all.
    pub fn validate_card(&self, card: &CustomProfileCard) -> Vec<String> {
        let md = self.snapshot();
        let mut warnings = Vec::new();
        for (i, text) in card.texts.iter().enumerate() {
            if md.resolve_color(text.color_id).is_none() {
                warnings.push(format!(
                    "texts[{i}]: colorId {} is not in the master data",
                    text.color_id
                ));
            }
            if md.resolve_font(text.font_id).is_none() {
                warnings.push(format!(
                    "texts[{i}]: fontId {} is not in the master data",
                    text.font_id
                ));
            }
        }
        for (i, shape) in card.shapes.iter().enumerate() {
            if md.resolve_color(shape.color_id).is_none() {
                warnings.push(format!(
                    "shapes[{i}]: colorId {} is not in the master data",
                    shape.color_id
                ));
            }
            if md.resolve_resource("shape", shape.id).is_none() {
                warnings.push(format!(
                    "shapes[{i}]: shape id {} is not in the master data",
                    shape.id
                ));
            }
        }
        for (i, icon) in card.user_interface_icons.iter().enumerate() {
            if md.resolve_color(icon.color_id).is_none() {
                warnings.push(format!(
                    "userInterfaceIcons[{i}]: colorId {} is not in the master data",
                    icon.color_id
                ));
            }
            if md
                .resolve_resource("user_interface_icon", icon.id)
                .is_none()
            {
                warnings.push(format!(
                    "userInterfaceIcons[{i}]: icon id {} is not in the master data",
                    icon.id
                ));
            }
        }
        if !warnings.is_empty() {
            tracing::warn!(
                count = warnings.len(),
                "the card refers to entries the master data lacks"
            );
        }
        warnings
    }
}

fn elapsed_ns(started: std::time::Instant) -> u64 {
    started.elapsed().as_nanos().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests;
