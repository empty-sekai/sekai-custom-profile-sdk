//! The codepoints the text of a card draws, and the fallback-font glyphs
//! generated for codepoints the primary atlases lack.

use super::authored::resolve_native_profile_scene;
use super::CustomProfileRenderer;
use crate::masterdata::MasterData;
use crate::types::CustomProfileCard;
use std::sync::Arc;

/// Adds the codepoints the profile fallback font has to supply for `text`,
/// whose primary atlas holds the codepoints `primary_has` accepts.
///
/// TextMesh Pro draws a character from the primary face when the face has
/// it or its same-face stand-in. Otherwise the fallback font is searched for
/// the character and then its stand-in, and when that font has neither, the
/// missing-glyph square is drawn instead, from the primary face if it has
/// one. Characters that draw nothing, markup and control characters need no
/// glyph.
pub(super) fn extend_profile_fallback_codepoints(
    requested: &mut std::collections::BTreeSet<u32>,
    text: &str,
    primary_has: impl Fn(u32) -> bool,
) {
    use sekai_profile_core::tmp_text::{glyph, parse_segments};

    let has = |ch: char| primary_has(u32::from(ch));
    let mut needs_fallback = false;
    for segment in parse_segments(text, 0.0) {
        for source in segment.text.chars() {
            let (ch, _) = segment.transform_char(source);
            if !glyph::is_drawn(ch) || ch.is_control() || has(ch) {
                continue;
            }
            let alternate =
                glyph::same_face_alternate(ch).filter(|alternate| glyph::is_drawn(*alternate));
            if alternate.is_some_and(has) {
                continue;
            }
            requested.insert(u32::from(ch));
            requested.extend(alternate.map(u32::from));
            needs_fallback = true;
        }
    }
    if needs_fallback && !has(glyph::MISSING_GLYPH_CHARACTER) {
        requested.insert(u32::from(glyph::MISSING_GLYPH_CHARACTER));
    }
}

impl CustomProfileRenderer {
    /// The codepoints each font family has to supply to draw the text of
    /// `card`, keyed by family as the master data resolves it. Each set holds
    /// the characters TextMesh Pro draws after case transforms, their
    /// same-face stand-ins, and the missing-glyph square. Pass the result to
    /// [`crate::prepare::PrepareOptions::codepoints`] to build atlases that
    /// cover just these cards.
    pub fn text_codepoints(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
    ) -> Result<
        std::collections::BTreeMap<String, std::collections::BTreeSet<u32>>,
        crate::semantic_resolve::ResolveError,
    > {
        use sekai_profile_core::tmp_text::{glyph, parse_segments};
        use sekai_profile_core::{FontRole, SemanticCommandPayload, TextSource};

        let md = self.snapshot();
        let scene = resolve_native_profile_scene(card, &md, profile, self.assets.as_deref(), None)?;
        let mut demand = std::collections::BTreeMap::new();
        for command in &scene.commands {
            let SemanticCommandPayload::Text {
                source, font_role, ..
            } = &command.payload
            else {
                continue;
            };
            let font_id = match font_role {
                FontRole::RegionFontId(font_id) => *font_id,
            };
            let Some(family) = md.resolve_font_or_default(font_id) else {
                continue;
            };
            let value = match source {
                TextSource::Authored { value }
                | TextSource::ProfileField { value, .. }
                | TextSource::MasterData { value, .. }
                | TextSource::Localized { value, .. } => value,
            };
            let codepoints: &mut std::collections::BTreeSet<u32> =
                demand.entry(family).or_default();
            for segment in parse_segments(value, 0.0) {
                for source in segment.text.chars() {
                    let (ch, _) = segment.transform_char(source);
                    if !glyph::is_drawn(ch) || ch.is_control() {
                        continue;
                    }
                    codepoints.insert(u32::from(ch));
                    if let Some(alternate) = glyph::same_face_alternate(ch)
                        .filter(|alternate| glyph::is_drawn(*alternate))
                    {
                        codepoints.insert(u32::from(alternate));
                    }
                }
            }
            codepoints.insert(u32::from(glyph::MISSING_GLYPH_CHARACTER));
        }
        Ok(demand)
    }

    /// Generates any codepoints the declared fonts' atlases lack, from the
    /// FreeType fallback face, and publishes the result into the atlas set.
    ///
    /// Runs before rendering so the request path only ever reads atlases; it
    /// never generates glyphs while compositing.
    pub(super) fn ensure_profile_fallback_for_scenes<'a>(
        &self,
        scenes: impl IntoIterator<Item = &'a sekai_profile_core::profile_scene::ResolvedProfileScene>,
        md: &MasterData,
    ) -> Result<Option<crate::sdf::fallback_cache::PersistentFallbackSdfCacheReport>, String> {
        use sekai_profile_core::{FontRole, SemanticCommandPayload, TextSource};
        use std::collections::BTreeSet;

        let Some(cache) = self.profile_fallback_sdf_cache.as_deref() else {
            return Ok(None);
        };
        let atlases = self.sdf_atlases.load_full();
        let mut requested = BTreeSet::new();
        for scene in scenes {
            for command in &scene.commands {
                // Live-master progress text is player state, not authored glyphs.
                if command.role.starts_with("honor-") && command.role.ends_with("-progress") {
                    continue;
                }
                let SemanticCommandPayload::Text {
                    source, font_role, ..
                } = &command.payload
                else {
                    continue;
                };
                let font_id = match font_role {
                    FontRole::RegionFontId(font_id) => *font_id,
                };
                let Some(primary_family) = md.resolve_font_or_default(font_id) else {
                    continue;
                };
                if primary_family == crate::sdf::atlas::PROFILE_TEXT_FALLBACK_FONT_FAMILY {
                    continue;
                }
                let Some((_, primary_atlas)) = atlases.atlas_for_font_family(&primary_family)
                else {
                    continue;
                };
                let value = match source {
                    TextSource::Authored { value }
                    | TextSource::ProfileField { value, .. }
                    | TextSource::MasterData { value, .. }
                    | TextSource::Localized { value, .. } => value,
                };
                extend_profile_fallback_codepoints(&mut requested, value, |codepoint| {
                    primary_atlas.glyph(codepoint).is_some()
                });
            }
        }
        self.ensure_profile_fallback_codepoints(requested, cache, &atlases)
    }

    /// Generates the fallback glyphs an animation export needs from a
    /// resolved scene, installing the resulting atlas the same way a page
    /// render does.
    #[cfg(feature = "animation-export")]
    pub(crate) fn prepare_profile_fallback_for_animation_scene(
        &self,
        scene: &sekai_profile_core::profile_scene::ResolvedProfileScene,
        md: &MasterData,
    ) -> Result<Option<crate::sdf::fallback_cache::PersistentFallbackSdfCacheReport>, String> {
        if self.profile_fallback_sdf_cache.is_none() {
            return Ok(None);
        }
        self.ensure_profile_fallback_for_scenes(std::iter::once(scene), md)
    }

    /// The same preparation for a card that resolved to no scene: the
    /// codepoints come straight off the authored text elements.
    #[cfg(feature = "animation-export")]
    pub(crate) fn prepare_profile_fallback_for_animation_card(
        &self,
        card: &CustomProfileCard,
        md: &MasterData,
    ) -> Result<Option<crate::sdf::fallback_cache::PersistentFallbackSdfCacheReport>, String> {
        use std::collections::BTreeSet;

        let Some(cache) = self.profile_fallback_sdf_cache.as_deref() else {
            return Ok(None);
        };
        let atlases = self.sdf_atlases.load_full();
        let mut requested = BTreeSet::new();
        for text in &card.texts {
            let Some(primary_family) = md.resolve_font_or_default(text.font_id) else {
                continue;
            };
            if primary_family == crate::sdf::atlas::PROFILE_TEXT_FALLBACK_FONT_FAMILY {
                continue;
            }
            let Some((_, primary_atlas)) = atlases.atlas_for_font_family(&primary_family) else {
                continue;
            };
            extend_profile_fallback_codepoints(&mut requested, &text.text, |codepoint| {
                primary_atlas.glyph(codepoint).is_some()
            });
        }
        self.ensure_profile_fallback_codepoints(requested, cache, &atlases)
    }

    fn ensure_profile_fallback_codepoints(
        &self,
        requested: std::collections::BTreeSet<u32>,
        cache: &crate::sdf::fallback_cache::PersistentFallbackSdfCache,
        atlases: &crate::sdf::atlas::MappedSdfAtlasSet,
    ) -> Result<Option<crate::sdf::fallback_cache::PersistentFallbackSdfCacheReport>, String> {
        if requested.is_empty() {
            return Ok(None);
        }

        let (atlas, report) = cache.ensure_codepoints(&requested)?;
        if let Some(atlas) = atlas {
            let already_published = atlases
                .atlas_for_font_family(cache.font_family())
                .is_some_and(|(_, installed)| {
                    installed.manifest_sha256() == atlas.manifest_sha256()
                });
            if !already_published {
                let mut updated = (*self.sdf_atlases.load_full()).clone();
                updated
                    .replace_or_insert(atlas)
                    .map_err(|error| error.to_string())?;
                self.sdf_atlases.store(Arc::new(updated));
            }
        }
        Ok(Some(report))
    }
}
