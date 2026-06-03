//! 普通称号渲染。

use super::common::draw_placeholder;
use crate::assets::AssetStore;
use crate::masterdata::{MasterData, ResolvedHonor};
use skia_safe::{Canvas, Color4f, Font, FontMgr, FontStyle, Paint, PaintStyle, Point, Rect};

/// 绘制普通称号。
pub fn render_honor(
    canvas: &Canvas,
    honor_id: i32,
    honor_level: i32,
    full_size: bool,
    md: &MasterData,
    assets: &AssetStore,
    profile: Option<&crate::profile::ProfileData>,
) {
    let resolved = match md.resolve_honor(honor_id, honor_level) {
        Some(r) => r,
        None => {
            tracing::warn!(honor_id, "Honor 未找到 MasterData");
            let (w, h) = if full_size {
                (380.0, 80.0)
            } else {
                (180.0, 80.0)
            };
            draw_placeholder(canvas, "Honor", honor_id, w, h);
            return;
        }
    };

    let (w, h) = if full_size {
        (380.0, 80.0)
    } else {
        (180.0, 80.0)
    };
    let suffix = if full_size { "main" } else { "sub" };
    let paint = Paint::default();

    let bg_abn = resolved
        .background_asset_bundle_name
        .as_deref()
        .unwrap_or(&resolved.asset_bundle_name);
    let bg_dir = if resolved.honor_type == "rank_match" {
        "rank_live/honor"
    } else {
        "honor"
    };
    let bg_key = format!("{}/{}/degree_{}", bg_dir, bg_abn, suffix);
    if let Some(img) = assets.get_image(&bg_key) {
        let iw = img.width() as f32;
        let ih = img.height() as f32;
        canvas.draw_image_rect(
            img,
            Some((
                &Rect::from_xywh(0.0, 0.0, iw, ih),
                skia_safe::canvas::SrcRectConstraint::Fast,
            )),
            Rect::from_xywh(-w / 2.0, -h / 2.0, w, h),
            &paint,
        );
    }

    let rarity_num = match resolved.honor_rarity.as_str() {
        "low" => 1,
        "middle" => 2,
        "high" => 3,
        _ => 4,
    };
    let size_char = if full_size { "m" } else { "s" };
    let mut frame_img = None;
    if let Some(ref fname) = resolved.frame_name {
        let frame_key = format!(
            "honor_frame/{}/frame_degree_{}_{}",
            fname, size_char, rarity_num
        );
        frame_img = assets.get_image(&frame_key);
    }
    if frame_img.is_none() {
        let default_key = format!("honor/frame_degree_{}_{}", size_char, rarity_num);
        frame_img = assets.get_image(&default_key);
    }

    if let Some(img) = frame_img {
        let iw = img.width() as f32;
        let ih = img.height() as f32;
        // 按边框实际宽度水平居中（与 bonds.rs 同素材同逻辑）。低稀有度 sub 边框比
        // honor 窄（164 vs 180）居中后两侧各留 8px；满宽边框 ox=0 不偏移。
        // 旧逻辑硬编码 ox=8，在满宽 main 低稀有度边框（380）上会整体右移 8px。
        let ox = (w - iw) / 2.0;
        let dst = Rect::from_xywh(-w / 2.0 + ox, -h / 2.0, iw, ih);
        tracing::debug!(
            honor_type = %resolved.honor_type, full_size, rarity_num,
            frame_name = ?resolved.frame_name,
            frame_w = img.width(), frame_h = img.height(), honor_w = w, honor_h = h, ox,
            dst_x = dst.left, dst_right = dst.right, honor_right = w / 2.0,
            "standard 边框绘制几何"
        );
        canvas.draw_image_rect(
            img,
            Some((
                &Rect::from_xywh(0.0, 0.0, iw, ih),
                skia_safe::canvas::SrcRectConstraint::Fast,
            )),
            dst,
            &paint,
        );
    }

    let (overlay_dir, overlay_name) = if resolved.honor_type == "rank_match" {
        ("rank_live/honor", suffix.to_string())
    } else if resolved.is_live_master {
        ("honor", "scroll".to_string())
    } else if resolved.honor_type == "character" {
        let tier = (resolved.honor_level / 10) + 1;
        ("honor", format!("rank_{}_{tier}", suffix))
    } else {
        ("honor", format!("rank_{}", suffix))
    };
    let overlay_key = format!(
        "{}/{}/{}",
        overlay_dir, resolved.asset_bundle_name, overlay_name
    );
    if let Some(img) = assets.get_image(&overlay_key) {
        let iw = img.width() as f32;
        let ih = img.height() as f32;
        let (dx, dy) = if resolved.is_live_master {
            if full_size {
                (218.0, 3.0)
            } else {
                (40.0, 3.0)
            }
        } else if resolved.honor_type == "rank_match" {
            if full_size { (190.0, 0.0) } else { (17.0, 42.0) }
        } else if (full_size && iw == 380.0) || (!full_size && ih == 80.0) {
            (0.0, 0.0)
        } else if full_size {
            (190.0, 0.0)
        } else {
            (34.0, 42.0)
        };
        canvas.draw_image_rect(
            img,
            Some((
                &Rect::from_xywh(0.0, 0.0, iw, ih),
                skia_safe::canvas::SrcRectConstraint::Fast,
            )),
            Rect::from_xywh(-w / 2.0 + dx, -h / 2.0 + dy, iw, ih),
            &paint,
        );
    }

    if resolved.is_live_master {
        render_live_master_overlay(canvas, &resolved, full_size, w, h, assets, profile);
    }
    render_stars(canvas, &resolved, full_size, w, h, assets);
}

fn render_live_master_overlay(
    canvas: &Canvas,
    resolved: &ResolvedHonor,
    full_size: bool,
    w: f32,
    h: f32,
    assets: &AssetStore,
    profile: Option<&crate::profile::ProfileData>,
) {
    let progress = profile
        .and_then(|p| {
            resolved
                .honor_mission_type
                .as_ref()
                .and_then(|mt| p.user_honor_missions.get(mt))
                .copied()
        })
        .unwrap_or(0);

    let font_mgr = FontMgr::default();
    let typeface = font_mgr
        .match_family_style("Noto Sans CJK SC", FontStyle::bold())
        .or_else(|| font_mgr.legacy_make_typeface(None, FontStyle::bold()));
    if let Some(tf) = typeface {
        let font = Font::new(tf, Some(20.0));
        let text = progress.to_string();
        let text_w = font.measure_str(&text, None).0;
        let mut tp = Paint::default();
        tp.set_style(PaintStyle::Fill);
        tp.set_color4f(Color4f::new(1.0, 1.0, 1.0, 1.0), None);
        tp.set_anti_alias(true);
        let (cx, cy) = if full_size {
            (-w / 2.0 + 270.0, -h / 2.0 + 70.0)
        } else {
            (-w / 2.0 + 90.0, -h / 2.0 + 70.0)
        };
        canvas.draw_str(&text, Point::new(cx - text_w / 2.0, cy), &font, &tp);
    }

    let star_count = ((progress / 10) % 10 + 1) as usize;
    let star_on = assets.get_image("honor/live_master_honor_star_1");
    let star_off = assets.get_image("honor/live_master_honor_star_2");
    let stars_pos: [(f32, f32); 10] = if full_size {
        [
            (223.0, 68.0),
            (216.0, 56.0),
            (208.0, 42.0),
            (216.0, 27.0),
            (223.0, 13.0),
            (295.0, 68.0),
            (304.0, 56.0),
            (311.0, 42.0),
            (303.0, 27.0),
            (295.0, 13.0),
        ]
    } else {
        [
            (45.0, 68.0),
            (38.0, 56.0),
            (30.0, 42.0),
            (38.0, 27.0),
            (45.0, 13.0),
            (117.0, 68.0),
            (126.0, 56.0),
            (133.0, 42.0),
            (125.0, 27.0),
            (117.0, 13.0),
        ]
    };
    let paint = Paint::default();
    for (i, &(px, py)) in stars_pos.iter().enumerate() {
        let star_img = if i < star_count { &star_on } else { &star_off };
        if let Some(img) = star_img {
            let sw = img.width() as f32;
            let sh = img.height() as f32;
            canvas.draw_image_rect(
                img.clone(),
                Some((
                    &Rect::from_xywh(0.0, 0.0, sw, sh),
                    skia_safe::canvas::SrcRectConstraint::Fast,
                )),
                Rect::from_xywh(-w / 2.0 + px, -h / 2.0 + py - 8.0, sw, sh),
                &paint,
            );
        }
    }
}

fn render_stars(
    canvas: &Canvas,
    resolved: &ResolvedHonor,
    full_size: bool,
    w: f32,
    h: f32,
    assets: &AssetStore,
) {
    if !resolved.has_star || resolved.is_live_master {
        return;
    }
    if resolved.honor_type != "character" && resolved.honor_type != "achievement" {
        return;
    }

    let paint = Paint::default();
    let mut level = resolved.honor_level % 10;
    if level == 0 && resolved.honor_level > 0 {
        level = 10;
    }
    let base_y = -h / 2.0 + 63.0;
    let base_x = if full_size { -w / 2.0 + 54.0 } else { -40.0 };
    let normal_count = level.min(5);

    if let Some(star_img) = assets.get_image("honor/icon_degreeLv") {
        let sw = star_img.width() as f32;
        let sh = star_img.height() as f32;
        for i in 0..normal_count {
            let x = base_x + (i as f32) * 16.0;
            canvas.draw_image_rect(
                star_img.clone(),
                Some((
                    &Rect::from_xywh(0.0, 0.0, sw, sh),
                    skia_safe::canvas::SrcRectConstraint::Fast,
                )),
                Rect::from_xywh(x, base_y, sw, sh),
                &paint,
            );
        }
    }

    if level > 5 {
        if let Some(star6_img) = assets.get_image("honor/icon_degreeLv6") {
            let sw = star6_img.width() as f32;
            let sh = star6_img.height() as f32;
            for i in 0..(level - 5) {
                let x = base_x + (i as f32) * 16.0;
                canvas.draw_image_rect(
                    star6_img.clone(),
                    Some((
                        &Rect::from_xywh(0.0, 0.0, sw, sh),
                        skia_safe::canvas::SrcRectConstraint::Fast,
                    )),
                    Rect::from_xywh(x, base_y, sw, sh),
                    &paint,
                );
            }
        }
    }
}
