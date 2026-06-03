//! 排行榜图片渲染。

#![cfg_attr(not(feature = "skia"), allow(dead_code))]

use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};

use crate::assets::AssetStore;
use crate::error::RenderError;
use crate::traits::RenderOutput;

const OUTPUT_QUALITY: u8 = 85;
const BOARD_WIDTH: u32 = 907;
const PANEL_RADIUS: f32 = 8.0;
const DETAIL_WIDTH: u32 = 1040;
const LIVE_BOARD_NOTE_HEIGHT: f32 = 106.0;
const PLAYER_DETAIL_CARD_HEIGHT: f32 = 492.0;
const PLAYER_HOURLY_PANEL_HEIGHT: f32 = 132.0;
const MIN_STABLE_RATE_WINDOW_SECONDS: i64 = 10 * 60;

/// 排行榜图片输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RankingRenderInput {
    /// 实时榜单页或紧凑 top100。
    LiveBoard(RankingLiveBoardInput),
    /// 榜线摘要。
    BorderSummary(RankingBorderSummaryInput),
    /// 单玩家详情。
    PlayerDetail(RankingPlayerDetailInput),
    /// 多玩家详情。
    PlayerBatch(RankingPlayerBatchInput),
    /// 单玩家时速卡片。
    SpeedDetail(RankingSpeedDetailInput),
    /// 所有榜线时速汇总。
    SpeedBorders(RankingSpeedBordersInput),
    /// 多目标时速对比图。
    SpeedCompare(RankingSpeedCompareInput),
    /// 单玩家日速卡片。
    DailyDetail(RankingDailyDetailInput),
    /// 所有榜线日速汇总。
    DailyBorders(RankingDailyBordersInput),
    /// 多目标日速对比图。
    DailyCompare(RankingDailyCompareInput),
}

/// 排行榜活动元信息。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RankingEventMeta {
    pub name: Option<String>,
    pub remaining_secs: Option<i64>,
}

/// 排行榜单行玩家。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingPlayerRecord {
    pub user_id: i64,
    pub character_id: i16,
    pub rank: i32,
    pub score: i64,
    pub name: String,
    pub score_delta: i64,
    pub rank_delta: i32,
    pub hourly_rate: f64,
    #[serde(default)]
    pub tracking_window_secs: i64,
    pub parked: bool,
    pub rank_delta_1h: i32,
    pub rank_delta_1h_estimated: bool,
    pub last_active_ts: i64,
}

/// 排行榜单条榜线。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingBorderRecord {
    pub rank: i32,
    pub score: i64,
    pub character_id: i16,
    pub score_delta: i64,
    pub hourly_rate: f64,
    #[serde(default)]
    pub tracking_window_secs: i64,
}

/// 玩家追踪指标。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingPlayerMetrics {
    pub average_recent_pt: f64,
    pub hourly_rate: f64,
    #[serde(default)]
    pub tracking_window_secs: i64,
    pub projected_20m_x3: f64,
    pub plays_this_hour: u32,
    pub hourly_plays: Vec<RankingHourlyPlayCount>,
    pub consecutive_play_secs: i64,
    pub parked: bool,
    pub rank_delta_1h: i32,
    pub rank_delta_1h_estimated: bool,
}

/// 玩家每小时周回数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingHourlyPlayCount {
    pub hour_start_ts: i64,
    pub plays: u32,
}

/// 玩家与可查榜线的差距。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingBorderGap {
    pub rank: i32,
    pub score: i64,
    pub gap: i64,
}

/// 玩家前后可查榜线差距。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RankingBorderGaps {
    pub upper: Option<RankingBorderGap>,
    pub lower: Option<RankingBorderGap>,
}

/// 玩家与相邻玩家的差距。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingNeighborGap {
    pub rank: i32,
    pub name: String,
    pub score: i64,
    pub gap: i64,
}

/// 玩家前后相邻排名差距。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RankingNeighborGaps {
    pub previous: Option<RankingNeighborGap>,
    pub next: Option<RankingNeighborGap>,
}

/// 玩家详情卡片。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingPlayerCard {
    pub record: RankingPlayerRecord,
    pub metrics: RankingPlayerMetrics,
    pub border_gaps: RankingBorderGaps,
    pub neighbor_gaps: RankingNeighborGaps,
}

/// 实时榜单图输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingLiveBoardInput {
    pub event_id: i32,
    #[serde(default)]
    pub event: RankingEventMeta,
    pub character_id: i16,
    pub snapshot_seq: i32,
    pub last_snapshot_ts: Option<i64>,
    pub phase: String,
    pub page: usize,
    pub page_size: usize,
    pub total: usize,
    pub compact_top100: bool,
    pub players: Vec<RankingPlayerRecord>,
    pub borders: Vec<RankingBorderRecord>,
}

/// 榜线摘要图输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingBorderSummaryInput {
    pub event_id: i32,
    #[serde(default)]
    pub event: RankingEventMeta,
    pub character_id: i16,
    pub snapshot_seq: i32,
    pub last_snapshot_ts: Option<i64>,
    pub phase: String,
    pub borders: Vec<RankingBorderRecord>,
}

/// 单玩家详情图输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingPlayerDetailInput {
    pub event_id: i32,
    #[serde(default)]
    pub event: RankingEventMeta,
    pub character_id: i16,
    pub snapshot_seq: i32,
    pub last_snapshot_ts: Option<i64>,
    pub phase: String,
    pub player: RankingPlayerCard,
}

/// 多玩家详情图输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingPlayerBatchInput {
    pub event_id: i32,
    #[serde(default)]
    pub event: RankingEventMeta,
    pub character_id: i16,
    pub snapshot_seq: i32,
    pub last_snapshot_ts: Option<i64>,
    pub phase: String,
    pub players: Vec<RankingPlayerCard>,
}

// ============= 时速 =============

/// 单玩家时速卡片输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingSpeedDetailInput {
    pub event_id: i32,
    #[serde(default)]
    pub event: RankingEventMeta,
    pub character_id: i16,
    pub snapshot_seq: i32,
    pub last_snapshot_ts: Option<i64>,
    pub phase: String,
    pub player: RankingSpeedCard,
}

/// 所有榜线时速汇总输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingSpeedBordersInput {
    pub event_id: i32,
    #[serde(default)]
    pub event: RankingEventMeta,
    pub character_id: i16,
    pub snapshot_seq: i32,
    pub last_snapshot_ts: Option<i64>,
    pub phase: String,
    pub borders: Vec<RankingSpeedBorderCard>,
}

/// 多目标时速对比图输入（水平条状图）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingSpeedCompareInput {
    pub event_id: i32,
    #[serde(default)]
    pub event: RankingEventMeta,
    pub character_id: i16,
    pub snapshot_seq: i32,
    pub last_snapshot_ts: Option<i64>,
    pub phase: String,
    /// 对比对象列表。每条 = 一行。
    pub subjects: Vec<RankingSpeedCompareSubject>,
    /// 缺失的 user_ids（仅 user_ids 查询时有意义）。
    #[serde(default)]
    pub missing_user_ids: Vec<i64>,
}

/// 单个玩家的时速卡片（用于单人 / 榜线汇总的 card 区域）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingSpeedCard {
    pub user_id: i64,
    pub name: String,
    pub rank: i32,
    pub score: i64,
    pub hourly_rate: f64,
    pub tracking_window_secs: i64,
    pub parked: bool,
    pub plays_this_hour: u32,
    pub hourly_plays: Vec<RankingHourlyPlayCount>,
    pub rank_delta_1h: i32,
    pub rank_delta_1h_estimated: bool,
    pub projected_20m_x3: f64,
}

/// 榜线时速汇总中的单条榜线。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingSpeedBorderCard {
    pub rank: i32,
    pub score: i64,
    pub hourly_rate: f64,
    pub tracking_window_secs: i64,
    pub score_delta: i64,
    pub chapter_type: Option<String>,
}

/// 对比图中的单行对象（条状图一行）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingSpeedCompareSubject {
    pub user_id: i64,
    pub name: String,
    pub rank: i32,
    pub score: i64,
    pub hourly_rate: f64,
    pub tracking_window_secs: i64,
    pub parked: bool,
    pub rank_delta_1h: i32,
    pub rank_delta_1h_estimated: bool,
    pub projected_20m_x3: f64,
}

// ============= 日速 =============

/// 单玩家日速卡片输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingDailyDetailInput {
    pub event_id: i32,
    #[serde(default)]
    pub event: RankingEventMeta,
    pub character_id: i16,
    pub snapshot_seq: i32,
    pub last_snapshot_ts: Option<i64>,
    pub phase: String,
    pub player: RankingDailyCard,
}

/// 所有榜线日速汇总输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingDailyBordersInput {
    pub event_id: i32,
    #[serde(default)]
    pub event: RankingEventMeta,
    pub character_id: i16,
    pub snapshot_seq: i32,
    pub last_snapshot_ts: Option<i64>,
    pub phase: String,
    pub borders: Vec<RankingDailyBorderCard>,
}

/// 多目标日速对比图输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingDailyCompareInput {
    pub event_id: i32,
    #[serde(default)]
    pub event: RankingEventMeta,
    pub character_id: i16,
    pub snapshot_seq: i32,
    pub last_snapshot_ts: Option<i64>,
    pub phase: String,
    pub subjects: Vec<RankingDailyCompareSubject>,
    #[serde(default)]
    pub missing_user_ids: Vec<i64>,
}

/// 单玩家日速卡片。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingDailyCard {
    pub user_id: i64,
    pub name: String,
    pub rank: i32,
    pub score: i64,
    /// 日速（一天的速度，PT/day）
    pub daily_rate: f64,
    /// 日均时速（PT/h）
    pub daily_hourly_rate: f64,
    pub daily_tracking_window_secs: i64,
    pub rank_delta_24h: i32,
    pub rank_delta_24h_estimated: bool,
    pub daily_score_delta: i64,
    /// 日速是否为内存时速 fallback。
    pub is_fallback: bool,
    /// 时速对比（来自内存 tracker）
    pub hourly_rate: f64,
}

/// 榜线日速汇总中的单条榜线。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingDailyBorderCard {
    pub rank: i32,
    pub score: i64,
    /// 日速（一天的速度，PT/day）
    pub daily_rate: f64,
    /// 日均时速（PT/h）
    pub daily_hourly_rate: f64,
    pub daily_tracking_window_secs: i64,
    pub daily_score_delta: i64,
    /// 时速对比参考
    pub hourly_rate: f64,
}

/// 日速对比图中的单行对象。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingDailyCompareSubject {
    pub user_id: i64,
    pub name: String,
    pub rank: i32,
    pub score: i64,
    /// 日速（一天的速度，PT/day）
    pub daily_rate: f64,
    /// 日均时速（PT/h）
    pub daily_hourly_rate: f64,
    pub daily_tracking_window_secs: i64,
    pub rank_delta_24h: i32,
    pub rank_delta_24h_estimated: bool,
    pub daily_score_delta: i64,
    /// 日速是否为内存时速 fallback。
    pub is_fallback: bool,
    /// 时速对比参考
    pub hourly_rate: f64,
}

/// 渲染排行榜图片。
pub fn render_ranking(
    input: &RankingRenderInput,
    assets: &AssetStore,
) -> Result<RenderOutput, RenderError> {
    let _ = assets;
    match input {
        RankingRenderInput::LiveBoard(input) => render_live_board(input),
        RankingRenderInput::BorderSummary(input) => render_border_summary(input),
        RankingRenderInput::PlayerDetail(input) => render_player_detail(input),
        RankingRenderInput::PlayerBatch(input) => render_player_batch(input),
        RankingRenderInput::SpeedDetail(input) => render_speed_detail(input),
        RankingRenderInput::SpeedBorders(input) => render_speed_borders(input),
        RankingRenderInput::SpeedCompare(input) => render_speed_compare(input),
        RankingRenderInput::DailyDetail(input) => render_daily_detail(input),
        RankingRenderInput::DailyBorders(input) => render_daily_borders(input),
        RankingRenderInput::DailyCompare(input) => render_daily_compare(input),
    }
}

fn render_live_board(input: &RankingLiveBoardInput) -> Result<RenderOutput, RenderError> {
    let row_h = if input.compact_top100 { 31.0 } else { 47.0 };
    let header_h = if input.compact_top100 { 126.0 } else { 146.0 };
    let bottom_h = LIVE_BOARD_NOTE_HEIGHT;
    let player_rows = if input.compact_top100 {
        input.players.len().div_ceil(2)
    } else {
        input.players.len()
    };
    let height = (header_h + 34.0 + row_h * player_rows as f32 + bottom_h + 32.0 + FOOTER_HEIGHT as f32)
        .ceil()
        .max(360.0) as u32;

    #[cfg(feature = "skia")]
    {
        if input.compact_top100 {
            return render_top100_compact_skia(input, BOARD_WIDTH, height, row_h, header_h);
        }
        return render_live_board_skia(input, BOARD_WIDTH, height, row_h, header_h);
    }

    #[cfg(not(feature = "skia"))]
    Ok(placeholder_output(BOARD_WIDTH, height))
}

fn render_border_summary(input: &RankingBorderSummaryInput) -> Result<RenderOutput, RenderError> {
    let row_h = 42.0;
    let header_h = 146.0;
    let height = (header_h + 34.0 + row_h * input.borders.len() as f32 + 32.0 + FOOTER_HEIGHT as f32)
        .ceil()
        .max(330.0) as u32;

    #[cfg(feature = "skia")]
    {
        return render_border_summary_skia(input, BOARD_WIDTH, height, row_h, header_h);
    }

    #[cfg(not(feature = "skia"))]
    Ok(placeholder_output(BOARD_WIDTH, height))
}

fn render_player_detail(input: &RankingPlayerDetailInput) -> Result<RenderOutput, RenderError> {
    let _ = input;
    let width = DETAIL_WIDTH;
    let height = 704 + FOOTER_HEIGHT;

    #[cfg(feature = "skia")]
    {
        return render_player_detail_skia(input, width, height);
    }

    #[cfg(not(feature = "skia"))]
    Ok(placeholder_output(width, height))
}

fn render_player_batch(input: &RankingPlayerBatchInput) -> Result<RenderOutput, RenderError> {
    let width = DETAIL_WIDTH;
    let height = (106 + input.players.len() as u32 * 512 + 34).max(392);

    #[cfg(feature = "skia")]
    {
        return render_player_batch_skia(input, width, height);
    }

    #[cfg(not(feature = "skia"))]
    Ok(placeholder_output(width, height))
}

#[cfg(feature = "skia")]
fn render_top100_compact_skia(
    input: &RankingLiveBoardInput,
    width: u32,
    height: u32,
    row_h: f32,
    header_h: f32,
) -> Result<RenderOutput, RenderError> {
    let mut canvas = RankingCanvas::new(width, height)?;
    canvas.background();
    draw_snapshot_meta_lines(
        &mut canvas,
        input.event_id,
        &input.event,
        input.character_id,
        input.last_snapshot_ts,
        48.0,
        28.0,
        22.0,
        18.0,
        760.0,
    );
    canvas.text(
        "时速与名次按近1小时；不足1小时标约，未满10分钟统计中",
        width as f32 - 48.0,
        72.0,
        18.0,
        Color::MUTED,
        TextAlign::Right,
        500.0,
    );

    let panel_x = 40.0;
    let panel_y = 92.0;
    let panel_w = width as f32 - panel_x * 2.0;
    canvas.glass(panel_x, panel_y, panel_w, height as f32 - panel_y - 32.0 - FOOTER_HEIGHT as f32);

    let col_gap = 18.0;
    let col_w = (panel_w - 48.0 - col_gap) / 2.0;
    let left_x = panel_x + 18.0;
    let right_x = left_x + col_w + col_gap;
    let header_y = header_h - 8.0;
    draw_compact_columns(&mut canvas, left_x, header_y, col_w);
    draw_compact_columns(&mut canvas, right_x, header_y, col_w);

    let rows = input.players.len().div_ceil(2);
    for row in 0..rows {
        let y = header_h + 28.0 + row as f32 * row_h;
        if let Some(player) = input.players.get(row) {
            draw_compact_player_row(&mut canvas, player, left_x, y, col_w, row);
        }
        if let Some(player) = input.players.get(row + rows) {
            draw_compact_player_row(&mut canvas, player, right_x, y, col_w, row + rows);
        }
    }

    let note_y = header_h + 28.0 + rows as f32 * row_h + 10.0;
    draw_activity_note(&mut canvas, panel_x + 12.0, note_y, panel_w - 24.0);
    draw_footer(&mut canvas, width, height);
    canvas.finish(OUTPUT_QUALITY)
}

#[cfg(feature = "skia")]
fn render_live_board_skia(
    input: &RankingLiveBoardInput,
    width: u32,
    height: u32,
    row_h: f32,
    header_h: f32,
) -> Result<RenderOutput, RenderError> {
    let mut canvas = RankingCanvas::new(width, height)?;
    canvas.background();
    draw_snapshot_meta_lines(
        &mut canvas,
        input.event_id,
        &input.event,
        input.character_id,
        input.last_snapshot_ts,
        48.0,
        28.0,
        22.0,
        18.0,
        820.0,
    );
    let page_label = if input.compact_top100 {
        format!("前100 · {} 人", input.players.len())
    } else {
        format!(
            "第 {} 页 · {} / {} · 名次按1小时",
            input.page,
            input.players.len(),
            input.total
        )
    };
    canvas.text(
        &page_label,
        width as f32 - 48.0,
        72.0,
        18.0,
        Color::MUTED,
        TextAlign::Right,
        420.0,
    );

    let table_x = 40.0;
    let table_y = 92.0;
    let table_w = width as f32 - table_x * 2.0;
    canvas.glass(table_x, table_y, table_w, height as f32 - table_y - 32.0 - FOOTER_HEIGHT as f32);
    canvas.panel(
        table_x + 12.0,
        header_h - 33.0,
        table_w - 24.0,
        34.0,
        Color::PANEL_ALT,
    );
    draw_board_columns(
        &mut canvas,
        table_x + 24.0,
        header_h - 10.0,
        table_w - 48.0,
        input.compact_top100,
    );

    let mut y = header_h + 34.0;
    for (index, player) in input.players.iter().enumerate() {
        let bg = if index % 2 == 0 {
            Color::ROW
        } else {
            Color::ROW_ALT
        };
        canvas.panel(table_x + 12.0, y - 25.0, table_w - 24.0, row_h - 4.0, bg);
        draw_player_row(
            &mut canvas,
            player,
            table_x + 24.0,
            y,
            table_w - 48.0,
            input.compact_top100,
        );
        y += row_h;
    }

    draw_activity_note(&mut canvas, table_x + 12.0, y + 5.0, table_w - 24.0);
    draw_footer(&mut canvas, width, height);
    canvas.finish(OUTPUT_QUALITY)
}

#[cfg(feature = "skia")]
fn render_border_summary_skia(
    input: &RankingBorderSummaryInput,
    width: u32,
    height: u32,
    row_h: f32,
    header_h: f32,
) -> Result<RenderOutput, RenderError> {
    let mut canvas = RankingCanvas::new(width, height)?;
    canvas.background();
    draw_snapshot_meta_lines(
        &mut canvas,
        input.event_id,
        &input.event,
        input.character_id,
        input.last_snapshot_ts,
        48.0,
        28.0,
        22.0,
        18.0,
        820.0,
    );
    canvas.text(
        &format!("{} 条可查榜线", input.borders.len()),
        width as f32 - 48.0,
        72.0,
        18.0,
        Color::MUTED,
        TextAlign::Right,
        240.0,
    );

    let table_x = 40.0;
    let table_y = 92.0;
    let table_w = 827.0;
    canvas.glass(
        table_x,
        table_y,
        table_w,
        height as f32 - table_y - 32.0 - FOOTER_HEIGHT as f32,
    );

    // 表头
    canvas.panel(
        table_x + 12.0,
        header_h - 33.0,
        table_w - 24.0,
        34.0,
        Color::PANEL_ALT,
    );
    draw_border_table_columns(
        &mut canvas,
        table_x + 24.0,
        header_h - 10.0,
        table_w - 48.0,
    );

    if input.borders.is_empty() {
        canvas.text(
            "当前没有可查榜线",
            width as f32 / 2.0,
            height as f32 / 2.0,
            28.0,
            Color::MUTED,
            TextAlign::Center,
            480.0,
        );
    } else {
        // 绘制每一行
        let mut y = header_h + 34.0;
        for (index, border) in input.borders.iter().enumerate() {
            let bg = if index % 2 == 0 {
                Color::ROW
            } else {
                Color::ROW_ALT
            };
            canvas.panel(table_x + 12.0, y - 23.0, table_w - 24.0, row_h - 4.0, bg);
            draw_border_table_row(
                &mut canvas,
                border,
                input.last_snapshot_ts,
                table_x + 24.0,
                y,
                table_w - 48.0,
            );
            y += row_h;
        }
    }

    draw_footer(&mut canvas, width, height);
    canvas.finish(OUTPUT_QUALITY)
}

#[cfg(feature = "skia")]
fn render_player_detail_skia(
    input: &RankingPlayerDetailInput,
    width: u32,
    height: u32,
) -> Result<RenderOutput, RenderError> {
    let mut canvas = RankingCanvas::new(width, height)?;
    canvas.background();
    let meta_lines = snapshot_meta_lines(
        input.event_id,
        &input.event,
        input.character_id,
        input.last_snapshot_ts,
    );
    draw_player_detail_card(
        &mut canvas,
        &input.player,
        &meta_lines,
        40.0,
        34.0,
        width as f32 - 96.0,
    );
    draw_hourly_plays_panel(
        &mut canvas,
        &input.player.metrics.hourly_plays,
        40.0,
        548.0,
        width as f32 - 96.0,
    );
    draw_footer(&mut canvas, width, height);
    canvas.finish(OUTPUT_QUALITY)
}

#[cfg(feature = "skia")]
fn render_player_batch_skia(
    input: &RankingPlayerBatchInput,
    width: u32,
    height: u32,
) -> Result<RenderOutput, RenderError> {
    let mut canvas = RankingCanvas::new(width, height)?;
    canvas.background();
    draw_snapshot_meta_lines(
        &mut canvas,
        input.event_id,
        &input.event,
        input.character_id,
        input.last_snapshot_ts,
        48.0,
        26.0,
        22.0,
        18.0,
        760.0,
    );
    canvas.text(
        &format!("{} 人", input.players.len()),
        width as f32 - 48.0,
        70.0,
        18.0,
        Color::MUTED,
        TextAlign::Right,
        160.0,
    );
    let mut y = 104.0;
    let meta = snapshot_meta_lines(
        input.event_id,
        &input.event,
        input.character_id,
        input.last_snapshot_ts,
    );
    for player in &input.players {
        draw_player_detail_card(&mut canvas, player, &meta, 48.0, y, width as f32 - 96.0);
        y += 512.0;
    }
    canvas.finish(OUTPUT_QUALITY)
}

// ============= 时速渲染 =============

fn render_speed_detail(_input: &RankingSpeedDetailInput) -> Result<RenderOutput, RenderError> {
    let width = DETAIL_WIDTH;
    let height = 460 + FOOTER_HEIGHT;

    #[cfg(feature = "skia")]
    {
        return render_speed_detail_skia(_input, width, height);
    }

    #[cfg(not(feature = "skia"))]
    Ok(placeholder_output(width, height))
}

fn render_speed_borders(input: &RankingSpeedBordersInput) -> Result<RenderOutput, RenderError> {
    let row_h = 42.0;
    let header_h = 146.0;
    let height = (header_h + 34.0 + row_h * input.borders.len() as f32 + FOOTER_HEIGHT as f32 + 32.0)
        .ceil()
        .max(330.0) as u32;

    #[cfg(feature = "skia")]
    {
        return render_speed_borders_skia(input, BOARD_WIDTH, height, row_h, header_h);
    }

    #[cfg(not(feature = "skia"))]
    Ok(placeholder_output(BOARD_WIDTH, height))
}

fn render_speed_compare(input: &RankingSpeedCompareInput) -> Result<RenderOutput, RenderError> {
    let width = DETAIL_WIDTH;
    let height = (120 + input.subjects.len() as u32 * 64 + 60).max(240);

    #[cfg(feature = "skia")]
    {
        return render_speed_compare_skia(input, width, height);
    }

    #[cfg(not(feature = "skia"))]
    Ok(placeholder_output(width, height))
}

// ============= 日速渲染 =============

fn render_daily_detail(_input: &RankingDailyDetailInput) -> Result<RenderOutput, RenderError> {
    let width = DETAIL_WIDTH;
    let height = 380 + FOOTER_HEIGHT;

    #[cfg(feature = "skia")]
    {
        return render_daily_detail_skia(_input, width, height);
    }

    #[cfg(not(feature = "skia"))]
    Ok(placeholder_output(width, height))
}

fn render_daily_borders(input: &RankingDailyBordersInput) -> Result<RenderOutput, RenderError> {
    let row_h = 42.0;
    let header_h = 146.0;
    let height = (header_h + 34.0 + row_h * input.borders.len() as f32 + FOOTER_HEIGHT as f32 + 32.0)
        .ceil()
        .max(330.0) as u32;

    #[cfg(feature = "skia")]
    {
        return render_daily_borders_skia(input, BOARD_WIDTH, height, row_h, header_h);
    }

    #[cfg(not(feature = "skia"))]
    Ok(placeholder_output(BOARD_WIDTH, height))
}

fn render_daily_compare(input: &RankingDailyCompareInput) -> Result<RenderOutput, RenderError> {
    let width = DETAIL_WIDTH;
    let height = (120 + input.subjects.len() as u32 * 64 + 60).max(240);

    #[cfg(feature = "skia")]
    {
        return render_daily_compare_skia(input, width, height);
    }

    #[cfg(not(feature = "skia"))]
    Ok(placeholder_output(width, height))
}

const FOOTER_HEIGHT: u32 = 36;

#[cfg(feature = "skia")]
fn render_speed_detail_skia(
    input: &RankingSpeedDetailInput,
    width: u32,
    height: u32,
) -> Result<RenderOutput, RenderError> {
    let mut canvas = RankingCanvas::new(width, height)?;
    canvas.background();
    draw_snapshot_meta_lines(
        &mut canvas,
        input.event_id,
        &input.event,
        input.character_id,
        input.last_snapshot_ts,
        48.0,
        28.0,
        22.0,
        18.0,
        760.0,
    );
    canvas.text(
        "时速详情",
        width as f32 - 48.0,
        72.0,
        18.0,
        Color::MUTED,
        TextAlign::Right,
        240.0,
    );

    // glass 卡片
    let card_x = 40.0;
    let card_y = 100.0;
    let card_w = width as f32 - 80.0;
    let card_h = height as f32 - card_y - FOOTER_HEIGHT as f32 - 8.0;
    canvas.glass(card_x, card_y, card_w, card_h);

    // 玩家名 + 排名 + 分数
    let p = &input.player;
    let inner_x = card_x + 20.0;
    let inner_w = card_w - 40.0;
    let mut y = card_y + 36.0;

    canvas.text(
        &format!("#{} {}", p.rank, p.name),
        inner_x,
        y,
        28.0,
        Color::TEXT,
        TextAlign::Left,
        inner_w,
    );
    y += 38.0;
    canvas.text(
        &format_w(p.score as f64),
        inner_x,
        y,
        24.0,
        Color::TEXT,
        TextAlign::Left,
        inner_w,
    );
    y += 40.0;

    // 3 stat_chips: 最近一轮 / 1h名次 / 时速
    let chip_w = (inner_w - 16.0) / 3.0;
    let chip_h = 52.0;
    let chips = [
        ("最近一轮", &format_w(p.projected_20m_x3 / 3.0), rate_color(p.projected_20m_x3 / 3.0, p.tracking_window_secs)),
        ("1h名次", &display_rank_delta_1h(p.rank_delta_1h, p.rank_delta_1h_estimated, p.tracking_window_secs), rank_delta_color(p.rank_delta_1h)),
        ("时速", &display_hourly_rate(p.hourly_rate, p.tracking_window_secs), rate_color(p.hourly_rate, p.tracking_window_secs)),
    ];
    for (i, (label, value, color)) in chips.iter().enumerate() {
        draw_stat_chip(&mut canvas, value, label, inner_x + i as f32 * (chip_w + 8.0), y, chip_w, *color);
    }
    y += chip_h + 16.0;

    // 5 mini_chips: 停车状态 / 近1小时 / 窗口 / 20m×3 / 活跃
    let mini_w = (inner_w - 32.0) / 5.0;
    let (status_label, status_color) = player_activity_status(p.parked);
    let mini_chips: [(&str, &str, Color); 5] = [
        ("状态", status_label, status_color),
        ("近1小时", &p.plays_this_hour.to_string(), Color::TEXT),
        ("窗口", &format_window(p.tracking_window_secs), Color::MUTED),
        ("20m×3", &display_hourly_rate(p.projected_20m_x3, p.tracking_window_secs), rate_color(p.projected_20m_x3, p.tracking_window_secs)),
        ("活跃", &format_plays(p.plays_this_hour), Color::MUTED),
    ];
    for (i, (label, value, color)) in mini_chips.iter().enumerate() {
        draw_mini_chip(&mut canvas, label, value, inner_x + i as f32 * (mini_w + 8.0), y, mini_w, *color);
    }

    // footer
    draw_footer(&mut canvas, width, height);

    canvas.finish(OUTPUT_QUALITY)
}

#[cfg(feature = "skia")]
fn render_speed_borders_skia(
    input: &RankingSpeedBordersInput,
    width: u32,
    height: u32,
    row_h: f32,
    header_h: f32,
) -> Result<RenderOutput, RenderError> {
    let mut canvas = RankingCanvas::new(width, height)?;
    canvas.background();
    draw_snapshot_meta_lines(
        &mut canvas,
        input.event_id,
        &input.event,
        input.character_id,
        input.last_snapshot_ts,
        48.0,
        28.0,
        22.0,
        18.0,
        820.0,
    );
    canvas.text(
        &format!("{} 条榜线时速", input.borders.len()),
        width as f32 - 48.0,
        72.0,
        18.0,
        Color::MUTED,
        TextAlign::Right,
        240.0,
    );

    let table_x = 40.0;
    let table_y = 92.0;
    let table_w = 827.0;
    let table_h = height as f32 - table_y - FOOTER_HEIGHT as f32 - 32.0;
    canvas.glass(table_x, table_y, table_w, table_h);

    // 表头
    canvas.panel(
        table_x + 12.0,
        header_h - 33.0,
        table_w - 24.0,
        34.0,
        Color::PANEL_ALT,
    );
    draw_speed_border_table_columns(
        &mut canvas,
        table_x + 24.0,
        header_h - 10.0,
        table_w - 48.0,
    );

    if input.borders.is_empty() {
        canvas.text(
            "当前没有榜线时速数据",
            width as f32 / 2.0,
            table_y + table_h / 2.0,
            28.0,
            Color::MUTED,
            TextAlign::Center,
            480.0,
        );
    } else {
        // 绘制每一行
        let mut y = header_h + 34.0;
        for (index, border) in input.borders.iter().enumerate() {
            let bg = if index % 2 == 0 {
                Color::ROW
            } else {
                Color::ROW_ALT
            };
            canvas.panel(table_x + 12.0, y - 23.0, table_w - 24.0, row_h - 4.0, bg);
            draw_speed_border_table_row(
                &mut canvas,
                border,
                input.last_snapshot_ts,
                table_x + 24.0,
                y,
                table_w - 48.0,
            );
            y += row_h;
        }
    }

    draw_footer(&mut canvas, width, height);
    canvas.finish(OUTPUT_QUALITY)
}

#[cfg(feature = "skia")]
fn render_speed_compare_skia(
    input: &RankingSpeedCompareInput,
    width: u32,
    height: u32,
) -> Result<RenderOutput, RenderError> {
    let mut canvas = RankingCanvas::new(width, height)?;
    canvas.background();
    draw_snapshot_meta_lines(
        &mut canvas,
        input.event_id,
        &input.event,
        input.character_id,
        input.last_snapshot_ts,
        48.0,
        28.0,
        22.0,
        18.0,
        760.0,
    );
    canvas.text(
        &format!("{} 人时速对比", input.subjects.len()),
        width as f32 - 48.0,
        72.0,
        18.0,
        Color::MUTED,
        TextAlign::Right,
        240.0,
    );

    // 水平条状图
    let max_rate = input
        .subjects
        .iter()
        .map(|s| s.hourly_rate)
        .fold(0.0f64, f64::max)
        .max(1.0);

    let panel_x = 40.0;
    let panel_y = 100.0;
    let panel_w = width as f32 - 80.0;
    let panel_h = height as f32 - panel_y - FOOTER_HEIGHT as f32 - 8.0;
    canvas.glass(panel_x, panel_y, panel_w, panel_h);

    let inner_x = panel_x + 20.0;
    let inner_w = panel_w - 40.0;
    let bar_label_w = 160.0;
    let bar_value_w = 120.0;
    let bar_max_w = inner_w - bar_label_w - bar_value_w - 16.0;
    let row_h = 64.0;

    for (i, subject) in input.subjects.iter().enumerate() {
        let y = panel_y + 20.0 + i as f32 * row_h;

        // 排名 + 名字
        let label = format!("#{} {}", subject.rank, subject.name);
        canvas.text(
            &label,
            inner_x,
            y + 22.0,
            20.0,
            Color::TEXT,
            TextAlign::Left,
            bar_label_w,
        );

        // 条状图
        let bar_x = inner_x + bar_label_w + 8.0;
        let bar_y = y + 8.0;
        let bar_h = 28.0;
        // 背景轨道
        canvas.panel(bar_x, bar_y, bar_max_w, bar_h, Color::ROW_ALT);
        // 实际条
        let filled_w = if max_rate > 0.0 {
            (subject.hourly_rate / max_rate * bar_max_w as f64) as f32
        } else {
            0.0
        };
        if filled_w > 0.0 {
            canvas.panel(bar_x, bar_y, filled_w, bar_h, Color::MIKU);
        }

        // 速率值
        let rate_text = display_hourly_rate(subject.hourly_rate, subject.tracking_window_secs);
        canvas.text(
            &rate_text,
            inner_x + inner_w,
            y + 22.0,
            18.0,
            rate_color(subject.hourly_rate, subject.tracking_window_secs),
            TextAlign::Right,
            bar_value_w,
        );
    }

    draw_footer(&mut canvas, width, height);
    canvas.finish(OUTPUT_QUALITY)
}

#[cfg(feature = "skia")]
fn render_daily_detail_skia(
    input: &RankingDailyDetailInput,
    width: u32,
    height: u32,
) -> Result<RenderOutput, RenderError> {
    let mut canvas = RankingCanvas::new(width, height)?;
    canvas.background();
    draw_snapshot_meta_lines(
        &mut canvas,
        input.event_id,
        &input.event,
        input.character_id,
        input.last_snapshot_ts,
        48.0,
        28.0,
        22.0,
        18.0,
        760.0,
    );
    canvas.text(
        "日速详情",
        width as f32 - 48.0,
        72.0,
        18.0,
        Color::MUTED,
        TextAlign::Right,
        240.0,
    );

    // glass 卡片
    let card_x = 40.0;
    let card_y = 100.0;
    let card_w = width as f32 - 80.0;
    let card_h = height as f32 - card_y - FOOTER_HEIGHT as f32 - 8.0;
    canvas.glass(card_x, card_y, card_w, card_h);

    let p = &input.player;
    let inner_x = card_x + 20.0;
    let inner_w = card_w - 40.0;
    let mut y = card_y + 36.0;

    canvas.text(
        &format!("#{} {}", p.rank, p.name),
        inner_x,
        y,
        28.0,
        Color::TEXT,
        TextAlign::Left,
        inner_w,
    );
    y += 38.0;
    canvas.text(
        &format_w(p.score as f64),
        inner_x,
        y,
        24.0,
        Color::TEXT,
        TextAlign::Left,
        inner_w,
    );
    y += 40.0;

    // 4 stat_chips: 日速 / 日均时速 / 24h名次 / 24h增量
    let chip_w = (inner_w - 24.0) / 4.0;
    let chip_h = 52.0;
    let chips = [
        ("日速", &display_daily_rate(p.daily_rate, p.daily_tracking_window_secs, p.is_fallback), rate_color(p.daily_hourly_rate, p.daily_tracking_window_secs)),
        ("日均时速", &display_hourly_rate(p.daily_hourly_rate, p.daily_tracking_window_secs), rate_color(p.daily_hourly_rate, p.daily_tracking_window_secs)),
        ("24h名次", &display_rank_delta_1h(p.rank_delta_24h, p.rank_delta_24h_estimated, p.daily_tracking_window_secs), rank_delta_color(p.rank_delta_24h)),
        ("24h增量", &format_signed(p.daily_score_delta), delta_color(p.daily_score_delta)),
    ];
    for (i, (label, value, color)) in chips.iter().enumerate() {
        draw_stat_chip(&mut canvas, value, label, inner_x + i as f32 * (chip_w + 8.0), y, chip_w, *color);
    }
    y += chip_h + 16.0;

    // 时速对比参考
    let mini_w = (inner_w - 8.0) / 2.0;
    let mini_chips: [(&str, &str, Color); 2] = [
        ("内存时速", &display_hourly_rate(p.hourly_rate, 3600), rate_color(p.hourly_rate, 3600)),
        ("窗口", &format_window(p.daily_tracking_window_secs), Color::MUTED),
    ];
    for (i, (label, value, color)) in mini_chips.iter().enumerate() {
        draw_mini_chip(&mut canvas, label, value, inner_x + i as f32 * (mini_w + 8.0), y, mini_w, *color);
    }

    draw_footer(&mut canvas, width, height);
    canvas.finish(OUTPUT_QUALITY)
}

#[cfg(feature = "skia")]
fn render_daily_borders_skia(
    input: &RankingDailyBordersInput,
    width: u32,
    height: u32,
    row_h: f32,
    header_h: f32,
) -> Result<RenderOutput, RenderError> {
    let mut canvas = RankingCanvas::new(width, height)?;
    canvas.background();
    draw_snapshot_meta_lines(
        &mut canvas,
        input.event_id,
        &input.event,
        input.character_id,
        input.last_snapshot_ts,
        48.0,
        28.0,
        22.0,
        18.0,
        820.0,
    );
    canvas.text(
        &format!("{} 条榜线日速", input.borders.len()),
        width as f32 - 48.0,
        72.0,
        18.0,
        Color::MUTED,
        TextAlign::Right,
        240.0,
    );

    let table_x = 40.0;
    let table_y = 92.0;
    let table_w = 827.0;
    let table_h = height as f32 - table_y - FOOTER_HEIGHT as f32 - 32.0;
    canvas.glass(table_x, table_y, table_w, table_h);

    // 表头
    canvas.panel(
        table_x + 12.0,
        header_h - 33.0,
        table_w - 24.0,
        34.0,
        Color::PANEL_ALT,
    );
    draw_daily_border_table_columns(
        &mut canvas,
        table_x + 24.0,
        header_h - 10.0,
        table_w - 48.0,
    );

    if input.borders.is_empty() {
        canvas.text(
            "当前没有榜线日速数据",
            width as f32 / 2.0,
            table_y + table_h / 2.0,
            28.0,
            Color::MUTED,
            TextAlign::Center,
            480.0,
        );
    } else {
        // 绘制每一行
        let mut y = header_h + 34.0;
        for (index, border) in input.borders.iter().enumerate() {
            let bg = if index % 2 == 0 {
                Color::ROW
            } else {
                Color::ROW_ALT
            };
            canvas.panel(table_x + 12.0, y - 23.0, table_w - 24.0, row_h - 4.0, bg);
            draw_daily_border_table_row(
                &mut canvas,
                border,
                input.last_snapshot_ts,
                table_x + 24.0,
                y,
                table_w - 48.0,
            );
            y += row_h;
        }
    }

    draw_footer(&mut canvas, width, height);
    canvas.finish(OUTPUT_QUALITY)
}

#[cfg(feature = "skia")]
fn render_daily_compare_skia(
    input: &RankingDailyCompareInput,
    width: u32,
    height: u32,
) -> Result<RenderOutput, RenderError> {
    let mut canvas = RankingCanvas::new(width, height)?;
    canvas.background();
    draw_snapshot_meta_lines(
        &mut canvas,
        input.event_id,
        &input.event,
        input.character_id,
        input.last_snapshot_ts,
        48.0,
        28.0,
        22.0,
        18.0,
        760.0,
    );
    canvas.text(
        &format!("{} 人日速对比", input.subjects.len()),
        width as f32 - 48.0,
        72.0,
        18.0,
        Color::MUTED,
        TextAlign::Right,
        240.0,
    );

    // 水平条状图
    let max_rate = input
        .subjects
        .iter()
        .map(|s| s.daily_rate)
        .fold(0.0f64, f64::max)
        .max(1.0);

    let panel_x = 40.0;
    let panel_y = 100.0;
    let panel_w = width as f32 - 80.0;
    let panel_h = height as f32 - panel_y - FOOTER_HEIGHT as f32 - 8.0;
    canvas.glass(panel_x, panel_y, panel_w, panel_h);

    let inner_x = panel_x + 20.0;
    let inner_w = panel_w - 40.0;
    let bar_label_w = 160.0;
    let bar_value_w = 120.0;
    let bar_max_w = inner_w - bar_label_w - bar_value_w - 16.0;
    let row_h = 64.0;

    for (i, subject) in input.subjects.iter().enumerate() {
        let y = panel_y + 20.0 + i as f32 * row_h;

        let label = format!("#{} {}", subject.rank, subject.name);
        canvas.text(
            &label,
            inner_x,
            y + 22.0,
            20.0,
            Color::TEXT,
            TextAlign::Left,
            bar_label_w,
        );

        let bar_x = inner_x + bar_label_w + 8.0;
        let bar_y = y + 8.0;
        let bar_h = 28.0;
        canvas.panel(bar_x, bar_y, bar_max_w, bar_h, Color::ROW_ALT);
        let filled_w = if max_rate > 0.0 {
            (subject.daily_rate / max_rate * bar_max_w as f64) as f32
        } else {
            0.0
        };
        if filled_w > 0.0 {
            canvas.panel(bar_x, bar_y, filled_w, bar_h, Color::MIKU);
        }

        let rate_text = display_daily_rate(subject.daily_rate, subject.daily_tracking_window_secs, subject.is_fallback);
        canvas.text(
            &rate_text,
            inner_x + inner_w,
            y + 22.0,
            18.0,
            rate_color(subject.daily_hourly_rate, subject.daily_tracking_window_secs),
            TextAlign::Right,
            bar_value_w,
        );
    }

    draw_footer(&mut canvas, width, height);
    canvas.finish(OUTPUT_QUALITY)
}

#[cfg(feature = "skia")]
fn draw_snapshot_meta_lines(
    canvas: &mut RankingCanvas,
    event_id: i32,
    event: &RankingEventMeta,
    character_id: i16,
    last_snapshot_ts: Option<i64>,
    x: f32,
    first_baseline: f32,
    line_height: f32,
    size: f32,
    max_width: f32,
) {
    let lines = snapshot_meta_lines(event_id, event, character_id, last_snapshot_ts);
    draw_meta_lines(
        canvas,
        &lines,
        x,
        first_baseline,
        line_height,
        size,
        max_width,
    );
}

#[cfg(feature = "skia")]
fn draw_meta_lines(
    canvas: &mut RankingCanvas,
    lines: &[String],
    x: f32,
    first_baseline: f32,
    line_height: f32,
    size: f32,
    max_width: f32,
) {
    for (index, line) in lines.iter().enumerate() {
        canvas.text(
            line,
            x,
            first_baseline + index as f32 * line_height,
            size,
            Color::MUTED,
            TextAlign::Left,
            max_width,
        );
    }
}

#[cfg(feature = "skia")]
fn draw_board_columns(canvas: &mut RankingCanvas, x: f32, y: f32, width: f32, compact: bool) {
    let columns = board_columns(x, width, compact);
    for (label, col_x, align) in [
        ("排名", columns.rank, TextAlign::Left),
        ("玩家", columns.name, TextAlign::Left),
        ("分数", columns.score, TextAlign::Right),
        ("时速", columns.delta, TextAlign::Right),
        ("1小时名次", columns.rank_delta, TextAlign::Right),
    ] {
        canvas.text(label, col_x, y, 18.0, Color::MUTED, align, 160.0);
    }
    if !compact {
        canvas.text(
            "状态",
            columns.active,
            y,
            18.0,
            Color::MUTED,
            TextAlign::Right,
            160.0,
        );
    }
}

#[cfg(feature = "skia")]
fn draw_player_row(
    canvas: &mut RankingCanvas,
    player: &RankingPlayerRecord,
    x: f32,
    y: f32,
    width: f32,
    compact: bool,
) {
    let columns = board_columns(x, width, compact);
    let rank_size = if compact { 18.0 } else { 22.0 };
    let text_size = if compact { 17.0 } else { 21.0 };
    canvas.text(
        &format!("#{}", player.rank),
        columns.rank,
        y,
        rank_size,
        Color::MIKU,
        TextAlign::Left,
        100.0,
    );
    canvas.text(
        &player.name,
        columns.name,
        y,
        text_size,
        Color::TEXT,
        TextAlign::Left,
        if compact { 360.0 } else { 430.0 },
    );
    canvas.text(
        &format_number(player.score),
        columns.score,
        y,
        text_size,
        Color::TEXT,
        TextAlign::Right,
        220.0,
    );
    canvas.text(
        &display_hourly_rate(player.hourly_rate, player.tracking_window_secs),
        columns.delta,
        y,
        text_size,
        rate_color(player.hourly_rate, player.tracking_window_secs),
        TextAlign::Right,
        170.0,
    );
    canvas.text(
        &display_rank_delta_1h(
            player.rank_delta_1h,
            player.rank_delta_1h_estimated,
            player.tracking_window_secs,
        ),
        columns.rank_delta,
        y,
        text_size,
        rank_delta_color(player.rank_delta_1h),
        TextAlign::Right,
        150.0,
    );
    if !compact {
        let (status, color) = player_activity_status(player.parked);
        canvas.text(
            status,
            columns.active,
            y,
            18.0,
            color,
            TextAlign::Right,
            160.0,
        );
    }
}

#[cfg(feature = "skia")]
fn draw_compact_columns(canvas: &mut RankingCanvas, x: f32, y: f32, width: f32) {
    canvas.panel(x, y - 25.0, width, 32.0, Color::PANEL_ALT);
    canvas.text(
        "排名",
        x + 14.0,
        y - 3.0,
        15.0,
        Color::MUTED,
        TextAlign::Left,
        70.0,
    );
    canvas.text(
        "玩家",
        x + 80.0,
        y - 3.0,
        15.0,
        Color::MUTED,
        TextAlign::Left,
        150.0,
    );
    canvas.text(
        "分数",
        x + width - 250.0,
        y - 3.0,
        15.0,
        Color::MUTED,
        TextAlign::Right,
        120.0,
    );
    canvas.text(
        "时速",
        x + width - 154.0,
        y - 3.0,
        15.0,
        Color::MUTED,
        TextAlign::Right,
        90.0,
    );
    canvas.text(
        "1小时",
        x + width - 82.0,
        y - 3.0,
        15.0,
        Color::MUTED,
        TextAlign::Right,
        60.0,
    );
    canvas.text(
        "状态",
        x + width - 12.0,
        y - 3.0,
        15.0,
        Color::MUTED,
        TextAlign::Right,
        48.0,
    );
}

#[cfg(feature = "skia")]
fn draw_compact_player_row(
    canvas: &mut RankingCanvas,
    player: &RankingPlayerRecord,
    x: f32,
    y: f32,
    width: f32,
    index: usize,
) {
    let bg = if index % 2 == 0 {
        Color::ROW
    } else {
        Color::ROW_ALT
    };
    canvas.panel(x, y - 22.0, width, 27.0, bg);
    canvas.text(
        &format!("#{}", player.rank),
        x + 14.0,
        y - 3.0,
        16.0,
        Color::MIKU,
        TextAlign::Left,
        62.0,
    );
    canvas.text(
        &player.name,
        x + 80.0,
        y - 3.0,
        15.0,
        Color::TEXT,
        TextAlign::Left,
        width - 462.0,
    );
    canvas.text(
        &format_number(player.score),
        x + width - 250.0,
        y - 3.0,
        15.0,
        Color::TEXT,
        TextAlign::Right,
        118.0,
    );
    canvas.text(
        &display_rate(player.hourly_rate, player.tracking_window_secs),
        x + width - 154.0,
        y - 3.0,
        15.0,
        rate_color(player.hourly_rate, player.tracking_window_secs),
        TextAlign::Right,
        88.0,
    );
    canvas.text(
        &display_rank_delta_1h(
            player.rank_delta_1h,
            player.rank_delta_1h_estimated,
            player.tracking_window_secs,
        ),
        x + width - 82.0,
        y - 3.0,
        15.0,
        rank_delta_color(player.rank_delta_1h),
        TextAlign::Right,
        56.0,
    );
    let (status, color) = player_activity_status(player.parked);
    canvas.text(
        status,
        x + width - 12.0,
        y - 3.0,
        15.0,
        color,
        TextAlign::Right,
        52.0,
    );
}

#[cfg(feature = "skia")]
fn draw_border_table_columns(canvas: &mut RankingCanvas, x: f32, y: f32, width: f32) {
    let cols = border_table_columns(x, width);
    canvas.text("排名", cols.rank, y, 18.0, Color::MUTED, TextAlign::Left, 120.0);
    canvas.text("分数", cols.score_rate, y, 18.0, Color::MUTED, TextAlign::Right, 360.0);
    canvas.text("更新时间", cols.timestamp, y, 18.0, Color::MUTED, TextAlign::Right, 180.0);
}

#[cfg(feature = "skia")]
fn draw_speed_border_table_columns(canvas: &mut RankingCanvas, x: f32, y: f32, width: f32) {
    let cols = border_table_columns(x, width);
    canvas.text("排名", cols.rank, y, 18.0, Color::MUTED, TextAlign::Left, 120.0);
    canvas.text("时速", cols.score_rate, y, 18.0, Color::MUTED, TextAlign::Right, 360.0);
    canvas.text("更新时间", cols.timestamp, y, 18.0, Color::MUTED, TextAlign::Right, 180.0);
}

#[cfg(feature = "skia")]
fn draw_daily_border_table_columns(canvas: &mut RankingCanvas, x: f32, y: f32, width: f32) {
    let cols = border_table_columns(x, width);
    canvas.text("排名", cols.rank, y, 18.0, Color::MUTED, TextAlign::Left, 120.0);
    canvas.text("日速", cols.score_rate, y, 18.0, Color::MUTED, TextAlign::Right, 360.0);
    canvas.text("更新时间", cols.timestamp, y, 18.0, Color::MUTED, TextAlign::Right, 180.0);
}

#[cfg(feature = "skia")]
fn draw_border_table_row(
    canvas: &mut RankingCanvas,
    border: &RankingBorderRecord,
    last_snapshot_ts: Option<i64>,
    x: f32,
    y: f32,
    width: f32,
) {
    let cols = border_table_columns(x, width);

    // 排名
    canvas.text(
        &format!("#{}", border.rank),
        cols.rank,
        y,
        24.0,
        Color::NIIGO,
        TextAlign::Left,
        120.0,
    );

    // 分数（单行显示）
    canvas.text(
        &format_number(border.score),
        cols.score_rate,
        y,
        26.0,
        Color::TEXT,
        TextAlign::Right,
        480.0,
    );

    // 更新时间
    canvas.text(
        &display_time_ago(last_snapshot_ts),
        cols.timestamp,
        y,
        18.0,
        Color::MUTED,
        TextAlign::Right,
        180.0,
    );
}

#[cfg(feature = "skia")]
#[derive(Debug, Clone, Copy)]
struct BorderTableColumns {
    rank: f32,
    score_rate: f32,
    timestamp: f32,
}

#[cfg(feature = "skia")]
fn border_table_columns(x: f32, _width: f32) -> BorderTableColumns {
    // Fixed 827px table: rank(80px) | score/rate(500px) | timestamp(199px)
    // x is content start (table_x + 24), content width = 779px
    BorderTableColumns {
        rank: x + 18.0,         // Left-aligned in rank column
        score_rate: x + 562.0,  // Right-aligned in score/rate column (80+500-18)
        timestamp: x + 761.0,   // Right-aligned in timestamp column (779-18)
    }
}

#[cfg(feature = "skia")]
fn draw_speed_border_table_row(
    canvas: &mut RankingCanvas,
    border: &RankingSpeedBorderCard,
    last_snapshot_ts: Option<i64>,
    x: f32,
    y: f32,
    width: f32,
) {
    let cols = border_table_columns(x, width);

    // 排名
    canvas.text(
        &format!("#{}", border.rank),
        cols.rank,
        y,
        20.0,
        Color::NIIGO,
        TextAlign::Left,
        120.0,
    );

    // 时速（单独显示，不带分数）
    canvas.text(
        &display_hourly_rate(border.hourly_rate, border.tracking_window_secs),
        cols.score_rate,
        y,
        20.0,
        rate_color(border.hourly_rate, border.tracking_window_secs),
        TextAlign::Right,
        480.0,
    );

    // 更新时间
    canvas.text(
        &display_time_ago(last_snapshot_ts),
        cols.timestamp,
        y,
        17.0,
        Color::MUTED,
        TextAlign::Right,
        180.0,
    );
}

#[cfg(feature = "skia")]
fn draw_daily_border_table_row(
    canvas: &mut RankingCanvas,
    border: &RankingDailyBorderCard,
    last_snapshot_ts: Option<i64>,
    x: f32,
    y: f32,
    width: f32,
) {
    let cols = border_table_columns(x, width);

    // 排名
    canvas.text(
        &format!("#{}", border.rank),
        cols.rank,
        y,
        20.0,
        Color::NIIGO,
        TextAlign::Left,
        120.0,
    );

    // 日速（单独显示，不带分数）
    canvas.text(
        &display_daily_rate(border.daily_rate, border.daily_tracking_window_secs, false),
        cols.score_rate,
        y,
        20.0,
        rate_color(border.daily_hourly_rate, border.daily_tracking_window_secs),
        TextAlign::Right,
        480.0,
    );

    // 更新时间
    canvas.text(
        &display_time_ago(last_snapshot_ts),
        cols.timestamp,
        y,
        17.0,
        Color::MUTED,
        TextAlign::Right,
        180.0,
    );
}

#[cfg(feature = "skia")]
fn draw_footer(canvas: &mut RankingCanvas, width: u32, height: u32) {
    let footer_y = height as f32 - FOOTER_HEIGHT as f32;
    // footer 背景线
    canvas.panel(0.0, footer_y, width as f32, FOOTER_HEIGHT as f32, Color::SURFACE);
    // 数据来源
    canvas.text(
        "数据来源自 allium-scapus",
        width as f32 / 2.0,
        footer_y + 23.0,
        12.0,
        Color::MUTED,
        TextAlign::Center,
        width as f32 - 40.0,
    );
}

#[cfg(feature = "skia")]
fn draw_activity_note(canvas: &mut RankingCanvas, x: f32, y: f32, width: f32) {
    canvas.panel(x, y, width, 88.0, Color::PANEL_ALT);
    canvas.text(
        "状态说明",
        x + 18.0,
        y + 26.0,
        18.0,
        Color::MIKU,
        TextAlign::Left,
        120.0,
    );
    canvas.text(
        "活跃：分数仍在增长或未达到不活跃阈值",
        x + 132.0,
        y + 26.0,
        18.0,
        Color::GOOD,
        TextAlign::Left,
        420.0,
    );
    canvas.text(
        "不活跃：连续24轮（约4分钟）分数无增长；增长后恢复活跃",
        x + 132.0,
        y + 51.0,
        18.0,
        Color::WARN,
        TextAlign::Left,
        width - 160.0,
    );
    canvas.text(
        "时速/1小时名次：不足1小时标约，未满10分钟统计中",
        x + 132.0,
        y + 76.0,
        18.0,
        Color::MUTED,
        TextAlign::Left,
        width - 160.0,
    );
}

#[cfg(feature = "skia")]
fn draw_hourly_plays_panel(
    canvas: &mut RankingCanvas,
    hourly_plays: &[RankingHourlyPlayCount],
    x: f32,
    y: f32,
    width: f32,
) {
    canvas.glass(x, y, width, PLAYER_HOURLY_PANEL_HEIGHT);
    canvas.text(
        "最近10小时周回",
        x + 24.0,
        y + 31.0,
        20.0,
        Color::TEXT,
        TextAlign::Left,
        220.0,
    );
    canvas.text(
        "按每小时分数增长轮数统计",
        x + width - 24.0,
        y + 31.0,
        16.0,
        Color::MUTED,
        TextAlign::Right,
        260.0,
    );

    let max_plays = hourly_plays
        .iter()
        .map(|item| item.plays)
        .max()
        .unwrap_or(0)
        .max(1) as f32;
    let bar_area_x = x + 24.0;
    let bar_area_y = y + 48.0;
    let bar_area_w = width - 48.0;
    let bar_gap = 10.0;
    let count = hourly_plays.len().max(1) as f32;
    let bar_w = ((bar_area_w - bar_gap * (count - 1.0)) / count).max(28.0);
    let max_bar_h = 38.0;

    for (index, item) in hourly_plays.iter().enumerate() {
        let left = bar_area_x + index as f32 * (bar_w + bar_gap);
        let bar_h =
            (item.plays as f32 / max_plays * max_bar_h).max(if item.plays > 0 { 5.0 } else { 0.0 });
        let top = bar_area_y + max_bar_h - bar_h;
        canvas.panel(left, bar_area_y, bar_w, max_bar_h, Color::ROW_ALT);
        if bar_h > 0.0 {
            canvas.panel(left, top, bar_w, bar_h, Color::MIKU);
        }
        canvas.text(
            &format!("{}局", item.plays),
            left + bar_w / 2.0,
            bar_area_y + max_bar_h + 23.0,
            15.0,
            if item.plays > 0 {
                Color::TEXT
            } else {
                Color::MUTED
            },
            TextAlign::Center,
            bar_w + 4.0,
        );
        canvas.text(
            &display_hour(item.hour_start_ts),
            left + bar_w / 2.0,
            bar_area_y + max_bar_h + 46.0,
            14.0,
            Color::MUTED,
            TextAlign::Center,
            bar_w + 8.0,
        );
    }
}

#[cfg(feature = "skia")]
fn draw_player_detail_card(
    canvas: &mut RankingCanvas,
    card: &RankingPlayerCard,
    meta_lines: &[String],
    x: f32,
    y: f32,
    width: f32,
) {
    let height = PLAYER_DETAIL_CARD_HEIGHT;
    let player = &card.record;
    canvas.glass(x, y, width, height);
    canvas.rule(x + 22.0, y + 24.0, 92.0, 5.0, Color::NIIGO);
    draw_meta_lines(
        canvas,
        meta_lines,
        x + 132.0,
        y + 28.0,
        21.0,
        16.0,
        width - 330.0,
    );
    canvas.text(
        &format!("用户 {}", player.user_id),
        x + width - 28.0,
        y + 28.0,
        15.0,
        Color::MUTED,
        TextAlign::Right,
        260.0,
    );

    canvas.text_fit(
        &format!("#{}", player.rank),
        x + 26.0,
        y + 136.0,
        74.0,
        52.0,
        Color::NIIGO,
        TextAlign::Left,
        190.0,
    );
    canvas.text_fit(
        &format_number(player.score),
        x + width - 28.0,
        y + 128.0,
        64.0,
        46.0,
        Color::TEXT,
        TextAlign::Right,
        width - 280.0,
    );
    canvas.text(
        &player.name,
        x + 30.0,
        y + 174.0,
        29.0,
        Color::TEXT,
        TextAlign::Left,
        width - 360.0,
    );
    canvas.text(
        "当前分数",
        x + width - 30.0,
        y + 160.0,
        16.0,
        Color::MUTED,
        TextAlign::Right,
        180.0,
    );

    let chip_y = y + 210.0;
    let chip_w = (width - 76.0) / 3.0;
    draw_stat_chip(
        canvas,
        &format_signed(player.score_delta),
        "最近一轮",
        x + 24.0,
        chip_y,
        chip_w,
        delta_color(player.score_delta),
    );
    draw_stat_chip(
        canvas,
        &display_rank_delta_1h(
            player.rank_delta_1h,
            player.rank_delta_1h_estimated,
            player.tracking_window_secs,
        ),
        "1小时名次",
        x + 38.0 + chip_w,
        chip_y,
        chip_w,
        rank_delta_color(player.rank_delta_1h),
    );
    draw_stat_chip(
        canvas,
        &display_hourly_rate(card.metrics.hourly_rate, card.metrics.tracking_window_secs),
        "时速",
        x + 52.0 + chip_w * 2.0,
        chip_y,
        chip_w,
        Color::MIKU,
    );

    let gap_y = y + 292.0;
    draw_gap_cells(
        canvas,
        &border_gap_cells(card),
        "当前没有前后可查榜线",
        x + 24.0,
        gap_y,
        width - 48.0,
    );
    draw_gap_cells(
        canvas,
        &neighbor_gap_cells(card),
        "当前没有相邻玩家",
        x + 24.0,
        gap_y + 62.0,
        width - 48.0,
    );

    let small_y = y + 416.0;
    let small_w = (width - 88.0) / 5.0;
    draw_mini_chip(
        canvas,
        "近10次",
        &format_float(card.metrics.average_recent_pt),
        x + 24.0,
        small_y,
        small_w,
        Color::TEXT,
    );
    draw_mini_chip(
        canvas,
        "20分三倍",
        &format_float(card.metrics.projected_20m_x3),
        x + 34.0 + small_w,
        small_y,
        small_w,
        Color::TEXT,
    );
    draw_mini_chip(
        canvas,
        "近1小时",
        &format!("{}局", card.metrics.plays_this_hour),
        x + 44.0 + small_w * 2.0,
        small_y,
        small_w,
        Color::TEXT,
    );
    draw_mini_chip(
        canvas,
        "连续",
        &format_duration(card.metrics.consecutive_play_secs),
        x + 54.0 + small_w * 3.0,
        small_y,
        small_w,
        Color::TEXT,
    );
    draw_mini_chip(
        canvas,
        "状态",
        if card.metrics.parked {
            "不活跃"
        } else {
            "活跃"
        },
        x + 64.0 + small_w * 4.0,
        small_y,
        small_w,
        if card.metrics.parked {
            Color::WARN
        } else {
            Color::GOOD
        },
    );
}

#[cfg(feature = "skia")]
fn draw_stat_chip(
    canvas: &mut RankingCanvas,
    value: &str,
    label: &str,
    x: f32,
    y: f32,
    width: f32,
    color: Color,
) {
    canvas.panel(x, y, width, 56.0, Color::PANEL_ALT);
    canvas.text_fit(
        value,
        x + 16.0,
        y + 37.0,
        28.0,
        21.0,
        color,
        TextAlign::Left,
        width - 118.0,
    );
    canvas.text(
        label,
        x + width - 16.0,
        y + 35.0,
        16.0,
        Color::MUTED,
        TextAlign::Right,
        100.0,
    );
}

#[cfg(feature = "skia")]
#[derive(Debug, Clone)]
struct GapCell {
    relation: &'static str,
    gap_label: &'static str,
    relation_color: Color,
    relation_bg: Color,
    rank: String,
    name: String,
    score: i64,
    gap: i64,
}

#[cfg(feature = "skia")]
fn draw_gap_cells(
    canvas: &mut RankingCanvas,
    cells: &[GapCell],
    empty_text: &str,
    x: f32,
    y: f32,
    width: f32,
) {
    if cells.is_empty() {
        canvas.panel(x, y, width, 50.0, Color::PANEL_ALT);
        canvas.text(
            empty_text,
            x + 18.0,
            y + 32.0,
            20.0,
            Color::MUTED,
            TextAlign::Left,
            width - 36.0,
        );
        return;
    }

    let gap = if cells.len() > 1 { 12.0 } else { 0.0 };
    let cell_w = (width - gap * (cells.len().saturating_sub(1) as f32)) / cells.len() as f32;
    for (index, cell) in cells.iter().enumerate() {
        let cell_x = x + index as f32 * (cell_w + gap);
        draw_gap_cell(canvas, cell, cell_x, y, cell_w);
    }
}

#[cfg(feature = "skia")]
fn draw_gap_cell(canvas: &mut RankingCanvas, cell: &GapCell, x: f32, y: f32, width: f32) {
    canvas.panel(x, y, width, 50.0, Color::PANEL_ALT);
    canvas.panel(x, y, width, 50.0, cell.relation_bg);
    canvas.panel(x, y, 6.0, 50.0, cell.relation_color);
    canvas.panel(x + 14.0, y + 10.0, 58.0, 30.0, cell.relation_bg);
    canvas.text(
        cell.relation,
        x + 43.0,
        y + 31.0,
        17.0,
        cell.relation_color,
        TextAlign::Center,
        52.0,
    );

    let mid_x = x + 86.0;
    let right_x = x + width - 16.0;
    let right_w = (width * 0.42).clamp(160.0, 230.0);
    let mid_w = (right_x - right_w - mid_x - 12.0).max(92.0);
    if cell.name.is_empty() {
        canvas.text_fit(
            &cell.rank,
            mid_x,
            y + 36.0,
            24.0,
            18.0,
            cell.relation_color,
            TextAlign::Left,
            mid_w,
        );
    } else {
        canvas.text_fit(
            &cell.name,
            mid_x,
            y + 18.0,
            14.0,
            11.0,
            Color::MUTED,
            TextAlign::Left,
            mid_w,
        );
        canvas.text_fit(
            &cell.rank,
            mid_x,
            y + 42.0,
            23.0,
            18.0,
            cell.relation_color,
            TextAlign::Left,
            mid_w,
        );
    }
    canvas.text(
        &format!("分数 {}", format_number(cell.score)),
        right_x,
        y + 19.0,
        14.0,
        Color::MUTED,
        TextAlign::Right,
        right_w,
    );
    canvas.text_fit(
        &format!("{} {}", cell.gap_label, format_number(cell.gap.max(0))),
        right_x,
        y + 42.0,
        21.0,
        16.0,
        cell.relation_color,
        TextAlign::Right,
        right_w,
    );
}

#[cfg(feature = "skia")]
fn draw_mini_chip(
    canvas: &mut RankingCanvas,
    label: &str,
    value: &str,
    x: f32,
    y: f32,
    width: f32,
    color: Color,
) {
    canvas.panel(x, y, width, 52.0, Color::PANEL_ALT);
    canvas.text(
        label,
        x + 12.0,
        y + 20.0,
        14.0,
        Color::MUTED,
        TextAlign::Left,
        width - 24.0,
    );
    canvas.text_fit(
        value,
        x + 12.0,
        y + 42.0,
        20.0,
        15.0,
        color,
        TextAlign::Left,
        width - 24.0,
    );
}

#[cfg(feature = "skia")]
#[derive(Debug, Clone, Copy)]
struct BoardColumns {
    rank: f32,
    name: f32,
    score: f32,
    delta: f32,
    rank_delta: f32,
    active: f32,
}

#[cfg(feature = "skia")]
fn board_columns(x: f32, width: f32, compact: bool) -> BoardColumns {
    if compact {
        BoardColumns {
            rank: x + 18.0,
            name: x + 104.0,
            score: x + width - 360.0,
            delta: x + width - 170.0,
            rank_delta: x + width - 32.0,
            active: x + width - 32.0,
        }
    } else {
        BoardColumns {
            rank: x + 18.0,
            name: x + 118.0,
            score: x + width - 520.0,
            delta: x + width - 300.0,
            rank_delta: x + width - 170.0,
            active: x + width - 32.0,
        }
    }
}

#[cfg(feature = "skia")]
struct RankingCanvas {
    surface: skia_safe::Surface,
    typeface: skia_safe::Typeface,
}

#[cfg(feature = "skia")]
impl RankingCanvas {
    fn new(width: u32, height: u32) -> Result<Self, RenderError> {
        let surface = skia_safe::surfaces::raster_n32_premul((width as i32, height as i32))
            .ok_or_else(|| RenderError::Render("创建排行榜 Surface 失败".to_string()))?;
        let typeface = ranking_typeface()
            .ok_or_else(|| RenderError::Render("排行榜字体初始化失败".to_string()))?;
        Ok(Self { surface, typeface })
    }

    fn background(&mut self) {
        let width = self.surface.width() as f32;
        let canvas = self.surface.canvas();
        canvas.clear(Color::BACKGROUND.skia());
        let mut paint = skia_safe::Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(Color::NIIGO.skia());
        canvas.draw_rect(
            skia_safe::Rect::from_xywh(0.0, 0.0, width * 0.34, 4.0),
            &paint,
        );
        paint.set_color(Color::MIKU.skia());
        canvas.draw_rect(
            skia_safe::Rect::from_xywh(width * 0.34, 0.0, width * 0.18, 4.0),
            &paint,
        );
    }

    fn panel(&mut self, x: f32, y: f32, width: f32, height: f32, color: Color) {
        let canvas = self.surface.canvas();
        let rect = skia_safe::Rect::from_xywh(x, y, width, height);
        let mut paint = skia_safe::Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(skia_safe::PaintStyle::Fill);
        paint.set_color(color.skia());
        canvas.draw_round_rect(rect, PANEL_RADIUS, PANEL_RADIUS, &paint);
    }

    fn glass(&mut self, x: f32, y: f32, width: f32, height: f32) {
        let canvas = self.surface.canvas();
        let shadow = skia_safe::Rect::from_xywh(x + 3.0, y + 4.0, width, height);
        let rect = skia_safe::Rect::from_xywh(x, y, width, height);
        let mut paint = skia_safe::Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(skia_safe::PaintStyle::Fill);
        paint.set_color(Color::BLACK_35.skia());
        canvas.draw_round_rect(shadow, PANEL_RADIUS + 2.0, PANEL_RADIUS + 2.0, &paint);
        paint.set_color(Color::GLASS.skia());
        canvas.draw_round_rect(rect, PANEL_RADIUS, PANEL_RADIUS, &paint);
        paint.set_style(skia_safe::PaintStyle::Stroke);
        paint.set_stroke_width(1.0);
        paint.set_color(Color::GLASS_EDGE.skia());
        canvas.draw_round_rect(rect, PANEL_RADIUS, PANEL_RADIUS, &paint);
    }

    fn rule(&mut self, x: f32, y: f32, width: f32, height: f32, color: Color) {
        let canvas = self.surface.canvas();
        let rect = skia_safe::Rect::from_xywh(x, y, width, height);
        let mut paint = skia_safe::Paint::default();
        paint.set_anti_alias(true);
        paint.set_style(skia_safe::PaintStyle::Fill);
        paint.set_color(color.skia());
        canvas.draw_round_rect(rect, height / 2.0, height / 2.0, &paint);
    }

    fn text(
        &mut self,
        text: &str,
        x: f32,
        baseline: f32,
        size: f32,
        color: Color,
        align: TextAlign,
        max_width: f32,
    ) {
        let font = skia_safe::Font::new(self.typeface.clone(), Some(size));
        let mut paint = skia_safe::Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(color.skia());
        let text = truncate_text(text, max_width, size);
        let width = measure_text(&font, &paint, &text);
        let draw_x = match align {
            TextAlign::Left => x,
            TextAlign::Center => x - width / 2.0,
            TextAlign::Right => x - width,
        };
        self.surface
            .canvas()
            .draw_str(text, (draw_x, baseline), &font, &paint);
    }

    fn text_fit(
        &mut self,
        text: &str,
        x: f32,
        baseline: f32,
        size: f32,
        min_size: f32,
        color: Color,
        align: TextAlign,
        max_width: f32,
    ) {
        let mut fitted = size;
        while fitted > min_size && estimate_text_width(text, fitted) > max_width {
            fitted -= 1.0;
        }
        self.text(text, x, baseline, fitted, color, align, max_width);
    }

    fn finish(mut self, quality: u8) -> Result<RenderOutput, RenderError> {
        let image = self.surface.image_snapshot();
        let width = image.width() as u32;
        let height = image.height() as u32;
        let encoded = image
            .encode(
                None,
                skia_safe::EncodedImageFormat::JPEG,
                Some(u32::from(quality.clamp(1, 100))),
            )
            .ok_or_else(|| RenderError::Encode("排行榜 JPEG 编码失败".to_string()))?;
        Ok(RenderOutput {
            data: encoded.as_bytes().to_vec(),
            content_type: "image/jpeg".to_string(),
            width,
            height,
            timing: None,
        })
    }
}

#[cfg(feature = "skia")]
#[derive(Debug, Clone, Copy)]
enum TextAlign {
    Left,
    Center,
    Right,
}

#[cfg(feature = "skia")]
#[derive(Debug, Clone, Copy)]
struct Color(u8, u8, u8, u8);

#[cfg(feature = "skia")]
impl Color {
    // void 色系（与 allium-web globals.css @theme 一致）
    const BACKGROUND: Self = Self(16, 15, 20, 255);   // --color-void-bg #100f14
    const SURFACE: Self = Self(30, 27, 36, 255);       // --color-void-surface #1e1b24
    const SURFACE_LIGHT: Self = Self(45, 41, 54, 255); // --color-void-surface-light #2d2936
    const TEXT: Self = Self(226, 232, 240, 255);        // --color-void-text #e2e8f0
    const MUTED: Self = Self(139, 136, 150, 255);      // --color-void-muted #8b8896
    const NIIGO: Self = Self(136, 68, 153, 255);       // --color-deep-purple #884499
    const MIKU: Self = Self(51, 204, 187, 255);        // --color-ethereal-teal #33ccbb
    const GOOD: Self = Self(121, 217, 157, 255);
    const WARN: Self = Self(244, 194, 99, 255);
    const BAD: Self = Self(238, 122, 122, 255);

    // 半透明派生（从 TEXT 色派生，不从纯白派生）
    const GLASS: Self = Self(226, 232, 240, 22);
    const GLASS_EDGE: Self = Self(226, 232, 240, 58);
    const PANEL_ALT: Self = Self(226, 232, 240, 28);
    const ROW: Self = Self(226, 232, 240, 17);
    const ROW_ALT: Self = Self(226, 232, 240, 10);
    const GOOD_BG: Self = Self(121, 217, 157, 44);
    const BAD_BG: Self = Self(238, 122, 122, 44);
    const BLACK_35: Self = Self(0, 0, 0, 90);

    fn skia(self) -> skia_safe::Color {
        skia_safe::Color::from_argb(self.3, self.0, self.1, self.2)
    }
}

#[cfg(feature = "skia")]
fn ranking_typeface() -> Option<skia_safe::Typeface> {
    let font_mgr = skia_safe::FontMgr::default();
    crate::text::resolve_custom_profile_typeface(
        &font_mgr,
        Some(crate::widgets::theme::fonts::PRIMARY),
    )
    .or_else(|| {
        crate::text::resolve_custom_profile_typeface(
            &font_mgr,
            Some(crate::widgets::theme::fonts::EMPHASIS),
        )
    })
    .or_else(|| font_mgr.match_family_style("Noto Sans CJK SC", skia_safe::FontStyle::default()))
    .or_else(|| font_mgr.match_family_style("Noto Sans CJK", skia_safe::FontStyle::default()))
    .or_else(|| font_mgr.legacy_make_typeface(None, skia_safe::FontStyle::default()))
}

#[cfg(feature = "skia")]
fn measure_text(font: &skia_safe::Font, paint: &skia_safe::Paint, text: &str) -> f32 {
    let (width, _) = font.measure_str(text, Some(paint));
    width
}

#[cfg(feature = "skia")]
fn truncate_text(text: &str, max_width: f32, font_size: f32) -> String {
    if estimate_text_width(text, font_size) <= max_width {
        return text.to_string();
    }
    let mut output = String::new();
    let ellipsis = "...";
    let mut used = estimate_text_width(ellipsis, font_size);
    for ch in text.chars() {
        let w = estimate_text_width(&ch.to_string(), font_size);
        if used + w > max_width {
            break;
        }
        output.push(ch);
        used += w;
    }
    output.push_str(ellipsis);
    output
}

#[cfg(feature = "skia")]
fn estimate_text_width(text: &str, font_size: f32) -> f32 {
    text.chars()
        .map(|ch| if ch.is_ascii() { 0.56 } else { 0.98 })
        .sum::<f32>()
        * font_size
}

fn display_character(character_id: i16) -> String {
    if character_id == 0 {
        "总榜".to_string()
    } else {
        format!("角色 {character_id}")
    }
}

#[cfg(feature = "skia")]
fn player_activity_status(parked: bool) -> (&'static str, Color) {
    if parked {
        ("不活跃", Color::WARN)
    } else {
        ("活跃", Color::GOOD)
    }
}

fn snapshot_meta(
    event_id: i32,
    event: &RankingEventMeta,
    character_id: i16,
    last_snapshot_ts: Option<i64>,
) -> String {
    snapshot_meta_lines(event_id, event, character_id, last_snapshot_ts).join(" · ")
}

fn snapshot_meta_lines(
    event_id: i32,
    event: &RankingEventMeta,
    character_id: i16,
    last_snapshot_ts: Option<i64>,
) -> Vec<String> {
    let event_name = event
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("活动 {event_id}"));
    let mut context = vec![display_character(character_id)];
    if let Some(remaining) = event.remaining_secs {
        context.push(display_remaining(remaining));
    }
    vec![
        event_name,
        context.join(" · "),
        display_ts(last_snapshot_ts),
    ]
}

fn display_ts(ts: Option<i64>) -> String {
    display_time(ts)
        .map(|value| format!("{value} 更新"))
        .unwrap_or_else(|| "时间未知".to_string())
}

fn display_time(ts: Option<i64>) -> Option<String> {
    let value = ts?;
    let utc = DateTime::<Utc>::from_timestamp(value, 0)?;
    let offset = FixedOffset::east_opt(8 * 3600)?;
    Some(
        utc.with_timezone(&offset)
            .format("%-m月%-d日 %H:%M:%S")
            .to_string(),
    )
}

fn display_time_ago(ts: Option<i64>) -> String {
    let Some(value) = ts else {
        return "--".to_string();
    };
    let now = Utc::now().timestamp();
    let diff = now - value;

    if diff < 0 {
        return "刚刚".to_string();
    }

    if diff < 60 {
        return format!("{}秒前", diff);
    }

    if diff < 3600 {
        return format!("{}分钟前", diff / 60);
    }

    if diff < 86400 {
        return format!("{}小时前", diff / 3600);
    }

    format!("{}天前", diff / 86400)
}

fn display_remaining(remaining_secs: i64) -> String {
    if remaining_secs <= 0 {
        return "已结束".to_string();
    }
    let days = remaining_secs / 86_400;
    let hours = (remaining_secs % 86_400) / 3_600;
    let minutes = (remaining_secs % 3_600) / 60;
    let seconds = remaining_secs % 60;
    if days > 0 {
        format!("剩余{days}天{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("剩余{hours:02}:{minutes:02}:{seconds:02}")
    }
}

fn display_hour(ts: i64) -> String {
    let Some(utc) = DateTime::<Utc>::from_timestamp(ts, 0) else {
        return "--时".to_string();
    };
    let Some(offset) = FixedOffset::east_opt(8 * 3600) else {
        return "--时".to_string();
    };
    utc.with_timezone(&offset).format("%H时").to_string()
}

#[cfg(feature = "skia")]
fn border_gap_cells(card: &RankingPlayerCard) -> Vec<GapCell> {
    let mut cells = Vec::new();
    if let Some(gap) = &card.border_gaps.upper {
        cells.push(GapCell {
            relation: "落后",
            gap_label: "还差",
            relation_color: Color::BAD,
            relation_bg: Color::BAD_BG,
            rank: format!("{}名", gap.rank),
            name: String::new(),
            score: gap.score,
            gap: gap.gap,
        });
    }
    if let Some(gap) = &card.border_gaps.lower {
        cells.push(GapCell {
            relation: "领先",
            gap_label: "领先",
            relation_color: Color::GOOD,
            relation_bg: Color::GOOD_BG,
            rank: format!("{}名", gap.rank),
            name: String::new(),
            score: gap.score,
            gap: gap.gap,
        });
    }
    cells
}

#[cfg(feature = "skia")]
fn neighbor_gap_cells(card: &RankingPlayerCard) -> Vec<GapCell> {
    let mut cells = Vec::new();
    if let Some(gap) = &card.neighbor_gaps.previous {
        cells.push(GapCell {
            relation: "落后",
            gap_label: "还差",
            relation_color: Color::BAD,
            relation_bg: Color::BAD_BG,
            rank: format!("{}名", gap.rank),
            name: gap.name.clone(),
            score: gap.score,
            gap: gap.gap,
        });
    }
    if let Some(gap) = &card.neighbor_gaps.next {
        cells.push(GapCell {
            relation: "领先",
            gap_label: "领先",
            relation_color: Color::GOOD,
            relation_bg: Color::GOOD_BG,
            rank: format!("{}名", gap.rank),
            name: gap.name.clone(),
            score: gap.score,
            gap: gap.gap,
        });
    }
    cells
}

fn format_duration(secs: i64) -> String {
    let secs = secs.max(0);
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    if hours > 0 {
        format!("{hours}时{minutes:02}分")
    } else {
        format!("{minutes}分")
    }
}

fn format_number(value: i64) -> String {
    format_w(value as f64)
}

fn format_float(value: f64) -> String {
    format_w(value)
}

fn format_hourly(value: f64) -> String {
    format!("{}/时", format_rate(value))
}

fn display_rate(value: f64, window_secs: i64) -> String {
    if window_secs < MIN_STABLE_RATE_WINDOW_SECONDS {
        "统计中".to_string()
    } else if window_secs < 60 * 60 {
        format!("约{}", format_rate(value))
    } else {
        format_rate(value)
    }
}

fn display_hourly_rate(value: f64, window_secs: i64) -> String {
    if window_secs < MIN_STABLE_RATE_WINDOW_SECONDS {
        "统计中".to_string()
    } else if window_secs < 60 * 60 {
        format!("约{}", format_hourly(value))
    } else {
        format_hourly(value)
    }
}

/// 显示日速（PT/day），fallback 时标注"时速参考"。
fn display_daily_rate(value: f64, window_secs: i64, is_fallback: bool) -> String {
    let base = if window_secs < MIN_STABLE_RATE_WINDOW_SECONDS {
        "统计中".to_string()
    } else if window_secs < 60 * 60 {
        format!("约{}/日", format_rate(value))
    } else {
        format!("{}/日", format_rate(value))
    };
    if is_fallback {
        format!("{}*", base)
    } else {
        base
    }
}

fn format_window(secs: i64) -> String {
    if secs <= 0 {
        return "--".to_string();
    }
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    if hours > 0 {
        format!("{hours}h{minutes:02}m")
    } else {
        format!("{minutes}m")
    }
}

fn format_plays(plays: u32) -> String {
    format!("{plays}轮")
}

fn format_rate(value: f64) -> String {
    format_w(value)
}

fn format_w(value: f64) -> String {
    let wan = value / 10_000.0;
    let formatted = format!("{wan:.3}");
    if formatted == "-0.000" {
        "0.000w".to_string()
    } else {
        format!("{formatted}w")
    }
}

fn format_signed(value: i64) -> String {
    if value > 0 {
        format!("+{}", format_number(value))
    } else {
        format_number(value)
    }
}

fn display_rank_delta(value: i32) -> String {
    match value.cmp(&0) {
        std::cmp::Ordering::Less => format!("↑{}", value.abs()),
        std::cmp::Ordering::Greater => format!("↓{value}"),
        std::cmp::Ordering::Equal => "0".to_string(),
    }
}

fn display_rank_delta_1h(value: i32, estimated: bool, window_secs: i64) -> String {
    if window_secs < MIN_STABLE_RATE_WINDOW_SECONDS {
        return "统计中".to_string();
    }
    let value = display_rank_delta(value);
    if estimated && value != "0" {
        format!("约{value}")
    } else {
        value
    }
}

#[cfg(feature = "skia")]
fn delta_color(value: i64) -> Color {
    if value > 0 {
        Color::GOOD
    } else if value < 0 {
        Color::BAD
    } else {
        Color::MUTED
    }
}

#[cfg(feature = "skia")]
fn rate_color(value: f64, window_secs: i64) -> Color {
    if window_secs < MIN_STABLE_RATE_WINDOW_SECONDS {
        Color::MUTED
    } else {
        delta_color(value.round() as i64)
    }
}

#[cfg(feature = "skia")]
fn rank_delta_color(value: i32) -> Color {
    if value < 0 {
        Color::GOOD
    } else if value > 0 {
        Color::BAD
    } else {
        Color::MUTED
    }
}

#[cfg(not(feature = "skia"))]
fn placeholder_output(width: u32, height: u32) -> RenderOutput {
    RenderOutput {
        data: placeholder_jpeg().to_vec(),
        content_type: "image/jpeg".to_string(),
        width,
        height,
        timing: None,
    }
}

#[cfg(not(feature = "skia"))]
fn placeholder_jpeg() -> &'static [u8] {
    &[
        0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01, 0x01, 0x00,
        0x48, 0x00, 0x48, 0x00, 0x00, 0xff, 0xdb, 0x00, 0x43, 0x00, 0x08, 0x06, 0x06, 0x07, 0x06,
        0x05, 0x08, 0x07, 0x07, 0x07, 0x09, 0x09, 0x08, 0x0a, 0x0c, 0x14, 0x0d, 0x0c, 0x0b, 0x0b,
        0x0c, 0x19, 0x12, 0x13, 0x0f, 0x14, 0x1d, 0x1a, 0x1f, 0x1e, 0x1d, 0x1a, 0x1c, 0x1c, 0x20,
        0x24, 0x2e, 0x27, 0x20, 0x22, 0x2c, 0x23, 0x1c, 0x1c, 0x28, 0x37, 0x29, 0x2c, 0x30, 0x31,
        0x34, 0x34, 0x34, 0x1f, 0x27, 0x39, 0x3d, 0x38, 0x32, 0x3c, 0x2e, 0x33, 0x34, 0x32, 0xff,
        0xc0, 0x00, 0x0b, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11, 0x00, 0xff, 0xc4, 0x00,
        0x14, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0xff, 0xc4, 0x00, 0x14, 0x10, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xda, 0x00, 0x08,
        0x01, 0x01, 0x00, 0x00, 0x3f, 0x00, 0xd2, 0xcf, 0x20, 0xff, 0xd9,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(rank: i32, name: &str) -> RankingPlayerRecord {
        RankingPlayerRecord {
            user_id: rank as i64,
            character_id: 0,
            rank,
            score: 1_000_000 - rank as i64 * 1000,
            name: name.to_string(),
            score_delta: 1200,
            rank_delta: -1,
            hourly_rate: 36_000.0,
            tracking_window_secs: 3600,
            parked: false,
            rank_delta_1h: -3,
            rank_delta_1h_estimated: false,
            last_active_ts: 1_710_000_000,
        }
    }

    fn card(rank: i32, name: &str) -> RankingPlayerCard {
        RankingPlayerCard {
            record: player(rank, name),
            metrics: RankingPlayerMetrics {
                average_recent_pt: 1200.0,
                hourly_rate: 36_000.0,
                tracking_window_secs: 3600,
                projected_20m_x3: 42_000.0,
                plays_this_hour: 12,
                hourly_plays: Vec::new(),
                consecutive_play_secs: 3600,
                parked: false,
                rank_delta_1h: -3,
                rank_delta_1h_estimated: false,
            },
            border_gaps: RankingBorderGaps {
                upper: Some(RankingBorderGap {
                    rank: 10,
                    score: 990_000,
                    gap: 10_000,
                }),
                lower: Some(RankingBorderGap {
                    rank: 50,
                    score: 900_000,
                    gap: 80_000,
                }),
            },
            neighbor_gaps: RankingNeighborGaps {
                previous: Some(RankingNeighborGap {
                    rank: rank - 1,
                    name: "前一名".to_string(),
                    score: 980_000,
                    gap: 1000,
                }),
                next: Some(RankingNeighborGap {
                    rank: rank + 1,
                    name: "后一名".to_string(),
                    score: 978_000,
                    gap: 1000,
                }),
            },
        }
    }

    #[test]
    fn formats_signed_rank_delta() {
        assert_eq!(display_rank_delta(-3), "↑3");
        assert_eq!(display_rank_delta(2), "↓2");
        assert_eq!(display_rank_delta(0), "0");
    }

    #[test]
    fn formats_event_meta_with_remaining_time() {
        let meta = snapshot_meta(
            133,
            &RankingEventMeta {
                name: Some("测试活动".to_string()),
                remaining_secs: Some(90_061),
            },
            0,
            Some(1_710_000_000),
        );
        assert!(meta.contains("测试活动"));
        assert!(meta.contains("总榜"));
        assert!(meta.contains("剩余1天01:01:01"));
        assert!(meta.ends_with("更新"));
    }

    #[test]
    fn formats_rate_in_w_unit() {
        assert_eq!(format_rate(0.0), "0.000w");
        assert_eq!(format_rate(36_000.0), "3.600w");
        assert_eq!(format_rate(125_000.0), "12.500w");
        assert_eq!(format_hourly(36_000.0), "3.600w/时");
        assert_eq!(display_rate(36_000.0, 60), "统计中");
        assert_eq!(display_hourly_rate(36_000.0, 1800), "约3.600w/时");
    }

    #[cfg(feature = "skia")]
    #[test]
    fn renders_ranking_outputs() {
        let assets = AssetStore::new(16);
        let board = RankingRenderInput::LiveBoard(RankingLiveBoardInput {
            event_id: 133,
            event: RankingEventMeta {
                name: Some("测试活动".to_string()),
                remaining_secs: Some(86_400),
            },
            character_id: 0,
            snapshot_seq: 7,
            last_snapshot_ts: Some(1_710_000_000),
            phase: "采集中".to_string(),
            page: 1,
            page_size: 100,
            total: 2,
            compact_top100: true,
            players: vec![player(1, "测试玩家一"), player(2, "测试玩家二")],
            borders: vec![RankingBorderRecord {
                rank: 100,
                score: 800_000,
                character_id: 0,
                score_delta: 1000,
                hourly_rate: 12_000.0,
                tracking_window_secs: 3600,
            }],
        });
        let output = render_ranking(&board, &assets).expect("board render");
        assert_eq!(output.content_type, "image/jpeg");
        assert!(output.data.len() > 10_000);

        let batch = RankingRenderInput::PlayerBatch(RankingPlayerBatchInput {
            event_id: 133,
            event: RankingEventMeta {
                name: Some("测试活动".to_string()),
                remaining_secs: Some(86_400),
            },
            character_id: 0,
            snapshot_seq: 7,
            last_snapshot_ts: Some(1_710_000_000),
            phase: "采集中".to_string(),
            players: vec![card(11, "测试玩家一"), card(12, "测试玩家二")],
        });
        let output = render_ranking(&batch, &assets).expect("batch render");
        assert_eq!(output.content_type, "image/jpeg");
        assert!(output.data.len() > 10_000);
    }
}
