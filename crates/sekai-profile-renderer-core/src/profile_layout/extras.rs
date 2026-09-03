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
        }, // [0] Character-rank tab
        ElementLayout {
            cx: 188.0,
            cy: 355.0,
            w: 378.0,
            h: 59.0,
        }, // [1] Challenge-stage tab
        ElementLayout {
            cx: -1.0,
            cy: -40.0,
            w: 828.0,
            h: 683.0,
        }, // [2] Scrollable content area
        ElementLayout {
            cx: -317.0,
            cy: 259.0,
            w: 197.0,
            h: 86.0,
        }, // [3] First character row
        ElementLayout {
            cx: -373.0,
            cy: 261.0,
            w: 85.0,
            h: 84.0,
        }, // [4] Sample character avatar
        ElementLayout {
            cx: -288.0,
            cy: 247.0,
            w: 39.0,
            h: 29.0,
        }, // [5] Sample character rank
    ],
};

/// Type 14: favorite stories (967x872).
///
/// The heading and divider occupy the upper area. Story artwork is arranged in
/// a two-column grid below it, with each tile measuring about 400x170 pixels.
///
/// ## Grid derivation
/// - Column centers are `-212` and `212`, giving a 424-pixel pitch.
/// - Row centers use an approximately 195-pixel pitch.
/// - Tiles are approximately 400x170 pixels; source measurements vary slightly.
///
/// ## Asset sources
/// Story IDs come from `userStoryFavorites`; the host resolves their artwork.
///
/// Elements: heading, divider, then eight story artwork slots.
pub static STORY_FAVORITE: PanelLayout = PanelLayout {
    w: 967.0,
    h: 872.0,
    elements: &[
        ElementLayout {
            cx: -309.0,
            cy: 367.0,
            w: 192.0,
            h: 34.0,
        }, // [0] Heading
        ElementLayout {
            cx: 1.0,
            cy: 339.0,
            w: 831.0,
            h: 17.0,
        }, // [1] Divider
        ElementLayout {
            cx: -212.0,
            cy: 220.0,
            w: 400.0,
            h: 170.0,
        }, // [2] Story 1, row 1 left
        ElementLayout {
            cx: 212.0,
            cy: 220.0,
            w: 400.0,
            h: 170.0,
        }, // [3] Story 2, row 1 right
        ElementLayout {
            cx: -213.0,
            cy: 25.0,
            w: 402.0,
            h: 171.0,
        }, // [4] Story 3, row 2 left
        ElementLayout {
            cx: 212.0,
            cy: 25.0,
            w: 400.0,
            h: 170.0,
        }, // [5] Story 4, row 2 right, derived
        ElementLayout {
            cx: -213.0,
            cy: -170.0,
            w: 401.0,
            h: 170.0,
        }, // [6] Story 5, row 3 left, derived
        ElementLayout {
            cx: 212.0,
            cy: -170.0,
            w: 400.0,
            h: 170.0,
        }, // [7] Story 6, row 3 right, derived
        ElementLayout {
            cx: -213.0,
            cy: -365.0,
            w: 401.0,
            h: 170.0,
        }, // [8] Story 7, row 4 left, derived
        ElementLayout {
            cx: 212.0,
            cy: -365.0,
            w: 400.0,
            h: 170.0,
        }, // [9] Story 8, row 4 right, derived
    ],
};
