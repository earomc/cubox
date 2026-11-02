//! cubox, a Minecraft seed viewer. The desktop binary is `src/main.rs`, browsers start through
//! `web.rs`.

#[cfg(not(target_arch = "wasm32"))]
pub mod background;
pub mod biome_colors;
pub mod dimension;
pub mod features;
pub mod gui;
pub mod noise;
pub mod seed;
#[cfg(not(target_arch = "wasm32"))]
pub mod seedfind;
pub mod structures;
pub mod terrain;
pub mod tile_renderer;
#[cfg(target_arch = "wasm32")]
pub(crate) mod web;
