//! Bounded RGBA canvas built on the profile compositor's image, shape, and blend routines.
use super::*;
use crate::codec;
use sekai_profile_renderer_core::LayerKind;

pub struct Image {
    entry: RenderObjectEntry,
    pixels: Vec<u8>,
}
impl Image {
    pub fn from_png(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 24
            || bytes.len() > 32 * 1024 * 1024
            || !codec::png::is_png(bytes)
            || &bytes[12..16] != b"IHDR"
        {
            return Err("invalid PNG input".into());
        }
        let width = u32::from_be_bytes(bytes[16..20].try_into().map_err(|_| "invalid PNG width")?);
        let height =
            u32::from_be_bytes(bytes[20..24].try_into().map_err(|_| "invalid PNG height")?);
        if u64::from(width) * u64::from(height) > 32 * 1024 * 1024 {
            return Err("image exceeds pixel budget".into());
        }
        let decoded = codec::png::decode(bytes).map_err(|e| e.to_string())?;
        Self::from_rgba(decoded.width, decoded.height, decoded.pixels)
    }
    pub fn from_rgba(width: u32, height: u32, mut pixels: Vec<u8>) -> Result<Self, String> {
        let size = canvas_bytes(width, height).map_err(|e| e.to_string())?;
        if size > 128 * 1024 * 1024 || pixels.len() != size {
            return Err("invalid or oversized image".into());
        }
        let source = hex::encode(Sha256::digest(&pixels));
        for pixel in pixels.chunks_exact_mut(4) {
            for c in 0..3 {
                pixel[c] = codec::premultiply_channel(pixel[c], pixel[3]);
            }
        }
        let entry = RenderObjectEntry {
            key: "canvas-image".into(),
            kind: RenderObjectKind::Texture,
            source_sha256: source,
            page: 0,
            offset: 0,
            length: pixels.len() as u64,
            width,
            height,
            row_bytes: width * 4,
            pixel_sha256: hex::encode(Sha256::digest(&pixels)),
        };
        Ok(Self { entry, pixels })
    }
    pub fn width(&self) -> u32 {
        self.entry.width
    }
    pub fn height(&self) -> u32 {
        self.entry.height
    }
    fn mapped(&self) -> MappedRenderObject<'_> {
        MappedRenderObject {
            entry: &self.entry,
            pixels: &self.pixels,
        }
    }
}
pub struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    clip: Option<Rect>,
}
impl Canvas {
    pub fn new(width: u32, height: u32, clear: [u8; 4]) -> Result<Self, String> {
        let len = canvas_bytes(width, height).map_err(|e| e.to_string())?;
        if len > 128 * 1024 * 1024 {
            return Err("canvas exceeds pixel budget".into());
        }
        let color = [
            codec::premultiply_channel(clear[0], clear[3]),
            codec::premultiply_channel(clear[1], clear[3]),
            codec::premultiply_channel(clear[2], clear[3]),
            clear[3],
        ];
        let mut pixels = vec![0; len];
        for p in pixels.chunks_exact_mut(4) {
            p.copy_from_slice(&color)
        }
        Ok(Self {
            width,
            height,
            pixels,
            clip: None,
        })
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn premultiplied_rgba(&self) -> &[u8] {
        &self.pixels
    }
    pub fn set_clip(&mut self, clip: Option<Rect>) -> Result<(), String> {
        if let Some(r) = clip {
            validate_rect(r)?;
        }
        self.clip = clip;
        Ok(())
    }
    pub fn image(
        &mut self,
        image: &Image,
        bounds: Rect,
        uv: Rect,
        opacity: f32,
    ) -> Result<(), String> {
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err("invalid opacity".into());
        }
        let clip = self.clip.map(|r| AxisAlignedClip {
            min_x: r.x,
            min_y: r.y,
            max_x: r.x + r.width,
            max_y: r.y + r.height,
        });
        raster_image_command(
            &mut self.pixels,
            self.width,
            self.height,
            image.mapped(),
            bounds,
            uv,
            [1.0, 1.0, 1.0, opacity],
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            BlendMode::SrcOver,
            "canvas-image",
            clip,
            None,
            None,
            ImageExecutor::Scalar,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }
    pub fn shape(
        &mut self,
        bounds: Rect,
        primitive: ShapePrimitive,
        fill: [u8; 4],
        stroke: [u8; 4],
        stroke_width: f32,
    ) -> Result<(), String> {
        validate_rect(bounds)?;
        let mut command = SemanticCommandSource::shape(
            StableId(1),
            StableId(1),
            "canvas-shape",
            bounds,
            primitive.clone(),
        );
        command.clip = self.clip.map(|bounds| {
            [
                [bounds.x, bounds.y],
                [bounds.x + bounds.width, bounds.y],
                [bounds.x + bounds.width, bounds.y + bounds.height],
                [bounds.x, bounds.y + bounds.height],
            ]
        });
        let layer = LayerSource {
            id: StableId(1),
            parent_id: None,
            kind: LayerKind::Shape,
            authored_kind: AuthoredElementKind::Shape,
            authored_index: 0,
            game_layer: 0,
            z: 0,
            authored_visible: true,
            source_content: String::new(),
            resolved_parameters: BTreeMap::new(),
            bounds,
            quad: [[0.0; 2]; 4],
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            hit_geometry: [[0.0; 2]; 4],
            line_indent: None,
        };
        raster_semantic_shape_command(
            &mut self.pixels,
            self.width,
            self.height,
            &command,
            &layer,
            &primitive,
            fill.map(|v| f32::from(v) / 255.0),
            None,
            stroke.map(|v| f32::from(v) / 255.0),
            stroke_width,
            0.0,
            ImageExecutor::Scalar,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    /// Crops the premultiplied buffer without a decode or alpha round-trip.
    pub fn crop(&self, x: u32, y: u32, width: u32, height: u32) -> Result<Self, String> {
        if width == 0
            || height == 0
            || x.checked_add(width).is_none_or(|v| v > self.width)
            || y.checked_add(height).is_none_or(|v| v > self.height)
        {
            return Err("crop outside canvas".into());
        }
        let mut out = Self::new(width, height, [0; 4])?;
        for row in 0..height as usize {
            let start = ((y as usize + row) * self.width as usize + x as usize) * 4;
            let target = row * width as usize * 4;
            out.pixels[target..target + width as usize * 4]
                .copy_from_slice(&self.pixels[start..start + width as usize * 4]);
        }
        Ok(out)
    }
    pub fn encode_png(&self) -> Result<Vec<u8>, String> {
        let mut rgba = self.pixels.clone();
        for p in rgba.chunks_exact_mut(4) {
            for c in 0..3 {
                p[c] = codec::unpremultiply_channel_like_skia(p[c], p[3]);
            }
        }
        codec::png::encode_rgba(self.width, self.height, &rgba).map_err(|e| e.to_string())
    }
}
fn validate_rect(r: Rect) -> Result<(), String> {
    if [r.x, r.y, r.width, r.height].iter().any(|v| !v.is_finite())
        || r.width <= 0.0
        || r.height <= 0.0
    {
        return Err("invalid rectangle".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_and_shape_share_source_over() {
        let mut canvas = Canvas::new(4, 4, [0, 0, 0, 0]).unwrap();
        canvas
            .shape(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 4.0,
                    height: 4.0,
                },
                ShapePrimitive::Rect,
                [255, 0, 0, 255],
                [0; 4],
                0.0,
            )
            .unwrap();
        let image = Image::from_rgba(1, 1, vec![0, 0, 255, 128]).unwrap();
        canvas
            .image(
                &image,
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 4.0,
                    height: 4.0,
                },
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                },
                1.0,
            )
            .unwrap();
        assert_eq!(&canvas.premultiplied_rgba()[20..24], &[127, 0, 128, 255]);
    }
    #[test]
    fn png_preserves_alpha() {
        let canvas = Canvas::new(1, 1, [255, 0, 0, 128]).unwrap();
        let decoded = codec::png::decode(&canvas.encode_png().unwrap()).unwrap();
        assert_eq!(decoded.pixels, vec![255, 0, 0, 128]);
    }

    #[test]
    fn clip_bounds_shapes_and_curves() {
        let mut canvas = Canvas::new(8, 8, [0; 4]).unwrap();
        canvas
            .set_clip(Some(Rect {
                x: 2.0,
                y: 2.0,
                width: 4.0,
                height: 4.0,
            }))
            .unwrap();
        canvas
            .shape(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 8.0,
                    height: 8.0,
                },
                ShapePrimitive::Ellipse,
                [0, 255, 0, 255],
                [0; 4],
                0.0,
            )
            .unwrap();
        canvas
            .quadratic(
                [[0.0, 4.0], [4.0, 0.0], [8.0, 4.0]],
                2.0,
                [255, 255, 255, 255],
            )
            .unwrap();
        for y in 0..8 {
            for x in 0..8 {
                if !(2..6).contains(&x) || !(2..6).contains(&y) {
                    assert_eq!(canvas.premultiplied_rgba()[(y * 8 + x) * 4 + 3], 0);
                }
            }
        }
        assert!(canvas
            .premultiplied_rgba()
            .chunks_exact(4)
            .any(|p| p[3] > 0));
    }

    #[test]
    fn huge_png_is_rejected_before_decode() {
        let mut data = vec![137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82];
        data.extend_from_slice(&u32::MAX.to_be_bytes());
        data.extend_from_slice(&u32::MAX.to_be_bytes());
        assert!(Image::from_png(&data).is_err());
    }

    #[test]
    fn invalid_geometry_is_rejected() {
        assert!(Canvas::new(u32::MAX, u32::MAX, [0; 4]).is_err());
        let mut canvas = Canvas::new(2, 2, [0; 4]).unwrap();
        assert!(canvas
            .set_clip(Some(Rect {
                x: f32::NAN,
                y: 0.0,
                width: 1.0,
                height: 1.0
            }))
            .is_err());
    }
}

impl Canvas {
    /// Stroke a quadratic curve using the core's analytic geometry distance field.
    pub fn quadratic(
        &mut self,
        points: [[f32; 2]; 3],
        width: f32,
        color: [u8; 4],
    ) -> Result<(), String> {
        use sekai_profile_renderer_core::sdf_geometry::{
            AnalyticDistanceField, QuadSeg, Segment, Vec2,
        };
        if !width.is_finite()
            || width <= 0.0
            || width > 4096.0
            || points.iter().flatten().any(|v| !v.is_finite())
        {
            return Err("invalid curve".into());
        }
        let field = AnalyticDistanceField::new(&[vec![Segment::Quad(QuadSeg {
            p0: Vec2 {
                x: points[0][0],
                y: points[0][1],
            },
            p1: Vec2 {
                x: points[1][0],
                y: points[1][1],
            },
            p2: Vec2 {
                x: points[2][0],
                y: points[2][1],
            },
        })]]);
        let min_x = points.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min) - width;
        let max_x = points
            .iter()
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max)
            + width;
        let min_y = points.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min) - width;
        let max_y = points
            .iter()
            .map(|p| p[1])
            .fold(f32::NEG_INFINITY, f32::max)
            + width;
        for y in
            (min_y.floor().max(0.0) as u32)..(max_y.ceil().min(self.height as f32).max(0.0) as u32)
        {
            for x in (min_x.floor().max(0.0) as u32)
                ..(max_x.ceil().min(self.width as f32).max(0.0) as u32)
            {
                let distance = field
                    .signed_distance(Vec2 {
                        x: x as f32 + 0.5,
                        y: y as f32 + 0.5,
                    })
                    .abs();
                self.coverage(
                    x as i32,
                    y as i32,
                    color,
                    (width * 0.5 + 0.5 - distance).clamp(0.0, 1.0),
                );
            }
        }
        Ok(())
    }
    fn coverage(&mut self, x: i32, y: i32, color: [u8; 4], coverage: f32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        if let Some(clip) = self.clip {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            if cx < clip.x || cy < clip.y || cx >= clip.x + clip.width || cy >= clip.y + clip.height
            {
                return;
            }
        }
        let alpha = (f32::from(color[3]) * coverage.clamp(0.0, 1.0)).round() as u8;
        let src = [
            codec::premultiply_channel(color[0], alpha),
            codec::premultiply_channel(color[1], alpha),
            codec::premultiply_channel(color[2], alpha),
            alpha,
        ];
        let offset = (y as usize * self.width as usize + x as usize) * 4;
        blend_pixel(
            &mut self.pixels[offset..offset + 4],
            src,
            BlendMode::SrcOver,
        );
    }
    pub fn text(
        &mut self,
        font: &Font<'_>,
        text: &str,
        x: f32,
        baseline: f32,
        color: [u8; 4],
    ) -> Result<(), String> {
        if !x.is_finite()
            || !baseline.is_finite()
            || x.abs() > 1.0e7
            || baseline.abs() > 1.0e7
            || text.len() > 4096
        {
            return Err("invalid text run".into());
        }
        let mut pen = x;
        for ch in text.chars() {
            font.load(ch)?;
            let glyph = font.face.glyph();
            let bitmap = glyph.bitmap();
            if bitmap.width() == 0 || bitmap.rows() == 0 {
                pen += glyph.advance().x as f32 / 64.0;
                continue;
            }
            if bitmap.pixel_mode().map_err(|e| format!("{e:?}"))?
                != freetype::bitmap::PixelMode::Gray
            {
                return Err("unsupported glyph bitmap".into());
            }
            let pitch = bitmap.pitch();
            let rows = bitmap.rows();
            let stride = pitch.unsigned_abs() as usize;
            for row in 0..rows {
                let srcrow = if pitch < 0 { rows - 1 - row } else { row } as usize;
                for col in 0..bitmap.width() {
                    let coverage =
                        f32::from(bitmap.buffer()[srcrow * stride + col as usize]) / 255.0;
                    self.coverage(
                        pen.round() as i32 + glyph.bitmap_left() + col,
                        baseline.round() as i32 - glyph.bitmap_top() + row,
                        color,
                        coverage,
                    );
                }
            }
            pen += glyph.advance().x as f32 / 64.0;
        }
        Ok(())
    }
}
/// A drawing-local FreeType face borrowing a shared font byte allocation.
/// This helper handles plain labels; rich text continues to use the TMP pipeline.
pub struct Font<'a> {
    face: freetype::Face<&'a [u8]>,
}
impl<'a> Font<'a> {
    pub fn new(bytes: &'a [u8], size: f32) -> Result<Self, String> {
        if !size.is_finite() || size <= 0.0 || size > 1024.0 {
            return Err("invalid font size".into());
        }
        let library = freetype::Library::init().map_err(|e| format!("{e:?}"))?;
        let face = library
            .new_memory_face2(bytes, 0)
            .map_err(|e| format!("{e:?}"))?;
        face.set_char_size((size * 64.0).round() as isize, 0, 72, 72)
            .map_err(|e| format!("{e:?}"))?;
        Ok(Self { face })
    }
    fn load(&self, ch: char) -> Result<(), String> {
        if self.face.get_char_index(ch as usize).is_none() {
            return Err(format!("font has no glyph U+{:04X}", ch as u32));
        }
        self.face
            .load_char(
                ch as usize,
                freetype::face::LoadFlag::RENDER | freetype::face::LoadFlag::TARGET_NORMAL,
            )
            .map_err(|e| format!("{e:?}"))
    }
    pub fn measure(&self, text: &str) -> Result<f32, String> {
        if text.len() > 4096 {
            return Err("text run exceeds length limit".into());
        }
        let mut width = 0.0;
        for ch in text.chars() {
            self.load(ch)?;
            width += self.face.glyph().advance().x as f32 / 64.0;
        }
        Ok(width)
    }
}
