//! 玩家 Profile 数据模型（跨渲染管线共享）
//!
//! `ProfileData` 是从游戏 Profile API 响应中提取的玩家信息，
//! 被 Path A（基础名片）和 Path B（自定义名片 generals 面板）共同使用。
//!
//! ## 设计原则
//! - 纯数据结构，不依赖 skia 或任何渲染逻辑
//! - JSON → struct 转换逻辑只有这一份（`ProfileData::from_json`）
//! - Honor level 注入逻辑只有这一份（`build_honor_maps`）

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
    /// 是否为全尺寸（第一个槽位 = true → 380×80，后两个 = false → 180×80）
    pub full_size: bool,
    /// 称号类型（"normal" 或 "bonds"）
    pub profile_honor_type: String,
    /// bonds 称号的文字 ID（仅 bonds 类型有效）
    pub bonds_honor_word_id: Option<i64>,
    /// bonds 称号查看类型（"normal" 或 "reverse"），决定角色左右顺序
    pub bonds_honor_view_type: Option<String>,
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
    /// 最喜欢的剧情（type=14 最喜欢的剧情面板使用）
    pub story_favorites: Vec<StoryFavoriteInfo>,
    /// 打歌 honor 完成进度映射：honorMissionType → progress（live_master 称号使用）
    pub user_honor_missions: HashMap<String, i32>,
    /// 玩家拥有卡牌的运行时状态索引。
    pub user_cards: HashMap<i32, UserCardInfo>,
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
                master_rank: 2,
                level: 60,
            },
            DeckMember {
                card_id: 7,
                after_training: false,
                master_rank: 3,
                level: 60,
            },
            DeckMember {
                card_id: 11,
                after_training: false,
                master_rank: 4,
                level: 60,
            },
            DeckMember {
                card_id: 15,
                after_training: false,
                master_rank: 0,
                level: 60,
            },
            DeckMember {
                card_id: 4,
                after_training: false,
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
        story_favorites: vec![
            StoryFavoriteInfo {
                story_id: 1,
                story_type: "unit".to_string(),
            },
            StoryFavoriteInfo {
                story_id: 2,
                story_type: "unit".to_string(),
            },
            StoryFavoriteInfo {
                story_id: 3,
                story_type: "unit".to_string(),
            },
            StoryFavoriteInfo {
                story_id: 4,
                story_type: "unit".to_string(),
            },
        ],
        user_honor_missions,
        user_cards,
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
// Honor Level Maps 构建（唯一一份）
// ============================================================

/// 从 profile JSON 提取 honor level 映射表
///
/// 返回 (honor_map, bonds_honor_map, char_rank_map)
pub fn build_honor_maps(
    body: &serde_json::Value,
) -> (HashMap<i32, i32>, HashMap<i32, i32>, HashMap<i32, i32>) {
    let mut honor_map = HashMap::new();
    if let Some(arr) = body.get("userHonors").and_then(|v| v.as_array()) {
        for h in arr {
            if let Some(inner) = h.as_array() {
                if inner.len() >= 2 {
                    let id = inner[0].as_i64().unwrap_or(0) as i32;
                    let lvl = inner[1].as_i64().unwrap_or(1) as i32;
                    honor_map.insert(id, lvl);
                }
            }
        }
    }

    let mut bh_map = HashMap::new();
    if let Some(arr) = body.get("userBondsHonors").and_then(|v| v.as_array()) {
        for b in arr {
            if let (Some(id), Some(lvl)) = (
                b.get("bondsHonorId").and_then(|x| x.as_i64()),
                b.get("level").and_then(|x| x.as_i64()),
            ) {
                bh_map.insert(id as i32, lvl as i32);
            }
        }
    }

    // char_rank_map 从 ProfileData.char_ranks 构建更合适，
    // 但 profile JSON 中也有 userCharacters，这里直接提取
    let mut char_map = HashMap::new();
    if let Some(chars) = body.get("userCharacters").and_then(|v| v.as_array()) {
        for c in chars {
            if let (Some(cid), Some(rank)) = (
                c.get("characterId").and_then(|v| v.as_i64()),
                c.get("characterRank").and_then(|v| v.as_i64()),
            ) {
                char_map.insert(cid as i32, rank as i32);
            }
        }
    }

    (honor_map, bh_map, char_map)
}

// ============================================================
// ProfileData 构建（唯一一份 JSON → struct 逻辑）
// ============================================================

impl ProfileData {
    /// 从 profile API 响应 JSON 构建 ProfileData
    ///
    /// 提取所有 generals 面板所需数据。这是唯一的 JSON → struct 转换入口。
    pub fn from_json(body: &serde_json::Value) -> Self {
        let mut pd = Self::default();

        // 基础字段
        if let Some(u) = body.get("user") {
            pd.user_name = u.get("name").and_then(|v| v.as_str()).unwrap_or("").into();
            pd.user_rank = u.get("rank").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        }
        if let Some(p) = body.get("userProfile") {
            pd.word = p.get("word").and_then(|v| v.as_str()).unwrap_or("").into();
        }
        if let Some(tp) = body.get("totalPower") {
            pd.total_power = tp.get("totalPower").and_then(|v| v.as_i64()).unwrap_or(0);
        }
        if let Some(m) = body.get("userMultiLiveTopScoreCount") {
            pd.mvp = m.get("mvp").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            pd.superstar = m.get("superStar").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        }
        if let Some(ch) = body.get("userChallengeLiveSoloResult") {
            pd.challenge_score = ch.get("highScore").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            pd.challenge_character_id =
                ch.get("characterId").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        }

        // char_ranks（type=11/15）
        if let Some(chars) = body.get("userCharacters").and_then(|v| v.as_array()) {
            pd.char_ranks = chars
                .iter()
                .filter_map(|c| {
                    Some(CharacterRankInfo {
                        character_id: c.get("characterId")?.as_i64()? as i32,
                        rank: c.get("characterRank")?.as_i64()? as i32,
                    })
                })
                .collect();
        }

        // music_results（type=12/16）
        if let Some(mdc) = body
            .get("userMusicDifficultyClearCount")
            .and_then(|v| v.as_array())
        {
            let mut mr = MusicResults::default();
            for item in mdc {
                let diff = item
                    .get("musicDifficultyType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let stats = MusicDifficultyStats {
                    clear: item.get("liveClear").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                    full_combo: item.get("fullCombo").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                    all_perfect: item.get("allPerfect").and_then(|v| v.as_i64()).unwrap_or(0)
                        as i32,
                };
                match diff {
                    "easy" => mr.easy = stats,
                    "normal" => mr.normal = stats,
                    "hard" => mr.hard = stats,
                    "expert" => mr.expert = stats,
                    "master" => mr.master = stats,
                    "append" => mr.append = stats,
                    _ => {}
                }
            }
            pd.music_results = Some(mr);
        }

        // userCards 索引：cardId → (after_training, master_rank)
        let mut ucm: HashMap<i32, UserCardInfo> = HashMap::new();
        if let Some(uc) = body.get("userCards").and_then(|v| v.as_array()) {
            for c in uc {
                if let Some(cid) = c.get("cardId").and_then(|v| v.as_i64()) {
                    let di = c
                        .get("defaultImage")
                        .and_then(|v| v.as_str())
                        .unwrap_or("normal");
                    let mr = c.get("masterRank").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                    let level = c.get("level").and_then(|v| v.as_i64()).unwrap_or(60) as i32;
                    ucm.insert(
                        cid as i32,
                        UserCardInfo {
                            after_training: di == "special_training",
                            master_rank: mr,
                            level,
                        },
                    );
                }
            }
        }
        pd.user_cards = ucm.clone();

        // deck_members + leader_card（type=3/5）
        if let Some(deck) = body.get("userDeck") {
            for i in 1..=5 {
                let key = format!("member{i}");
                if let Some(cid) = deck.get(&key).and_then(|v| v.as_i64()) {
                    let info = ucm.get(&(cid as i32)).copied().unwrap_or(UserCardInfo {
                        after_training: false,
                        master_rank: 0,
                        level: 60,
                    });
                    pd.deck_members.push(DeckMember {
                        card_id: cid as i32,
                        after_training: info.after_training,
                        master_rank: info.master_rank,
                        level: info.level,
                    });
                }
            }
            if let Some(lid) = deck.get("leader").and_then(|v| v.as_i64()) {
                let info = ucm.get(&(lid as i32)).copied().unwrap_or(UserCardInfo {
                    after_training: false,
                    master_rank: 0,
                    level: 60,
                });
                pd.leader_card = Some(LeaderCardInfo {
                    card_id: lid as i32,
                    after_training: info.after_training,
                    master_rank: info.master_rank,
                });
            }
        }

        // honor_slots（type=6）
        if let Some(ph) = body.get("userProfileHonors").and_then(|v| v.as_array()) {
            for h in ph {
                pd.honor_slots.push(HonorSlot {
                    honor_id: h.get("honorId").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                    honor_level: h.get("honorLevel").and_then(|v| v.as_i64()).unwrap_or(1) as i32,
                    full_size: h.get("seq").and_then(|v| v.as_i64()).unwrap_or(0) == 2,
                    profile_honor_type: h
                        .get("profileHonorType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("normal")
                        .into(),
                    bonds_honor_word_id: h.get("bondsHonorWordId").and_then(|v| v.as_i64()),
                    bonds_honor_view_type: h
                        .get("bondsHonorViewType")
                        .and_then(|v| v.as_str())
                        .map(|s| s.into()),
                });
            }
        }

        // story_favorites（type=14）
        if let Some(sf) = body.get("userStoryFavorites").and_then(|v| v.as_array()) {
            for s in sf {
                pd.story_favorites.push(StoryFavoriteInfo {
                    story_id: s.get("storyId").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                    story_type: s
                        .get("storyType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .into(),
                });
            }
        }

        // user_honor_missions（live_master 称号进度）
        if let Some(hm) = body.get("userHonorMissions").and_then(|v| v.as_array()) {
            for m in hm {
                if let (Some(mt), Some(prog)) = (
                    m.get("honorMissionType").and_then(|v| v.as_str()),
                    m.get("progress").and_then(|v| v.as_i64()),
                ) {
                    pd.user_honor_missions.insert(mt.to_string(), prog as i32);
                }
            }
        }

        pd
    }

    /// 查询指定卡牌的玩家状态信息。
    pub fn user_card(&self, card_id: i32) -> Option<&UserCardInfo> {
        self.user_cards.get(&card_id)
    }
}
