//! 场景元素渲染模块。

pub mod generals;
#[cfg(feature = "skia-core")]
pub mod honor;
#[cfg(feature = "skia-core")]
pub mod image;
pub mod shape;

/// Builds a Skia face from font bytes the caller supplied.
///
/// The element draw path still rasterizes a few labels through Skia, but it
/// resolves the face the same way the rest of the engine does: from a declared
/// family mapped to a file under the configured font directory. There is no
/// system-font lookup, because a substituted face silently renders text in
/// whatever the host image happens to ship — which is how the card info bar came
/// to draw its level in DejaVu Sans while declaring a game font.
///
/// Returns `None` when the family is not available, so callers skip the label
/// rather than draw it in an unrelated typeface.
#[cfg(feature = "skia-core")]
pub(crate) fn bundled_typeface(family: &str) -> Option<skia_safe::Typeface> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static CACHE: OnceLock<Mutex<HashMap<String, Option<skia_safe::Typeface>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache
        .lock()
        .ok()
        .and_then(|entries| entries.get(family).cloned())
    {
        return cached;
    }
    let resolved = crate::sdf::outline::load_font_bytes_for_family(family)
        .and_then(|bytes| skia_safe::FontMgr::new().new_from_data(bytes.as_slice(), None));
    if resolved.is_none() {
        tracing::warn!(
            font_family = family,
            "declared font family is unavailable; the label is skipped"
        );
    }
    if let Ok(mut entries) = cache.lock() {
        entries.insert(family.to_string(), resolved.clone());
    }
    resolved
}
use crate::types::*;

/// 扁平化后的渲染元素（统一不同类型以便排序）。
#[derive(Debug)]
pub enum RenderElement<'a> {
    Text(&'a TextElement),
    Shape(&'a ShapeElement),
    CardMember(&'a CardMemberElement),
    Stamp(&'a StampElement),
    Other(&'a OtherElement),
    BondsHonor(&'a BondsHonorElement),
    Honor(&'a HonorElement),
    Collection(&'a CollectionElement),
    General(&'a GeneralElement),
    StandMember(&'a StandMemberElement),
    GeneralBackground(&'a GeneralBackgroundElement),
    StoryBackground(&'a StoryBackgroundElement),
}

impl<'a> RenderElement<'a> {
    pub fn object_data(&self) -> &ObjectData {
        match self {
            Self::Text(e) => &e.object_data,
            Self::Shape(e) => &e.object_data,
            Self::CardMember(e) => &e.object_data,
            Self::Stamp(e) => &e.object_data,
            Self::Other(e) => &e.object_data,
            Self::BondsHonor(e) => &e.object_data,
            Self::Honor(e) => &e.object_data,
            Self::Collection(e) => &e.object_data,
            Self::General(e) => &e.object_data,
            Self::StandMember(e) => &e.object_data,
            Self::GeneralBackground(e) => &e.object_data,
            Self::StoryBackground(e) => &e.object_data,
        }
    }

    pub fn layer(&self) -> i32 {
        self.object_data().layer
    }

    pub fn visible(&self) -> bool {
        self.object_data().visible
    }
}

/// 从 CustomProfileCard 提取所有元素，按 layer 升序排序。
pub fn flatten_and_sort(card: &CustomProfileCard) -> Vec<RenderElement<'_>> {
    let total = card.texts.len()
        + card.shapes.len()
        + card.card_members.len()
        + card.stamps.len()
        + card.others.len()
        + card.bonds_honors.len()
        + card.honors.len()
        + card.collections.len()
        + card.generals.len()
        + card.stand_members.len()
        + card.general_backgrounds.len()
        + card.story_backgrounds.len();
    let mut elements: Vec<RenderElement<'_>> = Vec::with_capacity(total);

    for e in &card.texts {
        elements.push(RenderElement::Text(e));
    }
    for e in &card.shapes {
        elements.push(RenderElement::Shape(e));
    }
    for e in &card.card_members {
        elements.push(RenderElement::CardMember(e));
    }
    for e in &card.stamps {
        elements.push(RenderElement::Stamp(e));
    }
    for e in &card.others {
        elements.push(RenderElement::Other(e));
    }
    for e in &card.bonds_honors {
        elements.push(RenderElement::BondsHonor(e));
    }
    for e in &card.honors {
        elements.push(RenderElement::Honor(e));
    }
    for e in &card.collections {
        elements.push(RenderElement::Collection(e));
    }
    for e in &card.generals {
        elements.push(RenderElement::General(e));
    }
    for e in &card.stand_members {
        elements.push(RenderElement::StandMember(e));
    }
    for e in &card.general_backgrounds {
        elements.push(RenderElement::GeneralBackground(e));
    }
    for e in &card.story_backgrounds {
        elements.push(RenderElement::StoryBackground(e));
    }

    elements.sort_by_key(|e| e.layer());
    elements
}

#[cfg(feature = "skia-core")]
pub fn draw_element(
    canvas: &skia_safe::Canvas,
    elem: &RenderElement<'_>,
    md: &crate::masterdata::MasterData,
    assets: Option<&crate::assets::AssetStore>,
    profile: Option<&crate::profile::ProfileData>,
) {
    // 单次调用便利包装：自行构造共享上下文。批量渲染请用 draw_element_on_canvas
    // 并在循环外复用 fallback_assets。
    let fallback_assets = crate::assets::AssetStore::new(1);
    draw_element_on_canvas(
        canvas,
        elem,
        md,
        assets,
        profile,
        &fallback_assets,
        crate::transform::CANVAS_WIDTH,
        crate::transform::CANVAS_HEIGHT,
    );
}

#[cfg(feature = "skia-core")]
#[allow(clippy::too_many_arguments)]
pub fn draw_element_on_canvas(
    canvas: &skia_safe::Canvas,
    elem: &RenderElement<'_>,
    md: &crate::masterdata::MasterData,
    assets: Option<&crate::assets::AssetStore>,
    profile: Option<&crate::profile::ProfileData>,
    fallback_assets: &crate::assets::AssetStore,
    canvas_width: f32,
    canvas_height: f32,
) {
    draw_element_on_canvas_observed(
        canvas,
        elem,
        md,
        assets,
        profile,
        fallback_assets,
        canvas_width,
        canvas_height,
        None,
        SdfObservationMode::RenderAndObserve,
        None,
        None,
    );
}

#[cfg(feature = "skia-core")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SdfObservationMode {
    RenderAndObserve,
    ObserveOnly,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SdfObservationTimings {
    pub text_capture: crate::text::TextSdfCaptureTimings,
}

/// Canvas-free capture of one element's SDF command stream. Only Text and
/// Shape elements produce SDF primitives; the ordered path routes every other
/// element kind through the image compositor before reaching here. The device
/// transform composes the surface origin with the element transform exactly as
/// the canvas stack does, and the parity tests pin both compositions.
#[allow(clippy::too_many_arguments)]
pub(crate) fn capture_element_sdf(
    elem: &RenderElement<'_>,
    md: &crate::masterdata::MasterData,
    assets: Option<&crate::assets::AssetStore>,
    canvas_width: f32,
    canvas_height: f32,
    origin: [f32; 2],
    text_atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
    text_observer: Option<
        &mut dyn FnMut(Result<crate::text::ResolvedTextSdfGlyph, crate::text::TextSdfCaptureError>),
    >,
    shape_observer: Option<
        &mut dyn FnMut(
            Result<
                crate::elements::shape::ResolvedShapeSdfCommand,
                crate::elements::shape::ShapeSdfCaptureError,
            >,
        ),
    >,
) -> SdfObservationTimings {
    let mut observation_timings = SdfObservationTimings::default();
    let device = concat_affine(
        [1.0, 0.0, 0.0, 1.0, origin[0], origin[1]],
        element_device_affine(elem.object_data(), canvas_width, canvas_height),
    );
    match elem {
        RenderElement::Text(e) => {
            if let Some(observer) = text_observer {
                observation_timings.text_capture = crate::text::capture_text_sdf_from_affine(
                    scale_affine(device, crate::text::TEXT_SCALE),
                    e,
                    md,
                    text_atlases,
                    observer,
                );
            }
        }
        RenderElement::Shape(e) => {
            if let Some(observer) = shape_observer {
                crate::elements::shape::capture_shape_sdf_from_affine(
                    device, e, md, assets, observer,
                );
            }
        }
        _ => {}
    }
    observation_timings
}

/// Device transform an element's own drawing sits under, as an affine in
/// `[sx, ky, kx, sy, tx, ty]` layout.
///
/// This is the translate/rotate/scale sequence the element walker applies, in
/// the same order and with the same skipping thresholds, so a capture that
/// never touches a canvas resolves the identical matrix. A parity test pins it
/// against the canvas composition.
pub(crate) fn element_device_affine(
    obj: &crate::types::ObjectData,
    canvas_width: f32,
    canvas_height: f32,
) -> [f32; 6] {
    let (x, y, angle, sx, sy) =
        crate::transform::extract_transform_for_canvas(obj, canvas_width, canvas_height);
    let mut affine = [1.0, 0.0, 0.0, 1.0, x, y];
    if angle.abs() > 0.01 {
        let radians = angle.to_radians();
        let (sin, cos) = (radians.sin(), radians.cos());
        affine = concat_affine(affine, [cos, sin, -sin, cos, 0.0, 0.0]);
    }
    if (sx - 1.0).abs() > 0.001 || (sy - 1.0).abs() > 0.001 {
        affine = concat_affine(affine, [sx, 0.0, 0.0, sy, 0.0, 0.0]);
    }
    affine
}

/// Scales an affine by a uniform factor, the way `Canvas::scale` composes.
pub(crate) fn scale_affine(affine: [f32; 6], factor: f32) -> [f32; 6] {
    concat_affine(affine, [factor, 0.0, 0.0, factor, 0.0, 0.0])
}

/// `base * local` for affine matrices, with the translation terms folded in the
/// same order the canvas concatenation uses.
fn concat_affine(base: [f32; 6], local: [f32; 6]) -> [f32; 6] {
    [
        base[0] * local[0] + base[2] * local[1],
        base[1] * local[0] + base[3] * local[1],
        base[0] * local[2] + base[2] * local[3],
        base[1] * local[2] + base[3] * local[3],
        base[0] * local[4] + (base[2] * local[5] + base[4]),
        base[1] * local[4] + (base[3] * local[5] + base[5]),
    ]
}

#[cfg(feature = "skia-core")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_element_on_canvas_observed(
    canvas: &skia_safe::Canvas,
    elem: &RenderElement<'_>,
    md: &crate::masterdata::MasterData,
    assets: Option<&crate::assets::AssetStore>,
    profile: Option<&crate::profile::ProfileData>,
    fallback_assets: &crate::assets::AssetStore,
    canvas_width: f32,
    canvas_height: f32,
    text_atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
    observation_mode: SdfObservationMode,
    text_observer: Option<
        &mut dyn FnMut(Result<crate::text::ResolvedTextSdfGlyph, crate::text::TextSdfCaptureError>),
    >,
    shape_observer: Option<
        &mut dyn FnMut(
            Result<
                crate::elements::shape::ResolvedShapeSdfCommand,
                crate::elements::shape::ShapeSdfCaptureError,
            >,
        ),
    >,
) -> SdfObservationTimings {
    use crate::context::RenderContext;
    use crate::elements::shape::{capture_shape_sdf, draw_shape, draw_shape_observed};
    use crate::text::{capture_text_sdf, draw_text, TEXT_SCALE};
    use crate::transform;
    use crate::widgets::adapters::card_member::CardMemberWidget;
    use crate::widgets::adapters::general::GeneralWidget;
    use crate::widgets::adapters::honor::{BondsHonorWidget, HonorWidget};
    use crate::widgets::adapters::simple_asset::{
        CollectionWidget, GeneralBgWidget, OtherWidget, StampWidget, StandMemberWidget,
        StoryBgWidget,
    };
    use crate::widgets::Widget;
    use skia_safe::Point;

    let mut observation_timings = SdfObservationTimings::default();

    let obj = elem.object_data();
    let (x, y, angle, sx, sy) =
        transform::extract_transform_for_canvas(obj, canvas_width, canvas_height);

    canvas.save();
    canvas.translate(Point::new(x, y));
    if angle.abs() > 0.01 {
        canvas.rotate(angle, None);
    }
    if (sx - 1.0).abs() > 0.001 || (sy - 1.0).abs() > 0.001 {
        canvas.scale((sx, sy));
    }

    match elem {
        RenderElement::Text(e) => {
            tracing::debug!(
                x = x, y = y, angle = angle, sx = sx, sy = sy,
                text = %e.text.chars().take(20).collect::<String>(),
                "Text 元素坐标"
            );
            canvas.scale((TEXT_SCALE, TEXT_SCALE));
            if let Some(observer) = text_observer {
                match observation_mode {
                    SdfObservationMode::RenderAndObserve => {
                        crate::text::draw_text_observed(canvas, e, md, observer)
                    }
                    SdfObservationMode::ObserveOnly => {
                        observation_timings.text_capture =
                            capture_text_sdf(canvas, e, md, text_atlases, observer)
                    }
                }
            } else {
                draw_text(canvas, e, md);
            }
        }
        RenderElement::Shape(e) => {
            if let Some(observer) = shape_observer {
                match observation_mode {
                    SdfObservationMode::RenderAndObserve => {
                        draw_shape_observed(canvas, e, md, assets, Some(observer))
                    }
                    SdfObservationMode::ObserveOnly => {
                        capture_shape_sdf(canvas, e, md, assets, observer)
                    }
                }
            } else {
                draw_shape(canvas, e, md, assets);
            }
        }
        RenderElement::CardMember(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let mut ctx = RenderContext::new(asset_store).with_masterdata(md);
            if let Some(profile) = profile {
                ctx = ctx.with_profile(profile);
            }
            if let Some(widget) = CardMemberWidget::from_element(e, &ctx) {
                widget.draw(canvas, 0.0, 0.0, &ctx);
            }
        }
        RenderElement::Stamp(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let ctx = RenderContext::new(asset_store).with_masterdata(md);
            StampWidget::from_element(e, &ctx).draw(canvas, 0.0, 0.0, &ctx);
        }
        RenderElement::Other(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let ctx = RenderContext::new(asset_store).with_masterdata(md);
            if let Some(widget) = OtherWidget::from_element(e, &ctx) {
                widget.draw(canvas, 0.0, 0.0, &ctx);
            }
        }
        RenderElement::BondsHonor(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let ctx = RenderContext::new(asset_store).with_masterdata(md);
            BondsHonorWidget::from_element(e).draw(canvas, 0.0, 0.0, &ctx);
        }
        RenderElement::Honor(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let mut ctx = RenderContext::new(asset_store).with_masterdata(md);
            if let Some(profile) = profile {
                ctx = ctx.with_profile(profile);
            }
            HonorWidget::from_element(e).draw(canvas, 0.0, 0.0, &ctx);
        }
        RenderElement::Collection(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let ctx = RenderContext::new(asset_store).with_masterdata(md);
            if let Some(widget) = CollectionWidget::from_element(e, &ctx) {
                widget.draw(canvas, 0.0, 0.0, &ctx);
            }
        }
        RenderElement::General(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let mut ctx = RenderContext::new(asset_store).with_masterdata(md);
            if let Some(profile) = profile {
                ctx = ctx.with_profile(profile);
            }
            GeneralWidget::from_element(e).draw(canvas, 0.0, 0.0, &ctx);
        }
        RenderElement::StandMember(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let ctx = RenderContext::new(asset_store).with_masterdata(md);
            if let Some(widget) = StandMemberWidget::from_element(e, &ctx) {
                widget.draw(canvas, 0.0, 0.0, &ctx);
            }
        }
        RenderElement::GeneralBackground(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let ctx = RenderContext::new(asset_store).with_masterdata(md);
            if let Some(widget) = GeneralBgWidget::from_element(e, &ctx) {
                widget.draw(canvas, 0.0, 0.0, &ctx);
            }
        }
        RenderElement::StoryBackground(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let ctx = RenderContext::new(asset_store).with_masterdata(md);
            if let Some(widget) = StoryBgWidget::from_element(e, &ctx) {
                widget.draw(canvas, 0.0, 0.0, &ctx);
            }
        }
    }

    canvas.restore();
    observation_timings
}

#[cfg(all(test, feature = "skia-core"))]
mod tests {
    use crate::types::{ObjectData, Quaternion, Vec3};

    fn object(position: (f32, f32), rotation: &Quaternion, scale: (f32, f32)) -> ObjectData {
        ObjectData {
            layer: 0,
            lock: false,
            position: Vec3 {
                x: position.0,
                y: position.1,
                z: 0.0,
            },
            rotation: Quaternion {
                w: rotation.w,
                x: rotation.x,
                y: rotation.y,
                z: rotation.z,
            },
            scale: Vec3 {
                x: scale.0,
                y: scale.1,
                z: 1.0,
            },
            visible: true,
        }
    }

    /// The canvas-free element transform must equal the canvas composition it
    /// replaces, bit for bit, including the thresholds that skip a rotation or
    /// a scale entirely.
    #[test]
    fn the_element_affine_matches_the_canvas_transform_stack() {
        let identity = Quaternion {
            w: 1.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        // 30 degrees about Z, and a tiny rotation below the skip threshold.
        let rotated = Quaternion {
            w: 0.9659258,
            x: 0.0,
            y: 0.0,
            z: 0.258819,
        };
        let barely = Quaternion {
            w: 0.99999996,
            x: 0.0,
            y: 0.0,
            z: 0.00002,
        };
        let cases = [
            object((0.0, 0.0), &identity, (1.0, 1.0)),
            object((120.5, -80.25), &identity, (1.0, 1.0)),
            object((120.5, -80.25), &identity, (1.5, 0.75)),
            object((-33.0, 44.0), &rotated, (1.0, 1.0)),
            object((-33.0, 44.0), &rotated, (2.25, 1.125)),
            object((7.5, 9.5), &barely, (1.0005, 0.9995)),
            object((7.5, 9.5), &barely, (3.0, 3.0)),
        ];
        let (canvas_width, canvas_height) = (
            crate::transform::CANVAS_WIDTH as f32,
            crate::transform::CANVAS_HEIGHT as f32,
        );
        for (index, obj) in cases.iter().enumerate() {
            let (x, y, angle, sx, sy) =
                crate::transform::extract_transform_for_canvas(obj, canvas_width, canvas_height);

            let mut surface = skia_safe::surfaces::null((64, 64)).expect("null surface");
            let canvas = surface.canvas();
            canvas.translate(skia_safe::Point::new(x, y));
            if angle.abs() > 0.01 {
                canvas.rotate(angle, None);
            }
            if (sx - 1.0).abs() > 0.001 || (sy - 1.0).abs() > 0.001 {
                canvas.scale((sx, sy));
            }
            let expected = canvas
                .local_to_device_as_3x3()
                .to_affine()
                .expect("affine canvas transform");

            let actual = super::element_device_affine(obj, canvas_width, canvas_height);
            assert_eq!(actual, expected, "case {index}");

            // The text path scales again on top; that must compose the same way.
            canvas.scale((crate::text::TEXT_SCALE, crate::text::TEXT_SCALE));
            let expected_scaled = canvas
                .local_to_device_as_3x3()
                .to_affine()
                .expect("affine canvas transform");
            assert_eq!(
                super::scale_affine(actual, crate::text::TEXT_SCALE),
                expected_scaled,
                "case {index} scaled"
            );
        }
    }
}
