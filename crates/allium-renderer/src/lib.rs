//! # allium-renderer
//!
//! 双路径渲染引擎。
//!
//! ## 路径 A：图元组合（基础名片，我们设计）
//! 1. `Renderable.compose()` → SceneTree（纯数据，可测试）
//! 2. `RenderExecutor.execute()` → Skia Canvas → JPEG 字节
//!
//! ## 路径 B：场景图解释器（游戏自定义名片，1:1 复原）
//! 1. Profile JSON → `types::CustomProfileCard`
//! 2. `flatten_and_sort()` → 按 layer 排序的元素数组
//! 3. 逐元素驱动 Skia Canvas（translate/rotate/scale/draw）
//!
//! ## 模块结构
//! - `profile`: 玩家数据模型（跨管线共享）
//! - `init`: 启动初始化（字体安装等）
//! - `assets`: 素材内存缓存
//! - `primitives`: 图元定义（路径 A）
//! - `executor`: 渲染执行器
//! - `profile_card`: 基础名片配方
//! - `traits`: Renderable trait

#![deny(clippy::unwrap_used)]

pub mod asset_keys;
pub mod assets;
pub mod context;
pub mod deck_result;
pub mod elements;
pub mod error;
#[cfg(feature = "executor")]
pub mod executor;
pub mod init;
pub mod instantiate;
pub mod masterdata;
#[cfg(feature = "scenes")]
pub mod mysekai_harvest;
#[cfg(feature = "scenes")]
pub mod personal_profile;
pub mod primitives;
pub mod profile;
pub mod profile_card;
#[cfg(feature = "scenes")]
pub mod ranking;
pub mod render_document;
pub mod renderer;
pub mod resource_provider;
#[cfg(feature = "skia-core")]
pub mod sdf;
pub mod text;
pub mod traits;
pub mod transform;
pub mod types;
pub mod widget_node;
pub mod widgets;
