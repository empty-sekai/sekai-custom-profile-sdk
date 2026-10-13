//! 卡面合成公共工具。
//!
//! 稀有度、星标与裁切规则由 `sekai_profile_core::general_recipe`
//! 定义，这里只做转发。

use sekai_profile_core::general_recipe;

/// object-fit: cover 的源矩形计算，返回 `(x, y, w, h)`。
pub fn cover_crop_rect(src_w: f32, src_h: f32, dst_w: f32, dst_h: f32) -> (f32, f32, f32, f32) {
    let rect = general_recipe::cover_source_rect(src_w, src_h, dst_w, dst_h);
    (rect.x, rect.y, rect.width, rect.height)
}

/// 稀有度后缀映射。
pub fn rarity_suffix(rarity: &str) -> &str {
    general_recipe::card_rarity_suffix(rarity)
}

/// 稀有度对应的星级数量。
pub fn rarity_count(rarity: &str) -> usize {
    general_recipe::card_rarity_star_count(rarity)
}

/// 星图 key 映射。
pub fn star_icon_key(rarity: &str, trained: bool) -> &'static str {
    general_recipe::card_rarity_star_key(rarity, trained)
}
