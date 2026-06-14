use crate::assets::AssetStore;
use crate::masterdata::{MasterData, ResolvedColor};
use crate::types::ShapeElement;
use skia_safe::{Canvas, Color4f, Paint, PaintStyle, Rect};

/// 绘制图形元素。
pub fn draw_shape(
    canvas: &Canvas,
    shape: &ShapeElement,
    md: &MasterData,
    assets: Option<&AssetStore>,
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

    let outline_color_paint = if shape.outline_size > 0.0 {
        let outline_color = md
            .resolve_color(shape.outline_color_id)
            .unwrap_or(ResolvedColor {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            });
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
    let file_name = shape_info
        .as_ref()
        .map(|r| r.file_name.as_str())
        .unwrap_or("square");
    let asset_key = format!("custom_profile/shape/{file_name}");

    if let Some(mask_img) = assets.and_then(|s| s.get_image(&asset_key)) {
        let sprite_w = mask_img.width() as f32;
        let sprite_h = mask_img.height() as f32;
        let dst = Rect::from_xywh(-sprite_w / 2.0, -sprite_h / 2.0, sprite_w, sprite_h);

        let outer_fill_ratio = shape.outline_size * 0.95;
        let face_sdf_threshold = 0.5 + shape.outline_size * 0.2375;
        let outline_sdf_threshold = (1.0_f32 - outer_fill_ratio * 0.75).min(0.5);

        let make_sdf_mask = |img: &skia_safe::Image, threshold: f32| -> Option<skia_safe::Image> {
            let info = img.image_info();
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
            if !img.read_pixels(
                &unpremul_info,
                &mut src,
                row_bytes,
                (0, 0),
                skia_safe::image::CachingHint::Allow,
            ) {
                return None;
            }
            let th = threshold * 255.0;
            let sharp = 1.5_f32;
            let mut dst_pixels = vec![0u8; row_bytes * h];
            for i in (0..dst_pixels.len()).step_by(4) {
                let r_val = src[i] as f32;
                let a = ((r_val - th + sharp) / (2.0 * sharp)).clamp(0.0, 1.0);
                dst_pixels[i] = 255;
                dst_pixels[i + 1] = 255;
                dst_pixels[i + 2] = 255;
                dst_pixels[i + 3] = (a * 255.0) as u8;
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
                make_sdf_mask(mask_img.as_ref(), outline_sdf_threshold),
                make_sdf_mask(mask_img.as_ref(), face_sdf_threshold),
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

        if let Some(face_mask) = make_sdf_mask(mask_img.as_ref(), face_sdf_threshold) {
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
