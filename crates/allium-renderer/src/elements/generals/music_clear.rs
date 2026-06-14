// Auto-split from generals/mod.rs

use super::*;

pub(super) fn draw_music_clear(canvas: &Canvas, profile: &ProfileData, md: &MasterData) {
    use layout::MUSIC_CLEAR;
    let els = &MUSIC_CLEAR.elements;
    let stats = profile.music_results.as_ref();

    // 三组: (标题, 灰泡索引, 数据栏索引, 取值函数)
    let groups: [(&str, usize, usize, fn(&MusicDifficultyStats) -> i32); 3] = [
        ("完成", 0, 1, |s| s.clear),
        ("FULL COMBO", 2, 3, |s| s.full_combo),
        ("AP", 4, 5, |s| s.all_perfect),
    ];

    let diff_colors = [
        Color4f::new(0.000, 0.859, 0.451, 1.0), // RGB(0,219,115) EASY
        Color4f::new(0.149, 0.792, 0.996, 1.0), // RGB(38,202,254) NORMAL
        Color4f::new(0.996, 0.788, 0.000, 1.0), // RGB(254,201,0) HARD
        Color4f::new(0.996, 0.239, 0.447, 1.0), // RGB(254,61,114) EXPERT
        Color4f::new(0.788, 0.169, 1.000, 1.0), // RGB(201,43,255) MASTER
        Color4f::new(0.843, 0.741, 1.000, 1.0), // RGB(215,189,255) APPEND 纯色近似
    ];

    for (title, bubble_idx, bar_idx, getter) in &groups {
        // 灰色药丸标题
        let bubble = &els[*bubble_idx];
        {
            let mut bp = Paint::default();
            bp.set_style(PaintStyle::Fill);
            bp.set_color4f(TAB_BG_COLOR, None);
            bp.set_anti_alias(true);
            let r = Rect::from_xywh(
                bubble.cx - bubble.w / 2.0,
                -bubble.cy - bubble.h / 2.0,
                bubble.w,
                bubble.h,
            );
            canvas.draw_round_rect(r, bubble.h / 2.0, bubble.h / 2.0, &bp);
        }
        draw_general_text(
            canvas,
            title,
            bubble,
            1,
            md,
            Color4f::new(1.0, 1.0, 1.0, 1.0), // 白字
            Align::Center,
            bubble.h * 0.42,
        );

        // 数据栏内 6 列：每列上面彩色标签 + 下面数字（同 type=16 样式）
        let bar = &els[*bar_idx];
        let col_w = bar.w / 6.0;
        let tab_h = 28.0_f32;
        let num_h_inner = 22.0_f32;
        // 标签行 cy = 数据栏上半部中心，数字行 cy = 下半部中心
        let label_row_cy = bar.cy + bar.h * 0.24;
        let num_row_cy = bar.cy - bar.h * 0.26;

        let diff_names = ["EASY", "NORMAL", "HARD", "EXPERT", "MASTER", "APPEND"];
        let diff_data = stats
            .map(|mr| {
                [
                    getter(&mr.easy),
                    getter(&mr.normal),
                    getter(&mr.hard),
                    getter(&mr.expert),
                    getter(&mr.master),
                    getter(&mr.append),
                ]
            })
            .unwrap_or([0; 6]);

        for (j, &count) in diff_data.iter().enumerate() {
            let col_cx = bar.cx - bar.w / 2.0 + col_w * (j as f32 + 0.5);

            // 彩色标签
            draw_colored_tab(
                canvas,
                col_cx,
                label_row_cy,
                col_w * 0.9,
                tab_h,
                diff_colors[j],
            );
            let lbl_el = layout::ElementLayout {
                cx: col_cx,
                cy: label_row_cy,
                w: col_w * 0.9,
                h: tab_h,
            };
            draw_general_text(
                canvas,
                diff_names[j],
                &lbl_el,
                1,
                md,
                Color4f::new(1.0, 1.0, 1.0, 1.0),
                Align::Center,
                tab_h * 0.55,
            );

            // 数字
            let num_el = layout::ElementLayout {
                cx: col_cx,
                cy: num_row_cy,
                w: col_w,
                h: num_h_inner,
            };
            draw_general_text(
                canvas,
                &format!("{}", count),
                &num_el,
                1,
                md,
                Color4f::new(0.2, 0.2, 0.2, 1.0),
                Align::Center,
                num_h_inner,
            );
        }
    }
}
