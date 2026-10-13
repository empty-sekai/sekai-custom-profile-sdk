//! General 面板渲染模块。
//!
//! 每个 general 面板是自定义名片的可选组件，
//! 通过 objectData 控制位置/旋转/缩放。
//! 面板内部内容从 ProfileData 动态填充。

pub(crate) mod layout;
pub(crate) mod sdf_text;

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
    fn general_text_never_applies_a_canvas_clip() {
        let source = include_str!("sdf_text.rs");
        assert!(!source.contains(&["canvas.clip_", "rect(spec.clip"].concat()));
    }
}
