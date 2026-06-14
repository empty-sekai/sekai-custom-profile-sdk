//! 通用图片类元素 adapter。

use crate::context::RenderContext;
#[cfg(feature = "skia-core")]
use crate::elements::image::draw_asset_image;
use crate::types::{
    CollectionElement, GeneralBackgroundElement, OtherElement, StampElement, StandMemberElement,
    StoryBackgroundElement,
};
use crate::widgets::Widget;

/// 印章元素 Widget adapter。
pub struct StampWidget {
    id: i32,
    asset_key: String,
}

impl StampWidget {
    /// 从印章元素构建 adapter。
    pub fn from_element(elem: &StampElement, ctx: &RenderContext<'_>) -> Self {
        let asset_key = ctx
            .masterdata
            .and_then(|md| md.resolve_stamp(elem.id))
            .map(|abn| format!("stamp/{abn}/{abn}"))
            .unwrap_or_else(|| format!("stamp/stamp{:04}/stamp{:04}", elem.id, elem.id));
        Self {
            id: elem.id,
            asset_key,
        }
    }
}

impl Widget for StampWidget {
    fn name(&self) -> &'static str {
        let _ = self.id;
        "stamp"
    }

    fn measure(&self, ctx: &RenderContext<'_>) -> (f32, f32) {
        image_size(ctx, &self.asset_key)
    }

    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, ctx: &RenderContext<'_>) {
        canvas.save();
        canvas.translate((x, y));
        draw_asset_image(canvas, Some(ctx.assets), &self.asset_key, "Stamp", self.id);
        canvas.restore();
    }

    fn asset_keys(&self, _ctx: &RenderContext<'_>) -> Vec<String> {
        vec![self.asset_key.clone()]
    }
}

/// 其它装饰元素 Widget adapter。
pub struct OtherWidget {
    id: i32,
    asset_key: String,
}

impl OtherWidget {
    /// 从其它装饰元素构建 adapter。
    pub fn from_element(elem: &OtherElement, ctx: &RenderContext<'_>) -> Option<Self> {
        let asset_key = ctx
            .masterdata
            .and_then(|md| md.resolve_resource("etc", elem.id))
            .map(|info| format!("{}/{}", info.load_val, info.file_name))?;
        Some(Self {
            id: elem.id,
            asset_key,
        })
    }
}

impl Widget for OtherWidget {
    fn name(&self) -> &'static str {
        let _ = self.id;
        "other"
    }

    fn measure(&self, ctx: &RenderContext<'_>) -> (f32, f32) {
        image_size(ctx, &self.asset_key)
    }

    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, ctx: &RenderContext<'_>) {
        canvas.save();
        canvas.translate((x, y));
        draw_asset_image(canvas, Some(ctx.assets), &self.asset_key, "Other", self.id);
        canvas.restore();
    }

    fn asset_keys(&self, _ctx: &RenderContext<'_>) -> Vec<String> {
        vec![self.asset_key.clone()]
    }
}

/// 收藏品元素 Widget adapter。
pub struct CollectionWidget {
    id: i32,
    asset_key: String,
}

impl CollectionWidget {
    /// 从收藏品元素构建 adapter。
    pub fn from_element(elem: &CollectionElement, ctx: &RenderContext<'_>) -> Option<Self> {
        let asset_key = ctx
            .masterdata
            .and_then(|md| md.resolve_resource("collection", elem.id))
            .map(|info| format!("{}/{}", info.load_val, info.file_name))?;
        Some(Self {
            id: elem.id,
            asset_key,
        })
    }
}

impl Widget for CollectionWidget {
    fn name(&self) -> &'static str {
        let _ = self.id;
        "collection"
    }

    fn measure(&self, ctx: &RenderContext<'_>) -> (f32, f32) {
        image_size(ctx, &self.asset_key)
    }

    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, ctx: &RenderContext<'_>) {
        canvas.save();
        canvas.translate((x, y));
        draw_asset_image(
            canvas,
            Some(ctx.assets),
            &self.asset_key,
            "Collection",
            self.id,
        );
        canvas.restore();
    }

    fn asset_keys(&self, _ctx: &RenderContext<'_>) -> Vec<String> {
        vec![self.asset_key.clone()]
    }
}

/// 站立立绘元素 Widget adapter。
pub struct StandMemberWidget {
    id: i32,
    asset_key: String,
}

impl StandMemberWidget {
    /// 从站立立绘元素构建 adapter。
    pub fn from_element(elem: &StandMemberElement, ctx: &RenderContext<'_>) -> Option<Self> {
        let asset_key = ctx
            .masterdata
            .and_then(|md| md.resolve_resource("standing", elem.id))
            .map(|info| format!("{}/{}", info.load_val, info.file_name))?;
        Some(Self {
            id: elem.id,
            asset_key,
        })
    }
}

impl Widget for StandMemberWidget {
    fn name(&self) -> &'static str {
        let _ = self.id;
        "stand_member"
    }

    fn measure(&self, ctx: &RenderContext<'_>) -> (f32, f32) {
        image_size(ctx, &self.asset_key)
    }

    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, ctx: &RenderContext<'_>) {
        canvas.save();
        canvas.translate((x, y));
        draw_asset_image(
            canvas,
            Some(ctx.assets),
            &self.asset_key,
            "StandMember",
            self.id,
        );
        canvas.restore();
    }

    fn asset_keys(&self, _ctx: &RenderContext<'_>) -> Vec<String> {
        vec![self.asset_key.clone()]
    }
}

/// 通用背景元素 Widget adapter。
pub struct GeneralBgWidget {
    id: i32,
    asset_key: String,
}

impl GeneralBgWidget {
    /// 从通用背景元素构建 adapter。
    pub fn from_element(elem: &GeneralBackgroundElement, ctx: &RenderContext<'_>) -> Option<Self> {
        let asset_key = ctx
            .masterdata
            .and_then(|md| md.resolve_resource("general_bg", elem.id))
            .map(|info| format!("{}/{}", info.load_val, info.file_name))?;
        Some(Self {
            id: elem.id,
            asset_key,
        })
    }
}

impl Widget for GeneralBgWidget {
    fn name(&self) -> &'static str {
        let _ = self.id;
        "general_background"
    }

    fn measure(&self, ctx: &RenderContext<'_>) -> (f32, f32) {
        image_size(ctx, &self.asset_key)
    }

    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, ctx: &RenderContext<'_>) {
        canvas.save();
        canvas.translate((x, y));
        draw_asset_image(
            canvas,
            Some(ctx.assets),
            &self.asset_key,
            "GeneralBg",
            self.id,
        );
        canvas.restore();
    }

    fn asset_keys(&self, _ctx: &RenderContext<'_>) -> Vec<String> {
        vec![self.asset_key.clone()]
    }
}

/// 剧情背景元素 Widget adapter。
pub struct StoryBgWidget {
    id: i32,
    asset_key: String,
}

impl StoryBgWidget {
    /// 从剧情背景元素构建 adapter。
    pub fn from_element(elem: &StoryBackgroundElement, ctx: &RenderContext<'_>) -> Option<Self> {
        let asset_key = ctx
            .masterdata
            .and_then(|md| md.resolve_resource("story_bg", elem.id))
            .map(|info| format!("{}/{}", info.load_val, info.file_name))?;
        Some(Self {
            id: elem.id,
            asset_key,
        })
    }
}

impl Widget for StoryBgWidget {
    fn name(&self) -> &'static str {
        let _ = self.id;
        "story_background"
    }

    fn measure(&self, ctx: &RenderContext<'_>) -> (f32, f32) {
        image_size(ctx, &self.asset_key)
    }

    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, ctx: &RenderContext<'_>) {
        canvas.save();
        canvas.translate((x, y));
        draw_asset_image(
            canvas,
            Some(ctx.assets),
            &self.asset_key,
            "StoryBg",
            self.id,
        );
        canvas.restore();
    }

    fn asset_keys(&self, _ctx: &RenderContext<'_>) -> Vec<String> {
        vec![self.asset_key.clone()]
    }
}

fn image_size(_ctx: &RenderContext<'_>, _asset_key: &str) -> (f32, f32) {
    #[cfg(feature = "skia-core")]
    {
        if let Some(image) = _ctx.assets.get_image(_asset_key) {
            return (image.width() as f32, image.height() as f32);
        }
    }
    (100.0, 100.0)
}

#[cfg(test)]
mod tests {
    use super::StampWidget;
    use crate::assets::AssetStore;
    use crate::context::RenderContext;
    use crate::types::{ObjectData, Quaternion, StampElement, Vec3};
    use crate::widgets::theme::Theme;
    use crate::widgets::Widget;

    fn stamp_element() -> StampElement {
        StampElement {
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
            id: 7,
        }
    }

    #[test]
    fn stamp_widget_has_asset_key_and_reasonable_measure() {
        let assets = AssetStore::new(8);
        let theme = Theme::default();
        let ctx = RenderContext::new(&assets, &theme);
        let widget = StampWidget::from_element(&stamp_element(), &ctx);

        assert_eq!(widget.name(), "stamp");
        assert_eq!(widget.asset_keys(&ctx), vec!["stamp/stamp0007/stamp0007"]);
        assert_eq!(widget.measure(&ctx), (100.0, 100.0));
    }
}
