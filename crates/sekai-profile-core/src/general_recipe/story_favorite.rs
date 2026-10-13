//! The story favorite panel (type 14) and its scrolling list.

use super::nodes::{image_node, item_region, layout_rect, localized_source, shape_node, text_node};
use super::{
    GeneralClip, GeneralGeometry, GeneralImageSampling, GeneralImageSource,
    GeneralImageSourceConstraint, GeneralRecipeNode, GeneralTextAlign,
};
use crate::profile_scene::{
    ComponentControlSource, ComponentControlState, ProfileComponentSnapshot, ProfileResolveError,
};
use crate::{CommandControlBinding, InteractionRegionSource, ParameterValue, Rect, StableId};

pub(super) fn build_story_favorite_recipe(
    layer_id: StableId,
    source_key: &str,
    snapshot: &ProfileComponentSnapshot,
    nodes: &mut Vec<GeneralRecipeNode>,
    controls: &mut Vec<ComponentControlSource>,
    interaction_regions: &mut Vec<InteractionRegionSource>,
) -> Result<(), ProfileResolveError> {
    let panel = &crate::profile_layout::STORY_FAVORITE;
    nodes.push(text_node(
        layer_id,
        source_key,
        "title",
        0,
        layout_rect(panel.elements[0]),
        localized_source(snapshot, "custom_profile.general.story_favorite.title")?,
        panel.elements[0].h,
        [0.33, 0.33, 0.33, 1.0],
        GeneralTextAlign::Center,
        0.0,
        false,
    ));
    nodes.push(shape_node(
        layer_id,
        source_key,
        "separator",
        1,
        Rect {
            height: 2.0,
            ..layout_rect(panel.elements[1])
        },
        GeneralGeometry::Rect,
        [0.78, 0.78, 0.78, 0.5],
    ));
    let content_start = nodes.len();
    let region_start = interaction_regions.len();
    // Every slot is drawn and slots nobody shares stay empty. Slots past the
    // measured ones continue the two-column grid.
    let measured_slots = &panel.elements[2..];
    let slot_count = measured_slots.len().max(snapshot.story_favorites.len());
    for index in 0..slot_count {
        let target = measured_slots.get(index).copied().unwrap_or_else(|| {
            let column = index % 2;
            let row = index / 2;
            crate::profile_layout::ElementLayout {
                cx: if column == 0 { -212.0 } else { 212.0 },
                cy: 220.0 - row as f32 * 195.0,
                w: 400.0,
                h: 170.0,
            }
        });
        let bounds = layout_rect(target);
        let role = format!("story-{index}");
        let Some(favorite) = snapshot
            .story_favorites
            .get(index)
            .filter(|favorite| favorite.story_id > 0)
        else {
            nodes.push(shape_node(
                layer_id,
                source_key,
                &format!("{role}-empty"),
                2 + index as u32,
                bounds,
                GeneralGeometry::RoundedRect { radius: [8.0, 8.0] },
                [0.82, 0.82, 0.82, 0.4],
            ));
            continue;
        };
        if let Some(descriptor) = &favorite.image.descriptor {
            nodes.push(image_node(
                layer_id,
                source_key,
                &role,
                2 + index as u32,
                bounds,
                descriptor.resource.clone(),
                Some([descriptor.natural_width, descriptor.natural_height]),
                GeneralImageSource::WholeImageImplicit,
                GeneralImageSampling::Nearest,
                GeneralImageSourceConstraint::Implicit,
                &favorite.image,
                descriptor.provenance.clone(),
                Vec::new(),
            ));
        } else {
            nodes.push(shape_node(
                layer_id,
                source_key,
                &format!("{role}-placeholder"),
                2 + index as u32,
                bounds,
                GeneralGeometry::RoundedRect { radius: [8.0, 8.0] },
                [0.82, 0.82, 0.82, 0.4],
            ));
        }
        interaction_regions.push(item_region(
            layer_id,
            source_key,
            &role,
            bounds,
            std::collections::BTreeMap::from([
                (
                    "field".into(),
                    ParameterValue::Text(favorite.image.source_field.clone()),
                ),
                (
                    "source_id".into(),
                    ParameterValue::Text(favorite.image.source_id.clone()),
                ),
                (
                    "story_type".into(),
                    ParameterValue::Text(favorite.story_type.clone()),
                ),
                (
                    "story_id".into(),
                    ParameterValue::I64(favorite.story_id.into()),
                ),
            ]),
        ));
    }
    let viewport = Rect {
        x: -424.0,
        y: -320.0,
        width: 848.0,
        height: 756.0,
    };
    let content_bottom = nodes[content_start..]
        .iter()
        .map(|node| node.bounds.y + node.bounds.height)
        .fold(viewport.y + viewport.height, f32::max);
    let max = (content_bottom - (viewport.y + viewport.height)).max(0.0);
    if max > 0.0 {
        let control_id =
            crate::profile_scene::component_control_id(source_key, "story-favorite-scroll");
        let clip = GeneralClip::Rect {
            bounds: viewport,
            anti_alias: false,
        };
        for node in &mut nodes[content_start..] {
            node.clips.push(clip.clone());
            node.control_bindings
                .push(CommandControlBinding::ScrollContent { control_id });
        }
        for region in &mut interaction_regions[region_start..] {
            region.clip = Some(crate::profile_scene::rect_quad(viewport));
            region
                .control_bindings
                .push(CommandControlBinding::ScrollContent { control_id });
        }
        let track_bounds = Rect {
            x: 475.5,
            y: viewport.y,
            width: 4.0,
            height: viewport.height,
        };
        nodes.push(shape_node(
            layer_id,
            source_key,
            "story-scroll-track",
            10_000,
            track_bounds,
            GeneralGeometry::RoundedRect { radius: [2.0, 2.0] },
            [0.25, 0.25, 0.25, 0.18],
        ));
        let thumb_height = (viewport.height * viewport.height / (viewport.height + max))
            .clamp(40.0, viewport.height);
        let thumb_bounds = Rect {
            height: thumb_height,
            ..track_bounds
        };
        let mut thumb = shape_node(
            layer_id,
            source_key,
            "story-scroll-thumb",
            10_001,
            thumb_bounds,
            GeneralGeometry::RoundedRect { radius: [2.0, 2.0] },
            [0.25, 0.25, 0.25, 0.55],
        );
        thumb
            .control_bindings
            .push(CommandControlBinding::ScrollThumb { control_id });
        nodes.push(thumb);
        let mut thumb_region = item_region(
            layer_id,
            source_key,
            "story-scroll-thumb",
            thumb_bounds,
            std::collections::BTreeMap::from([
                (
                    "control_id".into(),
                    ParameterValue::Text(format!("{:016x}", control_id.0)),
                ),
                ("action".into(), ParameterValue::Text("scroll_thumb".into())),
            ]),
        );
        thumb_region.capabilities = vec!["inspect".into(), "scroll".into(), "drag".into()];
        thumb_region
            .control_bindings
            .push(CommandControlBinding::ScrollThumb { control_id });
        interaction_regions.push(thumb_region);
        controls.push(ComponentControlSource {
            id: control_id,
            layer_id,
            role: "story-favorite-scroll".into(),
            state: ComponentControlState::Scroll {
                offset: 0.0,
                min: 0.0,
                max,
                viewport_extent: viewport.height,
                content_extent: viewport.height + max,
                step: 195.0,
            },
        });
        let mut scroll_region = item_region(
            layer_id,
            source_key,
            "story-favorite-scroll",
            viewport,
            std::collections::BTreeMap::from([
                (
                    "control_id".into(),
                    ParameterValue::Text(format!("{:016x}", control_id.0)),
                ),
                ("action".into(), ParameterValue::Text("scroll_by".into())),
            ]),
        );
        scroll_region.capabilities = vec!["inspect".into(), "scroll".into()];
        scroll_region
            .control_bindings
            .push(CommandControlBinding::ScrollViewport { control_id });
        interaction_regions.push(scroll_region);
    }
    Ok(())
}
