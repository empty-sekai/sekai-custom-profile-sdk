//! Player profile data read by the renderer.
//!
//! [`ProfileData`] holds the player information taken from a game profile API
//! response. The profile-driven parts of a card, such as the General panels,
//! read their content from it.
//!
//! - It is plain data with no rendering logic.
//! - JSON parsing lives only in
//!   [`sekai_profile_core::profile_data::ProfileData::from_json`];
//!   [`ProfileData::from_json`] only converts that result into these types.
//! - The level and ownership of an honor placed on a card are decided by
//!   [`sekai_profile_core::profile_data::placed_honor_level`].

use std::collections::HashMap;

// ============================================================
// Panel data types
// ============================================================

/// One honor slot, shown by the honor panel (General type 6).
#[derive(Debug, Clone, Default)]
pub struct HonorSlot {
    /// Honor ID: the primary key of the masterdata `honors` or `bondsHonors`
    /// table, depending on [`Self::profile_honor_type`].
    pub honor_id: i32,
    /// Honor level; it sets the number of stars drawn.
    pub honor_level: i32,
    /// Display preference flag, `true` for the slot the game stores with
    /// `seq == 2`. The semantic card path sorts slots by this flag (large slot
    /// first) and then swaps the first two slots. The render size follows the
    /// slot position after the swap: slot 0 is the 380×80 main slot, the others
    /// are 180×80 sub slots.
    pub full_size: bool,
    /// Honor type: `"normal"` or `"bonds"`.
    pub profile_honor_type: String,
    /// Word ID of a bonds honor; only meaningful for the `bonds` type.
    pub bonds_honor_word_id: Option<i64>,
    /// View type of a bonds honor. It may combine `reverse` (swap the left and
    /// right characters) and `unit_virtual_singer` (show the unit's virtual
    /// singer instead), as in `reverse_unit_virtual_singer`; see
    /// [`Self::bonds_honor_view_flags`].
    pub bonds_honor_view_type: Option<String>,
}

impl HonorSlot {
    /// Decodes [`Self::bonds_honor_view_type`] into `(reverse,
    /// unit_virtual_singer)` flags. A missing view type yields
    /// `(false, false)`.
    pub fn bonds_honor_view_flags(&self) -> (bool, bool) {
        sekai_profile_core::profile_data::bonds_honor_view_flags(
            self.bonds_honor_view_type.as_deref(),
        )
    }
}

/// The leader card, shown by the leader member panel (General type 5).
#[derive(Debug, Clone, Default)]
pub struct LeaderCardInfo {
    /// Card ID.
    pub card_id: i32,
    /// Whether the card shows its trained illustration.
    pub after_training: bool,
    /// Master rank (0–5).
    pub master_rank: i32,
}

/// State of one card the player owns.
#[derive(Debug, Clone, Copy, Default)]
pub struct UserCardInfo {
    /// Whether the card shows its trained illustration.
    pub after_training: bool,
    /// Whether the card has finished special training (`specialTrainingStatus`
    /// is `done`); it decides the rarity stars.
    pub special_training_done: bool,
    /// Master rank.
    pub master_rank: i32,
    /// Current card level.
    pub level: i32,
}

/// One deck member, shown by the deck panel (General type 3).
#[derive(Debug, Clone, Default)]
pub struct DeckMember {
    /// Card ID.
    pub card_id: i32,
    /// Whether the card shows its trained illustration.
    pub after_training: bool,
    /// Whether the card has finished special training; it decides the rarity
    /// stars.
    pub special_training_done: bool,
    /// Master rank (0–5).
    pub master_rank: i32,
    /// Current card level.
    pub level: i32,
}

/// Music results for one difficulty, shown by the music clear panels (General
/// types 12 and 16).
#[derive(Debug, Clone, Default)]
pub struct MusicDifficultyStats {
    /// Number of songs cleared.
    pub clear: i32,
    /// Number of songs with a Full Combo.
    pub full_combo: i32,
    /// Number of songs with an All Perfect.
    pub all_perfect: i32,
}

/// Music results for all six difficulties.
#[derive(Debug, Clone, Default)]
pub struct MusicResults {
    /// Easy difficulty.
    pub easy: MusicDifficultyStats,
    /// Normal difficulty.
    pub normal: MusicDifficultyStats,
    /// Hard difficulty.
    pub hard: MusicDifficultyStats,
    /// Expert difficulty.
    pub expert: MusicDifficultyStats,
    /// Master difficulty.
    pub master: MusicDifficultyStats,
    /// Append difficulty.
    pub append: MusicDifficultyStats,
}

/// A rank for one character, shown by the character rank panel (General
/// types 11 and 15).
#[derive(Debug, Clone, Default)]
pub struct CharacterRankInfo {
    /// Character ID.
    pub character_id: i32,
    /// Rank value.
    pub rank: i32,
}

/// One favorite story, shown by the story favorite panel (General type 14).
#[derive(Debug, Clone, Default)]
pub struct StoryFavoriteInfo {
    /// Story ID, used to find the cover image.
    pub story_id: i32,
    /// Story type, used to build the asset path.
    pub story_type: String,
    /// One-based panel slot (`shareNo`).
    pub share_no: i32,
}

// ============================================================
// ProfileData
// ============================================================

/// Player profile data: plain values with no rendering logic.
///
/// Build it from a profile API response with [`ProfileData::from_json`], or
/// use [`neutral_preview_profile`] for previews.
#[derive(Debug, Clone, Default)]
pub struct ProfileData {
    /// Player name.
    pub user_name: String,
    /// Player rank.
    pub user_rank: i32,
    /// Total power.
    pub total_power: i64,
    /// Player comment (the profile "word").
    pub word: String,
    /// Number of MVP results.
    pub mvp: i32,
    /// Number of SUPERSTAR results.
    pub superstar: i32,
    /// Challenge live score.
    pub challenge_score: i32,
    /// Challenge live character ID; the challenge live panel (type 10) shows
    /// this character's icon.
    pub challenge_character_id: i32,
    /// Leader card, for the leader member panel (type 5).
    pub leader_card: Option<LeaderCardInfo>,
    /// Honor slots for the honor panel (type 6), at most three.
    pub honor_slots: Vec<HonorSlot>,
    /// Deck members for the deck panel (type 3), five members.
    pub deck_members: Vec<DeckMember>,
    /// Music results for the music clear panels (types 12 and 16).
    pub music_results: Option<MusicResults>,
    /// Character ranks for the character rank panel (type 11).
    pub char_ranks: Vec<CharacterRankInfo>,
    /// Per-character challenge live ranks, shown on the second tab of the
    /// character rank panels (types 11 and 15).
    pub challenge_ranks: Vec<CharacterRankInfo>,
    /// Favorite stories for the story favorite panel (type 14).
    pub story_favorites: Vec<StoryFavoriteInfo>,
    /// Live honor mission progress, keyed by `honorMissionType`; read by
    /// `live_master` honors.
    pub user_honor_missions: HashMap<String, i32>,
    /// State of the player's cards, keyed by card ID.
    pub user_cards: HashMap<i32, UserCardInfo>,
    /// Levels of the honors and bonds honors the player owns (`userHonors` /
    /// `userBondsHonors`).
    pub owned_honors: sekai_profile_core::profile_data::OwnedHonorLevels,
}

/// Neutral profile data for previews.
///
/// It is meant for component thumbnails and local template previews. The text
/// is generic and does not describe a real player.
pub fn neutral_preview_profile() -> ProfileData {
    let mut user_cards = HashMap::new();
    let deck_card_ids = [3, 7, 11, 15, 4]; // different characters and rarities
    for (i, &card_id) in deck_card_ids.iter().enumerate() {
        user_cards.insert(
            card_id,
            UserCardInfo {
                after_training: false, // untrained illustrations only
                special_training_done: false,
                master_rank: (i + 1) as i32 % 5,
                level: 60,
            },
        );
    }

    let mut user_honor_missions = HashMap::new();
    user_honor_missions.insert("live_master".to_string(), 50);

    ProfileData {
        user_name: "玩家名".to_string(),
        user_rank: 100,
        total_power: 123_456,
        word: "个性签名".to_string(),
        mvp: 100,
        superstar: 50,
        challenge_score: 1_234_567,
        challenge_character_id: 1,
        leader_card: Some(LeaderCardInfo {
            card_id: 4,
            after_training: false,
            master_rank: 1,
        }),
        honor_slots: vec![
            HonorSlot {
                honor_id: 1,
                honor_level: 10,
                full_size: true,
                profile_honor_type: "normal".to_string(),
                bonds_honor_word_id: None,
                bonds_honor_view_type: None,
            },
            HonorSlot {
                honor_id: 2,
                honor_level: 5,
                full_size: false,
                profile_honor_type: "normal".to_string(),
                bonds_honor_word_id: None,
                bonds_honor_view_type: None,
            },
            HonorSlot {
                honor_id: 3,
                honor_level: 5,
                full_size: false,
                profile_honor_type: "normal".to_string(),
                bonds_honor_word_id: None,
                bonds_honor_view_type: None,
            },
        ],
        deck_members: [
            DeckMember {
                card_id: 3,
                after_training: false,
                special_training_done: false,
                master_rank: 2,
                level: 60,
            },
            DeckMember {
                card_id: 7,
                after_training: false,
                special_training_done: false,
                master_rank: 3,
                level: 60,
            },
            DeckMember {
                card_id: 11,
                after_training: false,
                special_training_done: false,
                master_rank: 4,
                level: 60,
            },
            DeckMember {
                card_id: 15,
                after_training: false,
                special_training_done: false,
                master_rank: 0,
                level: 60,
            },
            DeckMember {
                card_id: 4,
                after_training: false,
                special_training_done: false,
                master_rank: 1,
                level: 60,
            },
        ]
        .to_vec(),
        music_results: Some(MusicResults {
            easy: preview_music_stats(120, 110, 80),
            normal: preview_music_stats(112, 103, 74),
            hard: preview_music_stats(104, 96, 68),
            expert: preview_music_stats(96, 89, 62),
            master: preview_music_stats(88, 82, 56),
            append: preview_music_stats(40, 32, 20),
        }),
        char_ranks: (1..=26)
            .map(|character_id| CharacterRankInfo {
                character_id,
                rank: 60 - (character_id % 8),
            })
            .collect(),
        challenge_ranks: (1..=26)
            .map(|character_id| CharacterRankInfo {
                character_id,
                rank: 10 + character_id,
            })
            .collect(),
        story_favorites: vec![
            StoryFavoriteInfo {
                story_id: 1,
                story_type: "unit".to_string(),
                share_no: 1,
            },
            StoryFavoriteInfo {
                story_id: 2,
                story_type: "unit".to_string(),
                share_no: 2,
            },
            StoryFavoriteInfo {
                story_id: 3,
                story_type: "unit".to_string(),
                share_no: 3,
            },
            StoryFavoriteInfo {
                story_id: 4,
                story_type: "unit".to_string(),
                share_no: 4,
            },
        ],
        user_honor_missions,
        user_cards,
        owned_honors: Default::default(),
    }
}

fn preview_music_stats(clear: i32, full_combo: i32, all_perfect: i32) -> MusicDifficultyStats {
    MusicDifficultyStats {
        clear,
        full_combo,
        all_perfect,
    }
}

// ============================================================
// Construction (JSON parsing lives in sekai-profile-core)
// ============================================================

impl ProfileData {
    /// Builds profile data from a profile API response body.
    ///
    /// Extracts everything the General panels read. Parsing is done once, by
    /// [`sekai_profile_core::profile_data::ProfileData::from_json`], so the
    /// browser and native renderers get the same data from the same response.
    /// Missing or malformed fields fall back to their defaults; this function
    /// does not fail.
    pub fn from_json(body: &serde_json::Value) -> Self {
        Self::from_core(sekai_profile_core::profile_data::ProfileData::from_json(
            body,
        ))
    }

    fn from_core(value: sekai_profile_core::profile_data::ProfileData) -> Self {
        let character_rank =
            |rank: sekai_profile_core::profile_data::CharacterRank| CharacterRankInfo {
                character_id: rank.character_id,
                rank: rank.rank,
            };
        let music_stats =
            |stats: sekai_profile_core::profile_data::MusicDifficultyStats| MusicDifficultyStats {
                clear: stats.clear,
                full_combo: stats.full_combo,
                all_perfect: stats.all_perfect,
            };
        Self {
            user_name: value.user_name,
            user_rank: value.user_rank,
            total_power: value.total_power,
            word: value.word,
            mvp: value.mvp,
            superstar: value.superstar,
            challenge_score: value.challenge_score,
            challenge_character_id: value.challenge_character_id,
            leader_card: value.leader_card.map(|card| LeaderCardInfo {
                card_id: card.card_id,
                after_training: card.after_training,
                master_rank: card.master_rank,
            }),
            honor_slots: value
                .honor_slots
                .into_iter()
                .map(|slot| HonorSlot {
                    honor_id: slot.honor_id,
                    honor_level: slot.honor_level,
                    full_size: slot.full_size,
                    profile_honor_type: slot.profile_honor_type,
                    bonds_honor_word_id: slot.bonds_honor_word_id,
                    bonds_honor_view_type: slot.bonds_honor_view_type,
                })
                .collect(),
            deck_members: value
                .deck_members
                .into_iter()
                .map(|card| DeckMember {
                    card_id: card.card_id,
                    after_training: card.after_training,
                    special_training_done: card.special_training_done,
                    master_rank: card.master_rank,
                    level: card.level,
                })
                .collect(),
            music_results: value.music_results.map(|results| MusicResults {
                easy: music_stats(results.easy),
                normal: music_stats(results.normal),
                hard: music_stats(results.hard),
                expert: music_stats(results.expert),
                master: music_stats(results.master),
                append: music_stats(results.append),
            }),
            char_ranks: value
                .character_ranks
                .into_iter()
                .map(character_rank)
                .collect(),
            challenge_ranks: value
                .challenge_ranks
                .into_iter()
                .map(character_rank)
                .collect(),
            story_favorites: value
                .story_favorites
                .into_iter()
                .map(|favorite| StoryFavoriteInfo {
                    story_id: favorite.story_id,
                    story_type: favorite.story_type,
                    share_no: favorite.share_no,
                })
                .collect(),
            user_honor_missions: value.honor_mission_progress.into_iter().collect(),
            user_cards: value
                .user_cards
                .into_iter()
                .map(|(card_id, card)| {
                    (
                        card_id,
                        UserCardInfo {
                            after_training: card.after_training,
                            special_training_done: card.special_training_done,
                            master_rank: card.master_rank,
                            level: card.level,
                        },
                    )
                })
                .collect(),
            owned_honors: value.owned_honors,
        }
    }

    /// Returns the player's state for `card_id`, or `None` when the profile
    /// does not list that card.
    pub fn user_card(&self, card_id: i32) -> Option<&UserCardInfo> {
        self.user_cards.get(&card_id)
    }

    /// Converts the production profile model back into the backend-neutral
    /// model it was parsed from.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn to_core_profile(&self) -> sekai_profile_core::profile_data::ProfileData {
        use sekai_profile_core::profile_data as core;

        let card_state = |card_id: i32, value: UserCardInfo| core::CardState {
            card_id,
            after_training: value.after_training,
            special_training_done: value.special_training_done,
            master_rank: value.master_rank,
            level: value.level,
        };
        let user_cards = self
            .user_cards
            .iter()
            .map(|(&card_id, &value)| (card_id, card_state(card_id, value)))
            .collect();
        core::ProfileData {
            user_name: self.user_name.clone(),
            user_rank: self.user_rank,
            total_power: self.total_power,
            word: self.word.clone(),
            mvp: self.mvp,
            superstar: self.superstar,
            challenge_score: self.challenge_score,
            challenge_character_id: self.challenge_character_id,
            leader_card: self.leader_card.as_ref().map(|value| core::CardState {
                card_id: value.card_id,
                after_training: value.after_training,
                special_training_done: self
                    .user_cards
                    .get(&value.card_id)
                    .is_some_and(|card| card.special_training_done),
                master_rank: value.master_rank,
                level: self
                    .user_cards
                    .get(&value.card_id)
                    .map(|card| card.level)
                    .unwrap_or(60),
            }),
            honor_slots: self
                .honor_slots
                .iter()
                .map(|value| core::HonorSlot {
                    honor_id: value.honor_id,
                    honor_level: value.honor_level,
                    full_size: value.full_size,
                    profile_honor_type: value.profile_honor_type.clone(),
                    bonds_honor_word_id: value.bonds_honor_word_id,
                    bonds_honor_view_type: value.bonds_honor_view_type.clone(),
                })
                .collect(),
            deck_members: self
                .deck_members
                .iter()
                .map(|value| core::CardState {
                    card_id: value.card_id,
                    after_training: value.after_training,
                    special_training_done: value.special_training_done,
                    master_rank: value.master_rank,
                    level: value.level,
                })
                .collect(),
            music_results: self.music_results.as_ref().map(|value| core::MusicResults {
                easy: music_stats_to_core(&value.easy),
                normal: music_stats_to_core(&value.normal),
                hard: music_stats_to_core(&value.hard),
                expert: music_stats_to_core(&value.expert),
                master: music_stats_to_core(&value.master),
                append: music_stats_to_core(&value.append),
            }),
            character_ranks: self
                .char_ranks
                .iter()
                .map(|value| core::CharacterRank {
                    character_id: value.character_id,
                    rank: value.rank,
                })
                .collect(),
            challenge_ranks: self
                .challenge_ranks
                .iter()
                .map(|value| core::CharacterRank {
                    character_id: value.character_id,
                    rank: value.rank,
                })
                .collect(),
            story_favorites: self
                .story_favorites
                .iter()
                .map(|value| core::StoryFavorite {
                    story_id: value.story_id,
                    story_type: value.story_type.clone(),
                    share_no: value.share_no,
                })
                .collect(),
            honor_mission_progress: self
                .user_honor_missions
                .iter()
                .map(|(key, &value)| (key.clone(), value))
                .collect(),
            user_cards,
            owned_honors: self.owned_honors.clone(),
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn music_stats_to_core(
    value: &MusicDifficultyStats,
) -> sekai_profile_core::profile_data::MusicDifficultyStats {
    sekai_profile_core::profile_data::MusicDifficultyStats {
        clear: value.clear,
        full_combo: value.full_combo,
        all_perfect: value.all_perfect,
    }
}

#[cfg(test)]
mod interaction_profile_tests {
    use super::*;

    #[test]
    fn parses_per_character_challenge_live_ranks_for_interactive_tab() {
        let profile = ProfileData::from_json(&serde_json::json!({
            "userCharacters": [
                { "characterId": 1, "characterRank": 42 },
                { "characterId": 2, "characterRank": 31 }
            ],
            "userChallengeLiveSoloStages": [
                { "characterId": 1, "rank": 1 },
                { "characterId": 1, "rank": 18 },
                { "characterId": 1, "rank": 12 },
                { "characterId": 2, "rank": 1 },
                { "characterId": 2, "rank": 7 }
            ]
        }));
        assert_eq!(profile.char_ranks[0].rank, 42);
        assert_eq!(profile.challenge_ranks[0].rank, 18);
        assert_eq!(profile.challenge_ranks[1].character_id, 2);
    }

    #[test]
    fn production_and_shared_profile_parsers_match_supported_fields() {
        let raw = serde_json::json!({
            "user": { "name": "Sample", "rank": 123 },
            "userProfile": { "word": "Hello" },
            "totalPower": { "totalPower": 456789 },
            "userMultiLiveTopScoreCount": { "mvp": 7, "superStar": 8 },
            "userChallengeLiveSoloResult": { "highScore": 987654, "characterId": 2 },
            "userCards": [
                { "cardId": 10, "defaultImage": "special_training", "specialTrainingStatus": "done", "masterRank": 3, "level": 60 },
                { "cardId": 11, "defaultImage": "original", "specialTrainingStatus": "done", "masterRank": 1, "level": 55 }
            ],
            "userDeck": { "leader": 10, "member1": 10, "member2": 11 },
            "userProfileHonors": [
                { "honorId": 20, "honorLevel": 4, "seq": 2, "profileHonorType": "normal" },
                { "honorId": 21, "honorLevel": 2, "seq": 3, "profileHonorType": "bonds", "bondsHonorWordId": 99, "bondsHonorViewType": "reverse" }
            ],
            "userCharacters": [{ "characterId": 2, "characterRank": 31 }],
            "userChallengeLiveSoloStages": [{ "characterId": 2, "rank": 9 }],
            "userStoryFavorites": [
                { "storyId": 30, "storyType": "event_story", "shareNo": 2 },
                { "storyId": 31, "storyType": "card_story", "shareNo": 1 },
                { "storyId": 32, "storyType": "unit_story", "shareNo": 0 }
            ],
            "userHonorMissions": [{ "honorMissionType": "live_master", "progress": 50 }],
            "userHonors": [[20, 4, null]],
            "userBondsHonors": [{ "bondsHonorId": 21, "level": 2 }],
            "userMusicDifficultyClearCount": [
                { "musicDifficultyType": "expert", "liveClear": 40, "fullCombo": 30, "allPerfect": 20 },
                { "musicDifficultyType": "append", "liveClear": 4, "fullCombo": 3, "allPerfect": 2 }
            ]
        });
        let production = ProfileData::from_json(&raw).to_core_profile();
        let shared = sekai_profile_core::profile_data::ProfileData::from_json(&raw);
        assert_eq!(production, shared);
    }

    #[test]
    fn production_and_shared_profile_parsers_agree_on_out_of_range_values() {
        let raw = serde_json::json!({
            "userCards": [
                { "cardId": 0, "defaultImage": "special_training", "level": 10 },
                { "cardId": 5, "level": 40 }
            ],
            "userDeck": { "leader": 0, "member1": 5, "member2": 0 },
            "userProfileHonors": [{ "honorId": 3, "honorLevel": 0, "seq": 1 }],
            "userCharacters": [
                { "characterId": 3, "characterRank": 5 },
                { "characterId": 1, "characterRank": 9 },
                { "characterId": 3, "characterRank": 7 },
                { "characterId": 0, "characterRank": 2 }
            ],
            "userChallengeLiveSoloStages": [{ "characterId": 0, "rank": 4 }],
            "userHonorMissions": [{ "honorMissionType": "live_master" }],
            "userMusicDifficultyClearCount": [
                { "musicDifficultyType": "hard", "liveClear": 1 },
                { "musicDifficultyType": "hard", "liveClear": 2 }
            ]
        });
        let production = ProfileData::from_json(&raw).to_core_profile();
        let shared = sekai_profile_core::profile_data::ProfileData::from_json(&raw);
        assert_eq!(production, shared);
    }
}
