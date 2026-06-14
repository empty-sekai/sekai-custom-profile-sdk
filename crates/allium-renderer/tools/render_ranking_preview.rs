use std::fs;
use std::path::PathBuf;

use allium_renderer::assets::AssetStore;
use allium_renderer::ranking::{
    render_ranking, RankingBorderGap, RankingBorderGaps, RankingBorderRecord, RankingEventMeta,
    RankingHourlyPlayCount, RankingLiveBoardInput, RankingNeighborGap, RankingNeighborGaps,
    RankingPlayerBatchInput, RankingPlayerCard, RankingPlayerDetailInput, RankingPlayerMetrics,
    RankingPlayerRecord, RankingRenderInput,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("results/ranking_design"));
    fs::create_dir_all(&out_dir)?;

    let assets = AssetStore::new(16);
    let snapshot_ts = 1_777_632_000;
    let borders = vec![
        border(1, 10_889_500, 520_000),
        border(10, 10_345_000, 480_000),
        border(20, 9_740_000, 430_000),
        border(30, 9_135_000, 385_000),
        border(40, 8_530_000, 350_000),
        border(50, 7_925_000, 310_000),
        border(100, 4_900_000, 190_000),
        border(200, 3_100_000, 130_000),
        border(300, 2_620_000, 95_000),
        border(400, 2_220_000, 72_000),
        border(500, 1_800_000, 48_000),
        border(1000, 1_180_000, 32_000),
        border(1500, 930_000, 25_000),
        border(2000, 780_000, 21_000),
        border(2500, 690_000, 18_000),
        border(3000, 620_000, 15_000),
        border(4000, 520_000, 13_000),
        border(5000, 450_000, 11_000),
        border(10000, 310_000, 7_000),
        border(20000, 210_000, 4_400),
        border(30000, 170_000, 3_200),
        border(40000, 138_000, 2_600),
        border(50000, 112_000, 2_100),
        border(100000, 54_000, 900),
        border(200000, 12_000, 260),
    ];

    for obsolete in [
        "01_live_page.jpeg",
        "02_top100_compact.jpeg",
        "03_borders.jpeg",
        "04_player_detail.jpeg",
        "05_player_batch.jpeg",
    ] {
        let _ = fs::remove_file(out_dir.join(obsolete));
    }

    write(
        &assets,
        &out_dir,
        "01_top100.jpeg",
        RankingRenderInput::LiveBoard(RankingLiveBoardInput {
            event_id: 133,
            event: preview_event(),
            character_id: 0,
            snapshot_seq: 42,
            last_snapshot_ts: Some(snapshot_ts),
            phase: "采集中".to_string(),
            page: 1,
            page_size: 100,
            total: 100,
            compact_top100: true,
            players: (1..=100)
                .map(|rank| {
                    player(
                        rank,
                        744_508_491_012_320_000 + i64::from(rank),
                        "紧凑榜单玩家",
                    )
                })
                .collect(),
            borders: borders.clone(),
        }),
    )?;

    write(
        &assets,
        &out_dir,
        "02_borders.jpeg",
        RankingRenderInput::BorderSummary(allium_renderer::ranking::RankingBorderSummaryInput {
            event_id: 133,
            event: preview_event(),
            character_id: 0,
            snapshot_seq: 42,
            last_snapshot_ts: Some(snapshot_ts),
            phase: "采集中".to_string(),
            borders: borders.clone(),
        }),
    )?;

    write(
        &assets,
        &out_dir,
        "03_player_detail.jpeg",
        RankingRenderInput::PlayerDetail(RankingPlayerDetailInput {
            event_id: 133,
            event: preview_event(),
            character_id: 0,
            snapshot_seq: 42,
            last_snapshot_ts: Some(snapshot_ts),
            phase: "采集中".to_string(),
            player: card(player(17, 744_508_491_012_319_519, "犬犬")),
        }),
    )?;

    write(
        &assets,
        &out_dir,
        "04_player_batch.jpeg",
        RankingRenderInput::PlayerBatch(RankingPlayerBatchInput {
            event_id: 133,
            event: preview_event(),
            character_id: 0,
            snapshot_seq: 42,
            last_snapshot_ts: Some(snapshot_ts),
            phase: "采集中".to_string(),
            players: vec![
                card(player(17, 744_508_491_012_319_519, "犬犬")),
                card(player(18, 744_508_491_012_319_520, "薄暮之后")),
                card(player(19, 744_508_491_012_319_521, "窗边练习")),
                card(player(20, 744_508_491_012_319_522, "第25时")),
                card(player(21, 744_508_491_012_319_523, "空白日记")),
            ],
        }),
    )?;

    println!("{}", out_dir.display());
    Ok(())
}

fn write(
    assets: &AssetStore,
    out_dir: &PathBuf,
    filename: &str,
    input: RankingRenderInput,
) -> Result<(), Box<dyn std::error::Error>> {
    let output = render_ranking(&input, assets)?;
    fs::write(out_dir.join(filename), output.data)?;
    println!("{} {}x{}", filename, output.width, output.height);
    Ok(())
}

fn preview_event() -> RankingEventMeta {
    RankingEventMeta {
        name: Some("荆棘中的独白".to_string()),
        remaining_secs: Some(2 * 86_400 + 3 * 3_600 + 12 * 60 + 35),
    }
}

fn player(rank: i32, user_id: i64, name: &str) -> RankingPlayerRecord {
    RankingPlayerRecord {
        user_id,
        character_id: 0,
        rank,
        score: 10_950_000 - i64::from(rank) * 60_500,
        name: name.to_string(),
        score_delta: 80_000 - i64::from(rank) * 750,
        rank_delta: if rank % 5 == 0 { 1 } else { -1 },
        hourly_rate: 420_000.0 - f64::from(rank) * 1_800.0,
        tracking_window_secs: 3_600,
        parked: rank % 7 == 0,
        rank_delta_1h: if rank % 9 == 0 { 2 } else { -3 },
        rank_delta_1h_estimated: rank > 80,
        last_active_ts: 1_777_632_000 - i64::from(rank) * 45,
    }
}

fn border(rank: i32, score: i64, hourly_rate: i64) -> RankingBorderRecord {
    RankingBorderRecord {
        rank,
        score,
        character_id: 0,
        score_delta: hourly_rate / 12,
        hourly_rate: hourly_rate as f64,
        tracking_window_secs: 3_600,
    }
}

fn card(record: RankingPlayerRecord) -> RankingPlayerCard {
    let rank = record.rank;
    let score = record.score;
    let previous_score = record.score + 22_000;
    let next_score = record.score - 18_000;
    let (upper_rank, upper_score, lower_rank, lower_score) = if rank < 20 {
        (10, 10_345_000, 20, 9_740_000)
    } else if rank == 20 {
        (10, 10_345_000, 30, 9_135_000)
    } else {
        (20, 9_740_000, 30, 9_135_000)
    };
    RankingPlayerCard {
        record,
        metrics: RankingPlayerMetrics {
            average_recent_pt: 12_450.0,
            hourly_rate: 362_000.0,
            tracking_window_secs: 3_600,
            projected_20m_x3: 381_000.0,
            plays_this_hour: 18,
            hourly_plays: hourly_plays(),
            consecutive_play_secs: 5_420,
            parked: false,
            rank_delta_1h: -3,
            rank_delta_1h_estimated: false,
        },
        border_gaps: RankingBorderGaps {
            upper: Some(RankingBorderGap {
                rank: upper_rank,
                score: upper_score,
                gap: (upper_score - score).max(0),
            }),
            lower: Some(RankingBorderGap {
                rank: lower_rank,
                score: lower_score,
                gap: (score - lower_score).max(0),
            }),
        },
        neighbor_gaps: RankingNeighborGaps {
            previous: Some(RankingNeighborGap {
                rank: rank - 1,
                name: "星空下的测试玩家".to_string(),
                score: previous_score,
                gap: 22_000,
            }),
            next: Some(RankingNeighborGap {
                rank: rank + 1,
                name: "静かな夜".to_string(),
                score: next_score,
                gap: 18_000,
            }),
        },
    }
}

fn hourly_plays() -> Vec<RankingHourlyPlayCount> {
    (0..10)
        .map(|index| RankingHourlyPlayCount {
            hour_start_ts: 1_777_599_600 + i64::from(index) * 3_600,
            plays: [0, 2, 4, 5, 3, 0, 6, 8, 7, 4][index as usize],
        })
        .collect()
}
