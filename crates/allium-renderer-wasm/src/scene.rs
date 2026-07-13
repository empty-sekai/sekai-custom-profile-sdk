use std::cell::RefCell;
use std::collections::BTreeMap;

use allium_renderer_core::{
    AuthoredElementKind, BlendMode, GlyphSource, InteractionRegionSource, LayerId, LayerKind,
    LayerSource, LineIndentSource, Matrix2d, ParameterValue, Quad, Rect, RenderMaskOverride, Scene,
    SceneSource, SemanticCommandPayload, SemanticCommandSource, StableId,
};
use serde::{Deserialize, Serialize};

thread_local! {
    static SCENES: RefCell<SceneTable> = RefCell::new(SceneTable::default());
}

#[derive(Default)]
struct SceneTable {
    next_handle: u32,
    scenes: BTreeMap<u32, Scene>,
}

#[derive(Serialize)]
struct CreateResponse {
    handle: u32,
    layer_bindings: Vec<LayerBinding>,
    snapshot: allium_renderer_core::SceneSnapshot,
}

#[derive(Serialize)]
struct LayerBinding {
    source_key: String,
    layer_id: LayerId,
}

#[derive(Deserialize)]
struct MaskRequest {
    layer_id: LayerId,
    visible: bool,
}

#[derive(Deserialize)]
struct BulkMaskRequest {
    expected_layer_table_revision: u64,
    overrides: Vec<MaskOverrideRequest>,
}

#[derive(Deserialize)]
struct MaskOverrideRequest {
    layer_id: LayerId,
    mask_override: RenderMaskOverride,
}

#[derive(Deserialize)]
struct TabRequest {
    control_id: StableId,
    value: String,
}

#[derive(Deserialize)]
struct ScrollRequest {
    control_id: StableId,
    #[serde(default)]
    offset: Option<f32>,
    #[serde(default)]
    delta: Option<f32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SceneCreateInput {
    scene_key: String,
    region: String,
    font_engine_fingerprint: String,
    raster_contract: String,
    layers: Vec<LayerCreateInput>,
    #[serde(default)]
    glyphs: Vec<GlyphCreateInput>,
    #[serde(default)]
    semantic_commands: Vec<SemanticCommandCreateInput>,
    #[serde(default)]
    interaction_regions: Vec<InteractionRegionCreateInput>,
    #[serde(default)]
    component_controls: Vec<allium_renderer_core::profile_scene::ComponentControlSource>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolvedProfileSceneCreateInput {
    document_key: String,
    #[serde(default = "default_region")]
    region: String,
    card: allium_renderer_core::profile_source::CustomProfileCard,
    snapshot: allium_renderer_core::profile_scene::ProfileResolveSnapshot,
}

fn default_region() -> String {
    "cn".into()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SemanticCommandCreateInput {
    layer_source_key: String,
    ordinal: u32,
    role: String,
    bounds: Rect,
    matrix: Matrix2d,
    hit_geometry: Quad,
    blend_mode: BlendMode,
    #[serde(default)]
    clip: Option<Quad>,
    #[serde(default)]
    control_bindings: Vec<allium_renderer_core::CommandControlBinding>,
    #[serde(default)]
    metadata: BTreeMap<String, ParameterValue>,
    payload: SemanticCommandPayload,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InteractionRegionCreateInput {
    layer_source_key: String,
    ordinal: u32,
    role: String,
    bounds: Rect,
    quad: Quad,
    matrix: Matrix2d,
    hit_geometry: Quad,
    #[serde(default)]
    clip: Option<Quad>,
    #[serde(default)]
    control_bindings: Vec<allium_renderer_core::CommandControlBinding>,
    #[serde(default)]
    resolved_data: BTreeMap<String, ParameterValue>,
    #[serde(default)]
    capabilities: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlyphCreateInput {
    layer_source_key: String,
    output_ordinal: u32,
    source_span: [u32; 2],
    bounds: Rect,
    quad: Quad,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LayerCreateInput {
    source_key: String,
    parent_source_key: Option<String>,
    kind: LayerKind,
    #[serde(default)]
    authored_kind: AuthoredElementKind,
    #[serde(default)]
    authored_index: u32,
    z: i32,
    authored_visible: bool,
    source_content: String,
    #[serde(default)]
    resolved_parameters: BTreeMap<String, ParameterValue>,
    bounds: Rect,
    quad: Quad,
    matrix: Matrix2d,
    hit_geometry: Quad,
    line_indent: Option<LineIndentCreateInput>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LineIndentCreateInput {
    percent: f32,
    advances_tmp: Vec<f32>,
    rotation_deg: f32,
    scale_x: f32,
}

pub fn create(input: &str) -> Result<String, String> {
    let input: SceneCreateInput =
        serde_json::from_str(input).map_err(|error| format!("parse core scene failed: {error}"))?;
    let scene_id = StableId::derive("scene-v1", input.scene_key.as_bytes());
    let ids = input
        .layers
        .iter()
        .map(|layer| {
            let key = format!("{}\0{}", input.scene_key, layer.source_key);
            (
                layer.source_key.clone(),
                StableId::derive("layer-v1", key.as_bytes()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let layer_bindings = ids
        .iter()
        .map(|(source_key, layer_id)| LayerBinding {
            source_key: source_key.clone(),
            layer_id: *layer_id,
        })
        .collect();
    let layers = input
        .layers
        .into_iter()
        .map(|layer| {
            let id = ids[&layer.source_key];
            let parent_id = layer
                .parent_source_key
                .as_ref()
                .and_then(|key| ids.get(key))
                .copied();
            LayerSource {
                id,
                parent_id,
                kind: layer.kind,
                authored_kind: layer.authored_kind,
                authored_index: layer.authored_index,
                game_layer: layer.z,
                z: layer.z,
                authored_visible: layer.authored_visible,
                source_content: layer.source_content,
                resolved_parameters: layer.resolved_parameters,
                bounds: layer.bounds,
                quad: layer.quad,
                matrix: layer.matrix,
                hit_geometry: layer.hit_geometry,
                line_indent: layer.line_indent.map(|program| LineIndentSource {
                    percent: program.percent,
                    advances_tmp: program.advances_tmp,
                    rotation_deg: program.rotation_deg,
                    scale_x: program.scale_x,
                }),
            }
        })
        .collect();
    let glyphs = input
        .glyphs
        .into_iter()
        .map(|glyph| {
            let layer_id = ids[&glyph.layer_source_key];
            let key = format!(
                "{}\0{}\0{}\0{}:{}",
                input.scene_key,
                glyph.layer_source_key,
                glyph.output_ordinal,
                glyph.source_span[0],
                glyph.source_span[1],
            );
            GlyphSource {
                id: StableId::derive("glyph-instance-v1", key.as_bytes()),
                layer_id,
                output_ordinal: glyph.output_ordinal,
                source_span: glyph.source_span,
                bounds: glyph.bounds,
                quad: glyph.quad,
            }
        })
        .collect();
    let semantic_commands = input
        .semantic_commands
        .into_iter()
        .map(|command| {
            let layer_id = ids[&command.layer_source_key];
            let key = format!(
                "{}\0{}\0{}\0{}",
                input.scene_key, command.layer_source_key, command.role, command.ordinal
            );
            SemanticCommandSource {
                id: StableId::derive("semantic-command-v1", key.as_bytes()),
                layer_id,
                role: command.role,
                bounds: command.bounds,
                matrix: command.matrix,
                hit_geometry: command.hit_geometry,
                blend_mode: command.blend_mode,
                clip: command.clip,
                control_bindings: command.control_bindings,
                metadata: command.metadata,
                numeric_text_runs: allium_renderer_core::tmp_text::numeric_text_runs(
                    match &command.payload {
                        SemanticCommandPayload::Text { source, .. } => match source {
                            allium_renderer_core::TextSource::Authored { value }
                            | allium_renderer_core::TextSource::ProfileField { value, .. }
                            | allium_renderer_core::TextSource::MasterData { value, .. }
                            | allium_renderer_core::TextSource::Localized { value, .. } => value,
                        },
                        _ => "",
                    },
                ),
                payload: command.payload,
            }
        })
        .collect();
    let interaction_regions = input
        .interaction_regions
        .into_iter()
        .map(|region| {
            let layer_id = ids[&region.layer_source_key];
            let key = format!(
                "{}\0{}\0{}\0{}",
                input.scene_key, region.layer_source_key, region.role, region.ordinal
            );
            InteractionRegionSource {
                id: StableId::derive("interaction-region-v1", key.as_bytes()),
                layer_id,
                role: region.role,
                bounds: region.bounds,
                quad: region.quad,
                matrix: region.matrix,
                hit_geometry: region.hit_geometry,
                clip: region.clip,
                control_bindings: region.control_bindings,
                resolved_data: region.resolved_data,
                capabilities: region.capabilities,
            }
        })
        .collect();
    let source = SceneSource {
        scene_id,
        region: input.region,
        font_engine_fingerprint: input.font_engine_fingerprint,
        raster_contract: input.raster_contract,
        layers,
        glyphs,
        semantic_commands,
        interaction_regions,
        component_controls: input.component_controls,
    };
    let scene = Scene::new(source).map_err(|error| error.to_string())?;
    SCENES.with(|table| {
        let mut table = table.borrow_mut();
        table.next_handle = table.next_handle.wrapping_add(1).max(1);
        let handle = table.next_handle;
        let snapshot = scene.snapshot();
        table.scenes.insert(handle, scene);
        serde_json::to_string(&CreateResponse {
            handle,
            layer_bindings,
            snapshot,
        })
        .map_err(|error| error.to_string())
    })
}

pub fn create_resolved_profile(input: &str) -> Result<String, String> {
    let input: ResolvedProfileSceneCreateInput = serde_json::from_str(input)
        .map_err(|error| format!("parse resolved profile scene failed: {error}"))?;
    let resolved = allium_renderer_core::profile_scene::resolve_profile_scene(
        &input.card,
        &input.document_key,
        &input.snapshot,
    )
    .map_err(|error| error.to_string())?;
    create_compiled_profile(&input.document_key, &input.region, resolved, false)
}

pub fn create_compiled_profile(
    document_key: &str,
    region: &str,
    resolved: allium_renderer_core::profile_scene::ResolvedProfileScene,
    static_final: bool,
) -> Result<String, String> {
    let layer_bindings = resolved
        .layers
        .iter()
        .map(|layer| LayerBinding {
            source_key: format!(
                "{}:{}",
                allium_renderer_core::profile_scene::authored_kind_name(layer.authored_kind),
                layer.authored_index
            ),
            layer_id: layer.id,
        })
        .collect();
    let mut scene = Scene::new(SceneSource {
        scene_id: StableId::derive("profile-scene-v2", document_key.as_bytes()),
        region: region.to_owned(),
        font_engine_fingerprint: "freetype-wasm-profile-v2".into(),
        raster_contract: "sdf-edt-v2".into(),
        layers: resolved.layers,
        glyphs: Vec::new(),
        semantic_commands: resolved.commands,
        interaction_regions: resolved.interaction_regions,
        component_controls: resolved.controls,
    })
    .map_err(|error| error.to_string())?;
    if static_final {
        scene.advance_to_static_final();
    }
    SCENES.with(|table| {
        let mut table = table.borrow_mut();
        table.next_handle = table.next_handle.wrapping_add(1).max(1);
        let handle = table.next_handle;
        let snapshot = scene.snapshot();
        table.scenes.insert(handle, scene);
        serde_json::to_string(&CreateResponse {
            handle,
            layer_bindings,
            snapshot,
        })
        .map_err(|error| error.to_string())
    })
}

pub fn advance(handle: u32, tick: u64) -> Result<String, String> {
    with_scene(handle, |scene| {
        serde_json::to_string(&scene.advance_to_tick(tick)).map_err(|error| error.to_string())
    })
}

pub unsafe fn advance_binary(
    handle: u32,
    tick: u64,
    output: *mut u8,
    capacity: usize,
) -> Result<usize, String> {
    with_scene(handle, |scene| {
        let maximum = 40 + scene.layer_count() * 24;
        if output.is_null() || capacity < maximum {
            return Err(format!(
                "core delta buffer too small: capacity={capacity}, required={maximum}"
            ));
        }
        let delta = scene.advance_to_tick(tick);
        let required = 40 + delta.patches.len() * 24;
        let bytes = unsafe { std::slice::from_raw_parts_mut(output, required) };
        bytes.fill(0);
        put_u32(bytes, 0, 0x3144_4341);
        put_u16(bytes, 4, delta.schema_major);
        put_u16(bytes, 6, delta.schema_minor);
        let dirty = (delta.dirty.mask as u32)
            | ((delta.dirty.transform as u32) << 1)
            | ((delta.dirty.material as u32) << 2)
            | ((delta.dirty.layout as u32) << 3)
            | ((delta.dirty.command as u32) << 4)
            | ((delta.dirty.atlas as u32) << 5);
        put_u32(bytes, 8, dirty);
        put_u32(bytes, 12, delta.patches.len() as u32);
        put_u64(bytes, 16, delta.tick);
        put_u64(bytes, 24, delta.base_revision);
        put_u64(bytes, 32, delta.revision);
        for (index, patch) in delta.patches.iter().enumerate() {
            let offset = 40 + index * 24;
            put_u64(bytes, offset, patch.layer_id.0);
            let flags =
                (patch.render_mask.is_some() as u32) | ((patch.transform.is_some() as u32) << 1);
            put_u32(bytes, offset + 8, flags);
            put_u32(
                bytes,
                offset + 12,
                patch.render_mask.unwrap_or(false) as u32,
            );
            let transform = patch.transform.unwrap_or_default();
            put_f32(bytes, offset + 16, transform.dx);
            put_f32(bytes, offset + 20, transform.dy);
        }
        Ok(required)
    })
}

fn put_u16(output: &mut [u8], offset: usize, value: u16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(output: &mut [u8], offset: usize, value: u64) {
    output[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn put_f32(output: &mut [u8], offset: usize, value: f32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub fn set_mask(handle: u32, input: &str) -> Result<String, String> {
    let request: MaskRequest = serde_json::from_str(input)
        .map_err(|error| format!("parse mask request failed: {error}"))?;
    with_scene(handle, |scene| {
        let delta = scene
            .set_render_mask(request.layer_id, request.visible)
            .map_err(|error| error.to_string())?;
        serde_json::to_string(&delta).map_err(|error| error.to_string())
    })
}

pub fn set_masks(handle: u32, input: &str) -> Result<String, String> {
    let request: BulkMaskRequest = serde_json::from_str(input)
        .map_err(|error| format!("parse bulk mask request failed: {error}"))?;
    let overrides = request
        .overrides
        .into_iter()
        .map(|value| (value.layer_id, value.mask_override))
        .collect::<Vec<_>>();
    with_scene(handle, |scene| {
        let delta = scene
            .set_render_mask_overrides(request.expected_layer_table_revision, &overrides)
            .map_err(|error| error.to_string())?;
        serde_json::to_string(&delta).map_err(|error| error.to_string())
    })
}

pub fn set_tab(handle: u32, input: &str) -> Result<String, String> {
    let request: TabRequest = serde_json::from_str(input)
        .map_err(|error| format!("parse tab request failed: {error}"))?;
    with_scene(handle, |scene| {
        let delta = scene
            .set_tab(request.control_id, &request.value)
            .map_err(|error| error.to_string())?;
        serde_json::to_string(&delta).map_err(|error| error.to_string())
    })
}

pub fn scroll(handle: u32, input: &str) -> Result<String, String> {
    let request: ScrollRequest = serde_json::from_str(input)
        .map_err(|error| format!("parse scroll request failed: {error}"))?;
    with_scene(handle, |scene| {
        let delta = match (request.offset, request.delta) {
            (Some(offset), None) => scene.set_scroll_offset(request.control_id, offset),
            (None, Some(delta)) => scene.scroll_by(request.control_id, delta),
            _ => return Err("scroll request requires exactly one of offset or delta".into()),
        }
        .map_err(|error| error.to_string())?;
        serde_json::to_string(&delta).map_err(|error| error.to_string())
    })
}

pub fn dump(handle: u32) -> Result<String, String> {
    with_scene(handle, |scene| {
        serde_json::to_string(&scene.dump()).map_err(|error| error.to_string())
    })
}

pub fn destroy(handle: u32) -> bool {
    SCENES.with(|table| table.borrow_mut().scenes.remove(&handle).is_some())
}

fn with_scene<T>(
    handle: u32,
    call: impl FnOnce(&mut Scene) -> Result<T, String>,
) -> Result<T, String> {
    SCENES.with(|table| {
        let mut table = table.borrow_mut();
        let scene = table
            .scenes
            .get_mut(&handle)
            .ok_or_else(|| format!("unknown core scene handle {handle}"))?;
        call(scene)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn wasm_create_and_dump_preserve_tab_and_scroll_region_bindings() {
        let scene_key = "interaction-contract";
        let layer_key = "component";
        let layer_id = StableId::derive("layer-v1", format!("{scene_key}\0{layer_key}").as_bytes());
        let tab_id = StableId(0x6100);
        let scroll_id = StableId(0x6200);
        let rect = json!({ "x": 0.0, "y": 0.0, "width": 20.0, "height": 20.0 });
        let quad = json!([[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]]);
        let input = json!({
            "sceneKey": scene_key,
            "region": "cn",
            "fontEngineFingerprint": "ft",
            "rasterContract": "sdf",
            "layers": [{
                "sourceKey": layer_key,
                "parentSourceKey": null,
                "kind": "shape",
                "z": 0,
                "authoredVisible": true,
                "sourceContent": "",
                "bounds": rect,
                "quad": quad,
                "matrix": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                "hitGeometry": quad,
                "lineIndent": null
            }],
            "interactionRegions": [
                {
                    "layerSourceKey": layer_key,
                    "ordinal": 0,
                    "role": "rank-tab",
                    "bounds": rect,
                    "quad": quad,
                    "matrix": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                    "hitGeometry": quad,
                    "controlBindings": [{
                        "kind": "tab_option",
                        "control_id": tab_id,
                        "value": "rank"
                    }],
                    "capabilities": ["activate"]
                },
                {
                    "layerSourceKey": layer_key,
                    "ordinal": 1,
                    "role": "rank-list",
                    "bounds": rect,
                    "quad": quad,
                    "matrix": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                    "hitGeometry": quad,
                    "controlBindings": [{
                        "kind": "scroll_content",
                        "control_id": scroll_id
                    }],
                    "capabilities": ["inspect"]
                }
            ],
            "componentControls": [
                {
                    "id": tab_id,
                    "layer_id": layer_id,
                    "role": "mode",
                    "state": { "kind": "tabs", "options": ["rank", "challenge"], "active": "rank" }
                },
                {
                    "id": scroll_id,
                    "layer_id": layer_id,
                    "role": "list",
                    "state": {
                        "kind": "scroll",
                        "offset": 0.0,
                        "min": 0.0,
                        "max": 100.0,
                        "viewport_extent": 100.0,
                        "content_extent": 200.0,
                        "step": 20.0
                    }
                }
            ]
        });
        let created: Value = serde_json::from_str(&create(&input.to_string()).unwrap()).unwrap();
        let handle = created["handle"].as_u64().unwrap() as u32;
        let snapshot_regions = created["snapshot"]["interaction_regions"]
            .as_array()
            .unwrap();
        assert_eq!(
            snapshot_regions[0]["control_bindings"][0]["kind"],
            "tab_option"
        );
        assert_eq!(
            snapshot_regions[1]["control_bindings"][0]["kind"],
            "scroll_content"
        );

        let dumped: Value = serde_json::from_str(&dump(handle).unwrap()).unwrap();
        let dump_regions = dumped["interaction_regions"].as_array().unwrap();
        assert_eq!(
            dump_regions[0]["control_bindings"],
            snapshot_regions[0]["control_bindings"]
        );
        assert_eq!(
            dump_regions[1]["control_bindings"],
            snapshot_regions[1]["control_bindings"]
        );
        assert!(destroy(handle));
    }
}
