//! Rendered pages and their encodings.

use serde::{Deserialize, Serialize};

use crate::render_config::Background;
use crate::telemetry::RenderTelemetry;

/// One rendered page: premultiplied RGBA8 pixels and the telemetry of the
/// render that produced them.
pub struct RenderedPage {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
    pub(crate) background: Background,
    pub telemetry: RenderTelemetry,
}

impl RenderedPage {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn background(&self) -> Background {
        self.background
    }

    /// Premultiplied RGBA8, row-major and tightly packed.
    pub fn premultiplied_rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Straight (non-premultiplied) RGBA8, row-major and tightly packed.
    pub fn to_straight_rgba(&self) -> Vec<u8> {
        let mut straight = self.rgba.clone();
        unpremultiply(&mut straight);
        straight
    }

    /// Encodes the page as PNG.
    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        crate::codec::png::encode_rgba(self.width, self.height, &self.to_straight_rgba())
            .map_err(|error| format!("PNG encoding failed: {error}"))
    }

    /// Encodes the page as a baseline JPEG at `quality` (1 to 100) with the
    /// standard encoder. A page with a transparent background has no JPEG
    /// form.
    #[cfg(feature = "jpeg-turbo")]
    pub fn to_jpeg(&self, quality: u8) -> Result<Vec<u8>, String> {
        self.jpeg_input()?;
        let options = JpegOptions {
            quality,
            ..JpegOptions::default()
        };
        options.validate()?;
        crate::jpeg_turbo::encode_rgba(&self.rgba, self.width, self.height, u32::from(quality))
    }

    #[cfg(feature = "jpeg-turbo")]
    pub(crate) fn jpeg_input(&self) -> Result<(), String> {
        if self.background == Background::Transparent {
            return Err(
                "a page with a transparent background has no JPEG form; encode it as PNG".into(),
            );
        }
        Ok(())
    }
}

/// Straightens premultiplied RGBA8 in place with the engine's rounding.
fn unpremultiply(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = pixel[3];
        if alpha != 0 && alpha != u8::MAX {
            for channel in &mut pixel[..3] {
                *channel = crate::codec::unpremultiply_channel(*channel, alpha);
            }
        }
    }
}

/// The JPEG encoder implementation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JpegEncoder {
    /// libjpeg-turbo with its own colour conversion.
    #[default]
    Standard,
    /// AVX-512 RGB to YUV 4:2:0 conversion feeding libjpeg-turbo's raw-data
    /// encoder. It needs an x86-64 CPU with AVX-512, and its bytes differ
    /// from [`JpegEncoder::Standard`].
    Avx512Yuv420,
}

/// How to encode a page as JPEG.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct JpegOptions {
    /// 1 to 100.
    pub quality: u8,
    pub encoder: JpegEncoder,
}

impl Default for JpegOptions {
    fn default() -> Self {
        Self {
            quality: 90,
            encoder: JpegEncoder::Standard,
        }
    }
}

impl JpegOptions {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !(1..=100).contains(&self.quality) {
            return Err(format!(
                "JPEG quality must be from 1 to 100, got {}",
                self.quality
            ));
        }
        Ok(())
    }
}

/// An encoded page and the telemetry of its render and encoding.
pub struct EncodedPage {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
    pub telemetry: RenderTelemetry,
}

/// The pages of one profile, rendered together.
pub struct RenderedBatch {
    /// The resolved scenes and resource requests of every page.
    pub compiled: crate::compiled_profile::CompiledProfileBatch,
    /// How the pinned render-object store covered the pages' resources;
    /// `None` without a store.
    pub prepared_render_objects: Option<crate::compiled_profile::PreparedRenderObjectBatch>,
    /// Each page with its `seq`, in ascending `seq` order.
    pub pages: Vec<(i32, RenderedPage)>,
    pub fallback_glyph_cache: Option<crate::sdf::fallback_cache::PersistentFallbackSdfCacheReport>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(rgba: Vec<u8>, background: Background) -> RenderedPage {
        RenderedPage {
            width: 2,
            height: 1,
            rgba,
            background,
            telemetry: RenderTelemetry::new(
                crate::RenderConfig::default(),
                crate::SdfExecutor::Scalar,
            ),
        }
    }

    #[test]
    fn straight_rgba_divides_alpha_back_out() {
        let page = page(
            vec![64, 32, 0, 128, 10, 20, 30, 255],
            Background::Transparent,
        );
        let straight = page.to_straight_rgba();
        assert_eq!(&straight[4..], &[10, 20, 30, 255]);
        assert_eq!(straight[3], 128);
        assert_eq!(
            straight[0],
            crate::codec::unpremultiply_channel(64, 128),
            "partial alpha follows the engine's rounding"
        );
        let png = page.to_png().expect("encode PNG");
        let decoded = crate::codec::png::decode(&png).expect("decode PNG");
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(decoded.pixels, straight);
    }

    #[cfg(feature = "jpeg-turbo")]
    #[test]
    fn a_transparent_page_has_no_jpeg_form() {
        let transparent = page(vec![0; 8], Background::Transparent);
        assert!(transparent.to_jpeg(90).is_err());
        let white = page(vec![255; 8], Background::White);
        assert!(white.to_jpeg(0).is_err());
        let jpeg = white.to_jpeg(90).expect("encode JPEG");
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
    }
}
