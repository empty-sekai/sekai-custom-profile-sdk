//! Scene dumps for inspecting the scene a card builds at a given tick.

use super::CustomProfileRenderer;
use crate::types::CustomProfileCard;

impl CustomProfileRenderer {
    /// Builds the text-only scene of `card` at `tick` and dumps it.
    pub fn dump_text_scene(
        &self,
        card: &CustomProfileCard,
        document_key: &str,
        tick: u64,
    ) -> Result<sekai_profile_core::SceneDump, String> {
        let started = std::time::Instant::now();
        let md = self.snapshot();
        let mut scene = crate::core_shadow::build_text_scene(card, &md, document_key)
            .map_err(|error| error.to_string())?;
        scene.advance_to_tick(tick);
        let dump = scene.dump();
        tracing::debug!(
            document_key,
            tick,
            layers = dump.layers.len(),
            dynamic_layers = dump
                .layers
                .iter()
                .filter(|layer| layer.dynamic.is_some())
                .count(),
            dynamic_evaluations = dump.telemetry.dynamic_evaluations,
            elapsed_us = started.elapsed().as_micros(),
            "renderer core native text shadow dump"
        );
        Ok(dump)
    }

    /// Builds the scene of every authored layer of `card` at `tick` and dumps
    /// it.
    pub fn dump_semantic_scene(
        &self,
        card: &CustomProfileCard,
        document_key: &str,
        region: &str,
        tick: u64,
    ) -> Result<sekai_profile_core::SceneDump, String> {
        self.dump_semantic_scene_with_profile(card, None, document_key, region, region, tick)
    }

    /// [`Self::dump_semantic_scene`] with player data.
    pub fn dump_semantic_scene_with_profile(
        &self,
        card: &CustomProfileCard,
        profile: Option<&crate::profile::ProfileData>,
        document_key: &str,
        region: &str,
        locale: &str,
        tick: u64,
    ) -> Result<sekai_profile_core::SceneDump, String> {
        let md = self.snapshot();
        let mut scene = crate::core_shadow::build_scene(
            card,
            &md,
            document_key,
            region,
            profile,
            locale,
            self.assets.as_deref(),
        )?;
        scene.advance_to_tick(tick);
        Ok(scene.dump())
    }
}
