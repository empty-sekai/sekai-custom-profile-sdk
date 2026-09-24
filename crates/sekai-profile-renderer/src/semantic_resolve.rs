//! Native resolve-snapshot adapter for the shared renderer v0.2 profile resolver.

use sekai_profile_renderer_core::profile_resolve::{authored_resource, AuthoredResource};
use sekai_profile_renderer_core::profile_scene::{
    card_member_lookup_key, ordered_profile_elements, resolve_profile_scene, CardVisualSnapshot,
    CharacterRankSnapshot, ComponentImageSnapshot, HonorVisualKind, HonorVisualSnapshot,
    MusicDifficultySnapshot, MusicResultsSnapshot, ProfileComponentSnapshot, ProfileElementRef,
    ProfileResolveError, ProfileResolveSnapshot, ResolvedProfileScene, ResourceDescriptor,
    StoryFavoriteSnapshot,
};
use sekai_profile_renderer_core::{ParameterValue, ResourceKey};

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
    let reads_component = !card.generals.is_empty()
        || card
            .card_members
            .iter()
            .any(|member| member.show_master_rank.unwrap_or(false));
    let mut base = if !reads_component {
        // Shared core only reads ProfileResolveSnapshot::component while lowering General
        // elements and the localized level of card-member overlays. Avoid building the
        // player-wide component graph for other pages.
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
        for element in ordered_profile_elements(card, document_key) {
            // The viewer does not build hidden elements.
            if !element.object().visible {
                continue;
            }
            match element.value {
                ProfileElementRef::Text(text) => {
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        snapshot.fonts.entry(text.font_id)
                    {
                        if let Some(font) = md.resolve_font_or_default(text.font_id) {
                            entry.insert(font);
                        }
                    }
                    insert_color(snapshot, md, text.color_id);
                    insert_color(snapshot, md, text.outline_color_id);
                }
                ProfileElementRef::Shape(shape) => {
                    insert_color(snapshot, md, shape.color_id);
                    insert_color(snapshot, md, shape.outline_color_id);
                }
                ProfileElementRef::UserInterfaceIcon(icon) => {
                    insert_color(snapshot, md, icon.color_id);
                }
                _ => {}
            }
            match authored_resource(element.value, md) {
                AuthoredResource::None => {}
                AuthoredResource::MissingRow => {
                    snapshot.omitted_elements.insert(element.source_key);
                }
                AuthoredResource::Request(request) => {
                    let (table, id) = resource_provenance(element.value);
                    insert_resource_descriptor(
                        snapshot,
                        request.lookup_key,
                        request.resource.key,
                        (request.fallback.width, request.fallback.height),
                        table,
                        id,
                        assets,
                    );
                }
            }
        }
        for element in ordered_profile_elements(card, document_key) {
            if !element.object().visible || snapshot.omitted_elements.contains(&element.source_key)
            {
                continue;
            }
            match element.value {
                ProfileElementRef::Text(text) => {
                    if let Some(mut program) = crate::text::line_indent_program(text, md) {
                        let (_, _, rotation_deg, scale_x, _) =
                            sekai_profile_renderer_core::profile_transform::extract_transform(
                                element.object(),
                            );
                        program.rotation_deg = rotation_deg;
                        program.scale_x = scale_x;
                        snapshot.line_indent.insert(element.source_key, program);
                    }
                }
                ProfileElementRef::Honor(honor) => {
                    let Some(level) = sekai_profile_renderer_core::profile_data::placed_honor_level(
                        profile.map(|profile| &profile.owned_honors),
                        honor.id,
                        honor.honor_level,
                    ) else {
                        continue;
                    };
                    if let Some(visual) = build_standard_honor_visual(
                        "customProfile.honors",
                        honor.id,
                        level,
                        honor.full_size,
                        profile,
                        md,
                        assets,
                    ) {
                        snapshot.honor_visuals.insert(element.source_key, visual);
                    }
                }
                ProfileElementRef::BondsHonor(honor) => {
                    let Some(level) =
                        sekai_profile_renderer_core::profile_data::placed_bonds_honor_level(
                            profile.map(|profile| &profile.owned_honors),
                            honor.id,
                            honor.honor_level,
                        )
                    else {
                        continue;
                    };
                    if let Some(visual) = build_bonds_honor_visual(
                        "customProfile.bondsHonors",
                        honor.id,
                        level,
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
                        let lookup_key = card_member_lookup_key(value);
                        let user_card = profile.and_then(|profile| profile.user_card(value.id));
                        snapshot.card_member_visuals.insert(
                            element.source_key,
                            CardVisualSnapshot {
                                card_id: value.id,
                                after_training: user_card.map_or(
                                    value.use_after_special_training.unwrap_or(false),
                                    |card| card.special_training_done,
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
            let story_favorites = sekai_profile_renderer_core::profile_data::story_favorite_slots(
                &profile.story_favorites,
                |favorite| favorite.share_no,
            )
            .into_iter()
            .map(|favorite| match favorite {
                Some(favorite) => StoryFavoriteSnapshot {
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
                },
                None => StoryFavoriteSnapshot {
                    story_id: 0,
                    story_type: String::new(),
                    image: ComponentImageSnapshot {
                        source_field: "userProfile.storyFavorites".into(),
                        source_id: String::new(),
                        descriptor: None,
                    },
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
                        after_training: member.special_training_done,
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
                    // Same fallback as the card-member snapshot: a leader card
                    // absent from userCards is stale data, rendered at the
                    // default level rather than level 0.
                    level: profile
                        .user_cards
                        .get(&leader.card_id)
                        .map(|card| card.level)
                        .unwrap_or(60),
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
                localized_text: sekai_profile_renderer_core::locale::GENERAL_LOCALIZATION_KEYS
                    .iter()
                    .filter_map(|key| {
                        md.resolve_localized_text(key)
                            .or_else(|| sekai_profile_renderer_core::locale::resolve(locale, key))
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
    let (w, h) = if full_size {
        (380.0, 80.0)
    } else {
        (180.0, 80.0)
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
    let plan = resolved.asset_plan(full_size);
    let background = optional_descriptor(
        plan.background.key.clone(),
        (w, h),
        "honor_background",
        honor_id,
        resources,
    );
    let frame_key = plan.frame_candidates[0]
        .as_ref()
        .filter(|key| asset_size(resources, &key.namespace, &key.key).is_some())
        .or(plan.frame_candidates[1].as_ref());
    let frame = frame_key.and_then(|key| {
        optional_descriptor(key.key.clone(), (w, h), "honor_frame", honor_id, resources)
    });
    let overlay = plan.overlay.as_ref().and_then(|key| {
        optional_descriptor(
            key.key.clone(),
            (w, h),
            "honor_overlay",
            honor_id,
            resources,
        )
    });
    Some(HonorVisualSnapshot {
        source_field: source_field.into(),
        source_id: honor_id.to_string(),
        honor_id,
        honor_level,
        full_size,
        visual: HonorVisualKind::Standard {
            has_star: resolved.draws_level_stars(),
            honor_type: resolved.honor_type,
            is_live_master: resolved.is_live_master,
            progress,
            background,
            frame_candidates: vec![frame],
            overlay,
            star: plan.star.as_ref().and_then(|key| {
                optional_descriptor(
                    key.key.clone(),
                    (16.0, 16.0),
                    "honor_static",
                    honor_id,
                    resources,
                )
            }),
            star_high: plan.star_high.as_ref().and_then(|key| {
                optional_descriptor(
                    key.key.clone(),
                    (16.0, 16.0),
                    "honor_static",
                    honor_id,
                    resources,
                )
            }),
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
    md.get_bonds_honor(honor_id)?;
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
    let plan = sekai_profile_renderer_core::masterdata::bonds_honor_asset_plan(
        md,
        honor_id,
        full_size,
        word_id,
        inverse,
        use_unit_virtual_singer,
    )?;
    let (w, h) = if full_size {
        (380.0, 80.0)
    } else {
        (180.0, 80.0)
    };
    let descriptor = |key: &ResourceKey, size: (f32, f32), table: &str, id: i32| {
        static_descriptor(key.key.clone(), size, table, id, resources)
    };
    Some(HonorVisualSnapshot {
        source_field: source_field.into(),
        source_id: honor_id.to_string(),
        honor_id,
        honor_level,
        full_size,
        visual: HonorVisualKind::Bonds {
            character_ids: plan.character_ids,
            backgrounds: plan
                .backgrounds
                .each_ref()
                .map(|key| Some(descriptor(key, (w, h), "bonds_honor_background", honor_id))),
            characters: plan.characters.each_ref().map(|key| {
                Some(descriptor(
                    key,
                    (160.0, 160.0),
                    "bonds_honor_character",
                    honor_id,
                ))
            }),
            mask: Some(descriptor(&plan.mask, (w, h), "honor_static", honor_id)),
            frame: Some(descriptor(&plan.frame, (w, h), "honor_frame", honor_id)),
            word: plan
                .word
                .as_ref()
                .map(|key| descriptor(key, (180.0, 40.0), "bonds_honor_word", word_id as i32)),
            star: Some(descriptor(
                &plan.star,
                (16.0, 16.0),
                "honor_static",
                honor_id,
            )),
            star_high: Some(descriptor(
                &plan.star_high,
                (16.0, 16.0),
                "honor_static",
                honor_id,
            )),
        },
    })
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

/// Provenance table and id recorded on an authored element's resource.
fn resource_provenance(element: ProfileElementRef<'_>) -> (&'static str, i32) {
    match element {
        ProfileElementRef::Shape(value) => ("customProfileResource", value.id),
        ProfileElementRef::CardMember(value) => ("cards", value.id),
        ProfileElementRef::Stamp(value) => ("stamps", value.id),
        ProfileElementRef::Other(value) => ("etc", value.id),
        ProfileElementRef::Collection(value) => ("collection", value.id),
        ProfileElementRef::StandMember(value) => ("standing", value.id),
        ProfileElementRef::GeneralBackground(value) => ("general_bg", value.id),
        ProfileElementRef::StoryBackground(value) => ("story_bg", value.id),
        ProfileElementRef::CharacterIcon(value) => ("character_icon", value.id),
        ProfileElementRef::Material(value) => ("material", value.id),
        ProfileElementRef::UserInterfaceIcon(value) => ("user_interface_icon", value.id),
        ProfileElementRef::Text(_)
        | ProfileElementRef::Honor(_)
        | ProfileElementRef::BondsHonor(_)
        | ProfileElementRef::General(_) => ("", 0),
    }
}

fn insert_color(snapshot: &mut ProfileResolveSnapshot, md: &MasterData, color_id: i32) {
    if snapshot.colors.contains_key(&color_id) {
        return;
    }
    if let Some(color) = md.resolve_color_or_default(color_id) {
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
    let (width, height) = resources.assets?.image_size(key)?;
    Some((width as f32, height as f32))
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

    use sekai_profile_renderer_core::{FontRole, ParameterValue, SemanticCommandPayload};

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

    /// Master data holding one row in each table these tests read: font 1,
    /// colour 3, which is also the first colour row, shape 1, which loads
    /// [`SHAPE_KEY`], and `customProfileEtcResources` 1, which loads
    /// [`OTHER_IMAGE_KEY`].
    struct SparseRowsProvider;

    const OTHER_IMAGE_KEY: &str = "custom_profile/etc/etc_001";
    const SHAPE_KEY: &str = "custom_profile/shape_v2/shape_001";
    const FIRST_COLOR: ResolvedColor = ResolvedColor {
        r: 12,
        g: 34,
        b: 56,
        a: 255,
    };

    impl MasterDataProvider for SparseRowsProvider {
        fn resolve_story_banner(&self, story_type: &str, story_id: i32) -> Option<String> {
            EmptyProvider.resolve_story_banner(story_type, story_id)
        }
        fn get_card(&self, id: i32) -> Option<CardEntry> {
            EmptyProvider.get_card(id)
        }
        fn resolve_color(&self, id: i32) -> Option<ResolvedColor> {
            (id == 3).then_some(FIRST_COLOR)
        }
        fn default_color(&self) -> Option<ResolvedColor> {
            Some(FIRST_COLOR)
        }
        fn resolve_font(&self, id: i32) -> Option<String> {
            (id == 1).then(|| "FZLanTingHei-DB-GBK".into())
        }
        fn resolve_stamp(&self, id: i32) -> Option<String> {
            EmptyProvider.resolve_stamp(id)
        }
        fn resolve_resource(&self, res_type: &str, id: i32) -> Option<ResourceInfo> {
            let (file_name, load_val) = match (res_type, id) {
                ("etc", 1) => ("etc_001", "custom_profile/etc"),
                ("shape", 1) => ("shape_001", "custom_profile/shape_v2"),
                _ => return None,
            };
            Some(ResourceInfo {
                file_name: file_name.into(),
                load_val: load_val.into(),
                resource_type: res_type.into(),
            })
        }
        fn resolve_honor(&self, id: i32, level: i32) -> Option<ResolvedHonor> {
            EmptyProvider.resolve_honor(id, level)
        }
        fn get_bonds_honor(&self, id: i32) -> Option<BondsHonorEntry> {
            EmptyProvider.get_bonds_honor(id)
        }
        fn get_bonds_honor_word(&self, id: i64) -> Option<BondsHonorWordEntry> {
            EmptyProvider.get_bonds_honor_word(id)
        }
        fn get_honor(&self, id: i32) -> Option<HonorEntry> {
            EmptyProvider.get_honor(id)
        }
        fn resolve_unit_vs_sd(&self, self_id: i32, partner_id: i32) -> i32 {
            EmptyProvider.resolve_unit_vs_sd(self_id, partner_id)
        }
        fn font_count(&self) -> usize {
            1
        }
        fn color_count(&self) -> usize {
            1
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
    fn placed_honors_follow_the_profile_honor_lists() {
        let object = serde_json::json!({
            "layer": 7, "lock": false,
            "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
            "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
        });
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "honors": [{ "objectData": object, "id": 4242, "fullSize": true }]
        }))
        .expect("card fixture");
        let md = MasterData::new(Arc::new(EmptyProvider));
        let levels = |profile: Option<&ProfileData>| {
            build_resolve_snapshot_parts(
                &card,
                &md,
                "placed-honors",
                profile,
                "cn",
                ResolveResourceContext::default(),
                true,
                false,
            )
            .honor_visuals
            .values()
            .map(|visual| visual.honor_level)
            .collect::<Vec<_>>()
        };
        assert_eq!(levels(None), vec![1]);
        let owned = ProfileData::from_json(&serde_json::json!({ "userHonors": [[4242, 3, null]] }));
        assert_eq!(levels(Some(&owned)), vec![3]);
        let unowned = ProfileData::from_json(&serde_json::json!({ "userHonors": [[7, 3, null]] }));
        assert_eq!(levels(Some(&unowned)), Vec::<i32>::new());
        let scene = resolve_card_commands_with_profile(
            &card,
            &md,
            "placed-honors",
            Some(&unowned),
            "cn",
            None,
        )
        .expect("scene");
        assert!(!scene
            .commands
            .iter()
            .any(|command| command.role.starts_with("honor-4242-")));
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
        let shared = sekai_profile_renderer_core::profile_resolve::compile_profile_scene(
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
    fn hidden_elements_and_missing_master_rows_stay_empty_like_the_shared_core() {
        let object = |layer, visible| {
            serde_json::json!({
                "layer": layer, "lock": false,
                "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": visible
            })
        };
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "shapes": [{
                "objectData": object(1, true), "alpha": 1.0, "colorId": 1, "id": 5,
                "outlineAlpha": 0.0, "outlineColorId": 1, "outlineSize": 0.0
            }],
            "stamps": [
                { "objectData": object(2, true), "id": 3 },
                { "objectData": object(3, false), "id": 3 }
            ],
            "cardMembers": [{ "objectData": object(4, true), "id": 7, "type": 2 }]
        }))
        .expect("card fixture");
        let md = MasterData::new(Arc::new(EmptyProvider));
        let resolved = resolve_card_commands(&card, &md, "omitted").expect("native scene");
        let shared = sekai_profile_renderer_core::profile_resolve::compile_profile_scene(
            &card,
            None,
            &md,
            "omitted",
            "und",
            &(),
            std::collections::BTreeMap::new(),
        )
        .expect("shared scene");
        assert_shared_semantic_parity(resolved.clone(), shared);
        assert_eq!(resolved.layers.len(), 4);
        assert!(resolved
            .commands
            .iter()
            .all(|command| matches!(command.payload, SemanticCommandPayload::Composite { .. })));
        assert!(crate::asset_keys::collect_card_asset_keys(&card, &md).is_empty());
    }

    #[test]
    fn hidden_honors_are_neither_drawn_nor_requested() {
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "honors": [{
                "objectData": {
                    "layer": 1, "lock": false,
                    "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                    "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": false
                },
                "id": 4242, "fullSize": true, "honorLevel": 3
            }]
        }))
        .expect("card fixture");
        let md = MasterData::new(Arc::new(EmptyProvider));
        let resolved = resolve_card_commands(&card, &md, "hidden-honor").expect("native scene");
        let shared = sekai_profile_renderer_core::profile_resolve::compile_profile_scene(
            &card,
            None,
            &md,
            "hidden-honor",
            "und",
            &(),
            std::collections::BTreeMap::new(),
        )
        .expect("shared scene");
        assert_shared_semantic_parity(resolved.clone(), shared);
        assert!(resolved
            .commands
            .iter()
            .all(|command| matches!(command.payload, SemanticCommandPayload::Composite { .. })));
        assert!(crate::asset_keys::collect_card_asset_keys(&card, &md).is_empty());
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
                        special_training_done: false,
                        master_rank: 4,
                        level: 37,
                    },
                ),
                (
                    9002,
                    crate::profile::UserCardInfo {
                        after_training: false,
                        special_training_done: true,
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
        let shared = sekai_profile_renderer_core::profile_resolve::compile_profile_scene(
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
                source: sekai_profile_renderer_core::TextSource::Localized { key, value, .. },
                font_role: FontRole::RegionFontId(1),
                ..
            } if key == "custom_profile.general.card_level" && value == "Lv.37"
        ));
        assert!(matches!(
            &cropped[5].payload,
            SemanticCommandPayload::Image { resource, .. }
                if resource.key == "card/rarity_star_normal"
        ));
        assert!(matches!(
            &cropped[9].payload,
            SemanticCommandPayload::Image { resource, .. }
                if resource.key == "card/masterRank_L_4"
        ));

        let full = commands_for(1);
        assert_eq!(full.len(), 7);
        assert!(matches!(
            &full[3].payload,
            SemanticCommandPayload::Image { resource, .. }
                if resource.key == "card/rarity_star_afterTraining"
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
        let shared = sekai_profile_renderer_core::profile_resolve::compile_profile_scene(
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
        assert!(text_payloads.iter().any(|(source, _)| matches!(source, sekai_profile_renderer_core::TextSource::Localized { key, locale, .. } if key == "custom_profile.general.comment.title" && locale == "ja-JP")));
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
        if crate::sdf::outline::load_font_bytes_for_family(crate::widgets::theme::fonts::PRIMARY)
            .is_none()
        {
            eprintln!("skipping: FONT_DIR does not provide the test family");
            return;
        }

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
        assert!(full_preflight.observable_program_count > 0);
        assert_eq!(full_preflight, motion_preflight);
        let authored = sekai_profile_renderer_core::profile_scene::ordered_profile_elements(
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

    #[test]
    fn text_scene_layers_keep_layer_local_geometry() {
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "texts": [{
                "objectData": {
                    "layer": 1, "lock": false,
                    "position": { "x": 100.0, "y": 50.0, "z": 0.0 },
                    "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
                },
                "colorId": 1, "fontId": 1, "lineSpacing": 0.0, "outlineColorId": 1,
                "outlineSize": 0.0, "size": 24.0, "text": "A", "type": 1
            }]
        }))
        .expect("text card");
        let md = MasterData::new(Arc::new(EmptyProvider));
        let dump = crate::core_shadow::build_text_scene(&card, &md, "fixture")
            .expect("text scene")
            .dump();
        let layer = &dump.layers[0];
        assert_eq!(layer.bounds, sekai_profile_renderer_core::Rect::default());
        assert_eq!(layer.quad, [[0.0; 2]; 4]);
        assert_eq!(layer.hit_geometry, [[0.0; 2]; 4]);
        assert_eq!((layer.matrix[4], layer.matrix[5]), (1015.0, 356.0));
    }

    #[test]
    fn text_scene_layers_use_the_default_font_and_color_rows_for_unknown_ids() {
        let text = |layer: i32, font_id: i32, color_id: i32, outline_color_id: i32| {
            serde_json::json!({
                "objectData": {
                    "layer": layer, "lock": false,
                    "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                    "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
                },
                "colorId": color_id, "fontId": font_id, "lineSpacing": 0.0,
                "outlineColorId": outline_color_id, "outlineSize": 0.2, "size": 24.0,
                "text": "A", "type": 1
            })
        };
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "texts": [text(1, 1, 3, 3), text(2, 404, 405, 406)]
        }))
        .expect("text card");
        let md = MasterData::new(Arc::new(SparseRowsProvider));
        let dump = crate::core_shadow::build_text_scene(&card, &md, "fallback-rows")
            .expect("text scene")
            .dump();
        let drawn = |index: usize| {
            let parameters = &dump.layers[index].resolved_parameters;
            ["font_family", "color", "outline_color"].map(|name| parameters.get(name).cloned())
        };
        assert!(drawn(0).iter().all(Option::is_some), "{:?}", drawn(0));
        assert_eq!(drawn(1), drawn(0));
    }

    #[test]
    fn image_elements_request_the_keys_of_their_master_rows() {
        let object = |layer: i32| {
            serde_json::json!({
                "layer": layer, "lock": false,
                "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
            })
        };
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "shapes": [{
                "objectData": object(1), "alpha": 1.0, "colorId": 3, "id": 1,
                "outlineAlpha": 0.0, "outlineColorId": 3, "outlineSize": 0.0
            }],
            "others": [{ "objectData": object(2), "id": 1 }]
        }))
        .expect("image card");
        let md = MasterData::new(Arc::new(SparseRowsProvider));
        let mut keys = crate::asset_keys::collect_card_asset_keys(&card, &md);
        keys.sort();
        assert_eq!(keys, [OTHER_IMAGE_KEY, SHAPE_KEY]);
    }

    #[test]
    fn source_images_draw_at_their_pixel_size_without_a_baked_object() {
        let card: CustomProfileCard = serde_json::from_value(serde_json::json!({
            "others": [{
                "objectData": {
                    "layer": 1, "lock": false,
                    "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
                    "rotation": { "w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }, "visible": true
                },
                "id": 1
            }]
        }))
        .expect("image card");
        let md = MasterData::new(Arc::new(SparseRowsProvider));
        let (width, height) = (64u32, 36u32);
        let assets = crate::assets::AssetStore::new(8);
        assets.put(
            OTHER_IMAGE_KEY.into(),
            crate::codec::png::encode_rgba(width, height, &[255, 0, 0, 255].repeat(64 * 36))
                .expect("png"),
        );

        let resolved =
            resolve_card_commands_with_profile(&card, &md, "fixture", None, "und", Some(&assets))
                .expect("resolved scene");
        let layer = &resolved.layers[0];
        assert_eq!(
            (layer.bounds.width, layer.bounds.height),
            (width as f32, height as f32)
        );

        // The store holds an unrelated object only, so the image falls back to
        // the asset store's source bytes.
        let temp = tempfile::tempdir().expect("tempdir");
        let mut writer = crate::render_object::RenderObjectStoreWriter::create(
            temp.path().join("store"),
            "fixture",
            4096,
        )
        .expect("store writer");
        writer
            .add(crate::render_object::RenderObjectWrite {
                key: "unrelated",
                kind: crate::render_object::RenderObjectKind::Texture,
                source_sha256: &"0".repeat(64),
                width: 1,
                height: 1,
                row_bytes: 4,
                pixels: &[0; 4],
            })
            .expect("store object");
        let manifest = writer.finish().expect("store manifest");
        let store = crate::render_object::MappedRenderObjectStore::open(manifest).expect("store");
        let (canvas_width, canvas_height) = (
            sekai_profile_renderer_core::profile_transform::CANVAS_WIDTH as u32,
            sekai_profile_renderer_core::profile_transform::CANVAS_HEIGHT as u32,
        );
        let mut pixels = vec![0u8; canvas_width as usize * canvas_height as usize * 4];
        let stats = crate::profile_compositor::render_authored_profile_into_scalar(
            &resolved,
            &store,
            None,
            &md,
            Some(&assets),
            sekai_profile_renderer_core::AuthoredElementKind::Other,
            0,
            &mut pixels,
            canvas_width,
            canvas_height,
        )
        .expect("composited element");
        assert_eq!(stats.source_fallback_object_count, 1);

        let painted = pixels
            .iter()
            .skip(3)
            .step_by(4)
            .enumerate()
            .filter(|(_, alpha)| **alpha > 0)
            .map(|(index, _)| (index as u32 % canvas_width, index as u32 / canvas_width))
            .collect::<Vec<_>>();
        assert_eq!(painted.len(), (width * height) as usize);
        let (min_x, min_y, max_x, max_y) = painted.iter().fold(
            (u32::MAX, u32::MAX, 0, 0),
            |(min_x, min_y, max_x, max_y), &(x, y)| {
                (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
            },
        );
        assert_eq!((max_x - min_x + 1, max_y - min_y + 1), (width, height));
    }
}
