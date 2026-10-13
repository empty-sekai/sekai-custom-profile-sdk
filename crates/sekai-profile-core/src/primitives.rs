//! Stable identifiers, geometry aliases, and the value, style and kind types
//! that layers and semantic commands share.

use bincode::{Decode, Encode};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Encode, Decode)]
pub struct StableId(pub u64);

pub type SceneId = StableId;
pub type LayerId = StableId;
pub type GlyphId = StableId;
pub type CommandId = StableId;

impl StableId {
    pub fn derive(namespace: &str, source_key: &[u8]) -> Self {
        let mut hash = Sha256::new();
        hash.update((namespace.len() as u64).to_le_bytes());
        hash.update(namespace.as_bytes());
        hash.update((source_key.len() as u64).to_le_bytes());
        hash.update(source_key);
        let digest = hash.finalize();
        Self(u64::from_le_bytes(
            digest[..8].try_into().expect("sha256 prefix"),
        ))
    }
}

impl Serialize for StableId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            serializer.serialize_str(&format!("{:016x}", self.0))
        } else {
            serializer.serialize_u64(self.0)
        }
    }
}

impl<'de> Deserialize<'de> for StableId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if !deserializer.is_human_readable() {
            return u64::deserialize(deserializer).map(Self);
        }
        let raw = String::deserialize(deserializer)?;
        if raw.len() != 16 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(de::Error::custom(
                "stable id must be 16 lowercase/uppercase hex digits",
            ));
        }
        u64::from_str_radix(&raw, 16)
            .map(Self)
            .map_err(de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

pub type Matrix2d = [f32; 6];
pub type Quad = [[f32; 2]; 4];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ParameterValue {
    Bool(bool),
    I64(i64),
    F64(f64),
    Text(String),
    Vec2([f32; 2]),
    Color([f32; 4]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    Text,
    Image,
    Shape,
    Composite,
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    Deserialize,
    Encode,
    Decode,
)]
#[serde(rename_all = "snake_case")]
pub enum AuthoredElementKind {
    #[default]
    Text,
    Shape,
    CardMember,
    Stamp,
    Other,
    BondsHonor,
    Honor,
    Collection,
    General,
    StandMember,
    GeneralBackground,
    StoryBackground,
    CharacterIcon,
    Material,
    UserInterfaceIcon,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum FontRole {
    RegionFontId(i32),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TextSource {
    Authored {
        value: String,
    },
    ProfileField {
        field: String,
        value: String,
    },
    MasterData {
        table: String,
        key: String,
        value: String,
    },
    Localized {
        key: String,
        locale: String,
        value: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct ResourceKey {
    pub namespace: String,
    pub key: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    SrcOver,
    SrcIn,
    DstIn,
    Multiply,
    Screen,
    Add,
}

impl Default for BlendMode {
    fn default() -> Self {
        Self::SrcOver
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum CompositeOperation {
    #[default]
    Marker,
    BeginIsolation,
    EndIsolation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum ShapePrimitive {
    Rect,
    RoundedRect { radius: [f32; 2] },
    Ellipse,
    AssetMask { resource: ResourceKey },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct LinearGradient {
    /// Normalized shape-space coordinates.
    pub start: [f32; 2],
    pub end: [f32; 2],
    pub start_color: [f32; 4],
    pub end_color: [f32; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum ImageClip {
    RoundedRect { radius: [f32; 2] },
    Ellipse,
}

/// How an image command shades its texels.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Encode, Decode)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImageMaterial {
    /// The texel colour multiplied by the tint.
    #[default]
    Plain,
    /// The lit material of [`badge_material`](crate::badge_material), with `normal_map` sampled at
    /// the image's UV. The tint's alpha is the element alpha; the material
    /// applies no tint colour.
    LitBadge { normal_map: ResourceKey },
}

impl ImageMaterial {
    pub fn is_plain(&self) -> bool {
        matches!(self, Self::Plain)
    }
}
