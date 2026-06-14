// Auto-split from generals/mod.rs

use super::*;

pub(super) fn draw_music_clear_tab(canvas: &Canvas, profile: &ProfileData, md: &MasterData) {
    use layout::MUSIC_CLEAR_TAB;
    let els = &MUSIC_CLEAR_TAB.elements;

    // 难度名称、颜色、元素索引对照
    let difficulties = [
        ("EASY", Color4f::new(0.000, 0.859, 0.451, 1.0), 1, 2), // RGB(0,219,115)
        ("NORMAL", Color4f::new(0.149, 0.792, 0.996, 1.0), 3, 4), // RGB(38,202,254)
        ("HARD", Color4f::new(0.996, 0.788, 0.000, 1.0), 5, 6), // RGB(254,201,0)
        ("EXPERT", Color4f::new(0.996, 0.239, 0.447, 1.0), 7, 8), // RGB(254,61,114)
        ("MASTER", Color4f::new(0.788, 0.169, 1.000, 1.0), 9, 10), // RGB(201,43,255)
        ("APPEND", Color4f::new(0.0, 0.0, 0.0, 0.0), 12, 13),   // 渐变色占位
    ];

    // 获取统计数据
    let stats = profile.music_results.as_ref();

    // 统一标签/数字行的 cy 和高度（测绘值有 1-2px 偏差，对齐观感）
    let label_cy = -20.0_f32; // 标签行统一 cy
    let label_h = 43.0_f32; // 标签统一高度
    let num_cy = -63.0_f32; // 数字行统一 cy
    let num_h = 29.0_f32; // 数字统一高度

    for (name, color, label_idx, num_idx) in &difficulties {
        let lbl = &els[*label_idx];
        // 难度标签（APPEND 用渐变，其他用纯色）
        if *name == "APPEND" {
            // 渐变 tab：粉 → 青
            let rect = Rect::from_xywh(
                lbl.cx - lbl.w / 2.0,
                -label_cy - label_h / 2.0,
                lbl.w,
                label_h,
            );
            let colors: [Color; 2] = [
                Color::from_argb(255, 206, 191, 255), // 左端淡紫 RGB(206,191,255)
                Color::from_argb(255, 233, 182, 252), // 右端粉紫 RGB(233,182,252)
            ];
            let pts = (Point::new(rect.left, 0.0), Point::new(rect.right, 0.0));
            if let Some(shader) = skia_safe::Shader::linear_gradient(
                pts,
                colors.as_slice(),
                None,
                skia_safe::TileMode::Clamp,
                None,
                None,
            ) {
                let mut gp = Paint::default();
                gp.set_style(PaintStyle::Fill);
                gp.set_anti_alias(true);
                gp.set_shader(shader);
                canvas.draw_round_rect(rect, 8.0, 8.0, &gp);
            }
        } else {
            draw_colored_tab(canvas, lbl.cx, label_cy, lbl.w, label_h, *color);
        }
        let aligned_lbl = layout::ElementLayout {
            cx: lbl.cx,
            cy: label_cy,
            w: lbl.w,
            h: label_h,
        };
        draw_general_text(
            canvas,
            name,
            &aligned_lbl,
            1,
            md,
            Color4f::new(1.0, 1.0, 1.0, 1.0),
            Align::Center,
            label_h * 0.55,
        );
        // 数字（统一水平线）
        let count = match stats {
            Some(mr) => match *name {
                "EASY" => mr.easy.clear,
                "NORMAL" => mr.normal.clear,
                "HARD" => mr.hard.clear,
                "EXPERT" => mr.expert.clear,
                "MASTER" => mr.master.clear,
                "APPEND" => mr.append.clear,
                _ => 0,
            },
            None => 0,
        };
        let num_el = &els[*num_idx];
        let aligned_num = layout::ElementLayout {
            cx: num_el.cx,
            cy: num_cy,
            w: num_el.w,
            h: num_h,
        };
        draw_general_text(
            canvas,
            &format!("{}", count),
            &aligned_num,
            1,
            md,
            Color4f::new(0.2, 0.2, 0.2, 1.0),
            Align::Center,
            num_h,
        );
    }

    // 竖线分隔符 [11]（只关注高度，宽度统一 2px）
    let sep = &els[11];
    let mut sp = Paint::default();
    sp.set_style(PaintStyle::Fill);
    sp.set_color4f(Color4f::new(0.78, 0.78, 0.78, 0.6), None);
    sp.set_anti_alias(true);
    canvas.draw_rect(
        Rect::from_xywh(sep.cx - 1.0, -sep.cy - sep.h / 2.0, 2.0, sep.h),
        &sp,
    );

    // 底部标题行 [0] — 连体药丸（完成 / Full Combo / AP）
    let tab_el = &els[0];
    let labels = ["完成", "Full Combo", "AP"];
    let label_count = labels.len() as f32;
    let label_w = tab_el.w / label_count;
    let pill_r = tab_el.h / 2.0;

    // 1) 整体灰色药丸底色
    {
        let mut bg = Paint::default();
        bg.set_style(PaintStyle::Fill);
        bg.set_color4f(TAB_BG_COLOR, None);
        bg.set_anti_alias(true);
        canvas.draw_round_rect(
            Rect::from_xywh(
                tab_el.cx - tab_el.w / 2.0,
                -tab_el.cy - tab_el.h / 2.0,
                tab_el.w,
                tab_el.h,
            ),
            pill_r,
            pill_r,
            &bg,
        );
    }

    // 2) 「完成」选中态白色覆盖
    {
        let first_cx = tab_el.cx - tab_el.w / 2.0 + label_w * 0.5;
        let mut sel = Paint::default();
        sel.set_style(PaintStyle::Fill);
        sel.set_color4f(Color4f::new(1.0, 1.0, 1.0, 0.9), None);
        sel.set_anti_alias(true);
        canvas.draw_round_rect(
            Rect::from_xywh(
                first_cx - label_w / 2.0,
                -tab_el.cy - tab_el.h / 2.0,
                label_w,
                tab_el.h,
            ),
            pill_r,
            pill_r,
            &sel,
        );
    }

    // 3) 三段文字（选中=黑字，非选中=白字）
    for (i, label) in labels.iter().enumerate() {
        let lcx = tab_el.cx - tab_el.w / 2.0 + label_w * (i as f32 + 0.5);
        let lel = layout::ElementLayout {
            cx: lcx,
            cy: tab_el.cy,
            w: label_w,
            h: tab_el.h,
        };
        let text_color = if i == 0 {
            Color4f::new(0.2, 0.2, 0.2, 1.0) // 选中：黑字
        } else {
            Color4f::new(1.0, 1.0, 1.0, 1.0) // 非选中：白字
        };
        draw_general_text(
            canvas,
            label,
            &lel,
            1,
            md,
            text_color,
            Align::Center,
            tab_el.h * 0.42,
        );
    }
}
