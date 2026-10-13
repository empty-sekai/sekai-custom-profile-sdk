//! Normalized player data required by profile-card components.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HonorSlot {
    pub honor_id: i32,
    pub honor_level: i32,
    pub full_size: bool,
    pub profile_honor_type: String,
    pub bonds_honor_word_id: Option<i64>,
    pub bonds_honor_view_type: Option<String>,
}

/// Decodes the game's composable bonds-honor view type.
pub fn bonds_honor_view_flags(view_type: Option<&str>) -> (bool, bool) {
    let view_type = view_type.unwrap_or_default();
    (
        view_type.contains("reverse"),
        view_type.contains("unit_virtual_singer"),
    )
}

impl HonorSlot {
    /// Decodes the game's composable bonds-honor view type.
    ///
    /// Values may combine `reverse` and `unit_virtual_singer`; treating the
    /// field as an enum would silently discard one of those flags.
    pub fn bonds_honor_view_flags(&self) -> (bool, bool) {
        bonds_honor_view_flags(self.bonds_honor_view_type.as_deref())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardState {
    pub card_id: i32,
    /// The player shows the trained illustration (`defaultImage` is
    /// `special_training`).
    pub after_training: bool,
    /// The card has finished special training (`specialTrainingStatus` is
    /// `done`). Rarity stars follow this, not the illustration choice.
    #[serde(default)]
    pub special_training_done: bool,
    pub master_rank: i32,
    pub level: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MusicDifficultyStats {
    pub clear: i32,
    pub full_combo: i32,
    pub all_perfect: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MusicResults {
    pub easy: MusicDifficultyStats,
    pub normal: MusicDifficultyStats,
    pub hard: MusicDifficultyStats,
    pub expert: MusicDifficultyStats,
    pub master: MusicDifficultyStats,
    pub append: MusicDifficultyStats,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterRank {
    pub character_id: i32,
    pub rank: i32,
}

/// Story types the favorite-story panel shows; entries of any other type are
/// left out.
pub const STORY_FAVORITE_TYPES: [&str; 2] = ["event_story", "unit_story"];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryFavorite {
    pub story_id: i32,
    pub story_type: String,
    /// One-based panel slot (`shareNo`).
    #[serde(default)]
    pub share_no: i32,
}

/// Arranges favorite stories by panel slot: entry `i` is the story shared as
/// `shareNo` `i + 1`, or `None` when nobody shares that slot. When two stories
/// name the same slot the later one takes it, as in the game. Stories without
/// a slot (`shareNo` below 1) are left out.
pub fn story_favorite_slots<T>(stories: &[T], share_no: impl Fn(&T) -> i32) -> Vec<Option<&T>> {
    let mut slots = Vec::new();
    for story in stories {
        let Ok(slot) = usize::try_from(share_no(story) - 1) else {
            continue;
        };
        if slots.len() <= slot {
            slots.resize(slot + 1, None);
        }
        slots[slot] = Some(story);
    }
    slots
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileData {
    pub user_name: String,
    pub user_rank: i32,
    pub total_power: i64,
    pub word: String,
    pub mvp: i32,
    pub superstar: i32,
    pub challenge_score: i32,
    pub challenge_character_id: i32,
    pub leader_card: Option<CardState>,
    pub honor_slots: Vec<HonorSlot>,
    pub deck_members: Vec<CardState>,
    pub music_results: Option<MusicResults>,
    pub character_ranks: Vec<CharacterRank>,
    pub challenge_ranks: Vec<CharacterRank>,
    pub story_favorites: Vec<StoryFavorite>,
    pub honor_mission_progress: BTreeMap<String, i32>,
    pub user_cards: BTreeMap<i32, CardState>,
    #[serde(default)]
    pub owned_honors: OwnedHonorLevels,
}

/// Levels of the honors a player owns, read from `userHonors` and
/// `userBondsHonors`. A list the response does not carry stays `None`:
/// ownership is then unknown rather than empty.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedHonorLevels {
    pub honors: Option<BTreeMap<i32, i32>>,
    pub bonds_honors: Option<BTreeMap<i32, i32>>,
}

impl OwnedHonorLevels {
    pub fn from_json(body: &Value) -> Self {
        Self {
            honors: owned_levels(body.get("userHonors"), "honorId"),
            bonds_honors: owned_levels(body.get("userBondsHonors"), "bondsHonorId"),
        }
    }
}

/// Level a standard honor placed on a card is drawn with.
///
/// The game draws a placed honor only when the player owns it, at the owned
/// level; `None` means the element is not drawn. Without a `userHonors` list
/// the element's own level is kept.
pub fn placed_honor_level(
    owned: Option<&OwnedHonorLevels>,
    honor_id: i32,
    element_level: i32,
) -> Option<i32> {
    placed_level(
        owned.and_then(|value| value.honors.as_ref()),
        honor_id,
        element_level,
    )
}

/// Level a bonds honor placed on a card is drawn with; the same ownership
/// rule as [`placed_honor_level`] applied to `userBondsHonors`.
pub fn placed_bonds_honor_level(
    owned: Option<&OwnedHonorLevels>,
    bonds_honor_id: i32,
    element_level: i32,
) -> Option<i32> {
    placed_level(
        owned.and_then(|value| value.bonds_honors.as_ref()),
        bonds_honor_id,
        element_level,
    )
}

fn placed_level(levels: Option<&BTreeMap<i32, i32>>, id: i32, element_level: i32) -> Option<i32> {
    match levels {
        Some(levels) => levels.get(&id).copied(),
        None => Some(element_level),
    }
}

/// Reads `[id, level, obtainedAt]` rows, the compact form `userHonors` is
/// sent in, and `{ "<id_field>": id, "level": level }` rows alike.
fn owned_levels(value: Option<&Value>, id_field: &str) -> Option<BTreeMap<i32, i32>> {
    let rows = value?.as_array()?;
    Some(
        rows.iter()
            .filter_map(|row| {
                let (id, level) = match row {
                    Value::Array(fields) => (fields.first()?.as_i64()?, fields.get(1)?.as_i64()?),
                    _ => (row.get(id_field)?.as_i64()?, row.get("level")?.as_i64()?),
                };
                Some((id as i32, level as i32))
            })
            .collect(),
    )
}

impl ProfileData {
    /// Extracts only renderer-owned fields from an unmodified profile API response.
    pub fn from_json(body: &Value) -> Self {
        let mut output = Self::default();
        if let Some(user) = body.get("user") {
            output.user_name = string(user, "name");
            output.user_rank = integer(user, "rank");
        }
        output.word = body
            .get("userProfile")
            .map(|v| string(v, "word"))
            .unwrap_or_default();
        output.total_power = body
            .get("totalPower")
            .and_then(|v| v.get("totalPower"))
            .and_then(Value::as_i64)
            .unwrap_or_default();
        if let Some(score) = body.get("userMultiLiveTopScoreCount") {
            output.mvp = integer(score, "mvp");
            output.superstar = integer(score, "superStar");
        }
        if let Some(challenge) = body.get("userChallengeLiveSoloResult") {
            output.challenge_score = integer(challenge, "highScore");
            output.challenge_character_id = integer(challenge, "characterId");
        }
        output.character_ranks = ranks(body.get("userCharacters"), "characterRank");
        output.challenge_ranks = ranks(body.get("userChallengeLiveSoloStages"), "rank");

        if let Some(cards) = body.get("userCards").and_then(Value::as_array) {
            for card in cards {
                let card_id = integer(card, "cardId");
                if card_id <= 0 {
                    continue;
                }
                output.user_cards.insert(
                    card_id,
                    CardState {
                        card_id,
                        after_training: card.get("defaultImage").and_then(Value::as_str)
                            == Some("special_training"),
                        special_training_done: card
                            .get("specialTrainingStatus")
                            .and_then(Value::as_str)
                            == Some("done"),
                        master_rank: integer(card, "masterRank"),
                        level: card.get("level").and_then(Value::as_i64).unwrap_or(60) as i32,
                    },
                );
            }
        }
        if let Some(deck) = body.get("userDeck") {
            for index in 1..=5 {
                let card_id = integer(deck, &format!("member{index}"));
                if card_id > 0 {
                    output.deck_members.push(output.card_state(card_id));
                }
            }
            let leader = integer(deck, "leader");
            if leader > 0 {
                output.leader_card = Some(output.card_state(leader));
            }
        }
        if let Some(slots) = body.get("userProfileHonors").and_then(Value::as_array) {
            output.honor_slots = slots
                .iter()
                .map(|slot| HonorSlot {
                    honor_id: integer(slot, "honorId"),
                    honor_level: integer(slot, "honorLevel").max(1),
                    full_size: integer(slot, "seq") == 2,
                    profile_honor_type: optional_string(slot, "profileHonorType")
                        .unwrap_or_else(|| "normal".into()),
                    bonds_honor_word_id: slot.get("bondsHonorWordId").and_then(Value::as_i64),
                    bonds_honor_view_type: optional_string(slot, "bondsHonorViewType"),
                })
                .collect();
        }
        if let Some(stories) = body.get("userStoryFavorites").and_then(Value::as_array) {
            output.story_favorites = stories
                .iter()
                .filter_map(|story| {
                    let story_id = integer(story, "storyId");
                    let share_no = integer(story, "shareNo");
                    let story_type = optional_string(story, "storyType")
                        .filter(|value| STORY_FAVORITE_TYPES.contains(&value.as_str()))?;
                    (story_id > 0 && share_no >= 1).then_some(StoryFavorite {
                        story_id,
                        story_type,
                        share_no,
                    })
                })
                .collect();
        }
        if let Some(missions) = body.get("userHonorMissions").and_then(Value::as_array) {
            for mission in missions {
                if let Some(kind) = optional_string(mission, "honorMissionType") {
                    output
                        .honor_mission_progress
                        .insert(kind, integer(mission, "progress"));
                }
            }
        }
        output.music_results = body
            .get("userMusicDifficultyClearCount")
            .and_then(Value::as_array)
            .map(|rows| parse_music_results(rows));
        output.owned_honors = OwnedHonorLevels::from_json(body);
        output
    }

    fn card_state(&self, card_id: i32) -> CardState {
        self.user_cards.get(&card_id).cloned().unwrap_or(CardState {
            card_id,
            level: 60,
            ..CardState::default()
        })
    }
}

fn ranks(value: Option<&Value>, rank_field: &str) -> Vec<CharacterRank> {
    let mut maximum = BTreeMap::<i32, i32>::new();
    for entry in value.and_then(Value::as_array).into_iter().flatten() {
        let character_id = integer(entry, "characterId");
        if character_id > 0 {
            maximum
                .entry(character_id)
                .and_modify(|rank| *rank = (*rank).max(integer(entry, rank_field)))
                .or_insert_with(|| integer(entry, rank_field));
        }
    }
    maximum
        .into_iter()
        .map(|(character_id, rank)| CharacterRank { character_id, rank })
        .collect()
}

fn parse_music_results(rows: &[Value]) -> MusicResults {
    fn one(rows: &[Value], name: &str) -> MusicDifficultyStats {
        let value = rows
            .iter()
            .find(|row| row.get("musicDifficultyType").and_then(Value::as_str) == Some(name))
            .unwrap_or(&Value::Null);
        MusicDifficultyStats {
            clear: integer(value, "liveClear"),
            full_combo: integer(value, "fullCombo"),
            all_perfect: integer(value, "allPerfect"),
        }
    }
    MusicResults {
        easy: one(rows, "easy"),
        normal: one(rows, "normal"),
        hard: one(rows, "hard"),
        expert: one(rows, "expert"),
        master: one(rows, "master"),
        append: one(rows, "append"),
    }
}

fn integer(value: &Value, key: &str) -> i32 {
    value.get(key).and_then(Value::as_i64).unwrap_or_default() as i32
}
fn string(value: &Value, key: &str) -> String {
    optional_string(value, key).unwrap_or_default()
}
fn optional_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_profile_components_without_retaining_raw_json() {
        let raw = serde_json::json!({
            "user": { "name": "Sample", "rank": 42, "privateToken": "must-not-survive" },
            "userProfile": { "word": "Hello" },
            "userCards": [{ "cardId": 7, "defaultImage": "special_training", "masterRank": 3, "level": 60 }],
            "userDeck": { "leader": 7, "member1": 7 },
            "userCharacters": [{ "characterId": 2, "characterRank": 31 }],
            "userChallengeLiveSoloStages": [
                { "characterId": 2, "rank": 1 },
                { "characterId": 2, "rank": 9 },
                { "characterId": 2, "rank": 4 }
            ]
        });
        let profile = ProfileData::from_json(&raw);
        assert_eq!(profile.user_name, "Sample");
        assert_eq!(
            profile.leader_card.as_ref().map(|card| card.after_training),
            Some(true)
        );
        assert_eq!(profile.character_ranks[0].rank, 31);
        assert_eq!(profile.challenge_ranks[0].rank, 9);
        assert!(!serde_json::to_string(&profile)
            .unwrap()
            .contains("privateToken"));
    }

    #[test]
    fn owned_honor_lists_decide_placed_honor_levels() {
        let profile = ProfileData::from_json(&serde_json::json!({
            "userHonors": [[5, 12, 1700000000000_i64], { "honorId": 6, "level": 2 }, ["bad"]],
            "userBondsHonors": [{ "bondsHonorId": 7, "level": 3 }]
        }));
        let owned = Some(&profile.owned_honors);
        assert_eq!(placed_honor_level(owned, 5, 1), Some(12));
        assert_eq!(placed_honor_level(owned, 6, 1), Some(2));
        assert_eq!(placed_honor_level(owned, 8, 1), None);
        assert_eq!(placed_bonds_honor_level(owned, 7, 1), Some(3));
        assert_eq!(placed_bonds_honor_level(owned, 5, 1), None);

        // A response without the lists leaves ownership unknown.
        let unknown = ProfileData::from_json(&serde_json::json!({}));
        assert_eq!(unknown.owned_honors, OwnedHonorLevels::default());
        assert_eq!(
            placed_honor_level(Some(&unknown.owned_honors), 8, 4),
            Some(4)
        );
        assert_eq!(placed_bonds_honor_level(None, 8, 2), Some(2));
    }

    #[test]
    fn rarity_training_follows_special_training_status_not_the_illustration() {
        let profile = ProfileData::from_json(&serde_json::json!({
            "userCards": [
                { "cardId": 1, "defaultImage": "original", "specialTrainingStatus": "done" },
                { "cardId": 2, "defaultImage": "special_training", "specialTrainingStatus": "done" },
                { "cardId": 3, "defaultImage": "original", "specialTrainingStatus": "not_doing" }
            ]
        }));
        let state = |id: i32| {
            let card = &profile.user_cards[&id];
            (card.after_training, card.special_training_done)
        };
        assert_eq!(state(1), (false, true));
        assert_eq!(state(2), (true, true));
        assert_eq!(state(3), (false, false));
    }

    #[test]
    fn story_favorites_keep_share_slots_and_only_panel_story_types() {
        let profile = ProfileData::from_json(&serde_json::json!({
            "userStoryFavorites": [
                { "storyId": 30, "storyType": "event_story", "shareNo": 3 },
                { "storyId": 31, "storyType": "unit_story", "shareNo": 1 },
                { "storyId": 32, "storyType": "event_story", "shareNo": 0 },
                { "storyId": 33, "storyType": "card_story", "shareNo": 2 }
            ]
        }));
        assert_eq!(
            profile
                .story_favorites
                .iter()
                .map(|story| (story.story_id, story.share_no))
                .collect::<Vec<_>>(),
            vec![(30, 3), (31, 1)]
        );
    }

    #[test]
    fn story_favorite_slots_follow_share_numbers() {
        let stories = [(30, 3), (31, 1), (32, 3), (33, 0)];
        let slots = story_favorite_slots(&stories, |story| story.1);
        assert_eq!(
            slots
                .iter()
                .map(|slot| slot.map(|story| story.0))
                .collect::<Vec<_>>(),
            vec![Some(31), None, Some(32)]
        );
    }

    #[test]
    fn bonds_honor_view_type_preserves_composed_flags() {
        let slot = HonorSlot {
            bonds_honor_view_type: Some("reverse_unit_virtual_singer".into()),
            ..HonorSlot::default()
        };
        assert_eq!(slot.bonds_honor_view_flags(), (true, true));

        let normal = HonorSlot::default();
        assert_eq!(normal.bonds_honor_view_flags(), (false, false));
    }
}
