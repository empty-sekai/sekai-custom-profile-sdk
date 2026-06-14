// Auto-split from generals/mod.rs

use super::*;

pub(super) fn draw_char_rank(
    canvas: &Canvas,
    profile: &ProfileData,
    md: &MasterData,
    assets: Option<&AssetStore>,
    general_type: i32,
) {
    use layout::CHAR_RANK;
    let els = &CHAR_RANK.elements;

    // type=15 是滑动变体：裁掉下方超出的部分
    let is_compact = general_type == 15;
    if is_compact {
        let compact_h = CHAR_RANK.h - layout::MUSIC_CLEAR_TAB.h; // 572
        let shift = (CHAR_RANK.h - compact_h) / 2.0; // 150
        canvas.save();
        canvas.clip_rect(
            Rect::from_xywh(-CHAR_RANK.w / 2.0, -compact_h / 2.0, CHAR_RANK.w, compact_h),
            None,
            None,
        );
        // 内容下移，使原面板顶部对齐到 clip 区域顶部
        canvas.translate((0.0, shift));
    }

    // ━━ Tab 标签绘制（连体药丸） ━━
    let tab_active = &els[0]; // 角色收藏等级 (选中态)
    let tab_inactive = &els[1]; // 挑战舞台 (非选中态)

    // 计算整体药丸区域（合并两个 tab）
    let pill_left =
        (tab_active.cx - tab_active.w / 2.0).min(tab_inactive.cx - tab_inactive.w / 2.0);
    let pill_right =
        (tab_active.cx + tab_active.w / 2.0).max(tab_inactive.cx + tab_inactive.w / 2.0);
    let pill_w = pill_right - pill_left;
    let pill_h = tab_active.h;
    let pill_cy = tab_active.cy;
    let pill_r = pill_h / 2.0; // 药丸圆角

    let mut tab_bg = Paint::default();
    tab_bg.set_style(PaintStyle::Fill);
    tab_bg.set_anti_alias(true);

    // 1) 整体灰色药丸底色
    tab_bg.set_color4f(Color4f::new(0.88, 0.88, 0.88, 0.7), None);
    canvas.draw_round_rect(
        Rect::from_xywh(pill_left, -pill_cy - pill_h / 2.0, pill_w, pill_h),
        pill_r,
        pill_r,
        &tab_bg,
    );

    // 2) 选中态白色覆盖左半
    tab_bg.set_color4f(Color4f::new(1.0, 1.0, 1.0, 0.9), None);
    canvas.draw_round_rect(
        Rect::from_xywh(
            tab_active.cx - tab_active.w / 2.0,
            -tab_active.cy - tab_active.h / 2.0,
            tab_active.w,
            tab_active.h,
        ),
        pill_r,
        pill_r,
        &tab_bg,
    );

    // 3) 文字
    draw_general_text(
        canvas,
        "角色收藏等级",
        tab_active,
        1,
        md,
        Color4f::new(0.2, 0.2, 0.2, 1.0),
        Align::Center,
        tab_active.h * 0.42,
    );
    draw_general_text(
        canvas,
        "挑战舞台",
        tab_inactive,
        1,
        md,
        Color4f::new(0.5, 0.5, 0.5, 1.0),
        Align::Center,
        tab_inactive.h * 0.42,
    );

    // ━━ 角色网格绘制 ━━
    // ━━ 角色网格绘制 ━━
    // 4 列的 canvas_x（直接取 game_center_x）
    // 列坐标偏移以补偿药丸右侧不对称，使得左右等量超出tab各~10px
    // 视觉中心 = cx + (col_half_w - avatar_r)/2 ≈ [-300.5, -100.5, 99.5, 299.5]
    let col_cx: [f32; 4] = [-350.0, -150.0, 50.0, 250.0]; // 间距200
    let first_row_cy = -259.0_f32;
    let row_h = 105.0_f32;

    // 形状参数（药丸大幅延长，左右各超出tab约10px）
    let avatar_r = 38.0_f32;
    let pill_h = (avatar_r * 2.0) * 0.80;
    let col_half_w = 137.0_f32; // 右端: 250+137=387, 超出tab右377约10px
    let pill_total_w = avatar_r + col_half_w; // 左端: -350-38=-388, 超出tab左-378约10px

    // 背景色 — 精确提取 RGB(52,220,254)
    let bg_color = Color4f::new(0.204, 0.863, 0.996, 1.0);

    // 等级字体
    let font_mgr = FontMgr::default();
    let rank_tf = md
        .resolve_font(1)
        .and_then(|name| font_mgr.match_family_style(name, FontStyle::normal()))
        .or_else(|| font_mgr.legacy_make_typeface(None, FontStyle::normal()));
    let rank_tf = match rank_tf {
        Some(tf) => tf,
        None => {
            tracing::warn!("draw_char_rank: 无法创建 Typeface, 跳过绘制");
            return;
        }
    };
    let rank_font = Font::new(rank_tf, Some(22.0));
    let mut rank_paint = Paint::default();
    rank_paint.set_color4f(Color4f::new(0.0, 0.0, 0.0, 1.0), None); // 黑色
    rank_paint.set_anti_alias(true);

    // 按组合分组排列（游戏真实行为）
    // 组合顺序: VS 最前（6人占2行，第2行空2格），其余每组4人1行
    // characterId: 21-26=VS, 1-4=Leo/need, 5-8=MMJ, 9-12=VBS, 13-16=25ji, 17-20=WxS
    let group_order: &[&[i32]] = &[
        &[21, 22, 23, 24, 25, 26], // VS (6人，前2行)
        &[1, 2, 3, 4],             // Leo/need
        &[5, 6, 7, 8],             // MMJ
        &[9, 10, 11, 12],          // VBS
        &[13, 14, 15, 16],         // 25ji
        &[17, 18, 19, 20],         // WxS
    ];

    let mut slot = 0usize;
    for group in group_order {
        for &cid in *group {
            let cr = match profile.char_ranks.iter().find(|c| c.character_id == cid) {
                Some(c) => c,
                None => continue,
            };
            let col = slot % 4;
            let row = slot / 4;
            let cx = col_cx[col];
            let cy = first_row_cy + row_h * row as f32;
            slot += 1;

            let mut bg = Paint::default();
            bg.set_style(PaintStyle::Fill);
            bg.set_color4f(bg_color, None);
            bg.set_anti_alias(true);

            // ── 1) 蓝色大药丸（左端=头像左缘，底部对齐头像底边） ──
            let pill_left = cx - avatar_r;
            let pill_bottom = cy + avatar_r;
            let pill_top = pill_bottom - pill_h;
            canvas.draw_round_rect(
                Rect::from_xywh(pill_left, pill_top, pill_total_w, pill_h),
                pill_h / 2.0,
                pill_h / 2.0,
                &bg,
            );

            // ── 2) 蓝色大圆（头像背景，覆盖药丸左端） ──
            canvas.draw_circle(Point::new(cx, cy), avatar_r, &bg);

            // ── 头像（大圆中心，圆形裁切） ──
            let mut avatar_drawn = false;
            if let Some(store) = assets {
                let key = format!("chara_avatar/chara{:02}_02", cr.character_id);
                if let Some(img) = store.get_image(&key) {
                    let p = Paint::default();
                    // 圆形裁切
                    canvas.save();
                    let clip_r = Rect::from_xywh(
                        cx - avatar_r,
                        cy - avatar_r,
                        avatar_r * 2.0,
                        avatar_r * 2.0,
                    );
                    canvas.clip_rect(clip_r, None, Some(true));
                    canvas.draw_image_rect(img, None, clip_r, &p);
                    canvas.restore();
                    avatar_drawn = true;
                }
            }
            // 无头像时的占位圆
            if !avatar_drawn {
                let mut ap = Paint::default();
                ap.set_style(PaintStyle::Fill);
                ap.set_color4f(Color4f::new(0.85, 0.85, 0.85, 0.4), None);
                ap.set_anti_alias(true);
                canvas.draw_circle(Point::new(cx, cy), avatar_r, &ap);
            }

            // ── 等级数字（右侧区域中心白字） ──
            let rank_text = format!("{}", cr.rank);
            let tw = rank_font
                .measure_str(&rank_text, Some(&rank_paint))
                .1
                .width();
            // 数字居中在 [头像右缘, 药丸右端-圆角半径] 之间
            let num_cx = cx + avatar_r + (col_half_w - avatar_r - pill_h / 2.0) / 2.0;
            let num_cy = pill_top + pill_h / 2.0 + 8.0;
            canvas.draw_str(
                &rank_text,
                Point::new(num_cx - tw / 2.0, num_cy),
                &rank_font,
                &rank_paint,
            );
        }
        // 每组结束后对齐到下一行（VS第2行空出2格）
        if slot % 4 != 0 {
            slot = (slot / 4 + 1) * 4;
        }
    } // for group

    // 恢复 type=15 的 clip 裁剪
    if is_compact {
        canvas.restore();
    }
}
