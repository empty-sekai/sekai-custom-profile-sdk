// Auto-split from generals/mod.rs

use super::*;
use crate::elements::image::cover_crop_source_rect;

pub(super) fn draw_player_avatar(
    canvas: &Canvas,
    profile: &ProfileData,
    md: &MasterData,
    assets: Option<&AssetStore>,
) {
    let size = 180.0;
    let radius = size / 2.0;

    // 创建圆形裁剪区域
    let path = {
        let mut b = skia_safe::PathBuilder::new();
        b.add_circle((0.0, 0.0), radius, skia_safe::PathDirection::CW);
        b.detach()
    };
    canvas.save();
    canvas.clip_path(&path, None, true);

    // 绘制默认背景占位
    let mut bg = Paint::default();
    bg.set_style(PaintStyle::Fill);
    bg.set_color4f(Color4f::new(0.85, 0.85, 0.9, 1.0), None);
    canvas.draw_circle((0.0, 0.0), radius, &bg);

    // 加载并绘制队长卡面缩略图头像
    if let (Some(lc), Some(store)) = (&profile.leader_card, assets) {
        let suffix = if lc.after_training {
            "after_training"
        } else {
            "normal"
        };
        // 使用缩略图路径 thumbnail/chara/{abn}_{suffix}
        if let Some(card) = md.get_card(lc.card_id) {
            let thumb_key = format!("thumbnail/chara/{}_{}", card.asset_bundle_name, suffix);
            if let Some(img) = store.get_image(&thumb_key) {
                let src =
                    cover_crop_source_rect(img.width() as f32, img.height() as f32, size, size);
                let dst = Rect::from_xywh(-radius, -radius, size, size);
                canvas.draw_image_rect(
                    img,
                    Some((&src, skia_safe::canvas::SrcRectConstraint::Fast)),
                    dst,
                    &Paint::default(),
                );
            }
        }
    }

    canvas.restore();

    // 绘制白色边框
    let mut border = Paint::default();
    border.set_style(PaintStyle::Stroke);
    border.set_stroke_width(4.0);
    border.set_color4f(Color4f::new(1.0, 1.0, 1.0, 0.85), None);
    border.set_anti_alias(true);
    canvas.draw_circle((0.0, 0.0), radius - 2.0, &border);
}
