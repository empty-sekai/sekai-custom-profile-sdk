//! What a render is asked to do, and what the renderer can do.
//!
//! Text layout is outside executor selection: every executor consumes the
//! same completed TMP glyph operations. The SDF executor rasterizes text and
//! shape coverage; images are composited by the same CPU compositor with
//! either executor.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// How SDF text and shape coverage is rasterized.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SdfExecutor {
    /// [`SdfExecutor::Simd`] when the CPU supports it, otherwise
    /// [`SdfExecutor::Scalar`].
    #[default]
    Auto,
    /// The AVX-512 packet executor. It needs an x86-64 CPU with AVX-512 F,
    /// BW and VBMI, and FMA.
    Simd,
    /// The portable scalar executor.
    Scalar,
}

/// What a page is drawn over.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Background {
    /// Opaque white, as the game shows the card.
    #[default]
    White,
    /// Transparent. Such a page encodes to PNG only.
    Transparent,
}

impl Background {
    /// The straight RGBA the surface is cleared to.
    pub(crate) fn clear_rgba(self) -> [u8; 4] {
        match self {
            Self::White => [255, 255, 255, 255],
            Self::Transparent => [0, 0, 0, 0],
        }
    }
}

/// The reverse pixel-occlusion pass over SDF runs.
///
/// The pass finds SDF pixels that opaque images drawn later cover completely,
/// using the images of the installed render-object store. It never changes
/// the rendered pixels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PixelOcclusion {
    /// No occlusion pass. The default.
    #[default]
    Off,
    /// Count the covered pixels in the telemetry and draw them anyway.
    Measure,
    /// Skip the covered pixels.
    Cull,
}

impl PixelOcclusion {
    pub(crate) fn enabled(self) -> bool {
        self != Self::Off
    }
}

/// How to render one page.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderConfig {
    /// The SDF executor; [`SdfExecutor::Auto`] by default.
    pub executor: SdfExecutor,
    /// What the page is drawn over; [`Background::White`] by default.
    pub background: Background,
    /// SDF tile width in pixels: a power of two from 8 to 128; 32 by default.
    pub tile_width: u16,
    /// SDF tile height in pixels: a power of two from 8 to 128; 32 by
    /// default.
    pub tile_height: u16,
    /// The reverse pixel-occlusion pass; [`PixelOcclusion::Off`] by default.
    pub pixel_occlusion: PixelOcclusion,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            executor: SdfExecutor::Auto,
            background: Background::White,
            tile_width: 32,
            tile_height: 32,
            pixel_occlusion: PixelOcclusion::Off,
        }
    }
}

impl RenderConfig {
    /// Checks the config and resolves [`SdfExecutor::Auto`] for a CPU that
    /// does or does not support the SIMD executor.
    pub(crate) fn resolve(&self, simd_available: bool) -> Result<Executor, RenderError> {
        let tile_ok = |edge: u16| (8..=128).contains(&edge) && edge.is_power_of_two();
        if !tile_ok(self.tile_width) || !tile_ok(self.tile_height) {
            return Err(RenderError::InvalidTileSize {
                width: self.tile_width,
                height: self.tile_height,
            });
        }
        match self.executor {
            SdfExecutor::Auto if simd_available => Ok(Executor::Simd),
            SdfExecutor::Auto | SdfExecutor::Scalar => Ok(Executor::Scalar),
            SdfExecutor::Simd if simd_available => Ok(Executor::Simd),
            SdfExecutor::Simd => Err(RenderError::SimdUnavailable),
        }
    }
}

/// The executor a render runs on.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) enum Executor {
    Scalar,
    Simd,
}

impl Executor {
    pub(crate) fn public(self) -> SdfExecutor {
        match self {
            Self::Scalar => SdfExecutor::Scalar,
            Self::Simd => SdfExecutor::Simd,
        }
    }
}

/// Whether this CPU runs [`SdfExecutor::Simd`].
pub fn sdf_simd_available() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512bw")
            && std::arch::is_x86_feature_detected!("avx512vbmi")
            && std::arch::is_x86_feature_detected!("fma")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// The resources a renderer has installed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderCapabilities {
    /// This CPU runs [`SdfExecutor::Simd`].
    pub simd: bool,
    /// Installed text atlases, counting each font family and sampling tier.
    pub text_atlases: usize,
    /// A shape SDF atlas is installed.
    pub shape_atlas: bool,
    /// A render-object store is installed. Without one, images are decoded
    /// from the asset store during each render.
    pub render_objects: bool,
    /// A fallback glyph cache is installed, so characters the card fonts
    /// lack are drawn from the fallback face.
    pub fallback_glyphs: bool,
}

/// Why a render, or the encoding of its page, failed.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum RenderError {
    /// [`RenderConfig::tile_width`] or [`RenderConfig::tile_height`] is not a
    /// power of two from 8 to 128.
    #[error("invalid SDF tile size {width}x{height}; expected powers of two from 8 to 128")]
    InvalidTileSize {
        /// The configured tile width.
        width: u16,
        /// The configured tile height.
        height: u16,
    },
    /// [`SdfExecutor::Simd`] was requested on a CPU that cannot run it; see
    /// [`sdf_simd_available`].
    #[error("the SIMD executor needs an x86-64 CPU with AVX-512 F, BW and VBMI, and FMA")]
    SimdUnavailable,
    /// Any other failure while resolving, drawing or encoding a page, with
    /// its message.
    #[error("{0}")]
    Render(String),
}

impl From<String> for RenderError {
    fn from(message: String) -> Self {
        Self::Render(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_config_renders_white_on_the_best_executor() {
        let config = RenderConfig::default();
        assert_eq!(config.background, Background::White);
        assert_eq!(config.resolve(true), Ok(Executor::Simd));
        assert_eq!(config.resolve(false), Ok(Executor::Scalar));
    }

    #[test]
    fn simd_is_refused_on_a_cpu_without_it() {
        let config = RenderConfig {
            executor: SdfExecutor::Simd,
            ..RenderConfig::default()
        };
        assert_eq!(config.resolve(false), Err(RenderError::SimdUnavailable));
        assert_eq!(config.resolve(true), Ok(Executor::Simd));
        let scalar = RenderConfig {
            executor: SdfExecutor::Scalar,
            ..RenderConfig::default()
        };
        assert_eq!(scalar.resolve(true), Ok(Executor::Scalar));
    }

    #[test]
    fn tile_sizes_must_be_powers_of_two_from_8_to_128() {
        for (width, height) in [(24, 32), (32, 4), (256, 32)] {
            let config = RenderConfig {
                tile_width: width,
                tile_height: height,
                ..RenderConfig::default()
            };
            assert_eq!(
                config.resolve(true),
                Err(RenderError::InvalidTileSize { width, height })
            );
        }
    }

    #[test]
    fn a_partial_config_takes_defaults_for_missing_fields() {
        let config: RenderConfig =
            serde_json::from_value(serde_json::json!({ "executor": "scalar" })).expect("config");
        assert_eq!(config.executor, SdfExecutor::Scalar);
        assert_eq!((config.tile_width, config.tile_height), (32, 32));
        assert_eq!(
            serde_json::to_value(Background::Transparent).expect("serialize"),
            serde_json::json!("transparent")
        );
    }
}
