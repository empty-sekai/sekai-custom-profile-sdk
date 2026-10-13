//! The omikuji fortune slip an omikuji collection shows.
//!
//! An omikuji collection row names a slip prefab (`resourceLoadVal`
//! `lottery_game/new_year_<year>`, `fileName` `Prefabs/Omikuji`) and the
//! element's `targetId` picks the `omikujis` row it shows. The slip is
//! [`SLIP_SIZE`] units, centred on the element, and draws in this order:
//!
//! 1. the cover image `{omikujiCoverAssetbundleName}/bg_{omikujiCoverFilePath}`,
//!    stretched over the whole slip;
//! 2. the fortune image `{fortuneAssetbundleName}/{fortuneFilePath}`, 118 × 300
//!    units near the right edge;
//! 3. `title1`, `title2` and `title3` in white, each over a solid background in
//!    the colour of the row's `unit` ([`unit_color`]);
//! 4. `summary` and `description1..3` in grey, each description to the left of
//!    its title.
//!
//! Every text is a uGUI text node ([`crate::ugui_text`]) turned a quarter turn
//! clockwise with the vertical-text modifier, so lines run down the slip and
//! follow each other from right to left. Strings are shown as the game sets
//! them: every U+0020 becomes U+00A0, which draws as a blank glyph rather than
//! a space.
//!
//! Title backgrounds are 50 × 126 units. For regions `cn`, `tw` and `kr` each
//! keeps its top edge and its height becomes its title's preferred width plus
//! [`TITLE_BACKGROUND_FIT_PADDING`]; for `kr` the summary also takes the
//! descriptions' font size ([`OmikujiClient`]).
//!
//! A character the slip's font file has no glyph for comes from the first of
//! two fallback faces that has one: [`LATIN_FALLBACK_FAMILY`], then the
//! region's face of the Noto Sans CJK collection ([`CjkFallback`]). Both are
//! required fonts of every slip text, like the slip's own font.

use serde::{Deserialize, Serialize};

use crate::masterdata::{normalize_region, OmikujiRow, ProfileMasterData};
use crate::profile_scene::semantic_command_id;
use crate::ugui_text::{
    UguiFontAsset, UguiFontFace, UguiTextBackdrop, UguiTextSource, UguiVerticalModifier,
    VerticalGlyphOffset,
};
use crate::{Matrix2d, Rect, ResourceKey, SemanticCommandSource, StableId};

/// Width and height of the slip, which is also the element's content size.
pub const SLIP_SIZE: [f32; 2] = [1480.0, 490.0];
/// `fileName` of the slip prefab in its bundle.
pub const PREFAB_FILE: &str = "Prefabs/Omikuji";
/// Units added to a title's preferred width to give the height of a fitted
/// title background.
pub const TITLE_BACKGROUND_FIT_PADDING: f32 = 34.0;

/// A slip prefab this module lays out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OmikujiPrefab {
    /// `resourceLoadVal` of the collection row.
    pub bundle: &'static str,
    /// Font asset of the text nodes, requested by this family name.
    pub font_family: &'static str,
    /// The vertical-text modifier also turns `.` and `,`.
    pub turns_periods_and_commas: bool,
}

/// The slip prefabs this module lays out. They share their geometry and
/// differ in the font asset and in the modifier's offsets.
pub const PREFABS: [OmikujiPrefab; 4] = [
    OmikujiPrefab {
        bundle: "lottery_game/new_year_2022",
        font_family: "FOT-Omikuji",
        turns_periods_and_commas: true,
    },
    OmikujiPrefab {
        bundle: "lottery_game/new_year_2023",
        font_family: "FOT-UDMinchoPro-B",
        turns_periods_and_commas: false,
    },
    OmikujiPrefab {
        bundle: "lottery_game/new_year_2024",
        font_family: "FOT-UDMinchoPro-B",
        turns_periods_and_commas: false,
    },
    OmikujiPrefab {
        bundle: "lottery_game/new_year_2025",
        font_family: "FOT-UDMinchoPro-B",
        turns_periods_and_commas: false,
    },
];

/// The prefab a collection row names, or `None` for a prefab this module
/// does not lay out.
pub fn prefab(load_value: &str, file_name: &str) -> Option<OmikujiPrefab> {
    if file_name != PREFAB_FILE {
        return None;
    }
    PREFABS
        .iter()
        .copied()
        .find(|prefab| prefab.bundle == load_value)
}

/// Metrics of both font assets the slips use; they differ only in name.
pub fn font_asset(family: &str) -> UguiFontAsset {
    UguiFontAsset {
        family: family.into(),
        reference_size: 16.0,
        ascent: 14.080_001,
        descent: -1.920_000_1,
        line_spacing: 32.0,
        character_padding: 1,
        tracking: 1.0,
        round_advance: true,
    }
}

/// Family of the first fallback face: the system UI font, whose file is
/// `Roboto-Regular.ttf`.
pub const LATIN_FALLBACK_FAMILY: &str = "Roboto";

/// The face of the Noto Sans CJK collection (`NotoSansCJK-Regular.ttc`) a
/// region's slips fall back to after [`LATIN_FALLBACK_FAMILY`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CjkFallback {
    Japanese,
    Korean,
    /// Also the face of a region without a script of its own.
    #[default]
    SimplifiedChinese,
    TraditionalChinese,
}

impl CjkFallback {
    /// Every face, in the order of the collection.
    pub const ALL: [CjkFallback; 4] = [
        CjkFallback::Japanese,
        CjkFallback::Korean,
        CjkFallback::SimplifiedChinese,
        CjkFallback::TraditionalChinese,
    ];

    /// Family the collection file is requested by for this face.
    pub fn family(self) -> &'static str {
        match self {
            CjkFallback::Japanese => "Noto Sans CJK JP",
            CjkFallback::Korean => "Noto Sans CJK KR",
            CjkFallback::SimplifiedChinese => "Noto Sans CJK SC",
            CjkFallback::TraditionalChinese => "Noto Sans CJK TC",
        }
    }

    /// Index of the face in the collection file.
    pub fn face_index(self) -> u32 {
        match self {
            CjkFallback::Japanese => 0,
            CjkFallback::Korean => 1,
            CjkFallback::SimplifiedChinese => 2,
            CjkFallback::TraditionalChinese => 3,
        }
    }
}

/// How a region's game client sets the slip up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OmikujiClient {
    /// Each title background keeps its top edge and its height becomes its
    /// title's preferred width plus [`TITLE_BACKGROUND_FIT_PADDING`].
    pub fit_title_backgrounds: bool,
    /// The summary takes the descriptions' font size.
    pub summary_at_description_size: bool,
    /// The CJK face characters fall back to.
    pub cjk_fallback: CjkFallback,
}

impl OmikujiClient {
    /// The setup for a region code or locale tag.
    pub fn for_region(region: &str) -> Self {
        match normalize_region(region).as_str() {
            "cn" => Self {
                fit_title_backgrounds: true,
                ..Self::default()
            },
            "tw" => Self {
                fit_title_backgrounds: true,
                cjk_fallback: CjkFallback::TraditionalChinese,
                ..Self::default()
            },
            "kr" => Self {
                fit_title_backgrounds: true,
                summary_at_description_size: true,
                cjk_fallback: CjkFallback::Korean,
            },
            "jp" => Self {
                cjk_fallback: CjkFallback::Japanese,
                ..Self::default()
            },
            _ => Self::default(),
        }
    }

    /// Faces a slip text falls back to, in order, after its font file.
    pub fn fallback_faces(self) -> Vec<UguiFontFace> {
        vec![
            UguiFontFace {
                family: LATIN_FALLBACK_FAMILY.into(),
                face_index: 0,
            },
            UguiFontFace {
                family: self.cjk_fallback.family().into(),
                face_index: self.cjk_fallback.face_index(),
            },
        ]
    }

    /// The setup of the client whose tables `masterdata` holds. `locale`
    /// stands in when the tables do not name their region.
    pub fn for_masterdata(masterdata: &(impl ProfileMasterData + ?Sized), locale: &str) -> Self {
        Self::for_region(masterdata.region_code().unwrap_or(locale))
    }
}

/// Colour of the title backgrounds as the prefab ships them. It is also the
/// colour a `piapro` row sets, and the one a unit outside [`unit_color`]'s
/// table keeps.
const SLIP_TITLE_COLOR: [u8; 3] = [0x00, 0xcc, 0xbb];
/// Unit colours of the title backgrounds.
const UNIT_COLORS: [(&str, [u8; 3]); 6] = [
    ("piapro", SLIP_TITLE_COLOR),
    ("light_sound", [0x44, 0x55, 0xdd]),
    ("idol", [0x88, 0xdd, 0x44]),
    ("street", [0xee, 0x11, 0x66]),
    ("theme_park", [0xff, 0x99, 0x00]),
    ("school_refusal", [0x88, 0x44, 0x99]),
];

/// Colour of the title backgrounds for an `omikujis` `unit`, straight RGBA in
/// `0..=1`.
pub fn unit_color(unit: &str) -> [f32; 4] {
    let rgb = UNIT_COLORS
        .iter()
        .find(|(name, _)| *name == unit)
        .map_or(SLIP_TITLE_COLOR, |(_, rgb)| *rgb);
    [
        f32::from(rgb[0]) / 255.0,
        f32::from(rgb[1]) / 255.0,
        f32::from(rgb[2]) / 255.0,
        1.0,
    ]
}

/// An omikuji collection to draw: its prefab and its `omikujis` row.
#[derive(Clone, Debug, PartialEq)]
pub struct OmikujiPlan {
    pub prefab: OmikujiPrefab,
    pub row: OmikujiRow,
}

impl OmikujiPlan {
    /// The cover image, drawn over the whole slip.
    pub fn cover(&self) -> ResourceKey {
        ResourceKey {
            namespace: "assets".into(),
            key: format!(
                "{}/bg_{}",
                self.row.omikuji_cover_assetbundle_name, self.row.omikuji_cover_file_path
            ),
        }
    }

    /// The fortune image.
    pub fn fortune(&self) -> ResourceKey {
        ResourceKey {
            namespace: "assets".into(),
            key: format!(
                "{}/{}",
                self.row.fortune_assetbundle_name, self.row.fortune_file_path
            ),
        }
    }

    /// Everything the slip draws for a client set up as `client`.
    pub fn visual(&self, client: OmikujiClient) -> OmikujiVisualSnapshot {
        let row = &self.row;
        let mut offsets = vec![
            offset('ゃ', 10.0, 0.0),
            offset('ゅ', 10.0, 0.0),
            offset('ょ', 10.0, 0.0),
            offset('っ', 10.0, 0.0),
            offset('、', 22.0, 90.0),
            offset('。', 22.0, 90.0),
            offset('ー', 0.0, 0.0),
            offset('々', 0.0, 90.0),
        ];
        if self.prefab.turns_periods_and_commas {
            offsets.extend([offset('.', 22.0, 90.0), offset(',', 22.0, 90.0)]);
        }
        OmikujiVisualSnapshot {
            omikuji_id: row.id,
            cover: self.cover(),
            fortune: self.fortune(),
            font: font_asset(self.prefab.font_family),
            fallback_faces: client.fallback_faces(),
            vertical: UguiVerticalModifier { offsets },
            title_background: unit_color(&row.unit),
            fit_title_backgrounds: client.fit_title_backgrounds,
            summary_font_size: if client.summary_at_description_size {
                DESCRIPTION_FONT_SIZE
            } else {
                SUMMARY_FONT_SIZE
            },
            titles: [&row.title1, &row.title2, &row.title3].map(|text| displayed(text)),
            summary: displayed(&row.summary),
            descriptions: [&row.description1, &row.description2, &row.description3]
                .map(|text| displayed(text)),
        }
    }
}

fn offset(target: char, y: f32, angle_degrees: f32) -> VerticalGlyphOffset {
    VerticalGlyphOffset {
        target,
        offset: [0.0, y],
        angle_degrees,
    }
}

/// A string as the slip's text nodes show it: every U+0020 becomes U+00A0.
pub fn displayed(text: &str) -> String {
    text.replace(' ', "\u{a0}")
}

/// Resolved content of one omikuji collection, lowered by
/// [`lower_omikuji`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OmikujiVisualSnapshot {
    pub omikuji_id: i32,
    pub cover: ResourceKey,
    pub fortune: ResourceKey,
    pub font: UguiFontAsset,
    /// Faces every text falls back to after the font asset's file.
    pub fallback_faces: Vec<UguiFontFace>,
    pub vertical: UguiVerticalModifier,
    /// Colour of the three title backgrounds.
    pub title_background: [f32; 4],
    pub fit_title_backgrounds: bool,
    pub summary_font_size: i32,
    /// Displayed strings of the titles, summary and descriptions.
    pub titles: [String; 3],
    pub summary: String,
    pub descriptions: [String; 3],
}

const SUMMARY_FONT_SIZE: i32 = 36;
const DESCRIPTION_FONT_SIZE: i32 = 30;
const TITLE_FONT_SIZE: i32 = 40;
/// Grey of the summary and descriptions before 8-bit quantisation.
const BODY_GREY: f32 = 0.311_320_78;
/// Rotation of every text node, the z and w components of its quaternion: a
/// quarter turn clockwise. The components are the stored `f32` values, one
/// ulp larger in magnitude than `FRAC_1_SQRT_2`.
#[allow(clippy::approx_constant)]
const TEXT_ROTATION: [f32; 2] = [-0.707_106_8, 0.707_106_8];
/// Centre and size of the fortune image in slip space (+y up).
const FORTUNE_CENTRE: [f32; 2] = [623.999_9, 1.0];
const FORTUNE_SIZE: [f32; 2] = [118.0, 300.0];
/// Size of the group holding the texts; it is centred on the slip.
const TEXT_GROUP_SIZE: [f32; 2] = [1920.0, 1080.0];
/// Centres of the title backgrounds in slip space (+y up), right to left.
const TITLE_BACKGROUND_CENTRES: [[f32; 2]; 3] = [[19.0, 147.2], [-136.3, 147.2], [-287.1, 147.2]];
const TITLE_BACKGROUND_SIZE: [f32; 2] = [50.0, 126.0];
/// Pivot of a title relative to its background's centre.
const TITLE_POSITION: [f32; 2] = [-80.8, 44.400_01];
const TITLE_RECT: [f32; 2] = [100.0, 100.0];
/// The summary and the descriptions hang from the right edge of the text
/// group, pivoted at their top-left corners.
const SUMMARY_POSITION: [f32; 2] = [-424.0, 209.73];
const SUMMARY_RECT: [f32; 2] = [418.634_28, 134.812_13];
const DESCRIPTION_POSITIONS: [[f32; 2]; 3] = [[-982.0, 195.0], [-1135.0, 195.0], [-1285.0, 195.0]];
const DESCRIPTION_RECT: [f32; 2] = [416.0, 78.0];
const BODY_LINE_SPACING: f32 = 0.62;

/// Maps a text node's space (origin at its pivot, +y up) to the layer (+y
/// down) for a node pivoted at `position` in slip space: the rotation matrix
/// of `TEXT_ROTATION`, then the translation.
pub fn text_node_matrix(position: [f32; 2]) -> Matrix2d {
    let [z, w] = TEXT_ROTATION;
    let z2 = z + z;
    let zz = z * z2;
    let wz = w * z2;
    let (m00, m01, m10, m11) = (1.0 - zz, -wz, wz, 1.0 - zz);
    [m00, -m10, m01, -m11, position[0], -position[1]]
}

/// Lowers an omikuji collection into the commands of its layer.
pub fn lower_omikuji(
    source_key: &str,
    layer_id: StableId,
    visual: &OmikujiVisualSnapshot,
) -> Vec<SemanticCommandSource> {
    let id = |role: &str, ordinal: u32| semantic_command_id(source_key, role, ordinal);
    let mut commands = vec![
        SemanticCommandSource::image(
            id("omikuji-cover", 0),
            layer_id,
            "omikuji-cover",
            visual.cover.clone(),
            Rect {
                x: -SLIP_SIZE[0] / 2.0,
                y: -SLIP_SIZE[1] / 2.0,
                width: SLIP_SIZE[0],
                height: SLIP_SIZE[1],
            },
        ),
        SemanticCommandSource::image(
            id("omikuji-fortune", 0),
            layer_id,
            "omikuji-fortune",
            visual.fortune.clone(),
            layer_rect(FORTUNE_CENTRE, FORTUNE_SIZE),
        ),
    ];
    let text = |role: &str, ordinal: u32, source: UguiTextSource| {
        let mut bounds = node_bounds(&source);
        if let Some(backdrop) = &source.backdrop {
            bounds = union(bounds, backdrop.rect);
        }
        SemanticCommandSource::ugui_text(id(role, ordinal), layer_id, role, bounds, source)
    };
    let white = [1.0; 4];
    let grey = [BODY_GREY, BODY_GREY, BODY_GREY, 1.0].map(crate::sdf_material::color32_unit);
    for (index, (title, centre)) in visual
        .titles
        .iter()
        .zip(TITLE_BACKGROUND_CENTRES)
        .enumerate()
    {
        let mut source = text_source(
            visual,
            title,
            TITLE_FONT_SIZE,
            1.0,
            white,
            TITLE_RECT,
            [0.0, 0.0],
            [centre[0] + TITLE_POSITION[0], centre[1] + TITLE_POSITION[1]],
        );
        source.backdrop = Some(UguiTextBackdrop {
            color: visual.title_background,
            rect: layer_rect(centre, TITLE_BACKGROUND_SIZE),
            fit_padding: visual
                .fit_title_backgrounds
                .then_some(TITLE_BACKGROUND_FIT_PADDING),
        });
        commands.push(text("omikuji-title", index as u32, source));
    }
    let right_edge = TEXT_GROUP_SIZE[0] / 2.0;
    commands.push(text(
        "omikuji-summary",
        0,
        text_source(
            visual,
            &visual.summary,
            visual.summary_font_size,
            BODY_LINE_SPACING,
            grey,
            SUMMARY_RECT,
            [0.0, 1.0],
            [right_edge + SUMMARY_POSITION[0], SUMMARY_POSITION[1]],
        ),
    ));
    for (index, (description, position)) in visual
        .descriptions
        .iter()
        .zip(DESCRIPTION_POSITIONS)
        .enumerate()
    {
        commands.push(text(
            "omikuji-description",
            index as u32,
            text_source(
                visual,
                description,
                DESCRIPTION_FONT_SIZE,
                BODY_LINE_SPACING,
                grey,
                DESCRIPTION_RECT,
                [0.0, 1.0],
                [right_edge + position[0], position[1]],
            ),
        ));
    }
    commands
}

#[allow(clippy::too_many_arguments)]
fn text_source(
    visual: &OmikujiVisualSnapshot,
    text: &str,
    font_size: i32,
    line_spacing: f32,
    color: [f32; 4],
    rect_size: [f32; 2],
    pivot: [f32; 2],
    position: [f32; 2],
) -> UguiTextSource {
    UguiTextSource {
        text: text.into(),
        font: visual.font.clone(),
        fallback_faces: visual.fallback_faces.clone(),
        font_size,
        line_spacing,
        color,
        rect_size,
        pivot,
        node_matrix: text_node_matrix(position),
        vertical: Some(visual.vertical.clone()),
        backdrop: None,
    }
}

/// A rect of `size` centred on `centre` in slip space (+y up), in layer
/// space (+y down).
fn layer_rect(centre: [f32; 2], size: [f32; 2]) -> Rect {
    Rect {
        x: centre[0] - size[0] / 2.0,
        y: -(centre[1] + size[1] / 2.0),
        width: size[0],
        height: size[1],
    }
}

/// Layer-space bounds of a text node's rect.
fn node_bounds(source: &UguiTextSource) -> Rect {
    let [width, height] = source.rect_size;
    let left = -source.pivot[0] * width;
    let bottom = -source.pivot[1] * height;
    let m = source.node_matrix;
    let corners = [
        [left, bottom],
        [left + width, bottom],
        [left + width, bottom + height],
        [left, bottom + height],
    ]
    .map(|[x, y]| [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]);
    let min_x = corners.iter().map(|c| c[0]).fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|c| c[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = corners.iter().map(|c| c[1]).fold(f32::INFINITY, f32::min);
    let max_y = corners
        .iter()
        .map(|c| c[1])
        .fold(f32::NEG_INFINITY, f32::max);
    Rect {
        x: min_x,
        y: min_y,
        width: max_x - min_x,
        height: max_y - min_y,
    }
}

fn union(a: Rect, b: Rect) -> Rect {
    let left = a.x.min(b.x);
    let top = a.y.min(b.y);
    let right = (a.x + a.width).max(b.x + b.width);
    let bottom = (a.y + a.height).max(b.y + b.height);
    Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

#[cfg(test)]
mod tests;
