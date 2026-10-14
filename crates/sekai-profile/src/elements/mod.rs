//! Authored card elements and their layer order.

pub mod generals;
pub mod shape;

use crate::types::*;

/// One authored element of a card, with every element kind under one type so
/// the elements can be sorted together.
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
    CharacterIcon(&'a CharacterIconElement),
    Material(&'a MaterialElement),
    UserInterfaceIcon(&'a UserInterfaceIconElement),
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
            Self::CharacterIcon(e) => &e.object_data,
            Self::Material(e) => &e.object_data,
            Self::UserInterfaceIcon(e) => &e.object_data,
        }
    }

    pub fn layer(&self) -> i32 {
        self.object_data().layer
    }

    pub fn visible(&self) -> bool {
        self.object_data().visible
    }
}

/// Collects every element of `card` and sorts them by ascending layer. The
/// sort is stable: elements on the same layer keep the order of their kind's
/// list in the card, kinds in the order of [`RenderElement`]'s variants.
pub fn flatten_and_sort(card: &CustomProfileCard) -> Vec<RenderElement<'_>> {
    let mut elements: Vec<RenderElement<'_>> = Vec::with_capacity(card.element_count());

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
    for e in &card.character_icons {
        elements.push(RenderElement::CharacterIcon(e));
    }
    for e in &card.materials {
        elements.push(RenderElement::Material(e));
    }
    for e in &card.user_interface_icons {
        elements.push(RenderElement::UserInterfaceIcon(e));
    }

    elements.sort_by_key(|e| e.layer());
    elements
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SdfObservationTimings {
    pub text_capture: crate::text::TextSdfCaptureTimings,
}

/// Receives the SDF primitives one element capture produces: the glyphs of a
/// text element or the command of a shape element. An element of the other
/// kind produces nothing.
pub(crate) enum SdfCaptureObserver<'a> {
    Text(
        &'a mut dyn FnMut(
            Result<crate::text::ResolvedTextSdfGlyph, crate::text::TextSdfCaptureError>,
        ),
    ),
    Shape(
        &'a mut dyn FnMut(
            Result<
                crate::elements::shape::ResolvedShapeSdfCommand,
                crate::elements::shape::ShapeSdfCaptureError,
            >,
        ),
    ),
}

/// Capture of one element's SDF command stream. Only Text and Shape elements
/// produce SDF primitives; the ordered path routes every other element kind
/// through the image compositor before reaching here.
///
/// `canvas` is the card canvas on the capture surface: element positions
/// resolve against its size, and its origin offsets the device transform,
/// which is that offset composed with [`element_device_affine`].
pub(crate) fn capture_element_sdf(
    elem: &RenderElement<'_>,
    md: &crate::masterdata::MasterData,
    assets: Option<&crate::assets::AssetStore>,
    canvas: sekai_profile_core::Rect,
    text_atlases: Option<&crate::sdf::atlas::MappedSdfAtlasSet>,
    observer: SdfCaptureObserver<'_>,
) -> SdfObservationTimings {
    let mut observation_timings = SdfObservationTimings::default();
    let device = concat_affine(
        [1.0, 0.0, 0.0, 1.0, canvas.x, canvas.y],
        element_device_affine(elem.object_data(), canvas.width, canvas.height),
    );
    match (elem, observer) {
        (RenderElement::Text(e), SdfCaptureObserver::Text(observer)) => {
            observation_timings.text_capture = crate::text::capture_text_sdf_from_affine(
                scale_affine(device, crate::text::TEXT_SCALE),
                e,
                md,
                text_atlases,
                observer,
            );
        }
        (RenderElement::Shape(e), SdfCaptureObserver::Shape(observer)) => {
            crate::elements::shape::capture_shape_sdf_from_affine(device, e, md, assets, observer);
        }
        _ => {}
    }
    observation_timings
}

/// Device transform an element's own drawing sits under, as an affine in
/// `[sx, ky, kx, sy, tx, ty]` layout.
///
/// The element position is applied first, then its rotation when the angle
/// exceeds 0.01 degrees, then its scale when either axis differs from 1 by more
/// than 0.001. Rotations and scales inside those thresholds are skipped
/// entirely, so the result carries no near-identity rounding noise.
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

/// Scales an affine by a uniform factor applied in local space (`affine · S`).
pub(crate) fn scale_affine(affine: [f32; 6], factor: f32) -> [f32; 6] {
    concat_affine(affine, [factor, 0.0, 0.0, factor, 0.0, 0.0])
}

/// `base * local` for affine matrices in `[sx, ky, kx, sy, tx, ty]` layout. Each
/// translation term is folded as `x + (y + t)`.
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

#[cfg(test)]
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

    /// The element transform composes translate, rotate and scale in that
    /// order, skips a rotation or a scale inside its threshold, and the text
    /// path's extra scale composes on top. The pinned values are exact bit
    /// patterns.
    #[test]
    fn the_element_affine_composes_translate_rotate_and_scale() {
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
        // (object, element affine, element affine scaled by TEXT_SCALE)
        let cases: [(ObjectData, [f32; 6], [f32; 6]); 7] = [
            (
                object((0.0, 0.0), &identity, (1.0, 1.0)),
                [1.0, 0.0, 0.0, 1.0, 915.0, 406.0],
                [2.0, 0.0, 0.0, 2.0, 915.0, 406.0],
            ),
            (
                object((120.5, -80.25), &identity, (1.0, 1.0)),
                [1.0, 0.0, 0.0, 1.0, 1035.5, 486.25],
                [2.0, 0.0, 0.0, 2.0, 1035.5, 486.25],
            ),
            (
                object((120.5, -80.25), &identity, (1.5, 0.75)),
                [1.5, 0.0, 0.0, 0.75, 1035.5, 486.25],
                [3.0, 0.0, 0.0, 1.5, 1035.5, 486.25],
            ),
            (
                object((-33.0, 44.0), &rotated, (1.0, 1.0)),
                [
                    0.86602545,
                    -0.49999997,
                    0.49999997,
                    0.86602545,
                    882.0,
                    362.0,
                ],
                [1.7320509, -0.99999994, 0.99999994, 1.7320509, 882.0, 362.0],
            ),
            (
                object((-33.0, 44.0), &rotated, (2.25, 1.125)),
                [1.9485573, -1.1249999, 0.56249994, 0.9742786, 882.0, 362.0],
                [3.8971145, -2.2499998, 1.1249999, 1.9485573, 882.0, 362.0],
            ),
            (
                object((7.5, 9.5), &barely, (1.0005, 0.9995)),
                [1.0, 0.0, 0.0, 1.0, 922.5, 396.5],
                [2.0, 0.0, 0.0, 2.0, 922.5, 396.5],
            ),
            (
                object((7.5, 9.5), &barely, (3.0, 3.0)),
                [3.0, 0.0, 0.0, 3.0, 922.5, 396.5],
                [6.0, 0.0, 0.0, 6.0, 922.5, 396.5],
            ),
        ];
        let (canvas_width, canvas_height) = (
            crate::transform::CANVAS_WIDTH,
            crate::transform::CANVAS_HEIGHT,
        );
        for (index, (obj, expected, expected_scaled)) in cases.iter().enumerate() {
            let actual = super::element_device_affine(obj, canvas_width, canvas_height);
            assert_eq!(
                actual.map(f32::to_bits),
                expected.map(f32::to_bits),
                "case {index}: {actual:?}"
            );
            let scaled = super::scale_affine(actual, crate::text::TEXT_SCALE);
            assert_eq!(
                scaled.map(f32::to_bits),
                expected_scaled.map(f32::to_bits),
                "case {index} scaled: {scaled:?}"
            );
        }
    }
}
