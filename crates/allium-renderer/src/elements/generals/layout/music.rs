// Auto-split from layout.rs

use super::{ElementLayout, PanelLayout};

pub static MUSIC_CLEAR_TAB: PanelLayout = PanelLayout {
    w: 939.0,
    h: 300.0,
    elements: &[
        ElementLayout {
            cx: -1.0,
            cy: 51.0,
            w: 835.0,
            h: 58.0,
        }, // [0] 完成|FC|AP 标题行
        ElementLayout {
            cx: -349.0,
            cy: -21.0,
            w: 131.0,
            h: 44.0,
        }, // [1] EASY 标签
        ElementLayout {
            cx: -347.0,
            cy: -63.0,
            w: 20.0,
            h: 29.0,
        }, // [2] EASY 数字
        ElementLayout {
            cx: -213.0,
            cy: -21.0,
            w: 128.0,
            h: 42.0,
        }, // [3] NORMAL 标签
        ElementLayout {
            cx: -213.0,
            cy: -63.0,
            w: 38.0,
            h: 29.0,
        }, // [4] NORMAL 数字
        ElementLayout {
            cx: -79.0,
            cy: -21.0,
            w: 129.0,
            h: 39.0,
        }, // [5] HARD 标签
        ElementLayout {
            cx: -77.0,
            cy: -62.0,
            w: 55.0,
            h: 28.0,
        }, // [6] HARD 数字
        ElementLayout {
            cx: 58.0,
            cy: -20.0,
            w: 129.0,
            h: 41.0,
        }, // [7] EXPERT 标签
        ElementLayout {
            cx: 57.0,
            cy: -62.0,
            w: 56.0,
            h: 28.0,
        }, // [8] EXPERT 数字
        ElementLayout {
            cx: 193.0,
            cy: -20.0,
            w: 130.0,
            h: 43.0,
        }, // [9] MASTER 标签
        ElementLayout {
            cx: 193.0,
            cy: -62.0,
            w: 34.0,
            h: 29.0,
        }, // [10] MASTER 数字
        ElementLayout {
            cx: 271.0,
            cy: -46.0,
            w: 13.0,
            h: 88.0,
        }, // [11] 竖线分隔符
        ElementLayout {
            cx: 349.0,
            cy: -19.0,
            w: 129.0,
            h: 43.0,
        }, // [12] APPEND 标签
        ElementLayout {
            cx: 350.0,
            cy: -63.0,
            w: 14.0,
            h: 29.0,
        }, // [13] APPEND 数字
    ],
};

/// type=12 歌曲信息-详细版 (939×330)
///
/// 三组: 完成情况 + FULL COMBO 情况 + AP 情况，每组有灰色标题气泡+数据栏。
///
/// ## 素材来源
/// - 纯文本/图标面板，无 S3 素材依赖
/// - 数据来源同 type=16
///
/// elements: [完成灰泡, 完成情况栏, FC灰泡, FC情况栏, AP灰泡, AP情况栏]
pub static MUSIC_CLEAR: PanelLayout = PanelLayout {
    w: 939.0,
    h: 330.0,
    elements: &[
        ElementLayout {
            cx: 0.0,
            cy: 132.0,
            w: 862.0,
            h: 40.0,
        }, // [0] 完成（灰色气泡）
        ElementLayout {
            cx: 0.0,
            cy: 82.0,
            w: 828.0,
            h: 58.0,
        }, // [1] 完成情况栏
        ElementLayout {
            cx: 0.0,
            cy: 25.0,
            w: 860.0,
            h: 40.0,
        }, // [2] FULL COMBO 灰色气泡
        ElementLayout {
            cx: 0.0,
            cy: -25.0,
            w: 828.0,
            h: 58.0,
        }, // [3] FULL COMBO 情况栏
        ElementLayout {
            cx: 0.0,
            cy: -82.0,
            w: 860.0,
            h: 40.0,
        }, // [4] AP 灰色气泡
        ElementLayout {
            cx: 0.0,
            cy: -132.0,
            w: 828.0,
            h: 58.0,
        }, // [5] AP 情况栏
    ],
};
