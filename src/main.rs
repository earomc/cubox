mod biome_colors;
mod cli;
mod tile_renderer;
mod gui;
use rayon::prelude::*;
use std::time::Instant;
use steel_worldgen::biomes::BiomeSourceKind;

use crate::{cli::build_cmd, tile_renderer::{TileKey, render_tile}};

fn main() {
    let mut cmd = build_cmd();
    let matches = cmd.get_matches_mut();
    match matches.subcommand() {
        Some(("gui", _)) => {
            println!("launching gui");
            crate::gui::run();
        },
        Some(("img", matches)) => {
            let seed = *matches.get_one::<u64>("seed").unwrap();
            let tiles_per_side = *matches.get_one::<i32>("tiles").unwrap();
            let tile_size_px = *matches.get_one::<usize>("tile-size").unwrap();
            let scale = *matches.get_one::<u8>("scale").unwrap();
            let sample_height = *matches.get_one::<i32>("sample-height").unwrap();

            let half = tiles_per_side / 2;
            let keys: Vec<TileKey> = (0..tiles_per_side)
                .flat_map(|tz| {
                    (0..tiles_per_side).map(move |tx| TileKey {
                        scale,
                        sample_height,
                        tile_size_px,
                        x: tx - half,
                        z: tz - half,
                    })
                })
                .collect();

            let biome_source = BiomeSourceKind::overworld(seed);
            let start = Instant::now();
            let tiles: Vec<Vec<u8>> = keys.par_iter().map(|&k| render_tile(&biome_source, k)).collect();
            println!("generated {} tiles in {:?}", tiles.len(), start.elapsed());

            // Pixels per side, and how much world each pixel covers.
            let pixels_per_side = tiles_per_side as u64 * tile_size_px as u64;
            let quarts_per_pixel = 1u64 << scale;
            let side_quarts = pixels_per_side * quarts_per_pixel;
            let side_blocks = side_quarts * 4;
            println!(
                "image: {0}x{0} px, visible area: {1}x{1} quarts ({2}x{2} blocks), 1px = {3}x{3} blocks",
                pixels_per_side, side_quarts, side_blocks, side_blocks / pixels_per_side
            );

            // stitch tiles together
            let side_px = tiles_per_side as usize * tile_size_px;
            let mut img = image::RgbImage::new(side_px as u32, side_px as u32);
            let buf: &mut [u8] = &mut img;
            for (i, tile) in tiles.iter().enumerate() {
                let (tx, tz) = (i % tiles_per_side as usize, i / tiles_per_side as usize); // coordinates of tiles in grid
                for row in 0..tile_size_px { // horizontal row of pixels
                    // computing start location of pixel row in buffer
                    let dst = ((tz * tile_size_px + row) * side_px + tx * tile_size_px) * 3;

                    // copying
                    buf[dst..dst + tile_size_px * 3].copy_from_slice(&tile[row * tile_size_px * 3..][..tile_size_px * 3]);
                }
            }
            img.save("map.png").unwrap();
        },
        _ => cmd.print_help().unwrap(),
    }
}
