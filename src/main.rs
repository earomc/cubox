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
    env_logger::init();
    let matches = build_cmd().get_matches();
    match matches.subcommand() {
        Some(("gui", matches)) => {
            let seed = *matches.get_one::<i64>("seed").unwrap();
            let dimension = Dimension::from_name(matches.get_one::<String>("dimension").unwrap()).unwrap();
            let script = matches.get_one::<std::path::PathBuf>("script").cloned();
            run_gui(seed, dimension, script);
        }
        Some(("img", matches)) => img(matches),
        Some(("find", matches)) => find(matches),
        // started without arguments, e.g. by double clicking it
        _ => run_gui(0, Dimension::Overworld, None),
    }
}

/// `cubox img`: renders a map around the origin to `map.png`.
#[cfg(not(target_arch = "wasm32"))]
fn img(matches: &clap::ArgMatches) {
    let seed = *matches.get_one::<i64>("seed").unwrap();
    let tiles_per_side = *matches.get_one::<i32>("tiles").unwrap();
    let tile_size_px = *matches.get_one::<u32>("tile-size").unwrap() as usize;
    let level = *matches.get_one::<u8>("level").unwrap();
    let sample_height = *matches.get_one::<i32>("sample-height").unwrap();
    let samples = *matches.get_one::<u8>("samples").unwrap();
    let dimension = Dimension::from_name(matches.get_one::<String>("dimension").unwrap()).unwrap();

    let side_px = tiles_per_side as u64 * tile_size_px as u64;
    if side_px > cli::MAX_IMAGE_SIDE {
        build_cmd()
            .error(
                clap::error::ErrorKind::ValueValidation,
                format!(
                    "an image of {side_px}x{side_px} pixels is too big, tiles * tile-size can be at most {}",
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
                level,
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
    let shading = if matches.get_flag("exact") {
        Shading::exact(mean)
    } else {
        calibrate(&biome_source, keys[0], mean)
    };
    let tiles: Vec<Vec<u8>> = keys
        .par_iter()
        .map(|&k| render_tile(&biome_source, k, &shading, &|| true).unwrap().rgb)
        .collect();
    println!("generated {} tiles in {:?}", tiles.len(), start.elapsed());

    let side_quarts = side_px << level;
    println!(
        "image: {side_px}x{side_px} px, visible area: {side_quarts}x{side_quarts} quarts ({0}x{0} blocks), 1px = {1}x{1} blocks",
        side_quarts * 4,
        4u64 << level
    );

    // stitch the tiles together, row by row of pixels
    let side = side_px as usize;
    let mut img = image::RgbImage::new(side as u32, side as u32);
    let buf: &mut [u8] = &mut img;
    for (i, tile) in tiles.iter().enumerate() {
        let (tx, tz) = (i % tiles_per_side as usize, i / tiles_per_side as usize);
        for row in 0..tile_size_px {
            let dst = ((tz * tile_size_px + row) * side + tx * tile_size_px) * 3;
            buf[dst..dst + tile_size_px * 3].copy_from_slice(&tile[row * tile_size_px * 3..][..tile_size_px * 3]);
        }
    }
    if let Err(err) = img.save("map.png") {
        eprintln!("error: saving map.png: {err}");
        std::process::exit(1);
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
    use std::time::Duration;

    let fail = |message: String| -> ! {
        eprintln!("error: {message}");
        std::process::exit(2);
    };
    let text = match matches.get_one::<std::path::PathBuf>("file") {
        Some(path) => {
            std::fs::read_to_string(path).unwrap_or_else(|err| fail(format!("reading {}: {err}", path.display())))
        }
        None => matches.get_one::<String>("conditions").cloned().unwrap_or_default(),
    };
    let conditions = spec::parse(&text).unwrap_or_else(|err| fail(err));
    let seeds = if let Some(seed) = matches.get_one::<i64>("in-seed") {
        Seeds::Locations {
            seed: *seed,
            radius: *matches.get_one::<i32>("within").unwrap(),
            center: *matches.get_one::<[i32; 2]>("of").unwrap(),
        }
    } else if let Some(from) = matches.get_one::<i64>("from") {
        Seeds::From(*from)
    } else if let Some(path) = matches.get_one::<std::path::PathBuf>("seeds") {
        let text =
            std::fs::read_to_string(path).unwrap_or_else(|err| fail(format!("reading {}: {err}", path.display())));
        // blank lines are no seeds, like an empty seed field
        Seeds::List(text.lines().filter_map(cubox::seed::parse_seed).collect())
    } else {
        Seeds::Random(cubox::seed::random_seed() as u64)
    };
    let threads = matches
        .get_one::<usize>("threads")
        .copied()
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()));
    // an area has an end, so all of it unless asked otherwise
    let limit = matches
        .get_one::<u64>("limit")
        .copied()
        .unwrap_or(if matches.contains_id("in-seed") { u64::MAX } else { 10 });
    let tested = matches.get_one::<u64>("tested").copied();
    let top = *matches.get_one::<usize>("top").unwrap();

    let sort_best = |kept: &mut Vec<Match>| {
        kept.sort_by(|a, b| b.score.unwrap_or(f64::MIN).total_cmp(&a.score.unwrap_or(f64::MIN)));
    };
    let line = |found: &Match| {
        let score = found.score.map_or("-".to_owned(), |score| format!("{score:.0}"));
        let hits: Vec<String> = found.hits.iter().map(|hit| format!("{},{}", hit.x, hit.z)).collect();
        format!("{} {score} {}", found.seed, hits.join(" "))
    };
    let search = Search::start(&conditions, seeds, threads).unwrap_or_else(|err| fail(err));
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
                    // only the best are shown, a common match must not fill the memory
                    if kept.len() >= top.max(1) * 2 + 1024 {
                        sort_best(&mut kept);
                        kept.truncate(top);
                    }
                } else {
                    println!("{}", line(&found));
                }
            }
            // what is left of a finished search has come by now, workers send before saying so
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => ended = search.is_done(),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => ended = true,
        }
        if last_report.elapsed() >= Duration::from_secs(2) {
            last_report = Instant::now();
            eprintln!("{}", search.status().replace('\n', ", "));
        }
        if done(&search, ended, shown) {
            break;
        }
    }
    eprintln!("{}", search.status().replace('\n', ", "));
    search.stop();
    sort_best(&mut kept);
    for found in kept.iter().take(top) {
        println!("{}", line(found));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_gui(seed: i64, dimension: Dimension, script: Option<std::path::PathBuf>) {
    if let Err(err) = cubox::gui::run(seed, dimension, script) {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
