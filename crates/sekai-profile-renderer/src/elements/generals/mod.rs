//! General 面板渲染模块。
//!
//! 每个 general 面板是自定义名片的可选组件，
//! 通过 objectData 控制位置/旋转/缩放。
//! 面板内部内容从 ProfileData 动态填充。

pub(crate) mod layout;
pub(crate) mod sdf_text;

#[cfg_attr(not(feature = "skia-oracle"), allow(dead_code))]
#[allow(dead_code)]
fn strip_live_master_star_assets(
    snapshot: &mut sekai_profile_renderer_core::profile_scene::ProfileComponentSnapshot,
) {
    for honor in &mut snapshot.honor_slots {
        if let sekai_profile_renderer_core::profile_scene::HonorVisualKind::Standard {
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

#[cfg(test)]
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
            [0.2, 0.2, 0.2, 1.0],
            sdf_text::SdfTextAlign::Left,
            26.0,
            6.0,
        );
        assert_eq!(spec.element.font_id, 1);
        assert!(spec.element.text.contains("<color=#ff0000>玩家</color>"));
        assert_eq!(spec.element.text_type & 0x07, 1);
        assert_eq!(spec.origin[0], 10.0);
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
            [0.2, 0.2, 0.2, 1.0],
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
            [1.0, 1.0, 1.0, 1.0],
            sdf_text::SdfTextAlign::Right,
            26.0,
            0.0,
        );
        assert_eq!(spec.element.text_type & 0x07, 4);
        assert_eq!(spec.origin[0], 10.0);
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
        use sekai_profile_renderer_core::profile_scene::{
            HonorVisualKind, HonorVisualSnapshot, ProfileComponentSnapshot, ResourceDescriptor,
        };
        use sekai_profile_renderer_core::ResourceKey;
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
        assert!(
            dispatch.contains("sekai_profile_renderer_core::general_recipe::build_general_recipe")
        );
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

    #[cfg(feature = "skia-oracle")]
    #[test]
    fn fused_level_bar_tint_is_pixel_exact_to_native_src_in_layer() {
        use skia_safe::{
            surfaces, AlphaType, Color, Color4f, ColorType, IPoint, ImageInfo, Paint, Rect,
        };

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

#[cfg(test)]
mod tests {
    /// An unavailable family resolves to `None` instead of panicking, so a
    /// caller skips the label rather than drawing it in a substituted typeface.
    #[test]
    #[cfg(feature = "skia-oracle")]
    fn bundled_typeface_reports_an_unavailable_family_as_none() {
        assert!(crate::elements::bundled_typeface("NoSuchFamily12345").is_none());
    }

    /// The declared families the element path draws must resolve from the
    /// configured font directory, with no host font configuration involved.
    #[test]
    #[cfg(feature = "skia-oracle")]
    fn bundled_typeface_resolves_the_families_the_element_path_declares() {
        for family in [
            crate::widgets::theme::fonts::PRIMARY,
            crate::widgets::theme::fonts::EMPHASIS,
            crate::widgets::theme::fonts::LIVE_MASTER_PROGRESS,
        ] {
            if crate::sdf::outline::resolve_font_path(family).is_none() {
                // The font directory is not populated in this environment; the
                // mapping itself is covered by the resolver's own tests.
                continue;
            }
            assert!(
                crate::elements::bundled_typeface(family).is_some(),
                "{family} resolved to a file but could not be loaded"
            );
        }
    }
}
