// Auto-split from layout.rs

use super::{ElementLayout, PanelLayout};

pub static MVP_SUPERSTAR: PanelLayout = PanelLayout {
    w: 922.0,
    h: 228.0,
    elements: &[
        ElementLayout {
            cx: -333.0,
            cy: 45.0,
            w: 124.0,
            h: 31.0,
        }, // Multiplayer-show label
        ElementLayout {
            cx: -346.0,
            cy: -33.0,
            w: 121.0,
            h: 56.0,
        }, // MVP icon
        ElementLayout {
            cx: -65.0,
            cy: -33.0,
            w: 90.0,
            h: 42.0,
        }, // MVP count
        ElementLayout {
            cx: 85.0,
            cy: -33.0,
            w: 125.0,
            h: 57.0,
        }, // SUPERSTAR icon
        ElementLayout {
            cx: 194.0,
            cy: -33.0,
            w: 69.0,
            h: 31.0,
        }, // SUPERSTAR count
        ElementLayout {
            cx: 7.0,
            cy: 20.0,
            w: 828.0,
            h: 20.0,
        }, // Divider
    ],
};

/// Type 10: challenge show (921x240).
pub static CHALLENGE_LIVE: PanelLayout = PanelLayout {
    w: 921.0,
    h: 240.0,
    elements: &[
        ElementLayout {
            cx: -334.0,
            cy: 53.0,
            w: 129.0,
            h: 32.0,
        }, // Challenge-show label
        ElementLayout {
            cx: -345.0,
            cy: -32.0,
            w: 123.0,
            h: 58.0,
        }, // Solo icon in a gray rounded box
        ElementLayout {
            cx: -222.0,
            cy: -32.0,
            w: 92.0,
            h: 92.0,
        }, // Character avatar
        ElementLayout {
            cx: -75.0,
            cy: -33.0,
            w: 127.0,
            h: 33.0,
        }, // Score
        ElementLayout {
            cx: 6.0,
            cy: 25.0,
            w: 830.0,
            h: 11.0,
        }, // Divider
    ],
};
