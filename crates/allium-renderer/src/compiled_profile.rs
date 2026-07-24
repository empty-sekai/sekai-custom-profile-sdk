//! Cross-page compilation for game custom-profile rendering.
//!
//! This module lifts the shared renderer-core semantic scene into a batch
//! identity and a unique render-object working set. It does not execute pixels;
//! native backends consume the resulting ordered scenes without resolving the
//! same profile, component recipes, or asset identities again per page.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use allium_renderer_core::profile_scene::authored_kind_name;
#[cfg(test)]
use allium_renderer_core::AuthoredElementKind;
use allium_renderer_core::{ResourceKey, SemanticCommandPayload, ShapePrimitive};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::assets::AssetStore;
use crate::masterdata::MasterData;
use crate::profile::ProfileData;
use crate::render_object::{render_object_key_for_resource, MappedRenderObjectStore};
use crate::semantic_resolve::{
    build_profile_resolve_base_snapshot, resolve_card_commands_with_base_timed, ResolveError,
    ResolveResourceContext, ResolvedCardCommands,
};
use crate::types::CustomProfileCard;

pub const COMPILED_PROFILE_BATCH_SCHEMA: &str = "allium.compiled-profile-batch.v4";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompiledResourceUseKind {
    Image,
    AlphaMask,
    ShapeMask,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct CompiledResourceRequest {
    pub namespace: String,
    pub key: String,
    pub use_kind: CompiledResourceUseKind,
    pub render_object_key: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledProfilePage {
    pub seq: i32,
    pub document_key: String,
    pub scene: ResolvedCardCommands,
    #[serde(default)]
    pub render_object_keys: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompiledProfileBatchWork {
    pub page_count: u64,
    pub layer_count: u64,
    pub command_count: u64,
    pub text_command_count: u64,
    pub image_command_count: u64,
    pub shape_command_count: u64,
    pub composite_command_count: u64,
    pub unique_resource_count: u64,
    pub unique_render_object_count: u64,
    pub authored_layer_counts: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledProfileBatch {
    pub schema: String,
    pub identity: String,
    pub batch_key: String,
    pub locale: String,
    pub pages: Vec<CompiledProfilePage>,
    pub resources: Vec<CompiledResourceRequest>,
    pub render_object_keys: Vec<String>,
    pub work: CompiledProfileBatchWork,
    pub profile_snapshot_base_ns: u64,
    pub page_snapshot_overlay_ns: u64,
    pub semantic_lowering_ns: u64,
    pub resource_catalog_lookup_ns: u64,
    pub compile_ns: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PreparedRenderObjectBatch {
    pub requested_object_count: u64,
    pub mapped_object_count: u64,
    pub missing_object_keys: Vec<String>,
    #[serde(default)]
    pub missing_page_object_keys: BTreeMap<i32, Vec<String>>,
    pub logical_pixel_bytes: u64,
    pub will_need_object_count: u64,
    pub will_need_bytes: u64,
    pub will_need_error_count: u64,
    pub synchronous_prefault_object_count: u64,
    pub synchronous_prefault_bytes: u64,
    pub prefault_checksum: u64,
    pub prepare_ns: u64,
}

#[derive(Debug, Error)]
pub enum CompileProfileBatchError {
    #[error("compiled profile batch key and locale must not be empty")]
    EmptyIdentity,
    #[error("compiled profile batch contains duplicate page seq {0}")]
    DuplicatePage(i32),
    #[error("profile page {seq} semantic resolution failed: {source}")]
    Resolve {
        seq: i32,
        #[source]
        source: ResolveError,
    },
}

pub fn compile_profile_batch<'a>(
    batch_key: &str,
    locale: &str,
    pages: impl IntoIterator<Item = (i32, &'a CustomProfileCard)>,
    md: &MasterData,
    profile: Option<&ProfileData>,
    assets: Option<&AssetStore>,
) -> Result<CompiledProfileBatch, CompileProfileBatchError> {
    compile_profile_batch_with_store(batch_key, locale, pages, md, profile, assets, None)
}

#[allow(clippy::too_many_arguments)]
pub fn compile_profile_batch_with_store<'a>(
    batch_key: &str,
    locale: &str,
    pages: impl IntoIterator<Item = (i32, &'a CustomProfileCard)>,
    md: &MasterData,
    profile: Option<&ProfileData>,
    assets: Option<&AssetStore>,
    render_objects: Option<&MappedRenderObjectStore>,
) -> Result<CompiledProfileBatch, CompileProfileBatchError> {
    if batch_key.trim().is_empty() || locale.trim().is_empty() {
        return Err(CompileProfileBatchError::EmptyIdentity);
    }
    let started = Instant::now();
    let mut inputs = pages.into_iter().collect::<Vec<_>>();
    inputs.sort_unstable_by_key(|(seq, _)| *seq);
    if let Some(duplicate) = inputs
        .windows(2)
        .find(|window| window[0].0 == window[1].0)
        .map(|window| window[0].0)
    {
        return Err(CompileProfileBatchError::DuplicatePage(duplicate));
    }

    let mut compiled_pages = Vec::with_capacity(inputs.len());
    let mut resource_set = BTreeSet::new();
    let mut render_object_set = BTreeSet::new();
    let mut work = CompiledProfileBatchWork::default();
    let catalog_lookup_ns = std::cell::Cell::new(0u64);
    let resources = ResolveResourceContext {
        assets,
        render_objects,
        catalog_lookup_ns: Some(&catalog_lookup_ns),
    };
    let base_started = Instant::now();
    let mut base = inputs.first().map(|(_, seed_card)| {
        build_profile_resolve_base_snapshot(seed_card, md, profile, locale, resources)
    });
    let profile_snapshot_base_ns = elapsed_ns(base_started);
    let mut page_snapshot_overlay_ns = 0u64;
    let mut semantic_lowering_ns = 0u64;
    for (seq, card) in inputs {
        let document_key = format!("{batch_key}:seq:{seq}");
        let (scene, timings) = resolve_card_commands_with_base_timed(
            base.as_mut()
                .expect("non-empty inputs have a base snapshot"),
            card,
            md,
            &document_key,
            profile,
            resources,
        )
        .map_err(|source| CompileProfileBatchError::Resolve { seq, source })?;
        page_snapshot_overlay_ns =
            page_snapshot_overlay_ns.saturating_add(timings.page_snapshot_overlay_ns);
        semantic_lowering_ns = semantic_lowering_ns.saturating_add(timings.semantic_lowering_ns);
        accumulate_scene_work(&scene, &mut work);
        let page_resources = collect_scene_resource_requests(&scene);
        let page_render_object_keys = page_resources
            .iter()
            .map(|request| request.render_object_key.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for request in page_resources {
            render_object_set.insert(request.render_object_key.clone());
            resource_set.insert(request);
        }
        compiled_pages.push(CompiledProfilePage {
            seq,
            document_key,
            scene,
            render_object_keys: page_render_object_keys,
        });
    }
    work.page_count = compiled_pages.len() as u64;
    work.unique_resource_count = resource_set.len() as u64;
    work.unique_render_object_count = render_object_set.len() as u64;
    let resources = resource_set.into_iter().collect::<Vec<_>>();
    let render_object_keys = render_object_set.into_iter().collect::<Vec<_>>();
    let mut identity = Sha256::new();
    identity.update(COMPILED_PROFILE_BATCH_SCHEMA.as_bytes());
    identity.update((batch_key.len() as u64).to_le_bytes());
    identity.update(batch_key.as_bytes());
    identity.update((locale.len() as u64).to_le_bytes());
    identity.update(locale.as_bytes());
    for page in &compiled_pages {
        identity.update(page.seq.to_le_bytes());
        identity.update((page.document_key.len() as u64).to_le_bytes());
        identity.update(page.document_key.as_bytes());
        for layer in &page.scene.layers {
            identity.update(layer.id.0.to_le_bytes());
        }
        for command in &page.scene.commands {
            identity.update(command.id.0.to_le_bytes());
        }
    }
    for resource in &resources {
        identity.update((resource.namespace.len() as u64).to_le_bytes());
        identity.update(resource.namespace.as_bytes());
        identity.update((resource.key.len() as u64).to_le_bytes());
        identity.update(resource.key.as_bytes());
        identity.update([resource.use_kind as u8]);
    }
    Ok(CompiledProfileBatch {
        schema: COMPILED_PROFILE_BATCH_SCHEMA.into(),
        identity: hex::encode(identity.finalize()),
        batch_key: batch_key.into(),
        locale: locale.into(),
        pages: compiled_pages,
        resources,
        render_object_keys,
        work,
        profile_snapshot_base_ns,
        page_snapshot_overlay_ns,
        semantic_lowering_ns,
        resource_catalog_lookup_ns: catalog_lookup_ns.get(),
        compile_ns: elapsed_ns(started),
    })
}

impl CompiledProfileBatch {
    /// Resolve and prepare the exact immutable objects required by this batch.
    /// `MADV_WILLNEED` starts I/O without blocking the request thread; platforms
    /// without it, or an advice failure, fall back to an ordered page touch.
    pub fn prepare_render_objects(
        &self,
        store: &MappedRenderObjectStore,
    ) -> PreparedRenderObjectBatch {
        let started = Instant::now();
        let mut output = PreparedRenderObjectBatch {
            requested_object_count: self.render_object_keys.len() as u64,
            ..PreparedRenderObjectBatch::default()
        };
        let mut objects = Vec::with_capacity(self.render_object_keys.len());
        for key in &self.render_object_keys {
            let Some(object) = store.object(key) else {
                output.missing_object_keys.push(key.clone());
                continue;
            };
            output.mapped_object_count = output.mapped_object_count.saturating_add(1);
            output.logical_pixel_bytes = output
                .logical_pixel_bytes
                .saturating_add(object.pixels.len() as u64);
            objects.push(object);
        }
        let missing = output
            .missing_object_keys
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        output.missing_page_object_keys = missing_render_objects_by_page(
            self.pages
                .iter()
                .map(|page| (page.seq, page.render_object_keys.as_slice())),
            &missing,
        );
        objects.sort_unstable_by_key(|object| (object.entry.page, object.entry.offset));

        for object in objects {
            #[cfg(unix)]
            let synchronous_prefault = match store.advise_object_will_need(&object.entry.key) {
                Some(Ok(())) => {
                    output.will_need_object_count = output.will_need_object_count.saturating_add(1);
                    output.will_need_bytes = output
                        .will_need_bytes
                        .saturating_add(object.pixels.len() as u64);
                    false
                }
                Some(Err(_)) | None => {
                    output.will_need_error_count = output.will_need_error_count.saturating_add(1);
                    true
                }
            };

            #[cfg(not(unix))]
            let synchronous_prefault = true;

            if synchronous_prefault {
                output.synchronous_prefault_object_count =
                    output.synchronous_prefault_object_count.saturating_add(1);
                output.synchronous_prefault_bytes = output
                    .synchronous_prefault_bytes
                    .saturating_add(object.pixels.len() as u64);
                for page in object.pixels.chunks(4096) {
                    output.prefault_checksum = output
                        .prefault_checksum
                        .wrapping_mul(0x100000001b3)
                        .wrapping_add(u64::from(page[0]));
                }
                if let Some(last) = object.pixels.last() {
                    output.prefault_checksum =
                        output.prefault_checksum.wrapping_add(u64::from(*last));
                }
            }
        }
        output.prepare_ns = elapsed_ns(started);
        output
    }
}

fn missing_render_objects_by_page<'a>(
    pages: impl IntoIterator<Item = (i32, &'a [String])>,
    missing: &BTreeSet<String>,
) -> BTreeMap<i32, Vec<String>> {
    pages
        .into_iter()
        .filter_map(|(seq, keys)| {
            let page_missing = keys
                .iter()
                .filter(|key| missing.contains(*key))
                .cloned()
                .collect::<Vec<_>>();
            (!page_missing.is_empty()).then_some((seq, page_missing))
        })
        .collect()
}

fn collect_scene_resource_requests(scene: &ResolvedCardCommands) -> Vec<CompiledResourceRequest> {
    let mut resources = BTreeSet::new();
    for command in &scene.commands {
        match &command.payload {
            SemanticCommandPayload::Image {
                resource,
                alpha_mask,
                ..
            } => {
                insert_resource(&mut resources, resource, CompiledResourceUseKind::Image);
                if let Some(mask) = alpha_mask {
                    insert_resource(&mut resources, mask, CompiledResourceUseKind::AlphaMask);
                }
            }
            SemanticCommandPayload::Shape {
                primitive: ShapePrimitive::AssetMask { resource },
                ..
            } => insert_resource(&mut resources, resource, CompiledResourceUseKind::ShapeMask),
            SemanticCommandPayload::Text { .. }
            | SemanticCommandPayload::Shape { .. }
            | SemanticCommandPayload::Composite { .. } => {}
        }
    }
    resources.into_iter().collect()
}

fn insert_resource(
    output: &mut BTreeSet<CompiledResourceRequest>,
    resource: &ResourceKey,
    use_kind: CompiledResourceUseKind,
) {
    output.insert(CompiledResourceRequest {
        namespace: resource.namespace.clone(),
        key: resource.key.clone(),
        use_kind,
        render_object_key: render_object_key_for_resource(resource),
    });
}

fn accumulate_scene_work(scene: &ResolvedCardCommands, work: &mut CompiledProfileBatchWork) {
    work.layer_count = work.layer_count.saturating_add(scene.layers.len() as u64);
    work.command_count = work
        .command_count
        .saturating_add(scene.commands.len() as u64);
    for layer in &scene.layers {
        let name = authored_kind_name(layer.authored_kind).to_string();
        let count = work.authored_layer_counts.entry(name).or_default();
        *count = count.saturating_add(1);
    }
    for command in &scene.commands {
        match &command.payload {
            SemanticCommandPayload::Text { .. } => {
                work.text_command_count = work.text_command_count.saturating_add(1)
            }
            SemanticCommandPayload::Image { .. } => {
                work.image_command_count = work.image_command_count.saturating_add(1)
            }
            SemanticCommandPayload::Shape { .. } => {
                work.shape_command_count = work.shape_command_count.saturating_add(1)
            }
            SemanticCommandPayload::Composite { .. } => {
                work.composite_command_count = work.composite_command_count.saturating_add(1)
            }
        }
    }
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_object_resource_keys_are_namespace_stable() {
        assert_eq!(
            render_object_key_for_resource(&ResourceKey {
                namespace: "assets".into(),
                key: "honor/frame_degree_m_1".into(),
            }),
            "texture:assets/honor/frame_degree_m_1"
        );
        assert_eq!(
            render_object_key_for_resource(&ResourceKey {
                namespace: "render-object".into(),
                key: "standard_honor:contract/id-1".into(),
            }),
            "standard_honor:contract/id-1"
        );
    }

    #[test]
    fn authored_kind_names_remain_usable_as_low_cardinality_metrics() {
        assert_eq!(
            authored_kind_name(AuthoredElementKind::CardMember),
            "card-member"
        );
        assert_eq!(
            authored_kind_name(AuthoredElementKind::BondsHonor),
            "bonds-honor"
        );
    }

    #[test]
    fn render_object_misses_mark_only_affected_pages_for_legacy_fallback() {
        let complete_page = vec!["texture:assets/complete".to_string()];
        let missing_page = vec![
            "texture:assets/complete".to_string(),
            "texture:assets/missing".to_string(),
        ];
        let missing = BTreeSet::from(["texture:assets/missing".to_string()]);
        let affected = missing_render_objects_by_page(
            [(1, complete_page.as_slice()), (2, missing_page.as_slice())],
            &missing,
        );

        assert_eq!(affected.len(), 1);
        assert!(!affected.contains_key(&1));
        assert_eq!(
            affected.get(&2),
            Some(&vec!["texture:assets/missing".to_string()])
        );
    }
}
