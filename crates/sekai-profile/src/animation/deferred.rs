//! Deferred SDF layers: the source window a dirty rect needs and the tile cache
//! that serves it.

use super::compositor::{LayerFrameState, PixelRect, RasterLayer};
use super::AnimationError;

/// Source-space cache for deferred SDF layers. The cache stores only pixels
/// that were actually requested by a dirty window; it never changes the
/// layer's source bounds or visibility semantics.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct DeferredSdfTileKey {
    pub(super) layer_index: usize,
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) width: u32,
    pub(super) height: u32,
}

pub(super) struct DeferredSdfTileCacheEntry {
    pixels: Vec<u8>,
    bytes: usize,
}

pub(super) struct DeferredSdfTileCache {
    pub(super) budget_bytes: usize,
    pub(super) resident_bytes: usize,
    pub(super) peak_bytes: usize,
    pub(super) entries: std::collections::HashMap<DeferredSdfTileKey, DeferredSdfTileCacheEntry>,
    order: std::collections::VecDeque<DeferredSdfTileKey>,
}

impl DeferredSdfTileCache {
    const TILE_EDGE: i32 = 128;

    pub(super) fn new(budget_bytes: usize) -> Self {
        Self {
            budget_bytes,
            resident_bytes: 0,
            peak_bytes: 0,
            entries: std::collections::HashMap::new(),
            order: std::collections::VecDeque::new(),
        }
    }

    fn enabled(&self) -> bool {
        self.budget_bytes != 0
    }

    pub(super) fn get(&mut self, key: DeferredSdfTileKey) -> Option<Vec<u8>> {
        let pixels = self.entries.get(&key)?.pixels.clone();
        if let Some(index) = self.order.iter().position(|value| *value == key) {
            self.order.remove(index);
        }
        self.order.push_back(key);
        Some(pixels)
    }

    pub(super) fn insert(&mut self, key: DeferredSdfTileKey, pixels: Vec<u8>) {
        if !self.enabled() {
            return;
        }
        let bytes = pixels.len();
        if bytes == 0 || bytes > self.budget_bytes {
            return;
        }
        if let Some(previous) = self.entries.remove(&key) {
            self.resident_bytes = self.resident_bytes.saturating_sub(previous.bytes);
            if let Some(index) = self.order.iter().position(|value| *value == key) {
                self.order.remove(index);
            }
        }
        while self.resident_bytes.saturating_add(bytes) > self.budget_bytes {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.resident_bytes = self.resident_bytes.saturating_sub(entry.bytes);
            }
        }
        if self.resident_bytes.saturating_add(bytes) > self.budget_bytes {
            return;
        }
        self.resident_bytes = self.resident_bytes.saturating_add(bytes);
        self.peak_bytes = self.peak_bytes.max(self.resident_bytes);
        self.order.push_back(key);
        self.entries
            .insert(key, DeferredSdfTileCacheEntry { pixels, bytes });
    }
}

/// Computes the source-space window of a deferred layer a dirty device rect
/// needs, in layer coordinates, clamped to the layer's bounds. Returns `None`
/// when the dirty rect cannot reach the layer.
pub(super) fn deferred_source_window(
    layer: &RasterLayer,
    state: &LayerFrameState,
    dirty: PixelRect,
    scale: f32,
) -> Option<PixelRect> {
    if scale <= 0.0 || !scale.is_finite() || dirty.is_empty() {
        return None;
    }
    const GUARD: i32 = 2;
    let dx = state.destination.left() / scale - layer.x;
    let dy = state.destination.top() / scale - layer.y;
    let layer_left = layer.x.floor() as i32;
    let layer_top = layer.y.floor() as i32;
    let layer_right = layer_left.checked_add(i32::try_from(layer.width).ok()?)?;
    let layer_bottom = layer_top.checked_add(i32::try_from(layer.height).ok()?)?;
    let left = ((dirty.left as f32 / scale - dx).floor() as i32 - GUARD).max(layer_left);
    let top = ((dirty.top as f32 / scale - dy).floor() as i32 - GUARD).max(layer_top);
    let right = ((dirty.right as f32 / scale - dx).ceil() as i32 + GUARD).min(layer_right);
    let bottom = ((dirty.bottom as f32 / scale - dy).ceil() as i32 + GUARD).min(layer_bottom);
    let window = PixelRect {
        left,
        top,
        right,
        bottom,
    };
    (!window.is_empty()).then_some(window)
}

fn record_deferred_window_execution(
    render_telemetry: &mut Option<crate::telemetry::RenderTelemetry>,
    rendered: &crate::renderer::RenderedDeferredAnimationWindow,
) {
    if let Some(telemetry) = render_telemetry.as_mut() {
        telemetry.bytes.scratch_peak_bytes = telemetry
            .bytes
            .scratch_peak_bytes
            .max(rendered.scratch_peak_bytes as u64);
    }
}

/// Rasterizes one window of a deferred layer, serving tiles from the LRU cache
/// when it is enabled and assembling them into a contiguous window buffer by
/// exact sub-rectangle copy. When the cache is disabled the window is rendered
/// whole.
pub(super) fn render_deferred_animation_window_cached(
    renderer: &crate::renderer::CustomProfileRenderer,
    layer: &crate::renderer::DeferredAnimationSdfLayer,
    window: PixelRect,
    layer_index: usize,
    cache: &mut DeferredSdfTileCache,
    render_telemetry: &mut Option<crate::telemetry::RenderTelemetry>,
) -> Result<crate::renderer::RenderedDeferredAnimationWindow, AnimationError> {
    let width = u32::try_from(window.right - window.left)
        .map_err(|_| AnimationError::Internal("deferred animation window width overflow"))?;
    let height = u32::try_from(window.bottom - window.top)
        .map_err(|_| AnimationError::Internal("deferred animation window height overflow"))?;
    if !cache.enabled() {
        let rendered = renderer.render_deferred_animation_sdf_window(
            layer,
            window.left,
            window.top,
            width,
            height,
        )?;
        record_deferred_window_execution(render_telemetry, &rendered);
        return Ok(rendered);
    }

    let mut staging = vec![0u8; width as usize * height as usize * 4];
    let layer_left = layer.x;
    let layer_top = layer.y;
    let layer_right = layer
        .x
        .checked_add(
            i32::try_from(layer.width)
                .map_err(|_| AnimationError::Internal("deferred layer width overflow"))?,
        )
        .ok_or(AnimationError::Internal("deferred layer right overflow"))?;
    let layer_bottom = layer
        .y
        .checked_add(
            i32::try_from(layer.height)
                .map_err(|_| AnimationError::Internal("deferred layer height overflow"))?,
        )
        .ok_or(AnimationError::Internal("deferred layer bottom overflow"))?;
    let edge = DeferredSdfTileCache::TILE_EDGE;
    let mut tile_top = layer_top + (window.top - layer_top).div_euclid(edge) * edge;
    let mut scratch_peak_bytes = width as usize * height as usize * 8;
    while tile_top < window.bottom {
        let tile_bottom = tile_top.saturating_add(edge).min(layer_bottom);
        let mut tile_left = layer_left + (window.left - layer_left).div_euclid(edge) * edge;
        while tile_left < window.right {
            let tile_right = tile_left.saturating_add(edge).min(layer_right);
            let key = DeferredSdfTileKey {
                layer_index,
                x: tile_left,
                y: tile_top,
                width: u32::try_from(tile_right - tile_left)
                    .map_err(|_| AnimationError::Internal("deferred cache tile width overflow"))?,
                height: u32::try_from(tile_bottom - tile_top)
                    .map_err(|_| AnimationError::Internal("deferred cache tile height overflow"))?,
            };
            let cached = cache.get(key);
            if let Some(telemetry) = render_telemetry.as_mut() {
                if cached.is_some() {
                    telemetry.caches.layer.hits = telemetry.caches.layer.hits.saturating_add(1);
                } else {
                    telemetry.caches.layer.misses = telemetry.caches.layer.misses.saturating_add(1);
                }
            }
            let pixels = if let Some(pixels) = cached {
                pixels
            } else {
                let rendered = renderer.render_deferred_animation_sdf_window(
                    layer, key.x, key.y, key.width, key.height,
                )?;
                scratch_peak_bytes = scratch_peak_bytes.max(rendered.scratch_peak_bytes);
                record_deferred_window_execution(render_telemetry, &rendered);
                let pixels = rendered.pixels.clone();
                cache.insert(key, pixels.clone());
                pixels
            };
            let visible_left = tile_left.max(window.left);
            let visible_top = tile_top.max(window.top);
            let visible_right = tile_right.min(window.right);
            let visible_bottom = tile_bottom.min(window.bottom);
            let tile_width = (tile_right - tile_left) as usize;
            let window_row_bytes = width as usize * 4;
            let visible_width = (visible_right - visible_left) as usize;
            let copy_bytes = visible_width * 4;
            for y in visible_top..visible_bottom {
                let source = (y - tile_top) as usize * tile_width * 4
                    + (visible_left - tile_left) as usize * 4;
                let destination = (y - window.top) as usize * window_row_bytes
                    + (visible_left - window.left) as usize * 4;
                staging[destination..destination + copy_bytes]
                    .copy_from_slice(&pixels[source..source + copy_bytes]);
            }
            tile_left = tile_left.saturating_add(edge);
        }
        tile_top = tile_top.saturating_add(edge);
    }

    Ok(crate::renderer::RenderedDeferredAnimationWindow {
        pixels: staging,
        x: window.left,
        y: window.top,
        width,
        height,
        scratch_peak_bytes,
    })
}
