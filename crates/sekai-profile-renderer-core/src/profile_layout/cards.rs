//! Measured layouts for the panels that show cards.

// Auto-split from layout.rs

use super::{ElementLayout, PanelLayout};

/// Type 5: leader card (997x589).
///
/// Entries 1 to 3 place the full-size card overlay relative to the artwork;
/// the same offsets apply to a full-size `CardMember` element. The star
/// column is the four-star column, each star as wide as the column.
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
            cx: -419.0,
            cy: -139.0,
            w: 56.0,
            h: 200.0,
        }, // Rarity stars, four-star column
        ElementLayout {
            cx: 385.0,
            cy: 220.0,
            w: 88.0,
            h: 92.0,
        }, // Attribute badge
        ElementLayout {
            cx: 393.0,
            cy: -188.0,
            w: 104.0,
            h: 104.0,
        }, // Master-rank badge
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
/// Each slot is 156.6 pixels wide and 243 pixels high.
///
/// ## Asset sources
/// - Card artwork: `character/member_cutout/{assetbundleName}/{normal|after_training}`.
/// - Source record: `userDeck.member1` to `userDeck.member5`.
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
