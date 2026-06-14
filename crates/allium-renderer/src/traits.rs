//! Renderable trait 定义。
//!
//! 两阶段渲染：
//! 1. `compose()` → 返回图元树 SceneTree（纯数据，无 Skia 调用）
//! 2. 渲染引擎遍历 SceneTree → Skia Canvas → JPEG 字节

use crate::assets::AssetStore;
use crate::error::RenderError;
use crate::primitives::SceneTree;
use crate::render_document::RenderTiming;

/// 渲染输出：编码后的图片字节
pub struct RenderOutput {
    /// 编码后的图片字节（JPEG/WebP）
    pub data: Vec<u8>,
    /// MIME 类型（如 "image/jpeg"）
    pub content_type: String,
    /// 图片宽度
    pub width: u32,
    /// 图片高度
    pub height: u32,
    /// 文档渲染分段耗时；旧图元路径暂不填充。
    pub timing: Option<RenderTiming>,
}

/// 可渲染场景 trait。
///
/// 场景实现者只需关心**图元组合**（compose），
/// 不需要关心 Skia 绑定如何绘制——这由渲染引擎统一处理。
///
/// # 两阶段分离
/// - `compose()`: 业务逻辑 → 描述"画什么"（图元树）
/// - 渲染引擎: 机械执行 → 负责"怎么画"（Skia 调用）
pub trait Renderable: Send + Sync {
    /// 组合图元树，描述要渲染的画面。
    ///
    /// # 参数
    /// - `assets`: 素材缓存（通过 asset_key 引用素材）
    ///
    /// # 返回
    /// SceneTree：包含根图元 + 画布尺寸
    fn compose(&self, assets: &AssetStore) -> Result<SceneTree, RenderError>;

    /// 场景名称（用于日志和 Prometheus 指标标签）
    fn name(&self) -> &'static str;
}
