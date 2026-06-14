//! 场景图元素定义。

use serde::Deserialize;

/// 三维向量（Unity 坐标系）。
#[derive(Debug, Clone, Deserialize)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// 四元数旋转。
#[derive(Debug, Clone, Deserialize)]
pub struct Quaternion {
    pub w: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// 所有可见元素共享的空间变换数据。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectData {
    pub layer: i32,
    pub lock: bool,
    pub position: Vec3,
    pub rotation: Quaternion,
    pub scale: Vec3,
    pub visible: bool,
}

/// 文本元素。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextElement {
    pub object_data: ObjectData,
    pub color_id: i32,
    pub font_id: i32,
    pub line_spacing: f32,
    pub outline_color_id: i32,
    pub outline_size: f32,
    pub size: f32,
    pub text: String,
    #[serde(rename = "type")]
    pub text_type: i32,
}

/// 形状元素。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeElement {
    pub object_data: ObjectData,
    pub alpha: f32,
    pub color_id: i32,
    pub id: i32,
    pub outline_alpha: f32,
    pub outline_color_id: i32,
    pub outline_size: f32,
}

/// 卡面立绘元素。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardMemberElement {
    pub object_data: ObjectData,
    pub id: i32,
    #[serde(rename = "type")]
    pub member_type: Option<i32>,
    pub show_master_rank: Option<bool>,
    pub use_after_special_training: Option<bool>,
}

/// 印章元素。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StampElement {
    pub object_data: ObjectData,
    pub id: i32,
}

/// 其它装饰元素。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherElement {
    pub object_data: ObjectData,
    pub id: i32,
}

/// 羁绊称号元素。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BondsHonorElement {
    pub object_data: ObjectData,
    pub id: i32,
    pub word_id: i64,
    pub full_size: bool,
    pub inverse: bool,
    pub use_unit_virtual_singer: bool,
    #[serde(default = "default_honor_level")]
    pub honor_level: i32,
}

/// 普通称号元素。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HonorElement {
    pub object_data: ObjectData,
    pub id: i32,
    pub full_size: bool,
    #[serde(default = "default_honor_level")]
    pub honor_level: i32,
}

fn default_honor_level() -> i32 {
    1
}

/// 收藏品元素。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionElement {
    pub object_data: ObjectData,
    pub id: i32,
    pub target_id: Option<i32>,
}

/// 通用贴图元素。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralElement {
    pub object_data: ObjectData,
    #[serde(rename = "type")]
    pub general_type: Option<i32>,
}

/// 角色站立立绘元素。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StandMemberElement {
    pub object_data: ObjectData,
    pub id: i32,
}

/// 通用背景元素。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralBackgroundElement {
    pub object_data: ObjectData,
    pub id: i32,
}

/// 剧情背景元素。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryBackgroundElement {
    pub object_data: ObjectData,
    pub id: i32,
}
