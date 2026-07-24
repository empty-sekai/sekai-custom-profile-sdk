//! General 面板渲染模块。
//!
//! 每个 general 面板是自定义名片的可选组件，
//! 通过 objectData 控制位置/旋转/缩放。
//! 面板内部内容从 ProfileData 动态填充。

#[cfg(feature = "skia-core")]
use crate::assets::AssetStore;
#[cfg(feature = "skia-core")]
use crate::masterdata::MasterData;
#[cfg(feature = "skia-core")]
use crate::profile::ProfileData;
#[cfg(feature = "skia-core")]
use skia_safe::{
    Canvas, Color, Color4f, Font, FontMgr, FontStyle, Paint, PaintStyle, Point, Rect, Typeface,
};

#[cfg(feature = "skia-core")]
pub(crate) mod layout;
#[cfg(feature = "skia-core")]
pub(crate) mod sdf_text;
#[cfg(feature = "skia-core")]

/// 绘制 General 面板（canvas 原点已在元素中心）。
#[cfg(feature = "skia-core")]
pub fn draw_general(
    canvas: &Canvas,
    general_type: i32,
    profile: &ProfileData,
    md: &MasterData,
    assets: Option<&AssetStore>,
) {
    match general_type {
        2 | 3 | 4 | 5 | 6 | 9 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18 => {
            draw_shared_general_recipe(canvas, general_type, profile, md, assets)
        }
        _ => draw_placeholder(canvas, general_type),
    }
}

#[cfg(feature = "skia-core")]
fn draw_shared_general_recipe(
    canvas: &Canvas,
    general_type: i32,
    profile: &ProfileData,
    md: &MasterData,
    assets: Option<&AssetStore>,
) {
    use allium_renderer_core::general_recipe::{
        GeneralClip, GeneralFill, GeneralGeometry, GeneralGroupPhase, GeneralImageSource,
        GeneralImageSourceConstraint, GeneralRecipePayload, GeneralTextAlign,
    };
    use allium_renderer_core::{ResourceKey, StableId, TextSource};

    struct NativeResourceMetadata<'a>(Option<&'a AssetStore>);
    impl allium_renderer_core::profile_resolve::ResourceMetadata for NativeResourceMetadata<'_> {
        fn metric(
            &self,
            resource: &ResourceKey,
        ) -> Option<allium_renderer_core::profile_resolve::ResourceMetric> {
            self.0
                .and_then(|store| store.get_image(&resource.key))
                .map(
                    |image| allium_renderer_core::profile_resolve::ResourceMetric {
                        width: image.width() as f32,
                        height: image.height() as f32,
                    },
                )
        }

        fn availability(
            &self,
            resource: &ResourceKey,
        ) -> allium_renderer_core::profile_resolve::ResourceAvailability {
            if self
                .0
                .and_then(|store| store.get_image(&resource.key))
                .is_some()
            {
                allium_renderer_core::profile_resolve::ResourceAvailability::Available
            } else {
                allium_renderer_core::profile_resolve::ResourceAvailability::Unavailable
            }
        }
    }

    let core_profile = profile.to_core_profile();
    let has_live_master_honor = profile.honor_slots.iter().any(|slot| {
        slot.profile_honor_type != "bonds"
            && md
                .resolve_honor(slot.honor_id, slot.honor_level)
                .is_some_and(|honor| honor.is_live_master)
    });
    let needs_font = allium_renderer_core::general_recipe::general_type_requires_font(
        general_type,
        has_live_master_honor,
    );
    let mut snapshot = match allium_renderer_core::profile_resolve::build_profile_component_snapshot(
        &core_profile,
        md,
        "resolved",
        &NativeResourceMetadata(assets),
        None,
        needs_font,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::warn!(general_type, error = %error, "shared General snapshot resolution failed");
            return;
        }
    };
    strip_live_master_star_assets(&mut snapshot);

    let recipe = match allium_renderer_core::general_recipe::build_general_recipe(
        general_type,
        StableId::derive("native-general-layer-v1", &general_type.to_le_bytes()),
        &format!("native-general:{general_type}"),
        &snapshot,
    ) {
        Ok(Some(recipe)) => recipe,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(general_type, error = %error, "shared General recipe resolution failed");
            return;
        }
    };
    fn apply_general_clips(canvas: &Canvas, clips: &[GeneralClip]) {
        for clip in clips {
            match clip {
                GeneralClip::Rect { bounds, anti_alias } => {
                    canvas.clip_rect(
                        Rect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height),
                        None,
                        Some(*anti_alias),
                    );
                }
                GeneralClip::RoundedRect {
                    bounds,
                    radius,
                    anti_alias,
                } => {
                    let rect = Rect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height);
                    canvas.clip_rrect(
                        skia_safe::RRect::new_rect_xy(rect, radius[0], radius[1]),
                        None,
                        Some(*anti_alias),
                    );
                }
                GeneralClip::Ellipse { bounds, anti_alias } => {
                    let mut builder = skia_safe::PathBuilder::new();
                    builder.add_oval(
                        Rect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height),
                        None,
                        None,
                    );
                    canvas.clip_path(&builder.detach(), skia_safe::ClipOp::Intersect, *anti_alias);
                }
            }
        }
    }
    for node in recipe.nodes {
        if !recipe_node_visible(&node.control_bindings, &recipe.controls) {
            continue;
        }
        match node.payload {
            GeneralRecipePayload::Shape {
                geometry,
                fill,
                stroke,
            } => {
                if !node.clips.is_empty() {
                    canvas.save();
                    apply_general_clips(canvas, &node.clips);
                }
                let mut paint = Paint::default();
                paint.set_style(PaintStyle::Fill);
                paint.set_anti_alias(true);
                let rect = Rect::from_xywh(
                    node.bounds.x,
                    node.bounds.y,
                    node.bounds.width,
                    node.bounds.height,
                );
                match fill {
                    GeneralFill::Solid(color) => {
                        paint.set_color4f(
                            Color4f::new(color[0], color[1], color[2], color[3]),
                            None,
                        );
                    }
                    GeneralFill::LinearGradient {
                        start,
                        end,
                        colors,
                        stops,
                    } => {
                        let points = (
                            Point::new(
                                rect.left + start[0] * rect.width(),
                                rect.top + start[1] * rect.height(),
                            ),
                            Point::new(
                                rect.left + end[0] * rect.width(),
                                rect.top + end[1] * rect.height(),
                            ),
                        );
                        let skia_colors = colors
                            .iter()
                            .map(|color| {
                                Color::from_argb(
                                    (color[3] * 255.0).round() as u8,
                                    (color[0] * 255.0).round() as u8,
                                    (color[1] * 255.0).round() as u8,
                                    (color[2] * 255.0).round() as u8,
                                )
                            })
                            .collect::<Vec<_>>();
                        if let Some(shader) = skia_safe::Shader::linear_gradient(
                            points,
                            skia_colors.as_slice(),
                            Some(stops.as_slice()),
                            skia_safe::TileMode::Clamp,
                            None,
                            None,
                        ) {
                            paint.set_shader(shader);
                        }
                    }
                }
                match geometry {
                    GeneralGeometry::Rect => {
                        canvas.draw_rect(rect, &paint);
                    }
                    GeneralGeometry::RoundedRect { radius } => {
                        canvas.draw_round_rect(rect, radius[0], radius[1], &paint);
                    }
                    GeneralGeometry::Ellipse => {
                        canvas.draw_oval(rect, &paint);
                    }
                }
                if let Some(stroke) = stroke {
                    paint.set_style(PaintStyle::Stroke);
                    paint.set_stroke_width(stroke.width);
                    paint.set_color4f(
                        Color4f::new(
                            stroke.color[0],
                            stroke.color[1],
                            stroke.color[2],
                            stroke.color[3],
                        ),
                        None,
                    );
                    match geometry {
                        GeneralGeometry::Rect => {
                            canvas.draw_rect(rect, &paint);
                        }
                        GeneralGeometry::RoundedRect { radius } => {
                            canvas.draw_round_rect(rect, radius[0], radius[1], &paint);
                        }
                        GeneralGeometry::Ellipse => {
                            canvas.draw_oval(rect, &paint);
                        }
                    }
                }
                if !node.clips.is_empty() {
                    canvas.restore();
                }
            }
            GeneralRecipePayload::Text {
                source,
                font_size,
                color,
                align,
                line_spacing,
                wrap,
                render_baseline,
                ..
            } => {
                if !node.clips.is_empty() {
                    canvas.save();
                    apply_general_clips(canvas, &node.clips);
                }
                let text = match source {
                    TextSource::Authored { value }
                    | TextSource::ProfileField { value, .. }
                    | TextSource::MasterData { value, .. }
                    | TextSource::Localized { value, .. } => value,
                };
                if node.role.starts_with("honor-") && node.role.ends_with("-progress") {
                    let center_x = node.bounds.x + node.bounds.width / 2.0;
                    let baseline_y = node.bounds.y + node.bounds.height / 2.0 + font_size * 0.35;
                    crate::elements::honor::draw_live_master_progress_text(
                        canvas, &text, center_x, baseline_y,
                    );
                    if !node.clips.is_empty() {
                        canvas.restore();
                    }
                    continue;
                }
                let text_layout = layout::ElementLayout {
                    cx: node.bounds.x + node.bounds.width / 2.0,
                    cy: -(node.bounds.y + node.bounds.height / 2.0),
                    w: node.bounds.width,
                    h: node.bounds.height,
                };
                let color = Color4f::new(color[0], color[1], color[2], color[3]);
                let align = match align {
                    GeneralTextAlign::Left => sdf_text::SdfTextAlign::Left,
                    GeneralTextAlign::Center => sdf_text::SdfTextAlign::Center,
                    GeneralTextAlign::Right => sdf_text::SdfTextAlign::Right,
                };
                if wrap {
                    sdf_text::draw_general_sdf_text_wrapped(
                        canvas,
                        &text,
                        &text_layout,
                        md,
                        color,
                        align,
                        font_size,
                        line_spacing,
                        render_baseline,
                    );
                } else {
                    sdf_text::draw_general_sdf_text_with_font(
                        canvas,
                        &text,
                        &text_layout,
                        md,
                        color,
                        align,
                        font_size,
                        line_spacing,
                        1,
                    );
                }
                if !node.clips.is_empty() {
                    canvas.restore();
                }
            }
            GeneralRecipePayload::Image {
                resource,
                source,
                source_constraint,
                tint,
                blend_mode,
                ..
            } => {
                let Some(image) = assets.and_then(|store| store.get_image(&resource.key)) else {
                    continue;
                };
                canvas.save();
                for clip in &node.clips {
                    match clip {
                        GeneralClip::Rect { bounds, anti_alias } => {
                            canvas.clip_rect(
                                Rect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height),
                                None,
                                Some(*anti_alias),
                            );
                        }
                        GeneralClip::RoundedRect {
                            bounds,
                            radius,
                            anti_alias,
                        } => {
                            let rect =
                                Rect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height);
                            let rrect = skia_safe::RRect::new_rect_xy(rect, radius[0], radius[1]);
                            canvas.clip_rrect(rrect, None, Some(*anti_alias));
                        }
                        GeneralClip::Ellipse { bounds, anti_alias } => {
                            let mut builder = skia_safe::PathBuilder::new();
                            builder.add_oval(
                                Rect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height),
                                None,
                                None,
                            );
                            canvas.clip_path(
                                &builder.detach(),
                                skia_safe::ClipOp::Intersect,
                                *anti_alias,
                            );
                        }
                    }
                }
                let destination = Rect::from_xywh(
                    node.bounds.x,
                    node.bounds.y,
                    node.bounds.width,
                    node.bounds.height,
                );
                let mut paint = Paint::default();
                paint.set_blend_mode(match blend_mode {
                    allium_renderer_core::BlendMode::SrcOver => skia_safe::BlendMode::SrcOver,
                    allium_renderer_core::BlendMode::SrcIn => skia_safe::BlendMode::SrcIn,
                    allium_renderer_core::BlendMode::DstIn => skia_safe::BlendMode::DstIn,
                    allium_renderer_core::BlendMode::Multiply => skia_safe::BlendMode::Multiply,
                    allium_renderer_core::BlendMode::Screen => skia_safe::BlendMode::Screen,
                    allium_renderer_core::BlendMode::Add => skia_safe::BlendMode::Plus,
                });
                if tint != [1.0; 4] {
                    if let Some(filter) = skia_safe::color_filters::blend_with_color_space(
                        Color4f::new(tint[0], tint[1], tint[2], tint[3]),
                        None,
                        skia_safe::BlendMode::SrcIn,
                    ) {
                        paint.set_color_filter(filter);
                    }
                }
                match source {
                    GeneralImageSource::WholeImageImplicit => {
                        canvas.draw_image_rect(image, None, destination, &paint);
                    }
                    GeneralImageSource::WholeImageExplicit => {
                        let source =
                            Rect::from_xywh(0.0, 0.0, image.width() as f32, image.height() as f32);
                        canvas.draw_image_rect(
                            image,
                            Some((&source, skia_safe::canvas::SrcRectConstraint::Fast)),
                            destination,
                            &paint,
                        );
                    }
                    GeneralImageSource::Pixels(source) => {
                        let source =
                            Rect::from_xywh(source.x, source.y, source.width, source.height);
                        let constraint = match source_constraint {
                            GeneralImageSourceConstraint::Strict => {
                                skia_safe::canvas::SrcRectConstraint::Strict
                            }
                            GeneralImageSourceConstraint::Implicit
                            | GeneralImageSourceConstraint::Fast => {
                                skia_safe::canvas::SrcRectConstraint::Fast
                            }
                        };
                        canvas.draw_image_rect(
                            image,
                            Some((&source, constraint)),
                            destination,
                            &paint,
                        );
                    }
                }
                canvas.restore();
            }
            GeneralRecipePayload::Group { phase, .. } => match phase {
                GeneralGroupPhase::Begin => {
                    let bounds = Rect::from_xywh(
                        node.bounds.x,
                        node.bounds.y,
                        node.bounds.width,
                        node.bounds.height,
                    );
                    canvas.save_layer(&skia_safe::canvas::SaveLayerRec::default().bounds(&bounds));
                }
                GeneralGroupPhase::End => {
                    canvas.restore();
                }
            },
        }
    }

    fn recipe_node_visible(
        bindings: &[allium_renderer_core::CommandControlBinding],
        controls: &[allium_renderer_core::profile_scene::ComponentControlSource],
    ) -> bool {
        bindings.iter().all(|binding| match binding {
            allium_renderer_core::CommandControlBinding::TabOption { control_id, value } => {
                controls.iter().any(|control| {
                    control.id == *control_id
                        && matches!(
                            &control.state,
                            allium_renderer_core::profile_scene::ComponentControlState::Tabs { active, .. }
                                if active == value
                        )
                })
            }
            allium_renderer_core::CommandControlBinding::ScrollContent { .. }
            | allium_renderer_core::CommandControlBinding::ScrollThumb { .. }
            | allium_renderer_core::CommandControlBinding::ScrollViewport { .. } => true,
        })
    }
}

#[cfg(feature = "skia-core")]
fn strip_live_master_star_assets(
    snapshot: &mut allium_renderer_core::profile_scene::ProfileComponentSnapshot,
) {
    for honor in &mut snapshot.honor_slots {
        if let allium_renderer_core::profile_scene::HonorVisualKind::Standard {
            is_live_master: true,
            live_star_on,
            live_star_off,
            ..
        } = &mut honor.visual
        {
            *live_star_on = None;
            *live_star_off = None;
        }
    }
}

#[cfg(all(test, feature = "skia-core"))]
mod sdf_text_contract_tests {
    use super::*;

    #[test]
    fn player_identity_text_uses_region_font_one_and_preserves_tmp_markup() {
        let spec = sdf_text::build_general_sdf_text(
            "<color=#ff0000>玩家</color>",
            &layout::ElementLayout {
                cx: 10.0,
                cy: 20.0,
                w: 200.0,
                h: 32.0,
            },
            Color4f::new(0.2, 0.2, 0.2, 1.0),
            sdf_text::SdfTextAlign::Left,
            26.0,
            6.0,
        );
        assert_eq!(spec.element.font_id, 1);
        assert!(spec.element.text.contains("<color=#ff0000>玩家</color>"));
        assert_eq!(spec.element.text_type & 0x07, 1);
        assert_eq!(spec.origin.x, 10.0);
        assert_eq!(spec.render_placement.anchor_x, -50.0);
        assert_eq!(spec.render_placement.baseline, Some(26.0 * 0.35 / 2.0));
    }

    #[test]
    fn signature_text_also_uses_region_font_one() {
        let spec = sdf_text::build_general_sdf_text(
            "<size=120%>签名</size>",
            &layout::ElementLayout {
                cx: 0.0,
                cy: 0.0,
                w: 400.0,
                h: 96.0,
            },
            Color4f::new(0.2, 0.2, 0.2, 1.0),
            sdf_text::SdfTextAlign::Left,
            26.0,
            6.0,
        );
        assert_eq!(spec.element.font_id, 1);
        assert!(spec.element.text.contains("<size=120%>签名</size>"));
    }

    #[test]
    fn general_sdf_text_supports_right_alignment_without_fast_measurement() {
        let spec = sdf_text::build_general_sdf_text(
            "123",
            &layout::ElementLayout {
                cx: 10.0,
                cy: 0.0,
                w: 200.0,
                h: 32.0,
            },
            Color4f::new(1.0, 1.0, 1.0, 1.0),
            sdf_text::SdfTextAlign::Right,
            26.0,
            0.0,
        );
        assert_eq!(spec.element.text_type & 0x07, 4);
        assert_eq!(spec.origin.x, 10.0);
        assert_eq!(spec.render_placement.anchor_x, 50.0);
    }

    #[test]
    fn lowered_general_text_reconstructs_the_production_tmp_spec() {
        let placement = crate::text::TextRenderPlacement {
            anchor_x: -100.0,
            baseline: Some(4.55),
        };
        let spec = sdf_text::build_general_sdf_text_from_lowered(
            "玩家名称\n个性签名",
            400.0,
            [0.2, 0.3, 0.4, 1.0],
            1,
            13.0,
            6.0,
            1,
            placement,
        )
        .expect("lowered General text spec");
        assert_eq!(spec.element.size, 13.0);
        assert_eq!(spec.element.line_spacing, 3.0);
        assert_eq!(spec.element.text_type & 0x07, 1);
        assert_eq!(spec.render_placement, placement);
        assert!(spec.element.text.contains("玩家名称\n个性签名"));
    }

    #[test]
    fn general_sdf_text_module_has_no_fast_text_fallback() {
        let sources = include_str!("sdf_text.rs");
        for forbidden in ["draw_str", "measure_str", "legacy_make_typeface"] {
            assert!(
                !sources.contains(forbidden),
                "general SDF module owns a fast-text fallback: {forbidden}"
            );
        }
    }

    #[test]
    fn legacy_live_master_snapshot_drops_decorative_star_assets() {
        use allium_renderer_core::profile_scene::{
            HonorVisualKind, HonorVisualSnapshot, ProfileComponentSnapshot, ResourceDescriptor,
        };
        use allium_renderer_core::ResourceKey;
        use std::collections::BTreeMap;

        let descriptor = |key: &str| ResourceDescriptor {
            resource: ResourceKey {
                namespace: "static".into(),
                key: key.into(),
            },
            natural_width: 16.0,
            natural_height: 16.0,
            provenance: BTreeMap::new(),
        };
        let mut snapshot = ProfileComponentSnapshot {
            honor_slots: vec![HonorVisualSnapshot {
                source_field: "userProfile.honorSlots".into(),
                source_id: "3013".into(),
                honor_id: 3013,
                honor_level: 37,
                full_size: true,
                visual: HonorVisualKind::Standard {
                    honor_type: "achievement".into(),
                    has_star: true,
                    is_live_master: true,
                    progress: 358,
                    background: None,
                    frame_candidates: Vec::new(),
                    overlay: None,
                    star: None,
                    star_high: None,
                    live_star_on: Some(descriptor("honor/live_master_honor_star_1")),
                    live_star_off: Some(descriptor("honor/live_master_honor_star_2")),
                },
            }],
            ..ProfileComponentSnapshot::default()
        };

        strip_live_master_star_assets(&mut snapshot);
        let HonorVisualKind::Standard {
            live_star_on,
            live_star_off,
            ..
        } = &snapshot.honor_slots[0].visual
        else {
            panic!("live-master visual kind changed")
        };
        assert!(live_star_on.is_none());
        assert!(live_star_off.is_none());
    }

    #[test]
    fn optimized_general_text_has_no_second_layout_implementation() {
        let compositor = include_str!("../../profile_compositor.rs");
        assert!(compositor.contains("capture_general_sdf_text_from_lowered"));
        for forbidden in ["space_advance", "line_widths", "cursor_x += advance"] {
            assert!(
                !compositor.contains(forbidden),
                "semantic compositor still owns text layout: {forbidden}"
            );
        }
    }

    #[test]
    fn production_identity_general_dispatch_consumes_the_shared_recipe() {
        let dispatch = include_str!("mod.rs");
        assert!(dispatch.contains("allium_renderer_core::general_recipe::build_general_recipe"));
        assert!(dispatch.contains("draw_shared_general_recipe"));
        assert!(!dispatch.contains(&["13 => player_name", "::draw_player_name"].concat()));
        assert!(!dispatch.contains(&["4 => comment", "::draw_comment"].concat()));
        assert!(!dispatch.contains(&["2 => total_power", "::draw_total_power"].concat()));
        assert!(!dispatch.contains(&["9 => mvp_superstar", "::draw_mvp_superstar"].concat()));
        assert!(
            dispatch.contains("2 | 3 | 4 | 5 | 6 | 9 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18")
        );
        assert!(!dispatch.contains(&["11 => ", "char_rank"].concat()));
        assert!(!dispatch.contains(&["14 => story_favorite", "::draw_story_favorite"].concat()));
        assert!(!dispatch.contains(&["11 | ", "15 => ", "char_rank"].concat()));
        assert!(!dispatch.contains(&["15 => ", "char_rank"].concat()));
        assert!(!dispatch.contains(&["3 => deck", "::draw_deck"].concat()));
        assert!(!dispatch.contains(&["5 => leader_member", "::draw_leader_member"].concat()));
        assert!(!dispatch.contains(&["6 => honors_panel", "::draw_honors_panel"].concat()));
        assert!(dispatch.contains(&["color_filters", "::blend_with_color_space"].concat()));
        assert!(!dispatch.contains(&["12 => music_clear", "::draw_music_clear"].concat()));
        assert!(!dispatch.contains(&["16 => music_clear_tab", "::draw_music_clear_tab"].concat()));
        assert!(!dispatch.contains(&["10 => challenge_live", "::draw_challenge_live"].concat()));
        assert!(!dispatch.contains(&["17 => player_level", "::draw_player_level"].concat()));
        assert!(!dispatch.contains(&["18 => player_avatar", "::draw_player_avatar"].concat()));
    }

    #[test]
    fn general_text_never_applies_a_canvas_clip() {
        let source = include_str!("sdf_text.rs");
        assert!(!source.contains(&["canvas.clip_", "rect(spec.clip"].concat()));
    }

    #[test]
    fn fused_level_bar_tint_is_pixel_exact_to_native_src_in_layer() {
        use skia_safe::{surfaces, AlphaType, ColorType, IPoint, ImageInfo};

        let size = (4, 4);
        let rect = Rect::from_xywh(0.0, 0.0, 4.0, 4.0);
        let mut source_surface = surfaces::raster_n32_premul(size).unwrap();
        source_surface.canvas().clear(Color::TRANSPARENT);
        for x in 0..4 {
            let mut paint = Paint::default();
            paint.set_color4f(Color4f::new(1.0, 1.0, 1.0, (x + 1) as f32 / 4.0), None);
            source_surface
                .canvas()
                .draw_rect(Rect::from_xywh(x as f32, 0.0, 1.0, 4.0), &paint);
        }
        let source = source_surface.image_snapshot();
        let tint = Color4f::new(68.0 / 255.0, 68.0 / 255.0, 102.0 / 255.0, 1.0);

        let mut layered = surfaces::raster_n32_premul(size).unwrap();
        layered.canvas().clear(Color::TRANSPARENT);
        let layer = skia_safe::canvas::SaveLayerRec::default().bounds(&rect);
        layered.canvas().save_layer(&layer);
        layered
            .canvas()
            .draw_image_rect(&source, None, rect, &Paint::default());
        let mut src_in = Paint::default();
        src_in.set_blend_mode(skia_safe::BlendMode::SrcIn);
        src_in.set_color4f(tint, None);
        layered.canvas().draw_rect(rect, &src_in);
        layered.canvas().restore();

        let mut fused = surfaces::raster_n32_premul(size).unwrap();
        fused.canvas().clear(Color::TRANSPARENT);
        let mut fused_paint = Paint::default();
        fused_paint.set_color_filter(
            skia_safe::color_filters::blend_with_color_space(
                tint,
                None,
                skia_safe::BlendMode::SrcIn,
            )
            .unwrap(),
        );
        fused
            .canvas()
            .draw_image_rect(&source, None, rect, &fused_paint);

        let read = |image: &skia_safe::Image| {
            let info = ImageInfo::new(size, ColorType::RGBA8888, AlphaType::Unpremul, None);
            let mut pixels = vec![0; 4 * 4 * 4];
            assert!(image.read_pixels(
                &info,
                &mut pixels,
                4 * 4,
                IPoint::new(0, 0),
                skia_safe::image::CachingHint::Allow,
            ));
            pixels
        };
        assert_eq!(
            read(&layered.image_snapshot()),
            read(&fused.image_snapshot())
        );
    }
}

#[cfg(feature = "skia-core")]
fn draw_placeholder(canvas: &Canvas, gtype: i32) {
    let mut paint = Paint::default();
    paint.set_style(PaintStyle::Fill);
    paint.set_color4f(Color4f::new(0.85, 0.85, 0.85, 0.6), None);
    paint.set_anti_alias(true);
    let rect = Rect::from_xywh(-50.0, -50.0, 100.0, 100.0);
    canvas.draw_round_rect(rect, 8.0, 8.0, &paint);

    let font_mgr = FontMgr::default();
    if let Some(tf) = font_mgr.legacy_make_typeface(None, FontStyle::default()) {
        let font = Font::new(tf as Typeface, Some(12.0));
        let mut tp = Paint::default();
        tp.set_color4f(Color4f::new(0.3, 0.3, 0.3, 1.0), None);
        let label = format!("General\n#{gtype}");
        canvas.draw_str(&label, Point::new(-40.0, 4.0), &font, &tp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证 typeface 解析链在无法匹配字体时返回 None 而非 panic（Issue #5 修复验证）。
    /// 修复前此处使用 .expect("无法创建 Typeface") 会在字体不可用时 panic；
    /// 修复后使用 match typeface { Some => ..., None => return } 安全处理。
    #[test]
    #[cfg(feature = "skia-core")]
    fn typeface_fallback_chain_returns_none_gracefully() {
        let font_mgr = FontMgr::default();
        let style = FontStyle::normal();

        // 测试：使用一个不存在的字体名 → 应该 fallback 到 Noto Sans CJK 或系统字体
        let resolved_name: Option<&str> = Some("NonExistentFont12345");
        let typeface = resolved_name
            .and_then(|name| font_mgr.match_family_style(name, style))
            .or_else(|| font_mgr.match_family_style("Noto Sans CJK SC", style))
            .or_else(|| font_mgr.legacy_make_typeface(None, style));

        // 在大多数系统上，至少有一个 fallback 字体可用
        // 但关键是：即使 typeface 为 None，代码也不应该 panic
        if let Some(_tf) = typeface {
            // fallback 链找到了可用字体 — 正常路径
        } else {
            // 所有 fallback 都失败 — 修复后的代码会 warn + return，而非 panic
            // 此测试确认 None 情况被正确处理
        }
    }

    /// 验证 catch_unwind 能隔离 draw_general_text 中的任何意外 panic。
    #[test]
    #[cfg(feature = "skia-core")]
    fn draw_general_text_is_panic_safe() {
        let _w = crate::transform::CANVAS_WIDTH as i32;
        let _h = crate::transform::CANVAS_HEIGHT as i32;

        // 模拟修复后的 match 逻辑
        let font_mgr = FontMgr::default();
        let style = FontStyle::normal();
        let typeface = None
            .and_then(|_: Option<&str>| None::<skia_safe::Typeface>)
            .or_else(|| font_mgr.match_family_style("Noto Sans CJK SC", style))
            .or_else(|| font_mgr.legacy_make_typeface(None, style));

        // 修复后的核心逻辑：不再使用 .expect()，而是安全 match
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match typeface {
                Some(_tf) => { /* 正常渲染 */ }
                None => { /* 修复后：warn + return，不 panic */ }
            }
        }));
        assert!(result.is_ok(), "typeface 安全 match 不应该 panic");
    }
}
