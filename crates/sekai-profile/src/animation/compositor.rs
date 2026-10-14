//! Frame compositing: layer rasters drawn onto a premultiplied canvas,
//! dirty-region redraws between frames, and readback to the RGBA the encoders consume.

use super::deferred::{
    deferred_source_window, render_deferred_animation_window_cached, DeferredSdfTileCache,
};
use super::elapsed_ns;
use super::encode::frame_byte_len;
use super::AnimationError;

pub(super) struct RasterLayer {
    pub(super) dynamic_layer_id: Option<sekai_profile_core::LayerId>,
    /// What the layer carries: retained pixels, or commands to produce them.
    pub(super) content: RasterLayerContent,
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) width: u32,
    pub(super) height: u32,
}

/// A layer's payload: pixels retained for the whole export, or the commands a
/// per-frame window is rasterized from.
pub(super) enum RasterLayerContent {
    /// Tight premultiplied RGBA8, `width * 4` bytes per row.
    Pixels(Vec<u8>),
    /// Commands in canvas coordinates, rasterized per frame.
    Deferred(crate::renderer::DeferredAnimationSdfLayer),
}

/// Axis-aligned float rectangle in device space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FrameRect {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl FrameRect {
    pub(super) fn from_xywh(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            left: x,
            top: y,
            right: x + width,
            bottom: y + height,
        }
    }

    pub(super) fn left(self) -> f32 {
        self.left
    }

    pub(super) fn top(self) -> f32 {
        self.top
    }

    fn right(self) -> f32 {
        self.right
    }

    fn bottom(self) -> f32 {
        self.bottom
    }
}

/// Opaque white, the animation canvas ground.
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// Fills `rect` of a premultiplied RGBA8 canvas with opaque white, replacing
/// whatever was there. This is the `BlendMode::Src` clear the dirty-region
/// redraw performs before recompositing.
fn fill_white(canvas: &mut [u8], canvas_width: u32, rect: PixelRect) {
    if rect.is_empty() {
        return;
    }
    let row_bytes = canvas_width as usize * 4;
    for y in rect.top..rect.bottom {
        let start = y as usize * row_bytes + rect.left as usize * 4;
        let end = start + (rect.right - rect.left) as usize * 4;
        for pixel in canvas[start..end].as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&WHITE);
        }
    }
}

/// Premultiplied RGBA8 pixels, `width * 4` bytes per row.
#[derive(Clone, Copy)]
pub(super) struct PixelsRef<'a> {
    pub(super) pixels: &'a [u8],
    pub(super) width: u32,
    pub(super) height: u32,
}

/// Premultiplied RGBA8 pixels being drawn into, `width * 4` bytes per row.
pub(super) struct PixelsMut<'a> {
    pub(super) pixels: &'a mut [u8],
    pub(super) width: u32,
    pub(super) height: u32,
}

/// Draws a premultiplied RGBA8 source into `destination` over the device
/// rectangle `rect`, source-over, sampling nearest.
///
/// The device coverage is the aliased rounding of each edge, and each covered
/// pixel takes the source texel its center maps to — the same rule the image
/// compositor uses for an axis-aligned nearest-sampled command. `clip`
/// restricts the write without moving the sampling grid, so a dirty-region
/// redraw produces the pixels a full redraw would.
pub(super) fn blit_layer(
    destination: PixelsMut<'_>,
    source: PixelsRef<'_>,
    rect: FrameRect,
    clip: PixelRect,
) {
    let PixelsMut {
        pixels: destination,
        width: canvas_width,
        height: canvas_height,
    } = destination;
    let PixelsRef {
        pixels: source,
        width: source_width,
        height: source_height,
    } = source;
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0.0 || height <= 0.0 || source_width == 0 || source_height == 0 {
        return;
    }
    let x0 = ((rect.left + 0.5).floor() as i32).max(clip.left).max(0);
    let y0 = ((rect.top + 0.5).floor() as i32).max(clip.top).max(0);
    let x1 = ((rect.right + 0.5).floor() as i32)
        .min(clip.right)
        .min(canvas_width as i32);
    let y1 = ((rect.bottom + 0.5).floor() as i32)
        .min(clip.bottom)
        .min(canvas_height as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let u_scale = source_width as f32 / width;
    let v_scale = source_height as f32 / height;
    let row_bytes = canvas_width as usize * 4;
    let source_row_bytes = source_width as usize * 4;
    for y in y0..y1 {
        // Nearest sampling: map the device pixel center into texel coordinates,
        // then take the texel whose center is closest, with an exact tie going
        // to the lower index — the reference sampler's rounding.
        let v = ((y as f32 + 0.5 - rect.top) * v_scale - 1.0).ceil() as i32;
        let v = v.clamp(0, source_height as i32 - 1) as usize;
        let source_row = v * source_row_bytes;
        let destination_row = y as usize * row_bytes;
        for x in x0..x1 {
            let u = ((x as f32 + 0.5 - rect.left) * u_scale - 1.0).ceil() as i32;
            let u = u.clamp(0, source_width as i32 - 1) as usize;
            let si = source_row + u * 4;
            let alpha = source[si + 3];
            if alpha == 0 {
                continue;
            }
            let di = destination_row + x as usize * 4;
            if alpha == u8::MAX {
                destination[di..di + 4].copy_from_slice(&source[si..si + 4]);
                continue;
            }
            // Source-over on premultiplied bytes with the 255->256 alpha
            // promotion: `out = s + d - ((d * (a + 1)) >> 8)`. Reproduces the
            // reference blitter exactly; see the parity test.
            let scale = u32::from(alpha) + 1;
            for channel in 0..4 {
                let over = u32::from(source[si + channel]);
                let under = u32::from(destination[di + channel]);
                destination[di + channel] = (over + under - ((under * scale) >> 8)).min(255) as u8;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct PixelRect {
    pub(super) left: i32,
    pub(super) top: i32,
    pub(super) right: i32,
    pub(super) bottom: i32,
}

impl PixelRect {
    pub(super) fn full(width: u32, height: u32) -> Self {
        Self {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        }
    }

    pub(super) fn is_empty(self) -> bool {
        self.left >= self.right || self.top >= self.bottom
    }

    fn area(self) -> u64 {
        if self.is_empty() {
            return 0;
        }
        (self.right - self.left) as u64 * (self.bottom - self.top) as u64
    }

    pub(super) fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    fn intersects(self, other: Self) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }

    fn contains(self, other: Self) -> bool {
        !other.is_empty()
            && self.left <= other.left
            && self.top <= other.top
            && self.right >= other.right
            && self.bottom >= other.bottom
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LayerFrameState {
    pub(super) visible: bool,
    pub(super) destination: FrameRect,
    pub(super) pixel_bounds: PixelRect,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct FrameCompositeTelemetry {
    pub(super) full_composites: u64,
    pub(super) dirty_composites: u64,
    pub(super) dirty_regions: u64,
    pub(super) reused_composites: u64,
    pub(super) dirty_pixels: u64,
    pub(super) composite_ns: u64,
    pub(super) readback_ns: u64,
    pub(super) readback_bytes: u64,
    pub(super) deferred_cache_peak_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum FrameCompositeUpdate {
    Full,
    Dirty(Vec<u32>),
    Reused,
}

pub(super) struct AnimationFrameCompositor<'a> {
    renderer: Option<&'a crate::renderer::CustomProfileRenderer>,
    /// Premultiplied RGBA8 canvas the frames composite into.
    canvas: Vec<u8>,
    width: u32,
    height: u32,
    scale: f32,
    previous_index: Option<u32>,
    previous_states: Vec<LayerFrameState>,
    rgba: Vec<u8>,
    readback_scratch: Vec<u8>,
    changed_macroblock_flags: Vec<u8>,
    pub(super) telemetry: FrameCompositeTelemetry,
    pub(super) render_telemetry: Option<crate::telemetry::RenderTelemetry>,
    pub(super) deferred_cache: DeferredSdfTileCache,
}

#[derive(Default)]
struct DirtyRegionSet {
    regions: Vec<PixelRect>,
}

impl DirtyRegionSet {
    fn add(&mut self, mut region: PixelRect) {
        if region.is_empty()
            || self
                .regions
                .iter()
                .copied()
                .any(|value| value.contains(region))
        {
            return;
        }
        self.regions.retain(|value| !region.contains(*value));
        loop {
            let Some(index) = self.regions.iter().position(|value| {
                let union = value.union(region);
                union.area() <= value.area().saturating_add(region.area())
            }) else {
                break;
            };
            region = region.union(self.regions.swap_remove(index));
            self.regions.retain(|value| !region.contains(*value));
        }
        self.regions.push(region);
    }

    fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    fn total_area(&self) -> u64 {
        self.regions.iter().map(|value| value.area()).sum()
    }
}

impl<'a> AnimationFrameCompositor<'a> {
    #[cfg(test)]
    pub(super) fn new(width: u32, height: u32, scale: f32) -> Result<Self, AnimationError> {
        Self::new_inner(None, width, height, scale, None, 0)
    }

    pub(super) fn new_for_export(
        renderer: &'a crate::renderer::CustomProfileRenderer,
        width: u32,
        height: u32,
        scale: f32,
        render_telemetry: Option<crate::telemetry::RenderTelemetry>,
        deferred_cache_budget: usize,
    ) -> Result<Self, AnimationError> {
        Self::new_inner(
            Some(renderer),
            width,
            height,
            scale,
            render_telemetry,
            deferred_cache_budget,
        )
    }

    fn new_inner(
        renderer: Option<&'a crate::renderer::CustomProfileRenderer>,
        width: u32,
        height: u32,
        scale: f32,
        render_telemetry: Option<crate::telemetry::RenderTelemetry>,
        deferred_cache_budget: usize,
    ) -> Result<Self, AnimationError> {
        let bytes = frame_byte_len(width, height)?;
        Ok(Self {
            renderer,
            canvas: vec![0; bytes],
            width,
            height,
            scale,
            previous_index: None,
            previous_states: Vec::new(),
            rgba: vec![0; bytes],
            readback_scratch: Vec::new(),
            changed_macroblock_flags: Vec::new(),
            telemetry: FrameCompositeTelemetry::default(),
            render_telemetry,
            deferred_cache: DeferredSdfTileCache::new(deferred_cache_budget),
        })
    }

    pub(super) fn composite(
        &mut self,
        index: u32,
        scene: &sekai_profile_core::Scene,
        layers: &[RasterLayer],
    ) -> Result<Vec<u8>, AnimationError> {
        let mut rgba = std::mem::take(&mut self.rgba);
        self.composite_into(index, scene, layers, &mut rgba)?;
        self.rgba = rgba;
        Ok(self.rgba.clone())
    }

    pub(super) fn composite_into(
        &mut self,
        index: u32,
        scene: &sekai_profile_core::Scene,
        layers: &[RasterLayer],
        rgba: &mut Vec<u8>,
    ) -> Result<FrameCompositeUpdate, AnimationError> {
        rgba.resize(self.width as usize * self.height as usize * 4, 0);
        let states = layers
            .iter()
            .map(|layer| layer_frame_state(scene, layer, self.width, self.height, self.scale))
            .collect::<Vec<_>>();
        self.composite_states(index, layers, states, rgba)
    }

    pub(super) fn composite_states(
        &mut self,
        index: u32,
        layers: &[RasterLayer],
        states: Vec<LayerFrameState>,
        rgba: &mut Vec<u8>,
    ) -> Result<FrameCompositeUpdate, AnimationError> {
        let sequential = self
            .previous_index
            .is_some_and(|previous| index == previous.saturating_add(1))
            && self.previous_states.len() == states.len();
        let composite_started = std::time::Instant::now();
        let (mut update, dirty_regions) = if !sequential {
            self.redraw_full_frame(layers, &states)?;
            self.telemetry.full_composites = self.telemetry.full_composites.saturating_add(1);
            (FrameCompositeUpdate::Full, None)
        } else {
            let mut dirty = DirtyRegionSet::default();
            for ((layer, previous), current) in layers
                .iter()
                .zip(self.previous_states.iter())
                .zip(states.iter())
            {
                if layer.dynamic_layer_id.is_some() && previous != current {
                    dirty.add(previous.pixel_bounds);
                    dirty.add(current.pixel_bounds);
                }
            }
            if dirty.is_empty() {
                self.telemetry.reused_composites =
                    self.telemetry.reused_composites.saturating_add(1);
                (FrameCompositeUpdate::Reused, None)
            } else {
                for region in &dirty.regions {
                    self.redraw_dirty_frame(layers, &states, *region)?;
                }
                self.telemetry.dirty_composites = self.telemetry.dirty_composites.saturating_add(1);
                self.telemetry.dirty_regions = self
                    .telemetry
                    .dirty_regions
                    .saturating_add(dirty.regions.len() as u64);
                self.telemetry.dirty_pixels = self
                    .telemetry
                    .dirty_pixels
                    .saturating_add(dirty.total_area());
                (FrameCompositeUpdate::Dirty(Vec::new()), Some(dirty.regions))
            }
        };
        self.telemetry.composite_ns = self
            .telemetry
            .composite_ns
            .saturating_add(elapsed_ns(composite_started));
        self.previous_index = Some(index);
        self.previous_states = states;

        let readback_started = std::time::Instant::now();
        let readback_bytes = match dirty_regions.as_deref() {
            Some(regions) => {
                let changed_macroblocks = read_canvas_regions_rgba_into_exact(
                    &self.canvas,
                    self.width,
                    self.height,
                    regions,
                    rgba,
                    &mut self.readback_scratch,
                    &mut self.changed_macroblock_flags,
                )?;
                update = FrameCompositeUpdate::Dirty(changed_macroblocks);
                regions.iter().map(|region| region.area() * 4).sum()
            }
            None if update == FrameCompositeUpdate::Full => {
                read_canvas_rgba_into(&self.canvas, self.width, self.height, rgba)?;
                rgba.len() as u64
            }
            None => 0,
        };
        self.telemetry.readback_ns = self
            .telemetry
            .readback_ns
            .saturating_add(elapsed_ns(readback_started));
        self.telemetry.readback_bytes =
            self.telemetry.readback_bytes.saturating_add(readback_bytes);
        self.telemetry.deferred_cache_peak_bytes = self.deferred_cache.peak_bytes;
        Ok(update)
    }
}

fn layer_frame_state(
    scene: &sekai_profile_core::Scene,
    layer: &RasterLayer,
    width: u32,
    height: u32,
    scale: f32,
) -> LayerFrameState {
    let state = layer.dynamic_layer_id.and_then(|id| scene.state(id));
    let visible = !state.is_some_and(|value| !value.render_mask);
    let transform = state
        .and_then(|value| value.dynamic.as_ref())
        .map(|value| value.transform)
        .unwrap_or_default();
    let destination = FrameRect::from_xywh(
        (layer.x + transform.dx) * scale,
        (layer.y + transform.dy) * scale,
        layer.width as f32 * scale,
        layer.height as f32 * scale,
    );
    LayerFrameState {
        visible,
        destination,
        pixel_bounds: if visible {
            pixel_bounds_for_destination(destination, width, height)
        } else {
            PixelRect::default()
        },
    }
}

pub(super) fn pixel_bounds_for_destination(
    destination: FrameRect,
    width: u32,
    height: u32,
) -> PixelRect {
    // Keep one guard pixel around the geometric destination so sub-pixel
    // coverage never survives just outside the dirty clip.
    PixelRect {
        left: (destination.left().floor() as i32 - 1).max(0),
        top: (destination.top().floor() as i32 - 1).max(0),
        right: (destination.right().ceil() as i32 + 1).min(width as i32),
        bottom: (destination.bottom().ceil() as i32 + 1).min(height as i32),
    }
}

impl AnimationFrameCompositor<'_> {
    /// Clears the whole canvas to white and draws every visible layer.
    pub(super) fn redraw_full_frame(
        &mut self,
        layers: &[RasterLayer],
        states: &[LayerFrameState],
    ) -> Result<(), AnimationError> {
        let full = PixelRect::full(self.width, self.height);
        fill_white(&mut self.canvas, self.width, full);
        self.draw_animation_layers(layers, states, full)
    }

    /// Clears `dirty` to white and redraws the visible layers that reach it,
    /// leaving the canvas outside `dirty` untouched.
    pub(super) fn redraw_dirty_frame(
        &mut self,
        layers: &[RasterLayer],
        states: &[LayerFrameState],
        dirty: PixelRect,
    ) -> Result<(), AnimationError> {
        fill_white(&mut self.canvas, self.width, dirty);
        self.draw_animation_layers(layers, states, dirty)
    }

    /// Draws each visible layer that reaches `dirty`, clipped to `dirty`, in
    /// order. Deferred layers rasterize the window `dirty` needs first.
    fn draw_animation_layers(
        &mut self,
        layers: &[RasterLayer],
        states: &[LayerFrameState],
        dirty: PixelRect,
    ) -> Result<(), AnimationError> {
        let scale = self.scale;
        for (layer_index, (layer, state)) in layers.iter().zip(states).enumerate() {
            if !state.visible || !state.pixel_bounds.intersects(dirty) {
                continue;
            }
            match &layer.content {
                RasterLayerContent::Pixels(pixels) => {
                    blit_layer(
                        self.canvas_mut(),
                        PixelsRef {
                            pixels,
                            width: layer.width,
                            height: layer.height,
                        },
                        state.destination,
                        dirty,
                    );
                }
                RasterLayerContent::Deferred(deferred) => {
                    let renderer = self.renderer.ok_or(AnimationError::Internal(
                        "deferred animation layer requires an export renderer",
                    ))?;
                    let Some(window) = deferred_source_window(layer, state, dirty, scale) else {
                        continue;
                    };
                    let rendered = render_deferred_animation_window_cached(
                        renderer,
                        deferred,
                        window,
                        layer_index,
                        &mut self.deferred_cache,
                        &mut self.render_telemetry,
                    )?;
                    let dx = state.destination.left() / scale - layer.x;
                    let dy = state.destination.top() / scale - layer.y;
                    let destination = FrameRect::from_xywh(
                        (rendered.x as f32 + dx) * scale,
                        (rendered.y as f32 + dy) * scale,
                        rendered.width as f32 * scale,
                        rendered.height as f32 * scale,
                    );
                    blit_layer(
                        self.canvas_mut(),
                        PixelsRef {
                            pixels: &rendered.pixels,
                            width: rendered.width,
                            height: rendered.height,
                        },
                        destination,
                        dirty,
                    );
                }
            }
        }
        Ok(())
    }

    fn canvas_mut(&mut self) -> PixelsMut<'_> {
        PixelsMut {
            pixels: &mut self.canvas,
            width: self.width,
            height: self.height,
        }
    }

    /// The premultiplied canvas the frames composite into.
    #[cfg(test)]
    pub(super) fn canvas(&self) -> &[u8] {
        &self.canvas
    }
}

/// Copies the whole premultiplied canvas out as non-premultiplied RGBA, which is
/// what the frame encoders consume.
pub(super) fn read_canvas_rgba_into(
    canvas: &[u8],
    width: u32,
    height: u32,
    rgba: &mut [u8],
) -> Result<(), AnimationError> {
    let required = width as usize * height as usize * 4;
    if rgba.len() != required || canvas.len() != required {
        return Err(AnimationError::Internal(
            "animation frame RGBA buffer has the wrong length",
        ));
    }
    for (source, destination) in canvas
        .as_chunks::<4>()
        .0
        .iter()
        .zip(rgba.chunks_exact_mut(4))
    {
        unpremultiply_pixel(source, destination);
    }
    Ok(())
}

/// Non-premultiplied copy of one pixel, using the same recovery the page
/// encoder applies so both outputs agree byte for byte.
#[inline]
fn unpremultiply_pixel(source: &[u8], destination: &mut [u8]) {
    let alpha = source[3];
    destination[3] = alpha;
    if alpha == 0 {
        destination[0] = 0;
        destination[1] = 0;
        destination[2] = 0;
        return;
    }
    if alpha == u8::MAX {
        destination[0] = source[0];
        destination[1] = source[1];
        destination[2] = source[2];
        return;
    }
    for channel in 0..3 {
        destination[channel] = crate::codec::unpremultiply_channel(source[channel], alpha);
    }
}

fn read_canvas_regions_rgba_into_exact(
    canvas: &[u8],
    width: u32,
    height: u32,
    regions: &[PixelRect],
    rgba: &mut [u8],
    scratch: &mut Vec<u8>,
    changed_macroblock_flags: &mut Vec<u8>,
) -> Result<Vec<u32>, AnimationError> {
    let row_bytes = width as usize * 4;
    let required = row_bytes * height as usize;
    if rgba.len() != required {
        return Err(AnimationError::Internal(
            "animation frame RGBA buffer has the wrong length",
        ));
    }
    let mb_width = width.div_ceil(16) as usize;
    let mb_height = height.div_ceil(16) as usize;
    let mb_count = mb_width
        .checked_mul(mb_height)
        .ok_or(AnimationError::Internal(
            "animation macroblock map size overflow",
        ))?;
    changed_macroblock_flags.resize(mb_count, 0);
    changed_macroblock_flags.fill(0);
    if canvas.len() != required {
        return Err(AnimationError::Internal(
            "animation canvas has the wrong length",
        ));
    }
    for region in regions {
        if region.is_empty() {
            continue;
        }
        let region_width = (region.right - region.left) as usize;
        let region_height = (region.bottom - region.top) as usize;
        let packed_row_bytes = region_width * 4;
        scratch.resize(packed_row_bytes * region_height, 0);
        for local_y in 0..region_height {
            let source_row = (region.top as usize + local_y) * row_bytes + region.left as usize * 4;
            let destination_row = local_y * packed_row_bytes;
            for local_x in 0..region_width {
                let source = source_row + local_x * 4;
                let destination = destination_row + local_x * 4;
                let (before, after) = scratch.split_at_mut(destination);
                let _ = before;
                unpremultiply_pixel(&canvas[source..source + 4], &mut after[..4]);
            }
        }
        mark_changed_macroblocks_for_region(
            rgba,
            scratch,
            width,
            *region,
            changed_macroblock_flags,
        )?;
        for local_y in 0..region_height {
            let destination =
                (region.top as usize + local_y) * row_bytes + region.left as usize * 4;
            let source = local_y * packed_row_bytes;
            rgba[destination..destination + packed_row_bytes]
                .copy_from_slice(&scratch[source..source + packed_row_bytes]);
        }
    }
    Ok(changed_macroblock_flags
        .iter()
        .enumerate()
        .filter_map(|(index, &changed)| (changed != 0).then_some(index as u32))
        .collect())
}

pub(super) fn mark_changed_macroblocks_for_region(
    rgba: &[u8],
    packed_region: &[u8],
    width: u32,
    region: PixelRect,
    changed_macroblock_flags: &mut [u8],
) -> Result<(), AnimationError> {
    if region.is_empty() {
        return Ok(());
    }
    let width = width as usize;
    let row_bytes = width * 4;
    let region_width = (region.right - region.left) as usize;
    let region_height = (region.bottom - region.top) as usize;
    let packed_row_bytes = region_width * 4;
    if !rgba.len().is_multiple_of(row_bytes)
        || packed_region.len() != packed_row_bytes * region_height
    {
        return Err(AnimationError::Internal(
            "invalid exact dirty-region buffers",
        ));
    }
    let mb_width = width.div_ceil(16);
    let first_mb_x = region.left.max(0) as usize / 16;
    let last_mb_x = (region.right.max(0) as usize).div_ceil(16).min(mb_width);
    for local_y in 0..region_height {
        let y = region.top as usize + local_y;
        let mb_y = y / 16;
        for mb_x in first_mb_x..last_mb_x {
            let mb_index = mb_y * mb_width + mb_x;
            if changed_macroblock_flags.get(mb_index).copied() == Some(1) {
                continue;
            }
            let left = (mb_x * 16).max(region.left as usize);
            let right = ((mb_x + 1) * 16).min(region.right as usize);
            let bytes = (right - left) * 4;
            let before = y * row_bytes + left * 4;
            let after = local_y * packed_row_bytes + (left - region.left as usize) * 4;
            if rgba_segment_changed(
                &rgba[before..before + bytes],
                &packed_region[after..after + bytes],
            ) {
                changed_macroblock_flags[mb_index] = 1;
            }
        }
    }
    Ok(())
}

fn rgba_segment_changed(before: &[u8], after: &[u8]) -> bool {
    if before.len() != after.len() {
        return true;
    }
    #[cfg(target_arch = "x86_64")]
    if before.len() == 64
        && std::arch::is_x86_feature_detected!("avx512f")
        && std::arch::is_x86_feature_detected!("avx512bw")
    {
        // SAFETY: both slices contain one complete 16-pixel macroblock row.
        return unsafe { rgba_segment_changed_avx512(before, after) };
    }
    before != after
}

/// # Safety
///
/// The caller must have established that the CPU supports `avx512f` and
/// `avx512bw`, as [`rgba_segment_changed`] does before calling. Both slices
/// must be exactly `64` bytes long, one 16-pixel RGBA8 macroblock row, so that
/// each unaligned 64-byte load covers its whole slice and neither runs past
/// the end; the loads require no more than byte alignment.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn rgba_segment_changed_avx512(before: &[u8], after: &[u8]) -> bool {
    use std::arch::x86_64::*;
    let left = _mm512_loadu_si512(before.as_ptr().cast());
    let right = _mm512_loadu_si512(after.as_ptr().cast());
    _mm512_cmpeq_epi8_mask(left, right) != u64::MAX
}

#[cfg(test)]
pub(super) fn composite_frame(
    scene: &sekai_profile_core::Scene,
    layers: &[RasterLayer],
    width: u32,
    height: u32,
    scale: f32,
) -> Result<Vec<u8>, AnimationError> {
    let bytes = frame_byte_len(width, height)?;
    let mut canvas = vec![0u8; bytes];
    let full = PixelRect::full(width, height);
    // The game always renders a complete custom-profile card over its default
    // white canvas. Keeping full animation frames opaque also lets GIF delta
    // frames erase a moving layer by writing white pixels at its old position.
    fill_white(&mut canvas, width, full);
    for layer in layers {
        let state = layer.dynamic_layer_id.and_then(|id| scene.state(id));
        if state.is_some_and(|value| !value.render_mask) {
            continue;
        }
        let transform = state
            .and_then(|value| value.dynamic.as_ref())
            .map(|value| value.transform)
            .unwrap_or_default();
        let destination = FrameRect::from_xywh(
            (layer.x + transform.dx) * scale,
            (layer.y + transform.dy) * scale,
            layer.width as f32 * scale,
            layer.height as f32 * scale,
        );
        let RasterLayerContent::Pixels(pixels) = &layer.content else {
            return Err(AnimationError::Internal(
                "deferred animation layers require the streaming compositor",
            ));
        };
        blit_layer(
            PixelsMut {
                pixels: &mut canvas,
                width,
                height,
            },
            PixelsRef {
                pixels,
                width: layer.width,
                height: layer.height,
            },
            destination,
            full,
        );
    }
    let mut rgba = vec![0u8; bytes];
    read_canvas_rgba_into(&canvas, width, height, &mut rgba)?;
    Ok(rgba)
}
