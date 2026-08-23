//! SDF 文本渲染相关模块。

pub mod atlas;
pub mod fallback_cache;
pub mod material;
pub mod outline;
#[cfg(feature = "skia-core")]
/// Legacy per-glyph blitter for the element draw path.
#[cfg(feature = "skia-core")]
pub mod rasterize;
pub mod shape;
pub mod shape_atlas;
pub mod tile;
