//! 组卡推荐结果渲染卡。
//!
//! 采用 WidgetNode 树，视觉结构参考旧 bot HTML 模板：
//! 左侧 Rank + 目标指标，右侧卡组数据芯片 + 固有指标块，显式展示卡牌养成与前后篇状态。

use crate::widget_node::{
    CanvasSpec, ChildEntry, Layout, OutputFormat, Position, TextAlignValue, VAlignValue,
    WidgetDocument, WidgetNode,
};
use crate::widgets::card_util::{rarity_suffix, star_icon_key};
use crate::widgets::image::AssetImageFit;
use crate::widgets::text::ASCENT_RATIO;
use crate::widgets::theme::Color;

mod layout {
    pub mod page {
        pub const CANVAS_W: u32 = 1280;
        pub const PAD: f32 = 40.0;
        pub const CONTENT_W: f32 = 1200.0;
        pub const SECTION_GAP: f32 = 44.0;
        pub const OUTPUT_QUALITY: u8 = 92;
    }

    pub mod header {
        pub const H: f32 = 258.0;
        pub const RULE_Y: f32 = 0.0;
        pub const RULE_H: f32 = 2.0;
        pub const MUSIC_X: f32 = 0.0;
        pub const MUSIC_Y: f32 = 152.0;
        pub const PROTOCOL_BAR_X: f32 = 0.0;
        pub const PROTOCOL_BAR_Y: f32 = 32.0;
        pub const PROTOCOL_BAR_W: f32 = 96.0;
        pub const PROTOCOL_BAR_H: f32 = 7.0;
        pub const PROTOCOL_X: f32 = 112.0;
        pub const PROTOCOL_Y: f32 = 42.0;
        pub const TITLE_X: f32 = 0.0;
        pub const TITLE_Y: f32 = 118.0;
        pub const EVENT_X: f32 = 410.0;
        pub const EVENT_Y: f32 = 152.0;
        pub const USER_X: f32 = 790.0;
        pub const USER_Y: f32 = 82.0;
    }

    /// 参数摘要 strip：header 与首行卡组之间的独立全宽条，展示本次请求用了哪些参数。
    /// 仅当有参数摘要时插入；不插入时整体布局与旧版逐像素一致（不影响无参数请求）。
    pub mod param_strip {
        pub const H: f32 = 44.0;
        pub const PAD_X: f32 = 20.0;
        pub const LABEL_Y: f32 = 29.0;
        pub const GAP_AFTER: f32 = 20.0;
    }

    pub mod event {
        pub const W: f32 = 350.0;
        pub const H: f32 = 92.0;
        pub const PAD_X: f32 = 16.0;
        pub const LABEL_Y: f32 = 26.0;
        pub const NAME_Y: f32 = 56.0;
        pub const ID_Y: f32 = 78.0;
    }

    pub mod music {
        pub const W: f32 = 390.0;
        pub const H: f32 = 92.0;
        pub const PAD: f32 = 14.0;
        pub const JACKET: f32 = 64.0;
        pub const TEXT_WITH_JACKET_X: f32 = 94.0;
        pub const TEXT_NO_JACKET_X: f32 = 16.0;
        pub const TITLE_Y: f32 = 37.0;
        pub const DIFFICULTY_Y: f32 = 66.0;
    }

    pub mod user {
        pub const W: f32 = 410.0;
        pub const H: f32 = 104.0;
        pub const PAD_X: f32 = 20.0;
        pub const ACCENT_H: f32 = 4.0;
        pub const AVATAR_X: f32 = 18.0;
        pub const AVATAR_Y: f32 = 20.0;
        pub const AVATAR: f32 = 64.0;
        pub const TEXT_WITH_AVATAR_X: f32 = 102.0;
        pub const LABEL_Y: f32 = 30.0;
        pub const NAME_Y: f32 = 58.0;
        pub const ID_Y: f32 = 84.0;
    }

    pub mod row {
        use super::page;

        pub const H: f32 = 260.0;
        pub const GAP: f32 = 24.0;
        pub const RANK_W: f32 = 154.0;
        pub const RANK_GAP: f32 = 24.0;
        pub const DECK_W: f32 = page::CONTENT_W - RANK_W - RANK_GAP;
        pub const SHADOW_OFFSET: f32 = 8.0;
        pub const INNER_PAD_X: f32 = 28.0;
        pub const INNER_PAD_Y: f32 = 15.0;
    }

    pub mod rank {
        pub const NUMBER_Y: f32 = 92.0;
        pub const MARK_X: f32 = 56.0;
        pub const MARK_Y: f32 = 110.0;
        pub const MARK_W: f32 = 48.0;
        pub const MARK_H: f32 = 6.0;
        pub const DIVIDER_X: f32 = 30.0;
        pub const DIVIDER_Y: f32 = 164.0;
        pub const DIVIDER_W: f32 = 100.0;
        pub const DIVIDER_H: f32 = 1.0;
        pub const LABEL_Y: f32 = 182.0;
        pub const VALUE_Y: f32 = 218.0;
    }

    pub mod card {
        pub const W: f32 = 154.0;
        pub const H: f32 = 230.0;
        pub const IMAGE: f32 = 144.0;
        pub const GAP: f32 = 7.0;
        pub const PAD: f32 = 5.0;
        pub const ID_X: f32 = 100.0;
        pub const ID_Y: f32 = 8.0;
        pub const SKILL_Y: f32 = 154.0;
        pub const STATUS_Y: f32 = 181.0;
        pub const PROGRESS_Y: f32 = 222.0;
        pub const CONTENT_X: f32 = 7.0;
        pub const STATUS_GAP: f32 = 4.0;
        pub const ID_W: f32 = 48.0;
        pub const SKILL_W: f32 = 140.0;
        pub const BONUS_W: f32 = 66.0;
        pub const EPISODE_W: f32 = 33.0;
        pub const BADGE_H: f32 = 23.0;
        pub const STATUS_H: f32 = 23.0;
        pub const PROGRESS_W: f32 = 140.0;
        pub const PROGRESS_H: f32 = 4.0;
    }

    pub mod stats {
        use super::row;

        pub const W: f32 = 160.0;
        pub const X: f32 = row::DECK_W - W - row::INNER_PAD_X;
        pub const BLOCK_H: f32 = (row::H - row::INNER_PAD_Y * 2.0) / 3.0;
        pub const BLOCK_GAP: f32 = 0.0;
        pub const LABEL_X: f32 = 12.0;
        pub const LABEL_Y: f32 = 17.0;
        pub const VALUE_PAD_R: f32 = 12.0;
        pub const VALUE_Y: f32 = 56.0;
    }

    pub mod footer {
        pub const H: f32 = 154.0;
        pub const PAD_X: f32 = 26.0;
        pub const TITLE_X: f32 = 26.0;
        pub const TITLE_Y: f32 = 28.0;
        pub const TOTAL_X: f32 = 212.0;
        pub const TOTAL_Y: f32 = 36.0;
        pub const TOTAL_DETAIL_X: f32 = 442.0;
        pub const META_X: f32 = 1016.0;
        pub const META_Y: f32 = 29.0;
        pub const STAGE_Y: f32 = 60.0;
        pub const STAGE_H: f32 = 76.0;
        pub const STAGE_GAP: f32 = 10.0;
        pub const STAGE_PAD_X: f32 = 14.0;
        pub const STAGE_LABEL_Y: f32 = 20.0;
        pub const STAGE_VALUE_Y: f32 = 48.0;
        pub const STAGE_DETAIL_Y: f32 = 68.0;
    }
}

mod type_size {
    pub const PROTOCOL: f32 = 18.0;
    pub const TITLE: f32 = 64.0;
    pub const MUSIC_TITLE: f32 = 24.0;
    pub const EVENT_LABEL: f32 = 12.0;
    pub const EVENT_NAME: f32 = 18.0;
    pub const EVENT_ID: f32 = 13.0;
    pub const USER_LABEL: f32 = 13.0;
    pub const USER_NAME: f32 = 22.0;
    pub const USER_ID: f32 = 14.0;
    pub const RANK: f32 = 56.0;
    pub const RANK_LABEL: f32 = 15.0;
    pub const RANK_VALUE: f32 = 24.0;
    pub const STAT_LABEL: f32 = 12.0;
    pub const STAT_VALUE: f32 = 31.0;
    pub const CARD_EFFECT: f32 = 12.5;
    pub const CARD_BONUS: f32 = 12.0;
    pub const EPISODE: f32 = 11.5;
    pub const FOOTER_TITLE: f32 = 15.0;
    pub const FOOTER_TOTAL: f32 = 32.0;
    pub const FOOTER_META: f32 = 13.0;
    pub const FOOTER_LABEL: f32 = 12.0;
    pub const FOOTER_VALUE: f32 = 23.0;
    pub const FOOTER_DETAIL: f32 = 10.5;
    pub const PARAM_LABEL: f32 = 13.0;
    pub const PARAM_VALUE: f32 = 16.0;
}

mod pal {
    use super::Color;

    pub const BG: Color = Color::from_rgba8(33, 33, 43, 255);
    pub const BLACK_35: Color = Color::new(0.0, 0.0, 0.0, 0.35);
    pub const BLACK_55: Color = Color::new(0.0, 0.0, 0.0, 0.55);
    pub const BLACK_70: Color = Color::new(0.0, 0.0, 0.0, 0.70);
    pub const WHITE: Color = Color::new(1.0, 1.0, 1.0, 1.0);
    pub const WHITE_08: Color = Color::new(1.0, 1.0, 1.0, 0.08);
    pub const WHITE_16: Color = Color::new(1.0, 1.0, 1.0, 0.16);
    pub const WHITE_24: Color = Color::new(1.0, 1.0, 1.0, 0.24);
    pub const CHIP: Color = Color::from_rgba8(245, 247, 250, 255);
    pub const CHIP_LINE: Color = Color::from_rgba8(224, 228, 235, 255);
    pub const GRAY: Color = Color::from_rgba8(205, 209, 220, 255);
    pub const DARK_TEXT: Color = Color::from_rgba8(70, 70, 82, 255);
    pub const NIIGO: Color = Color::from_rgba8(136, 122, 240, 255);
    pub const MIKU: Color = Color::from_rgba8(168, 216, 232, 255);
    pub const MIKU_SOFT: Color = Color::from_rgba8(168, 216, 232, 42);
    pub const BONUS: Color = Color::from_rgba8(255, 159, 67, 255);
    pub const BONUS_SOFT: Color = Color::from_rgba8(255, 159, 67, 52);
    pub const GOLD: Color = Color::from_rgba8(255, 213, 100, 255);
    pub const GREEN: Color = Color::from_rgba8(34, 139, 83, 255);
    pub const GREEN_BG: Color = Color::from_rgba8(220, 252, 231, 255);
    pub const RED: Color = Color::from_rgba8(153, 27, 27, 255);
    pub const RED_BG: Color = Color::from_rgba8(254, 226, 226, 255);
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct DeckRenderCard {
    pub card_id: i32,
    pub asset_key: String,
    pub rarity: String,
    pub attr: String,
    pub level: i32,
    pub skill_level: i32,
    pub skill_score_up: f64,
    pub event_bonus: Option<f64>,
    pub master_rank: i32,
    pub trained: bool,
    pub episode1_read: bool,
    pub episode2_read: bool,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct DeckRenderUnit {
    pub rank: usize,
    pub cards: Vec<DeckRenderCard>,
    pub total_power: i32,
    pub live_score: i32,
    pub event_point: Option<i32>,
    #[serde(default)]
    pub target_value: Option<i64>,
    pub skill_score: f64,
    pub multi_live_score_up: Option<f64>,
    pub event_bonus_total: Option<f64>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct DeckResultHeader {
    pub event_id: Option<i32>,
    pub event_name: Option<String>,
    pub event_banner_key: Option<String>,
    pub recommend_type: Option<String>,
    pub target: Option<String>,
    pub music_title: Option<String>,
    pub music_jacket_key: Option<String>,
    pub difficulty: Option<String>,
    pub user_name: Option<String>,
    pub user_id: Option<String>,
    pub user_avatar_key: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Hash)]
pub struct DeckTimingStage {
    pub label: String,
    pub value: String,
    pub detail: Option<String>,
    pub tone: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct DeckResultCard {
    pub header: Option<DeckResultHeader>,
    pub decks: Vec<DeckRenderUnit>,
    pub cost_info: Option<String>,
    pub algorithm_info: Option<String>,
    /// 本次请求使用的参数摘要（顶配/养成/火数/歌曲/固定卡等），渲染为 header 与卡组间的一条 strip。
    #[serde(default)]
    pub param_summary: Option<String>,
    #[serde(default)]
    pub timing_lines: Vec<String>,
    #[serde(default)]
    pub timing_stages: Vec<DeckTimingStage>,
    #[serde(default)]
    pub output_quality: Option<u8>,
}

/// 组卡结果卡面缩略图当前设计尺寸。
pub const DECK_CARD_THUMBNAIL_SIZE: f32 = layout::card::IMAGE;

pub fn deck_result_glass_specs() -> Vec<(f32, f32, f32)> {
    let mut specs = vec![(layout::user::W, layout::user::H, 0.10)];
    specs.push((layout::page::CONTENT_W, layout::footer::H, 0.16));
    for rank in 1..=5 {
        let is_top = rank == 1;
        let rank_variance = glass_variance(rank, if is_top { 0.34 } else { 0.24 }, 0.035);
        let body_variance = glass_variance(rank, if is_top { 0.28 } else { 0.18 }, 0.030);
        let shadow_variance = glass_variance(rank + 5, 0.22, 0.025);
        specs.push((
            layout::row::RANK_W,
            layout::row::H - layout::row::SHADOW_OFFSET,
            shadow_variance + 0.06,
        ));
        specs.push((
            layout::row::DECK_W,
            layout::row::H - layout::row::SHADOW_OFFSET,
            shadow_variance,
        ));
        specs.push((layout::row::RANK_W, layout::row::H, rank_variance));
        specs.push((layout::row::DECK_W, layout::row::H, body_variance));
    }
    specs
}

impl DeckResultCard {
    pub fn all_asset_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        for deck in &self.decks {
            for card in &deck.cards {
                keys.push(card.asset_key.clone());
                if card.level > 0 {
                    keys.push("card/bg_base_wh".into());
                    keys.push(format!("card/cardFrame_S_{}", rarity_suffix(&card.rarity)));
                    keys.push(format!("card/icon_attribute_{}_64", card.attr));
                    keys.push(star_icon_key(&card.rarity, card.trained).into());
                    keys.push(format!(
                        "card/masterRank_S_{}",
                        card.master_rank.clamp(0, 5)
                    ));
                }
            }
        }
        if let Some(ref h) = &self.header {
            if let Some(ref k) = h.music_jacket_key {
                keys.push(k.clone());
            }
            if let Some(ref k) = h.event_banner_key {
                keys.push(k.clone());
            }
            if let Some(ref k) = h.user_avatar_key {
                keys.push(k.clone());
            }
        }
        keys.sort();
        keys.dedup();
        keys
    }

    pub fn to_widget_document(&self) -> WidgetDocument {
        let shown = self.decks.iter().take(5).collect::<Vec<_>>();
        let n = shown.len().max(1);
        let body_h = n as f32 * layout::row::H + (n.saturating_sub(1)) as f32 * layout::row::GAP;
        // 参数摘要 strip 只在有摘要时占高度；无摘要时整体布局与旧版完全一致。
        let param_summary = self
            .param_summary
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let param_strip_h = if param_summary.is_some() {
            layout::param_strip::H + layout::param_strip::GAP_AFTER
        } else {
            0.0
        };
        let h = layout::page::PAD
            + layout::header::H
            + layout::page::SECTION_GAP
            + param_strip_h
            + body_h
            + layout::page::SECTION_GAP
            + layout::footer::H
            + layout::page::PAD;
        let max_skill = self
            .decks
            .iter()
            .filter_map(|d| d.multi_live_score_up)
            .fold(0.0_f64, f64::max);

        let mut children = background_debris(h);
        let mut y = layout::page::PAD;
        children.push(entry(layout::page::PAD, y, header(self.header.as_ref())));
        y += layout::header::H + layout::page::SECTION_GAP;
        // 参数摘要 strip：展示本次组卡实际生效的参数（养成/顶配/火数/歌曲等），
        // 让用户在结果图上确认用了哪些参数。仅在有摘要时插入，不影响旧版布局。
        if let Some(summary) = param_summary {
            children.push(entry(
                layout::page::PAD,
                y,
                param_strip(summary),
            ));
            y += param_strip_h;
        }
        let target = resolve_target(self.header.as_ref());

        for (i, deck) in shown.iter().enumerate() {
            children.push(entry(
                layout::page::PAD,
                y,
                deck_panel(deck, i, max_skill, target),
            ));
            y += layout::row::H + layout::row::GAP;
        }

        y = y - layout::row::GAP + layout::page::SECTION_GAP;
        children.push(entry(
            layout::page::PAD,
            y,
            footer(
                self.cost_info.as_deref(),
                self.algorithm_info.as_deref(),
                &self.timing_lines,
                &self.timing_stages,
            ),
        ));

        WidgetDocument {
            version: 1,
            canvas: CanvasSpec {
                width: layout::page::CANVAS_W,
                height: h.ceil() as u32,
                background: pal::BG,
            },
            root: WidgetNode::Container {
                id: "root".into(),
                layout: Layout::Absolute,
                children,
            },
            output: OutputFormat::Jpeg(self.output_quality.unwrap_or(layout::page::OUTPUT_QUALITY)),
        }
    }
}

/// 构造 ChildEntry。
///
/// **deck baseline 适配**：当 `node` 是 SimpleText 时，`(x, y)` 被解释为 **(锚点 x, baseline y)**
/// （deck_result.rs 的 35 个 `_Y` 常量都是 baseline 语义，x 锚点按 align 不同含义不同）。
/// 内部把锚点坐标转换为 SimpleText 盒子的 (top_left x, top y)。
///
/// 前提：`text_align()` 构造的 SimpleText 固定 `padding=0`、`line_height=1.0`、`v_align=Top`，
/// 此时 baseline 距盒子顶部偏移恒为 `fs * ASCENT_RATIO`，文字宽度在水平定位公式中代数抵消，
/// 不需要测量文本宽度。`debug_assert` 保证契约。
fn entry(x: f32, y: f32, node: WidgetNode) -> ChildEntry {
    let position = match &node {
        WidgetNode::SimpleText {
            font_size,
            align,
            width,
            padding,
            line_height,
            ..
        } => {
            // 契约：deck 内部只通过 text_align() 构造 SimpleText，固定 padding=0、line_height=1.0。
            // 这两个值是下面 text_w 抵消推导的前提，违反则 Center/Right 锚点会偏移。
            debug_assert_eq!(
                *padding, 0.0,
                "deck entry() baseline 换算前提：padding 必须为 0（由 text_align() 保证）"
            );

            // SimpleText baseline 在盒内 y 位置（与 widgets/text.rs draw 算法一致）
            let line_box_h = font_size * line_height;
            let text_top_in_line = (line_box_h - font_size) / 2.0;
            let baseline_in_line = text_top_in_line + font_size * ASCENT_RATIO;
            let top_y = y - padding - baseline_in_line;

            // 水平锚点换算：旧 baseline API 的 Left/Center/Right 是相对锚点 x 的字符串定位。
            // 新 box 模型中文字位置 = inner_x + 偏移（取决于 box 内 align）。
            //
            // 关键观察：当 padding=0 时，text_w 项在公式中代数上抵消：
            //   Left:   top_x = x
            //   Center: top_x = (x - text_w/2) - (inner_w - text_w)/2 = x - inner_w/2
            //   Right:  top_x = (x - text_w)   - (inner_w - text_w)   = x - inner_w
            // 所以**完全不需要测量文字宽度**——deck box 几何就足够定位 baseline 锚点。
            let inner_w = width - 2.0 * padding;
            let top_x = match align {
                TextAlignValue::Left => x,
                TextAlignValue::Center => x - inner_w / 2.0,
                TextAlignValue::Right => x - inner_w,
            };
            pos(top_x, top_y)
        }
        _ => pos(x, y),
    };
    ChildEntry {
        position,
        node,
        visible: None,
    }
}

fn auto(node: WidgetNode) -> ChildEntry {
    ChildEntry {
        position: None,
        node,
        visible: None,
    }
}

fn pos(x: f32, y: f32) -> Option<Position> {
    Some(Position {
        x,
        y,
        rotation: 0.0,
        scale: (1.0, 1.0),
    })
}

/// 参数摘要 strip：header 与首行卡组之间的全宽玻璃条，左侧标签「参数」+ 右侧摘要文本。
/// 展示本次组卡实际生效的参数（养成/顶配/火数/歌曲等），让用户在结果图上确认。
fn param_strip(summary: &str) -> WidgetNode {
    WidgetNode::Container {
        id: "param_strip".into(),
        layout: Layout::Absolute,
        children: vec![
            entry(
                0.0,
                0.0,
                glass("param_strip_glass", layout::page::CONTENT_W, layout::param_strip::H, 0.12),
            ),
            entry(
                0.0,
                0.0,
                panel(
                    "param_strip_tint",
                    layout::page::CONTENT_W,
                    layout::param_strip::H,
                    8.0,
                    pal::BLACK_35,
                    Some(pal::WHITE_16),
                    1.0,
                ),
            ),
            entry(
                layout::param_strip::PAD_X,
                layout::param_strip::LABEL_Y,
                text(
                    "param_strip_label",
                    "参数",
                    type_size::PARAM_LABEL,
                    pal::NIIGO,
                ),
            ),
            entry(
                layout::param_strip::PAD_X + 64.0,
                layout::param_strip::LABEL_Y,
                text(
                    "param_strip_value",
                    summary,
                    type_size::PARAM_VALUE,
                    pal::GRAY,
                ),
            ),
        ],
    }
}

fn glass(id: &str, w: f32, h: f32, variance: f32) -> WidgetNode {
    WidgetNode::GlassPanel {
        id: id.into(),
        width: w,
        height: h,
        clip_variance: variance,
    }
}

fn panel(
    id: &str,
    w: f32,
    h: f32,
    radius: f32,
    fill: Color,
    border: Option<Color>,
    border_width: f32,
) -> WidgetNode {
    WidgetNode::Panel {
        id: id.into(),
        width: w,
        height: h,
        radius,
        fill,
        border,
        border_width,
    }
}

fn text(id: &str, content: impl Into<String>, size: f32, color: Color) -> WidgetNode {
    text_align(id, content, size, color, TextAlignValue::Left, false)
}

fn text_align(
    id: &str,
    content: impl Into<String>,
    size: f32,
    color: Color,
    align: TextAlignValue,
    glow: bool,
) -> WidgetNode {
    let content = content.into();
    // deck_result 使用扁平 box：
    //   - width = 9999.O（极大值）。Horizontal 布局中 measure() 返回估算文字实际宽度，
    //     不再用 9999 参与 layout 累加，避免兄弟节点被推飞。Absolute 布局下 entry() 的
    //     baseline 换算公式（padding=0 时 text_w 抵消）只用 width 本身，定位仍然一致。
    //     draw() 的 clip_rect 使用 self.width=9999 → 不会裁切文字右侧。
    //   - height = size * 2 = 2 倍字号，足够包住 line_box_h = fs，避免上下被裁。
    //   - padding = 0, line_height = 1.0, v_align = Top：配合 entry() 的 baseline → top
    //     自动换算（详见 entry() 注释），保持历史 baseline 语义不变，业务代码零修改。
    let width = 9999.0_f32;
    WidgetNode::SimpleText {
        id: id.into(),
        content,
        font_size: size,
        color,
        width,
        height: size * 2.0,
        align,
        v_align: VAlignValue::Top,
        padding: 0.0,
        line_height: 1.0,
        glow,
    }
}

fn image(id: &str, key: &str, w: f32, h: f32, fit: AssetImageFit, radius: f32) -> WidgetNode {
    WidgetNode::AssetImage {
        id: id.into(),
        asset_key: key.into(),
        width: w,
        height: h,
        fit,
        radius,
    }
}

#[derive(Clone, Copy)]
enum ResultTarget {
    EventPoint,
    Power,
    Skill,
    Score,
    Mysekai,
}

fn resolve_target(header: Option<&DeckResultHeader>) -> ResultTarget {
    match header
        .and_then(|h| h.target.as_deref())
        .unwrap_or("score")
        .to_ascii_lowercase()
        .as_str()
    {
        "ep" | "event_point" | "eventpoint" | "point" => ResultTarget::EventPoint,
        "power" => ResultTarget::Power,
        "skill" => ResultTarget::Skill,
        "mysekai" | "mysekai_point" | "mysekai_internal" => ResultTarget::Mysekai,
        "score" => {
            if header
                .and_then(|h| h.recommend_type.as_deref())
                .is_some_and(|t| matches!(t, "event" | "wl" | "wl_fake" | "unit_attr"))
            {
                ResultTarget::EventPoint
            } else {
                ResultTarget::Score
            }
        }
        _ => ResultTarget::Score,
    }
}

fn target_metric(deck: &DeckRenderUnit, target: ResultTarget) -> (&'static str, String, Color) {
    match target {
        ResultTarget::EventPoint => (
            "活动点数",
            deck.event_point
                .map(fmt)
                .unwrap_or_else(|| fmt(deck.live_score)),
            pal::BONUS,
        ),
        ResultTarget::Power => ("综合力", fmt(deck.total_power), pal::GOLD),
        ResultTarget::Skill => (
            "技能实效",
            deck.multi_live_score_up
                .map(|v| format!("+{}", fmt_pct(v)))
                .unwrap_or_else(|| format!("{:.1}", deck.skill_score)),
            pal::MIKU,
        ),
        ResultTarget::Score => ("分数", fmt(deck.live_score), pal::WHITE),
        ResultTarget::Mysekai => (
            "烤森Pt",
            deck.target_value
                .map(fmt_i64)
                .unwrap_or_else(|| fmt(deck.total_power)),
            pal::MIKU,
        ),
    }
}

fn background_debris(_height: f32) -> Vec<ChildEntry> {
    vec![entry(
        54.0,
        70.0,
        panel("bg_word", 520.0, 1.0, 0.0, pal::WHITE_08, None, 0.0),
    )]
}

fn header(h: Option<&DeckResultHeader>) -> WidgetNode {
    let recommend_type = h
        .and_then(|h| h.recommend_type.as_deref())
        .unwrap_or("deck");
    let target = resolve_target(h);
    let title = title_for(recommend_type, target);
    let protocol = protocol_for(recommend_type, target);

    let mut items = vec![
        entry(
            0.0,
            layout::header::RULE_Y,
            panel(
                "header_edge",
                layout::page::CONTENT_W,
                layout::header::RULE_H,
                0.0,
                pal::WHITE_24,
                None,
                0.0,
            ),
        ),
        entry(
            layout::header::PROTOCOL_BAR_X,
            layout::header::PROTOCOL_BAR_Y,
            panel(
                "protocol_bar",
                layout::header::PROTOCOL_BAR_W,
                layout::header::PROTOCOL_BAR_H,
                0.0,
                pal::NIIGO,
                None,
                0.0,
            ),
        ),
        entry(
            layout::header::PROTOCOL_X,
            layout::header::PROTOCOL_Y,
            text(
                "protocol",
                format!("推荐类型：{protocol}"),
                type_size::PROTOCOL,
                pal::GRAY,
            ),
        ),
        entry(
            layout::header::TITLE_X,
            layout::header::TITLE_Y,
            text("title", title, type_size::TITLE, pal::WHITE),
        ),
    ];

    if let Some(h) = h {
        if h.music_title.is_some() || h.music_jacket_key.is_some() {
            items.push(entry(
                layout::header::MUSIC_X,
                layout::header::MUSIC_Y,
                music_box(h),
            ));
        }

        if h.event_name.is_some() || h.event_id.is_some() || h.event_banner_key.is_some() {
            items.push(entry(
                layout::header::EVENT_X,
                layout::header::EVENT_Y,
                event_box(h),
            ));
        }

        if h.user_name.is_some() || h.user_id.is_some() || h.user_avatar_key.is_some() {
            items.push(entry(
                layout::header::USER_X,
                layout::header::USER_Y,
                user_box(h),
            ));
        }
    }

    WidgetNode::Container {
        id: "header".into(),
        layout: Layout::Absolute,
        children: items,
    }
}

fn title_for(recommend_type: &str, target: ResultTarget) -> &'static str {
    match target {
        ResultTarget::Power => "综合力组卡",
        ResultTarget::Skill => "技能组卡",
        ResultTarget::Mysekai => "MySekai 组卡",
        ResultTarget::EventPoint => match recommend_type {
            "wl" | "wl_fake" => "World Link 组卡",
            "mysekai" => "MySekai 组卡",
            _ => "活动组卡",
        },
        ResultTarget::Score => match recommend_type {
            "challenge" | "challenge_all" => "挑战组卡",
            "mysekai" => "MySekai 组卡",
            "no_event" => "最强组卡",
            _ => match recommend_type {
                "event" | "wl" | "wl_fake" | "unit_attr" => "活动组卡",
                _ => "组卡结果",
            },
        },
    }
}

fn protocol_for(recommend_type: &str, target: ResultTarget) -> &'static str {
    match target {
        ResultTarget::Power => "综合力",
        ResultTarget::Skill => "技能实效",
        ResultTarget::Mysekai => "MySekai",
        ResultTarget::EventPoint | ResultTarget::Score => match recommend_type {
            "event" => "活动",
            "wl" | "wl_fake" => "World Link",
            "challenge" | "challenge_all" => "挑战",
            "mysekai" => "MySekai",
            "no_event" => "无活动",
            "unit_attr" => "团队属性",
            _ => "组卡",
        },
    }
}

fn event_box(h: &DeckResultHeader) -> WidgetNode {
    let mut items = vec![entry(
        0.0,
        0.0,
        panel(
            "event_bg",
            layout::event::W,
            layout::event::H,
            8.0,
            pal::BLACK_35,
            Some(pal::WHITE_16),
            1.0,
        ),
    )];

    if let Some(key) = h.event_banner_key.as_deref() {
        items.push(entry(
            0.0,
            0.0,
            image(
                "event_banner",
                key,
                layout::event::W,
                layout::event::H,
                AssetImageFit::Cover,
                8.0,
            ),
        ));
        items.push(entry(
            0.0,
            0.0,
            panel(
                "event_scrim",
                layout::event::W,
                layout::event::H,
                8.0,
                pal::BLACK_55,
                None,
                0.0,
            ),
        ));
    }

    items.push(entry(
        layout::event::PAD_X,
        layout::event::LABEL_Y,
        text("event_label", "活动", type_size::EVENT_LABEL, pal::MIKU),
    ));
    items.push(entry(
        layout::event::PAD_X,
        layout::event::NAME_Y,
        text(
            "event_name",
            truncate_chars(h.event_name.as_deref().unwrap_or("未指定活动"), 18),
            type_size::EVENT_NAME,
            pal::WHITE,
        ),
    ));
    if let Some(event_id) = h.event_id {
        items.push(entry(
            layout::event::PAD_X,
            layout::event::ID_Y,
            text(
                "event_id",
                format!("活动 ID：{event_id}"),
                type_size::EVENT_ID,
                pal::GRAY,
            ),
        ));
    }

    WidgetNode::Container {
        id: "event_box".into(),
        layout: Layout::Absolute,
        children: items,
    }
}

fn music_box(h: &DeckResultHeader) -> WidgetNode {
    let mut items = vec![entry(
        0.0,
        0.0,
        panel(
            "music_bg",
            layout::music::W,
            layout::music::H,
            8.0,
            pal::BLACK_35,
            Some(pal::WHITE_16),
            1.0,
        ),
    )];
    if let Some(key) = h.music_jacket_key.as_deref() {
        items.push(entry(
            layout::music::PAD,
            layout::music::PAD,
            image(
                "jacket",
                key,
                layout::music::JACKET,
                layout::music::JACKET,
                AssetImageFit::Cover,
                6.0,
            ),
        ));
    }
    let text_x = if h.music_jacket_key.is_some() {
        layout::music::TEXT_WITH_JACKET_X
    } else {
        layout::music::TEXT_NO_JACKET_X
    };
    items.push(entry(
        text_x,
        layout::music::TITLE_Y,
        text(
            "music_title",
            truncate_chars(h.music_title.as_deref().unwrap_or("未指定歌曲"), 16),
            type_size::MUSIC_TITLE,
            pal::WHITE,
        ),
    ));
    if let Some(diff) = h.difficulty.as_deref() {
        items.push(entry(
            text_x,
            layout::music::DIFFICULTY_Y,
            tag("difficulty", diff.to_uppercase(), pal::BLACK_55, pal::MIKU),
        ));
    }
    WidgetNode::Container {
        id: "music_box".into(),
        layout: Layout::Absolute,
        children: items,
    }
}

fn user_box(h: &DeckResultHeader) -> WidgetNode {
    let display_name = h
        .user_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("名称待定");
    let id_text = mask_uid(
        h.user_id
            .as_deref()
            .filter(|uid| !uid.trim().is_empty())
            .unwrap_or("redacted"),
    );
    let mut items = vec![
        entry(
            0.0,
            0.0,
            glass("user_glass", layout::user::W, layout::user::H, 0.10),
        ),
        entry(
            layout::user::PAD_X,
            0.0,
            panel(
                "user_accent",
                82.0,
                layout::user::ACCENT_H,
                0.0,
                pal::MIKU,
                None,
                0.0,
            ),
        ),
    ];
    if let Some(key) = h.user_avatar_key.as_deref() {
        items.push(entry(
            layout::user::AVATAR_X,
            layout::user::AVATAR_Y,
            image(
                "avatar",
                key,
                layout::user::AVATAR,
                layout::user::AVATAR,
                AssetImageFit::Cover,
                8.0,
            ),
        ));
    } else {
        items.push(entry(
            layout::user::AVATAR_X,
            layout::user::AVATAR_Y,
            avatar_placeholder(display_name),
        ));
    }
    let text_x = layout::user::TEXT_WITH_AVATAR_X;
    items.push(entry(
        text_x,
        layout::user::LABEL_Y,
        text("user_label", "玩家", type_size::USER_LABEL, pal::MIKU),
    ));
    items.push(entry(
        text_x,
        layout::user::NAME_Y,
        text(
            "user_name",
            truncate_chars(display_name, 16),
            type_size::USER_NAME,
            pal::WHITE,
        ),
    ));
    items.push(entry(
        text_x,
        layout::user::ID_Y,
        text(
            "user_id",
            format!("UID：{id_text}"),
            type_size::USER_ID,
            pal::GRAY,
        ),
    ));
    WidgetNode::Container {
        id: "user_box".into(),
        layout: Layout::Absolute,
        children: items,
    }
}

fn avatar_placeholder(name: &str) -> WidgetNode {
    let label = name
        .chars()
        .find(|ch| !ch.is_whitespace())
        .map(|ch| ch.to_string())
        .unwrap_or_else(|| "U".to_string());
    WidgetNode::Container {
        id: "avatar_placeholder".into(),
        layout: Layout::Absolute,
        children: vec![
            entry(
                0.0,
                0.0,
                panel(
                    "avatar_bg",
                    layout::user::AVATAR,
                    layout::user::AVATAR,
                    8.0,
                    pal::BLACK_55,
                    Some(pal::MIKU),
                    1.0,
                ),
            ),
            entry(
                layout::user::AVATAR * 0.5,
                43.0,
                text_align(
                    "avatar_text",
                    label,
                    26.0,
                    pal::MIKU,
                    TextAlignValue::Center,
                    false,
                ),
            ),
        ],
    }
}

fn mask_uid(uid: &str) -> String {
    let trimmed = uid.trim();
    if trimmed.eq_ignore_ascii_case("redacted") {
        return "redacted".to_string();
    }
    let chars = trimmed.chars().collect::<Vec<_>>();
    if chars.len() <= 4 {
        return "*".repeat(chars.len().max(1));
    }
    let head = chars.iter().take(3).collect::<String>();
    let tail = chars
        .iter()
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{head}****{tail}")
}

fn deck_panel(
    deck: &DeckRenderUnit,
    index: usize,
    max_skill: f64,
    target: ResultTarget,
) -> WidgetNode {
    let is_top = index == 0;
    let rank_variance = glass_variance(deck.rank, if is_top { 0.34 } else { 0.24 }, 0.035);
    let body_variance = glass_variance(deck.rank, if is_top { 0.28 } else { 0.18 }, 0.030);
    let shadow_variance = glass_variance(deck.rank + 5, 0.22, 0.025);
    let mut items = vec![
        entry(
            0.0,
            layout::row::SHADOW_OFFSET,
            glass(
                "rank_thickness",
                layout::row::RANK_W,
                layout::row::H - layout::row::SHADOW_OFFSET,
                shadow_variance + 0.06,
            ),
        ),
        entry(
            layout::row::RANK_W + layout::row::RANK_GAP + layout::row::SHADOW_OFFSET,
            layout::row::SHADOW_OFFSET,
            glass(
                "deck_thickness",
                layout::row::DECK_W,
                layout::row::H - layout::row::SHADOW_OFFSET,
                shadow_variance,
            ),
        ),
        entry(0.0, 0.0, rank_panel(deck, is_top, target, rank_variance)),
        entry(
            layout::row::RANK_W + layout::row::RANK_GAP,
            0.0,
            deck_body(deck, is_top, max_skill, body_variance),
        ),
    ];

    if is_top {
        items.push(entry(
            layout::row::RANK_W + layout::row::RANK_GAP + 20.0,
            20.0,
            panel("top_glow", 420.0, 2.0, 0.0, pal::MIKU, None, 0.0),
        ));
    }

    WidgetNode::Container {
        id: format!("deck_panel_{}", deck.rank),
        layout: Layout::Absolute,
        children: items,
    }
}

fn glass_variance(rank: usize, base: f32, spread: f32) -> f32 {
    let phase = rank as f32 * 1.618;
    (base + phase.sin() * spread).clamp(0.04, 0.42)
}

fn rank_panel(
    deck: &DeckRenderUnit,
    is_top: bool,
    target: ResultTarget,
    variance: f32,
) -> WidgetNode {
    let (label, value, color) = target_metric(deck, target);
    let mut items = vec![
        entry(
            0.0,
            0.0,
            glass("rank_glass", layout::row::RANK_W, layout::row::H, variance),
        ),
        entry(
            layout::row::RANK_W * 0.5,
            layout::rank::NUMBER_Y,
            text_align(
                "rank",
                format!("#{}", deck.rank),
                type_size::RANK,
                pal::WHITE,
                TextAlignValue::Center,
                is_top,
            ),
        ),
    ];
    if is_top {
        items.push(entry(
            layout::rank::MARK_X,
            layout::rank::MARK_Y,
            panel(
                "rank_mark",
                layout::rank::MARK_W,
                layout::rank::MARK_H,
                0.0,
                pal::NIIGO,
                None,
                0.0,
            ),
        ));
    }
    items.push(entry(
        layout::rank::DIVIDER_X,
        layout::rank::DIVIDER_Y,
        panel(
            "rank_line",
            layout::rank::DIVIDER_W,
            layout::rank::DIVIDER_H,
            0.0,
            pal::WHITE_16,
            None,
            0.0,
        ),
    ));
    items.push(entry(
        layout::row::RANK_W * 0.5,
        layout::rank::LABEL_Y,
        text_align(
            "rank_label",
            label,
            type_size::RANK_LABEL,
            color,
            TextAlignValue::Center,
            false,
        ),
    ));
    items.push(entry(
        layout::row::RANK_W * 0.5,
        layout::rank::VALUE_Y,
        text_align(
            "rank_power",
            value,
            type_size::RANK_VALUE,
            pal::WHITE,
            TextAlignValue::Center,
            false,
        ),
    ));

    WidgetNode::Container {
        id: format!("rank_{}", deck.rank),
        layout: Layout::Absolute,
        children: items,
    }
}

fn deck_body(deck: &DeckRenderUnit, _is_top: bool, max_skill: f64, variance: f32) -> WidgetNode {
    let mut items = vec![
        entry(
            0.0,
            0.0,
            glass("deck_glass", layout::row::DECK_W, layout::row::H, variance),
        ),
        entry(
            layout::stats::X - 6.0,
            layout::row::INNER_PAD_Y,
            panel(
                "deck_divider",
                1.0,
                layout::row::H - layout::row::INNER_PAD_Y * 2.0,
                0.0,
                pal::WHITE_16,
                None,
                0.0,
            ),
        ),
    ];

    let mut cards = Vec::new();
    for (ci, card) in deck.cards.iter().take(5).enumerate() {
        cards.push(ChildEntry {
            position: pos(card_slot_x(ci), 0.0),
            node: card_chip(card, deck.rank, ci),
            visible: None,
        });
    }
    items.push(entry(
        layout::row::INNER_PAD_X,
        layout::row::INNER_PAD_Y,
        WidgetNode::Container {
            id: format!("cards_{}", deck.rank),
            layout: Layout::Absolute,
            children: cards,
        },
    ));

    items.push(entry(
        layout::stats::X,
        layout::row::INNER_PAD_Y,
        stats_panel(deck, max_skill),
    ));

    WidgetNode::Container {
        id: format!("deck_body_{}", deck.rank),
        layout: Layout::Absolute,
        children: items,
    }
}

fn card_chip(card: &DeckRenderCard, rank: usize, index: usize) -> WidgetNode {
    let mut items = vec![
        entry(
            0.0,
            0.0,
            panel(
                "chip_bg",
                layout::card::W,
                layout::card::H,
                5.0,
                pal::CHIP,
                Some(pal::WHITE),
                1.0,
            ),
        ),
        entry(
            layout::card::PAD,
            layout::card::PAD,
            WidgetNode::CardThumbnail {
                id: format!("thumb_{rank}_{index}"),
                size: layout::card::IMAGE,
                card_image_key: card.asset_key.clone(),
                rarity: card.rarity.clone(),
                attr: card.attr.clone(),
                master_rank: card.master_rank,
                trained: card.trained,
                show_info: true,
                level_text: format!("Lv.{}", card.level),
            },
        ),
        entry(
            layout::card::ID_X,
            layout::card::ID_Y,
            small_badge(
                "card_id",
                format!("#{}", card.card_id),
                layout::card::ID_W,
                pal::BLACK_55,
                pal::WHITE,
                type_size::CARD_EFFECT,
            ),
        ),
        entry(
            layout::card::CONTENT_X,
            layout::card::SKILL_Y,
            card_skill_row(card),
        ),
    ];

    items.push(entry(
        layout::card::CONTENT_X,
        layout::card::STATUS_Y,
        card_status_row(card),
    ));
    items.push(entry(
        layout::card::CONTENT_X,
        layout::card::PROGRESS_Y,
        panel(
            "slv_track",
            layout::card::PROGRESS_W,
            layout::card::PROGRESS_H,
            2.0,
            pal::CHIP_LINE,
            None,
            0.0,
        ),
    ));
    items.push(entry(
        layout::card::CONTENT_X,
        layout::card::PROGRESS_Y,
        panel(
            "slv_fill",
            layout::card::PROGRESS_W * (card.skill_level.clamp(0, 4) as f32 / 4.0),
            layout::card::PROGRESS_H,
            2.0,
            pal::NIIGO,
            None,
            0.0,
        ),
    ));

    WidgetNode::Container {
        id: format!("card_{rank}_{index}"),
        layout: Layout::Absolute,
        children: items,
    }
}

fn card_slot_x(index: usize) -> f32 {
    index as f32 * (layout::card::W + layout::card::GAP)
}

fn card_skill_row(card: &DeckRenderCard) -> WidgetNode {
    WidgetNode::Container {
        id: "card_skill".into(),
        layout: Layout::Absolute,
        children: vec![
            entry(
                0.0,
                0.0,
                panel(
                    "skill_bg",
                    layout::card::SKILL_W,
                    layout::card::BADGE_H,
                    3.0,
                    pal::MIKU_SOFT,
                    None,
                    0.0,
                ),
            ),
            entry(
                8.0,
                16.8,
                text("skill_label", "技能", type_size::CARD_EFFECT, pal::NIIGO),
            ),
            entry(
                layout::card::SKILL_W - 7.0,
                16.8,
                text_align(
                    "skill_value",
                    format!("+{}", fmt_pct(card.skill_score_up)),
                    type_size::CARD_EFFECT,
                    pal::DARK_TEXT,
                    TextAlignValue::Right,
                    false,
                ),
            ),
        ],
    }
}

fn card_status_row(card: &DeckRenderCard) -> WidgetNode {
    let bonus_text = card
        .event_bonus
        .filter(|bonus| *bonus > 0.0)
        .map(|bonus| format!("+{}", fmt_pct(bonus)))
        .unwrap_or_else(|| "-".to_string());
    WidgetNode::Container {
        id: "card_status".into(),
        layout: Layout::Horizontal {
            gap: layout::card::STATUS_GAP,
        },
        children: vec![
            auto(status_badge(
                "bonus",
                bonus_text,
                layout::card::BONUS_W,
                pal::BONUS_SOFT,
                pal::BONUS,
                type_size::CARD_BONUS,
            )),
            auto(episode_badge("ep1", "前篇", card.episode1_read)),
            auto(episode_badge("ep2", "后篇", card.episode2_read)),
        ],
    }
}

fn stats_panel(deck: &DeckRenderUnit, max_skill: f64) -> WidgetNode {
    let mut blocks = vec![auto(stat_block(
        "power",
        "卡组综合力",
        "基础属性",
        fmt(deck.total_power),
        pal::GOLD,
        pal::BLACK_70,
        false,
    ))];

    if let Some(skill) = deck.multi_live_score_up {
        let highlight = max_skill > 0.0 && (skill - max_skill).abs() < 0.001;
        blocks.push(auto(stat_block(
            "skill",
            "技能实效",
            "技能加分",
            format!("+{}", fmt_pct(skill)),
            pal::MIKU,
            if highlight {
                pal::MIKU_SOFT
            } else {
                pal::BLACK_70
            },
            highlight,
        )));
    }

    if let Some(bonus) = deck.event_bonus_total {
        if bonus > 0.0 {
            blocks.push(auto(stat_block(
                "bonus",
                "活动加成",
                "卡组加成",
                format!("+{}", fmt_pct(bonus)),
                pal::BONUS,
                pal::BLACK_70,
                false,
            )));
        }
    }

    let items = vec![entry(
        0.0,
        0.0,
        WidgetNode::Container {
            id: format!("stats_blocks_{}", deck.rank),
            layout: Layout::Vertical {
                gap: layout::stats::BLOCK_GAP,
            },
            children: blocks,
        },
    )];
    WidgetNode::Container {
        id: format!("stats_{}", deck.rank),
        layout: Layout::Absolute,
        children: items,
    }
}

fn stat_block(
    id: &str,
    label: &str,
    _sub: &str,
    value: String,
    accent: Color,
    bg: Color,
    glow: bool,
) -> WidgetNode {
    let items = vec![
        entry(
            0.0,
            0.0,
            panel(
                "bg",
                layout::stats::W,
                layout::stats::BLOCK_H,
                7.0,
                bg,
                Some(if glow { accent } else { pal::WHITE_16 }),
                1.0,
            ),
        ),
        entry(
            layout::stats::LABEL_X,
            layout::stats::LABEL_Y,
            text("label", label, type_size::STAT_LABEL, accent),
        ),
        entry(
            layout::stats::W - layout::stats::VALUE_PAD_R,
            layout::stats::VALUE_Y,
            text_align(
                "value",
                value,
                type_size::STAT_VALUE,
                pal::WHITE,
                TextAlignValue::Right,
                false,
            ),
        ),
    ];
    WidgetNode::Container {
        id: format!("stat_{id}"),
        layout: Layout::Absolute,
        children: items,
    }
}

fn tag(id: &str, content: impl Into<String>, bg: Color, fg: Color) -> WidgetNode {
    WidgetNode::TextBadge {
        id: id.into(),
        text: content.into(),
        bg_color: bg,
        text_color: fg,
    }
}

fn small_badge(
    id: &str,
    content: impl Into<String>,
    w: f32,
    bg: Color,
    fg: Color,
    font_size: f32,
) -> WidgetNode {
    let content = content.into();
    WidgetNode::Container {
        id: id.into(),
        layout: Layout::Absolute,
        children: vec![
            entry(
                0.0,
                0.0,
                panel("bg", w, layout::card::BADGE_H, 3.0, bg, None, 0.0),
            ),
            entry(
                w * 0.5,
                16.4,
                text_align(
                    "text",
                    content,
                    font_size,
                    fg,
                    TextAlignValue::Center,
                    false,
                ),
            ),
        ],
    }
}

fn status_badge(
    id: &str,
    content: impl Into<String>,
    w: f32,
    bg: Color,
    fg: Color,
    font_size: f32,
) -> WidgetNode {
    let content = content.into();
    WidgetNode::Container {
        id: id.into(),
        layout: Layout::Absolute,
        children: vec![
            entry(
                0.0,
                0.0,
                panel("bg", w, layout::card::STATUS_H, 3.0, bg, None, 0.0),
            ),
            entry(
                w * 0.5,
                16.5,
                text_align(
                    "text",
                    content,
                    font_size,
                    fg,
                    TextAlignValue::Center,
                    false,
                ),
            ),
        ],
    }
}

fn episode_badge(id: &str, label: &str, read: bool) -> WidgetNode {
    let (bg, fg) = if read {
        (pal::GREEN_BG, pal::GREEN)
    } else {
        (pal::RED_BG, pal::RED)
    };
    WidgetNode::Container {
        id: id.into(),
        layout: Layout::Absolute,
        children: vec![
            entry(
                0.0,
                0.0,
                panel(
                    "bg",
                    layout::card::EPISODE_W,
                    layout::card::STATUS_H,
                    3.0,
                    bg,
                    None,
                    0.0,
                ),
            ),
            entry(
                layout::card::EPISODE_W * 0.5,
                16.5,
                text_align(
                    "text",
                    if label == "前篇" { "前" } else { "后" },
                    type_size::EPISODE,
                    fg,
                    TextAlignValue::Center,
                    false,
                ),
            ),
        ],
    }
}

fn footer(
    cost: Option<&str>,
    algorithm: Option<&str>,
    timing_lines: &[String],
    timing_stages: &[DeckTimingStage],
) -> WidgetNode {
    let mut items = vec![
        entry(
            0.0,
            0.0,
            glass(
                "footer_glass",
                layout::page::CONTENT_W,
                layout::footer::H,
                0.16,
            ),
        ),
        entry(
            0.0,
            0.0,
            panel(
                "footer_tint",
                layout::page::CONTENT_W,
                layout::footer::H,
                8.0,
                pal::BLACK_35,
                Some(pal::WHITE_16),
                1.0,
            ),
        ),
    ];

    let total = timing_stages.first();
    let total_label = total
        .map(|stage| stage.label.as_str())
        .unwrap_or("链路耗时");
    let total_value = total.map(|stage| stage.value.as_str()).unwrap_or("--");
    let total_detail = total
        .and_then(|stage| stage.detail.as_deref())
        .unwrap_or("不含图片生成、上传");

    items.push(entry(
        layout::footer::TITLE_X,
        layout::footer::TITLE_Y,
        text_align(
            "footer_title",
            total_label,
            type_size::FOOTER_TITLE,
            pal::GRAY,
            TextAlignValue::Left,
            false,
        ),
    ));
    items.push(entry(
        layout::footer::TOTAL_X,
        layout::footer::TOTAL_Y,
        text_align(
            "footer_total",
            total_value,
            type_size::FOOTER_TOTAL,
            pal::GRAY,
            TextAlignValue::Left,
            false,
        ),
    ));
    items.push(entry(
        layout::footer::TOTAL_DETAIL_X,
        layout::footer::TITLE_Y,
        text_align(
            "footer_total_detail",
            total_detail,
            type_size::FOOTER_META,
            pal::GRAY,
            TextAlignValue::Left,
            false,
        ),
    ));

    let mut meta = Vec::new();
    if let Some(algorithm) = algorithm {
        meta.push(format!("算法 {algorithm}"));
    }
    if let Some(cost) = cost {
        meta.push(cost.to_string());
    }
    if !meta.is_empty() {
        items.push(entry(
            layout::footer::META_X,
            layout::footer::META_Y,
            text_align(
                "footer_meta",
                meta.join(" / "),
                type_size::FOOTER_META,
                pal::GRAY,
                TextAlignValue::Right,
                false,
            ),
        ));
    }

    if timing_stages.len() > 1 {
        let stage_count = (timing_stages.len() - 1).min(5);
        let stage_w = (layout::page::CONTENT_W
            - layout::footer::PAD_X * 2.0
            - layout::footer::STAGE_GAP * (stage_count.saturating_sub(1) as f32))
            / stage_count as f32;
        for (index, stage) in timing_stages.iter().skip(1).take(stage_count).enumerate() {
            let x = layout::footer::PAD_X + index as f32 * (stage_w + layout::footer::STAGE_GAP);
            items.push(entry(
                x,
                layout::footer::STAGE_Y,
                stage_chip(index, stage, stage_w),
            ));
        }
    } else {
        for (index, line) in timing_lines.iter().take(3).enumerate() {
            items.push(entry(
                layout::footer::PAD_X,
                layout::footer::STAGE_Y + index as f32 * 25.0,
                text_align(
                    &format!("timing_{index}"),
                    line,
                    type_size::FOOTER_META,
                    pal::GRAY,
                    TextAlignValue::Left,
                    false,
                ),
            ));
        }
    }

    WidgetNode::Container {
        id: "footer".into(),
        layout: Layout::Absolute,
        children: items,
    }
}

fn stage_chip(index: usize, stage: &DeckTimingStage, w: f32) -> WidgetNode {
    let accent = timing_tone_color(stage.tone.as_deref(), index);
    let detail = stage.detail.as_deref().unwrap_or("");
    WidgetNode::Container {
        id: format!("timing_stage_{index}"),
        layout: Layout::Absolute,
        children: vec![
            entry(
                0.0,
                0.0,
                panel(
                    "stage_bg",
                    w,
                    layout::footer::STAGE_H,
                    6.0,
                    pal::BLACK_35,
                    Some(pal::WHITE_16),
                    1.0,
                ),
            ),
            entry(
                0.0,
                0.0,
                panel("stage_accent", w, 3.0, 3.0, accent, None, 0.0),
            ),
            entry(
                layout::footer::STAGE_PAD_X,
                layout::footer::STAGE_LABEL_Y,
                text_align(
                    "stage_label",
                    &stage.label,
                    type_size::FOOTER_LABEL,
                    pal::GRAY,
                    TextAlignValue::Left,
                    false,
                ),
            ),
            entry(
                w - layout::footer::STAGE_PAD_X,
                layout::footer::STAGE_VALUE_Y,
                text_align(
                    "stage_value",
                    &stage.value,
                    type_size::FOOTER_VALUE,
                    pal::GRAY,
                    TextAlignValue::Right,
                    false,
                ),
            ),
            entry(
                w - layout::footer::STAGE_PAD_X,
                layout::footer::STAGE_DETAIL_Y,
                text_align(
                    "stage_detail",
                    truncate_chars(detail, 20),
                    type_size::FOOTER_DETAIL,
                    pal::GRAY,
                    TextAlignValue::Right,
                    false,
                ),
            ),
        ],
    }
}

fn timing_tone_color(_tone: Option<&str>, _index: usize) -> Color {
    pal::WHITE_24
}

fn fmt(v: i32) -> String {
    fmt_i64(i64::from(v))
}

fn fmt_i64(v: i64) -> String {
    let s = v.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    let result: String = out.chars().rev().collect();
    if v < 0 {
        format!("-{result}")
    } else {
        result
    }
}

fn fmt_pct(v: f64) -> String {
    if (v - v.round()).abs() < 0.01 {
        format!("{:.0}%", v)
    } else {
        format!("{:.1}%", v)
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}
