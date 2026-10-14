//! Stable ids of lowered commands, regions and controls, and the lookup keys
//! of resolved resources.

use crate::profile_source::CardMemberElement;
use crate::{AuthoredElementKind, StableId};

pub fn authored_kind_name(kind: AuthoredElementKind) -> &'static str {
    match kind {
        AuthoredElementKind::Text => "text",
        AuthoredElementKind::Shape => "shape",
        AuthoredElementKind::CardMember => "card-member",
        AuthoredElementKind::Stamp => "stamp",
        AuthoredElementKind::Other => "other",
        AuthoredElementKind::BondsHonor => "bonds-honor",
        AuthoredElementKind::Honor => "honor",
        AuthoredElementKind::Collection => "collection",
        AuthoredElementKind::General => "general",
        AuthoredElementKind::StandMember => "stand-member",
        AuthoredElementKind::GeneralBackground => "general-background",
        AuthoredElementKind::StoryBackground => "story-background",
        AuthoredElementKind::CharacterIcon => "character-icon",
        AuthoredElementKind::Material => "material",
        AuthoredElementKind::UserInterfaceIcon => "user-interface-icon",
    }
}

pub fn semantic_command_id(source_key: &str, role: &str, ordinal: u32) -> StableId {
    StableId::derive(
        "semantic-command-v1",
        format!("{source_key}\0{role}\0{ordinal}").as_bytes(),
    )
}

pub fn interaction_region_id(source_key: &str, role: &str, ordinal: u32) -> StableId {
    StableId::derive(
        "interaction-region-v1",
        format!("{source_key}\0{role}\0{ordinal}").as_bytes(),
    )
}

pub fn component_control_id(source_key: &str, role: &str) -> StableId {
    StableId::derive(
        "component-control-v1",
        format!("{source_key}\0{role}").as_bytes(),
    )
}

pub fn resource_lookup_key(kind: &str, id: i32, variant: &str) -> String {
    format!("{kind}\0{id}\0{variant}")
}

/// Lookup key of the normal map a can-badge collection draws its image with,
/// in [`ProfileResolveSnapshot::resources`]. A collection with this entry is
/// drawn with [`ImageMaterial::LitBadge`].
///
/// [`ProfileResolveSnapshot::resources`]: super::ProfileResolveSnapshot::resources
/// [`ImageMaterial::LitBadge`]: crate::ImageMaterial::LitBadge
pub fn collection_normal_map_lookup_key(id: i32) -> String {
    resource_lookup_key("collection", id, "normal-map")
}

/// Lookup key of a card-member element's artwork in
/// [`ProfileResolveSnapshot::resources`].
///
/// [`ProfileResolveSnapshot::resources`]: super::ProfileResolveSnapshot::resources
pub fn card_member_lookup_key(value: &CardMemberElement) -> String {
    let member_type = value.member_type.unwrap_or(2);
    let training = if value.use_after_special_training.unwrap_or(false) {
        "after_training"
    } else {
        "normal"
    };
    resource_lookup_key(
        "card-member",
        value.id,
        &format!("{member_type}:{training}"),
    )
}
