//! The desktop app and command line. Browsers use the library directly, see `src/web.rs`.
#![cfg_attr(target_arch = "wasm32", allow(unused))]

#[cfg(not(target_arch = "wasm32"))]
mod cli;

#[cfg(not(target_arch = "wasm32"))]
use cubox::{
    dimension::Dimension,
    tile_renderer::{Layer, Shading, TileKey, calibrate, render_tile, world_mean},
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
            let tile_size_px = *matches.get_one::<u32>("tile-size").unwrap() as usize;
            let scale = *matches.get_one::<u8>("scale").unwrap();
            let sample_height = *matches.get_one::<i32>("sample-height").unwrap();
            let samples = *matches.get_one::<u8>("samples").unwrap();
            let dimension = Dimension::from_name(matches.get_one::<String>("dimension").unwrap()).unwrap();
            let exact = matches.get_flag("exact");

            let side = tiles_per_side as u64 * tile_size_px as u64;
            if side > cli::MAX_IMAGE_SIDE {
                build_cmd()
                    .error(
                        clap::error::ErrorKind::ValueValidation,
                        format!(
                            "an image of {side}x{side} pixels is too big, tiles * tile-size can be at most {}",
                            cli::MAX_IMAGE_SIDE
                        ),
                    )
                    .exit();
            }

            let half = tiles_per_side / 2;
            let keys: Vec<TileKey> = (0..tiles_per_side)
                .flat_map(|tz| {
                    (0..tiles_per_side).map(move |tx| TileKey {
                        layer: Layer::Biome,
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
        Some(("find", matches)) => find(matches),
        // started without arguments, e.g. by double clicking it
        _ => run_gui(0, Dimension::Overworld, None),
    }
}

/// `cubox find`: matches on stdout as they come, one per line, how it is going on stderr.
#[cfg(not(target_arch = "wasm32"))]
fn find(matches: &clap::ArgMatches) {
    use cubox::seedfind::{
        engine::Match,
        run::{Search, Seeds},
        spec,
    };
    use std::{sync::atomic::Ordering, time::Duration};

    let fail = |message: String| -> ! {
        eprintln!("error: {message}");
        std::process::exit(2);
    };
    let text = match matches.get_one::<std::path::PathBuf>("file") {
        Some(path) => {
            std::fs::read_to_string(path).unwrap_or_else(|err| fail(format!("reading {}: {err}", path.display())))
        }
        None => matches.get_one::<String>("cards").cloned().unwrap_or_default(),
    };
    let cards = spec::parse(&text).unwrap_or_else(|err| fail(err));
    if cards.is_empty() {
        fail("no conditions".to_owned());
    }
    let seeds = if let Some(seed) = matches.get_one::<i64>("in-seed") {
        Seeds::Locations(*seed)
    } else if let Some(from) = matches.get_one::<i64>("from") {
        Seeds::From(*from)
    } else if let Some(path) = matches.get_one::<std::path::PathBuf>("seeds") {
        let text =
            std::fs::read_to_string(path).unwrap_or_else(|err| fail(format!("reading {}: {err}", path.display())));
        Seeds::List(
            text.split_whitespace()
                .map(|word| word.parse().unwrap_or_else(|_| fail(format!("`{word}` is not a seed"))))
                .collect(),
        )
    } else {
        Seeds::Random(cubox::seed::random_seed() as u64)
    };
    let threads = matches
        .get_one::<usize>("threads")
        .copied()
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()));
    let limit = *matches.get_one::<u64>("limit").unwrap();
    let tested = matches.get_one::<u64>("tested").copied();
    let top = *matches.get_one::<usize>("top").unwrap();

    let line = |found: &Match| {
        let score = found.score.map_or("-".to_owned(), |score| format!("{score:.0}"));
        let hits: Vec<String> = found.hits.iter().map(|hit| format!("{},{}", hit.x, hit.z)).collect();
        format!("{} {score} {}", found.seed, hits.join(" "))
    };
    let search = Search::start(&cards, seeds, threads);
    let start = Instant::now();
    let (mut shown, mut last_report, mut kept): (u64, _, Vec<Match>) = (0, Instant::now(), Vec::new());
    let done = |search: &Search, ended: bool, shown: u64| {
        ended || tested.map_or(shown >= limit, |tested| search.counters.covered() >= tested)
    };
    loop {
        let mut ended = false;
        match search.results.recv_timeout(Duration::from_millis(100)) {
            Ok(found) => {
                shown += 1;
                if tested.is_some() {
                    kept.push(found);
                } else {
                    println!("{}", line(&found));
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => ended = true,
        }
        if last_report.elapsed() >= Duration::from_secs(2) {
            last_report = Instant::now();
            let covered = search.counters.covered();
            let rate = covered as f64 / start.elapsed().as_secs_f64();
            let odds = match search.counters.one_in() {
                Some(one_in) => format!("about 1 in {one_in:.0}, {:.0} s each", one_in / rate),
                None => format!("rarer than 1 in {:.0} so far", search.counters.at_least_one_in()),
            };
            eprintln!(
                "{covered} seeds, {rate:.0}/s, {} checked with a world, {shown} matches, {odds}, at {}",
                search.counters.seeds.load(Ordering::Relaxed),
                search.counters.position.load(Ordering::Relaxed),
            );
        }
        if done(&search, ended, shown) {
            break;
        }
    }
    search.stop();
    kept.sort_by(|a, b| b.score.unwrap_or(f64::MIN).total_cmp(&a.score.unwrap_or(f64::MIN)));
    for found in kept.iter().take(top) {
        println!("{}", line(found));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_gui(seed: i64, dimension: Dimension, script: Option<std::path::PathBuf>) {
    if let Err(err) = cubox::gui::run(seed, dimension, script) {
        eprintln!("error: {err:#}");
    }
}
