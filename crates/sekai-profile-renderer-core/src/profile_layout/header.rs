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
        }, // Name plate
        ElementLayout {
            cx: -11.0,
            cy: 0.0,
            w: 449.0,
            h: 34.0,
        }, // Player name
    ],
};

/// Type 2: total power (813x136).
/// Elements: label, power icon, divider, numeric value, and info icon.
pub static TOTAL_POWER: PanelLayout = PanelLayout {
    w: 813.0,
    h: 136.0,
    elements: &[
        ElementLayout {
            cx: -328.0,
            cy: -1.0,
            w: 92.0,
            h: 33.0,
        }, // Total-power label
        ElementLayout {
            cx: -231.0,
            cy: 2.0,
            w: 45.0,
            h: 48.0,
        }, // Total-power icon
        ElementLayout {
            cx: -266.0,
            cy: 1.0,
            w: 12.0,
            h: 34.0,
        }, // Divider
        ElementLayout {
            cx: -116.0,
            cy: 0.0,
            w: 112.0,
            h: 33.0,
        }, // Numeric value
        ElementLayout {
            cx: -13.0,
            cy: 3.0,
            w: 64.0,
            h: 61.0,
        }, // Circular info icon
    ],
};

/// type=4 profile comment (700×251)
/// elements: [comment textbox, localized title, edit icon]
pub static COMMENT: PanelLayout = PanelLayout {
    w: 700.0,
    h: 251.0,
    elements: &[
        ElementLayout {
            cx: 2.0,
            cy: -25.0,
            w: 633.0,
            h: 142.0,
        }, // comment textbox
        ElementLayout {
            cx: -250.0,
            cy: 82.0,
            w: 125.0,
            h: 35.0,
        }, // localized title
        ElementLayout {
            cx: 279.0,
            cy: 13.0,
            w: 51.0,
            h: 51.0,
        }, // Edit icon, intentionally not rendered
    ],
};
