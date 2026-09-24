//! 玩家 Profile 数据模型（跨渲染管线共享）
//!
//! `ProfileData` 是从游戏 Profile API 响应中提取的玩家信息，
//! 被 Path A（基础名片）和 Path B（自定义名片 generals 面板）共同使用。
//!
//! ## 设计原则
//! - 纯数据结构，不依赖 skia 或任何渲染逻辑
//! - JSON 解析规则只有 core 的
//!   [`sekai_profile_renderer_core::profile_data::ProfileData::from_json`] 一份，
//!   `ProfileData::from_json` 只做类型转换
//! - 名片上称号的等级与持有判定由 core 的
//!   [`sekai_profile_renderer_core::profile_data::placed_honor_level`] 决定

use std::collections::HashMap;

// ============================================================
// 子类型定义
// ============================================================

/// 称号槽位数据（General 面板 type=6 使用）
#[derive(Debug, Clone, Default)]
pub struct HonorSlot {
    /// 称号 ID（MasterData honors 或 bondsHonors 表主键）
    pub honor_id: i32,
    /// 称号等级（决定星星数量）
    pub honor_level: i32,
    /// 展示偏好标志（游戏 seq==2 的槽位为 true）。名片 semantic 路径把本
    /// 字段当排序权重（大槽优先），排序后交换前两个槽位，最终渲染尺寸由
    /// 交换后的槽位位置决定（第 0 槽 380×80 main，其余 180×80 sub）。
    pub full_size: bool,
    /// 称号类型（"normal" 或 "bonds"）
    pub profile_honor_type: String,
    /// bonds 称号的文字 ID（仅 bonds 类型有效）
    pub bonds_honor_word_id: Option<i64>,
    /// bonds 称号查看类型（"normal" 或 "reverse"），决定角色左右顺序
    pub bonds_honor_view_type: Option<String>,
}

impl HonorSlot {
    pub fn bonds_honor_view_flags(&self) -> (bool, bool) {
        sekai_profile_renderer_core::profile_data::bonds_honor_view_flags(
            self.bonds_honor_view_type.as_deref(),
        )
    }
}

/// 队长卡面数据（General 面板 type=5 使用）
#[derive(Debug, Clone, Default)]
pub struct LeaderCardInfo {
    /// 卡牌 ID
    pub card_id: i32,
    /// 是否使用特训后图片
    pub after_training: bool,
    /// 突破等级 (0-5)
    pub master_rank: i32,
}

/// 玩家卡牌状态信息。
#[derive(Debug, Clone, Copy, Default)]
pub struct UserCardInfo {
    /// 是否使用特训后图片。
    pub after_training: bool,
    /// 是否已完成特训（`specialTrainingStatus` 为 `done`），决定稀有度星标。
    pub special_training_done: bool,
    /// 突破等级。
    pub master_rank: i32,
    /// 当前卡牌等级。
    pub level: i32,
}

/// 卡组成员（General 面板 type=3 主要组合使用）
#[derive(Debug, Clone, Default)]
pub struct DeckMember {
    /// 卡牌 ID
    pub card_id: i32,
    /// 是否使用特训后图片
    pub after_training: bool,
    /// 是否已完成特训，决定稀有度星标
    pub special_training_done: bool,
    /// 突破等级 (0-5)
    pub master_rank: i32,
    /// 当前卡牌等级
    pub level: i32,
}

/// 单难度歌曲统计（General 面板 type=12/16 使用）
#[derive(Debug, Clone, Default)]
pub struct MusicDifficultyStats {
    /// 已完成曲数
    pub clear: i32,
    /// Full Combo 曲数
    pub full_combo: i32,
    /// All Perfect 曲数
    pub all_perfect: i32,
}

/// 歌曲统计汇总（6 个难度）
#[derive(Debug, Clone, Default)]
pub struct MusicResults {
    pub easy: MusicDifficultyStats,
    pub normal: MusicDifficultyStats,
    pub hard: MusicDifficultyStats,
    pub expert: MusicDifficultyStats,
    pub master: MusicDifficultyStats,
    pub append: MusicDifficultyStats,
}

/// 角色等级信息（General 面板 type=11 使用）
#[derive(Debug, Clone, Default)]
pub struct CharacterRankInfo {
    /// 角色 ID
    pub character_id: i32,
    /// 收藏等级
    pub rank: i32,
}

/// 剧情收藏信息（General 面板 type=14 使用）
#[derive(Debug, Clone, Default)]
pub struct StoryFavoriteInfo {
    /// 剧情 ID（用于查找封面图）
    pub story_id: i32,
    /// 剧情类型（用于构建素材路径）
    pub story_type: String,
    /// 面板格位（`shareNo`，从 1 开始）
    pub share_no: i32,
}

// ============================================================
// ProfileData 主结构体
// ============================================================

/// 玩家 Profile 数据（不依赖 skia，跨管线共享）
#[derive(Debug, Clone, Default)]
pub struct ProfileData {
    /// 玩家名称
    pub user_name: String,
    /// 玩家 Rank。
    pub user_rank: i32,
    /// 综合力
    pub total_power: i64,
    /// 个性签名
    pub word: String,
    /// MVP 次数
    pub mvp: i32,
    /// SUPERSTAR 次数
    pub superstar: i32,
    /// 挑战演出分数
    pub challenge_score: i32,
    /// 挑战演出角色 ID（type=10 头像使用）
    pub challenge_character_id: i32,
    /// 队长卡面信息（type=5 队长成员面板使用）
    pub leader_card: Option<LeaderCardInfo>,
    /// 称号槽位（type=6 称号面板使用，最多 3 个）
    pub honor_slots: Vec<HonorSlot>,
    /// 卡组成员（type=3 主要组合面板使用，5 个成员）
    pub deck_members: Vec<DeckMember>,
    /// 歌曲统计（type=12/16 歌曲信息面板使用）
    pub music_results: Option<MusicResults>,
    /// 角色等级（type=11 角色收藏等级面板使用）
    pub char_ranks: Vec<CharacterRankInfo>,
    /// 各角色挑战演出等级（General type=11/15 第二个 tab）。
    pub challenge_ranks: Vec<CharacterRankInfo>,
    /// 最喜欢的剧情（type=14 最喜欢的剧情面板使用）
    pub story_favorites: Vec<StoryFavoriteInfo>,
    /// 打歌 honor 完成进度映射：honorMissionType → progress（live_master 称号使用）
    pub user_honor_missions: HashMap<String, i32>,
    /// 玩家拥有卡牌的运行时状态索引。
    pub user_cards: HashMap<i32, UserCardInfo>,
    /// 玩家持有的称号与羁绊称号等级（userHonors / userBondsHonors）。
    pub owned_honors: sekai_profile_renderer_core::profile_data::OwnedHonorLevels,
}

/// 中性预览用 Profile 数据。
///
/// 这份数据只用于组件目录缩略图和模板本地预览，文本保持通用客观，不表达真实玩家身份。
pub fn neutral_preview_profile() -> ProfileData {
    let mut user_cards = HashMap::new();
    let deck_card_ids = [3, 7, 11, 15, 4]; // 不同角色、不同稀有度、S3 资源确认存在
    for (i, &card_id) in deck_card_ids.iter().enumerate() {
        user_cards.insert(
            card_id,
            UserCardInfo {
                after_training: false, // 只用 normal 确保 S3 一定有
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
// ProfileData 构建（JSON 解析规则只在 core 一份）
// ============================================================

impl ProfileData {
    /// 从 profile API 响应 JSON 构建 ProfileData
    ///
    /// 提取所有 generals 面板所需数据。解析规则只有
    /// [`sekai_profile_renderer_core::profile_data::ProfileData::from_json`] 一份，
    /// 浏览器与 native 后端对同一响应得到同一份数据。
    pub fn from_json(body: &serde_json::Value) -> Self {
        Self::from_core(sekai_profile_renderer_core::profile_data::ProfileData::from_json(body))
    }

    fn from_core(value: sekai_profile_renderer_core::profile_data::ProfileData) -> Self {
        let character_rank =
            |rank: sekai_profile_renderer_core::profile_data::CharacterRank| CharacterRankInfo {
                character_id: rank.character_id,
                rank: rank.rank,
            };
        let music_stats =
            |stats: sekai_profile_renderer_core::profile_data::MusicDifficultyStats| {
                MusicDifficultyStats {
                    clear: stats.clear,
                    full_combo: stats.full_combo,
                    all_perfect: stats.all_perfect,
                }
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

    /// 查询指定卡牌的玩家状态信息。
    pub fn user_card(&self, card_id: i32) -> Option<&UserCardInfo> {
        self.user_cards.get(&card_id)
    }

    /// Converts the production profile model back into the backend-neutral
    /// model it was parsed from.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn to_core_profile(&self) -> sekai_profile_renderer_core::profile_data::ProfileData {
        use sekai_profile_renderer_core::profile_data as core;

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
) -> sekai_profile_renderer_core::profile_data::MusicDifficultyStats {
    sekai_profile_renderer_core::profile_data::MusicDifficultyStats {
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
        let shared = sekai_profile_renderer_core::profile_data::ProfileData::from_json(&raw);
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
        let shared = sekai_profile_renderer_core::profile_data::ProfileData::from_json(&raw);
        assert_eq!(production, shared);
    }
}
