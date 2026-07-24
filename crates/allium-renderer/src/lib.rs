//! # allium-renderer
//!
//! 场景图解释器渲染引擎。
//!
//! ## 渲染路径
//! - **游戏自定义名片（1:1 复原）**：Profile JSON → `types::CustomProfileCard`
//!   → `flatten_and_sort()` 按 layer 排序 → 逐元素驱动 Skia Canvas（translate/rotate/scale/draw）。
//! - **信息图 / 平台卡**：数据 → `WidgetDocument`（`widget_node`）→ `render_document` → Skia Canvas。
//!
//! ## 模块结构
//! - `profile`: 玩家数据模型（跨管线共享）
//! - `init`: 启动初始化（字体安装等）
//! - `assets`: 素材内存缓存
//! - `widget_node` / `render_document`: WidgetDocument IR 与渲染
//! - `traits`: `RenderOutput` 输出类型

#![deny(clippy::unwrap_used)]

pub use allium_renderer_core as core;

#[cfg(feature = "animation-export")]
pub mod animation;
pub mod asset_keys;
pub mod assets;
#[cfg(feature = "skia-core")]
pub mod compiled_profile;
pub mod context;
#[cfg(feature = "skia-core")]
pub mod core_shadow;
pub mod deck_result;
pub mod elements;
pub mod error;
pub mod init;
pub mod instantiate;
#[cfg(feature = "jpeg-turbo")]
pub mod jpeg_turbo;
pub mod masterdata;
#[cfg(feature = "scenes")]
pub mod mysekai_harvest;
#[cfg(feature = "scenes")]
pub mod personal_profile;
pub mod profile;
#[cfg(feature = "skia-core")]
pub mod profile_backend;
#[cfg(feature = "skia-core")]
pub mod profile_compositor;
#[cfg(feature = "scenes")]
pub mod ranking;
pub mod region;
pub mod render_document;
pub mod render_object;
pub mod render_object_catalog;
pub mod renderer;
pub mod resource_provider;
#[cfg(feature = "skia-core")]
pub mod sdf;
#[cfg(feature = "skia-core")]
pub mod semantic_resolve;
pub mod text;
pub mod traits;
pub mod transform;
pub mod types;
pub mod widget_node;
pub mod widgets;
