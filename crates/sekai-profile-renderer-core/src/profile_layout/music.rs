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
        }, // [0] Clear, full-combo, or all-perfect heading
        ElementLayout {
            cx: -349.0,
            cy: -21.0,
            w: 131.0,
            h: 44.0,
        }, // [1] EASY label
        ElementLayout {
            cx: -347.0,
            cy: -63.0,
            w: 20.0,
            h: 29.0,
        }, // [2] EASY count
        ElementLayout {
            cx: -213.0,
            cy: -21.0,
            w: 128.0,
            h: 42.0,
        }, // [3] NORMAL label
        ElementLayout {
            cx: -213.0,
            cy: -63.0,
            w: 38.0,
            h: 29.0,
        }, // [4] NORMAL count
        ElementLayout {
            cx: -79.0,
            cy: -21.0,
            w: 129.0,
            h: 39.0,
        }, // [5] HARD label
        ElementLayout {
            cx: -77.0,
            cy: -62.0,
            w: 55.0,
            h: 28.0,
        }, // [6] HARD count
        ElementLayout {
            cx: 58.0,
            cy: -20.0,
            w: 129.0,
            h: 41.0,
        }, // [7] EXPERT label
        ElementLayout {
            cx: 57.0,
            cy: -62.0,
            w: 56.0,
            h: 28.0,
        }, // [8] EXPERT count
        ElementLayout {
            cx: 193.0,
            cy: -20.0,
            w: 130.0,
            h: 43.0,
        }, // [9] MASTER label
        ElementLayout {
            cx: 193.0,
            cy: -62.0,
            w: 34.0,
            h: 29.0,
        }, // [10] MASTER count
        ElementLayout {
            cx: 271.0,
            cy: -46.0,
            w: 13.0,
            h: 88.0,
        }, // [11] Vertical divider
        ElementLayout {
            cx: 349.0,
            cy: -19.0,
            w: 129.0,
            h: 43.0,
        }, // [12] APPEND label
        ElementLayout {
            cx: 350.0,
            cy: -63.0,
            w: 14.0,
            h: 29.0,
        }, // [13] APPEND count
    ],
};

/// Type 12: detailed song completion panel (939x330).
///
/// Three groups show clear, full-combo, and all-perfect progress. Each group
/// contains a gray heading pill and a value row.
///
/// ## Asset sources
/// This text-and-icon panel has no remote image dependency. It uses the same
/// source records as type 16.
///
/// Elements: clear heading/value, full-combo heading/value, and all-perfect
/// heading/value.
pub static MUSIC_CLEAR: PanelLayout = PanelLayout {
    w: 939.0,
    h: 330.0,
    elements: &[
        ElementLayout {
            cx: 0.0,
            cy: 132.0,
            w: 862.0,
            h: 40.0,
        }, // [0] Clear heading pill
        ElementLayout {
            cx: 0.0,
            cy: 82.0,
            w: 828.0,
            h: 58.0,
        }, // [1] Clear values
        ElementLayout {
            cx: 0.0,
            cy: 25.0,
            w: 860.0,
            h: 40.0,
        }, // [2] Full-combo heading pill
        ElementLayout {
            cx: 0.0,
            cy: -25.0,
            w: 828.0,
            h: 58.0,
        }, // [3] Full-combo values
        ElementLayout {
            cx: 0.0,
            cy: -82.0,
            w: 860.0,
            h: 40.0,
        }, // [4] All-perfect heading pill
        ElementLayout {
            cx: 0.0,
            cy: -132.0,
            w: 828.0,
            h: 58.0,
        }, // [5] All-perfect values
    ],
};
