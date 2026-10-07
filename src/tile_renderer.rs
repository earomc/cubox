use steel_worldgen::biomes::BiomeSourceKind;

use crate::biome_colors::biome_color;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub scale: u8, // scale of sampling
    pub sample_height: i32, // the y coordinate in blocks
    pub tile_size_px: usize, // size of the tile in pixels

    // image coordinates of the tile
    pub x: i32,
    pub z: i32,
}

/// Renders one tile to RGB bytes (TILE * TILE * 3).
/// perf: instead of setting 3 bytes per pixel, set one byte with a biome id?
pub fn render_tile(source: &BiomeSourceKind, key: TileKey) -> Vec<u8> {
    let stride = 1i32 << key.scale;
    let origin_x = key.x * key.tile_size_px as i32 * stride;
    let origin_z = key.z * key.tile_size_px as i32 * stride;

    let mut sampler = source.chunk_sampler();
    let mut rgb = vec![0u8; key.tile_size_px * key.tile_size_px * 3]; // perf: avoid heap allocation for every tile

    for (i, pixel) in rgb.chunks_exact_mut(3).enumerate() {
        // + stride / 2 samples the center of the pixel's area, not its corner
        let qx = origin_x + (i % key.tile_size_px) as i32 * stride + stride / 2;
        let qz = origin_z + (i / key.tile_size_px) as i32 * stride + stride / 2;

        let qy = key.sample_height >> 2; // converting block height to quart height dividing by 4

        let biome = sampler.sample(qx, qy, qz);
        pixel.copy_from_slice(&biome_color(&biome.key.path));
    }
    rgb
}