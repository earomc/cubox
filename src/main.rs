use rayon::prelude::*;
use std::{env, time::Instant};
use steel_worldgen::biomes::BiomeSourceKind;

const TILE: usize = 128; // pixels per tile side
const SAMPLE_QUART_Y: i32 = -59 >> 2; // block Y -59 -> quart -15

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct TileKey {
    zoom: u8,
    x: i32,
    z: i32,
}

const fn rgb(hex: u32) -> [u8; 3] {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

#[inline]
fn biome_color(path: &str) -> [u8; 3] {
    rgb(match path {
        // ---- Offshore ----
        "ocean" => 0x000070,
        "deep_ocean" => 0x000030,
        "warm_ocean" => 0x0000AC,
        "lukewarm_ocean" => 0x000090,
        "deep_lukewarm_ocean" => 0x000040,
        "cold_ocean" => 0x202070,
        "deep_cold_ocean" => 0x202038,
        "frozen_ocean" => 0x7070D6,
        "deep_frozen_ocean" => 0x404090,
        "mushroom_fields" => 0xB05AA0,

        // ---- Highland ----
        "jagged_peaks" => 0xDCE4EE,
        "frozen_peaks" => 0xB4C8E6,
        "stony_peaks" => 0x8C8C8C,
        "meadow" => 0x83BB6D,
        "cherry_grove" => 0xF4B6D0,
        "grove" => 0x5E8A6E,
        "snowy_slopes" => 0xE8F0F0,
        "windswept_hills" => 0x606060,
        "windswept_gravelly_hills" => 0x888888,
        "windswept_forest" => 0x507050,

        // ---- Woodland ----
        "forest" => 0x056621,
        "flower_forest" => 0x2D8E49,
        "taiga" => 0x0B6659,
        "old_growth_pine_taiga" => 0x596651,
        "old_growth_spruce_taiga" => 0x818E79,
        "snowy_taiga" => 0x31554A,
        "birch_forest" => 0x307444,
        "old_growth_birch_forest" => 0x589C6C,
        "dark_forest" => 0x40511A,
        "pale_garden" => 0xA8B0A0,
        "jungle" => 0x537B09,
        "sparse_jungle" => 0x628B17,
        "bamboo_jungle" => 0x768E14,
        "dappled_forest" => 0xC86A1E,

        // ---- Wetland ----
        "river" => 0x0000FF,
        "frozen_river" => 0xA0A0FF,
        "swamp" => 0x07F9B2,
        "mangrove_swamp" => 0x67B86A,
        "beach" => 0xFADE55,
        "snowy_beach" => 0xFAF0C0,
        "stony_shore" => 0xA2A284,

        // ---- Flatland ----
        "plains" => 0x8DB360,
        "sunflower_plains" => 0xB5DB88,
        "snowy_plains" => 0xFFFFFF,
        "ice_spikes" => 0xB4DCDC,

        // ---- Arid land ----
        "desert" => 0xFA9418,
        "savanna" => 0xBDB25F,
        "savanna_plateau" => 0xA79D64,
        "windswept_savanna" => 0xE5DA87,
        "badlands" => 0xD94515,
        "wooded_badlands" => 0xB09765,
        "eroded_badlands" => 0xFF6D3D,

        // ---- Caves ----
        "deep_dark" => 0x0A2E3A,
        "dripstone_caves" => 0x8B6F4E,
        "lush_caves" => 0x6FA83C,
        "sulfur_caves" => 0xC8C03A,
        "ice_caves" => 0x9FD8F0,

        // ---- Void ----
        "the_void" => 0x000000,

        // ---- Nether ----
        "nether_wastes" => 0xBF3B3B,
        "soul_sand_valley" => 0x5E3830,
        "crimson_forest" => 0xDD0808,
        "warped_forest" => 0x49907B,
        "basalt_deltas" => 0x403636,

        // ---- End ----
        "the_end" => 0x8080FF,
        "small_end_islands" => 0x9A9AF0,
        "end_midlands" => 0xB8B880,
        "end_highlands" => 0xE0E0A8,
        "end_barrens" => 0x909060,

        _ => 0xFF00FF,
    })
}
/// Renders one tile to RGB bytes (TILE * TILE * 3).
fn render_tile(source: &BiomeSourceKind, key: TileKey) -> Vec<u8> {
    let stride = 1i32 << key.zoom;
    let origin_x = key.x * TILE as i32 * stride;
    let origin_z = key.z * TILE as i32 * stride;

    let mut sampler = source.chunk_sampler();
    let mut rgb = vec![0u8; TILE * TILE * 3];

    for (i, pixel) in rgb.chunks_exact_mut(3).enumerate() {
        // + stride / 2 samples the center of the pixel's area, not its corner
        let qx = origin_x + (i % TILE) as i32 * stride + stride / 2;
        let qz = origin_z + (i / TILE) as i32 * stride + stride / 2;
        let biome = sampler.sample(qx, SAMPLE_QUART_Y, qz);
        pixel.copy_from_slice(&biome_color(&biome.key.path));
    }
    rgb
}

const MAX_ZOOM: u8 = 16;

fn main() {
    let seed = 3699507159329829894;
    let source = BiomeSourceKind::overworld(seed);

    let mut args = env::args().skip(1);
    let n: i32 = args.next().unwrap().parse().unwrap(); // tiles per side
    let zoom: u8 = args.next().map_or(0, |s| s.parse().unwrap());
    assert!(zoom <= MAX_ZOOM, "zoom must be 0..={MAX_ZOOM}, got {zoom}");
    let half = n / 2;

    // Pixels per side, and how much world each pixel covers.
    let pixels_per_side = n as u64 * TILE as u64;
    let quarts_per_pixel = 1u64 << zoom;
    let side_quarts = pixels_per_side * quarts_per_pixel;
    let side_blocks = side_quarts * 4;

    let keys: Vec<TileKey> = (0..n)
        .flat_map(|tz| (0..n).map(move |tx| TileKey { zoom, x: tx - half, z: tz - half }))
        .collect();

    let start = Instant::now();
    let tiles: Vec<Vec<u8>> = keys.par_iter().map(|&k| render_tile(&source, k)).collect();
    println!("generated {} tiles in {:?}", tiles.len(), start.elapsed());
    println!(
        "image: {0}x{0} px, visible area: {1}x{1} quarts ({2}x{2} blocks)",
        pixels_per_side, side_quarts, side_blocks
    );

    // stitch tiles together
    let side = n as usize * TILE;
    let mut img = image::RgbImage::new(side as u32, side as u32);
    let buf: &mut [u8] = &mut img;
    for (i, tile) in tiles.iter().enumerate() {
        let (tx, tz) = (i % n as usize, i / n as usize);
        for row in 0..TILE {
            let dst = ((tz * TILE + row) * side + tx * TILE) * 3;
            buf[dst..dst + TILE * 3].copy_from_slice(&tile[row * TILE * 3..][..TILE * 3]);
        }
    }
    img.save("map.png").unwrap();
}