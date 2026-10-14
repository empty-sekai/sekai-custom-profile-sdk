//! Authored elements: the semantic scene a card resolves to, and drawing its
//! image elements through the profile compositor.

use crate::assets::AssetStore;
use crate::masterdata::MasterData;
use crate::profile_compositor::{CompositorSources, RgbaSurface};
use crate::render_config::Executor;
use crate::telemetry::CommandKind;
use crate::types::CustomProfileCard;

/// Resolves the semantic scene a native single-page render draws. Fixed General
/// captions follow the region the master data belongs to.
pub(super) fn resolve_native_profile_scene(
    card: &CustomProfileCard,
    md: &MasterData,
    profile: Option<&crate::profile::ProfileData>,
    assets: Option<&AssetStore>,
    render_objects: Option<&crate::render_object::MappedRenderObjectStore>,
) -> Result<crate::semantic_resolve::ResolvedCardCommands, crate::semantic_resolve::ResolveError> {
    crate::semantic_resolve::resolve_card_commands_with_resources(
        card,
        md,
        "native:ordered-profile-backend",
        profile,
        md.region().as_str(),
        crate::semantic_resolve::ResolveResourceContext {
            assets,
            render_objects,
            catalog_lookup_ns: None,
        },
    )
}

/// Composites one authored element into `surface`, whose origin sits at
/// `canvas_origin` on the profile canvas.
pub(super) fn render_authored_image_into_pixels(
    scene: &sekai_profile_core::profile_scene::ResolvedProfileScene,
    sources: CompositorSources<'_>,
    authored_kind: sekai_profile_core::AuthoredElementKind,
    authored_index: u32,
    surface: RgbaSurface<'_>,
    canvas_origin: [f32; 2],
    image_executor: Executor,
) -> Result<crate::profile_compositor::ProfileCompositorStats, String> {
    let [canvas_origin_x, canvas_origin_y] = canvas_origin;
    let translated_scene;
    let scene = if canvas_origin_x != 0.0 || canvas_origin_y != 0.0 {
        translated_scene = {
            let mut scene = scene.clone();
            for layer in &mut scene.layers {
                if layer.authored_kind == authored_kind && layer.authored_index == authored_index {
                    layer.matrix[4] += canvas_origin_x;
                    layer.matrix[5] += canvas_origin_y;
                }
            }
            scene
        };
        &translated_scene
    } else {
        scene
    };
    match image_executor {
        Executor::Simd => crate::profile_compositor::render_authored_profile_into_simd(
            scene,
            sources,
            authored_kind,
            authored_index,
            surface,
        ),
        Executor::Scalar => crate::profile_compositor::render_authored_profile_into_scalar(
            scene,
            sources,
            authored_kind,
            authored_index,
            surface,
        ),
    }
    .map_err(|error| error.to_string())
}

pub(super) fn render_element_authored_identity(
    card: &CustomProfileCard,
    element: &crate::elements::RenderElement<'_>,
) -> Option<(sekai_profile_core::AuthoredElementKind, u32)> {
    use sekai_profile_core::AuthoredElementKind;

    fn index_of<T>(items: &[T], target: &T) -> Option<u32> {
        items
            .iter()
            .position(|candidate| std::ptr::eq(candidate, target))
            .and_then(|index| u32::try_from(index).ok())
    }

    match element {
        crate::elements::RenderElement::Text(value) => {
            index_of(&card.texts, value).map(|index| (AuthoredElementKind::Text, index))
        }
        crate::elements::RenderElement::Shape(value) => {
            index_of(&card.shapes, value).map(|index| (AuthoredElementKind::Shape, index))
        }
        crate::elements::RenderElement::CardMember(value) => index_of(&card.card_members, value)
            .map(|index| (AuthoredElementKind::CardMember, index)),
        crate::elements::RenderElement::Stamp(value) => {
            index_of(&card.stamps, value).map(|index| (AuthoredElementKind::Stamp, index))
        }
        crate::elements::RenderElement::Other(value) => {
            index_of(&card.others, value).map(|index| (AuthoredElementKind::Other, index))
        }
        crate::elements::RenderElement::BondsHonor(value) => index_of(&card.bonds_honors, value)
            .map(|index| (AuthoredElementKind::BondsHonor, index)),
        crate::elements::RenderElement::Honor(value) => {
            index_of(&card.honors, value).map(|index| (AuthoredElementKind::Honor, index))
        }
        crate::elements::RenderElement::Collection(value) => {
            index_of(&card.collections, value).map(|index| (AuthoredElementKind::Collection, index))
        }
        crate::elements::RenderElement::General(value) => {
            index_of(&card.generals, value).map(|index| (AuthoredElementKind::General, index))
        }
        crate::elements::RenderElement::StandMember(value) => index_of(&card.stand_members, value)
            .map(|index| (AuthoredElementKind::StandMember, index)),
        crate::elements::RenderElement::GeneralBackground(value) => {
            index_of(&card.general_backgrounds, value)
                .map(|index| (AuthoredElementKind::GeneralBackground, index))
        }
        crate::elements::RenderElement::StoryBackground(value) => {
            index_of(&card.story_backgrounds, value)
                .map(|index| (AuthoredElementKind::StoryBackground, index))
        }
        crate::elements::RenderElement::CharacterIcon(value) => {
            index_of(&card.character_icons, value)
                .map(|index| (AuthoredElementKind::CharacterIcon, index))
        }
        crate::elements::RenderElement::Material(value) => {
            index_of(&card.materials, value).map(|index| (AuthoredElementKind::Material, index))
        }
        crate::elements::RenderElement::UserInterfaceIcon(value) => {
            index_of(&card.user_interface_icons, value)
                .map(|index| (AuthoredElementKind::UserInterfaceIcon, index))
        }
    }
}

pub(super) fn profile_command_kind(element: &crate::elements::RenderElement<'_>) -> CommandKind {
    match element {
        crate::elements::RenderElement::Text(_) => CommandKind::Text,
        crate::elements::RenderElement::Shape(_) => CommandKind::Shape,
        _ => CommandKind::Image,
    }
}
