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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardState {
    pub card_id: i32,
    pub after_training: bool,
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryFavorite {
    pub story_id: i32,
    pub story_type: String,
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
                    let story_type = optional_string(story, "storyType")?;
                    (story_id > 0).then_some(StoryFavorite {
                        story_id,
                        story_type,
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
}
