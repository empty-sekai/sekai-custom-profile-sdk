//! CLI / wasm 共享的宿主层。
//!
//! 提供 `JsonMasterDataProvider`：从 JSON 表字节构建
//! [`sekai_profile_renderer::masterdata::MasterDataProvider`]，
//! 表映射与生产网关适配层保持一致。

mod provider;
mod table;

pub use provider::{JsonMasterDataProvider, REQUIRED_TABLES};
pub use sekai_profile_renderer::region::Region;
pub use table::Table;
