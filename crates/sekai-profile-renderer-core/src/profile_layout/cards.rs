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
        }, // Leader card artwork
        ElementLayout {
            cx: -415.0,
            cy: -137.0,
            w: 63.0,
            h: 203.0,
        }, // Rarity stars
        ElementLayout {
            cx: 386.0,
            cy: 219.0,
            w: 85.0,
            h: 92.0,
        }, // Attribute badge
        ElementLayout {
            cx: 392.0,
            cy: -187.0,
            w: 108.0,
            h: 107.0,
        }, // Master-rank artwork
    ],
};

/// Type 6: honors (844x241).
pub static HONORS: PanelLayout = PanelLayout {
    w: 844.0,
    h: 241.0,
    elements: &[
        ElementLayout {
            cx: 2.0,
            cy: 1.0,
            w: 788.0,
            h: 179.0,
        }, // Honor-row frame
        ElementLayout {
            cx: -188.0,
            cy: 0.0,
            w: 378.0,
            h: 80.0,
        }, // First honor, full size
        ElementLayout {
            cx: 101.0,
            cy: 0.0,
            w: 180.0,
            h: 81.0,
        }, // Second honor, compact
        ElementLayout {
            cx: 288.0,
            cy: 1.0,
            w: 178.0,
            h: 81.0,
        }, // Third honor
    ],
};

/// Type 3: main deck (844x305).
///
/// The panel contains one 783x243 deck row with five equally spaced cards.
/// Each slot is approximately 156.6 pixels wide and 243 pixels high.
///
/// ## Asset sources
/// - Card thumbnail: `card_member/{cardId}/1/{normal|after_training}`.
/// - Source record: `userDeck.members[0..5]` containing five card IDs.
///
/// Elements: `[main deck row]`.
pub static DECK: PanelLayout = PanelLayout {
    w: 844.0,
    h: 305.0,
    elements: &[
        ElementLayout {
            cx: 1.0,
            cy: 3.0,
            w: 783.0,
            h: 243.0,
        }, // Main deck row containing five cards
    ],
};
