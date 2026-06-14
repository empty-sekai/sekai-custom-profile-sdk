//! 羁绊称号渲染。

use super::common::draw_placeholder;
use crate::assets::AssetStore;
use crate::masterdata::MasterData;
use crate::types::BondsHonorEntry;
use skia_safe::{Canvas, Paint, Rect};

/// 绘制羁绊称号。
pub fn render_bonds_honor(
    canvas: &Canvas,
    bonds_honor_id: i32,
    honor_level: i32,
    full_size: bool,
    word_id: i64,
    inverse: bool,
    use_unit_vs: bool,
    md: &MasterData,
    assets: &AssetStore,
) {
    let bonds_entry = match md.get_bonds_honor(bonds_honor_id) {
        Some(entry) => entry,
        None => {
            tracing::warn!(bonds_honor_id, "BondsHonor 未找到");
            let (w, h) = if full_size {
                (380.0, 80.0)
            } else {
                (180.0, 80.0)
            };
            draw_placeholder(canvas, "BondsHonor", bonds_honor_id, w, h);
            return;
        }
    };

    let (cid1, cid2) = if inverse {
        (
            bonds_entry.game_character_unit_id2,
            bonds_entry.game_character_unit_id1,
        )
    } else {
        (
            bonds_entry.game_character_unit_id1,
            bonds_entry.game_character_unit_id2,
        )
    };
    let (sd_cid1, sd_cid2) = if use_unit_vs {
        (
            md.resolve_unit_vs_sd(cid1, cid2),
            md.resolve_unit_vs_sd(cid2, cid1),
        )
    } else {
        (cid1, cid2)
    };

    let (w, h) = if full_size {
        (380.0, 80.0)
    } else {
        (180.0, 80.0)
    };
    let mask_key = if full_size {
        "honor/mask_degree_main"
    } else {
        "honor/mask_degree_sub"
    };

    let layer_bounds = Rect::from_xywh(-w / 2.0, -h / 2.0, w, h);
    canvas.save_layer(&skia_safe::canvas::SaveLayerRec::default().bounds(&layer_bounds));
    render_bonds_bg(canvas, cid1, cid2, full_size, w, h, assets);
    render_bonds_sd(canvas, sd_cid1, sd_cid2, full_size, w, h, assets);

    if let Some(mask_img) = assets.get_image(mask_key) {
        let mut mask_paint = Paint::default();
        mask_paint.set_blend_mode(skia_safe::BlendMode::DstIn);
        canvas.draw_image_rect(
            mask_img,
            None,
            Rect::from_xywh(-w / 2.0, -h / 2.0, w, h),
            &mask_paint,
        );
    }
    canvas.restore();

    render_bonds_frame_and_stars(
        canvas,
        &bonds_entry,
        full_size,
        word_id,
        honor_level,
        w,
        h,
        md,
        assets,
    );
}

fn render_bonds_bg(
    canvas: &Canvas,
    cid1: i32,
    cid2: i32,
    full_size: bool,
    w: f32,
    h: f32,
    assets: &AssetStore,
) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    let (bg1_key, bg2_key) = if full_size {
        (
            format!("honor/bonds/{}", cid1),
            format!("honor/bonds/{}", cid2),
        )
    } else {
        (
            format!("honor/bonds/{}_sub", cid1),
            format!("honor/bonds/{}_sub", cid2),
        )
    };
    if let Some(bg1) = assets.get_image(&bg1_key) {
        let iw = bg1.width() as f32;
        let ih = bg1.height() as f32;
        let half_iw = iw / 2.0;
        let hw = w / 2.0;
        canvas.draw_image_rect(
            bg1,
            Some((
                &Rect::from_xywh(0.0, 0.0, half_iw, ih),
                skia_safe::canvas::SrcRectConstraint::Strict,
            )),
            Rect::from_xywh(-w / 2.0, -h / 2.0, hw, h),
            &paint,
        );
    }
    if let Some(bg2) = assets.get_image(&bg2_key) {
        let iw = bg2.width() as f32;
        let ih = bg2.height() as f32;
        let half_iw = iw / 2.0;
        let hw = w / 2.0;
        canvas.draw_image_rect(
            bg2,
            Some((
                &Rect::from_xywh(half_iw, 0.0, iw - half_iw, ih),
                skia_safe::canvas::SrcRectConstraint::Strict,
            )),
            Rect::from_xywh(0.0, -h / 2.0, hw, h),
            &paint,
        );
    }
}

fn render_bonds_sd(
    canvas: &Canvas,
    cid1: i32,
    cid2: i32,
    full_size: bool,
    _w: f32,
    h: f32,
    assets: &AssetStore,
) {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    let sd1_key = format!("bonds_honor/chr_sd_{:02}_01", cid1);
    let sd2_key = format!("bonds_honor/chr_sd_{:02}_01", cid2);

    // 对齐 Haruki 画法：每个 SD 角色按原始分辨率 0.8 倍缩放（保留角色间尺寸差异，
    // 不再统一拉到同一高度），脸部锚点在中线两侧 offset_to_mid 处，
    // 越过中线的部分裁掉避免两角色重叠。底部对齐 honor 底边。
    const SCALE: f32 = 0.8;
    let offset_to_mid = if full_size { 120.0 } else { 30.0 };

    // 左角色：脸部锚在 x=-offset_to_mid，右缘不越过中线 (x=0)
    if let Some(sd1) = assets.get_image(&sd1_key) {
        let (nw, nh) = (sd1.width() as f32, sd1.height() as f32);
        let (sw, sh) = (nw * SCALE, nh * SCALE);
        let dst_left = -offset_to_mid - sw / 2.0;
        let dst_top = h / 2.0 - sh;
        let overlap = dst_left + sw; // 右缘越过中线的量
        let (src_w, draw_w) = if overlap > 0.0 {
            (nw - overlap / SCALE, sw - overlap)
        } else {
            (nw, sw)
        };
        let src = Rect::from_xywh(0.0, 0.0, src_w, nh);
        let dst = Rect::from_xywh(dst_left, dst_top, draw_w, sh);
        tracing::debug!(
            slot = "sd1", cid = cid1, full_size, orig_w = sd1.width(), orig_h = sd1.height(),
            scale = SCALE, dst_x = dst.left, dst_y = dst.top, dst_w = dst.width(), dst_h = dst.height(),
            src_w, overlap, "bonds SD 绘制几何"
        );
        canvas.draw_image_rect(
            sd1,
            Some((&src, skia_safe::canvas::SrcRectConstraint::Strict)),
            dst,
            &paint,
        );
    }

    // 右角色：脸部锚在 x=+offset_to_mid，左缘不越过中线 (x=0)
    if let Some(sd2) = assets.get_image(&sd2_key) {
        let (nw, nh) = (sd2.width() as f32, sd2.height() as f32);
        let (sw, sh) = (nw * SCALE, nh * SCALE);
        let dst_left = offset_to_mid - sw / 2.0;
        let dst_top = h / 2.0 - sh;
        let overlap = -dst_left; // 左缘越过中线的量
        let (src_x, draw_left, draw_w) = if overlap > 0.0 {
            (overlap / SCALE, 0.0, sw - overlap)
        } else {
            (0.0, dst_left, sw)
        };
        let src = Rect::from_xywh(src_x, 0.0, nw - src_x, nh);
        let dst = Rect::from_xywh(draw_left, dst_top, draw_w, sh);
        tracing::debug!(
            slot = "sd2", cid = cid2, full_size, orig_w = sd2.width(), orig_h = sd2.height(),
            scale = SCALE, dst_x = dst.left, dst_y = dst.top, dst_w = dst.width(), dst_h = dst.height(),
            src_x, overlap, "bonds SD 绘制几何"
        );
        canvas.draw_image_rect(
            sd2,
            Some((&src, skia_safe::canvas::SrcRectConstraint::Strict)),
            dst,
            &paint,
        );
    }
}

fn render_bonds_frame_and_stars(
    canvas: &Canvas,
    entry: &BondsHonorEntry,
    full_size: bool,
    word_id: i64,
    honor_level: i32,
    w: f32,
    h: f32,
    md: &MasterData,
    assets: &AssetStore,
) {
    let paint = Paint::default();
    let rarity_num = match entry.honor_rarity.as_str() {
        "low" => 1,
        "middle" => 2,
        "high" => 3,
        _ => 4,
    };
    let size_char = if full_size { "m" } else { "s" };
    let frame_key = format!("honor/frame_degree_{}_{}", size_char, rarity_num);
    if let Some(frame_img) = assets.get_image(&frame_key) {
        let (fw, fh) = (frame_img.width() as f32, frame_img.height() as f32);
        // 按边框实际宽度水平居中。低稀有度 sub 边框比 honor 窄（164 vs 180），
        // 居中后两侧各留 8px；满宽边框（main 各稀有度、sub 中高稀有度）dx=0 不偏移。
        // 旧逻辑硬编码 dx=8，在满宽 main 低稀有度边框上会整体右移 8px。
        let dx = (w - fw) / 2.0;
        let dst = Rect::from_xywh(-w / 2.0 + dx, -h / 2.0, fw, fh);
        tracing::debug!(
            frame_key = %frame_key, full_size, rarity_num,
            frame_w = frame_img.width(), frame_h = frame_img.height(),
            honor_w = w, honor_h = h, dx,
            dst_x = dst.left, dst_y = dst.top, dst_right = dst.right, dst_bottom = dst.bottom,
            honor_right = w / 2.0, honor_bottom = h / 2.0,
            "bonds 边框绘制几何"
        );
        canvas.draw_image_rect(
            frame_img,
            Some((
                &Rect::from_xywh(0.0, 0.0, fw, fh),
                skia_safe::canvas::SrcRectConstraint::Fast,
            )),
            dst,
            &paint,
        );
    }
    if full_size {
        let word_key = if let Some(word) = md.get_bonds_honor_word(word_id) {
            format!("bonds_honor/word/{}_01", word.assetbundle_name)
        } else {
            String::new()
        };
        if let Some(word_img) = assets.get_image(&word_key) {
            let (ww, wh) = (word_img.width() as f32, word_img.height() as f32);
            canvas.draw_image_rect(
                word_img,
                Some((
                    &Rect::from_xywh(0.0, 0.0, ww, wh),
                    skia_safe::canvas::SrcRectConstraint::Fast,
                )),
                Rect::from_xywh(-ww / 2.0, -wh / 2.0, ww, wh),
                &paint,
            );
        }
    }
    render_bonds_stars(canvas, honor_level, full_size, w, h, assets);
}

fn render_bonds_stars(
    canvas: &Canvas,
    honor_level: i32,
    full_size: bool,
    w: f32,
    h: f32,
    assets: &AssetStore,
) {
    let paint = Paint::default();
    let mut level = honor_level;
    if level > 10 {
        level -= 10;
    }
    let base_y = -h / 2.0 + 63.0;
    let base_x = if full_size { -w / 2.0 + 54.0 } else { -40.0 };
    let normal_count = level.min(5);
    if let Some(s) = assets.get_image("honor/icon_degreeLv") {
        let (sw, sh) = (s.width() as f32, s.height() as f32);
        for i in 0..normal_count {
            canvas.draw_image_rect(
                s.clone(),
                Some((
                    &Rect::from_xywh(0.0, 0.0, sw, sh),
                    skia_safe::canvas::SrcRectConstraint::Fast,
                )),
                Rect::from_xywh(base_x + (i as f32) * 16.0, base_y, sw, sh),
                &paint,
            );
        }
    }
    if level > 5 {
        if let Some(s6) = assets.get_image("honor/icon_degreeLv6") {
            let (sw, sh) = (s6.width() as f32, s6.height() as f32);
            for i in 0..(level - 5) {
                canvas.draw_image_rect(
                    s6.clone(),
                    Some((
                        &Rect::from_xywh(0.0, 0.0, sw, sh),
                        skia_safe::canvas::SrcRectConstraint::Fast,
                    )),
                    Rect::from_xywh(base_x + (i as f32) * 16.0, base_y, sw, sh),
                    &paint,
                );
            }
        }
    }
}
