//! Source model for custom profile cards.

use serde::{Deserialize, Serialize};

use super::{
    BondsHonorElement, CardMemberElement, CollectionElement, GeneralBackgroundElement,
    GeneralElement, HonorElement, OtherElement, ShapeElement, StampElement, StandMemberElement,
    StoryBackgroundElement, TextElement,
};

/// Complete source data for one custom profile-card page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomProfileCard {
    #[serde(default)]
    pub texts: Vec<TextElement>,
    #[serde(default)]
    pub shapes: Vec<ShapeElement>,
    #[serde(default)]
    pub card_members: Vec<CardMemberElement>,
    #[serde(default)]
    pub stamps: Vec<StampElement>,
    #[serde(default)]
    pub others: Vec<OtherElement>,
    #[serde(default)]
    pub bonds_honors: Vec<BondsHonorElement>,
    #[serde(default)]
    pub honors: Vec<HonorElement>,
    #[serde(default)]
    pub collections: Vec<CollectionElement>,
    #[serde(default)]
    pub generals: Vec<GeneralElement>,
    #[serde(default)]
    pub general_backgrounds: Vec<GeneralBackgroundElement>,
    #[serde(default)]
    pub stand_members: Vec<StandMemberElement>,
    #[serde(default)]
    pub story_backgrounds: Vec<StoryBackgroundElement>,
}

/// Wrapper returned by the Profile API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserCustomProfileCard {
    pub custom_profile_card: CustomProfileCard,
    pub custom_profile_card_id: i32,
    pub custom_profile_id: i32,
    pub seq: i32,
}

/// Order in which the game shows a profile's pages: indices into `pages` by
/// ascending `seq`, with the response order breaking ties. The Profile API
/// does not return pages sorted, so a page number must be looked up through
/// this order rather than by array position.
pub fn profile_page_order<T>(pages: &[T], seq: impl Fn(&T) -> i32) -> Vec<usize> {
    let mut order = (0..pages.len()).collect::<Vec<_>>();
    order.sort_by_key(|&index| seq(&pages[index]));
    order
}

/// The zero-based `index`-th page in the order the game shows them.
pub fn profile_page(
    cards: &[UserCustomProfileCard],
    index: usize,
) -> Option<&UserCustomProfileCard> {
    profile_page_order(cards, |card| card.seq)
        .get(index)
        .map(|&position| &cards[position])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_follow_ascending_seq_not_response_order() {
        let pages = [6, 10, 5, 8, 5]
            .into_iter()
            .enumerate()
            .map(|(position, seq)| UserCustomProfileCard {
                custom_profile_card: serde_json::from_value(serde_json::json!({})).unwrap(),
                custom_profile_card_id: position as i32,
                custom_profile_id: 1,
                seq,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            profile_page_order(&pages, |page| page.seq),
            vec![2, 4, 0, 3, 1]
        );
        assert_eq!(
            profile_page(&pages, 0).map(|page| page.custom_profile_card_id),
            Some(2)
        );
        assert_eq!(
            profile_page(&pages, 4).map(|page| page.custom_profile_card_id),
            Some(1)
        );
        assert!(profile_page(&pages, 5).is_none());
    }
}
