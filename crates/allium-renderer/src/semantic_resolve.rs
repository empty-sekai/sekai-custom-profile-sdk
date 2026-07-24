//! Native resolve-snapshot adapter for the shared renderer v0.2 profile resolver.

use allium_renderer_core::profile_scene::{
    ordered_profile_elements, resolve_profile_scene, resource_lookup_key, CardVisualSnapshot,
    CharacterRankSnapshot, ComponentImageSnapshot, HonorVisualKind, HonorVisualSnapshot,
    MusicDifficultySnapshot, MusicResultsSnapshot, ProfileComponentSnapshot, ProfileElementRef,
    ProfileResolveError, ProfileResolveSnapshot, ResolvedProfileScene, ResourceDescriptor,
    StoryFavoriteSnapshot,
};
use allium_renderer_core::{ParameterValue, ResourceKey};

use crate::asset_keys::resolve_card_member_key;
use crate::masterdata::MasterData;
use crate::profile::ProfileData;
use crate::types::CustomProfileCard;

pub type ResolvedCardCommands = ResolvedProfileScene;
pub type ResolveError = ProfileResolveError;

#[derive(Clone, Copy, Default)]
pub struct ResolveResourceContext<'a> {
    pub assets: Option<&'a crate::assets::AssetStore>,
    pub render_objects: Option<&'a crate::render_object::MappedRenderObjectStore>,
    pub catalog_lookup_ns: Option<&'a std::cell::Cell<u64>>,
}

/// Player-scoped semantic state reused across every page in one first-seen
/// request. Page-local maps are installed only while a page is being resolved,
/// so the potentially large component snapshot is never cloned per page.
pub struct ProfileResolveBaseSnapshot {
    snapshot: ProfileResolveSnapshot,
}

impl ProfileResolveBaseSnapshot {
    pub fn component(&self) -> Option<&ProfileComponentSnapshot> {
        self.snapshot.component.as_ref()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProfileResolvePageTimings {
    pub page_snapshot_overlay_ns: u64,
    pub semantic_lowering_ns: u64,
}

pub fn resolve_card_commands(
    card: &CustomProfileCard,
    md: &MasterData,
    document_key: &str,
) -> Result<ResolvedCardCommands, ResolveError> {
    resolve_card_commands_with_profile(card, md, document_key, None, "und", None)
}

pub fn resolve_card_commands_with_profile(
    card: &CustomProfileCard,
    md: &MasterData,
    document_key: &str,
    profile: Option<&ProfileData>,
    locale: &str,
    assets: Option<&crate::assets::AssetStore>,
) -> Result<ResolvedCardCommands, ResolveError> {
    resolve_card_commands_with_resources(
        card,
        md,
        document_key,
        profile,
        locale,
        ResolveResourceContext {
            assets,
            render_objects: None,
            catalog_lookup_ns: None,
        },
    )
}

pub fn resolve_card_commands_with_resources(
    card: &CustomProfileCard,
    md: &MasterData,
    document_key: &str,
    profile: Option<&ProfileData>,
    locale: &str,
    resources: ResolveResourceContext<'_>,
) -> Result<ResolvedCardCommands, ResolveError> {
    let mut base = if card.generals.is_empty() {
        // Shared core only reads ProfileResolveSnapshot::component while lowering General
        // elements. Avoid building the player-wide component graph for text/shape/image pages.
        ProfileResolveBaseSnapshot {
            snapshot: ProfileResolveSnapshot::default(),
        }
    } else {
        build_profile_resolve_base_snapshot(card, md, profile, locale, resources)
    };
    resolve_card_commands_with_base(&mut base, card, md, document_key, profile, resources)
}

pub fn build_profile_resolve_base_snapshot(
    seed_card: &CustomProfileCard,
    md: &MasterData,
    profile: Option<&ProfileData>,
    locale: &str,
    resources: ResolveResourceContext<'_>,
) -> ProfileResolveBaseSnapshot {
    let mut snapshot = build_resolve_snapshot_parts(
        seed_card,
        md,
        "profile-base",
        profile,
        locale,
        resources,
        false,
        true,
    );
    // Font 1 is copied into component.region_fonts while building the base.
    // Keeping it in this top-level map would make page overlay replacement
    // ambiguous, so only the immutable component remains in the base.
    snapshot.fonts.clear();
    ProfileResolveBaseSnapshot { snapshot }
}

pub fn resolve_card_commands_with_base(
    base: &mut ProfileResolveBaseSnapshot,
    card: &CustomProfileCard,
    md: &MasterData,
    document_key: &str,
    profile: Option<&ProfileData>,
    resources: ResolveResourceContext<'_>,
) -> Result<ResolvedCardCommands, ResolveError> {
    resolve_card_commands_with_base_timed(base, card, md, document_key, profile, resources)
        .map(|(scene, _)| scene)
}

pub fn resolve_card_commands_with_base_timed(
    base: &mut ProfileResolveBaseSnapshot,
    card: &CustomProfileCard,
    md: &MasterData,
    document_key: &str,
    profile: Option<&ProfileData>,
    resources: ResolveResourceContext<'_>,
) -> Result<(ResolvedCardCommands, ProfileResolvePageTimings), ResolveError> {
    let overlay_started = std::time::Instant::now();
    debug_assert!(base.snapshot.fonts.is_empty());
    debug_assert!(base.snapshot.colors.is_empty());
    debug_assert!(base.snapshot.resources.is_empty());
    debug_assert!(base.snapshot.line_indent.is_empty());
    debug_assert!(base.snapshot.honor_visuals.is_empty());
    debug_assert!(base.snapshot.card_member_visuals.is_empty());
    populate_resolve_snapshot_parts(
        &mut base.snapshot,
        card,
        md,
        document_key,
        profile,
        "unused-page-locale",
        resources,
        true,
        false,
    );
    let page_snapshot_overlay_ns = elapsed_ns(overlay_started);
    let semantic_started = std::time::Instant::now();
    let result = resolve_profile_scene(card, document_key, &base.snapshot);
    let semantic_lowering_ns = elapsed_ns(semantic_started);
    base.snapshot.fonts.clear();
    base.snapshot.colors.clear();
    base.snapshot.resources.clear();
    base.snapshot.line_indent.clear();
    base.snapshot.honor_visuals.clear();
    base.snapshot.card_member_visuals.clear();
    result.map(|scene| {
        (
            scene,
            ProfileResolvePageTimings {
                page_snapshot_overlay_ns,
                semantic_lowering_ns,
            },
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn build_resolve_snapshot_parts(
    card: &CustomProfileCard,
    md: &MasterData,
    document_key: &str,
    profile: Option<&ProfileData>,
    locale: &str,
    resources: ResolveResourceContext<'_>,
    include_page_overlay: bool,
    include_profile_base: bool,
) -> ProfileResolveSnapshot {
    let mut snapshot = ProfileResolveSnapshot::default();
    populate_resolve_snapshot_parts(
        &mut snapshot,
        card,
        md,
        document_key,
        profile,
        locale,
        resources,
        include_page_overlay,
        include_profile_base,
    );
    snapshot
}

#[allow(clippy::too_many_arguments)]
fn populate_resolve_snapshot_parts(
    snapshot: &mut ProfileResolveSnapshot,
    card: &CustomProfileCard,
    md: &MasterData,
    document_key: &str,
    profile: Option<&ProfileData>,
    locale: &str,
    resources: ResolveResourceContext<'_>,
    include_page_overlay: bool,
    include_profile_base: bool,
) {
    // Keep the existing call sites compact while the value now carries both
    // the legacy AssetStore and the mmap metadata catalog.
    let assets = resources;
    if include_page_overlay {
        for text in &card.texts {
            if !snapshot.fonts.contains_key(&text.font_id) {
                if let Some(font) = md.resolve_font(text.font_id) {
                    snapshot.fonts.insert(text.font_id, font);
                }
            }
            insert_color(snapshot, md, text.color_id);
            insert_color(snapshot, md, text.outline_color_id);
        }
        for shape in &card.shapes {
            insert_color(snapshot, md, shape.color_id);
            insert_color(snapshot, md, shape.outline_color_id);
            let lookup_key = resource_lookup_key("shape", shape.id, "");
            if snapshot.resources.contains_key(&lookup_key) {
                continue;
            }
            let key = md
                .resolve_resource("shape", shape.id)
                .map(|resource| format!("custom_profile/shape/{}", resource.file_name))
                .unwrap_or_else(|| format!("custom_profile/shape/{}", shape.id));
            insert_resource_descriptor(
                snapshot,
                lookup_key,
                key,
                (1024.0, 1024.0),
                "customProfileResource",
                shape.id,
                assets,
            );
        }
        for value in &card.card_members {
            let member_type = value.member_type.unwrap_or(2);
            let training = if value.use_after_special_training.unwrap_or(false) {
                "after_training"
            } else {
                "normal"
            };
            let lookup_key = resource_lookup_key(
                "card-member",
                value.id,
                &format!("{member_type}:{training}"),
            );
            if snapshot.resources.contains_key(&lookup_key) {
                continue;
            }
            let key = resolve_card_member_key(value.id, member_type, training, md)
                .unwrap_or_else(|| format!("card_member/{}", value.id));
            insert_resource_descriptor(
                snapshot,
                lookup_key,
                key,
                if member_type == 1 {
                    (312.0, 512.0)
                } else {
                    (940.0, 530.0)
                },
                "cards",
                value.id,
                assets,
            );
        }
        for value in &card.stamps {
            let lookup_key = resource_lookup_key("stamp", value.id, "");
            if snapshot.resources.contains_key(&lookup_key) {
                continue;
            }
            let assetbundle_name = md
                .resolve_stamp(value.id)
                .unwrap_or_else(|| format!("stamp{:04}", value.id));
            insert_resource_descriptor(
                snapshot,
                lookup_key,
                format!("stamp/{assetbundle_name}/{assetbundle_name}"),
                (100.0, 100.0),
                "stamps",
                value.id,
                assets,
            );
        }
        for value in &card.others {
            insert_master_resource(snapshot, md, "other", "etc", value.id, assets);
        }
        for value in &card.collections {
            insert_master_resource(snapshot, md, "collection", "collection", value.id, assets);
        }
        for value in &card.stand_members {
            insert_master_resource(snapshot, md, "stand-member", "standing", value.id, assets);
        }
        for value in &card.general_backgrounds {
            insert_master_resource(
                snapshot,
                md,
                "general-background",
                "general_bg",
                value.id,
                assets,
            );
        }
        for value in &card.story_backgrounds {
            insert_master_resource(
                snapshot,
                md,
                "story-background",
                "story_bg",
                value.id,
                assets,
            );
        }
        for element in ordered_profile_elements(card, document_key) {
            match element.value {
                ProfileElementRef::Text(text) => {
                    if let Some(mut program) = crate::text::line_indent_program(text, md) {
                        let (_, _, rotation_deg, scale_x, _) =
                            allium_renderer_core::profile_transform::extract_transform(
                                element.object(),
                            );
                        program.rotation_deg = rotation_deg;
                        program.scale_x = scale_x;
                        snapshot.line_indent.insert(element.source_key, program);
                    }
                }
                ProfileElementRef::Honor(honor) => {
                    if let Some(visual) = build_standard_honor_visual(
                        "customProfile.honors",
                        honor.id,
                        honor.honor_level,
                        honor.full_size,
                        profile,
                        md,
                        assets,
                    ) {
                        snapshot.honor_visuals.insert(element.source_key, visual);
                    }
                }
                ProfileElementRef::BondsHonor(honor) => {
                    if let Some(visual) = build_bonds_honor_visual(
                        "customProfile.bondsHonors",
                        honor.id,
                        honor.honor_level,
                        honor.full_size,
                        honor.word_id,
                        honor.inverse,
                        honor.use_unit_virtual_singer,
                        md,
                        assets,
                    ) {
                        snapshot.honor_visuals.insert(element.source_key, visual);
                    }
                }
                ProfileElementRef::CardMember(value) if value.show_master_rank.unwrap_or(false) => {
                    if let Some(card) = md.get_card(value.id) {
                        let member_type = value.member_type.unwrap_or(2);
                        let training = if value.use_after_special_training.unwrap_or(false) {
                            "after_training"
                        } else {
                            "normal"
                        };
                        let lookup_key = resource_lookup_key(
                            "card-member",
                            value.id,
                            &format!("{member_type}:{training}"),
                        );
                        let user_card = profile.and_then(|profile| profile.user_card(value.id));
                        snapshot.card_member_visuals.insert(
                            element.source_key,
                            CardVisualSnapshot {
                                card_id: value.id,
                                after_training: value.use_after_special_training.unwrap_or_else(
                                    || user_card.is_some_and(|card| card.after_training),
                                ),
                                master_rank: user_card.map_or(0, |card| card.master_rank),
                                level: user_card.map_or(60, |card| card.level),
                                rarity: card.card_rarity_type,
                                attribute: card.attr,
                                image: ComponentImageSnapshot {
                                    source_field: "customProfile.cardMembers".into(),
                                    source_id: value.id.to_string(),
                                    descriptor: snapshot.resources.get(&lookup_key).cloned(),
                                },
                            },
                        );
                        if member_type == 1 && !snapshot.fonts.contains_key(&1) {
                            if let Some(font) = md.resolve_font(1) {
                                snapshot.fonts.insert(1, font);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if include_profile_base {
        if let Some(profile) = profile {
            let challenge_rank_by_character = profile
                .challenge_ranks
                .iter()
                .map(|challenge| (challenge.character_id, challenge.rank))
                .collect::<std::collections::HashMap<_, _>>();
            if let Some(font) = md.resolve_font(1) {
                snapshot.fonts.insert(1, font);
            }
            let story_favorites = profile
                .story_favorites
                .iter()
                .map(|favorite| StoryFavoriteSnapshot {
                    story_id: favorite.story_id,
                    story_type: favorite.story_type.clone(),
                    image: ComponentImageSnapshot {
                        source_field: "userProfile.storyFavorites".into(),
                        source_id: format!("{}:{}", favorite.story_type, favorite.story_id),
                        descriptor: md
                            .resolve_story_banner(&favorite.story_type, favorite.story_id)
                            .map(|key| {
                                make_resource_descriptor(
                                    key,
                                    (400.0, 170.0),
                                    "storyFavorites",
                                    favorite.story_id,
                                    assets,
                                )
                            }),
                    },
                })
                .collect();
            let player_avatar = profile.leader_card.as_ref().and_then(|leader| {
                let card = md.get_card(leader.card_id)?;
                let training = if leader.after_training {
                    "after_training"
                } else {
                    "normal"
                };
                Some(ComponentImageSnapshot {
                    source_field: "userProfile.leaderCard".into(),
                    source_id: leader.card_id.to_string(),
                    descriptor: Some(make_resource_descriptor(
                        format!("thumbnail/chara/{}_{}", card.asset_bundle_name, training),
                        (180.0, 180.0),
                        "cards",
                        leader.card_id,
                        assets,
                    )),
                })
            });
            let character_ranks = profile
                .char_ranks
                .iter()
                .map(|rank| CharacterRankSnapshot {
                    character_id: rank.character_id,
                    rank: rank.rank,
                    challenge_rank: challenge_rank_by_character.get(&rank.character_id).copied(),
                    avatar: ComponentImageSnapshot {
                        source_field: "userProfile.characterRanks".into(),
                        source_id: rank.character_id.to_string(),
                        descriptor: Some(make_resource_descriptor(
                            format!("chara_avatar/chara{:02}_02", rank.character_id),
                            (76.0, 76.0),
                            "gameCharacters",
                            rank.character_id,
                            assets,
                        )),
                    },
                })
                .collect();
            let challenge_avatar =
                (profile.challenge_character_id > 0).then(|| ComponentImageSnapshot {
                    source_field: "userProfile.challengeLiveSoloResult.characterId".into(),
                    source_id: profile.challenge_character_id.to_string(),
                    descriptor: Some(make_resource_descriptor(
                        format!("chara_avatar/chara{:02}_02", profile.challenge_character_id),
                        (76.0, 76.0),
                        "gameCharacters",
                        profile.challenge_character_id,
                        assets,
                    )),
                });
            let deck_members = profile
                .deck_members
                .iter()
                .filter_map(|member| {
                    let card = md.get_card(member.card_id)?;
                    let training = if member.after_training {
                        "after_training"
                    } else {
                        "normal"
                    };
                    Some(CardVisualSnapshot {
                        card_id: member.card_id,
                        after_training: member.after_training,
                        master_rank: member.master_rank,
                        level: member.level,
                        rarity: card.card_rarity_type.clone(),
                        attribute: card.attr.clone(),
                        image: ComponentImageSnapshot {
                            source_field: "userProfile.deckMembers".into(),
                            source_id: member.card_id.to_string(),
                            descriptor: resolve_card_member_key(member.card_id, 1, training, md)
                                .map(|key| {
                                    make_resource_descriptor(
                                        key,
                                        (600.0, 576.0),
                                        "cards",
                                        member.card_id,
                                        assets,
                                    )
                                }),
                        },
                    })
                })
                .collect();
            let leader_card = profile.leader_card.as_ref().and_then(|leader| {
                let card = md.get_card(leader.card_id)?;
                let training = if leader.after_training {
                    "after_training"
                } else {
                    "normal"
                };
                Some(CardVisualSnapshot {
                    card_id: leader.card_id,
                    after_training: leader.after_training,
                    master_rank: leader.master_rank,
                    level: profile
                        .user_cards
                        .get(&leader.card_id)
                        .map(|card| card.level)
                        .unwrap_or_default(),
                    rarity: card.card_rarity_type.clone(),
                    attribute: card.attr.clone(),
                    image: ComponentImageSnapshot {
                        source_field: "userProfile.leaderCard".into(),
                        source_id: leader.card_id.to_string(),
                        descriptor: resolve_card_member_key(leader.card_id, 2, training, md).map(
                            |key| {
                                make_resource_descriptor(
                                    key,
                                    (940.0, 530.0),
                                    "cards",
                                    leader.card_id,
                                    assets,
                                )
                            },
                        ),
                    },
                })
            });
            let mut ordered_honor_slots = profile.honor_slots.iter().collect::<Vec<_>>();
            ordered_honor_slots.sort_by(|left, right| right.full_size.cmp(&left.full_size));
            if ordered_honor_slots.len() >= 2 {
                ordered_honor_slots.swap(0, 1);
            }
            let honor_slots = ordered_honor_slots
                .into_iter()
                .enumerate()
                .filter_map(|(index, slot)| {
                    let render_full = index == 0;
                    if slot.profile_honor_type == "bonds" {
                        build_bonds_honor_visual(
                            "userProfile.honorSlots",
                            slot.honor_id,
                            slot.honor_level,
                            render_full,
                            slot.bonds_honor_word_id.unwrap_or_default(),
                            slot.bonds_honor_view_type.as_deref() == Some("reverse"),
                            false,
                            md,
                            assets,
                        )
                    } else {
                        build_standard_honor_visual(
                            "userProfile.honorSlots",
                            slot.honor_id,
                            slot.honor_level,
                            render_full,
                            Some(profile),
                            md,
                            assets,
                        )
                    }
                })
                .collect();
            snapshot.component = Some(ProfileComponentSnapshot {
                locale: locale.into(),
                region_fonts: snapshot
                    .fonts
                    .get(&1)
                    .cloned()
                    .map(|font| std::collections::BTreeMap::from([(1, font)]))
                    .unwrap_or_default(),
                localized_text: allium_renderer_core::locale::GENERAL_LOCALIZATION_KEYS
                    .iter()
                    .filter_map(|key| {
                        md.resolve_localized_text(key)
                            .or_else(|| allium_renderer_core::locale::resolve(locale, key))
                            .map(|value| ((*key).into(), value))
                    })
                    .collect(),
                user_name: profile.user_name.clone(),
                word: profile.word.clone(),
                user_rank: profile.user_rank,
                total_power: profile.total_power,
                mvp: profile.mvp,
                superstar: profile.superstar,
                challenge_score: profile.challenge_score,
                challenge_character_id: profile.challenge_character_id,
                challenge_avatar,
                music_results: profile
                    .music_results
                    .as_ref()
                    .map(|results| MusicResultsSnapshot {
                        easy: music_stats(results.easy.clone()),
                        normal: music_stats(results.normal.clone()),
                        hard: music_stats(results.hard.clone()),
                        expert: music_stats(results.expert.clone()),
                        master: music_stats(results.master.clone()),
                        append: music_stats(results.append.clone()),
                    }),
                story_favorites,
                player_avatar,
                character_ranks,
                deck_members,
                leader_card,
                honor_slots,
            });
        }
    }
}

fn music_stats(value: crate::profile::MusicDifficultyStats) -> MusicDifficultySnapshot {
    MusicDifficultySnapshot {
        clear: value.clear,
        full_combo: value.full_combo,
        all_perfect: value.all_perfect,
    }
}

fn build_standard_honor_visual(
    source_field: &str,
    honor_id: i32,
    honor_level: i32,
    full_size: bool,
    profile: Option<&ProfileData>,
    md: &MasterData,
    resources: ResolveResourceContext<'_>,
) -> Option<HonorVisualSnapshot> {
    let resolved = md.resolve_honor(honor_id, honor_level)?;
    let (w, h, suffix, size_char) = if full_size {
        (380.0, 80.0, "main", "m")
    } else {
        (180.0, 80.0, "sub", "s")
    };
    let progress = profile
        .and_then(|profile| {
            resolved
                .honor_mission_type
                .as_ref()
                .and_then(|kind| profile.user_honor_missions.get(kind))
        })
        .copied()
        .unwrap_or_default();
    let object_key =
        crate::render_object::standard_honor_object_key(honor_id, honor_level, full_size);
    if let Some(base) = render_object_descriptor(
        resources,
        &object_key,
        crate::render_object::RenderObjectKind::StandardHonor,
        honor_id,
    ) {
        return Some(HonorVisualSnapshot {
            source_field: source_field.into(),
            source_id: honor_id.to_string(),
            honor_id,
            honor_level,
            full_size,
            visual: HonorVisualKind::Standard {
                honor_type: resolved.honor_type,
                has_star: false,
                is_live_master: resolved.is_live_master,
                progress,
                background: Some(base),
                frame_candidates: Vec::new(),
                overlay: None,
                star: None,
                star_high: None,
                live_star_on: None,
                live_star_off: None,
            },
        });
    }
    let background_name = resolved.effective_background_asset_bundle_name();
    let background_dir = if resolved.honor_type == "rank_match" {
        "rank_live/honor"
    } else {
        "honor"
    };
    let background = optional_descriptor(
        format!("{background_dir}/{background_name}/degree_{suffix}"),
        (w, h),
        "honor_background",
        honor_id,
        resources,
    );
    let rarity = honor_rarity_number(&resolved.honor_rarity);
    let custom_frame = resolved
        .frame_name
        .as_ref()
        .map(|name| format!("honor_frame/{name}/frame_degree_{size_char}_{rarity}"));
    let default_frame = format!("honor/frame_degree_{size_char}_{rarity}");
    let frame_key = custom_frame
        .filter(|key| asset_size(resources, "static", key).is_some())
        .unwrap_or(default_frame);
    let frame = optional_descriptor(frame_key, (w, h), "honor_frame", honor_id, resources);
    let (overlay_dir, overlay_name) = if resolved.honor_type == "rank_match" {
        ("rank_live/honor", suffix.to_string())
    } else if resolved.is_live_master {
        ("honor", "scroll".into())
    } else if resolved.honor_type == "character" {
        ("honor", format!("rank_{suffix}_{}", honor_level / 10 + 1))
    } else {
        ("honor", format!("rank_{suffix}"))
    };
    let overlay = resolved
        .has_rank_overlay()
        .then(|| {
            optional_descriptor(
                format!(
                    "{overlay_dir}/{}/{overlay_name}",
                    resolved.asset_bundle_name
                ),
                (w, h),
                "honor_overlay",
                honor_id,
                resources,
            )
        })
        .flatten();
    Some(HonorVisualSnapshot {
        source_field: source_field.into(),
        source_id: honor_id.to_string(),
        honor_id,
        honor_level,
        full_size,
        visual: HonorVisualKind::Standard {
            honor_type: resolved.honor_type,
            has_star: resolved.has_star,
            is_live_master: resolved.is_live_master,
            progress,
            background,
            frame_candidates: vec![frame],
            overlay,
            star: optional_descriptor(
                "honor/icon_degreeLv".into(),
                (16.0, 16.0),
                "honor_static",
                honor_id,
                resources,
            ),
            star_high: optional_descriptor(
                "honor/icon_degreeLv6".into(),
                (16.0, 16.0),
                "honor_static",
                honor_id,
                resources,
            ),
            live_star_on: None,
            live_star_off: None,
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn build_bonds_honor_visual(
    source_field: &str,
    honor_id: i32,
    honor_level: i32,
    full_size: bool,
    word_id: i64,
    inverse: bool,
    use_unit_virtual_singer: bool,
    md: &MasterData,
    resources: ResolveResourceContext<'_>,
) -> Option<HonorVisualSnapshot> {
    let entry = md.get_bonds_honor(honor_id)?;
    let object_key = crate::render_object::bonds_honor_object_key(
        honor_id,
        honor_level,
        full_size,
        word_id,
        inverse,
        use_unit_virtual_singer,
    );
    if let Some(base) = render_object_descriptor(
        resources,
        &object_key,
        crate::render_object::RenderObjectKind::BondsHonor,
        honor_id,
    ) {
        return Some(HonorVisualSnapshot {
            source_field: source_field.into(),
            source_id: honor_id.to_string(),
            honor_id,
            honor_level,
            full_size,
            visual: HonorVisualKind::Standard {
                honor_type: "bonds_prebuilt".into(),
                has_star: false,
                is_live_master: false,
                progress: 0,
                background: Some(base),
                frame_candidates: Vec::new(),
                overlay: None,
                star: None,
                star_high: None,
                live_star_on: None,
                live_star_off: None,
            },
        });
    }
    let (first, second) = if inverse {
        (entry.game_character_unit_id2, entry.game_character_unit_id1)
    } else {
        (entry.game_character_unit_id1, entry.game_character_unit_id2)
    };
    let character_ids = if use_unit_virtual_singer {
        [
            md.resolve_unit_vs_sd(first, second),
            md.resolve_unit_vs_sd(second, first),
        ]
    } else {
        [first, second]
    };
    let (w, h, size_char) = if full_size {
        (380.0, 80.0, "m")
    } else {
        (180.0, 80.0, "s")
    };
    let background_key = |id: i32| {
        if full_size {
            format!("honor/bonds/{id}")
        } else {
            format!("honor/bonds/{id}_sub")
        }
    };
    let rarity = honor_rarity_number(&entry.honor_rarity);
    Some(HonorVisualSnapshot {
        source_field: source_field.into(),
        source_id: honor_id.to_string(),
        honor_id,
        honor_level,
        full_size,
        visual: HonorVisualKind::Bonds {
            character_ids,
            backgrounds: [
                Some(static_descriptor(
                    background_key(first),
                    (w, h),
                    "bonds_honor_background",
                    honor_id,
                    resources,
                )),
                Some(static_descriptor(
                    background_key(second),
                    (w, h),
                    "bonds_honor_background",
                    honor_id,
                    resources,
                )),
            ],
            characters: [
                Some(static_descriptor(
                    format!("bonds_honor/chr_sd_{:02}_01", character_ids[0]),
                    (160.0, 160.0),
                    "bonds_honor_character",
                    honor_id,
                    resources,
                )),
                Some(static_descriptor(
                    format!("bonds_honor/chr_sd_{:02}_01", character_ids[1]),
                    (160.0, 160.0),
                    "bonds_honor_character",
                    honor_id,
                    resources,
                )),
            ],
            mask: Some(static_descriptor(
                if full_size {
                    "honor/mask_degree_main".into()
                } else {
                    "honor/mask_degree_sub".into()
                },
                (w, h),
                "honor_static",
                honor_id,
                resources,
            )),
            frame: Some(static_descriptor(
                format!("honor/frame_degree_{size_char}_{rarity}"),
                (w, h),
                "honor_frame",
                honor_id,
                resources,
            )),
            word: full_size
                .then(|| md.get_bonds_honor_word(word_id))
                .flatten()
                .map(|word| {
                    static_descriptor(
                        format!("bonds_honor/word/{}_01", word.assetbundle_name),
                        (180.0, 40.0),
                        "bonds_honor_word",
                        word_id as i32,
                        resources,
                    )
                }),
            star: Some(static_descriptor(
                "honor/icon_degreeLv".into(),
                (16.0, 16.0),
                "honor_static",
                honor_id,
                resources,
            )),
            star_high: Some(static_descriptor(
                "honor/icon_degreeLv6".into(),
                (16.0, 16.0),
                "honor_static",
                honor_id,
                resources,
            )),
        },
    })
}

fn honor_rarity_number(value: &str) -> i32 {
    match value {
        "low" => 1,
        "middle" => 2,
        "high" => 3,
        _ => 4,
    }
}

fn render_object_descriptor(
    resources: ResolveResourceContext<'_>,
    object_key: &str,
    expected_kind: crate::render_object::RenderObjectKind,
    id: i32,
) -> Option<ResourceDescriptor> {
    let metadata = resources.render_objects?.metadata(object_key)?;
    if metadata.kind != expected_kind {
        return None;
    }
    Some(ResourceDescriptor {
        resource: ResourceKey {
            namespace: "render-object".into(),
            key: object_key.into(),
        },
        natural_width: metadata.width as f32,
        natural_height: metadata.height as f32,
        provenance: std::collections::BTreeMap::from([
            ("kind".into(), ParameterValue::Text("render_object".into())),
            ("table".into(), ParameterValue::Text("honor_final".into())),
            ("id".into(), ParameterValue::I64(id.into())),
        ]),
    })
}

fn static_descriptor(
    key: String,
    fallback_size: (f32, f32),
    table: &str,
    id: i32,
    resources: ResolveResourceContext<'_>,
) -> ResourceDescriptor {
    let is_pinned_static = resource_is_static(resources, &key);
    let namespace = if is_pinned_static { "static" } else { "assets" };
    let natural_size = asset_size(resources, namespace, &key).unwrap_or(fallback_size);
    ResourceDescriptor {
        resource: ResourceKey {
            namespace: if is_pinned_static { "static" } else { "assets" }.into(),
            key,
        },
        natural_width: natural_size.0,
        natural_height: natural_size.1,
        provenance: std::collections::BTreeMap::from([
            (
                "kind".into(),
                ParameterValue::Text(
                    if is_pinned_static {
                        "renderer_static"
                    } else {
                        "region_asset"
                    }
                    .into(),
                ),
            ),
            ("table".into(), ParameterValue::Text(table.into())),
            ("id".into(), ParameterValue::I64(id.into())),
        ]),
    }
}

fn optional_descriptor(
    key: String,
    fallback_size: (f32, f32),
    table: &str,
    id: i32,
    resources: ResolveResourceContext<'_>,
) -> Option<ResourceDescriptor> {
    if !resource_exists(resources, &key) {
        return None;
    }
    Some(static_descriptor(key, fallback_size, table, id, resources))
}

fn insert_color(snapshot: &mut ProfileResolveSnapshot, md: &MasterData, color_id: i32) {
    if snapshot.colors.contains_key(&color_id) {
        return;
    }
    if let Some(color) = md.resolve_color(color_id) {
        snapshot.colors.insert(
            color_id,
            [
                color.r as f32 / 255.0,
                color.g as f32 / 255.0,
                color.b as f32 / 255.0,
                color.a as f32 / 255.0,
            ],
        );
    }
}

fn insert_master_resource(
    snapshot: &mut ProfileResolveSnapshot,
    md: &MasterData,
    lookup_kind: &str,
    masterdata_kind: &str,
    id: i32,
    resources: ResolveResourceContext<'_>,
) {
    let lookup_key = resource_lookup_key(lookup_kind, id, "");
    if snapshot.resources.contains_key(&lookup_key) {
        return;
    }
    let key = md
        .resolve_resource(masterdata_kind, id)
        .map(|resource| format!("{}/{}", resource.load_val, resource.file_name))
        .unwrap_or_else(|| format!("{masterdata_kind}/{id}"));
    insert_resource_descriptor(
        snapshot,
        lookup_key,
        key,
        (100.0, 100.0),
        masterdata_kind,
        id,
        resources,
    );
}

#[allow(clippy::too_many_arguments)]
fn insert_resource_descriptor(
    snapshot: &mut ProfileResolveSnapshot,
    lookup_key: String,
    key: String,
    fallback_size: (f32, f32),
    table: &str,
    id: i32,
    resources: ResolveResourceContext<'_>,
) {
    if snapshot.resources.contains_key(&lookup_key) {
        return;
    }
    snapshot.resources.insert(
        lookup_key,
        make_resource_descriptor(key, fallback_size, table, id, resources),
    );
}

fn make_resource_descriptor(
    key: String,
    fallback_size: (f32, f32),
    table: &str,
    id: i32,
    resources: ResolveResourceContext<'_>,
) -> ResourceDescriptor {
    let natural_size = asset_size(resources, "assets", &key).unwrap_or(fallback_size);
    ResourceDescriptor {
        resource: ResourceKey {
            namespace: "assets".into(),
            key,
        },
        natural_width: natural_size.0,
        natural_height: natural_size.1,
        provenance: std::collections::BTreeMap::from([
            ("kind".into(), ParameterValue::Text("master_data".into())),
            ("table".into(), ParameterValue::Text(table.into())),
            ("id".into(), ParameterValue::I64(id.into())),
        ]),
    }
}

fn asset_size(
    resources: ResolveResourceContext<'_>,
    namespace: &str,
    key: &str,
) -> Option<(f32, f32)> {
    if let Some(metadata) = timed_resource_metadata(resources, namespace, key) {
        return Some((metadata.width as f32, metadata.height as f32));
    }
    #[cfg(feature = "skia-core")]
    if let Some(image) = resources.assets.and_then(|assets| assets.get_image(key)) {
        return Some((image.width() as f32, image.height() as f32));
    }
    None
}

fn resource_is_static(resources: ResolveResourceContext<'_>, key: &str) -> bool {
    if timed_resource_metadata(resources, "static", key).is_some() {
        return true;
    }
    resources
        .assets
        .is_some_and(|assets| assets.is_pinned_static(key))
}

fn resource_exists(resources: ResolveResourceContext<'_>, key: &str) -> bool {
    if timed_resource_metadata(resources, "static", key).is_some()
        || timed_resource_metadata(resources, "assets", key).is_some()
    {
        return true;
    }
    resources.assets.is_none_or(|assets| assets.contains(key))
}

fn timed_resource_metadata<'a>(
    resources: ResolveResourceContext<'a>,
    namespace: &str,
    key: &str,
) -> Option<crate::render_object::RenderObjectMetadata<'a>> {
    let store = resources.render_objects?;
    let started = std::time::Instant::now();
    let metadata = store.resource_metadata(namespace, key);
    if let Some(total) = resources.catalog_lookup_ns {
        total.set(
            total
                .get()
                .saturating_add(started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64),
        );
    }
    metadata
}

fn elapsed_ns(started: std::time::Instant) -> u64 {
    started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use allium_renderer_core::{FontRole, ParameterValue, SemanticCommandPayload};

    use super::*;
    use crate::masterdata::{MasterDataProvider, ResolvedColor, ResolvedHonor, ResourceInfo};
    use crate::types::{BondsHonorEntry, BondsHonorWordEntry, CardEntry, HonorEntry};

    struct EmptyProvider;

    fn assert_shared_semantic_parity(
        mut native: ResolvedProfileScene,
        mut shared: ResolvedProfileScene,
    ) {
        // Native server diagnostics retain table/id provenance. It is not a
        // rendering input; every rendering and interaction field stays gated.
        for scene in [&mut native, &mut shared] {
            for layer in &mut scene.layers {
                layer.resolved_parameters.remove("resource_source.table");
                layer.resolved_parameters.remove("resource_source.id");
            }
            for region in &mut scene.interaction_regions {
                region.resolved_data.remove("resource_source.table");
                region.resolved_data.remove("resource_source.id");
            }
        }
        assert_eq!(
            native, shared,
            "native semantic adapter drifted from shared core"
        );
    }

    impl MasterDataProvider for EmptyProvider {
        fn resolve_story_banner(&self, _: &str, _: i32) -> Option<String> {
            None
        }
        fn get_card(&self, id: i32) -> Option<CardEntry> {
            match id {
                9001 => Some(CardEntry {
                    id,
                    asset_bundle_name: "card_member_cropped".into(),
                    card_rarity_type: "rarity_4".into(),
                    attr: "cute".into(),
                    character_id: 1,
                }),
                9002 => Some(CardEntry {
                    id,
                    asset_bundle_name: "card_member_full".into(),
                    card_rarity_type: "rarity_3".into(),
                    attr: "cool".into(),
                    character_id: 2,
                }),
                _ => None,
            }
        }
        fn resolve_color(&self, _: i32) -> Option<ResolvedColor> {
            None
        }
        fn resolve_font(&self, _: i32) -> Option<String> {
            Some("FZLanTingHei-DB-GBK".into())
        }
        fn resolve_stamp(&self, _: i32) -> Option<String> {
            None
        }
        fn resolve_resource(&self, _: &str, _: i32) -> Option<ResourceInfo> {
            None
        }
        fn resolve_honor(&self, id: i32, level: i32) -> Option<ResolvedHonor> {
            (id == 4242).then(|| ResolvedHonor {
                asset_bundle_name: "live-master-fixture".into(),
                honor_rarity: "high".into(),
                honor_type: "achievement".into(),
                background_asset_bundle_name: None,
                frame_name: None,
                is_live_master: true,
                has_star: true,
                honor_level: level,
                honor_mission_type: Some("live_master".into()),
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
            1
        }
        fn color_count(&self) -> usize {
            0
        }
        fn resolve_localized_text(&self, key: &str) -> Option<String> {
            (key == "custom_profile.general.comment.title").then(|| "ひとこと".into())
        }
    }

    #[test]
    fn live_master_render_object_keeps_dynamic_overlay_semantics() {
        use sha2::Digest as _;

        use crate::render_object::{
            standard_honor_object_key, MappedRenderObjectStore, RenderObjectKind,
            RenderObjectStoreWriter, RenderObjectWrite,
        };

        let root = tempfile::tempdir().unwrap();
        let key = standard_honor_object_key(4242, 3, true);
        let pixels = vec![0u8; 380 * 80 * 4];
        let mut writer =
            RenderObjectStoreWriter::create(root.path(), "honor-fixture", 1024 * 1024).unwrap();
        writer
            .add(RenderObjectWrite {
                key: &key,
                kind: RenderObjectKind::StandardHonor,
                source_sha256: &hex::encode(sha2::Sha256::digest(b"honor-fixture")),
                width: 380,
                height: 80,
                row_bytes: 380 * 4,
                pixels: &pixels,
            })
            .unwrap();
        let store = MappedRenderObjectStore::open(writer.finish().unwrap()).unwrap();
        let md = MasterData::new(Arc::new(EmptyProvider));
        let visual = build_standard_honor_visual(
            "slot-1",
            4242,
            3,
            true,
            None,
            &md,
            ResolveResourceContext {
                assets: None,
                render_objects: Some(&store),
                catalog_lookup_ns: None,
            },
        )
        .unwrap();
        match visual.visual {
            HonorVisualKind::Standard {
                is_live_master,
                background,
                frame_candidates,
                overlay,
                live_star_on,
                live_star_off,
                ..
            } => {
                assert!(is_live_master);
                assert_eq!(
                    background.unwrap().resource,
                    ResourceKey {
                        namespace: "render-object".into(),
                        key,
                    }
                );
                assert!(frame_candidates.is_empty());
                assert!(overlay.is_none());
                assert!(live_star_on.is_none());
                assert!(live_star_off.is_none());
            }
            HonorVisualKind::Bonds { .. } => panic!("live-master changed semantic kind"),
        }

        let object = serde_json::json!({
            "layer": 7, "lock": false,
            "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
        });
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "generals": [{ "objectData": object, "type": 6 }]
        }))
        .unwrap();
        let profile = ProfileData {
            honor_slots: vec![crate::profile::HonorSlot {
                honor_id: 4242,
                honor_level: 3,
                full_size: true,
                profile_honor_type: "normal".into(),
                ..crate::profile::HonorSlot::default()
            }],
            user_honor_missions: std::collections::HashMap::from([("live_master".into(), 73)]),
            ..ProfileData::default()
        };
        let scene = resolve_card_commands_with_resources(
            &card,
            &md,
            "live-master-general",
            Some(&profile),
            "cn",
            ResolveResourceContext {
                render_objects: Some(&store),
                ..ResolveResourceContext::default()
            },
        )
        .unwrap();
        assert!(scene
            .commands
            .iter()
            .any(|command| command.role == "honor-4242-progress"));
        assert!(!scene
            .commands
            .iter()
            .any(|command| command.role.starts_with("honor-4242-live-star-")));
    }

    #[test]
    fn every_authored_element_remains_exactly_one_game_layer() {
        let object = serde_json::json!({
            "layer": 7, "lock": false,
            "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
        });
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "texts": [{ "objectData": object, "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 1, "outlineSize": 0.0, "size": 24.0, "text": "<b>A</b>", "type": 1 }],
            "shapes": [{ "objectData": object, "alpha": 1.0, "colorId": 1, "id": 1, "outlineAlpha": 1.0, "outlineColorId": 1, "outlineSize": 0.0 }],
            "cardMembers": [{ "objectData": object, "id": 1 }], "stamps": [{ "objectData": object, "id": 1 }], "others": [{ "objectData": object, "id": 1 }],
            "bondsHonors": [{ "objectData": object, "id": 1, "wordId": 1, "fullSize": true, "inverse": false, "useUnitVirtualSinger": false }],
            "honors": [{ "objectData": object, "id": 1, "fullSize": true }], "collections": [{ "objectData": object, "id": 1 }],
            "generals": [{ "objectData": object, "type": 4 }], "standMembers": [{ "objectData": object, "id": 1 }],
            "generalBackgrounds": [{ "objectData": object, "id": 1 }], "storyBackgrounds": [{ "objectData": object, "id": 1 }]
        })).unwrap();
        let md = MasterData::new(Arc::new(EmptyProvider));
        let resolved = resolve_card_commands(&card, &md, "fixture").unwrap();
        let shared = allium_renderer_core::profile_resolve::compile_profile_scene(
            &card,
            None,
            &md,
            "fixture",
            "und",
            &(),
            std::collections::BTreeMap::new(),
        )
        .unwrap();
        assert_shared_semantic_parity(resolved.clone(), shared);
        assert_eq!(
            (
                resolved.layers.len(),
                resolved.commands.len(),
                resolved.interaction_regions.len()
            ),
            (12, 12, 12)
        );
        assert_eq!(
            resolved
                .layers
                .iter()
                .map(|layer| layer.authored_kind)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            12
        );
        assert!(resolved.layers.iter().all(|layer| layer.game_layer == 7));
    }

    #[test]
    fn authored_card_member_overlay_matches_shared_true_false_contract() {
        let object = |layer| {
            serde_json::json!({
                "layer": layer, "lock": false,
                "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
            })
        };
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "cardMembers": [
                {
                    "objectData": object(1), "id": 9001, "type": 1,
                    "showMasterRank": true
                },
                {
                    "objectData": object(2), "id": 9002, "type": 2,
                    "showMasterRank": true, "useAfterSpecialTraining": false
                },
                {
                    "objectData": object(3), "id": 9001, "type": 1,
                    "showMasterRank": false
                }
            ]
        }))
        .unwrap();
        let profile = ProfileData {
            user_cards: std::collections::HashMap::from([
                (
                    9001,
                    crate::profile::UserCardInfo {
                        after_training: true,
                        master_rank: 4,
                        level: 37,
                    },
                ),
                (
                    9002,
                    crate::profile::UserCardInfo {
                        after_training: true,
                        master_rank: 2,
                        level: 28,
                    },
                ),
            ]),
            ..ProfileData::default()
        };
        let md = MasterData::new(Arc::new(EmptyProvider));
        let resolved = resolve_card_commands_with_profile(
            &card,
            &md,
            "card-member-overlay",
            Some(&profile),
            "cn",
            None,
        )
        .unwrap();
        let shared = allium_renderer_core::profile_resolve::compile_profile_scene(
            &card,
            Some(&profile.to_core_profile()),
            &md,
            "card-member-overlay",
            "cn",
            &(),
            std::collections::BTreeMap::new(),
        )
        .unwrap();
        assert_shared_semantic_parity(resolved.clone(), shared);

        let commands_for = |index| {
            let layer = resolved
                .layers
                .iter()
                .find(|layer| layer.authored_index == index)
                .unwrap();
            resolved
                .commands
                .iter()
                .filter(|command| command.layer_id == layer.id)
                .collect::<Vec<_>>()
        };
        let cropped = commands_for(0);
        assert_eq!(cropped.len(), 10);
        assert!(matches!(
            &cropped[2].payload,
            SemanticCommandPayload::Text {
                source: allium_renderer_core::TextSource::ProfileField { field, value },
                font_role: FontRole::RegionFontId(1),
                ..
            } if field == "userCards.9001.level" && value == "Lv.37"
        ));
        assert!(matches!(
            &cropped[5].payload,
            SemanticCommandPayload::Image { resource, .. }
                if resource.key == "card/rarity_star_afterTraining"
        ));
        assert!(matches!(
            &cropped[9].payload,
            SemanticCommandPayload::Image { resource, .. }
                if resource.key == "card/masterRank_S_4"
        ));

        let full = commands_for(1);
        assert_eq!(full.len(), 7);
        assert!(matches!(
            &full[3].payload,
            SemanticCommandPayload::Image { resource, .. }
                if resource.key == "card/rarity_star_normal"
        ));
        assert!(matches!(
            &full[6].payload,
            SemanticCommandPayload::Image { resource, .. }
                if resource.key == "card/masterRank_L_2"
        ));

        let image_only = commands_for(2);
        assert_eq!(image_only.len(), 1);
        assert_eq!(image_only[0].role, "card-member");
    }

    #[test]
    fn player_name_and_signature_lower_inside_original_general_layers() {
        let object = serde_json::json!({
            "layer": 9, "lock": false,
            "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
        });
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "generals": [{ "objectData": object, "type": 13 }, { "objectData": object, "type": 4 }]
        }))
        .unwrap();
        let profile = ProfileData {
            user_name: "<b>Player</b>".into(),
            word: "<color=#ff0000>Bio</color>".into(),
            ..ProfileData::default()
        };
        let md = MasterData::new(Arc::new(EmptyProvider));
        let core_profile = profile.to_core_profile();
        let resolved = resolve_card_commands_with_profile(
            &card,
            &md,
            "identity-fixture",
            Some(&profile),
            "ja-JP",
            None,
        )
        .unwrap();
        let shared = allium_renderer_core::profile_resolve::compile_profile_scene(
            &card,
            Some(&core_profile),
            &md,
            "identity-fixture",
            "ja-JP",
            &(),
            std::collections::BTreeMap::new(),
        )
        .unwrap();
        assert_shared_semantic_parity(resolved.clone(), shared);
        assert_eq!(
            (
                resolved.layers.len(),
                resolved.commands.len(),
                resolved.interaction_regions.len()
            ),
            (2, 5, 4)
        );
        let text_payloads = resolved
            .commands
            .iter()
            .filter_map(|command| match &command.payload {
                SemanticCommandPayload::Text {
                    source, font_role, ..
                } => Some((source, font_role)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(text_payloads.len(), 3);
        assert!(text_payloads
            .iter()
            .all(|(_, role)| **role == FontRole::RegionFontId(1)));
        assert!(text_payloads.iter().any(|(source, _)| matches!(source, allium_renderer_core::TextSource::Localized { key, locale, .. } if key == "custom_profile.general.comment.title" && locale == "ja-JP")));
        assert_eq!(
            resolved
                .interaction_regions
                .iter()
                .filter(
                    |region| region.capabilities == ["inspect", "select_text", "edit_text"]
                        && matches!(
                            region.resolved_data.get("field"),
                            Some(ParameterValue::Text(_))
                        )
                )
                .count(),
            2
        );
        assert_eq!(
            resolved
                .interaction_regions
                .iter()
                .filter(|region| region.capabilities == ["inspect", "select_layer"])
                .count(),
            2
        );
    }

    #[test]
    fn page_without_generals_matches_full_profile_base_semantics() {
        let object = serde_json::json!({
            "layer": 3, "lock": false,
            "position": { "x": 12.0, "y": -7.0, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
        });
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "texts": [{
                "objectData": object, "colorId": 1, "fontId": 1, "lineSpacing": 0.0,
                "outlineColorId": 1, "outlineSize": 0.0, "size": 24.0,
                "text": "<line-indent=90.1%>No general component", "type": 513
            }]
        }))
        .unwrap();
        let profile = ProfileData {
            user_name: "Unused player component".into(),
            word: "Unused signature".into(),
            ..ProfileData::default()
        };
        let md = MasterData::new(Arc::new(EmptyProvider));
        let optimized = resolve_card_commands_with_resources(
            &card,
            &md,
            "page-only-fixture",
            Some(&profile),
            "ja-JP",
            ResolveResourceContext::default(),
        )
        .unwrap();
        let mut full_base = build_profile_resolve_base_snapshot(
            &card,
            &md,
            Some(&profile),
            "ja-JP",
            ResolveResourceContext::default(),
        );
        let full = resolve_card_commands_with_base(
            &mut full_base,
            &card,
            &md,
            "page-only-fixture",
            Some(&profile),
            ResolveResourceContext::default(),
        )
        .unwrap();
        assert_eq!(optimized, full);
        assert!(optimized.layers.iter().all(|layer| layer.game_layer == 3));

        let (full_scene, _) = crate::core_shadow::build_scene_with_resolved(
            &card,
            &md,
            "page-only-fixture",
            "cn",
            Some(&profile),
            "ja-JP",
            None,
        )
        .unwrap();
        let motion_scene =
            crate::core_shadow::build_text_scene(&card, &md, "page-only-fixture").unwrap();
        let full_preflight = full_scene.animation_preflight(120).unwrap();
        let motion_preflight = motion_scene.animation_preflight(120).unwrap();
        assert_eq!(full_preflight, motion_preflight);
        let authored = allium_renderer_core::profile_scene::ordered_profile_elements(
            &card,
            "page-only-fixture",
        );
        assert!(motion_preflight
            .observable_layer_ids
            .iter()
            .all(|layer_id| authored.iter().any(|element| element.layer_id == *layer_id)));
    }

    #[test]
    fn optional_resource_descriptors_follow_the_available_asset_snapshot() {
        let empty_assets = crate::assets::AssetStore::new(1);
        assert!(optional_descriptor(
            "honor/missing/rank_sub".into(),
            (180.0, 80.0),
            "honor_overlay",
            1,
            ResolveResourceContext {
                assets: Some(&empty_assets),
                ..ResolveResourceContext::default()
            },
        )
        .is_none());
        assert!(optional_descriptor(
            "honor/deferred/rank_sub".into(),
            (180.0, 80.0),
            "honor_overlay",
            1,
            ResolveResourceContext::default(),
        )
        .is_some());
    }
}
