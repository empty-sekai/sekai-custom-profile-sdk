// Auto-split from layout.rs

use super::{ElementLayout, PanelLayout};

pub static CHAR_RANK: PanelLayout = PanelLayout {
    w: 967.0,
    h: 872.0,
    elements: &[
        ElementLayout {
            cx: -189.0,
            cy: 355.0,
            w: 378.0,
            h: 58.0,
        }, // [0] 角色收藏等级 tab
        ElementLayout {
            cx: 188.0,
            cy: 355.0,
            w: 378.0,
            h: 59.0,
        }, // [1] 挑战舞台 tab
        ElementLayout {
            cx: -1.0,
            cy: -40.0,
            w: 828.0,
            h: 683.0,
        }, // [2] 情况栏区域
        ElementLayout {
            cx: -317.0,
            cy: 259.0,
            w: 197.0,
            h: 86.0,
        }, // [3] 角色1 (首列首行)
        ElementLayout {
            cx: -373.0,
            cy: 261.0,
            w: 85.0,
            h: 84.0,
        }, // [4] 角色头像 (样本)
        ElementLayout {
            cx: -288.0,
            cy: 247.0,
            w: 39.0,
            h: 29.0,
        }, // [5] 角色等级 (样本)
    ],
};

/// type=14 最喜欢的剧情 (967×872)
///
/// 上部: 标题文本 + 横线
/// 下部: 剧情图片网格（2 列，每张约 400×170）
///
/// ## 网格推算（从测绘样本）
/// - 图片1 cx=-212, 图片2 cx=212 → 列间距 424px, 2 列
/// - 图片1 cy=220, 图片3 cy=25 → 行间距 ≈ 195px
/// - 每张图片 ≈ 400×170（略有测绘误差）
///
/// ## 素材来源
/// - TODO: 剧情封面图的 AssetStore key 待用户微调确认
/// - 数据来源: `userStoryFavorites` → S3 剧情封面图
///
/// elements: [标题文本, 横线, 图片1, 图片2, 图片3, 不完全图片]
pub static STORY_FAVORITE: PanelLayout = PanelLayout {
    w: 967.0,
    h: 872.0,
    elements: &[
        ElementLayout {
            cx: -309.0,
            cy: 367.0,
            w: 192.0,
            h: 34.0,
        }, // [0] 标题文本
        ElementLayout {
            cx: 1.0,
            cy: 339.0,
            w: 831.0,
            h: 17.0,
        }, // [1] 横线
        ElementLayout {
            cx: -212.0,
            cy: 220.0,
            w: 400.0,
            h: 170.0,
        }, // [2] 剧情图片1 (第1行左)
        ElementLayout {
            cx: 212.0,
            cy: 220.0,
            w: 400.0,
            h: 170.0,
        }, // [3] 剧情图片2 (第1行右)
        ElementLayout {
            cx: -213.0,
            cy: 25.0,
            w: 402.0,
            h: 171.0,
        }, // [4] 剧情图片3 (第2行左)
        ElementLayout {
            cx: 212.0,
            cy: 25.0,
            w: 400.0,
            h: 170.0,
        }, // [5] 剧情图片4 (第2行右, 推算)
        ElementLayout {
            cx: -213.0,
            cy: -170.0,
            w: 401.0,
            h: 170.0,
        }, // [6] 剧情图片5 (第3行左, 推算)
        ElementLayout {
            cx: 212.0,
            cy: -170.0,
            w: 400.0,
            h: 170.0,
        }, // [7] 剧情图片6 (第3行右, 推算)
        ElementLayout {
            cx: -213.0,
            cy: -365.0,
            w: 401.0,
            h: 170.0,
        }, // [8] 剧情图片7 (第4行左, 推算)
        ElementLayout {
            cx: 212.0,
            cy: -365.0,
            w: 400.0,
            h: 170.0,
        }, // [9] 剧情图片8 (第4行右, 推算)
    ],
};
