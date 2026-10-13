//! # sekai-profile
//!
//! Scene-graph interpreter for game custom profile cards.
//!
//! ## Render path
//! Profile JSON → `types::CustomProfileCard` → `elements::flatten_and_sort()` orders
//! elements by layer → each element is lowered to ordered commands and drawn by
//! the CPU compositor.
//!
//! ## Modules
//! - `profile`: player data model
//! - `init`: startup initialization (font installation)
//! - `assets`: encoded asset cache, decoded on demand
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
pub mod init;
#[cfg(feature = "jpeg-turbo")]
pub mod jpeg_turbo;
pub mod masterdata;
pub mod prepare;
pub mod profile;
pub mod profile_backend;
pub mod profile_compositor;
pub mod region;
pub mod render_object;
pub mod render_object_catalog;
pub mod renderer;
pub mod resource_provider;
pub mod sdf;
pub mod semantic_resolve;
pub mod text;
pub mod transform;
pub mod types;
pub mod widgets;
