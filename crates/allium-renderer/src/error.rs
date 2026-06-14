//! 渲染引擎自含错误类型。

use thiserror::Error;

/// allium-renderer 的统一错误枚举。
#[derive(Debug, Error)]
pub enum RenderError {
    #[error("渲染失败: {0}")]
    Render(String),
    #[error("素材加载失败: {key}")]
    AssetNotFound { key: String },
    #[error("MasterData 查询失败: {0}")]
    DataQuery(String),
    #[error("编码失败: {0}")]
    Encode(String),
    #[error("配置错误: {0}")]
    Config(String),
}
