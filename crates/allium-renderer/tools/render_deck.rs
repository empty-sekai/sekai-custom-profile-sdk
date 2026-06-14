//! 渲染组卡推荐结果示例。
//!
//! ```bash
//! # 1. 从你的素材服务器下载卡面素材（可选，没有素材时用纯色块）
//! #    将 <asset-host> 替换为你自己的素材域名
//! mkdir -p output/assets
//! for id in 1 2 3 4 5; do
//!   curl -sSf "https://<asset-host>/assets/cn/character/member_small/card_${id}/card_normal.png" \
//!     -o "output/assets/character__member_small__card_${id}__card_normal.png" || true
//! done
//!
//! # 2. 在带 freetype/pkg-config 的容器中运行
//! cargo run --release -p allium-renderer \
//!   --example render_deck --features skia -- ./output/deck_result.jpeg ./output/assets
//! # 输出: ./output/deck_result.jpeg
//! ```
#![allow(deprecated)] // 本示例刻意演示已弃用的 RenderExecutor

use std::path::PathBuf;
use std::sync::Arc;

use allium_renderer::assets::AssetStore;
use allium_renderer::deck_result::{DeckRenderCard, DeckRenderUnit, DeckResultCard};
use allium_renderer::executor::RenderExecutor;
use allium_renderer::widgets::theme::Theme;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let output = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/output/deck_result.jpeg"));
    let assets_dir = args.get(2).map(PathBuf::from);

    let decks = build_sample_decks();

    // 先注入素材到 AssetStore
    let store = AssetStore::new(64);
    if let Some(ref dir) = assets_dir {
        for deck in &decks {
            for card in &deck.cards {
                let path = dir.join(&card.asset_key).with_extension("png");
                if path.exists() {
                    match std::fs::read(&path) {
                        Ok(data) => {
                            eprintln!("素材注入: {} ({} bytes)", card.asset_key, data.len());
                            store.put(card.asset_key.clone(), data);
                        }
                        Err(e) => eprintln!("读取失败: {path:?}: {e}"),
                    }
                } else {
                    eprintln!("素材缺失: {path:?}");
                }
            }
        }
    }

    let card = DeckResultCard {
        header: None,
        decks,
        cost_info: None,
        algorithm_info: None,
        timing_lines: Vec::new(),
        timing_stages: Vec::new(),
        output_quality: None,
        param_summary: None,
    };

    let assets = Arc::new(store);
    let executor = RenderExecutor::new(1, assets).expect("创建渲染执行器失败");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("创建 tokio runtime 失败");

    let output_path = output.clone();
    rt.block_on(async {
        let result = executor
            .execute_document(card.to_widget_document(), Theme::default())
            .await
            .expect("渲染失败");
        std::fs::write(&output_path, &result.data).expect("写入文件失败");
        eprintln!(
            "渲染完成: {} ({}x{}, {} bytes)",
            output_path.display(),
            result.width,
            result.height,
            result.data.len()
        );
    });
}

fn build_sample_decks() -> Vec<DeckRenderUnit> {
    // 示例卡 ID 和 assetbundleName（从游戏 masterdata 查得）
    let real_cards: Vec<(i32, &str, &str, &str)> = vec![
        (617, "res022_no024", "rarity_4", "cool"),
        (1096, "res021_no052", "rarity_4", "cool"),
        (950, "res025_no038", "rarity_4", "cool"),
        (830, "res024_no033", "rarity_4", "cool"),
        (160, "res008_no006", "rarity_4", "cool"),
    ];

    let make_unit = |rank: usize, power_offset: i32, score_offset: i32| {
        let cards: Vec<DeckRenderCard> = real_cards
            .iter()
            .enumerate()
            .map(|(idx, (card_id, abn, rarity, attr))| DeckRenderCard {
                card_id: *card_id,
                asset_key: format!("thumbnail/chara/{abn}_normal"),
                rarity: (*rarity).to_string(),
                attr: (*attr).to_string(),
                level: 80,
                skill_level: 4 - (idx as i32 % 2),
                skill_score_up: 120.0 - idx as f64 * 5.0,
                event_bonus: Some(if idx < 4 { 25.0 } else { 0.0 }),
                master_rank: 5 - idx as i32,
                trained: true,
                episode1_read: true,
                episode2_read: idx != 2,
            })
            .collect();
        DeckRenderUnit {
            rank,
            cards,
            total_power: 310_000 - power_offset,
            live_score: 1_234_567 - score_offset,
            event_point: Some(1_200 - score_offset / 500),
            target_value: Some(1_234_567 - score_offset as i64),
            skill_score: 245.0 - score_offset as f64 / 200_000.0,
            multi_live_score_up: Some(130.0 - rank as f64 * 2.5),
            event_bonus_total: Some(120.0),
        }
    };

    vec![
        make_unit(1, 0, 0),
        make_unit(2, 8_000, 50_000),
        make_unit(3, 16_000, 100_000),
        make_unit(4, 24_000, 150_000),
        make_unit(5, 32_000, 200_000),
    ]
}
