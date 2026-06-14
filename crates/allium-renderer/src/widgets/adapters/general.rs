//! General 面板元素 adapter。

use crate::asset_keys::collect_profile_asset_keys;
use crate::context::RenderContext;
use crate::types::GeneralElement;
use crate::widgets::Widget;

#[cfg(feature = "skia-core")]
use crate::elements::generals::draw_general;
#[cfg(feature = "skia-core")]
use crate::elements::image::draw_image_placeholder;

/// General 面板元素 Widget adapter。
pub struct GeneralWidget {
    general_type: i32,
    origin_centered: bool,
}

impl GeneralWidget {
    /// 从 General 元素构建 adapter。
    pub fn from_element(elem: &GeneralElement) -> Self {
        Self {
            general_type: elem.general_type.unwrap_or(0),
            origin_centered: true,
        }
    }

    /// 从 WidgetDocument 的 profile_general 节点构建 adapter。
    pub fn from_general_type(general_type: i32) -> Self {
        Self {
            general_type,
            origin_centered: false,
        }
    }
}

impl Widget for GeneralWidget {
    fn name(&self) -> &'static str {
        let _ = self.general_type;
        if self.origin_centered {
            "general"
        } else {
            "profile_general"
        }
    }

    fn measure(&self, _ctx: &RenderContext<'_>) -> (f32, f32) {
        profile_general_natural_size(self.general_type)
    }

    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, ctx: &RenderContext<'_>) {
        let (width, height) = self.measure(ctx);
        canvas.save();
        if self.origin_centered {
            canvas.translate((x, y));
        } else {
            canvas.translate((x + width / 2.0, y + height / 2.0));
        }
        match (ctx.profile, ctx.masterdata) {
            (Some(profile), Some(masterdata)) => {
                draw_general(
                    canvas,
                    self.general_type,
                    profile,
                    masterdata,
                    Some(ctx.assets),
                );
            }
            _ => draw_image_placeholder(canvas, "General", self.general_type),
        }
        canvas.restore();
    }

    fn asset_keys(&self, ctx: &RenderContext<'_>) -> Vec<String> {
        match (ctx.profile, ctx.masterdata) {
            (Some(profile), Some(masterdata)) => collect_profile_asset_keys(profile, masterdata),
            _ => Vec::new(),
        }
    }
}

/// 返回游戏 general 面板自然尺寸。
pub fn profile_general_natural_size(general_type: i32) -> (f32, f32) {
    match general_type {
        13 => (610.0, 127.0),
        2 => (813.0, 136.0),
        4 => (700.0, 251.0),
        3 => (844.0, 305.0),
        5 => (997.0, 589.0),
        6 => (844.0, 241.0),
        9 => (922.0, 228.0),
        10 => (921.0, 240.0),
        11 | 15 => (967.0, 872.0),
        12 => (939.0, 330.0),
        16 => (939.0, 300.0),
        14 => (967.0, 872.0),
        17 => (420.0, 68.0),
        18 => (180.0, 180.0),
        _ => (100.0, 100.0),
    }
}

#[cfg(test)]
mod tests {
    use super::GeneralWidget;
    use crate::assets::AssetStore;
    use crate::context::RenderContext;
    use crate::types::{GeneralElement, ObjectData, Quaternion, Vec3};
    use crate::widgets::theme::Theme;
    use crate::widgets::Widget;

    #[test]
    fn general_widget_has_reasonable_default_measure() {
        let widget = GeneralWidget::from_element(&GeneralElement {
            object_data: ObjectData {
                layer: 0,
                lock: false,
                position: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                rotation: Quaternion {
                    w: 1.0,
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                scale: Vec3 {
                    x: 1.0,
                    y: 1.0,
                    z: 1.0,
                },
                visible: true,
            },
            general_type: Some(3),
        });
        let assets = AssetStore::new(8);
        let theme = Theme::default();
        let ctx = RenderContext::new(&assets, &theme);

        assert_eq!(widget.name(), "general");
        assert_eq!(widget.measure(&ctx), (844.0, 305.0));
    }

    #[test]
    fn general_widget_from_document_uses_profile_general_name() {
        let widget = GeneralWidget::from_general_type(13);
        let assets = AssetStore::new(8);
        let theme = Theme::default();
        let ctx = RenderContext::new(&assets, &theme);

        assert_eq!(widget.name(), "profile_general");
        assert_eq!(widget.measure(&ctx), (610.0, 127.0));
    }
}
