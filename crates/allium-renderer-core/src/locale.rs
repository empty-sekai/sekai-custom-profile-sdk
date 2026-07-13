use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const GENERAL_LOCALIZATION_KEYS: &[&str] = &[
    "custom_profile.general.comment.title",
    "custom_profile.general.total_power.title",
    "custom_profile.general.challenge_live.title",
    "custom_profile.general.challenge_live.solo",
    "custom_profile.general.story_favorite.title",
    "custom_profile.general.multiplayer_live.title",
    "custom_profile.general.multiplayer_live.mvp",
    "custom_profile.general.multiplayer_live.superstar",
    "custom_profile.general.count_times",
    "custom_profile.general.character_rank.title",
    "custom_profile.general.character_rank.challenge",
    "custom_profile.general.music.clear",
    "custom_profile.general.music.full_combo",
    "custom_profile.general.music.all_perfect",
    "custom_profile.general.music.difficulty.easy",
    "custom_profile.general.music.difficulty.normal",
    "custom_profile.general.music.difficulty.hard",
    "custom_profile.general.music.difficulty.expert",
    "custom_profile.general.music.difficulty.master",
    "custom_profile.general.music.difficulty.append",
    "custom_profile.general.player_level.label",
    "custom_profile.general.card_level",
];

fn catalog(region: &str) -> Option<&'static BTreeMap<String, String>> {
    macro_rules! locale {
        ($cell:ident, $path:literal) => {{
            static $cell: OnceLock<BTreeMap<String, String>> = OnceLock::new();
            $cell.get_or_init(|| {
                serde_json::from_str(include_str!($path))
                    .expect(concat!("invalid renderer locale catalog: ", $path))
            })
        }};
    }
    Some(match region.trim().to_ascii_lowercase().as_str() {
        "cn" | "zh-cn" | "zh-hans" => locale!(CN, "../locales/cn.json"),
        "jp" | "ja" | "ja-jp" => locale!(JP, "../locales/jp.json"),
        "tw" | "zh-tw" | "zh-hant" => locale!(TW, "../locales/tw.json"),
        "en" | "en-us" | "en-gb" => locale!(EN, "../locales/en.json"),
        "kr" | "ko" | "ko-kr" => locale!(KR, "../locales/kr.json"),
        _ => return None,
    })
}

pub fn resolve(region: &str, key: &str) -> Option<String> {
    catalog(region)?.get(key).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_title_is_resolved_for_every_supported_region() {
        for (region, expected) in [
            ("cn", "个性签名"),
            ("jp", "ひとこと"),
            ("tw", "個性簽名"),
            ("en", "Bio"),
            ("kr", "한마디"),
        ] {
            assert_eq!(
                resolve(region, "custom_profile.general.comment.title"),
                Some(expected.to_string()),
                "region={region}"
            );
        }
        assert_eq!(
            resolve("unknown", "custom_profile.general.comment.title"),
            None
        );
        assert_eq!(resolve("cn", "unknown.key"), None);
    }

    #[test]
    fn locale_aliases_and_every_general_label_are_complete() {
        for (region, alias) in [
            ("cn", "zh-CN"),
            ("jp", "ja-JP"),
            ("tw", "zh-TW"),
            ("en", "en-US"),
            ("kr", "ko-KR"),
        ] {
            for key in GENERAL_LOCALIZATION_KEYS {
                assert_eq!(resolve(alias, key), resolve(region, key), "{alias}:{key}");
                assert!(resolve(alias, key).is_some(), "missing {alias}:{key}");
            }
        }
    }
}
