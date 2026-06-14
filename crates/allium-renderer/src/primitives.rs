//! 渲染图元定义模块。
//!
//! 在 Skia 和场景配方之间的抽象层。
//! 场景通过组合这些图元"积木"来描述最终画面，不直接接触 Skia Canvas。

/// RGBA 颜色值
#[derive(Debug, Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    /// 从 RGBA 创建颜色
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// 从 RGB 创建不透明颜色
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
}

/// 文字对齐方式
#[derive(Debug, Clone, Copy)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// 图片填充模式
#[derive(Debug, Clone, Copy)]
pub enum ImageFit {
    /// 拉伸填满区域
    Fill,
    /// 保持比例缩放，可能有留白
    Contain,
    /// 保持比例裁切，填满区域
    Cover,
}

/// 边框样式
#[derive(Debug, Clone)]
pub struct Border {
    /// 边框宽度（像素）
    pub width: f32,
    /// 边框颜色
    pub color: Color,
}

/// 容器内子元素的排列方式
#[derive(Debug, Clone)]
pub enum Layout {
    /// 绝对定位：子元素自带 x/y 坐标
    Absolute,
    /// 水平排列：等间距水平排列
    Horizontal { gap: f32 },
    /// 垂直排列：等间距垂直堆叠
    Vertical { gap: f32 },
    /// 网格排列：N 列等宽，自动换行
    Grid { columns: u32, gap: f32 },
}

/// 渲染图元（积木块）——场景配方的基本单元
#[derive(Debug, Clone)]
pub enum Primitive {
    /// 文字块
    Text {
        content: String,
        font_family: String,
        font_size: f32,
        color: Color,
        align: Align,
    },
    /// 图片块（从 AssetStore 取素材）
    Image {
        /// AssetStore 中的素材 key（S3 路径）
        asset_key: String,
        width: f32,
        height: f32,
        fit: ImageFit,
    },
    /// 矩形（背景/色块/边框）
    Rect {
        width: f32,
        height: f32,
        color: Color,
        /// 圆角半径
        radius: f32,
        border: Option<Border>,
    },
    /// 容器：组合多个子图元，按布局规则排列
    Container {
        layout: Layout,
        children: Vec<Positioned>,
    },
}

/// 带位置信息的图元
#[derive(Debug, Clone)]
pub struct Positioned {
    /// 相对于父容器的 X 偏移（Absolute 布局使用）
    pub x: f32,
    /// 相对于父容器的 Y 偏移（Absolute 布局使用）
    pub y: f32,
    /// 图元内容
    pub primitive: Primitive,
}

impl Positioned {
    /// 创建绝对定位的图元
    pub fn at(x: f32, y: f32, primitive: Primitive) -> Self {
        Self { x, y, primitive }
    }

    /// 创建自动布局的图元（x/y 由 Layout 引擎计算）
    pub fn auto(primitive: Primitive) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            primitive,
        }
    }
}

/// 场景树：完整描述一次渲染的输出。
///
/// 由 `Renderable.compose()` 生成，交给渲染引擎执行 Skia 绘制。
#[derive(Debug, Clone)]
pub struct SceneTree {
    /// 根图元（通常是一个 Container）
    pub root: Primitive,
    /// 画布宽度（像素）
    pub width: u32,
    /// 画布高度（像素）
    pub height: u32,
}
