//! # sekai-profile
//!
//! Scene-graph interpreter for game custom profile cards.
//!
//! ## Render path
//! Profile JSON → `types::CustomProfileCard` → `elements::flatten_and_sort()` orders
//! elements by layer → each element is lowered to ordered commands and drawn by
//! the CPU compositor.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use std::sync::Arc;
//! use sekai_profile::{AssetStore, CustomProfileRenderer, JsonMasterDataProvider, RenderConfig};
//!
//! let masterdata = JsonMasterDataProvider::from_dir("masterdata".as_ref())?;
//! let assets = Arc::new(AssetStore::new(512));
//! assets.load_png_dir("assets".as_ref())?;
//! let renderer = CustomProfileRenderer::new(Arc::new(masterdata))
//!     .with_assets(assets)
//!     .with_atlas_dir("atlases")?;
//! # let card: sekai_profile::types::CustomProfileCard = unimplemented!();
//! let page = renderer.render(&card, None, &RenderConfig::default())?;
//! std::fs::write("card.png", page.to_png()?)?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Modules
//!
//! | Module | Responsibility |
//! | --- | --- |
//! | [`renderer`] | [`CustomProfileRenderer`]: the render entry points, page and animation |
//! | [`render_config`] | [`RenderConfig`], [`SdfExecutor`], [`RenderCapabilities`], [`PixelOcclusion`] |
//! | [`output`] | [`RenderedPage`], [`EncodedPage`] and the JPEG/PNG options |
//! | [`telemetry`] | Per-render counters and timings |
//! | [`atlas_dir`] | The installed text, tier and shape SDF atlas directories |
//! | [`prepare`] | Offline atlas construction from fonts and masterdata |
//! | [`assets`] | Encoded asset cache, decoded on demand |
//! | [`asset_keys`] | Resource keys a card or profile reads, and their storage paths |
//! | [`compiled_profile`] | Card compilation into the ordered command list |
//! | [`semantic_resolve`] | Resolving those commands against a render-object store |
//! | [`profile_compositor`] | Software composition of the resolved commands |
//! | [`render_object`] | The mmap-backed render-object store and its writer |
//! | [`render_object_catalog`] | Describing a card's objects without a store |
//! | [`sdf`] | Font lookup, glyph outline generation and the SDF atlas format |
//! | [`text`] | TextMesh Pro layout and glyph placement |
//! | [`elements`] | Authored element shapes and their layer order |
//! | [`types`] | Card, profile and masterdata input shapes |
//! | [`masterdata`] | Masterdata provider and lookups |
//! | [`profile`] | Player profile data |
//! | [`region`] | Region identifiers and their locale strings |
//! | [`transform`] | Matrix and transform helpers |
//! | [`codec`] | PNG encoding and decoding |
//! | `jpeg_turbo` | The libjpeg-turbo encoder (feature `jpeg-turbo`) |
//! | `animation` | Animation encoding and export (feature `animation-export`) |
//! | [`core_shadow`] | Scene shadow built from [`core`] for diagnostics |

#![deny(clippy::unwrap_used)]

pub use sekai_profile_core as core;

#[cfg(feature = "animation-export")]
pub mod animation;
pub mod asset_keys;
pub mod assets;
pub mod atlas_dir;
pub mod codec;
pub mod compiled_profile;
pub mod core_shadow;
pub mod elements;
#[cfg(feature = "jpeg-turbo")]
pub mod jpeg_turbo;
pub mod masterdata;
pub mod output;
pub mod prepare;
pub mod profile;
pub mod profile_compositor;
pub mod region;
pub mod render_config;
pub mod render_object;
pub mod render_object_catalog;
pub mod renderer;
pub mod sdf;
pub mod semantic_resolve;
pub mod telemetry;
pub mod text;
pub mod transform;
pub mod types;

pub use assets::AssetStore;
pub use atlas_dir::{AtlasDir, InstalledAtlases};
pub use masterdata::JsonMasterDataProvider;
pub use output::{EncodedPage, JpegEncoder, JpegOptions, RenderedBatch, RenderedPage};
pub use render_config::{
    sdf_simd_available, Background, PixelOcclusion, RenderCapabilities, RenderConfig, RenderError,
    SdfExecutor,
};
pub use renderer::{CustomProfileRenderer, PrewarmReport, RenderObjectGenerationPin};
pub use sdf::outline::set_font_dirs;
pub use telemetry::RenderTelemetry;
