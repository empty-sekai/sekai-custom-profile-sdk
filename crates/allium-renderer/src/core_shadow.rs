//! Read-only native adapter for the v0.2 text scene contract.
//!
//! This deliberately does not participate in the production Skia pixel path. It lets server
//! diagnostics and parity gates consume the same dynamic/state contract as browser WASM while
//! text layout migration is still in progress.

use std::collections::BTreeMap;

use allium_renderer_core::{
    AuthoredElementKind, LayerKind, LayerSource, ParameterValue, Quad, Rect, Scene, SceneSource,
    StableId,
};

use crate::masterdata::{MasterData, ResolvedColor};
use crate::transform;
use crate::types::CustomProfileCard;

pub fn build_scene(
    card: &CustomProfileCard,
    md: &MasterData,
    document_key: &str,
    region: &str,
    profile: Option<&crate::profile::ProfileData>,
    locale: &str,
    assets: Option<&crate::assets::AssetStore>,
) -> Result<Scene, String> {
    build_scene_with_resolved(card, md, document_key, region, profile, locale, assets)
        .map(|(scene, _)| scene)
}

#[allow(clippy::too_many_arguments)]
pub fn build_scene_with_resolved(
    card: &CustomProfileCard,
    md: &MasterData,
    document_key: &str,
    region: &str,
    profile: Option<&crate::profile::ProfileData>,
    locale: &str,
    assets: Option<&crate::assets::AssetStore>,
) -> Result<
    (
        Scene,
        allium_renderer_core::profile_scene::ResolvedProfileScene,
    ),
    String,
> {
    let scene_id = StableId::derive("scene-v2", document_key.as_bytes());
    let resolved = crate::semantic_resolve::resolve_card_commands_with_profile(
        card,
        md,
        document_key,
        profile,
        locale,
        assets,
    )
    .map_err(|error| error.to_string())?;
    let scene = Scene::new(SceneSource {
        scene_id,
        region: region.into(),
        font_engine_fingerprint: "native-skia-freetype-semantic-v2".into(),
        raster_contract: "production-sdf-semantic-v2".into(),
        layers: resolved.layers.clone(),
        glyphs: Vec::new(),
        semantic_commands: resolved.commands.clone(),
        interaction_regions: resolved.interaction_regions.clone(),
        component_controls: resolved.controls.clone(),
    })
    .map_err(|error| error.to_string())?;
    Ok((scene, resolved))
}

pub fn build_text_scene(
    card: &CustomProfileCard,
    md: &MasterData,
    document_key: &str,
) -> Result<Scene, allium_renderer_core::CoreError> {
    build_text_scene_with_atlases(card, md, document_key, None)
}

pub fn build_text_scene_with_atlases(
    card: &CustomProfileCard,
    md: &MasterData,
    document_key: &str,
    atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
) -> Result<Scene, allium_renderer_core::CoreError> {
    let scene_id = StableId::derive("scene-v1", document_key.as_bytes());
    let mut layers =
        allium_renderer_core::profile_scene::ordered_profile_elements(card, document_key)
            .into_iter()
            .filter_map(|element| {
                let allium_renderer_core::profile_scene::ProfileElementRef::Text(text) =
                    element.value
                else {
                    return None;
                };
                Some({
                    let source_index = element.source_index;
                    let object = &text.object_data;
                    let (x, y, rotation_deg, scale_x, scale_y) =
                        transform::extract_transform(object);
                    let theta = rotation_deg.to_radians();
                    let point_quad: Quad = [[x, y], [x, y], [x, y], [x, y]];
                    let mut parameters = BTreeMap::new();
                    parameters.insert("font_id".into(), ParameterValue::I64(text.font_id.into()));
                    parameters.insert(
                        "font_family".into(),
                        ParameterValue::Text(md.resolve_font(text.font_id).unwrap_or_default()),
                    );
                    parameters.insert("font_size".into(), ParameterValue::F64(text.size.into()));
                    parameters.insert("color_id".into(), ParameterValue::I64(text.color_id.into()));
                    if let Some(color) = md.resolve_color(text.color_id) {
                        parameters
                            .insert("color".into(), ParameterValue::Color(color_value(color)));
                    }
                    parameters.insert(
                        "outline_color_id".into(),
                        ParameterValue::I64(text.outline_color_id.into()),
                    );
                    if let Some(color) = md.resolve_color(text.outline_color_id) {
                        parameters.insert(
                            "outline_color".into(),
                            ParameterValue::Color(color_value(color)),
                        );
                    }
                    parameters.insert(
                        "outline_width".into(),
                        ParameterValue::F64(text.outline_size.into()),
                    );
                    parameters.insert(
                        "line_spacing".into(),
                        ParameterValue::F64(text.line_spacing.into()),
                    );
                    parameters.insert(
                        "text_type".into(),
                        ParameterValue::I64(text.text_type.into()),
                    );
                    parameters.insert(
                        "geometry_status".into(),
                        ParameterValue::Text("shadow_text_layout_pending".into()),
                    );
                    let line_indent = atlases
                        .and_then(|atlases| {
                            crate::text::line_indent_program_with_atlases(text, md, atlases)
                        })
                        .or_else(|| crate::text::line_indent_program(text, md))
                        .map(|mut source| {
                            source.rotation_deg = rotation_deg;
                            source.scale_x = scale_x;
                            source
                        });
                    LayerSource {
                        id: element.layer_id,
                        parent_id: None,
                        kind: LayerKind::Text,
                        authored_kind: AuthoredElementKind::Text,
                        authored_index: source_index as u32,
                        game_layer: object.layer,
                        z: object.layer,
                        authored_visible: object.visible,
                        source_content: text.text.clone(),
                        resolved_parameters: parameters,
                        bounds: Rect {
                            x,
                            y,
                            width: 0.0,
                            height: 0.0,
                        },
                        quad: point_quad,
                        matrix: [
                            theta.cos() * scale_x,
                            theta.sin() * scale_x,
                            -theta.sin() * scale_y,
                            theta.cos() * scale_y,
                            x,
                            y,
                        ],
                        hit_geometry: point_quad,
                        line_indent,
                    }
                })
            })
            .collect::<Vec<_>>();
    layers.sort_by_key(|layer| layer.z);
    Scene::new(SceneSource {
        scene_id,
        region: "cn".into(),
        font_engine_fingerprint: "native-skia-freetype-shadow-v1".into(),
        raster_contract: "production-sdf-shadow-v1".into(),
        layers,
        glyphs: Vec::new(),
        semantic_commands: Vec::new(),
        interaction_regions: Vec::new(),
        component_controls: Vec::new(),
    })
}

fn color_value(color: ResolvedColor) -> [f32; 4] {
    [
        color.r as f32 / 255.0,
        color.g as f32 / 255.0,
        color.b as f32 / 255.0,
        color.a as f32 / 255.0,
    ]
}
