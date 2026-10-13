//! Pixel occlusion: masks of the opaque images drawn above each SDF element,
//! and the fragment statistics they produce.

use super::authored::render_element_authored_identity;
use super::elapsed_ns;
use crate::types::CustomProfileCard;
use std::sync::Arc;

pub(super) struct PixelOcclusionDryRun {
    pub(super) masks_by_element: Vec<Option<Arc<crate::sdf::tile::PixelOcclusionMask>>>,
    pub(super) eligible_image_count: u64,
    pub(super) mask_snapshot_count: u64,
    pub(super) mask_bytes: u64,
    pub(super) build_ns: u64,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_pixel_occlusion_dry_run(
    card: &CustomProfileCard,
    elements: &[crate::elements::RenderElement<'_>],
    scene: &sekai_profile_core::profile_scene::ResolvedProfileScene,
    store: &crate::render_object::MappedRenderObjectStore,
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
        let is_sdf = matches!(
            element,
            crate::elements::RenderElement::Text(_) | crate::elements::RenderElement::Shape(_)
        );
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

pub(super) fn add_sdf_occlusion_stats(
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
