//! Realtime EDT glyphs: oversized glyphs generated from their font during a
//! request, prepared once for a set of cards and installed for the current thread.

use super::capture::{capture_sdf_primitives, map_captured_sdf_commands, SdfCaptureKinds};
use super::CustomProfileRenderer;
use crate::assets::AssetStore;
use crate::masterdata::MasterData;
use crate::types::CustomProfileCard;
use std::sync::Arc;

impl CustomProfileRenderer {
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
        let mut telemetry = crate::telemetry::RealtimeEdtBatchTelemetry::default();
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

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct RealtimeEdtRequestKey {
    pub(super) font_family: String,
    pub(super) codepoint: u32,
    pub(super) point_size_bits: u32,
    pub(super) spread_bits: u32,
}

#[derive(Clone)]
pub(super) struct RealtimeEdtPreparedEntry {
    pub(super) atlas_set: u16,
    pub(super) manifest: crate::sdf::atlas::SdfAtlasGlyphManifest,
    pub(super) point_size: f32,
    pub(super) spread: f32,
}

pub(crate) struct RealtimeEdtPreparedBatch {
    pub(super) pages: Vec<crate::sdf::tile::RuntimeTextSdfPage>,
    pub(super) entries: std::collections::BTreeMap<RealtimeEdtRequestKey, RealtimeEdtPreparedEntry>,
    pub(crate) glyphs: Vec<crate::telemetry::RealtimeEdtGlyphTelemetry>,
    pub(crate) telemetry: crate::telemetry::RealtimeEdtBatchTelemetry,
}

thread_local! {
    static ACTIVE_REALTIME_EDT_BATCH: std::cell::RefCell<Option<Arc<RealtimeEdtPreparedBatch>>> =
        const { std::cell::RefCell::new(None) };
}

pub(super) fn active_realtime_edt_batch() -> Option<Arc<RealtimeEdtPreparedBatch>> {
    ACTIVE_REALTIME_EDT_BATCH.with(|slot| slot.borrow().clone())
}

pub(super) fn realtime_edt_request_key(
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

#[cfg(feature = "parallel")]
pub(super) fn realtime_edt_pool() -> &'static rayon::ThreadPool {
    use std::sync::OnceLock;
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::env::var("SEKAI_PROFILE_REALTIME_EDT_THREADS")
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

pub(super) fn realtime_edt_sampling_spread(
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
