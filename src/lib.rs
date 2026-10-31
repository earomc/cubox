//! cubox, a Minecraft seed viewer. The desktop binary is `src/main.rs`, browsers start through
//! `web.rs`.

pub mod biome_colors;
pub mod dimension;
pub mod gui;
pub mod seed;
pub mod seedfind;
pub mod structures;
pub mod terrain;
pub mod tile_renderer;
#[cfg(target_arch = "wasm32")]
pub(crate) mod web;
