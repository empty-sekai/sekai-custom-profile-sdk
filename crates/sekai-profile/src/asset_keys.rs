//! Collecting the asset keys a card or profile reads, and mapping keys to
//! storage paths.

use crate::masterdata::MasterData;
use crate::profile::ProfileData;
use crate::types::CustomProfileCard;
use sekai_profile_core::profile_resolve::{authored_resource, card_artwork_key, AuthoredResource};
use sekai_profile_core::profile_scene::ordered_profile_elements;
use sekai_profile_core::{masterdata::StandardHonorAssetPlan, ResourceKey};

#[derive(Default)]
struct AssetRequirements {
    // Each group needs one available member. A singleton is an ordinary asset.
    groups: Vec<Vec<ResourceKey>>,
}

impl AssetRequirements {
    fn push(&mut self, key: String) {
        self.resource(ResourceKey {
            namespace: "assets".into(),
            key,
        });
    }

    fn resource(&mut self, resource: ResourceKey) {
        self.groups.push(vec![resource]);
    }

    fn honor(&mut self, plan: &StandardHonorAssetPlan) {
        for key in std::iter::once(&plan.background).chain(plan.overlay.iter()) {
            self.groups.push(vec![key.clone()]);
        }
        self.groups
            .push(plan.frame_candidates.iter().flatten().cloned().collect());
        for key in plan.star.iter().chain(plan.star_high.iter()) {
            self.groups.push(vec![key.clone()]);
        }
    }

    fn keys(self) -> Vec<String> {
        self.groups
            .into_iter()
            .flatten()
            .map(|resource| resource.key)
            .collect()
    }

    fn missing(&self, mut available: impl FnMut(&str) -> bool) -> Vec<String> {
        let mut missing = Vec::new();
        for group in &self.groups {
            if !group.iter().any(|resource| available(&resource.key)) {
                missing.extend(group.iter().map(|resource| resource.key.clone()));
            }
        }
        missing.sort();
        missing.dedup();
        missing
    }
}

/// Collects possible asset keys, including alternatives that need not all exist.
pub fn collect_card_asset_keys(card: &CustomProfileCard, md: &MasterData) -> Vec<String> {
    card_asset_requirements(card, None, md).keys()
}

/// Reports missing assets without treating an available frame fallback as missing.
pub fn missing_card_asset_keys(
    card: &CustomProfileCard,
    md: &MasterData,
    available: impl FnMut(&str) -> bool,
) -> Vec<String> {
    card_asset_requirements(card, None, md).missing(available)
}

/// Same as [`missing_card_asset_keys`], with the card's honors taken at the
/// levels the player owns; honors the player does not own are not drawn and
/// need no assets.
pub fn missing_card_asset_keys_with_profile(
    card: &CustomProfileCard,
    profile: Option<&ProfileData>,
    md: &MasterData,
    available: impl FnMut(&str) -> bool,
) -> Vec<String> {
    card_asset_requirements(card, profile, md).missing(available)
}

fn card_asset_requirements(
    card: &CustomProfileCard,
    profile: Option<&ProfileData>,
    md: &MasterData,
) -> AssetRequirements {
    let mut keys = AssetRequirements::default();

    // Image elements (shapes, card artwork, stamps, customProfile*Resources)
    // take their keys from the core resource rules. Hidden elements and
    // elements whose masterdata row is missing are not built by the game and
    // are not counted. Lit badges also need a static normal map. Omikuji
    // collection items count both the slip and the fortune image; an omikuji
    // prefab without layout is not drawn and not counted.
    for element in ordered_profile_elements(card, "asset-keys") {
        if !element.object().visible {
            continue;
        }
        match authored_resource(element.value, md) {
            AuthoredResource::Request(request) => keys.push(request.resource.key),
            AuthoredResource::LitBadge { image, normal_map } => {
                keys.push(image.resource.key);
                keys.resource(normal_map.resource);
            }
            AuthoredResource::Omikuji(plan) => {
                keys.resource(plan.cover());
                keys.resource(plan.fortune());
            }
            AuthoredResource::None | AuthoredResource::MissingRow | AuthoredResource::NotDrawn => {}
        }
    }

    // Honors take their keys at the level the player owns; hidden or unowned
    // honors are not drawn and not counted.
    let owned_honors = profile.map(|profile| &profile.owned_honors);
    for e in card.honors.iter().filter(|e| e.object_data.visible) {
        if let Some(level) =
            sekai_profile_core::profile_data::placed_honor_level(owned_honors, e.id, e.honor_level)
        {
            collect_honor_keys(e.id, level, e.full_size, md, &mut keys);
        }
    }

    for e in card.bonds_honors.iter().filter(|e| e.object_data.visible) {
        if sekai_profile_core::profile_data::placed_bonds_honor_level(
            owned_honors,
            e.id,
            e.honor_level,
        )
        .is_none()
        {
            continue;
        }
        collect_bonds_honor_keys(
            e.id,
            e.full_size,
            e.word_id,
            e.inverse,
            e.use_unit_virtual_singer,
            md,
            &mut keys,
        );
    }

    keys
}

/// Resolves `cardId`, member type and training state to a card artwork asset
/// key. `training` is `"after_training"` for the trained illustration; any
/// other value selects the untrained one. Returns `None` when the card is not
/// in the `cards` table. See
/// [`sekai_profile_core::profile_resolve::card_artwork_key`]。
pub fn resolve_card_member_key(
    card_id: i32,
    member_type: i32,
    training: &str,
    md: &MasterData,
) -> Option<String> {
    let card = md.get_card(card_id)?;
    Some(card_artwork_key(
        &card.asset_bundle_name,
        member_type,
        training == "after_training",
    ))
}

/// Collects profile-panel asset candidates, including frame alternatives.
pub fn collect_profile_asset_keys(profile: &ProfileData, md: &MasterData) -> Vec<String> {
    profile_asset_requirements(profile, md).keys()
}

/// Reports missing profile-panel resources using the same fallback groups as cards.
pub fn missing_profile_asset_keys(
    profile: &ProfileData,
    md: &MasterData,
    available: impl FnMut(&str) -> bool,
) -> Vec<String> {
    profile_asset_requirements(profile, md).missing(available)
}

fn profile_asset_requirements(profile: &ProfileData, md: &MasterData) -> AssetRequirements {
    let mut keys = AssetRequirements::default();

    if let Some(lc) = &profile.leader_card {
        let suffix = if lc.after_training {
            "after_training"
        } else {
            "normal"
        };
        // Thumbnail for the player avatar.
        if let Some(card) = md.get_card(lc.card_id) {
            keys.push(format!(
                "thumbnail/chara/{}_{}",
                card.asset_bundle_name, suffix
            ));
        }
        // Full card artwork.
        if let Some(k) = resolve_card_member_key(lc.card_id, 2, suffix, md) {
            keys.push(k);
        }
    }

    for m in &profile.deck_members {
        let suffix = if m.after_training {
            "after_training"
        } else {
            "normal"
        };
        if let Some(k) = resolve_card_member_key(m.card_id, 1, suffix, md) {
            keys.push(k);
        }
    }

    // Player rank icon.
    keys.push("sprite/icon/icon_playerRank".to_string());

    for slot in &profile.honor_slots {
        if slot.profile_honor_type == "bonds" {
            let (inverse, use_unit_virtual_singer) = slot.bonds_honor_view_flags();
            collect_bonds_honor_keys(
                slot.honor_id,
                slot.full_size,
                slot.bonds_honor_word_id.unwrap_or(0),
                inverse,
                use_unit_virtual_singer,
                md,
                &mut keys,
            );
        } else {
            collect_honor_keys(
                slot.honor_id,
                slot.honor_level,
                slot.full_size,
                md,
                &mut keys,
            );
        }
    }

    // Banners of the favorite stories (type 14), looked up by story_type and
    // story_id.
    for sf in &profile.story_favorites {
        if let Some(k) = md.resolve_story_banner(&sf.story_type, sf.story_id) {
            keys.push(k);
        }
    }

    keys
}

fn collect_honor_keys(
    honor_id: i32,
    honor_level: i32,
    full_size: bool,
    md: &MasterData,
    keys: &mut AssetRequirements,
) {
    if let Some(resolved) = md.resolve_honor(honor_id, honor_level) {
        keys.honor(&resolved.asset_plan(full_size));
    }
}

fn collect_bonds_honor_keys(
    bonds_honor_id: i32,
    full_size: bool,
    word_id: i64,
    inverse: bool,
    use_unit_vs: bool,
    md: &MasterData,
    keys: &mut AssetRequirements,
) {
    if let Some(plan) = sekai_profile_core::masterdata::bonds_honor_asset_plan(
        md,
        bonds_honor_id,
        full_size,
        word_id,
        inverse,
        use_unit_vs,
    ) {
        for key in plan.resources() {
            keys.groups.push(vec![key.clone()]);
        }
    }
}

/// Maps a render asset key to an object storage path: `<prefix><key>.png`,
/// where the character and word images of bonds honors add their asset bundle
/// directory.
pub fn key_to_s3_path(key: &str, prefix: &str) -> String {
    if let Some(filename) = key.strip_prefix("bonds_honor/chr_sd_") {
        let full = format!("chr_sd_{filename}");
        format!("{prefix}bonds_honor/character/{full}/{full}.png")
    } else if let Some(dirname) = key.strip_prefix("bonds_honor/word/") {
        format!("{prefix}bonds_honor/word/{dirname}/{dirname}.png")
    } else {
        format!("{prefix}{key}.png")
    }
}

/// Maps an indexed game-asset path back to the runtime key used by `AssetStore`.
/// Most assets only drop their image extension; bonds Honor sources use an
/// additional directory layer that is not part of the runtime key.
pub fn logical_path_to_asset_key(logical_path: &str) -> Option<String> {
    let normalized = logical_path.trim_start_matches('/').replace('\\', "/");
    let (base, extension) = normalized.rsplit_once('.')?;
    if !matches!(
        extension.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg"
    ) {
        return None;
    }
    let parts = base.split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        ["bonds_honor", "character", directory, file]
            if directory == file && file.starts_with("chr_sd_") =>
        {
            Some(format!("bonds_honor/{file}"))
        }
        ["bonds_honor", "word", directory, file] if directory == file => {
            Some(format!("bonds_honor/word/{file}"))
        }
        _ => Some(base.to_string()),
    }
}

#[cfg(test)]
mod tests;
