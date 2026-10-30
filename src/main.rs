//! The desktop app and command line. Browsers use the library directly, see `src/web.rs`.
#![cfg_attr(target_arch = "wasm32", allow(unused))]

#[cfg(not(target_arch = "wasm32"))]
mod cli;

#[cfg(not(target_arch = "wasm32"))]
use cubox::{
    dimension::Dimension,
    tile_renderer::{Shading, TileKey, calibrate, render_tile, world_mean},
};
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

#[cfg(not(target_arch = "wasm32"))]
use crate::cli::build_cmd;

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let matches = build_cmd().get_matches();
    match matches.subcommand() {
        Some(("gui", matches)) => {
            let seed = *matches.get_one::<i64>("seed").unwrap();
            let dimension = Dimension::from_name(matches.get_one::<String>("dimension").unwrap()).unwrap();
            let script = matches.get_one::<std::path::PathBuf>("script").cloned();
            run_gui(seed, dimension, script);
        }
        Some(("img", matches)) => {
            let seed = *matches.get_one::<i64>("seed").unwrap();
            let tiles_per_side = *matches.get_one::<i32>("tiles").unwrap();
            let tile_size_px = *matches.get_one::<usize>("tile-size").unwrap();
            let scale = *matches.get_one::<u8>("scale").unwrap();
            let sample_height = *matches.get_one::<i32>("sample-height").unwrap();
            let samples = *matches.get_one::<u8>("samples").unwrap();
            let dimension = Dimension::from_name(matches.get_one::<String>("dimension").unwrap()).unwrap();
            let exact = matches.get_flag("exact");

            let half = tiles_per_side / 2;
            let keys: Vec<TileKey> = (0..tiles_per_side)
                .flat_map(|tz| {
                    (0..tiles_per_side).map(move |tx| TileKey {
                        scale,
                        sample_height,
                        tile_size_px,
                        samples,
                        x: tx - half,
                        z: tz - half,
                    })
                })
                .collect();

            let biome_source = dimension.biome_source(seed);
            let start = Instant::now();
            let mean = world_mean(&biome_source, sample_height);
            let shading = if exact {
                Shading::exact(mean)
            } else {
                calibrate(&biome_source, keys[0], mean)
            };
            let tiles: Vec<Vec<u8>> = keys
                .par_iter()
                .map(|&k| render_tile(&biome_source, k, &shading, &|| true).unwrap().rgb)
                .collect();
            println!("generated {} tiles in {:?}", tiles.len(), start.elapsed());

            // Pixels per side, and how much world each pixel covers.
            let pixels_per_side = tiles_per_side as u64 * tile_size_px as u64;
            let quarts_per_pixel = 1u64 << scale;
            let side_quarts = pixels_per_side * quarts_per_pixel;
            let side_blocks = side_quarts * 4;
            println!(
                "image: {0}x{0} px, visible area: {1}x{1} quarts ({2}x{2} blocks), 1px = {3}x{3} blocks",
                pixels_per_side,
                side_quarts,
                side_blocks,
                side_blocks / pixels_per_side
            );

            // stitch tiles together
            let side_px = tiles_per_side as usize * tile_size_px;
            let mut img = image::RgbImage::new(side_px as u32, side_px as u32);
            let buf: &mut [u8] = &mut img;
            for (i, tile) in tiles.iter().enumerate() {
                let (tx, tz) = (i % tiles_per_side as usize, i / tiles_per_side as usize); // coordinates of tiles in grid
                for row in 0..tile_size_px {
                    // horizontal row of pixels
                    // computing start location of pixel row in buffer
                    let dst = ((tz * tile_size_px + row) * side_px + tx * tile_size_px) * 3;

                    // copying
                    buf[dst..dst + tile_size_px * 3]
                        .copy_from_slice(&tile[row * tile_size_px * 3..][..tile_size_px * 3]);
                }
            }
            img.save("map.png").unwrap();
        }
        // started without arguments, e.g. by double clicking it
        _ => run_gui(0, Dimension::Overworld, None),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_gui(seed: i64, dimension: Dimension, script: Option<std::path::PathBuf>) {
    if let Err(err) = cubox::gui::run(seed, dimension, script) {
        eprintln!("error: {err:#}");
    }
}
