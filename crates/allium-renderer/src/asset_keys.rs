//! 素材 key 收集模块。

use crate::masterdata::MasterData;
use crate::profile::ProfileData;
use crate::types::CustomProfileCard;

/// 从单页名片中收集需要从 S3 下载的素材 key。
pub fn collect_card_asset_keys(card: &CustomProfileCard, md: &MasterData) -> Vec<String> {
    let mut keys = Vec::new();

    for e in &card.stamps {
        let abn = md
            .resolve_stamp(e.id)
            .unwrap_or_else(|| format!("stamp{:04}", e.id));
        keys.push(format!("stamp/{abn}/{abn}"));
    }

    for e in &card.others {
        if let Some(info) = md.resolve_resource("etc", e.id) {
            keys.push(format!("{}/{}", info.load_val, info.file_name));
        }
    }

    for e in &card.collections {
        if let Some(info) = md.resolve_resource("collection", e.id) {
            keys.push(format!("{}/{}", info.load_val, info.file_name));
        }
    }

    for e in &card.card_members {
        let suffix = if e.use_after_special_training.unwrap_or(false) {
            "after_training"
        } else {
            "normal"
        };
        let mtype = e.member_type.unwrap_or(2);
        if let Some(k) = resolve_card_member_key(e.id, mtype, suffix, md) {
            keys.push(k);
        }
    }

    for e in &card.shapes {
        if let Some(info) = md.resolve_resource("shape", e.id) {
            keys.push(format!("custom_profile/shape/{}", info.file_name));
        }
    }

    for e in &card.honors {
        collect_honor_keys(e.id, e.honor_level, e.full_size, md, &mut keys);
    }

    for e in &card.bonds_honors {
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

    for e in &card.stand_members {
        if let Some(info) = md.resolve_resource("standing", e.id) {
            keys.push(format!("{}/{}", info.load_val, info.file_name));
        }
    }
    for e in &card.general_backgrounds {
        if let Some(info) = md.resolve_resource("general_bg", e.id) {
            keys.push(format!("{}/{}", info.load_val, info.file_name));
        }
    }
    for e in &card.story_backgrounds {
        if let Some(info) = md.resolve_resource("story_bg", e.id) {
            keys.push(format!("{}/{}", info.load_val, info.file_name));
        }
    }

    keys
}

/// 将 `cardId + type + training` 解析为物理 S3 key。
pub fn resolve_card_member_key(
    card_id: i32,
    member_type: i32,
    training: &str,
    md: &MasterData,
) -> Option<String> {
    let card = md.get_card(card_id)?;
    let abn = &card.asset_bundle_name;
    Some(match member_type {
        1 => format!("character/member_cutout/{}/{}", abn, training),
        _ => format!("character/member_small/{}/card_{}", abn, training),
    })
}

/// 收集 `ProfileData` 中面板渲染需要的隐式素材 key。
pub fn collect_profile_asset_keys(profile: &ProfileData, md: &MasterData) -> Vec<String> {
    let mut keys = Vec::new();

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
            let inverse = slot.bonds_honor_view_type.as_deref() == Some("reverse");
            collect_bonds_honor_keys(
                slot.honor_id,
                slot.full_size,
                slot.bonds_honor_word_id.unwrap_or(0),
                inverse,
                false,
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
    keys: &mut Vec<String>,
) {
    let resolved = match md.resolve_honor(honor_id, honor_level) {
        Some(r) => r,
        None => return,
    };
    let suffix = if full_size { "main" } else { "sub" };

    let bg_abn = resolved
        .background_asset_bundle_name
        .as_deref()
        .unwrap_or(&resolved.asset_bundle_name);
    let bg_dir = if resolved.honor_type == "rank_match" {
        "rank_live/honor"
    } else {
        "honor"
    };
    keys.push(format!("{}/{}/degree_{}", bg_dir, bg_abn, suffix));

    if resolved.has_rank_overlay() {
        let (overlay_dir, overlay_name) = if resolved.honor_type == "rank_match" {
            ("rank_live/honor", suffix.to_string())
        } else if resolved.is_live_master {
            ("honor", "scroll".to_string())
        } else {
            ("honor", format!("rank_{}", suffix))
        };
        keys.push(format!(
            "{}/{}/{}",
            overlay_dir, resolved.asset_bundle_name, overlay_name
        ));
    }

    if let Some(ref fname) = resolved.frame_name {
        let sc = if full_size { "m" } else { "s" };
        let rarity = match resolved.honor_rarity.as_str() {
            "low" => 1,
            "middle" => 2,
            "high" => 3,
            _ => 4,
        };
        if rarity < 3 {
            return;
        }
        keys.push(format!(
            "honor_frame/{}/frame_degree_{}_{}",
            fname, sc, rarity
        ));
    }
}

fn collect_bonds_honor_keys(
    bonds_honor_id: i32,
    full_size: bool,
    word_id: i64,
    inverse: bool,
    use_unit_vs: bool,
    md: &MasterData,
    keys: &mut Vec<String>,
) {
    let entry = match md.get_bonds_honor(bonds_honor_id) {
        Some(e) => e,
        None => return,
    };
    let (mut cid1, mut cid2) = if inverse {
        (entry.game_character_unit_id2, entry.game_character_unit_id1)
    } else {
        (entry.game_character_unit_id1, entry.game_character_unit_id2)
    };
    if use_unit_vs {
        cid1 = md.resolve_unit_vs_sd(cid1, cid2);
        cid2 = md.resolve_unit_vs_sd(cid2, cid1);
    }

    keys.push(format!("bonds_honor/chr_sd_{:02}_01", cid1));
    keys.push(format!("bonds_honor/chr_sd_{:02}_01", cid2));

    if full_size {
        if let Some(word) = md.get_bonds_honor_word(word_id) {
            let abn = &word.assetbundle_name;
            keys.push(format!("bonds_honor/word/{}_01", abn));
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

/// 从 WidgetDocument 中收集所有 AssetImage 节点的 asset_key。
pub fn collect_document_asset_keys(document: &crate::widget_node::WidgetDocument) -> Vec<String> {
    let mut keys = Vec::new();
    collect_node_asset_keys(&document.root, &mut keys);
    keys.sort();
    keys.dedup();
    keys
}

fn collect_node_asset_keys(node: &crate::widget_node::WidgetNode, keys: &mut Vec<String>) {
    match &node.kind {
        crate::widget_node::NodeKind::AssetImage { asset_key, .. } => {
            if !asset_key.is_empty() {
                keys.push(asset_key.clone());
            }
        }
        crate::widget_node::NodeKind::Container { children, .. } => {
            for child in children {
                collect_node_asset_keys(child, keys);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::key_to_s3_path;

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
}
