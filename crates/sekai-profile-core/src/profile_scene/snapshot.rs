//! Snapshot and input types: the borrowed authored elements a scene is lowered
//! from and the resolved data recipe builders read.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::profile_source::*;
use crate::{
    AuthoredElementKind, InteractionRegionSource, LineIndentSource, ParameterValue, Rect,
    SemanticCommandSource, StableId,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProfileElementRef<'a> {
    Text(&'a TextElement),
    Shape(&'a ShapeElement),
    CardMember(&'a CardMemberElement),
    Stamp(&'a StampElement),
    Other(&'a OtherElement),
    BondsHonor(&'a BondsHonorElement),
    Honor(&'a HonorElement),
    Collection(&'a CollectionElement),
    General(&'a GeneralElement),
    StandMember(&'a StandMemberElement),
    GeneralBackground(&'a GeneralBackgroundElement),
    StoryBackground(&'a StoryBackgroundElement),
    CharacterIcon(&'a CharacterIconElement),
    Material(&'a MaterialElement),
    UserInterfaceIcon(&'a UserInterfaceIconElement),
}

impl ProfileElementRef<'_> {
    pub fn object(&self) -> &ObjectData {
        match self {
            Self::Text(value) => &value.object_data,
            Self::Shape(value) => &value.object_data,
            Self::CardMember(value) => &value.object_data,
            Self::Stamp(value) => &value.object_data,
            Self::Other(value) => &value.object_data,
            Self::BondsHonor(value) => &value.object_data,
            Self::Honor(value) => &value.object_data,
            Self::Collection(value) => &value.object_data,
            Self::General(value) => &value.object_data,
            Self::StandMember(value) => &value.object_data,
            Self::GeneralBackground(value) => &value.object_data,
            Self::StoryBackground(value) => &value.object_data,
            Self::CharacterIcon(value) => &value.object_data,
            Self::Material(value) => &value.object_data,
            Self::UserInterfaceIcon(value) => &value.object_data,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedProfileElement<'a> {
    pub kind: AuthoredElementKind,
    pub source_index: usize,
    pub source_key: String,
    pub layer_id: StableId,
    pub value: ProfileElementRef<'a>,
}

impl NormalizedProfileElement<'_> {
    pub fn object(&self) -> &ObjectData {
        self.value.object()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileComponentSnapshot {
    pub locale: String,
    pub region_fonts: BTreeMap<i32, String>,
    pub localized_text: BTreeMap<String, String>,
    pub user_name: String,
    pub word: String,
    #[serde(default)]
    pub user_rank: i32,
    #[serde(default)]
    pub total_power: i64,
    #[serde(default)]
    pub mvp: i32,
    #[serde(default)]
    pub superstar: i32,
    #[serde(default)]
    pub challenge_score: i32,
    #[serde(default)]
    pub challenge_character_id: i32,
    #[serde(default)]
    pub challenge_avatar: Option<ComponentImageSnapshot>,
    #[serde(default)]
    pub music_results: Option<MusicResultsSnapshot>,
    /// Favorite-story panel slots in order: entry `i` is the story shared as
    /// `shareNo` `i + 1`.
    #[serde(default)]
    pub story_favorites: Vec<StoryFavoriteSnapshot>,
    #[serde(default)]
    pub player_avatar: Option<ComponentImageSnapshot>,
    #[serde(default)]
    pub character_ranks: Vec<CharacterRankSnapshot>,
    #[serde(default)]
    pub deck_members: Vec<CardVisualSnapshot>,
    #[serde(default)]
    pub leader_card: Option<CardVisualSnapshot>,
    #[serde(default)]
    pub honor_slots: Vec<HonorVisualSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComponentImageSnapshot {
    pub source_field: String,
    pub source_id: String,
    pub descriptor: Option<ResourceDescriptor>,
}

/// One slot of the favorite-story panel. A slot nobody shares has
/// `story_id` 0 and no image.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoryFavoriteSnapshot {
    pub story_id: i32,
    pub story_type: String,
    pub image: ComponentImageSnapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CharacterRankSnapshot {
    pub character_id: i32,
    pub rank: i32,
    pub challenge_rank: Option<i32>,
    pub avatar: ComponentImageSnapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CardVisualSnapshot {
    pub card_id: i32,
    /// The card overlay draws trained rarity stars. This follows the card's
    /// special-training status, except on the leader card, whose stars follow
    /// the illustration the player shows. The illustration itself is already
    /// chosen by `image`.
    pub after_training: bool,
    pub master_rank: i32,
    pub level: i32,
    pub rarity: String,
    pub attribute: String,
    pub image: ComponentImageSnapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HonorVisualSnapshot {
    pub source_field: String,
    pub source_id: String,
    pub honor_id: i32,
    pub honor_level: i32,
    pub full_size: bool,
    pub visual: HonorVisualKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HonorVisualKind {
    Standard {
        honor_type: String,
        has_star: bool,
        is_live_master: bool,
        progress: i32,
        background: Option<ResourceDescriptor>,
        frame_candidates: Vec<Option<ResourceDescriptor>>,
        overlay: Option<ResourceDescriptor>,
        star: Option<ResourceDescriptor>,
        star_high: Option<ResourceDescriptor>,
        live_star_on: Option<ResourceDescriptor>,
        live_star_off: Option<ResourceDescriptor>,
    },
    Bonds {
        character_ids: [i32; 2],
        backgrounds: [Option<ResourceDescriptor>; 2],
        characters: [Option<ResourceDescriptor>; 2],
        mask: Option<ResourceDescriptor>,
        frame: Option<ResourceDescriptor>,
        word: Option<ResourceDescriptor>,
        star: Option<ResourceDescriptor>,
        star_high: Option<ResourceDescriptor>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MusicDifficultySnapshot {
    pub clear: i32,
    pub full_combo: i32,
    pub all_perfect: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MusicResultsSnapshot {
    pub easy: MusicDifficultySnapshot,
    pub normal: MusicDifficultySnapshot,
    pub hard: MusicDifficultySnapshot,
    pub expert: MusicDifficultySnapshot,
    pub master: MusicDifficultySnapshot,
    pub append: MusicDifficultySnapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileComponentLowering {
    pub commands: Vec<SemanticCommandSource>,
    pub interaction_regions: Vec<InteractionRegionSource>,
    pub controls: Vec<ComponentControlSource>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComponentControlSource {
    pub id: StableId,
    pub layer_id: StableId,
    pub role: String,
    pub state: ComponentControlState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComponentControlState {
    Tabs {
        options: Vec<String>,
        active: String,
    },
    Scroll {
        offset: f32,
        min: f32,
        max: f32,
        viewport_extent: f32,
        content_extent: f32,
        step: f32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceDescriptor {
    pub resource: crate::ResourceKey,
    pub natural_width: f32,
    pub natural_height: f32,
    #[serde(default)]
    pub provenance: BTreeMap<String, ParameterValue>,
}

impl ResourceDescriptor {
    pub fn centered_bounds(&self) -> Rect {
        Rect {
            x: -self.natural_width / 2.0,
            y: -self.natural_height / 2.0,
            width: self.natural_width,
            height: self.natural_height,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileResolveSnapshot {
    pub fonts: BTreeMap<i32, String>,
    pub colors: BTreeMap<i32, [f32; 4]>,
    pub resources: BTreeMap<String, ResourceDescriptor>,
    pub line_indent: BTreeMap<String, LineIndentSource>,
    pub component: Option<ProfileComponentSnapshot>,
    #[serde(default)]
    pub honor_visuals: BTreeMap<String, HonorVisualSnapshot>,
    #[serde(default)]
    pub card_member_visuals: BTreeMap<String, CardVisualSnapshot>,
    /// Source keys of elements that are not drawn: the game does not build an
    /// element whose master-data row does not exist, and an omikuji collection
    /// is left out when its slip is not laid out ([`crate::omikuji`]) or the
    /// resolver does not draw slips. Their layers stay empty.
    #[serde(default)]
    pub omitted_elements: BTreeSet<String>,
    /// Omikuji collections to draw, by source key.
    #[serde(default)]
    pub omikuji_visuals: BTreeMap<String, crate::omikuji::OmikujiVisualSnapshot>,
}
