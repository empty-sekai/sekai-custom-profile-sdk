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

use serde::{Deserialize, Serialize};

use crate::masterdata::{normalize_region, OmikujiRow};
use crate::profile_scene::semantic_command_id;
use crate::ugui_text::{
    UguiFontAsset, UguiTextBackdrop, UguiTextSource, UguiVerticalModifier, VerticalGlyphOffset,
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

/// How a region's game client sets the slip up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OmikujiClient {
    /// Each title background keeps its top edge and its height becomes its
    /// title's preferred width plus [`TITLE_BACKGROUND_FIT_PADDING`].
    pub fit_title_backgrounds: bool,
    /// The summary takes the descriptions' font size.
    pub summary_at_description_size: bool,
}

impl OmikujiClient {
    /// The setup for a region code or locale tag.
    pub fn for_region(region: &str) -> Self {
        match normalize_region(region).as_str() {
            "cn" | "tw" => Self {
                fit_title_backgrounds: true,
                summary_at_description_size: false,
            },
            "kr" => Self {
                fit_title_backgrounds: true,
                summary_at_description_size: true,
            },
            _ => Self::default(),
        }
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
/// of [`TEXT_ROTATION`], then the translation.
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
mod tests {
    use super::*;
    use crate::profile_source::CustomProfileCard;
    use crate::SemanticCommandPayload;

    fn row(unit: &str) -> OmikujiRow {
        OmikujiRow {
            id: 7,
            unit: unit.into(),
            summary: "夢の実現に\n近づく年".into(),
            title1: "願望".into(),
            description1: "必ず 叶う".into(),
            title2: "健康".into(),
            description2: "大変良好".into(),
            title3: "待人".into(),
            description3: "必ず来る".into(),
            fortune_assetbundle_name: "lottery_game/new_year_2022_material".into(),
            fortune_file_path: "unsei_daikichi".into(),
            omikuji_cover_assetbundle_name: "lottery_game/new_year_2022_material".into(),
            omikuji_cover_file_path: "omikuji_VIRTUAL SINGER".into(),
        }
    }

    fn plan(bundle: &str, unit: &str) -> OmikujiPlan {
        OmikujiPlan {
            prefab: prefab(bundle, PREFAB_FILE).expect("known prefab"),
            row: row(unit),
        }
    }

    fn texts(commands: &[SemanticCommandSource]) -> Vec<(&str, &UguiTextSource)> {
        commands
            .iter()
            .filter_map(|command| match &command.payload {
                SemanticCommandPayload::UguiText(source) => Some((command.role.as_str(), source)),
                _ => None,
            })
            .collect()
    }

    fn lowered(plan: &OmikujiPlan, client: OmikujiClient) -> Vec<SemanticCommandSource> {
        lower_omikuji("collection-0", StableId(3), &plan.visual(client))
    }

    #[test]
    fn prefabs_are_known_by_bundle_and_prefab_file() {
        assert_eq!(
            prefab("lottery_game/new_year_2022", "Prefabs/Omikuji").map(|p| p.font_family),
            Some("FOT-Omikuji")
        );
        for year in [2023, 2024, 2025] {
            let found = prefab(&format!("lottery_game/new_year_{year}"), PREFAB_FILE);
            assert_eq!(found.map(|p| p.font_family), Some("FOT-UDMinchoPro-B"));
        }
        assert_eq!(prefab("lottery_game/new_year_2026", PREFAB_FILE), None);
        assert_eq!(prefab("lottery_game/new_year_2022", "Prefabs/Other"), None);
    }

    #[test]
    fn the_slip_requests_its_cover_and_fortune_images_by_their_row_keys() {
        let plan = plan("lottery_game/new_year_2022", "piapro");
        let key = |key: &str| ResourceKey {
            namespace: "assets".into(),
            key: key.into(),
        };
        assert_eq!(
            plan.cover(),
            key("lottery_game/new_year_2022_material/bg_omikuji_VIRTUAL SINGER")
        );
        assert_eq!(
            plan.fortune(),
            key("lottery_game/new_year_2022_material/unsei_daikichi")
        );
        let commands = lowered(&plan, OmikujiClient::default());
        let images = commands
            .iter()
            .filter_map(|command| match &command.payload {
                SemanticCommandPayload::Image { resource, .. } => {
                    Some((command.role.as_str(), resource.clone(), command.bounds))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            images,
            [
                (
                    "omikuji-cover",
                    plan.cover(),
                    Rect {
                        x: -740.0,
                        y: -245.0,
                        width: 1480.0,
                        height: 490.0
                    }
                ),
                (
                    "omikuji-fortune",
                    plan.fortune(),
                    Rect {
                        x: 623.999_9 - 59.0,
                        y: -151.0,
                        width: 118.0,
                        height: 300.0
                    }
                ),
            ]
        );
    }

    #[test]
    fn commands_follow_the_prefab_hierarchy_order() {
        let commands = lowered(
            &plan("lottery_game/new_year_2022", "idol"),
            OmikujiClient::default(),
        );
        let roles = commands
            .iter()
            .map(|command| command.role.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            roles,
            [
                "omikuji-cover",
                "omikuji-fortune",
                "omikuji-title",
                "omikuji-title",
                "omikuji-title",
                "omikuji-summary",
                "omikuji-description",
                "omikuji-description",
                "omikuji-description",
            ]
        );
        let ids = commands
            .iter()
            .map(|command| command.id)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids.len(), commands.len());
        assert!(commands
            .iter()
            .all(|command| command.layer_id == StableId(3)
                && command.matrix == [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
    }

    #[test]
    fn texts_name_the_font_asset_and_modifier_offsets_of_their_prefab() {
        let first = lowered(
            &plan("lottery_game/new_year_2022", "idol"),
            OmikujiClient::default(),
        );
        let later = lowered(
            &plan("lottery_game/new_year_2024", "idol"),
            OmikujiClient::default(),
        );
        let targets = |source: &UguiTextSource| {
            source
                .vertical
                .as_ref()
                .expect("vertical text")
                .offsets
                .iter()
                .map(|offset| offset.target)
                .collect::<String>()
        };
        for (role, source) in texts(&first) {
            assert_eq!(source.font, font_asset("FOT-Omikuji"), "{role}");
            assert_eq!(targets(source), "ゃゅょっ、。ー々.,", "{role}");
        }
        for (role, source) in texts(&later) {
            assert_eq!(source.font.family, "FOT-UDMinchoPro-B", "{role}");
            assert_eq!(targets(source), "ゃゅょっ、。ー々", "{role}");
        }
        let marks = &texts(&first)[0].1.vertical.as_ref().unwrap().offsets;
        assert_eq!(
            marks[5],
            VerticalGlyphOffset {
                target: '。',
                offset: [0.0, 22.0],
                angle_degrees: 90.0
            }
        );
        assert_eq!(marks[0].offset, [0.0, 10.0]);
        assert_eq!(marks[6].angle_degrees, 0.0);
    }

    #[test]
    fn titles_summary_and_descriptions_show_their_row_strings_with_no_break_spaces() {
        let commands = lowered(
            &plan("lottery_game/new_year_2022", "idol"),
            OmikujiClient::default(),
        );
        let shown = texts(&commands)
            .into_iter()
            .map(|(_, source)| source.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            shown,
            [
                "願望",
                "健康",
                "待人",
                "夢の実現に\n近づく年",
                "必ず\u{a0}叶う",
                "大変良好",
                "必ず来る"
            ]
        );
        let settings = texts(&commands)
            .into_iter()
            .map(|(_, s)| (s.font_size, s.line_spacing, s.rect_size, s.pivot, s.color))
            .collect::<Vec<_>>();
        let white = [1.0; 4];
        let grey = [79.0 / 255.0, 79.0 / 255.0, 79.0 / 255.0, 1.0];
        let title = (40, 1.0, [100.0, 100.0], [0.0, 0.0], white);
        let description = (30, 0.62, [416.0, 78.0], [0.0, 1.0], grey);
        assert_eq!(
            settings,
            [
                title,
                title,
                title,
                (36, 0.62, [418.634_28, 134.812_13], [0.0, 1.0], grey),
                description,
                description,
                description,
            ]
        );
    }

    #[test]
    fn title_backgrounds_take_the_unit_colour() {
        let background = |unit: &str| {
            let commands = lowered(
                &plan("lottery_game/new_year_2023", unit),
                OmikujiClient::default(),
            );
            let colours = texts(&commands)
                .into_iter()
                .filter_map(|(_, source)| source.backdrop.as_ref().map(|b| b.color))
                .collect::<Vec<_>>();
            assert_eq!(colours.len(), 3, "{unit}");
            assert!(colours.windows(2).all(|pair| pair[0] == pair[1]), "{unit}");
            colours[0].map(|channel| (channel * 255.0).round() as u8)
        };
        assert_eq!(background("piapro"), [0x00, 0xcc, 0xbb, 255]);
        assert_eq!(background("light_sound"), [0x44, 0x55, 0xdd, 255]);
        assert_eq!(background("idol"), [0x88, 0xdd, 0x44, 255]);
        assert_eq!(background("street"), [0xee, 0x11, 0x66, 255]);
        assert_eq!(background("theme_park"), [0xff, 0x99, 0x00, 255]);
        assert_eq!(background("school_refusal"), [0x88, 0x44, 0x99, 255]);
        // A unit outside the table keeps the prefab's colour.
        assert_eq!(background("none"), [0x00, 0xcc, 0xbb, 255]);
    }

    #[test]
    fn regions_fit_the_title_backgrounds_and_the_korean_client_shrinks_the_summary() {
        let jp = OmikujiClient::for_region("jp");
        assert_eq!(jp, OmikujiClient::default());
        assert_eq!(OmikujiClient::for_region("en"), jp);
        for region in ["cn", "zh-CN", "tw"] {
            assert_eq!(
                OmikujiClient::for_region(region),
                OmikujiClient {
                    fit_title_backgrounds: true,
                    summary_at_description_size: false
                },
                "{region}"
            );
        }
        let kr = OmikujiClient::for_region("kr");
        assert!(kr.fit_title_backgrounds && kr.summary_at_description_size);

        let plan = plan("lottery_game/new_year_2022", "street");
        let backdrop = |client| {
            texts(&lowered(&plan, client))[0]
                .1
                .backdrop
                .clone()
                .expect("title background")
        };
        let fixed = backdrop(jp);
        assert_eq!(fixed.fit_padding, None);
        assert_eq!(
            fixed.rect,
            Rect {
                x: 19.0 - 25.0,
                y: -(147.2 + 63.0),
                width: 50.0,
                height: 126.0
            }
        );
        let fitted = backdrop(OmikujiClient::for_region("cn"));
        assert_eq!(fitted.fit_padding, Some(TITLE_BACKGROUND_FIT_PADDING));
        assert_eq!(fitted.rect, fixed.rect);
        // Four 40-unit characters: the background runs 194 units down from
        // its top edge.
        assert_eq!(fitted.resolved_rect(160.0).height, 194.0);
        assert_eq!(fitted.resolved_rect(160.0).y, fixed.rect.y);

        let summary_size = |client| texts(&lowered(&plan, client))[3].1.font_size;
        assert_eq!(summary_size(jp), 36);
        assert_eq!(summary_size(OmikujiClient::for_region("cn")), 36);
        assert_eq!(summary_size(kr), 30);
    }

    fn apply(m: Matrix2d, [x, y]: [f32; 2]) -> [f32; 2] {
        [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]
    }

    fn assert_close(actual: [f32; 2], expected: [f32; 2]) {
        assert!(
            (actual[0] - expected[0]).abs() < 1e-4 && (actual[1] - expected[1]).abs() < 1e-4,
            "{actual:?} != {expected:?}"
        );
    }

    #[test]
    fn text_nodes_turn_a_quarter_turn_clockwise_about_their_pivot() {
        let m = text_node_matrix([0.0, 0.0]);
        // The rotation matrix of the f32 quaternion is not exactly a quarter
        // turn.
        assert_eq!(
            m,
            [
                -1.192_092_9e-7,
                1.000_000_1,
                1.000_000_1,
                1.192_092_9e-7,
                0.0,
                -0.0
            ]
        );
        // Node +x (along a line) runs down the layer; node +y runs right.
        assert_close(apply(m, [10.0, 0.0]), [0.0, 10.0]);
        assert_close(apply(m, [0.0, 10.0]), [10.0, 0.0]);

        let commands = lowered(
            &plan("lottery_game/new_year_2022", "idol"),
            OmikujiClient::default(),
        );
        let texts = texts(&commands);
        // The first title's glyph rows (node y 60..101) sit on its background
        // (layer x -6..44) and its line (node x from -1) starts just under the
        // background's top edge (layer y -210.2).
        let title = texts[0].1.node_matrix;
        assert_close(apply(title, [-1.0, 60.0]), [-1.8, -192.6]);
        assert_close(apply(title, [92.0, 101.0]), [39.2, -99.6]);
        // The summary hangs from the right of the text group.
        assert_close(apply(texts[3].1.node_matrix, [0.0, 0.0]), [536.0, -209.73]);
        let description_pivots = texts[4..]
            .iter()
            .map(|(_, source)| apply(source.node_matrix, [0.0, 0.0]))
            .collect::<Vec<_>>();
        assert_close(description_pivots[0], [-22.0, -195.0]);
        assert_close(description_pivots[1], [-175.0, -195.0]);
        assert_close(description_pivots[2], [-325.0, -195.0]);
    }

    #[test]
    fn text_bounds_cover_the_turned_node_rect_and_its_background() {
        let commands = lowered(
            &plan("lottery_game/new_year_2022", "idol"),
            OmikujiClient::default(),
        );
        let summary = &commands[5];
        assert_eq!(summary.role, "omikuji-summary");
        // The rect pivoted at its top-left corner, turned: it spans its height
        // left of the pivot and its width down.
        let b = summary.bounds;
        assert!((b.x - (536.0 - 134.812_13)).abs() < 1e-3, "{b:?}");
        assert!((b.y + 209.73).abs() < 1e-3, "{b:?}");
        assert!((b.width - 134.812_13).abs() < 1e-3, "{b:?}");
        assert!((b.height - 418.634_28).abs() < 1e-3, "{b:?}");
        let title = &commands[2];
        let background = match &title.payload {
            SemanticCommandPayload::UguiText(source) => source.backdrop.clone().unwrap().rect,
            other => panic!("{other:?}"),
        };
        assert!(title.bounds.x <= background.x && title.bounds.y <= background.y);
        assert!(
            title.bounds.x + title.bounds.width >= background.x + background.width
                && title.bounds.y + title.bounds.height >= background.y + background.height
        );
    }

    #[test]
    fn the_scene_draws_a_visible_omikuji_collection_from_its_visual() {
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "collections": (0..2).map(|index| serde_json::json!({
                "objectData": {
                    "layer": index + 1, "lock": false, "visible": index == 0,
                    "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                    "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
                },
                "id": 1, "targetId": 7
            })).collect::<Vec<_>>()
        }))
        .unwrap();
        let elements = crate::profile_scene::ordered_profile_elements(&card, "slips");
        let visual = plan("lottery_game/new_year_2022", "idol").visual(OmikujiClient::default());
        let mut snapshot = crate::profile_scene::ProfileResolveSnapshot::default();
        for element in &elements {
            snapshot
                .omikuji_visuals
                .insert(element.source_key.clone(), visual.clone());
        }
        let scene = crate::profile_scene::resolve_profile_scene(&card, "slips", &snapshot).unwrap();
        let layer_commands = |index: usize| {
            scene
                .commands
                .iter()
                .filter(|command| command.layer_id == elements[index].layer_id)
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(
            layer_commands(0),
            lower_omikuji(&elements[0].source_key, elements[0].layer_id, &visual)
        );
        assert_eq!(scene.layers[0].kind, crate::LayerKind::Composite);
        assert_eq!(
            scene.layers[0].resolved_parameters.get("omikuji_id"),
            Some(&crate::ParameterValue::I64(7))
        );
        assert_eq!(scene.layers[0].bounds.width, 1480.0);
        // A hidden omikuji draws nothing, like any hidden element.
        let hidden = layer_commands(1);
        assert_eq!(hidden.len(), 1);
        assert!(matches!(
            hidden[0].payload,
            SemanticCommandPayload::Composite { .. }
        ));
    }
}
