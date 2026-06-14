// Auto-split from layout.rs

use super::{ElementLayout, PanelLayout};

pub static PLAYER_NAME: PanelLayout = PanelLayout {
    w: 610.0,
    h: 127.0,
    elements: &[
        ElementLayout {
            cx: -1.0,
            cy: 0.0,
            w: 544.0,
            h: 64.0,
        }, // 名称文本框
        ElementLayout {
            cx: -11.0,
            cy: 0.0,
            w: 449.0,
            h: 34.0,
        }, // 名称文本
    ],
};

/// type=2 综合力 (813×136)
/// elements: [文本"综合力", 综合力图标, 文本"|", 综合力数字, 图标"i"]
pub static TOTAL_POWER: PanelLayout = PanelLayout {
    w: 813.0,
    h: 136.0,
    elements: &[
        ElementLayout {
            cx: -328.0,
            cy: -1.0,
            w: 92.0,
            h: 33.0,
        }, // 文本"综合力"
        ElementLayout {
            cx: -231.0,
            cy: 2.0,
            w: 45.0,
            h: 48.0,
        }, // 综合力图标
        ElementLayout {
            cx: -266.0,
            cy: 1.0,
            w: 12.0,
            h: 34.0,
        }, // 文本"|"
        ElementLayout {
            cx: -116.0,
            cy: 0.0,
            w: 112.0,
            h: 33.0,
        }, // 综合力数字
        ElementLayout {
            cx: -13.0,
            cy: 3.0,
            w: 64.0,
            h: 61.0,
        }, // 图标"i"(圆)
    ],
};

/// type=4 个性签名 (700×251)
/// elements: [个性签名文本框, 文本"个性签名", 图标"铅笔"]
pub static COMMENT: PanelLayout = PanelLayout {
    w: 700.0,
    h: 251.0,
    elements: &[
        ElementLayout {
            cx: 2.0,
            cy: -25.0,
            w: 633.0,
            h: 142.0,
        }, // 个性签名文本框
        ElementLayout {
            cx: -250.0,
            cy: 82.0,
            w: 125.0,
            h: 35.0,
        }, // 文本"个性签名"
        ElementLayout {
            cx: 279.0,
            cy: 13.0,
            w: 51.0,
            h: 51.0,
        }, // 图标"铅笔"（不渲染）
    ],
};
