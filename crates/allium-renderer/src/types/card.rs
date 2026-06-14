//! 自定义名片卡片结构。

use serde::Deserialize;

use super::{
    BondsHonorElement, CardMemberElement, CollectionElement, GeneralBackgroundElement,
    GeneralElement, HonorElement, OtherElement, ShapeElement, StampElement, StandMemberElement,
    StoryBackgroundElement, TextElement,
};

/// 单页自定义名片的完整数据。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomProfileCard {
    #[serde(default)]
    pub texts: Vec<TextElement>,
    #[serde(default)]
    pub shapes: Vec<ShapeElement>,
    #[serde(default)]
    pub card_members: Vec<CardMemberElement>,
    #[serde(default)]
    pub stamps: Vec<StampElement>,
    #[serde(default)]
    pub others: Vec<OtherElement>,
    #[serde(default)]
    pub bonds_honors: Vec<BondsHonorElement>,
    #[serde(default)]
    pub honors: Vec<HonorElement>,
    #[serde(default)]
    pub collections: Vec<CollectionElement>,
    #[serde(default)]
    pub generals: Vec<GeneralElement>,
    #[serde(default)]
    pub general_backgrounds: Vec<GeneralBackgroundElement>,
    #[serde(default)]
    pub stand_members: Vec<StandMemberElement>,
    #[serde(default)]
    pub story_backgrounds: Vec<StoryBackgroundElement>,
}

/// Profile API 返回的自定义名片包裹层。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserCustomProfileCard {
    pub custom_profile_card: CustomProfileCard,
    pub custom_profile_card_id: i32,
    pub custom_profile_id: i32,
    pub seq: i32,
}
