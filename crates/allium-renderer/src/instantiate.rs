//! WidgetNode 到 `Box<dyn Widget>` 的实例化适配层。

use crate::widget_node::{Layout, TextAlignValue, VAlignValue, WidgetNode};
use crate::widgets::adapters::general::GeneralWidget;
use crate::widgets::card_thumbnail::CardThumbnail;
use crate::widgets::glass_panel::GlassPanel;
use crate::widgets::image::AssetImage;
use crate::widgets::panel::Panel;
use crate::widgets::stats_badge::StatsBadge;
use crate::widgets::text::{HAlign, SimpleText, VAlign};
use crate::widgets::text_badge::TextBadge;
use crate::widgets::Widget;

/// 将 WidgetNode 转换为 `Box<dyn Widget>`。
pub fn instantiate(node: &WidgetNode) -> Box<dyn Widget> {
    match node {
        WidgetNode::Container { id, layout, .. } => Box::new(ContainerWidget {
            id: id.clone(),
            layout: layout.clone(),
        }),
        WidgetNode::CardThumbnail {
            size,
            card_image_key,
            rarity,
            attr,
            master_rank,
            trained,
            show_info,
            level_text,
            ..
        } => Box::new(CardThumbnail {
            size: *size,
            card_image_key: card_image_key.clone(),
            rarity: rarity.clone(),
            attr: attr.clone(),
            master_rank: *master_rank,
            trained: *trained,
            show_info: *show_info,
            level_text: level_text.clone(),
        }),
        WidgetNode::GlassPanel {
            width,
            height,
            clip_variance,
            ..
        } => Box::new(GlassPanel {
            width: *width,
            height: *height,
            clip_variance: *clip_variance,
        }),
        WidgetNode::Panel {
            width,
            height,
            radius,
            fill,
            border,
            border_width,
            ..
        } => Box::new(Panel {
            width: *width,
            height: *height,
            radius: *radius,
            fill: *fill,
            border: *border,
            border_width: *border_width,
        }),
        WidgetNode::AssetImage {
            asset_key,
            width,
            height,
            fit,
            radius,
            ..
        } => Box::new(AssetImage {
            asset_key: asset_key.clone(),
            width: *width,
            height: *height,
            fit: *fit,
            radius: *radius,
        }),
        WidgetNode::SimpleText {
            content,
            font_size,
            color,
            width,
            height,
            align,
            v_align,
            padding,
            line_height,
            glow,
            ..
        } => Box::new(SimpleText {
            text: content.clone(),
            size: *font_size,
            color: *color,
            width: *width,
            height: *height,
            h_align: instantiate_h_align(*align),
            v_align: instantiate_v_align(*v_align),
            padding: *padding,
            line_height: *line_height,
            glow: *glow,
        }),
        WidgetNode::StatsBadge {
            label,
            value,
            color,
            is_highlight,
            ..
        } => Box::new(StatsBadge {
            label: label.clone(),
            value: value.clone(),
            color: *color,
            is_highlight: *is_highlight,
        }),
        WidgetNode::TextBadge {
            text,
            bg_color,
            text_color,
            ..
        } => Box::new(TextBadge {
            text: text.clone(),
            bg_color: *bg_color,
            text_color: *text_color,
        }),
        WidgetNode::ProfileGeneral { general_type, .. } => {
            Box::new(GeneralWidget::from_general_type(*general_type))
        }
    }
}

fn instantiate_h_align(align: TextAlignValue) -> HAlign {
    match align {
        TextAlignValue::Left => HAlign::Left,
        TextAlignValue::Center => HAlign::Center,
        TextAlignValue::Right => HAlign::Right,
    }
}

fn instantiate_v_align(align: VAlignValue) -> VAlign {
    match align {
        VAlignValue::Top => VAlign::Top,
        VAlignValue::Middle => VAlign::Middle,
        VAlignValue::Bottom => VAlign::Bottom,
    }
}

struct ContainerWidget {
    id: String,
    layout: Layout,
}

impl Widget for ContainerWidget {
    fn name(&self) -> &'static str {
        let _ = (&self.id, &self.layout);
        "container"
    }

    fn measure(&self, _ctx: &crate::context::RenderContext<'_>) -> (f32, f32) {
        (0.0, 0.0)
    }

    #[cfg(feature = "skia-core")]
    fn draw(
        &self,
        _canvas: &skia_safe::Canvas,
        _x: f32,
        _y: f32,
        _ctx: &crate::context::RenderContext<'_>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::instantiate;
    use crate::widget_node::{Layout, TextAlignValue, VAlignValue, WidgetNode};
    use crate::widgets::theme::Color;

    #[test]
    fn instantiate_returns_correct_names_for_all_variants() {
        let nodes = vec![
            WidgetNode::Container {
                id: "root".to_string(),
                layout: Layout::Absolute,
                children: Vec::new(),
            },
            WidgetNode::CardThumbnail {
                id: "card".to_string(),
                size: 156.0,
                card_image_key: "card/1".to_string(),
                rarity: "rarity_4".to_string(),
                attr: "cool".to_string(),
                master_rank: 1,
                trained: false,
                show_info: true,
                level_text: "Lv.1".to_string(),
            },
            WidgetNode::GlassPanel {
                id: "glass".to_string(),
                width: 100.0,
                height: 40.0,
                clip_variance: 0.0,
            },
            WidgetNode::Panel {
                id: "panel".to_string(),
                width: 100.0,
                height: 40.0,
                radius: 4.0,
                fill: Color::new(1.0, 1.0, 1.0, 1.0),
                border: None,
                border_width: 0.0,
            },
            WidgetNode::AssetImage {
                id: "image".to_string(),
                asset_key: "img/key".to_string(),
                width: 64.0,
                height: 64.0,
                fit: crate::widgets::image::AssetImageFit::Cover,
                radius: 4.0,
            },
            WidgetNode::SimpleText {
                id: "text".to_string(),
                content: "hello".to_string(),
                font_size: 18.0,
                color: Color::new(1.0, 1.0, 1.0, 1.0),
                width: 260.0,
                height: 72.0,
                align: TextAlignValue::Left,
                v_align: VAlignValue::Top,
                padding: 4.0,
                line_height: 1.2,
                glow: false,
            },
            WidgetNode::StatsBadge {
                id: "stats".to_string(),
                label: "L".to_string(),
                value: "V".to_string(),
                color: Color::new(1.0, 0.0, 0.0, 1.0),
                is_highlight: false,
            },
            WidgetNode::TextBadge {
                id: "badge".to_string(),
                text: "TAG".to_string(),
                bg_color: Color::new(0.0, 0.0, 0.0, 1.0),
                text_color: Color::new(1.0, 1.0, 1.0, 1.0),
            },
            WidgetNode::ProfileGeneral {
                id: "profile_name".to_string(),
                general_type: 13,
            },
        ];

        let names = nodes
            .iter()
            .map(|node| instantiate(node).name().to_string())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "container",
                "card_thumbnail",
                "glass_panel",
                "panel",
                "asset_image",
                "simple_text",
                "stats_badge",
                "text_badge",
                "profile_general",
            ]
        );
    }
}
