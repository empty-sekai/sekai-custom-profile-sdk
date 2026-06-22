// Auto-split from generals/mod.rs

use super::*;

pub(super) fn draw_challenge_live(
    canvas: &Canvas,
    profile: &ProfileData,
    md: &MasterData,
    assets: Option<&AssetStore>,
) {
    use layout::CHALLENGE_LIVE;
    let els = &CHALLENGE_LIVE.elements;

    // 横线 [4]
    draw_horizontal_line(canvas, els[4].cx, els[4].cy, els[4].w);

    // "挑战演出" [0] — 取 customProfilePlayerInfoResources[id=10].name
    let title = md
        .resolve_player_info_label(10)
        .unwrap_or_else(|| "挑战演出".to_string());
    draw_general_text(
        canvas,
        &title,
        &els[0],
        1,
        md,
        Color4f::new(0.33, 0.33, 0.33, 1.0),
        Align::Center,
        els[0].h,
    );

    // "独奏" 图标 [1] — 表外标签，走 RegionLabels 兜底
    let solo_label = md.labels().challenge_solo_label();
    draw_gray_icon_bg(canvas, els[1].cx, els[1].cy, els[1].w, els[1].h, 16.0);
    draw_general_text(
        canvas,
        solo_label,
        &els[1],
        1,
        md,
        Color4f::new(1.0, 1.0, 1.0, 1.0),
        Align::Center,
        els[1].h * 0.55,
    );

    // 角色头像 [2] — 圆形裁切
    let avatar_cx = els[2].cx;
    let avatar_cy = els[2].cy;
    let avatar_r = els[2].w / 2.0;
    let cid = profile.challenge_character_id;
    let avatar_key = format!("chara_avatar/chara{:02}_02", cid);
    let mut drawn = false;
    if let Some(store) = assets {
        if let Some(img) = store.get_image(&avatar_key) {
            canvas.save();
            let path = {
                let mut b = skia_safe::PathBuilder::new();
                b.add_circle(Point::new(avatar_cx, -avatar_cy), avatar_r, None);
                b.detach()
            };
            canvas.clip_path(&path, skia_safe::ClipOp::Intersect, true);
            let p = Paint::default();
            canvas.draw_image_rect(
                img,
                None,
                Rect::from_xywh(
                    avatar_cx - avatar_r,
                    -avatar_cy - avatar_r,
                    avatar_r * 2.0,
                    avatar_r * 2.0,
                ),
                &p,
            );
            canvas.restore();
            drawn = true;
        }
    }
    if !drawn {
        let mut paint = Paint::default();
        paint.set_style(PaintStyle::Fill);
        paint.set_color4f(Color4f::new(0.87, 0.87, 0.87, 1.0), None);
        paint.set_anti_alias(true);
        canvas.draw_circle(Point::new(avatar_cx, -avatar_cy), avatar_r, &paint);
    }

    // 分数 [3]（文本元素，h=字号）
    draw_general_text(
        canvas,
        &format!("{}", profile.challenge_score),
        &els[3],
        1,
        md,
        Color4f::new(0.27, 0.27, 0.27, 1.0),
        Align::Left,
        els[3].h,
    );
}
