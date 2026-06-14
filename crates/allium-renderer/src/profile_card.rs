//! 名片渲染实现（图元组合模式）。
//!
//! 这是 Renderable trait 的第一个具体实现。
//! 仅负责组合图元树，不接触 Skia Canvas。

use crate::assets::AssetStore;
use crate::error::RenderError;
use crate::primitives::*;
use crate::traits::Renderable;

/// 名片场景参数
pub struct ProfileCard {
    /// 玩家 ID
    pub user_id: String,
    /// 玩家昵称
    pub nickname: String,
    /// 展示卡组（最多 5 张卡的 card_id）
    pub deck_card_ids: Vec<String>,
    /// 等级
    pub rank: u32,
}

impl Renderable for ProfileCard {
    /// 组合名片图元树。
    ///
    /// # 图元结构示意
    /// ```text
    /// Container(Absolute) [1200×630]
    ///   ├── Rect(背景, 圆角 12)
    ///   ├── Container(Horizontal, gap=12) @ (20, 20)
    ///   │     ├── Image(头像, 64×64)
    ///   │     └── Text(昵称, 24px)
    ///   └── Container(Grid, 5列) @ (20, 100)
    ///         ├── Image(卡面1)
    ///         ├── Image(卡面2) ...
    /// ```
    fn compose(&self, _assets: &AssetStore) -> Result<SceneTree, RenderError> {
        // 卡面图片图元列表
        let deck_images: Vec<Positioned> = self
            .deck_card_ids
            .iter()
            .map(|card_id| {
                Positioned::auto(Primitive::Image {
                    asset_key: format!("assets/cards/{card_id}.webp"),
                    width: 200.0,
                    height: 200.0,
                    fit: ImageFit::Cover,
                })
            })
            .collect();

        // 根图元：绝对定位容器
        let root = Primitive::Container {
            layout: Layout::Absolute,
            children: vec![
                // 背景矩形
                Positioned::at(
                    0.0,
                    0.0,
                    Primitive::Rect {
                        width: 1200.0,
                        height: 630.0,
                        color: Color::rgb(30, 30, 40),
                        radius: 12.0,
                        border: None,
                    },
                ),
                // 头部：头像 + 昵称（水平排列）
                Positioned::at(
                    20.0,
                    20.0,
                    Primitive::Container {
                        layout: Layout::Horizontal { gap: 12.0 },
                        children: vec![
                            Positioned::auto(Primitive::Image {
                                asset_key: format!("assets/avatars/{}.webp", self.user_id),
                                width: 64.0,
                                height: 64.0,
                                fit: ImageFit::Cover,
                            }),
                            Positioned::auto(Primitive::Text {
                                content: self.nickname.clone(),
                                font_family: "Noto Sans SC".into(),
                                font_size: 24.0,
                                color: Color::rgb(255, 255, 255),
                                align: Align::Left,
                            }),
                        ],
                    },
                ),
                // 卡组展示（5 列网格）
                Positioned::at(
                    20.0,
                    100.0,
                    Primitive::Container {
                        layout: Layout::Grid {
                            columns: 5,
                            gap: 8.0,
                        },
                        children: deck_images,
                    },
                ),
            ],
        };

        Ok(SceneTree {
            root,
            width: 1200,
            height: 630,
        })
    }

    fn name(&self) -> &'static str {
        "profile_card"
    }
}
