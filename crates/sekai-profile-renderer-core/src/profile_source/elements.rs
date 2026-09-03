//! Source scene element definitions.

use serde::{Deserialize, Serialize};

/// Three-dimensional vector in Unity coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Quaternion rotation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quaternion {
    pub w: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Spatial transform shared by every visible source element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectData {
    pub layer: i32,
    pub lock: bool,
    pub position: Vec3,
    pub rotation: Quaternion,
    pub scale: Vec3,
    pub visible: bool,
}

/// Text element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// Shape element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// Card artwork element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardMemberElement {
    pub object_data: ObjectData,
    pub id: i32,
    #[serde(rename = "type")]
    pub member_type: Option<i32>,
    pub show_master_rank: Option<bool>,
    pub use_after_special_training: Option<bool>,
}

/// Stamp element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StampElement {
    pub object_data: ObjectData,
    pub id: i32,
}

/// Miscellaneous decorative element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherElement {
    pub object_data: ObjectData,
    pub id: i32,
}

/// Character-pair honor element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// Standard honor element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// Collectible element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionElement {
    pub object_data: ObjectData,
    pub id: i32,
    pub target_id: Option<i32>,
}

/// General-purpose image element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralElement {
    pub object_data: ObjectData,
    #[serde(rename = "type")]
    pub general_type: Option<i32>,
}

/// Full-body character artwork element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StandMemberElement {
    pub object_data: ObjectData,
    pub id: i32,
}

/// General background element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralBackgroundElement {
    pub object_data: ObjectData,
    pub id: i32,
}

/// Story background element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryBackgroundElement {
    pub object_data: ObjectData,
    pub id: i32,
}
