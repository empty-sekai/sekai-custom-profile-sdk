//! Backend-neutral master-data queries used by profile resolution.

use std::collections::BTreeMap;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const PROFILE_MASTERDATA_TABLES: &[&str] = &[
    "cards",
    "stamps",
    "honors",
    "honorGroups",
    "bondsHonors",
    "bondsHonorWords",
    "gameCharacterUnits",
    "customProfileTextColors",
    "customProfileTextFonts",
    "customProfileShapeResources",
    "customProfileEtcResources",
    "customProfileCollectionResources",
    "customProfileGeneralBackgroundResources",
    "customProfileMemberStandingPictureResources",
    "customProfileStoryBackgroundResources",
    "eventStories",
    "unitStoryEpisodeGroups",
];

/// Tables a table set may lack: the icon resource tables, which only some
/// regions ship, and `omikujis`, which only omikuji collections read. Without
/// one, the elements that draw from it have no master-data row and are left
/// out; nothing else changes.
pub const PROFILE_OPTIONAL_MASTERDATA_TABLES: &[&str] = &[
    "customProfileCharacterIconResources",
    "customProfileMaterialResources",
    "customProfileUserInterfaceIconResources",
    "omikujis",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl ResolvedColor {
    pub fn from_hex(value: &str) -> Option<Self> {
        let value = value.strip_prefix('#').unwrap_or(value);
        // Byte-based: the length is measured in bytes, and a string slice at
        // a non-char-boundary index would panic on multi-byte input. Invalid
        // digits yield None rather than a panic.
        let bytes = value.as_bytes();
        if bytes.len() != 6 && bytes.len() != 8 {
            return None;
        }
        let hex2 = |range: std::ops::Range<usize>| {
            std::str::from_utf8(&bytes[range])
                .ok()
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        };
        Some(Self {
            r: hex2(0..2)?,
            g: hex2(2..4)?,
            b: hex2(4..6)?,
            a: if bytes.len() == 8 { hex2(6..8)? } else { 255 },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceInfo {
    pub file_name: String,
    pub load_value: String,
    pub resource_type: String,
    /// `customProfileResourceCollectionType` of the row; rows of the other
    /// resource tables carry none.
    #[serde(default)]
    pub collection_type: CollectionResourceType,
}

/// Kind of a `customProfileCollectionResources` row, from its
/// `customProfileResourceCollectionType` column.
///
/// Only [`Self::Omikuji`] and [`Self::CanBadge`] draw differently from a plain
/// image: an omikuji row names a fortune-slip prefab ([`crate::omikuji`]), and
/// a can badge draws its image with the lit material of
/// [`crate::badge_material`]. Every other kind draws the row's image as is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectionResourceType {
    /// `none`, and also a missing, `null` or unrecognised value.
    #[default]
    None,
    Omikuji,
    CanBadge,
    Keyholder,
    AcrylicStand,
    Sticker,
    Towel,
    SilverTape,
    TicketHolder,
    Tapestry,
}

impl CollectionResourceType {
    /// Reads a master-data value. A missing or unrecognised value is
    /// [`Self::None`], which draws like any plain image.
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("omikuji") => Self::Omikuji,
            Some("can_badge") => Self::CanBadge,
            Some("keyholder") => Self::Keyholder,
            Some("acrylic_stand") => Self::AcrylicStand,
            Some("sticker") => Self::Sticker,
            Some("towel") => Self::Towel,
            Some("silver_tape") => Self::SilverTape,
            Some("ticket_holder") => Self::TicketHolder,
            Some("tapestry") => Self::Tapestry,
            _ => Self::None,
        }
    }
}

/// One `omikujis` row: the fortune an omikuji collection shows, picked by the
/// element's `targetId`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmikujiRow {
    pub id: i32,
    /// Unit whose colour the title backgrounds take (`piapro`, `light_sound`,
    /// ...).
    pub unit: String,
    pub summary: String,
    pub title1: String,
    pub description1: String,
    pub title2: String,
    pub description2: String,
    pub title3: String,
    pub description3: String,
    pub fortune_assetbundle_name: String,
    pub fortune_file_path: String,
    pub omikuji_cover_assetbundle_name: String,
    pub omikuji_cover_file_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardEntry {
    pub id: i32,
    #[serde(rename = "assetbundleName")]
    pub asset_bundle_name: String,
    pub card_rarity_type: String,
    pub attr: String,
    #[serde(default)]
    pub character_id: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HonorLevelEntry {
    pub level: i32,
    pub assetbundle_name: Option<String>,
    pub honor_rarity: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HonorEntry {
    pub id: i32,
    pub assetbundle_name: Option<String>,
    pub honor_rarity: Option<String>,
    pub group_id: Option<i32>,
    #[serde(default)]
    pub levels: Vec<HonorLevelEntry>,
    pub honor_mission_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedHonor {
    pub asset_bundle_name: String,
    pub honor_rarity: String,
    pub honor_type: String,
    pub background_asset_bundle_name: Option<String>,
    pub frame_name: Option<String>,
    pub is_live_master: bool,
    pub has_star: bool,
    pub honor_mission_type: Option<String>,
}

/// Resolves the bundle that owns a standard honor's degree layer.
///
/// This is a backend-neutral game-data rule. Native and browser renderers
/// must call it instead of maintaining independent bundle fallbacks.
pub fn effective_honor_background_asset_bundle_name<'a>(
    honor_type: &str,
    asset_bundle_name: &'a str,
    background_asset_bundle_name: Option<&'a str>,
) -> &'a str {
    if let Some(background) = background_asset_bundle_name.filter(|value| !value.is_empty()) {
        return background;
    }
    if honor_type == "limitevent" && asset_bundle_name.starts_with("honor_top_") {
        return "honor_bg_event_cheerteam";
    }
    asset_bundle_name
}

/// Returns whether a standard honor owns a rank/progress overlay layer.
///
/// Live Master honors draw their scroll and rank-match honors their rank
/// plate. Otherwise only event and SEKAI ECHO honors draw the `rank_main` /
/// `rank_sub` art of their own bundle; other honor types never show it,
/// whatever their bundle contains.
pub fn honor_has_rank_overlay(
    honor_type: &str,
    asset_bundle_name: &str,
    is_live_master: bool,
) -> bool {
    is_live_master
        || honor_type == "rank_match"
        || (matches!(honor_type, "event" | "sekai_echo") && !asset_bundle_name.is_empty())
}

/// Number of level stars drawn for `level`. Levels 1 to 10 show that many
/// stars and higher levels start over, so level 11 shows one star again;
/// the sixth to tenth stars replace the first five with the upgraded star.
pub fn honor_level_star_count(level: i32) -> i32 {
    if level <= 0 {
        0
    } else {
        (level - 1) % 10 + 1
    }
}

/// Numeric suffix of an honor rarity in frame and word asset names.
pub fn honor_rarity_number(rarity: &str) -> u8 {
    match rarity {
        "low" => 1,
        "middle" => 2,
        "high" => 3,
        _ => 4,
    }
}

/// Resource identities for one standard honor. Frame candidates are ordered
/// alternatives, not independent requirements: custom first, then default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardHonorAssetPlan {
    pub background: crate::ResourceKey,
    pub overlay: Option<crate::ResourceKey>,
    pub frame_candidates: [Option<crate::ResourceKey>; 2],
    pub star: Option<crate::ResourceKey>,
    pub star_high: Option<crate::ResourceKey>,
}

impl StandardHonorAssetPlan {
    /// All possible source keys, including both sides of the frame fallback.
    pub fn resources(&self) -> impl Iterator<Item = &crate::ResourceKey> {
        std::iter::once(&self.background)
            .chain(self.overlay.iter())
            .chain(self.frame_candidates.iter().flatten())
            .chain(self.star.iter())
            .chain(self.star_high.iter())
    }
}

impl ResolvedHonor {
    pub fn asset_plan(&self, level: i32, full_size: bool) -> StandardHonorAssetPlan {
        let resource = |namespace: &str, key: String| crate::ResourceKey {
            namespace: namespace.into(),
            key,
        };
        let (suffix, size_char) = if full_size {
            ("main", "m")
        } else {
            ("sub", "s")
        };
        let directory = if self.honor_type == "rank_match" {
            "rank_live/honor"
        } else {
            "honor"
        };
        let rarity = honor_rarity_number(&self.honor_rarity);
        let frame_candidates = [
            self.frame_name
                .as_ref()
                .filter(|_| self.uses_dedicated_frame())
                .map(|name| {
                    resource(
                        "assets",
                        format!("honor_frame/{name}/frame_degree_{size_char}_{rarity}"),
                    )
                }),
            Some(resource(
                "static",
                format!("honor/frame_degree_{size_char}_{rarity}"),
            )),
        ];
        let overlay = self.has_rank_overlay().then(|| {
            let name = if self.honor_type == "rank_match" {
                suffix.into()
            } else if self.is_live_master {
                "scroll".into()
            } else {
                format!("rank_{suffix}")
            };
            resource(
                "assets",
                format!("{directory}/{}/{name}", self.asset_bundle_name),
            )
        });
        let star_count = if self.draws_level_stars() {
            honor_level_star_count(level)
        } else {
            0
        };
        StandardHonorAssetPlan {
            background: resource(
                "assets",
                format!(
                    "{directory}/{}/degree_{suffix}",
                    self.effective_background_asset_bundle_name(),
                ),
            ),
            overlay,
            frame_candidates,
            star: (star_count > 0).then(|| resource("static", "honor/icon_degreeLv".into())),
            star_high: (star_count > 5).then(|| resource("static", "honor/icon_degreeLv6".into())),
        }
    }

    pub fn effective_background_asset_bundle_name(&self) -> &str {
        effective_honor_background_asset_bundle_name(
            &self.honor_type,
            &self.asset_bundle_name,
            self.background_asset_bundle_name.as_deref(),
        )
    }

    pub fn has_rank_overlay(&self) -> bool {
        honor_has_rank_overlay(
            &self.honor_type,
            &self.asset_bundle_name,
            self.is_live_master,
        )
    }

    /// Whether the honor draws the shared level stars. Single-level honors,
    /// Live Master honors (which show their clear count instead), event
    /// honors and birthday honors (which carry their own level art) draw none.
    pub fn draws_level_stars(&self) -> bool {
        self.has_star
            && !self.is_live_master
            && !matches!(self.honor_type.as_str(), "event" | "birthday")
    }

    /// Whether the group's own frame replaces the shared rarity frame.
    /// Birthday honors use it from the middle rarity up, every other honor
    /// only at the high and highest rarities.
    pub fn uses_dedicated_frame(&self) -> bool {
        let rarity = honor_rarity_number(&self.honor_rarity);
        self.frame_name
            .as_deref()
            .is_some_and(|name| !name.is_empty())
            && if self.honor_type == "birthday" {
                rarity >= 2
            } else {
                rarity >= 3
            }
    }
}

/// Resolves one `honors` row and its `honorGroups` row into render inputs.
///
/// Every master-data provider resolves honors through this function so the
/// level, rarity and group fallbacks stay identical across backends.
pub fn resolve_honor_rows(
    honor: &Value,
    group: Option<&Value>,
    honor_level: i32,
) -> Option<ResolvedHonor> {
    let honor: HonorEntry = serde_json::from_value(honor.clone()).ok()?;
    let live = honor.honor_mission_type.is_some() && honor.assetbundle_name.is_none();
    // A level the masterdata does not know yet (data lagging behind player
    // progress) falls back to the last known level, matching the game's
    // lookup, rather than producing an empty asset bundle name.
    let level = honor
        .levels
        .iter()
        .find(|entry| entry.level == honor_level)
        .or_else(|| honor.levels.last());
    let group_text = |field: &str| {
        group
            .and_then(|value| value.get(field))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    Some(ResolvedHonor {
        asset_bundle_name: if live {
            level
                .and_then(|v| v.assetbundle_name.clone())
                .unwrap_or_default()
        } else {
            honor.assetbundle_name.unwrap_or_default()
        },
        honor_rarity: if live {
            level
                .and_then(|v| v.honor_rarity.clone())
                .unwrap_or_else(|| "low".into())
        } else {
            honor.honor_rarity.unwrap_or_else(|| "low".into())
        },
        honor_type: group_text("honorType").unwrap_or_else(|| "normal".into()),
        background_asset_bundle_name: group_text("backgroundAssetbundleName"),
        frame_name: group_text("frameName"),
        is_live_master: live,
        has_star: honor.levels.len() > 1,
        honor_mission_type: honor.honor_mission_type,
    })
}

/// Resource identities for one bonds honor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BondsHonorAssetPlan {
    /// Character units in draw order after the optional virtual-singer
    /// substitution, which only affects the character art.
    pub character_ids: [i32; 2],
    /// Left and right background halves, keyed by the listed character units.
    pub backgrounds: [crate::ResourceKey; 2],
    pub characters: [crate::ResourceKey; 2],
    pub mask: crate::ResourceKey,
    pub frame: crate::ResourceKey,
    /// Word art; only the full-size slot shows it.
    pub word: Option<crate::ResourceKey>,
    pub star: crate::ResourceKey,
    pub star_high: crate::ResourceKey,
}

impl BondsHonorAssetPlan {
    /// Every source key the honor can draw.
    pub fn resources(&self) -> impl Iterator<Item = &crate::ResourceKey> {
        self.backgrounds
            .iter()
            .chain(self.characters.iter())
            .chain([&self.mask, &self.frame])
            .chain(self.word.iter())
            .chain([&self.star, &self.star_high])
    }
}

/// Plans the resources of one bonds honor, or `None` when the master data
/// does not list it.
pub fn bonds_honor_asset_plan(
    masterdata: &(impl ProfileMasterData + ?Sized),
    bonds_honor_id: i32,
    full_size: bool,
    word_id: i64,
    inverse: bool,
    use_unit_virtual_singer: bool,
) -> Option<BondsHonorAssetPlan> {
    let entry = masterdata.get_bonds_honor(bonds_honor_id)?;
    let (first, second) = if inverse {
        (entry.game_character_unit_id2, entry.game_character_unit_id1)
    } else {
        (entry.game_character_unit_id1, entry.game_character_unit_id2)
    };
    let character_ids = if use_unit_virtual_singer {
        [
            masterdata.resolve_unit_virtual_singer(first, second),
            masterdata.resolve_unit_virtual_singer(second, first),
        ]
    } else {
        [first, second]
    };
    let resource = |namespace: &str, key: String| crate::ResourceKey {
        namespace: namespace.into(),
        key,
    };
    let (size_char, slot) = if full_size {
        ("m", "main")
    } else {
        ("s", "sub")
    };
    let background = |unit: i32| {
        resource(
            "static",
            if full_size {
                format!("honor/bonds/{unit}")
            } else {
                format!("honor/bonds/{unit}_sub")
            },
        )
    };
    let character = |unit: i32| resource("assets", format!("bonds_honor/chr_sd_{unit:02}_01"));
    let rarity = honor_rarity_number(&entry.honor_rarity);
    Some(BondsHonorAssetPlan {
        character_ids,
        backgrounds: [background(first), background(second)],
        characters: [character(character_ids[0]), character(character_ids[1])],
        mask: resource("static", format!("honor/mask_degree_{slot}")),
        frame: resource("static", format!("honor/frame_degree_{size_char}_{rarity}")),
        word: full_size
            .then(|| masterdata.get_bonds_honor_word(word_id))
            .flatten()
            .map(|word| {
                resource(
                    "assets",
                    format!("bonds_honor/word/{}_{rarity:02}", word.assetbundle_name),
                )
            }),
        star: resource("static", "honor/icon_degreeLv".into()),
        star_high: resource("static", "honor/icon_degreeLv6".into()),
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BondsHonorEntry {
    pub id: i32,
    pub game_character_unit_id1: i32,
    pub game_character_unit_id2: i32,
    pub honor_rarity: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BondsHonorWordEntry {
    pub id: i64,
    pub assetbundle_name: String,
}

/// Font id drawn for a text element whose `fontId` has no
/// `customProfileTextFonts` row. The game keeps the text component's default
/// font in that case; font 1 stands in for it.
pub const DEFAULT_TEXT_FONT_ID: i32 = 1;

pub trait ProfileMasterData {
    fn resolve_story_banner(&self, story_type: &str, story_id: i32) -> Option<String>;
    fn get_card(&self, card_id: i32) -> Option<CardEntry>;
    fn resolve_color(&self, color_id: i32) -> Option<ResolvedColor>;
    /// Colour of the first `customProfileTextColors` row, or `None` when the
    /// table is empty or not loaded.
    fn default_color(&self) -> Option<ResolvedColor> {
        None
    }
    /// Colour a text or shape element draws with. A `colorId` the table does
    /// not contain uses the table's first row, as the game does.
    fn resolve_color_or_default(&self, color_id: i32) -> Option<ResolvedColor> {
        self.resolve_color(color_id)
            .or_else(|| self.default_color())
    }
    fn resolve_font(&self, font_id: i32) -> Option<String>;
    /// Font family a text element draws with. A `fontId` the table does not
    /// contain keeps the default face ([`DEFAULT_TEXT_FONT_ID`]) instead of
    /// failing the element.
    fn resolve_font_or_default(&self, font_id: i32) -> Option<String> {
        self.resolve_font(font_id)
            .or_else(|| self.resolve_font(DEFAULT_TEXT_FONT_ID))
    }
    fn resolve_stamp(&self, stamp_id: i32) -> Option<String>;
    fn resolve_resource(&self, resource_type: &str, id: i32) -> Option<ResourceInfo>;
    fn resolve_honor(&self, honor_id: i32, honor_level: i32) -> Option<ResolvedHonor>;
    fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry>;
    fn get_bonds_honor_word(&self, id: i64) -> Option<BondsHonorWordEntry>;
    fn resolve_unit_virtual_singer(&self, self_id: i32, partner_id: i32) -> i32;
    fn resolve_localized_text(&self, _key: &str) -> Option<String> {
        None
    }
    /// The `omikujis` row `id`, or `None` when the table or the row is
    /// missing (the table is optional) or the row lacks a field.
    fn resolve_omikuji(&self, _id: i32) -> Option<OmikujiRow> {
        None
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum MasterDataError {
    #[error("master-data table {table} is not a JSON array or object")]
    InvalidTable { table: String },
}

#[derive(Clone, Debug)]
struct JsonTable {
    rows: Vec<Value>,
    index: BTreeMap<i64, usize>,
}

impl JsonTable {
    fn new(name: &str, value: Value) -> Result<Self, MasterDataError> {
        let rows = match value {
            Value::Array(rows) => rows,
            Value::Object(_) => vec![value],
            _ => return Err(MasterDataError::InvalidTable { table: name.into() }),
        };
        let index = rows
            .iter()
            .enumerate()
            .filter_map(|(position, row)| {
                row.get("id")
                    .and_then(Value::as_i64)
                    .map(|id| (id, position))
            })
            .collect();
        Ok(Self { rows, index })
    }
    fn get(&self, id: i64) -> Option<&Value> {
        self.index.get(&id).map(|index| &self.rows[*index])
    }
    fn typed<T: DeserializeOwned>(&self, id: i64) -> Option<T> {
        serde_json::from_value(self.get(id)?.clone()).ok()
    }
}

/// Canonicalizes a region code or locale tag to one of `cn`, `jp`, `tw`, `en`
/// or `kr`, ignoring case and surrounding whitespace. This is the only region
/// alias table: the CN font-name mapping and [`crate::locale`] both use it.
/// Unknown values are returned lowercased.
pub fn normalize_region(region: &str) -> String {
    match region.trim().to_ascii_lowercase().as_str() {
        "cn" | "sc" | "zh-cn" | "zh-hans" => "cn".into(),
        "jp" | "ja" | "ja-jp" => "jp".into(),
        "tw" | "tc" | "zh-tw" | "zh-hant" => "tw".into(),
        "en" | "world" | "en-us" | "en-gb" => "en".into(),
        "kr" | "ko" | "ko-kr" => "kr".into(),
        other => other.into(),
    }
}

/// Parsed table collection. It contains no network or filesystem policy.
#[derive(Clone, Debug)]
pub struct JsonMasterData {
    region: String,
    tables: BTreeMap<String, JsonTable>,
}

impl JsonMasterData {
    pub fn new(region: impl Into<String>) -> Self {
        Self {
            region: normalize_region(&region.into()),
            tables: BTreeMap::new(),
        }
    }
    pub fn insert_value(&mut self, name: &str, value: Value) -> Result<(), MasterDataError> {
        self.tables
            .insert(name.into(), JsonTable::new(name, value)?);
        Ok(())
    }
    pub fn insert_json(&mut self, name: &str, json: &str) -> Result<(), serde_json::Error> {
        let value = serde_json::from_str(json)?;
        self.insert_value(name, value).map_err(|error| {
            serde_json::Error::io(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        })
    }
    pub fn loaded_tables(&self) -> impl Iterator<Item = &str> {
        self.tables.keys().map(String::as_str)
    }
    fn table(&self, name: &str) -> Option<&JsonTable> {
        self.tables.get(name)
    }
}

impl ProfileMasterData for JsonMasterData {
    fn resolve_story_banner(&self, story_type: &str, story_id: i32) -> Option<String> {
        let (table, suffix) = match story_type {
            "event_story" => ("eventStories", "banner_event_story"),
            "unit_story" => ("unitStoryEpisodeGroups", "banner_unit_story"),
            _ => return None,
        };
        let row = self.table(table)?.get(story_id.into())?;
        let bundle = row.get("assetbundleName")?.as_str()?;
        Some(format!("{story_type}/{bundle}/screen_image/{suffix}"))
    }
    fn get_card(&self, card_id: i32) -> Option<CardEntry> {
        self.table("cards")?.typed(card_id.into())
    }
    fn resolve_color(&self, color_id: i32) -> Option<ResolvedColor> {
        ResolvedColor::from_hex(
            self.table("customProfileTextColors")?
                .get(color_id.into())?
                .get("colorCode")?
                .as_str()?,
        )
    }
    fn default_color(&self) -> Option<ResolvedColor> {
        ResolvedColor::from_hex(
            self.table("customProfileTextColors")?
                .rows
                .first()?
                .get("colorCode")?
                .as_str()?,
        )
    }
    fn resolve_font(&self, font_id: i32) -> Option<String> {
        let name = self
            .table("customProfileTextFonts")?
            .get(font_id.into())?
            .get("fontName")?
            .as_str()?;
        Some(
            if self.region == "cn" {
                match name {
                    "FOT-RodinNTLGPro-DB" => "FZLanTingHei-DB-GBK",
                    "FOT-SkipProN-B" => "FZZhengHei-EB-GBK",
                    "FOT-PopHappinessStd-EB" => "FZShaoEr-M11-JF",
                    other => other,
                }
            } else {
                name
            }
            .into(),
        )
    }
    fn resolve_stamp(&self, stamp_id: i32) -> Option<String> {
        self.table("stamps")?
            .get(stamp_id.into())?
            .get("assetbundleName")?
            .as_str()
            .map(str::to_owned)
    }
    fn resolve_resource(&self, resource_type: &str, id: i32) -> Option<ResourceInfo> {
        let table = match resource_type {
            "shape" => "customProfileShapeResources",
            "etc" => "customProfileEtcResources",
            "collection" => "customProfileCollectionResources",
            "general_bg" => "customProfileGeneralBackgroundResources",
            "standing" => "customProfileMemberStandingPictureResources",
            "story_bg" => "customProfileStoryBackgroundResources",
            "character_icon" => "customProfileCharacterIconResources",
            "material" => "customProfileMaterialResources",
            "user_interface_icon" => "customProfileUserInterfaceIconResources",
            _ => return None,
        };
        let row = self.table(table)?.get(id.into())?;
        Some(ResourceInfo {
            file_name: row.get("fileName")?.as_str()?.into(),
            load_value: row.get("resourceLoadVal")?.as_str()?.into(),
            resource_type: row.get("customProfileResourceType")?.as_str()?.into(),
            collection_type: CollectionResourceType::parse(
                row.get("customProfileResourceCollectionType")
                    .and_then(Value::as_str),
            ),
        })
    }
    fn resolve_honor(&self, honor_id: i32, honor_level: i32) -> Option<ResolvedHonor> {
        let honor = self.table("honors")?.get(honor_id.into())?;
        let group = honor
            .get("groupId")
            .and_then(Value::as_i64)
            .and_then(|id| self.table("honorGroups")?.get(id));
        resolve_honor_rows(honor, group, honor_level)
    }
    fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry> {
        self.table("bondsHonors")?.typed(id.into())
    }
    fn get_bonds_honor_word(&self, id: i64) -> Option<BondsHonorWordEntry> {
        self.table("bondsHonorWords")?.typed(id)
    }
    fn resolve_omikuji(&self, id: i32) -> Option<OmikujiRow> {
        self.table("omikujis")?.typed(id.into())
    }
    fn resolve_unit_virtual_singer(&self, self_id: i32, partner_id: i32) -> i32 {
        let Some(table) = self.table("gameCharacterUnits") else {
            return self_id;
        };
        let Some(character) = table
            .get(self_id.into())
            .and_then(|v| v.get("gameCharacterId"))
            .and_then(Value::as_i64)
        else {
            return self_id;
        };
        if character < 21 {
            return self_id;
        }
        let Some(unit) = table
            .get(partner_id.into())
            .and_then(|v| v.get("unit"))
            .and_then(Value::as_str)
        else {
            return self_id;
        };
        table
            .rows
            .iter()
            .find(|row| {
                row.get("gameCharacterId").and_then(Value::as_i64) == Some(character)
                    && row.get("unit").and_then(Value::as_str) == Some(unit)
            })
            .and_then(|row| row.get("id"))
            .and_then(Value::as_i64)
            .unwrap_or(self_id.into()) as i32
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn planned_honor() -> ResolvedHonor {
        ResolvedHonor {
            asset_bundle_name: "honor_sample".into(),
            honor_rarity: "low".into(),
            honor_type: "character".into(),
            background_asset_bundle_name: None,
            frame_name: Some("custom_frame".into()),
            is_live_master: false,
            has_star: true,
            honor_mission_type: None,
        }
    }

    #[test]
    fn standard_honor_plan_keeps_ordered_frames_and_namespaces_for_all_rarities() {
        for (rarity, number) in [("low", 1), ("middle", 2), ("high", 3), ("highest", 4)] {
            for (full_size, size) in [(true, "m"), (false, "s")] {
                let mut honor = planned_honor();
                honor.honor_rarity = rarity.into();
                let plan = honor.asset_plan(10, full_size);
                let frames = plan
                    .frame_candidates
                    .iter()
                    .flatten()
                    .map(|key| (key.namespace.as_str(), key.key.clone()))
                    .collect::<Vec<_>>();
                let mut expected = Vec::new();
                if number >= 3 {
                    expected.push((
                        "assets",
                        format!("honor_frame/custom_frame/frame_degree_{size}_{number}"),
                    ));
                }
                expected.push(("static", format!("honor/frame_degree_{size}_{number}")));
                assert_eq!(frames, expected);
                assert_eq!(plan.star.as_ref().unwrap().namespace, "static");
                assert_eq!(plan.star_high.as_ref().unwrap().namespace, "static");
                assert_eq!(plan.resources().count(), 3 + expected.len());
            }
        }
    }

    #[test]
    fn standard_honor_plan_requests_stars_only_when_they_are_drawn() {
        let mut honor = planned_honor();
        for (level, normal, high) in [
            (-1, false, false),
            (0, false, false),
            (1, true, false),
            (5, true, false),
            (6, true, true),
            (10, true, true),
            (11, true, false),
            (20, true, true),
        ] {
            let plan = honor.asset_plan(level, false);
            assert_eq!(
                (plan.star.is_some(), plan.star_high.is_some()),
                (normal, high),
                "level={level}"
            );
        }
        for honor_type in ["event", "birthday"] {
            honor.honor_type = honor_type.into();
            let plan = honor.asset_plan(10, true);
            assert!(plan.star.is_none() && plan.star_high.is_none());
        }
        honor.honor_type = "achievement".into();
        assert!(honor.asset_plan(10, true).star_high.is_some());
        honor.has_star = false;
        assert!(honor.asset_plan(10, true).star.is_none());
        honor.has_star = true;
        honor.is_live_master = true;
        let plan = honor.asset_plan(10, true);
        assert!(plan.star.is_none() && plan.star_high.is_none());
        assert_eq!(plan.overlay.unwrap().key, "honor/honor_sample/scroll");
    }
    #[test]
    fn rank_overlay_follows_the_honor_type() {
        let mut honor = planned_honor();
        for (honor_type, bundle, expected) in [
            ("event", "honor_0305", true),
            ("event", "honor_top_000001", true),
            ("sekai_echo", "honor_0182", true),
            ("event", "", false),
            ("limitevent", "honor_top_000020", false),
            ("character", "honor_memorial_0001", false),
            ("achievement", "honor_0105", false),
        ] {
            honor.honor_type = honor_type.into();
            honor.asset_bundle_name = bundle.into();
            assert_eq!(honor.has_rank_overlay(), expected, "{honor_type} {bundle}");
            let overlay = honor.asset_plan(1, true).overlay;
            assert_eq!(overlay.is_some(), expected, "{honor_type} {bundle}");
            if let Some(overlay) = overlay {
                assert_eq!(overlay.key, format!("honor/{bundle}/rank_main"));
            }
        }
    }

    #[test]
    fn level_stars_follow_the_honor_type() {
        let mut honor = planned_honor();
        for (honor_type, expected) in [
            ("character", true),
            ("achievement", true),
            ("limitevent", true),
            ("normal", true),
            ("event", false),
            ("birthday", false),
        ] {
            honor.honor_type = honor_type.into();
            let plan = honor.asset_plan(7, true);
            assert_eq!(plan.star.is_some(), expected, "{honor_type}");
            assert_eq!(plan.star_high.is_some(), expected, "{honor_type}");
        }
    }

    #[test]
    fn dedicated_frames_need_the_group_rarity() {
        for (honor_type, rarity, dedicated) in [
            ("event", "low", false),
            ("event", "middle", false),
            ("event", "high", true),
            ("event", "highest", true),
            ("birthday", "low", false),
            ("birthday", "middle", true),
            ("birthday", "high", true),
            ("birthday", "highest", true),
        ] {
            let mut honor = planned_honor();
            honor.honor_type = honor_type.into();
            honor.honor_rarity = rarity.into();
            let plan = honor.asset_plan(1, false);
            assert_eq!(
                plan.frame_candidates[0].is_some(),
                dedicated,
                "{honor_type} {rarity}"
            );
            assert!(plan.frame_candidates[1].is_some(), "{honor_type} {rarity}");
        }
        let mut honor = planned_honor();
        honor.honor_rarity = "highest".into();
        honor.frame_name = None;
        assert!(honor.asset_plan(1, true).frame_candidates[0].is_none());
    }

    #[test]
    fn json_provider_maps_fonts_by_region_without_mutating_source_tables() {
        let table = serde_json::json!([{ "id": 1, "fontName": "FOT-RodinNTLGPro-DB" }]);
        let mut cn = JsonMasterData::new("cn");
        cn.insert_value("customProfileTextFonts", table.clone())
            .unwrap();
        let mut jp = JsonMasterData::new("jp");
        jp.insert_value("customProfileTextFonts", table).unwrap();
        assert_eq!(cn.resolve_font(1).as_deref(), Some("FZLanTingHei-DB-GBK"));
        assert_eq!(jp.resolve_font(1).as_deref(), Some("FOT-RodinNTLGPro-DB"));
    }
    #[test]
    fn missing_honor_group_fields_remain_absent() {
        let mut data = JsonMasterData::new("cn");
        data.insert_value("honors", serde_json::json!([{ "id": 3, "assetbundleName": "honor_sample", "honorRarity": "high", "groupId": 4, "levels": [], "honorMissionType": null }])).unwrap();
        data.insert_value(
            "honorGroups",
            serde_json::json!([{ "id": 4, "honorType": "character" }]),
        )
        .unwrap();
        let honor = data.resolve_honor(3, 1).unwrap();
        assert_eq!(honor.background_asset_bundle_name, None);
        assert_eq!(honor.frame_name, None);

        // Empty strings are treated like absent fields.
        data.insert_value(
            "honorGroups",
            serde_json::json!([{ "id": 4, "honorType": "character", "backgroundAssetbundleName": "", "frameName": "" }]),
        )
        .unwrap();
        let honor = data.resolve_honor(3, 1).unwrap();
        assert_eq!(honor.background_asset_bundle_name, None);
        assert_eq!(honor.frame_name, None);
    }

    #[test]
    fn level_star_count_wraps_every_ten_levels() {
        for (level, stars) in [
            (-3, 0),
            (0, 0),
            (1, 1),
            (5, 5),
            (10, 10),
            (11, 1),
            (20, 10),
            (21, 1),
            (27, 7),
        ] {
            assert_eq!(honor_level_star_count(level), stars, "level {level}");
        }
    }

    struct PairData {
        rarity: &'static str,
    }

    impl ProfileMasterData for PairData {
        fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> {
            None
        }
        fn get_card(&self, _: i32) -> Option<CardEntry> {
            None
        }
        fn resolve_color(&self, _: i32) -> Option<ResolvedColor> {
            None
        }
        fn resolve_font(&self, _: i32) -> Option<String> {
            None
        }
        fn resolve_stamp(&self, _: i32) -> Option<String> {
            None
        }
        fn resolve_resource(&self, _: &str, _: i32) -> Option<ResourceInfo> {
            None
        }
        fn resolve_honor(&self, _: i32, _: i32) -> Option<ResolvedHonor> {
            None
        }
        fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry> {
            Some(BondsHonorEntry {
                id,
                game_character_unit_id1: 21,
                game_character_unit_id2: 1,
                honor_rarity: self.rarity.into(),
            })
        }
        fn get_bonds_honor_word(&self, id: i64) -> Option<BondsHonorWordEntry> {
            Some(BondsHonorWordEntry {
                id,
                assetbundle_name: "honorname_0121_01".into(),
            })
        }
        fn resolve_unit_virtual_singer(&self, self_id: i32, partner_id: i32) -> i32 {
            match (self_id, partner_id) {
                (21, 1) => 27,
                // Only reachable when the partner was already substituted.
                (1, 27) => 99,
                _ => self_id,
            }
        }
    }

    #[test]
    fn bonds_plan_substitutes_only_the_character_art() {
        let plan = bonds_honor_asset_plan(&PairData { rarity: "middle" }, 5, true, 3, false, true)
            .unwrap();
        let keys = plan
            .resources()
            .map(|key| (key.namespace.as_str(), key.key.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(plan.character_ids, [27, 1]);
        assert_eq!(
            keys,
            vec![
                ("static", "honor/bonds/21"),
                ("static", "honor/bonds/1"),
                ("assets", "bonds_honor/chr_sd_27_01"),
                ("assets", "bonds_honor/chr_sd_01_01"),
                ("static", "honor/mask_degree_main"),
                ("static", "honor/frame_degree_m_2"),
                ("assets", "bonds_honor/word/honorname_0121_01_02"),
                ("static", "honor/icon_degreeLv"),
                ("static", "honor/icon_degreeLv6"),
            ]
        );

        let plan =
            bonds_honor_asset_plan(&PairData { rarity: "low" }, 5, false, 3, true, false).unwrap();
        assert_eq!(plan.character_ids, [1, 21]);
        assert_eq!(plan.backgrounds[0].key, "honor/bonds/1_sub");
        assert_eq!(plan.characters[1].key, "bonds_honor/chr_sd_21_01");
        assert_eq!(plan.frame.key, "honor/frame_degree_s_1");
        assert_eq!(plan.mask.key, "honor/mask_degree_sub");
        assert_eq!(plan.word, None);
    }

    #[test]
    fn cn_limited_event_top_honor_resolves_shared_background_without_rank_overlay() {
        let mut data = JsonMasterData::new("cn");
        data.insert_value("honors", serde_json::json!([{ "id": 10140, "assetbundleName": "honor_top_000020", "honorRarity": "low", "groupId": 10034, "levels": [{"level": 1}], "honorMissionType": null }])).unwrap();
        data.insert_value(
            "honorGroups",
            serde_json::json!([{ "id": 10034, "honorType": "limitevent" }]),
        )
        .unwrap();
        let honor = data.resolve_honor(10140, 1).unwrap();
        assert_eq!(
            honor.effective_background_asset_bundle_name(),
            "honor_bg_event_cheerteam"
        );
        assert!(!honor.has_rank_overlay());
    }

    #[test]
    fn collection_rows_carry_their_collection_type_and_unknown_values_read_as_none() {
        let mut data = JsonMasterData::new("jp");
        let row = |id: i32, collection_type: serde_json::Value| {
            let mut row = serde_json::json!({
                "id": id,
                "customProfileResourceType": "collection",
                "resourceLoadVal": "custom_profile/collection/fixture",
                "fileName": format!("item_{id}"),
            });
            if !collection_type.is_null() || id == 5 {
                row["customProfileResourceCollectionType"] = collection_type;
            }
            row
        };
        data.insert_value(
            "customProfileCollectionResources",
            serde_json::json!([
                row(1, "can_badge".into()),
                row(2, "omikuji".into()),
                row(3, "acrylic_stand".into()),
                row(4, serde_json::Value::Null),
                row(5, serde_json::Value::Null),
                row(6, "a_future_kind".into()),
                row(7, "none".into()),
                row(8, "tapestry".into()),
            ]),
        )
        .unwrap();
        data.insert_value(
            "customProfileEtcResources",
            serde_json::json!([{
                "id": 1, "customProfileResourceType": "etc",
                "resourceLoadVal": "custom_profile/etc", "fileName": "etc_001",
            }]),
        )
        .unwrap();
        let kind = |id| {
            data.resolve_resource("collection", id)
                .expect("collection row")
                .collection_type
        };
        assert_eq!(kind(1), CollectionResourceType::CanBadge);
        assert_eq!(kind(2), CollectionResourceType::Omikuji);
        assert_eq!(kind(3), CollectionResourceType::AcrylicStand);
        // Absent and null columns, an unknown value and "none" all draw plain.
        for id in [4, 5, 6, 7] {
            assert_eq!(kind(id), CollectionResourceType::None, "row {id}");
        }
        assert_eq!(kind(8), CollectionResourceType::Tapestry);
        assert_eq!(
            data.resolve_resource("etc", 1).unwrap().collection_type,
            CollectionResourceType::None
        );
        for (value, expected) in [
            ("none", CollectionResourceType::None),
            ("omikuji", CollectionResourceType::Omikuji),
            ("can_badge", CollectionResourceType::CanBadge),
            ("keyholder", CollectionResourceType::Keyholder),
            ("acrylic_stand", CollectionResourceType::AcrylicStand),
            ("sticker", CollectionResourceType::Sticker),
            ("towel", CollectionResourceType::Towel),
            ("silver_tape", CollectionResourceType::SilverTape),
            ("ticket_holder", CollectionResourceType::TicketHolder),
            ("tapestry", CollectionResourceType::Tapestry),
        ] {
            assert_eq!(CollectionResourceType::parse(Some(value)), expected);
            assert_eq!(
                serde_json::to_value(expected).unwrap(),
                serde_json::json!(value)
            );
        }
        assert_eq!(
            CollectionResourceType::parse(None),
            CollectionResourceType::None
        );
    }

    /// An `omikujis` row with every column the table ships.
    pub(crate) fn omikuji_row_value(id: i32, unit: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id, "omikujiGroupId": 1, "unit": unit, "fortuneType": "grate_fortune",
            "summary": "夢の実現に\n近づく年", "title1": "願望", "description1": "必ず叶う",
            "title2": "健康", "description2": "大変良好", "title3": "待人", "description3": "必ず来る",
            "unitAssetbundleName": "lottery_game/new_year_2022_material",
            "fortuneAssetbundleName": "lottery_game/new_year_2022_material",
            "omikujiCoverAssetbundleName": "lottery_game/new_year_2022_material",
            "unitFilePath": "bird_VIRTUAL SINGER", "fortuneFilePath": "unsei_daikichi",
            "omikujiCoverFilePath": "omikuji_VIRTUAL SINGER"
        })
    }

    #[test]
    fn omikuji_rows_resolve_by_id_and_missing_tables_rows_or_fields_resolve_to_none() {
        let mut data = JsonMasterData::new("cn");
        assert_eq!(data.resolve_omikuji(1), None);
        let mut incomplete = omikuji_row_value(3, "idol");
        incomplete
            .as_object_mut()
            .unwrap()
            .remove("fortuneFilePath");
        data.insert_value(
            "omikujis",
            serde_json::json!([omikuji_row_value(1, "piapro"), incomplete]),
        )
        .unwrap();
        assert_eq!(
            data.resolve_omikuji(1),
            Some(OmikujiRow {
                id: 1,
                unit: "piapro".into(),
                summary: "夢の実現に\n近づく年".into(),
                title1: "願望".into(),
                description1: "必ず叶う".into(),
                title2: "健康".into(),
                description2: "大変良好".into(),
                title3: "待人".into(),
                description3: "必ず来る".into(),
                fortune_assetbundle_name: "lottery_game/new_year_2022_material".into(),
                fortune_file_path: "unsei_daikichi".into(),
                omikuji_cover_assetbundle_name: "lottery_game/new_year_2022_material".into(),
                omikuji_cover_file_path: "omikuji_VIRTUAL SINGER".into(),
            })
        );
        assert_eq!(data.resolve_omikuji(2), None);
        assert_eq!(data.resolve_omikuji(3), None);
        assert!(PROFILE_OPTIONAL_MASTERDATA_TABLES.contains(&"omikujis"));
        assert!(!PROFILE_MASTERDATA_TABLES.contains(&"omikujis"));
    }

    #[test]
    fn from_hex_rejects_multi_byte_input_without_panicking() {
        // The length check counts bytes; a string slice at a non-char-boundary
        // index would panic on multi-byte input.
        assert_eq!(ResolvedColor::from_hex("日日"), None);
        assert_eq!(ResolvedColor::from_hex("#日日日"), None);
        assert_eq!(ResolvedColor::from_hex("#11223g"), None);
        assert_eq!(
            ResolvedColor::from_hex("#112233"),
            Some(ResolvedColor {
                r: 0x11,
                g: 0x22,
                b: 0x33,
                a: 255
            })
        );
    }

    #[test]
    fn live_master_honor_falls_back_to_the_last_known_level() {
        // The game resolves a level the masterdata does not list yet to the
        // last known level instead of dropping the honor or clearing its
        // asset bundle name.
        let mut data = JsonMasterData::new("cn");
        data.insert_value(
            "honors",
            serde_json::json!([{
                "id": 7,
                "assetbundleName": null,
                "honorRarity": null,
                "groupId": null,
                "honorMissionType": "master_full_perfect",
                "levels": [
                    { "level": 1, "assetbundleName": "honor_live_1", "honorRarity": "low" },
                    { "level": 5, "assetbundleName": "honor_live_5", "honorRarity": "high" },
                ],
            }]),
        )
        .unwrap();
        let honor = data.resolve_honor(7, 9).unwrap();
        assert!(honor.is_live_master);
        assert_eq!(honor.asset_bundle_name, "honor_live_5");
        assert_eq!(honor.honor_rarity, "high");
        // A listed level still resolves exactly.
        let honor = data.resolve_honor(7, 1).unwrap();
        assert_eq!(honor.asset_bundle_name, "honor_live_1");
        assert_eq!(honor.honor_rarity, "low");
    }

    #[test]
    fn unknown_color_and_font_ids_fall_back_to_the_default_rows() {
        let mut data = JsonMasterData::new("jp");
        data.insert_value(
            "customProfileTextColors",
            serde_json::json!([
                { "id": 5, "colorCode": "#444466" },
                { "id": 1, "colorCode": "#ffffff" }
            ]),
        )
        .unwrap();
        data.insert_value(
            "customProfileTextFonts",
            serde_json::json!([
                { "id": 2, "fontName": "Second" },
                { "id": 1, "fontName": "First" }
            ]),
        )
        .unwrap();
        let first_row = ResolvedColor {
            r: 0x44,
            g: 0x44,
            b: 0x66,
            a: 0xff,
        };
        assert_eq!(data.resolve_color(404), None);
        assert_eq!(data.resolve_color_or_default(404), Some(first_row));
        assert_eq!(
            data.resolve_color_or_default(1).map(|color| color.r),
            Some(0xff)
        );
        assert_eq!(data.resolve_font_or_default(404).as_deref(), Some("First"));
        assert_eq!(data.resolve_font_or_default(2).as_deref(), Some("Second"));
        assert_eq!(JsonMasterData::new("jp").resolve_color_or_default(1), None);
    }

    #[test]
    fn region_aliases_canonicalize_to_the_font_mapping_region() {
        let table = serde_json::json!([{ "id": 1, "fontName": "FOT-RodinNTLGPro-DB" }]);
        for region in ["CN", "zh-cn", "sc"] {
            let mut data = JsonMasterData::new(region);
            data.insert_value("customProfileTextFonts", table.clone())
                .unwrap();
            assert_eq!(
                data.resolve_font(1).as_deref(),
                Some("FZLanTingHei-DB-GBK"),
                "region {region} must map the CN font"
            );
        }
    }
}
