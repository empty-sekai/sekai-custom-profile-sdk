//! Native resolve-snapshot adapter for the shared renderer v0.2 profile resolver.

use sekai_profile_core::omikuji::OmikujiClient;
use sekai_profile_core::profile_resolve::{authored_resource, AuthoredResource};
use sekai_profile_core::profile_scene::{
    card_member_lookup_key, ordered_profile_elements, resolve_profile_scene, CardVisualSnapshot,
    CharacterRankSnapshot, ComponentImageSnapshot, HonorVisualKind, HonorVisualSnapshot,
    MusicDifficultySnapshot, MusicResultsSnapshot, ProfileComponentSnapshot, ProfileElementRef,
    ProfileResolveError, ProfileResolveSnapshot, ResolvedProfileScene, ResourceDescriptor,
    StoryFavoriteSnapshot,
};
use sekai_profile_core::{ParameterValue, ResourceKey};

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
    /// Locale the base was built for; page overlays resolve with it too.
    locale: String,
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
            locale: locale.into(),
        }
    } else {
        build_profile_resolve_base_snapshot(md, profile, locale, resources)
    };
    resolve_card_commands_with_base(&mut base, card, md, document_key, profile, resources)
}

pub fn build_profile_resolve_base_snapshot(
    md: &MasterData,
    profile: Option<&ProfileData>,
    locale: &str,
    resources: ResolveResourceContext<'_>,
) -> ProfileResolveBaseSnapshot {
    let mut snapshot = ProfileResolveSnapshot::default();
    populate_profile_base(
        &mut snapshot,
        SnapshotSources {
            md,
            profile,
            locale,
            resources,
        },
    );
    // Font 1 is copied into component.region_fonts while building the base.
    // Keeping it in this top-level map would make page overlay replacement
    // ambiguous, so only the immutable component remains in the base.
    snapshot.fonts.clear();
    ProfileResolveBaseSnapshot {
        snapshot,
        locale: locale.into(),
    }
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
    debug_assert!(base.snapshot.omikuji_visuals.is_empty());
    populate_page_overlay(
        &mut base.snapshot,
        card,
        document_key,
        SnapshotSources {
            md,
            profile,
            locale: &base.locale,
            resources,
        },
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
    base.snapshot.omikuji_visuals.clear();
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

/// Read-only inputs of the resolve-snapshot builders.
#[derive(Clone, Copy)]
struct SnapshotSources<'a> {
    md: &'a MasterData,
    profile: Option<&'a ProfileData>,
    locale: &'a str,
    /// Asset store and render-object metadata catalog that resource
    /// descriptors are sized from.
    resources: ResolveResourceContext<'a>,
}

/// Adds the page-local maps of one card page: fonts, colors, resource
/// descriptors, line-indent programs, and the honor, card-member and omikuji
/// visuals of its visible elements.
fn populate_page_overlay(
    snapshot: &mut ProfileResolveSnapshot,
    card: &CustomProfileCard,
    document_key: &str,
    sources: SnapshotSources<'_>,
) {
    let SnapshotSources {
        md,
        profile,
        locale,
        resources: assets,
    } = sources;
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
            AuthoredResource::MissingRow | AuthoredResource::NotDrawn => {
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
            AuthoredResource::Omikuji(plan) => {
                let client = OmikujiClient::for_masterdata(md, locale);
                snapshot
                    .omikuji_visuals
                    .insert(element.source_key, plan.visual(client));
            }
            AuthoredResource::LitBadge { image, normal_map } => {
                let (table, id) = resource_provenance(element.value);
                insert_resource_descriptor(
                    snapshot,
                    image.lookup_key,
                    image.resource.key,
                    (image.fallback.width, image.fallback.height),
                    table,
                    id,
                    assets,
                );
                // The normal map is a static asset, resolved like the
                // static honor frames.
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    snapshot.resources.entry(normal_map.lookup_key)
                {
                    entry.insert(static_descriptor(
                        normal_map.resource.key,
                        (normal_map.fallback.width, normal_map.fallback.height),
                        "badge_material",
                        id,
                        assets,
                    ));
                }
            }
        }
    }
    for element in ordered_profile_elements(card, document_key) {
        if !element.object().visible || snapshot.omitted_elements.contains(&element.source_key) {
            continue;
        }
        match element.value {
            ProfileElementRef::Text(text) => {
                if let Some(mut program) = crate::text::line_indent_program(text, md) {
                    let (_, _, rotation_deg, scale_x, _) =
                        sekai_profile_core::profile_transform::extract_transform(element.object());
                    program.rotation_deg = rotation_deg;
                    program.scale_x = scale_x;
                    snapshot.line_indent.insert(element.source_key, program);
                }
            }
            ProfileElementRef::Honor(honor) => {
                let Some(level) = sekai_profile_core::profile_data::placed_honor_level(
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
                let Some(level) = sekai_profile_core::profile_data::placed_bonds_honor_level(
                    profile.map(|profile| &profile.owned_honors),
                    honor.id,
                    honor.honor_level,
                ) else {
                    continue;
                };
                if let Some(visual) = build_bonds_honor_visual(
                    "customProfile.bondsHonors",
                    honor.id,
                    level,
                    honor.full_size,
                    BondsHonorView {
                        word_id: honor.word_id,
                        inverse: honor.inverse,
                        use_unit_virtual_singer: honor.use_unit_virtual_singer,
                    },
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

/// Installs region font 1 and the player-wide component snapshot that every
/// page of one profile shares. Without a profile the snapshot is unchanged.
fn populate_profile_base(snapshot: &mut ProfileResolveSnapshot, sources: SnapshotSources<'_>) {
    let SnapshotSources {
        md,
        profile,
        locale,
        resources: assets,
    } = sources;
    if let Some(profile) = profile {
        let challenge_rank_by_character = profile
            .challenge_ranks
            .iter()
            .map(|challenge| (challenge.character_id, challenge.rank))
            .collect::<std::collections::HashMap<_, _>>();
        if let Some(font) = md.resolve_font(1) {
            snapshot.fonts.insert(1, font);
        }
        let story_favorites = sekai_profile_core::profile_data::story_favorite_slots(
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
                        descriptor: resolve_card_member_key(member.card_id, 1, training, md).map(
                            |key| {
                                make_resource_descriptor(
                                    key,
                                    (600.0, 576.0),
                                    "cards",
                                    member.card_id,
                                    assets,
                                )
                            },
                        ),
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
        ordered_honor_slots.sort_by_key(|slot| std::cmp::Reverse(slot.full_size));
        if ordered_honor_slots.len() >= 2 {
            ordered_honor_slots.swap(0, 1);
        }
        let honor_slots = ordered_honor_slots
            .into_iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                let render_full = index == 0;
                if slot.profile_honor_type == "bonds" {
                    let (inverse, use_unit_virtual_singer) = slot.bonds_honor_view_flags();
                    build_bonds_honor_visual(
                        "userProfile.honorSlots",
                        slot.honor_id,
                        slot.honor_level,
                        render_full,
                        BondsHonorView {
                            word_id: slot.bonds_honor_word_id.unwrap_or_default(),
                            inverse,
                            use_unit_virtual_singer,
                        },
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
            localized_text: sekai_profile_core::locale::GENERAL_LOCALIZATION_KEYS
                .iter()
                .filter_map(|key| {
                    md.resolve_localized_text(key)
                        .or_else(|| sekai_profile_core::locale::resolve(locale, key))
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

/// How a bonds honor is drawn: the word shown over it, whether its two
/// characters swap sides, and whether the unit virtual singer art is used.
#[derive(Clone, Copy)]
struct BondsHonorView {
    word_id: i64,
    inverse: bool,
    use_unit_virtual_singer: bool,
}

fn build_bonds_honor_visual(
    source_field: &str,
    honor_id: i32,
    honor_level: i32,
    full_size: bool,
    view: BondsHonorView,
    md: &MasterData,
    resources: ResolveResourceContext<'_>,
) -> Option<HonorVisualSnapshot> {
    let BondsHonorView {
        word_id,
        inverse,
        use_unit_virtual_singer,
    } = view;
    md.get_bonds_honor(honor_id)?;
    let plan = sekai_profile_core::masterdata::bonds_honor_asset_plan(
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
mod tests;
