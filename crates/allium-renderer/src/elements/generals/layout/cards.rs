// Auto-split from layout.rs

use super::{ElementLayout, PanelLayout};

pub static LEADER_MEMBER: PanelLayout = PanelLayout {
    w: 997.0,
    h: 589.0,
    elements: &[
        ElementLayout {
            cx: -1.0,
            cy: 1.0,
            w: 940.0,
            h: 530.0,
        }, // 队长卡面
        ElementLayout {
            cx: -415.0,
            cy: -137.0,
            w: 63.0,
            h: 203.0,
        }, // 稀有度星级
        ElementLayout {
            cx: 386.0,
            cy: 219.0,
            w: 85.0,
            h: 92.0,
        }, // 属性标签长方框
        ElementLayout {
            cx: 392.0,
            cy: -187.0,
            w: 108.0,
            h: 107.0,
        }, // 突破等级图片
    ],
};

/// type=6 称号 (844x241)
pub static HONORS: PanelLayout = PanelLayout {
    w: 844.0,
    h: 241.0,
    elements: &[
        ElementLayout {
            cx: 2.0,
            cy: 1.0,
            w: 788.0,
            h: 179.0,
        }, // 称号栏边框
        ElementLayout {
            cx: -188.0,
            cy: 0.0,
            w: 378.0,
            h: 80.0,
        }, // 称号1 (大/完整)
        ElementLayout {
            cx: 101.0,
            cy: 0.0,
            w: 180.0,
            h: 81.0,
        }, // 称号2 (小)
        ElementLayout {
            cx: 288.0,
            cy: 1.0,
            w: 178.0,
            h: 81.0,
        }, // 称号3
    ],
};

/// type=3 主要组合 (844×305)
///
/// 面板内只有一个"主要组合栏" (783×243)，5 张卡面在栏内等宽排列。
/// 每张卡面宽 = 栏宽/5 ≈ 156.6px，高 = 栏高 243px。
///
/// ## 素材来源
/// - 卡面缩略图: `card_member/{cardId}/1/{normal|after_training}`
///   来自 S3 `startapp/card_member/{cardId}/card_after_training.png`
/// - 数据来源: `userDeck.members[0..5]` → 5 个 cardId
///
/// elements: [主要组合栏]
pub static DECK: PanelLayout = PanelLayout {
    w: 844.0,
    h: 305.0,
    elements: &[
        ElementLayout {
            cx: 1.0,
            cy: 3.0,
            w: 783.0,
            h: 243.0,
        }, // 主要组合栏（5 张卡面容器）
    ],
};
