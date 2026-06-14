use crate::assets::AssetStore;
use crate::widgets::card_util::{
    cover_crop_rect, draw_stars_horizontal, rarity_count, rarity_suffix, star_icon_key,
};
use skia_safe::{Canvas, Color4f, Font, FontMgr, FontStyle, Paint, Point, Rect};

const CARD_CROP_WIDTH: f32 = 312.0;
const CARD_CROP_HEIGHT: f32 = 512.0;
const TYPE1_RAW_ROOT_W: f32 = 328.0;
const TYPE1_RAW_ROOT_H: f32 = 520.0;
const TYPE1_SCALE_X: f32 = CARD_CROP_WIDTH / TYPE1_RAW_ROOT_W;
const TYPE1_SCALE_Y: f32 = CARD_CROP_HEIGHT / TYPE1_RAW_ROOT_H;
const CARD_INFO_BAR_HEIGHT: f32 = 56.39 * TYPE1_SCALE_Y;
const CARD_INFO_FONT_SIZE: f32 = 24.0;
const CARD_INFO_BAR_TINT: (u8, u8, u8, u8) = (68, 68, 102, 255);
const CARD_INFO_TEXT_LEFT: f32 = 12.9 * TYPE1_SCALE_X;
const CARD_INFO_ATTR_X: f32 = 8.0 * TYPE1_SCALE_X;
const CARD_INFO_ATTR_W: f32 = 64.0 * TYPE1_SCALE_X;
const CARD_INFO_ATTR_H: f32 = 68.0 * TYPE1_SCALE_Y;
const CARD_INFO_STAR_X: f32 = 5.0;
const CARD_INFO_STAR_BOTTOM_Y: f32 = 64.0;
const CARD_INFO_STAR_SIZE: f32 = 40.0;
const CARD_INFO_MASTER_W: f32 = 88.0 * TYPE1_SCALE_X;
const CARD_INFO_MASTER_H: f32 = 88.0 * TYPE1_SCALE_Y;
const CARD_INFO_MASTER_RIGHT_X: f32 = 1.4 * TYPE1_SCALE_X;
const CARD_INFO_MASTER_BOTTOM_Y: f32 = 0.8 * TYPE1_SCALE_Y;

/// 卡面 badge 渲染所需的元数据。
#[derive(Debug, Clone)]
pub struct CardBadgeData {
    /// 稀有度类型。
    pub rarity: String,
    /// 属性类型。
    pub attr: String,
    /// 突破等级。
    pub master_rank: i32,
    /// 是否为特训后卡面。
    pub trained: bool,
    /// 当前卡牌等级。
    pub level: i32,
}

fn centered_card_rect() -> Rect {
    Rect::from_xywh(
        -CARD_CROP_WIDTH / 2.0,
        -CARD_CROP_HEIGHT / 2.0,
        CARD_CROP_WIDTH,
        CARD_CROP_HEIGHT,
    )
}

fn draw_full_image(canvas: &Canvas, image: &skia_safe::Image, dst: Rect) {
    canvas.draw_image_rect(image, None, dst, &Paint::default());
}

fn draw_static_if_present(canvas: &Canvas, assets: Option<&AssetStore>, key: &str, dst: Rect) {
    if let Some(image) = assets.and_then(|store| store.get_image(key)) {
        draw_full_image(canvas, &image, dst);
    }
}

fn draw_member_crop(canvas: &Canvas, image: &skia_safe::Image) {
    let crop_h = CARD_CROP_HEIGHT.min(image.height() as f32);
    let crop_x = (image.width() as f32 - CARD_CROP_WIDTH).max(0.0) / 2.0;
    let src = Rect::from_xywh(
        crop_x,
        0.0,
        CARD_CROP_WIDTH.min(image.width() as f32),
        crop_h,
    );
    let dst = centered_card_rect();
    canvas.draw_image_rect(
        image,
        Some((&src, skia_safe::canvas::SrcRectConstraint::Fast)),
        dst,
        &Paint::default(),
    );
}

fn draw_card_info_badges(canvas: &Canvas, assets: Option<&AssetStore>, badge: &CardBadgeData) {
    let card_rect = centered_card_rect();
    let level_text = format!("Lv.{}", badge.level);
    draw_type1_info_bar(canvas, assets, card_rect, &level_text);

    let suffix = rarity_suffix(&badge.rarity);
    draw_static_if_present(
        canvas,
        assets,
        &format!("card/cardFrame_M_{suffix}"),
        card_rect,
    );
    draw_static_if_present(
        canvas,
        assets,
        &format!("card/icon_attribute_{}_64", badge.attr),
        Rect::from_xywh(
            card_rect.left + CARD_INFO_ATTR_X,
            card_rect.top,
            CARD_INFO_ATTR_W,
            CARD_INFO_ATTR_H,
        ),
    );

    if let Some(star) =
        assets.and_then(|store| store.get_image(star_icon_key(&badge.rarity, badge.trained)))
    {
        draw_stars_horizontal(
            canvas,
            &star,
            rarity_count(&badge.rarity),
            (
                card_rect.left + CARD_INFO_STAR_X,
                card_rect.bottom - CARD_INFO_STAR_BOTTOM_Y - CARD_INFO_STAR_SIZE,
            ),
            (CARD_INFO_STAR_SIZE, CARD_INFO_STAR_SIZE),
        );
    }

    draw_static_if_present(
        canvas,
        assets,
        &format!("card/masterRank_S_{}", badge.master_rank.clamp(0, 5)),
        Rect::from_xywh(
            card_rect.right - CARD_INFO_MASTER_W - CARD_INFO_MASTER_RIGHT_X,
            card_rect.bottom - CARD_INFO_MASTER_BOTTOM_Y - CARD_INFO_MASTER_H,
            CARD_INFO_MASTER_W,
            CARD_INFO_MASTER_H,
        ),
    );
}

fn draw_type1_info_bar(canvas: &Canvas, assets: Option<&AssetStore>, card_rect: Rect, text: &str) {
    let bar_rect = Rect::from_xywh(
        card_rect.left,
        card_rect.bottom - CARD_INFO_BAR_HEIGHT,
        card_rect.width(),
        CARD_INFO_BAR_HEIGHT,
    );

    let layer = skia_safe::canvas::SaveLayerRec::default().bounds(&bar_rect);
    canvas.save_layer(&layer);
    if let Some(texture) = assets.and_then(|store| store.get_image("card/bg_base_wh")) {
        canvas.draw_image_rect(texture, None, bar_rect, &Paint::default());
        let mut tint = Paint::default();
        tint.set_anti_alias(true);
        tint.set_blend_mode(skia_safe::BlendMode::SrcIn);
        tint.set_color4f(
            Color4f::new(
                CARD_INFO_BAR_TINT.0 as f32 / 255.0,
                CARD_INFO_BAR_TINT.1 as f32 / 255.0,
                CARD_INFO_BAR_TINT.2 as f32 / 255.0,
                CARD_INFO_BAR_TINT.3 as f32 / 255.0,
            ),
            None,
        );
        canvas.draw_rect(bar_rect, &tint);
    }
    canvas.restore();

    let font_mgr = FontMgr::default();
    let typeface = font_mgr
        .match_family_style(crate::widgets::theme::fonts::EMPHASIS, FontStyle::normal())
        .or_else(|| {
            font_mgr.match_family_style(crate::widgets::theme::fonts::PRIMARY, FontStyle::normal())
        })
        .or_else(|| font_mgr.legacy_make_typeface(None, FontStyle::normal()));
    let Some(typeface) = typeface else {
        return;
    };

    let font = Font::new(typeface, Some(CARD_INFO_FONT_SIZE));
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color4f(Color4f::new(1.0, 1.0, 1.0, 1.0), None);
    let bbox = font.measure_str(text, Some(&paint)).1;
    let x = bar_rect.left + CARD_INFO_TEXT_LEFT - bbox.left;
    let y = bar_rect.top + (bar_rect.height() - bbox.height()) / 2.0 - bbox.top;
    canvas.draw_str(text, (x, y), &font, &paint);
}

fn draw_card_member_small_badges(
    canvas: &Canvas,
    assets: Option<&AssetStore>,
    badge: &CardBadgeData,
    card_rect: Rect,
) {
    let suffix = rarity_suffix(&badge.rarity);
    draw_static_if_present(
        canvas,
        assets,
        &format!("card/cardFrame_L_{suffix}"),
        card_rect,
    );
    draw_static_if_present(
        canvas,
        assets,
        &format!("card/icon_attribute_{}_88", badge.attr),
        Rect::from_xywh(card_rect.right - 88.0 - 40.0, card_rect.top, 88.0, 92.0),
    );

    if let Some(star) =
        assets.and_then(|store| store.get_image(star_icon_key(&badge.rarity, badge.trained)))
    {
        crate::widgets::card_util::draw_stars_vertical(
            canvas,
            &star,
            rarity_count(&badge.rarity),
            (card_rect.left + 24.0, card_rect.bottom - 208.0 - 17.0),
            (56.0, 56.0),
            48.0,
            4,
        );
    }

    draw_static_if_present(
        canvas,
        assets,
        &format!("card/masterRank_L_{}", badge.master_rank.clamp(0, 5)),
        Rect::from_xywh(
            card_rect.right - 104.0 - 24.0,
            card_rect.bottom - 104.0 - 24.0,
            104.0,
            104.0,
        ),
    );
}

/// type=1 卡面裁切渲染。
///
/// 源图为 `member_cutout` 600×576，居中裁切 312×512 区域后绘制。
/// 当 `badge` 存在时，按 info 模式继续叠加等级条、边框、属性、星级和突破图标。
///
/// 返回值：`true` 表示使用了真实素材渲染，`false` 表示绘制了占位符。
pub fn draw_card_member_cropped(
    canvas: &Canvas,
    assets: Option<&AssetStore>,
    asset_key: &str,
    id: i32,
    badge: Option<CardBadgeData>,
) -> bool {
    if let Some(store) = assets {
        if let Some(img) = store.get_image(asset_key) {
            draw_member_crop(canvas, &img);
            if let Some(badge) = badge.as_ref() {
                draw_card_info_badges(canvas, assets, badge);
            }
            return true;
        }
    }
    draw_image_placeholder(canvas, "CardMember[crop]", id);
    false
}

/// 绘制 `member_small` 卡面。
///
/// 当 `badge` 存在时，按大卡样式叠加 frame、属性、星级和突破图标。
pub fn draw_card_member_small(
    canvas: &Canvas,
    assets: Option<&AssetStore>,
    asset_key: &str,
    id: i32,
    badge: Option<CardBadgeData>,
) {
    if let Some(store) = assets {
        if let Some(img) = store.get_image(asset_key) {
            let card_rect = Rect::from_xywh(
                -(img.width() as f32) / 2.0,
                -(img.height() as f32) / 2.0,
                img.width() as f32,
                img.height() as f32,
            );
            draw_full_image(canvas, &img, card_rect);
            if let Some(badge) = badge.as_ref() {
                draw_card_member_small_badges(canvas, assets, badge, card_rect);
            }
            return;
        }
    }
    draw_image_placeholder(canvas, "CardMember[small]", id);
}

/// 绘制图片素材元素。
pub fn draw_asset_image(
    canvas: &Canvas,
    assets: Option<&AssetStore>,
    asset_key: &str,
    type_name: &str,
    id: i32,
) {
    if let Some(store) = assets {
        if let Some(img) = store.get_image(asset_key) {
            let iw = img.width() as f32;
            let ih = img.height() as f32;
            let dst = Rect::from_xywh(-iw / 2.0, -ih / 2.0, iw, ih);
            let src = Rect::from_xywh(0.0, 0.0, iw, ih);
            let paint = Paint::default();

            canvas.draw_image_rect(
                img,
                Some((&src, skia_safe::canvas::SrcRectConstraint::Fast)),
                dst,
                &paint,
            );
            return;
        }
    }
    draw_image_placeholder(canvas, type_name, id);
}

/// 图片元素占位绘制。
pub fn draw_image_placeholder(canvas: &Canvas, type_name: &str, id: i32) {
    let size = 100.0;
    let half = size / 2.0;

    let mut bg = Paint::default();
    bg.set_style(skia_safe::PaintStyle::Fill);
    bg.set_color4f(Color4f::new(0.85, 0.85, 0.85, 0.6), None);
    bg.set_anti_alias(true);
    let rect = Rect::from_xywh(-half, -half, size, size);
    canvas.draw_round_rect(rect, 8.0, 8.0, &bg);

    let mut border = Paint::default();
    border.set_style(skia_safe::PaintStyle::Stroke);
    border.set_stroke_width(1.0);
    border.set_color4f(Color4f::new(0.5, 0.5, 0.5, 0.8), None);
    border.set_anti_alias(true);
    canvas.draw_round_rect(rect, 8.0, 8.0, &border);

    let font_mgr = FontMgr::default();
    if let Some(tf) = font_mgr.legacy_make_typeface(None, FontStyle::default()) {
        let font = Font::new(tf, Some(10.0));
        let mut tp = Paint::default();
        tp.set_color4f(Color4f::new(0.3, 0.3, 0.3, 1.0), None);
        let label = format!("{type_name}\n#{id}");
        canvas.draw_str(&label, Point::new(-half + 4.0, 0.0), &font, &tp);
    }
}

/// 计算 cover 裁切后的源矩形。
pub fn cover_crop_source_rect(src_w: f32, src_h: f32, dst_w: f32, dst_h: f32) -> Rect {
    let (x, y, w, h) = cover_crop_rect(src_w, src_h, dst_w, dst_h);
    Rect::from_xywh(x, y, w, h)
}
