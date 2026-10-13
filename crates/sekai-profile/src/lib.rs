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
//! - `renderer`: [`CustomProfileRenderer`], which renders pages and animations
//! - `render_config` / `output` / `telemetry`: what a render does and returns
//! - `atlas_dir` / `prepare`: installing and building the SDF atlases
//! - `assets`: encoded asset cache, decoded on demand
//! - `masterdata` / `profile` / `types`: master data, player data and cards
//! - `semantic_resolve` / `profile_compositor`: backend-neutral command lowering and compositing

#![deny(clippy::unwrap_used)]

pub use sekai_profile_core as core;

#[cfg(feature = "animation-export")]
pub mod animation;
pub mod asset_keys;
pub mod assets;
pub mod atlas_dir;
pub mod codec;
pub mod compiled_profile;
pub mod context;
pub mod core_shadow;
pub mod elements;
pub mod error;
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
pub mod resource_provider;
pub mod sdf;
pub mod semantic_resolve;
pub mod telemetry;
pub mod text;
pub mod transform;
pub mod types;
pub mod widgets;

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
