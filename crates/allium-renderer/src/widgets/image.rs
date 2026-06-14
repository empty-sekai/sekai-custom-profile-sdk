//! 通用素材图片组件。

use crate::context::RenderContext;

use super::Widget;

/// 图片填充方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetImageFit {
    /// 拉伸填满目标矩形。
    Fill,
    /// 等比覆盖目标矩形，裁掉多余区域。
    Cover,
    /// 等比完整放入目标矩形，可能留空。
    Contain,
}

/// 素材图片。
pub struct AssetImage {
    /// 素材 key。
    pub asset_key: String,
    /// 输出宽度。
    pub width: f32,
    /// 输出高度。
    pub height: f32,
    /// 填充方式。
    pub fit: AssetImageFit,
    /// 圆角半径。
    pub radius: f32,
}

impl Widget for AssetImage {
    fn name(&self) -> &'static str {
        "asset_image"
    }

    fn measure(&self, _ctx: &RenderContext<'_>) -> (f32, f32) {
        (self.width, self.height)
    }

    fn asset_keys(&self, _ctx: &RenderContext<'_>) -> Vec<String> {
        vec![self.asset_key.clone()]
    }

    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, ctx: &RenderContext<'_>) {
        let Some(image) = ctx.assets.get_image(&self.asset_key) else {
            return;
        };
        let dst = skia_safe::Rect::from_xywh(x, y, self.width, self.height);
        let clip_path = {
            let rrect = skia_safe::RRect::new_rect_xy(dst, self.radius, self.radius);
            let mut b = skia_safe::PathBuilder::new();
            b.add_rrect(rrect, None, None);
            b.detach()
        };

        canvas.save();
        canvas.clip_path(&clip_path, skia_safe::ClipOp::Intersect, true);

        match self.fit {
            AssetImageFit::Fill => {
                canvas.draw_image_rect(image, None, dst, &skia_safe::Paint::default());
            }
            AssetImageFit::Cover => {
                let (sx, sy, sw, sh) = crate::widgets::card_util::cover_crop_rect(
                    image.width() as f32,
                    image.height() as f32,
                    self.width,
                    self.height,
                );
                let src = skia_safe::Rect::from_xywh(sx, sy, sw, sh);
                canvas.draw_image_rect(
                    image,
                    Some((&src, skia_safe::canvas::SrcRectConstraint::Fast)),
                    dst,
                    &skia_safe::Paint::default(),
                );
            }
            AssetImageFit::Contain => {
                let iw = image.width() as f32;
                let ih = image.height() as f32;
                let scale = (self.width / iw).min(self.height / ih);
                let w = iw * scale;
                let h = ih * scale;
                let contained = skia_safe::Rect::from_xywh(
                    x + (self.width - w) * 0.5,
                    y + (self.height - h) * 0.5,
                    w,
                    h,
                );
                canvas.draw_image_rect(image, None, contained, &skia_safe::Paint::default());
            }
        }

        canvas.restore();
    }
}
