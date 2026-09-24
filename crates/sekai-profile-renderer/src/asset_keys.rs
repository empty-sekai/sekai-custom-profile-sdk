//! 素材 key 收集模块。

use crate::masterdata::MasterData;
use crate::profile::ProfileData;
use crate::types::CustomProfileCard;
use sekai_profile_renderer_core::profile_resolve::{
    authored_resource, card_artwork_key, AuthoredResource,
};
use sekai_profile_renderer_core::profile_scene::ordered_profile_elements;
use sekai_profile_renderer_core::{masterdata::StandardHonorAssetPlan, ResourceKey};

#[derive(Default)]
struct AssetRequirements {
    // Each group needs one available member. A singleton is an ordinary asset.
    groups: Vec<Vec<ResourceKey>>,
}

impl AssetRequirements {
    fn push(&mut self, key: String) {
        self.groups.push(vec![ResourceKey {
            namespace: "assets".into(),
            key,
        }]);
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

    // 图片类元素（形状 / 卡面 / 贴纸 / customProfile*Resources）按 core 的
    // 资源规则取 key；隐藏元素与 masterdata 缺行的元素游戏不会构建，不计入。
    for element in ordered_profile_elements(card, "asset-keys") {
        if !element.object().visible {
            continue;
        }
        if let AuthoredResource::Request(request) = authored_resource(element.value, md) {
            keys.push(request.resource.key);
        }
    }

    let owned_honors = profile.map(|profile| &profile.owned_honors);
    for e in &card.honors {
        if let Some(level) = sekai_profile_renderer_core::profile_data::placed_honor_level(
            owned_honors,
            e.id,
            e.honor_level,
        ) {
            collect_honor_keys(e.id, level, e.full_size, md, &mut keys);
        }
    }

    for e in &card.bonds_honors {
        if sekai_profile_renderer_core::profile_data::placed_bonds_honor_level(
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

/// 将 `cardId + type + training` 解析为卡面素材 key，规则见
/// [`sekai_profile_renderer_core::profile_resolve::card_artwork_key`]。
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
        // player_avatar 组件使用缩略图
        if let Some(card) = md.get_card(lc.card_id) {
            keys.push(format!(
                "thumbnail/chara/{}_{}",
                card.asset_bundle_name, suffix
            ));
        }
        // deck 渲染使用完整卡面
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

    // player_level 组件素材
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

    // 最喜欢的剧情（type=14）的封面 banner，渲染时按 story_type/story_id 查 banner key。
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
    if let Some(plan) = sekai_profile_renderer_core::masterdata::bonds_honor_asset_plan(
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

/// 将渲染用的 asset key 映射为 S3 对象路径。
pub fn key_to_s3_path(key: &str, prefix: &str) -> String {
    // 用户上传文件直读:仅放行已审核通过的 UGC 前缀,key 即完整 S3 路径。
    // - ugc/{editor_image,avatar}/  当前已审通过图(CDN 可服务的公开前缀)
    // - uploads/{editor_image,avatar}/  历史已审图(遗留前缀,兼容旧文档引用)
    // - presets/  官方预设素材(游戏自带,免审核;私有 ACL,服务端用桶凭证直读)
    // 其余 uploads/ 子路径(如 _staging/、_pending_review/、越权猜测路径)不在白名单内,
    // 落到普通分支拼成读不到的路径,从而无法绕过审核渲染未通过/他人的对象。
    if key.starts_with("ugc/editor_image/")
        || key.starts_with("ugc/avatar/")
        || key.starts_with("uploads/editor_image/")
        || key.starts_with("uploads/avatar/")
        || key.starts_with("presets/")
    {
        return key.to_string();
    }
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
mod tests {
    use super::{key_to_s3_path, logical_path_to_asset_key};

    const PREFIX: &str = "assets/cn/";

    #[test]
    fn approved_ugc_prefixes_are_read_literally() {
        // 已审核通过的前缀:key 即完整 S3 路径,原样返回。
        // 当前 ugc/ 公开前缀(CDN 可服务)+ 历史 uploads/ 遗留前缀 + presets/ 官方预设。
        for key in [
            "ugc/editor_image/2026-06/abc",
            "ugc/avatar/2026-06/xyz",
            "uploads/editor_image/2026-06/abc",
            "uploads/avatar/2026-06/xyz",
            "presets/stamp/stamp_miku_1",
            "presets/mysekai/item_wood_1",
        ] {
            assert_eq!(
                key_to_s3_path(key, PREFIX),
                key,
                "已审前缀应原样返回: {key}"
            );
        }
    }

    #[test]
    fn unapproved_uploads_prefixes_do_not_resolve_to_themselves() {
        // 越权关键:待审/暂存/裸 uploads 路径不得原样返回,
        // 否则用户可手工构造文档引用未通过审核的对象绕过复核。
        for key in [
            "uploads/_pending_review/2026-06/abc",
            "uploads/_staging/2026-06/abc",
            "uploads/editor_image", // 缺尾部斜杠,不匹配白名单
            "uploads/../secret",
            "uploads/other/abc",
        ] {
            let got = key_to_s3_path(key, PREFIX);
            assert_ne!(got, key, "未授权 key 不应原样返回: {key}");
            assert!(
                got.starts_with(PREFIX),
                "未授权 key 应落到 prefix 分支: {key} -> {got}"
            );
        }
    }

    #[test]
    fn normal_game_assets_still_resolve() {
        assert_eq!(
            key_to_s3_path("character/member_small/abn/card_normal", PREFIX),
            "assets/cn/character/member_small/abn/card_normal.png"
        );
    }

    #[test]
    fn indexed_bonds_honor_paths_restore_runtime_keys() {
        for key in [
            "bonds_honor/chr_sd_01_01",
            "bonds_honor/word/honorname_0102_01_01",
            "honor/example/degree_main",
        ] {
            let logical_path = key_to_s3_path(key, "");
            assert_eq!(
                logical_path_to_asset_key(&logical_path).as_deref(),
                Some(key)
            );
        }
    }
    struct PairHonorProvider;

    impl crate::masterdata::MasterDataProvider for PairHonorProvider {
        fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> {
            None
        }
        fn get_card(&self, _: i32) -> Option<crate::types::CardEntry> {
            None
        }
        fn resolve_color(&self, _: i32) -> Option<crate::masterdata::ResolvedColor> {
            None
        }
        fn resolve_font(&self, _: i32) -> Option<String> {
            None
        }
        fn resolve_stamp(&self, _: i32) -> Option<String> {
            None
        }
        fn resolve_resource(&self, _: &str, _: i32) -> Option<crate::masterdata::ResourceInfo> {
            None
        }
        fn resolve_honor(&self, _: i32, _: i32) -> Option<crate::masterdata::ResolvedHonor> {
            None
        }
        fn get_bonds_honor(&self, id: i32) -> Option<crate::types::BondsHonorEntry> {
            Some(crate::types::BondsHonorEntry {
                id,
                game_character_unit_id1: 21,
                game_character_unit_id2: 1,
                honor_rarity: "middle".to_string(),
                configurable_unit_virtual_singer: true,
            })
        }
        fn get_bonds_honor_word(&self, id: i64) -> Option<crate::types::BondsHonorWordEntry> {
            Some(crate::types::BondsHonorWordEntry {
                id: id as i32,
                assetbundle_name: "honorname_0121_01".to_string(),
                bonds_group_id: 1,
                seq: 1,
            })
        }
        fn get_honor(&self, _: i32) -> Option<crate::types::HonorEntry> {
            None
        }
        fn resolve_unit_vs_sd(&self, self_id: i32, partner_id: i32) -> i32 {
            match (self_id, partner_id) {
                (21, 1) => 27,
                // Only reachable when the partner was already substituted.
                (1, 27) => 99,
                _ => self_id,
            }
        }
        fn font_count(&self) -> usize {
            0
        }
        fn color_count(&self) -> usize {
            0
        }
    }

    #[test]
    fn card_honors_follow_the_levels_the_player_owns() {
        let card: crate::types::CustomProfileCard = serde_json::from_value(serde_json::json!({
            "bondsHonors": [
                {
                    "objectData": {
                        "layer": 0, "lock": false, "visible": true,
                        "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                        "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                        "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
                    },
                    "id": 1, "wordId": 5, "fullSize": true, "inverse": false,
                    "useUnitVirtualSinger": false
                }
            ]
        }))
        .expect("card fixture");
        let md = crate::masterdata::MasterData::new(std::sync::Arc::new(PairHonorProvider));
        let unowned = crate::profile::ProfileData::from_json(&serde_json::json!({
            "userHonors": [],
            "userBondsHonors": [{ "bondsHonorId": 2, "level": 3 }]
        }));
        assert!(
            super::missing_card_asset_keys_with_profile(&card, Some(&unowned), &md, |_| false)
                .is_empty()
        );
        let owned = crate::profile::ProfileData::from_json(&serde_json::json!({
            "userBondsHonors": [{ "bondsHonorId": 1, "level": 3 }]
        }));
        assert!(
            super::missing_card_asset_keys_with_profile(&card, Some(&owned), &md, |_| false)
                .contains(&"honor/bonds/21".to_string())
        );
    }

    #[test]
    fn card_bonds_honors_request_every_layer_of_the_game_art() {
        let card: crate::types::CustomProfileCard = serde_json::from_value(serde_json::json!({
            "bondsHonors": [{
                "objectData": {
                    "layer": 0, "lock": false, "visible": true,
                    "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                    "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
                },
                "id": 1, "wordId": 5, "fullSize": true, "inverse": false,
                "useUnitVirtualSinger": true
            }]
        }))
        .expect("card fixture");
        let md = crate::masterdata::MasterData::new(std::sync::Arc::new(PairHonorProvider));
        let mut keys = super::collect_card_asset_keys(&card, &md);
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "bonds_honor/chr_sd_01_01",
                "bonds_honor/chr_sd_27_01",
                "bonds_honor/word/honorname_0121_01_02",
                "honor/bonds/1",
                "honor/bonds/21",
                "honor/frame_degree_m_2",
                "honor/icon_degreeLv",
                "honor/icon_degreeLv6",
                "honor/mask_degree_main",
            ]
        );
    }

    /// A character honor whose masterdata row carries neither a background
    /// bundle nor a frame still has to produce a downloadable key. The
    /// background falls back to the honor's own bundle name, so no key may
    /// collapse into an empty path segment.
    #[test]
    fn a_character_honor_without_background_or_frame_keeps_every_path_segment() {
        use crate::masterdata::{
            MasterData, MasterDataProvider, ResolvedColor, ResolvedHonor, ResourceInfo,
        };
        use crate::types::{BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry};

        struct BackgroundlessHonorProvider;

        impl MasterDataProvider for BackgroundlessHonorProvider {
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
            fn resolve_honor(&self, _: i32, honor_level: i32) -> Option<ResolvedHonor> {
                Some(ResolvedHonor {
                    asset_bundle_name: "honor_0001".to_string(),
                    honor_rarity: "high".to_string(),
                    honor_type: "character".to_string(),
                    background_asset_bundle_name: None,
                    frame_name: None,
                    is_live_master: false,
                    has_star: true,
                    honor_level,
                    honor_mission_type: None,
                })
            }
            fn get_bonds_honor(&self, _: i32) -> Option<BondsHonorEntry> {
                None
            }
            fn get_bonds_honor_word(&self, _: i64) -> Option<BondsHonorWordEntry> {
                None
            }
            fn get_honor(&self, _: i32) -> Option<HonorEntry> {
                None
            }
            fn resolve_unit_vs_sd(&self, self_id: i32, _: i32) -> i32 {
                self_id
            }
            fn font_count(&self) -> usize {
                0
            }
            fn color_count(&self) -> usize {
                0
            }
        }

        let md = MasterData::new(std::sync::Arc::new(BackgroundlessHonorProvider));
        let mut requirements = super::AssetRequirements::default();
        super::collect_honor_keys(1, 1, false, &md, &mut requirements);
        let keys = requirements.keys();

        let background = keys
            .iter()
            .find(|key| key.contains("degree_sub"))
            .expect("a background key");
        assert_eq!(background, "honor/honor_0001/degree_sub");
        for key in &keys {
            assert!(!key.contains("//"), "key has an empty path segment: {key}");
        }
    }
}
