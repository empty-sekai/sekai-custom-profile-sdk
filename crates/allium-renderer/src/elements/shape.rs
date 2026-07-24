use crate::assets::AssetStore;
use crate::masterdata::{MasterData, ResolvedColor};
use crate::sdf::shape::ShapeSdfMaterial;
use crate::sdf::tile::{Affine2, Point2, SdfCommandBuildError, SdfDrawCommand};
use crate::types::ShapeElement;
use sha2::{Digest, Sha256};
use skia_safe::{Canvas, Color4f, Paint, PaintStyle, Rect};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedShapeSdfCommand {
    pub shape_id: i32,
    pub asset_key: String,
    pub source_size: [u32; 2],
    pub source_rg8_sha256: String,
    pub quad: [Point2; 4],
    pub material: ShapeSdfMaterial,
}

impl ResolvedShapeSdfCommand {
    pub(crate) fn to_sdf_command(
        &self,
        atlas: &crate::sdf::shape_atlas::MappedShapeSdfAtlas,
        atlas_set: u16,
    ) -> Result<SdfDrawCommand, ShapeSdfCommandError> {
        let entry = atlas
            .shape(self.shape_id)
            .ok_or(ShapeSdfCommandError::MissingShape {
                shape_id: self.shape_id,
            })?;
        self.to_sdf_command_from_entry(entry, atlas_set)
    }

    fn to_sdf_command_from_entry(
        &self,
        entry: &crate::sdf::shape_atlas::ShapeSdfAtlasEntry,
        atlas_set: u16,
    ) -> Result<SdfDrawCommand, ShapeSdfCommandError> {
        if entry.asset_key != self.asset_key {
            return Err(ShapeSdfCommandError::AssetKeyMismatch {
                shape_id: self.shape_id,
                captured: self.asset_key.clone(),
                atlas: entry.asset_key.clone(),
            });
        }
        if entry.source_size != self.source_size {
            return Err(ShapeSdfCommandError::SourceSizeMismatch {
                shape_id: self.shape_id,
                captured: self.source_size,
                atlas: entry.source_size,
            });
        }
        if entry.source_rg8_sha256 != self.source_rg8_sha256 {
            return Err(ShapeSdfCommandError::SourceContentMismatch {
                shape_id: self.shape_id,
                captured: self.source_rg8_sha256.clone(),
                atlas: entry.source_rg8_sha256.clone(),
            });
        }
        SdfDrawCommand::from_shape_atlas(atlas_set, entry, self.quad, self.material)
            .map_err(ShapeSdfCommandError::Placement)
    }
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub(crate) enum ShapeSdfCommandError {
    #[error("shape atlas does not contain shape id {shape_id}")]
    MissingShape { shape_id: i32 },
    #[error("shape {shape_id} asset key mismatch: captured {captured}, atlas {atlas}")]
    AssetKeyMismatch {
        shape_id: i32,
        captured: String,
        atlas: String,
    },
    #[error("shape {shape_id} source size mismatch: captured {captured:?}, atlas {atlas:?}")]
    SourceSizeMismatch {
        shape_id: i32,
        captured: [u32; 2],
        atlas: [u32; 2],
    },
    #[error("shape {shape_id} decoded RG8 identity mismatch: captured {captured}, atlas {atlas}")]
    SourceContentMismatch {
        shape_id: i32,
        captured: String,
        atlas: String,
    },
    #[error("invalid shape placement: {0}")]
    Placement(#[from] SdfCommandBuildError),
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub(crate) enum ShapeSdfCaptureError {
    #[error("shape id {shape_id} has no MasterData resource identity")]
    MissingResource { shape_id: i32 },
    #[error("shape asset {asset_key} is unavailable; legacy analytic fallback was used")]
    MissingAsset { asset_key: String },
    #[error("shape asset {asset_key} has invalid dimensions {width}x{height}")]
    InvalidDimensions {
        asset_key: String,
        width: i32,
        height: i32,
    },
    #[error("shape transform contains perspective")]
    PerspectiveTransform,
    #[error("shape asset {asset_key} pixels could not be read")]
    ReadPixels { asset_key: String },
}

fn sdf_mask_alpha(red: u8, source_alpha: u8, threshold: f32) -> u8 {
    let threshold = threshold * 255.0;
    let sharp = 1.5_f32;
    let coverage = ((red as f32 - threshold + sharp) / (2.0 * sharp)).clamp(0.0, 1.0);
    (coverage * source_alpha as f32).round() as u8
}

#[allow(clippy::too_many_arguments)]
fn resolve_shape_sdf_command(
    canvas: &Canvas,
    shape: &ShapeElement,
    asset_key: &str,
    width: i32,
    height: i32,
    face_color: ResolvedColor,
    outline_color: ResolvedColor,
    dst: Rect,
    source_rg8_sha256: String,
) -> Result<ResolvedShapeSdfCommand, ShapeSdfCaptureError> {
    let source_size = [
        u32::try_from(width).map_err(|_| ShapeSdfCaptureError::InvalidDimensions {
            asset_key: asset_key.to_string(),
            width,
            height,
        })?,
        u32::try_from(height).map_err(|_| ShapeSdfCaptureError::InvalidDimensions {
            asset_key: asset_key.to_string(),
            width,
            height,
        })?,
    ];
    if source_size.contains(&0) {
        return Err(ShapeSdfCaptureError::InvalidDimensions {
            asset_key: asset_key.to_string(),
            width,
            height,
        });
    }
    let affine = canvas
        .local_to_device_as_3x3()
        .to_affine()
        .ok_or(ShapeSdfCaptureError::PerspectiveTransform)?;
    let local_to_device = Affine2 {
        scale_x: affine[0],
        skew_y: affine[1],
        skew_x: affine[2],
        scale_y: affine[3],
        translate_x: affine[4],
        translate_y: affine[5],
    };
    let quad = [
        Point2::new(dst.left, dst.top),
        Point2::new(dst.right, dst.top),
        Point2::new(dst.right, dst.bottom),
        Point2::new(dst.left, dst.bottom),
    ]
    .map(|point| local_to_device.map_point(point));
    let rgb = |color: ResolvedColor| {
        [
            f32::from(color.r) / 255.0,
            f32::from(color.g) / 255.0,
            f32::from(color.b) / 255.0,
        ]
    };
    let layer_alpha = |alpha: f32| ((alpha * 255.0) as u32).min(255) as f32 / 255.0;
    Ok(ResolvedShapeSdfCommand {
        shape_id: shape.id,
        asset_key: asset_key.to_string(),
        source_size,
        source_rg8_sha256,
        quad,
        material: ShapeSdfMaterial::from_profile_values(
            rgb(face_color),
            layer_alpha(shape.alpha),
            rgb(outline_color),
            if shape.outline_size > 0.01 {
                layer_alpha(shape.outline_alpha)
            } else {
                0.0
            },
            shape.outline_size,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_distance_field_texels_never_produce_shape_coverage() {
        assert_eq!(sdf_mask_alpha(255, 0, 0.5), 0);
        assert_eq!(sdf_mask_alpha(255, 128, 0.5), 128);
        assert_eq!(sdf_mask_alpha(255, 255, 0.5), 255);
    }

    fn captured_shape() -> ResolvedShapeSdfCommand {
        ResolvedShapeSdfCommand {
            shape_id: 7,
            asset_key: "custom_profile/shape/star".into(),
            source_size: [16, 8],
            source_rg8_sha256: "11".repeat(32),
            quad: [
                Point2::new(1.0, 2.0),
                Point2::new(17.0, 2.0),
                Point2::new(17.0, 10.0),
                Point2::new(1.0, 10.0),
            ],
            material: ShapeSdfMaterial::from_profile_values(
                [1.0, 0.0, 0.0],
                0.8,
                [0.0, 0.0, 1.0],
                0.5,
                0.2,
            ),
        }
    }

    fn atlas_entry() -> crate::sdf::shape_atlas::ShapeSdfAtlasEntry {
        crate::sdf::shape_atlas::ShapeSdfAtlasEntry {
            shape_id: 7,
            asset_key: "custom_profile/shape/star".into(),
            source_sha256: "22".repeat(32),
            source_rg8_sha256: "11".repeat(32),
            page: 3,
            rect: [4, 5, 16, 8],
            source_size: [16, 8],
        }
    }

    #[test]
    fn captured_shape_maps_typed_atlas_command_without_relayout() {
        let command = captured_shape()
            .to_sdf_command_from_entry(&atlas_entry(), 4)
            .expect("valid shape command");
        assert_eq!(command.kind, crate::sdf::tile::SdfPrimitiveKind::Shape);
        assert_eq!(command.atlas_set, 4);
        assert_eq!(command.atlas_page, 3);
        assert_eq!(command.atlas_rect, [4, 5, 16, 8]);
        assert_eq!(command.quad, captured_shape().quad);
    }

    #[test]
    fn captured_shape_rejects_decoded_source_identity_mismatch() {
        let mut entry = atlas_entry();
        entry.source_rg8_sha256 = "33".repeat(32);
        assert!(matches!(
            captured_shape().to_sdf_command_from_entry(&entry, 4),
            Err(ShapeSdfCommandError::SourceContentMismatch { shape_id: 7, .. })
        ));
    }
}

/// 绘制图形元素。
pub fn draw_shape(
    canvas: &Canvas,
    shape: &ShapeElement,
    md: &MasterData,
    assets: Option<&AssetStore>,
) {
    draw_shape_observed(canvas, shape, md, assets, None);
}

pub(crate) fn draw_shape_observed(
    canvas: &Canvas,
    shape: &ShapeElement,
    md: &MasterData,
    assets: Option<&AssetStore>,
    observer: Option<&mut dyn FnMut(Result<ResolvedShapeSdfCommand, ShapeSdfCaptureError>)>,
) {
    draw_shape_with_observer(canvas, shape, md, assets, observer, true);
}

pub(crate) fn capture_shape_sdf(
    canvas: &Canvas,
    shape: &ShapeElement,
    md: &MasterData,
    assets: Option<&AssetStore>,
    observer: &mut dyn FnMut(Result<ResolvedShapeSdfCommand, ShapeSdfCaptureError>),
) {
    draw_shape_with_observer(canvas, shape, md, assets, Some(observer), false);
}

fn draw_shape_with_observer(
    canvas: &Canvas,
    shape: &ShapeElement,
    md: &MasterData,
    assets: Option<&AssetStore>,
    mut observer: Option<&mut dyn FnMut(Result<ResolvedShapeSdfCommand, ShapeSdfCaptureError>)>,
    render_pixels: bool,
) {
    let mut fill_paint = Paint::default();
    fill_paint.set_style(PaintStyle::Fill);
    fill_paint.set_anti_alias(true);
    let color = md.resolve_color(shape.color_id).unwrap_or(ResolvedColor {
        r: 128,
        g: 128,
        b: 128,
        a: 255,
    });
    fill_paint.set_color4f(
        Color4f::new(
            color.r as f32 / 255.0,
            color.g as f32 / 255.0,
            color.b as f32 / 255.0,
            shape.alpha,
        ),
        None,
    );

    let border_padding = 16.0_f32;
    let rect_size = 1024.0 - (1.0 - shape.outline_size) * border_padding;
    let half = rect_size / 2.0;
    let bounds = Rect::from_xywh(-half, -half, rect_size, rect_size);

    let outline_color = md
        .resolve_color(shape.outline_color_id)
        .unwrap_or(ResolvedColor {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        });
    let outline_color_paint = if shape.outline_size > 0.0 {
        let mut sp = Paint::default();
        sp.set_style(PaintStyle::Fill);
        sp.set_anti_alias(true);
        sp.set_color4f(
            Color4f::new(
                outline_color.r as f32 / 255.0,
                outline_color.g as f32 / 255.0,
                outline_color.b as f32 / 255.0,
                1.0,
            ),
            None,
        );
        Some(sp)
    } else {
        None
    };

    let shape_info = md.resolve_resource("shape", shape.id);
    let has_resource_identity = shape_info.is_some();
    let file_name = shape_info
        .as_ref()
        .map(|r| r.file_name.as_str())
        .unwrap_or("square");
    let asset_key = format!("custom_profile/shape/{file_name}");

    if let Some((asset_store, mask_img)) =
        assets.and_then(|store| store.get_image(&asset_key).map(|image| (store, image)))
    {
        let sprite_w = mask_img.width() as f32;
        let sprite_h = mask_img.height() as f32;
        let dst = Rect::from_xywh(-sprite_w / 2.0, -sprite_h / 2.0, sprite_w, sprite_h);

        let outer_fill_ratio = shape.outline_size * 0.95;
        let face_sdf_threshold = 0.5 + shape.outline_size * 0.2375;
        let outline_sdf_threshold = (1.0_f32 - outer_fill_ratio * 0.75).min(0.5);

        if !render_pixels {
            if let Some(observer) = observer.as_deref_mut() {
                let captured = if !has_resource_identity {
                    Err(ShapeSdfCaptureError::MissingResource { shape_id: shape.id })
                } else if let Some(identity) =
                    asset_store.shape_sdf_source_identity(&asset_key, &mask_img)
                {
                    resolve_shape_sdf_command(
                        canvas,
                        shape,
                        &asset_key,
                        identity.width,
                        identity.height,
                        color,
                        outline_color,
                        dst,
                        identity.rg8_sha256,
                    )
                } else {
                    Err(ShapeSdfCaptureError::ReadPixels {
                        asset_key: asset_key.clone(),
                    })
                };
                observer(captured);
            }
            return;
        }

        let info = mask_img.image_info();
        let w = info.width() as usize;
        let h = info.height() as usize;
        let row_bytes = w * 4;
        let mut src = vec![0u8; row_bytes * h];
        let unpremul_info = skia_safe::ImageInfo::new(
            (w as i32, h as i32),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Unpremul,
            None,
        );
        if !mask_img.read_pixels(
            &unpremul_info,
            &mut src,
            row_bytes,
            (0, 0),
            skia_safe::image::CachingHint::Allow,
        ) {
            if let Some(observer) = observer.as_deref_mut() {
                observer(Err(ShapeSdfCaptureError::ReadPixels {
                    asset_key: asset_key.clone(),
                }));
            }
            return;
        }

        if let Some(observer) = observer.as_deref_mut() {
            let captured = if !has_resource_identity {
                Err(ShapeSdfCaptureError::MissingResource { shape_id: shape.id })
            } else {
                let mut hasher = Sha256::new();
                for pixel in src.chunks_exact(4) {
                    hasher.update([pixel[0], pixel[3]]);
                }
                resolve_shape_sdf_command(
                    canvas,
                    shape,
                    &asset_key,
                    mask_img.width(),
                    mask_img.height(),
                    color,
                    outline_color,
                    dst,
                    hex::encode(hasher.finalize()),
                )
            };
            observer(captured);
        }

        let make_sdf_mask = |threshold: f32| -> Option<skia_safe::Image> {
            let mut dst_pixels = vec![0u8; row_bytes * h];
            for i in (0..dst_pixels.len()).step_by(4) {
                dst_pixels[i] = 255;
                dst_pixels[i + 1] = 255;
                dst_pixels[i + 2] = 255;
                dst_pixels[i + 3] = sdf_mask_alpha(src[i], src[i + 3], threshold);
            }
            let out_info = skia_safe::ImageInfo::new(
                (w as i32, h as i32),
                skia_safe::ColorType::RGBA8888,
                skia_safe::AlphaType::Unpremul,
                None,
            );
            skia_safe::images::raster_from_data(
                &out_info,
                skia_safe::Data::new_copy(&dst_pixels),
                row_bytes,
            )
        };

        if shape.outline_size > 0.01 {
            if let (Some(ol_mask), Some(face_mask)) = (
                make_sdf_mask(outline_sdf_threshold),
                make_sdf_mask(face_sdf_threshold),
            ) {
                let outline_alpha_u8 = (shape.outline_alpha * 255.0) as u32;
                canvas.save_layer_alpha(dst, outline_alpha_u8);
                canvas.draw_image_rect(&ol_mask, None, dst, &Paint::default());
                if let Some(mut ap) = outline_color_paint.clone() {
                    ap.set_blend_mode(skia_safe::BlendMode::SrcIn);
                    canvas.draw_rect(dst, &ap);
                }
                let mut co = Paint::default();
                co.set_blend_mode(skia_safe::BlendMode::DstOut);
                canvas.draw_image_rect(&face_mask, None, dst, &co);
                canvas.restore();
            }
        }

        if let Some(face_mask) = make_sdf_mask(face_sdf_threshold) {
            let alpha_u8 = (shape.alpha * 255.0) as u32;
            canvas.save_layer_alpha(dst, alpha_u8);
            canvas.draw_image_rect(&face_mask, None, dst, &Paint::default());
            fill_paint.set_blend_mode(skia_safe::BlendMode::SrcIn);
            fill_paint.set_color4f(
                Color4f::new(
                    color.r as f32 / 255.0,
                    color.g as f32 / 255.0,
                    color.b as f32 / 255.0,
                    1.0,
                ),
                None,
            );
            canvas.draw_rect(dst, &fill_paint);
            canvas.restore();
        }
        return;
    }

    if let Some(observer) = observer.as_deref_mut() {
        observer(if has_resource_identity {
            Err(ShapeSdfCaptureError::MissingAsset {
                asset_key: asset_key.clone(),
            })
        } else {
            Err(ShapeSdfCaptureError::MissingResource { shape_id: shape.id })
        });
    }
    if !render_pixels {
        return;
    }

    let outline_thickness = shape.outline_size * rect_size * 0.05;
    let stroke_paint = outline_color_paint.as_ref().map(|p| {
        let mut sp = p.clone();
        sp.set_style(PaintStyle::Stroke);
        sp.set_stroke_width(outline_thickness);
        sp
    });

    match shape.id {
        1 => {
            if let Some(sp) = &stroke_paint {
                let mut sb = bounds;
                sb.outset((outline_thickness / 2.0, outline_thickness / 2.0));
                canvas.draw_oval(sb, sp);
            }
            canvas.draw_oval(bounds, &fill_paint);
        }
        2 => {
            let triangle_h = rect_size * (3.0_f32).sqrt() / 2.0;
            let path = {
                let mut b = skia_safe::PathBuilder::new();
                b.move_to((0.0, -triangle_h / 2.0));
                b.line_to((half, triangle_h / 2.0));
                b.line_to((-half, triangle_h / 2.0));
                b.close();
                b.detach()
            };
            if let Some(sp) = &stroke_paint {
                let mut sp_out = sp.clone();
                sp_out.set_stroke_width(outline_thickness * 2.0);
                canvas.draw_path(&path, &sp_out);
            }
            canvas.draw_path(&path, &fill_paint);
        }
        4 => {
            let radius = rect_size * 0.17;
            if let Some(sp) = &stroke_paint {
                let mut sb = bounds;
                sb.outset((outline_thickness / 2.0, outline_thickness / 2.0));
                canvas.draw_round_rect(sb, radius, radius, sp);
            }
            canvas.draw_round_rect(bounds, radius, radius, &fill_paint);
        }
        _ => {
            if let Some(sp) = &stroke_paint {
                let mut sb = bounds;
                sb.outset((outline_thickness / 2.0, outline_thickness / 2.0));
                canvas.draw_rect(sb, sp);
            }
            canvas.draw_rect(bounds, &fill_paint);
        }
    }
}
