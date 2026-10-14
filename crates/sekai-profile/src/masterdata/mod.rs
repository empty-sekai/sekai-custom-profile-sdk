//! Masterdata lookups the renderer needs ([`MasterDataProvider`]), the
//! [`MasterData`] handle it renders with, and [`JsonMasterDataProvider`], which
//! answers them from the game's JSON tables.

use std::sync::Arc;

use crate::types::{BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry};

mod json;
mod table;

pub use json::{JsonMasterDataProvider, OPTIONAL_TABLES, REQUIRED_TABLES};
pub use sekai_profile_core::masterdata::{CollectionResourceType, OmikujiRow};
pub use table::Table;

/// A resolved RGBA color.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedColor {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel; 255 when the source has no alpha.
    pub a: u8,
}

impl ResolvedColor {
    /// Parses `#RRGGBB` or `#RRGGBBAA`; the `#` is optional. Returns `None`
    /// for any other length or a non-hex digit.
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim_start_matches('#');
        // Parse bytes: the length counts bytes, and slicing a string at a
        // non-character boundary would panic. Invalid input returns None.
        let bytes = hex.as_bytes();
        let len = bytes.len();
        if len != 6 && len != 8 {
            return None;
        }
        let pair = |range: std::ops::Range<usize>| -> Option<u8> {
            std::str::from_utf8(&bytes[range])
                .ok()
                .and_then(|value| u8::from_str_radix(value, 16).ok())
        };
        let r = pair(0..2)?;
        let g = pair(2..4)?;
        let b = pair(4..6)?;
        let a = if len == 8 { pair(6..8)? } else { 255 };
        Some(Self { r, g, b, a })
    }
}

/// A row of one of the `customProfile*Resources` tables.
#[derive(Debug, Clone)]
pub struct ResourceInfo {
    /// `fileName`.
    pub file_name: String,
    /// `resourceLoadVal`: the directory part of the asset key.
    pub load_val: String,
    /// `customProfileResourceType`.
    pub resource_type: String,
    /// `customProfileResourceCollectionType` of a collection row; other rows
    /// carry [`CollectionResourceType::None`].
    pub collection_type: CollectionResourceType,
}

impl ResourceInfo {
    /// Asset key the resource loads from: `{resourceLoadVal}/{fileName}`.
    pub fn asset_key(&self) -> String {
        format!("{}/{}", self.load_val, self.file_name)
    }
}

/// A standard honor resolved from the `honors` and `honorGroups` tables for one
/// level.
#[derive(Debug, Clone)]
pub struct ResolvedHonor {
    /// Asset bundle of the honor image.
    pub asset_bundle_name: String,
    /// `honorRarity` (`low`, `middle`, `high` or `highest`); `low` when the
    /// row has none.
    pub honor_rarity: String,
    /// `honorType` of the honor group; `normal` when the group has none.
    pub honor_type: String,
    /// `backgroundAssetbundleName` of the honor group, when present.
    pub background_asset_bundle_name: Option<String>,
    /// `frameName` of the honor group, when present.
    pub frame_name: Option<String>,
    /// Whether this is a Live Master honor: it has a mission type and takes
    /// its image and rarity from the level row.
    pub is_live_master: bool,
    /// Whether the honor has more than one level; see
    /// [`Self::draws_level_stars`].
    pub has_star: bool,
    /// The level the honor was resolved for.
    pub honor_level: i32,
    /// `honorMissionType`, used to look up mission progress.
    pub honor_mission_type: Option<String>,
}

impl ResolvedHonor {
    pub(crate) fn asset_plan(
        &self,
        full_size: bool,
    ) -> sekai_profile_core::masterdata::StandardHonorAssetPlan {
        self.clone()
            .into_core()
            .asset_plan(self.honor_level, full_size)
    }

    /// Wraps a core resolution with the level it was resolved for.
    pub fn from_core(
        value: sekai_profile_core::masterdata::ResolvedHonor,
        honor_level: i32,
    ) -> Self {
        Self {
            asset_bundle_name: value.asset_bundle_name,
            honor_rarity: value.honor_rarity,
            honor_type: value.honor_type,
            background_asset_bundle_name: value.background_asset_bundle_name,
            frame_name: value.frame_name,
            is_live_master: value.is_live_master,
            has_star: value.has_star,
            honor_level,
            honor_mission_type: value.honor_mission_type,
        }
    }

    /// See [`sekai_profile_core::masterdata::ResolvedHonor::draws_level_stars`].
    pub fn draws_level_stars(&self) -> bool {
        self.clone().into_core().draws_level_stars()
    }

    fn into_core(self) -> sekai_profile_core::masterdata::ResolvedHonor {
        sekai_profile_core::masterdata::ResolvedHonor {
            asset_bundle_name: self.asset_bundle_name,
            honor_rarity: self.honor_rarity,
            honor_type: self.honor_type,
            background_asset_bundle_name: self.background_asset_bundle_name,
            frame_name: self.frame_name,
            is_live_master: self.is_live_master,
            has_star: self.has_star,
            honor_mission_type: self.honor_mission_type,
        }
    }

    /// Returns the bundle that owns the degree layer. CN limited-event fan
    /// honors ship `honor_top_*` as rank overlays only and share the cheer-team
    /// degree layer; their honorGroups rows omit backgroundAssetbundleName.
    pub fn effective_background_asset_bundle_name(&self) -> &str {
        sekai_profile_core::masterdata::effective_honor_background_asset_bundle_name(
            &self.honor_type,
            &self.asset_bundle_name,
            self.background_asset_bundle_name.as_deref(),
        )
    }

    /// Whether the honor draws a rank overlay on top of its degree layer.
    pub fn has_rank_overlay(&self) -> bool {
        sekai_profile_core::masterdata::honor_has_rank_overlay(
            &self.honor_type,
            &self.asset_bundle_name,
            self.is_live_master,
        )
    }
}

#[cfg(test)]
mod resolved_honor_tests {
    use super::ResolvedHonor;

    fn honor(asset: &str, honor_type: &str, background: Option<&str>) -> ResolvedHonor {
        ResolvedHonor {
            asset_bundle_name: asset.into(),
            honor_rarity: "high".into(),
            honor_type: honor_type.into(),
            background_asset_bundle_name: background.map(str::to_owned),
            frame_name: None,
            is_live_master: false,
            has_star: false,
            honor_level: 1,
            honor_mission_type: None,
        }
    }

    #[test]
    fn cn_limited_event_top_honor_uses_shared_cheer_team_degree_layer() {
        let resolved = honor("honor_top_000020", "limitevent", None);
        assert_eq!(
            resolved.effective_background_asset_bundle_name(),
            "honor_bg_event_cheerteam"
        );
        assert!(!resolved.has_rank_overlay());
    }

    #[test]
    fn explicit_background_remains_authoritative() {
        let resolved = honor("honor_top_000020", "limitevent", Some("honor_bg_explicit"));
        assert_eq!(
            resolved.effective_background_asset_bundle_name(),
            "honor_bg_explicit"
        );
    }
}

/// The masterdata lookups the renderer performs.
///
/// Implement it to serve masterdata from your own storage, or use
/// [`JsonMasterDataProvider`], which reads the game's JSON tables. A lookup
/// returns `None` when its table or row is missing.
pub trait MasterDataProvider: Send + Sync {
    /// Asset key of the banner of a favorite story: `story_type` is
    /// `event_story` or `unit_story`, `story_id` the `eventStories` or
    /// `unitStoryEpisodeGroups` row.
    fn resolve_story_banner(&self, story_type: &str, story_id: i32) -> Option<String>;
    /// The `cards` row `card_id`.
    fn get_card(&self, card_id: i32) -> Option<CardEntry>;
    /// The `customProfileTextColors` row `color_id`, parsed from `colorCode`.
    fn resolve_color(&self, color_id: i32) -> Option<ResolvedColor>;
    /// Color of the first `customProfileTextColors` row, or `None` when the
    /// table is empty or not loaded (the default).
    fn default_color(&self) -> Option<ResolvedColor> {
        None
    }
    /// Font family name of the `customProfileTextFonts` row `font_id`, as the
    /// renderer looks it up in its font directories.
    fn resolve_font(&self, font_id: i32) -> Option<String>;
    /// Asset bundle name of the `stamps` row `stamp_id`.
    fn resolve_stamp(&self, stamp_id: i32) -> Option<String>;
    /// Row `id` of the resource table named by `res_type`: `shape`, `etc`,
    /// `collection`, `general_bg`, `standing`, `player_info`, `story_bg`,
    /// `character_icon`, `material` or `user_interface_icon`.
    fn resolve_resource(&self, res_type: &str, id: i32) -> Option<ResourceInfo>;
    /// The standard honor `honor_id` resolved for `honor_level`, from the
    /// `honors` row and its `honorGroups` row.
    fn resolve_honor(&self, honor_id: i32, honor_level: i32) -> Option<ResolvedHonor>;
    /// The `bondsHonors` row `id`.
    fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry>;
    /// The `bondsHonorWords` row `word_id`.
    fn get_bonds_honor_word(&self, word_id: i64) -> Option<BondsHonorWordEntry>;
    /// The `honors` row `honor_id`.
    fn get_honor(&self, honor_id: i32) -> Option<HonorEntry>;
    /// The `gameCharacterUnits` ID a bonds honor shows for character unit
    /// `self_id` when its view type asks for the unit's virtual singer. A
    /// virtual singer paired with `partner_id` takes the unit entry of the
    /// partner's unit; any other character, or a lookup that fails, keeps
    /// `self_id`.
    fn resolve_unit_vs_sd(&self, self_id: i32, partner_id: i32) -> i32;
    /// Number of `customProfileTextFonts` rows.
    fn font_count(&self) -> usize;
    /// Number of `customProfileTextColors` rows.
    fn color_count(&self) -> usize;

    /// Region of the game client the tables come from. Defaults to
    /// [`Region::Cn`](crate::region::Region::Cn).
    fn region(&self) -> crate::region::Region {
        crate::region::Region::Cn
    }

    /// The `name` of `customProfilePlayerInfoResources` row `id`: the
    /// localized title of a player info panel. Returns `None` by default.
    fn resolve_player_info_label(&self, _id: i32) -> Option<String> {
        None
    }

    /// Localized text for a General panel label key, overriding the built-in
    /// strings of `sekai_profile_core::locale`. Returns `None` by default,
    /// which keeps the built-in text.
    fn resolve_localized_text(&self, _key: &str) -> Option<String> {
        None
    }

    /// The `omikujis` row `id`: the fortune an omikuji collection item shows.
    /// Returns `None` when the table or the row is missing (the default); the
    /// item is then not drawn.
    fn resolve_omikuji(&self, _id: i32) -> Option<OmikujiRow> {
        None
    }

    /// Asset path (`<key>.png`) of an element image. For resource-backed types
    /// (`etc`, `collection`, `general_bg`, `standing`, `player_info`,
    /// `story_bg`) the key comes from [`Self::resolve_resource`]; other types,
    /// and resources the table lacks, use `<element_type>/<id>.png`.
    fn resolve_asset_path(&self, element_type: &str, id: i32) -> String {
        match element_type {
            "etc" | "collection" | "general_bg" | "standing" | "player_info" | "story_bg" => {
                if let Some(info) = self.resolve_resource(element_type, id) {
                    return format!("{}.png", info.asset_key());
                }
                format!("{}/{}.png", element_type, id)
            }
            "card_member" => format!("card_member/{}.png", id),
            "stamp" => format!("stamp/{}.png", id),
            "honor" => format!("honor/{}.png", id),
            "bonds_honor" => format!("bonds_honor/{}.png", id),
            _ => format!("{}/{}.png", element_type, id),
        }
    }
}

/// A cheaply clonable handle to a [`MasterDataProvider`], which the renderer
/// holds and passes to every lookup.
///
/// Each method forwards to the provider method of the same name; the
/// `*_or_default` methods add the game's fallback rules.
#[derive(Clone)]
pub struct MasterData {
    provider: Arc<dyn MasterDataProvider>,
}

impl MasterData {
    /// Wraps a provider.
    pub fn new(provider: Arc<dyn MasterDataProvider>) -> Self {
        Self { provider }
    }

    /// See [`MasterDataProvider::resolve_story_banner`].
    pub fn resolve_story_banner(&self, story_type: &str, story_id: i32) -> Option<String> {
        self.provider.resolve_story_banner(story_type, story_id)
    }

    /// See [`MasterDataProvider::get_card`].
    pub fn get_card(&self, card_id: i32) -> Option<CardEntry> {
        self.provider.get_card(card_id)
    }

    /// See [`MasterDataProvider::resolve_color`].
    pub fn resolve_color(&self, color_id: i32) -> Option<ResolvedColor> {
        self.provider.resolve_color(color_id)
    }

    /// Color an element draws with: an unknown `colorId` falls back to the
    /// first row of the color table. See
    /// [`sekai_profile_core::masterdata::ProfileMasterData::resolve_color_or_default`].
    pub fn resolve_color_or_default(&self, color_id: i32) -> Option<ResolvedColor> {
        sekai_profile_core::masterdata::ProfileMasterData::resolve_color_or_default(self, color_id)
            .map(|value| ResolvedColor {
                r: value.r,
                g: value.g,
                b: value.b,
                a: value.a,
            })
    }

    /// See [`MasterDataProvider::resolve_font`].
    pub fn resolve_font(&self, font_id: i32) -> Option<String> {
        self.provider.resolve_font(font_id)
    }

    /// Font family a text element draws with: an unknown `fontId` keeps the
    /// default font. See
    /// [`sekai_profile_core::masterdata::ProfileMasterData::resolve_font_or_default`].
    pub fn resolve_font_or_default(&self, font_id: i32) -> Option<String> {
        sekai_profile_core::masterdata::ProfileMasterData::resolve_font_or_default(self, font_id)
    }

    /// See [`MasterDataProvider::resolve_stamp`].
    pub fn resolve_stamp(&self, stamp_id: i32) -> Option<String> {
        self.provider.resolve_stamp(stamp_id)
    }

    /// See [`MasterDataProvider::resolve_resource`].
    pub fn resolve_resource(&self, res_type: &str, id: i32) -> Option<ResourceInfo> {
        self.provider.resolve_resource(res_type, id)
    }

    /// See [`MasterDataProvider::resolve_honor`].
    pub fn resolve_honor(&self, honor_id: i32, honor_level: i32) -> Option<ResolvedHonor> {
        self.provider.resolve_honor(honor_id, honor_level)
    }

    /// See [`MasterDataProvider::get_bonds_honor`].
    pub fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry> {
        self.provider.get_bonds_honor(id)
    }

    /// See [`MasterDataProvider::get_bonds_honor_word`].
    pub fn get_bonds_honor_word(&self, word_id: i64) -> Option<BondsHonorWordEntry> {
        self.provider.get_bonds_honor_word(word_id)
    }

    /// See [`MasterDataProvider::get_honor`].
    pub fn get_honor(&self, honor_id: i32) -> Option<HonorEntry> {
        self.provider.get_honor(honor_id)
    }

    /// See [`MasterDataProvider::resolve_unit_vs_sd`].
    pub fn resolve_unit_vs_sd(&self, self_id: i32, partner_id: i32) -> i32 {
        self.provider.resolve_unit_vs_sd(self_id, partner_id)
    }

    /// See [`MasterDataProvider::font_count`].
    pub fn font_count(&self) -> usize {
        self.provider.font_count()
    }

    /// See [`MasterDataProvider::color_count`].
    pub fn color_count(&self) -> usize {
        self.provider.color_count()
    }

    /// See [`MasterDataProvider::resolve_asset_path`].
    pub fn resolve_asset_path(&self, element_type: &str, id: i32) -> String {
        self.provider.resolve_asset_path(element_type, id)
    }

    /// See [`MasterDataProvider::region`].
    pub fn region(&self) -> crate::region::Region {
        self.provider.region()
    }

    /// See [`MasterDataProvider::resolve_player_info_label`].
    pub fn resolve_player_info_label(&self, id: i32) -> Option<String> {
        self.provider.resolve_player_info_label(id)
    }

    /// See [`MasterDataProvider::resolve_localized_text`].
    pub fn resolve_localized_text(&self, key: &str) -> Option<String> {
        self.provider.resolve_localized_text(key)
    }

    /// See [`MasterDataProvider::resolve_omikuji`].
    pub fn resolve_omikuji(&self, id: i32) -> Option<OmikujiRow> {
        self.provider.resolve_omikuji(id)
    }
}

impl sekai_profile_core::masterdata::ProfileMasterData for MasterData {
    fn resolve_story_banner(&self, story_type: &str, story_id: i32) -> Option<String> {
        MasterData::resolve_story_banner(self, story_type, story_id)
    }

    fn get_card(&self, card_id: i32) -> Option<sekai_profile_core::masterdata::CardEntry> {
        MasterData::get_card(self, card_id).map(|value| sekai_profile_core::masterdata::CardEntry {
            id: value.id,
            asset_bundle_name: value.asset_bundle_name,
            card_rarity_type: value.card_rarity_type,
            attr: value.attr,
            character_id: value.character_id,
        })
    }

    fn resolve_color(
        &self,
        color_id: i32,
    ) -> Option<sekai_profile_core::masterdata::ResolvedColor> {
        MasterData::resolve_color(self, color_id).map(|value| {
            sekai_profile_core::masterdata::ResolvedColor {
                r: value.r,
                g: value.g,
                b: value.b,
                a: value.a,
            }
        })
    }

    fn default_color(&self) -> Option<sekai_profile_core::masterdata::ResolvedColor> {
        self.provider
            .default_color()
            .map(|value| sekai_profile_core::masterdata::ResolvedColor {
                r: value.r,
                g: value.g,
                b: value.b,
                a: value.a,
            })
    }

    fn resolve_font(&self, font_id: i32) -> Option<String> {
        MasterData::resolve_font(self, font_id)
    }

    fn resolve_stamp(&self, stamp_id: i32) -> Option<String> {
        MasterData::resolve_stamp(self, stamp_id)
    }

    fn resolve_resource(
        &self,
        resource_type: &str,
        id: i32,
    ) -> Option<sekai_profile_core::masterdata::ResourceInfo> {
        MasterData::resolve_resource(self, resource_type, id).map(|value| {
            sekai_profile_core::masterdata::ResourceInfo {
                file_name: value.file_name,
                load_value: value.load_val,
                resource_type: value.resource_type,
                collection_type: value.collection_type,
            }
        })
    }

    fn resolve_honor(
        &self,
        honor_id: i32,
        honor_level: i32,
    ) -> Option<sekai_profile_core::masterdata::ResolvedHonor> {
        MasterData::resolve_honor(self, honor_id, honor_level).map(ResolvedHonor::into_core)
    }

    fn get_bonds_honor(&self, id: i32) -> Option<sekai_profile_core::masterdata::BondsHonorEntry> {
        MasterData::get_bonds_honor(self, id).map(|value| {
            sekai_profile_core::masterdata::BondsHonorEntry {
                id: value.id,
                game_character_unit_id1: value.game_character_unit_id1,
                game_character_unit_id2: value.game_character_unit_id2,
                honor_rarity: value.honor_rarity,
            }
        })
    }

    fn get_bonds_honor_word(
        &self,
        id: i64,
    ) -> Option<sekai_profile_core::masterdata::BondsHonorWordEntry> {
        MasterData::get_bonds_honor_word(self, id).map(|value| {
            sekai_profile_core::masterdata::BondsHonorWordEntry {
                id: i64::from(value.id),
                assetbundle_name: value.assetbundle_name,
            }
        })
    }

    fn resolve_unit_virtual_singer(&self, self_id: i32, partner_id: i32) -> i32 {
        MasterData::resolve_unit_vs_sd(self, self_id, partner_id)
    }

    fn resolve_localized_text(&self, key: &str) -> Option<String> {
        MasterData::resolve_localized_text(self, key)
    }

    fn resolve_omikuji(&self, id: i32) -> Option<OmikujiRow> {
        MasterData::resolve_omikuji(self, id)
    }

    fn region_code(&self) -> Option<&str> {
        Some(MasterData::region(self).as_str())
    }
}
