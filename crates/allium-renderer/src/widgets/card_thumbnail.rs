//! 可复用卡面缩略图组件。

use crate::context::RenderContext;
#[cfg(feature = "skia-core")]
use crate::widgets::card_util::{
    cover_crop_rect, draw_info_bar, draw_stars_horizontal, rarity_count, InfoBarSpec,
};
use crate::widgets::card_util::{rarity_suffix, star_icon_key};
#[cfg(feature = "skia-core")]
use crate::widgets::theme;
use crate::widgets::Widget;

/// 卡面缩略图组件。
pub struct CardThumbnail {
    /// 输出尺寸，默认建议使用 `156.0`。
    pub size: f32,
    /// 卡面素材 key。
    pub card_image_key: String,
    /// 稀有度类型。
    pub rarity: String,
    /// 属性类型。
    pub attr: String,
    /// 突破等级。
    pub master_rank: i32,
    /// 是否为特训后卡面。
    pub trained: bool,
    /// 是否显示信息层。
    pub show_info: bool,
    /// 等级文字。
    pub level_text: String,
}

impl CardThumbnail {
    /// 创建卡面缩略图组件。
    pub fn new(size: f32, card_image_key: impl Into<String>) -> Self {
        Self {
            size,
            card_image_key: card_image_key.into(),
            rarity: "rarity_4".to_string(),
            attr: "cool".to_string(),
            master_rank: 0,
            trained: false,
            show_info: true,
            level_text: "Lv.1".to_string(),
        }
    }

    /// 预热缩略图缓存。
    ///
    /// 生产组卡链路会在搜索进行时提前准备候选卡缩略图；真正绘制 top5 时如果命中
    /// 同一渲染线程的线程本地缓存，就只需要贴图和绘制动态文本。
    pub fn prewarm(&self, ctx: &RenderContext<'_>) {
        #[cfg(feature = "skia-core")]
        {
            let _ = cached_thumbnail_image(self, ctx);
        }
        #[cfg(not(feature = "skia-core"))]
        {
            let _ = ctx;
        }
    }
}

#[cfg(feature = "skia-core")]
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct ThumbnailCacheKey {
    size_px: i32,
    card_image_key: String,
    rarity: String,
    attr: String,
    master_rank: i32,
    trained: bool,
    show_info: bool,
    level_text: String,
}

#[cfg(feature = "skia-core")]
thread_local! {
    static THUMBNAIL_CACHE: std::cell::RefCell<lru::LruCache<ThumbnailCacheKey, skia_safe::Image>> =
        std::cell::RefCell::new(lru::LruCache::new(std::num::NonZeroUsize::new(512).unwrap_or(std::num::NonZeroUsize::MIN)));
}

#[cfg(feature = "skia-core")]
fn thumbnail_cache_key(card: &CardThumbnail) -> ThumbnailCacheKey {
    ThumbnailCacheKey {
        size_px: card.size.round() as i32,
        card_image_key: card.card_image_key.clone(),
        rarity: card.rarity.clone(),
        attr: card.attr.clone(),
        master_rank: card.master_rank,
        trained: card.trained,
        show_info: card.show_info,
        level_text: card.level_text.clone(),
    }
}

#[cfg(feature = "skia-core")]
fn cached_thumbnail_image(
    card: &CardThumbnail,
    ctx: &RenderContext<'_>,
) -> Option<skia_safe::Image> {
    if card.size <= 0.0 || ctx.assets.get_image(&card.card_image_key).is_none() {
        return None;
    }

    let key = thumbnail_cache_key(card);
    THUMBNAIL_CACHE.with(|cache| {
        if let Some(image) = cache.borrow_mut().get(&key).cloned() {
            return Some(image);
        }

        let size = card.size.ceil() as i32;
        let mut surface = skia_safe::surfaces::raster_n32_premul((size, size))?;
        let canvas = surface.canvas();
        canvas.clear(skia_safe::Color::from_argb(0, 0, 0, 0));
        draw_thumbnail_uncached(card, canvas, 0.0, 0.0, ctx);
        let image = surface.image_snapshot();
        cache.borrow_mut().put(key, image.clone());
        Some(image)
    })
}

#[cfg(feature = "skia-core")]
fn draw_thumbnail_uncached(
    card: &CardThumbnail,
    canvas: &skia_safe::Canvas,
    x: f32,
    y: f32,
    ctx: &RenderContext<'_>,
) {
    let Some(card_image) = ctx.assets.get_image(&card.card_image_key) else {
        return;
    };
    let rect = skia_safe::Rect::from_xywh(x, y, card.size, card.size);
    let scale = card.size / theme::card_thumbnail::NATIVE_SIZE as f32;

    let clip_path = {
        let r = theme::card_thumbnail::CORNER_RADIUS * scale;
        let rrect = skia_safe::RRect::new_rect_xy(rect, r, r);
        let mut b = skia_safe::PathBuilder::new();
        b.add_rrect(rrect, None, None);
        b.detach()
    };

    canvas.save();
    canvas.clip_path(&clip_path, skia_safe::ClipOp::Intersect, true);

    let (sx, sy, sw, sh) = cover_crop_rect(
        card_image.width() as f32,
        card_image.height() as f32,
        card.size,
        card.size,
    );
    let src = skia_safe::Rect::from_xywh(sx, sy, sw, sh);
    canvas.draw_image_rect(
        &card_image,
        Some((&src, skia_safe::canvas::SrcRectConstraint::Fast)),
        rect,
        &skia_safe::Paint::default(),
    );

    if card.show_info {
        let bar_spec = InfoBarSpec {
            height: 32.0 * scale,
            font_size: 24.0 * scale,
            text_rect_size: (-52.5 * scale, 27.0 * scale),
            text_rect_pos: (-14.25 * scale, -17.0 * scale),
            y_offset: -2.0 * scale,
            tint: (68, 68, 102, 204),
        };
        draw_info_bar(canvas, Some(ctx.assets), rect, &card.level_text, &bar_spec);

        let store = ctx.assets;
        {
            let frame_key = format!("card/cardFrame_S_{}", rarity_suffix(&card.rarity));
            if let Some(frame) = store.get_image(&frame_key) {
                canvas.draw_image_rect(frame, None, rect, &skia_safe::Paint::default());
            }

            let attr_key = format!("card/icon_attribute_{}_64", card.attr);
            if let Some(attr) = store.get_image(&attr_key) {
                canvas.draw_image_rect(
                    attr,
                    None,
                    skia_safe::Rect::from_xywh(x + 2.0 * scale, y, 34.0 * scale, 36.0 * scale),
                    &skia_safe::Paint::default(),
                );
            }

            if let Some(star) = store.get_image(star_icon_key(&card.rarity, card.trained)) {
                draw_stars_horizontal(
                    canvas,
                    &star,
                    rarity_count(&card.rarity),
                    (x + 4.0 * scale, y + 100.0 * scale),
                    (22.0 * scale, 22.0 * scale),
                );
            }

            let rank_key = format!("card/masterRank_S_{}", card.master_rank.clamp(0, 5));
            if let Some(rank) = store.get_image(&rank_key) {
                let rank_size = 54.0 * scale;
                canvas.draw_image_rect(
                    rank,
                    None,
                    skia_safe::Rect::from_xywh(
                        x + card.size - rank_size,
                        y + card.size - rank_size,
                        rank_size,
                        rank_size,
                    ),
                    &skia_safe::Paint::default(),
                );
            }
        }
    }

    canvas.restore();
}

impl Widget for CardThumbnail {
    /// 返回组件名称。
    fn name(&self) -> &'static str {
        "card_thumbnail"
    }

    /// 测量缩略图宽高。
    fn measure(&self, _ctx: &RenderContext<'_>) -> (f32, f32) {
        (self.size, self.size)
    }

    /// 枚举卡面缩略图依赖的素材。
    fn asset_keys(&self, _ctx: &RenderContext<'_>) -> Vec<String> {
        let mut keys = vec![self.card_image_key.clone()];
        if self.show_info {
            keys.push("card/bg_base_wh".to_string());
            keys.push(format!("card/cardFrame_S_{}", rarity_suffix(&self.rarity)));
            keys.push(format!("card/icon_attribute_{}_64", self.attr));
            keys.push(star_icon_key(&self.rarity, self.trained).to_string());
            keys.push(format!(
                "card/masterRank_S_{}",
                self.master_rank.clamp(0, 5)
            ));
        }
        keys
    }

    /// 在指定位置绘制卡面缩略图。
    #[cfg(feature = "skia-core")]
    fn draw(&self, canvas: &skia_safe::Canvas, x: f32, y: f32, ctx: &RenderContext<'_>) {
        if let Some(image) = cached_thumbnail_image(self, ctx) {
            canvas.draw_image(image, (x, y), None);
        } else {
            draw_thumbnail_uncached(self, canvas, x, y, ctx);
        }
    }
}
