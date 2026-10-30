//! Real terrain heights from the game's noise, and a hillshade made from them.

use std::sync::Once;

use rustc_hash::FxHashMap;
use steel_registry::{
    init_vanilla_registry,
    template_pool::{TemplateData, TemplatePoolData},
};
use steel_utils::{
    Identifier,
    random::{Random, RandomSplitter, legacy_random::LegacyRandom, xoroshiro::Xoroshiro},
};
use steel_worldgen::{
    biomes::BiomeSourceKind,
    density::{DimensionNoises, NoiseSettings},
    density_functions::overworld::OverworldNoises,
    noise::LazyAquifer,
    noise_parameters::get_noise_parameters,
    structure::GenerationContext,
};

use crate::{dimension::Dimension, seed::generator_seed};

/// The game's registry, which only terrain and structures need, is set up on first use.
pub(crate) fn init_registry() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        init_vanilla_registry();
    });
}

/// The noises of one dimension and the splitter they were made with.
pub(crate) struct Noises<N> {
    pub(crate) noises: Box<N>,
    pub(crate) splitter: RandomSplitter,
}

impl<N: DimensionNoises> Noises<N> {
    pub(crate) fn new(seed: i64) -> Self {
        let seed = generator_seed(seed);
        // the nether uses Java's random numbers
        let splitter = if N::Settings::LEGACY_RANDOM_SOURCE {
            LegacyRandom::from_seed(seed).next_positional()
        } else {
            Xoroshiro::from_seed(seed).next_positional()
        };
        let noises = Box::new(N::create(seed, &splitter, &get_noise_parameters()));
        Self { noises, splitter }
    }
}

/// Answers how high the terrain of a world's overworld is. The other dimensions have no relief:
/// the top of the nether is just its roof, and the end has no estimate of its surface to start
/// from, scanning its columns takes minutes per tile.
pub struct Terrain {
    seed: i64,
    source: BiomeSourceKind,
    noises: Noises<OverworldNoises>,
}

impl Terrain {
    pub fn new(seed: i64) -> Self {
        init_registry();
        Self {
            seed: generator_seed(seed) as i64,
            source: Dimension::Overworld.biome_source(seed),
            noises: Noises::new(seed),
        }
    }

    /// The height at `size * size` columns, `stride` blocks apart, in rows, or None if `keep_going`,
    /// which is asked for every chunk, said no. With `ocean_floor` water doesn't count as surface,
    /// like the game's `OCEAN_FLOOR_WG`, otherwise it does, like `WORLD_SURFACE_WG`.
    pub fn heights(
        &self,
        origin: [i32; 2],
        stride: i32,
        size: usize,
        ocean_floor: bool,
        keep_going: &dyn Fn() -> bool,
    ) -> Option<Vec<i32>> {
        let noises = &self.noises;
        // columns of a chunk share its caches, which is most of the work of a lone column
        let mut columns: Vec<(i32, i32, usize)> = (0..size * size)
            .map(|i| {
                let x = origin[0] + (i % size) as i32 * stride;
                let z = origin[1] + (i / size) as i32 * stride;
                (x, z, i)
            })
            .collect();
        columns.sort_unstable_by_key(|&(x, z, _)| (z >> 4, x >> 4));

        let (template_pools, templates) = (
            FxHashMap::<Identifier, TemplatePoolData>::default(),
            FxHashMap::<Identifier, TemplateData>::default(),
        );
        let mut heights = vec![0; size * size];
        for chunk in columns.chunk_by(|a, b| (a.0 >> 4, a.1 >> 4) == (b.0 >> 4, b.1 >> 4)) {
            if !keep_going() {
                return None;
            }
            let (chunk_x, chunk_z) = (chunk[0].0 >> 4, chunk[0].1 >> 4);
            let mut sampler = self.source.chunk_sampler();
            let mut height_cache = Default::default();
            let mut aquifer = LazyAquifer::new(chunk_x * 16, chunk_z * 16, &noises.splitter, &*noises.noises);
            let (mut surface_y_cache, mut height_cache_grid_ready) = (None, false);
            let mut ctx = GenerationContext::<'_, '_, OverworldNoises>::new(
                self.seed,
                chunk_x,
                chunk_z,
                <OverworldNoises as DimensionNoises>::Settings::SEA_LEVEL,
                &noises.noises,
                &noises.splitter,
                &template_pools,
                &templates,
                &mut sampler,
                &mut height_cache,
                &mut aquifer,
                &mut surface_y_cache,
                &mut height_cache_grid_ready,
            );
            for &(x, z, i) in chunk {
                // base_height is twice as fast but starts scanning a bit above an estimate of the surface,
                // which is wrong for almost half of the columns, oceans most of all, by up to 150 blocks
                heights[i] = ctx.base_height_full(x, z, ocean_floor);
            }
        }
        Some(heights)
    }
}

/// How strongly slopes are shaded: a 45° slope gets this much of the maximum opacity.
const SHADE_STRENGTH: f32 = 0.9;
const MAX_ALPHA: f32 = 0.55;
/// Samples further apart than this see lower slopes of the same terrain, which is made up for.
const FULL_SLOPE_DISTANCE: f32 = 4.0;
const SEA_LEVEL: i32 = 63;

impl Terrain {
    /// A hillshade tile of `size * size` pixels, `stride` blocks apart, with a real height sampled
    /// every `step` pixels and the shading between them interpolated.
    pub fn relief(
        &self,
        origin: [i32; 2],
        stride: i32,
        size: usize,
        step: usize,
        ocean_floor: bool,
        keep_going: &dyn Fn() -> bool,
    ) -> Option<Vec<u8>> {
        let distance = stride * step as i32;
        let samples = size / step + 2;
        if !ocean_floor {
            let heights = self.heights(origin, distance, samples, false, keep_going)?;
            return Some(hillshade(&heights, size, step, stride));
        }
        // a sample around each for smoothing
        let around = [origin[0] - distance, origin[1] - distance];
        let heights = self.heights(around, distance, samples + 2, true, keep_going)?;
        Some(hillshade(&smooth_sea_floor(&heights, samples), size, step, stride))
    }
}

/// Caves opening into the sea floor make single samples far deeper than those around them, which
/// shade as speckles all over the oceans. Takes the median of each sample under water and the 4
/// next to it. `heights` has `samples + 2` rows and columns, the result `samples`, without the edges.
fn smooth_sea_floor(heights: &[i32], samples: usize) -> Vec<i32> {
    let width = samples + 2;
    let mut smoothed = Vec::with_capacity(samples * samples);
    for z in 1..=samples {
        for x in 1..=samples {
            let i = z * width + x;
            smoothed.push(if heights[i] < SEA_LEVEL {
                let mut near = [
                    heights[i],
                    heights[i - 1],
                    heights[i + 1],
                    heights[i - width],
                    heights[i + width],
                ];
                near.sort_unstable();
                near[2]
            } else {
                heights[i]
            });
        }
    }
    smoothed
}

/// Hillshade with the light in the north west, as RGBA to draw over the map: slopes facing the
/// light are white, the others black, the more the steeper. `heights` has `size / step + 2` rows
/// and columns, `step` pixels apart, which are `stride` blocks each. Tiles side by side share their
/// samples, the 2 extra rows and columns make the slopes at the edges the same.
pub fn hillshade(heights: &[i32], size: usize, step: usize, stride: i32) -> Vec<u8> {
    let samples = size / step + 2;
    let distance = (stride * step as i32) as f32;
    let gain = (distance / FULL_SLOPE_DISTANCE).powf(0.25).max(1.0);
    let at = |x: usize, z: usize| heights[z * samples + x] as f32;
    // height gained going south east: a positive slope faces the light
    let slope_at = |x: usize, z: usize| (at(x + 1, z) - at(x, z) + at(x, z + 1) - at(x, z)) / (2.0 * distance);
    let mut rgba = Vec::with_capacity(size * size * 4);
    for z in 0..size {
        let (sample_z, t_z) = (z / step, (z % step) as f32 / step as f32);
        for x in 0..size {
            let (sample_x, t_x) = (x / step, (x % step) as f32 / step as f32);
            // exactly the sample if there is one for this pixel
            let mut slope = slope_at(sample_x, sample_z);
            if step > 1 {
                let top = slope + (slope_at(sample_x + 1, sample_z) - slope) * t_x;
                let left = slope_at(sample_x, sample_z + 1);
                let bottom = left + (slope_at(sample_x + 1, sample_z + 1) - left) * t_x;
                slope = top + (bottom - top) * t_z;
            }
            let alpha = (slope.abs() * gain * SHADE_STRENGTH).min(1.0) * MAX_ALPHA;
            let shade = if slope > 0.0 { 255 } else { 0 };
            rgba.extend_from_slice(&[shade, shade, shade, (alpha * 255.0) as u8]);
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slope_facing_the_light_is_white_and_flat_is_clear() {
        // rising to the south east
        let rising: Vec<i32> = (0..16).map(|i| i % 4 + i / 4).collect();
        assert_eq!(hillshade(&rising, 2, 1, 1)[0], 255);
        let falling: Vec<i32> = rising.iter().map(|h| -h).collect();
        assert_eq!(hillshade(&falling, 2, 1, 1)[0], 0);
        assert!(hillshade(&[64; 16], 2, 1, 1).chunks(4).all(|px| px[3] == 0));
    }

    #[test]
    fn sampled_tiles_match_their_samples_and_neighbours() {
        let terrain = Terrain::new(12345);
        let size = 32;
        let tile = |x: i32, step| {
            terrain
                .relief([1000 + x * size as i32 * 64, 1000], 64, size, step, false, &|| true)
                .unwrap()
        };
        // shading continues over the edge between tiles
        let (left, right) = (tile(0, 8), tile(1, 8));
        for z in 0..size {
            let signed = |rgba: &[u8], x: usize| {
                let px = &rgba[(z * size + x) * 4..];
                (if px[0] > 0 { 1.0 } else { -1.0 }) * px[3] as f32
            };
            assert!(
                (signed(&left, size - 1) - signed(&right, 0)).abs() < 40.0,
                "row {z}: {} {}",
                signed(&left, size - 1),
                signed(&right, 0)
            );
        }
    }

    #[test]
    fn lone_pits_in_the_sea_floor_are_filled_but_land_stays() {
        let flat = |at: i32| {
            let mut heights = vec![40; 25];
            heights[12] = at;
            smooth_sea_floor(&heights, 3)[4]
        };
        assert_eq!(flat(5), 40);
        assert_eq!(flat(90), 90);
    }

    #[test]
    fn ocean_floor_is_the_games_and_below_the_water() {
        let terrain = Terrain::new(12345);
        let (mut water, mut columns) = (0, 0);
        for x in (-20_000..20_000).step_by(2000) {
            let at = |ocean_floor| terrain.heights([x, 3000], 16, 1, ocean_floor, &|| true).unwrap()[0];
            let (surface, floor) = (at(false), at(true));
            assert!(floor <= surface, "at {x}: floor {floor} surface {surface}");
            columns += 1;
            water += (floor < surface) as i32;
            // the water is at sea level, and a surface can't be under it
            assert!(surface >= 63, "at {x}: surface {surface}");
        }
        assert!(water > 0 && water < columns, "{water} of {columns}");
    }

    #[test]
    fn heights_are_deterministic_and_sane() {
        let terrain = Terrain::new(12345);
        let heights = terrain.heights([1000, 1000], 64, 16, false, &|| true).unwrap();
        assert_eq!(
            heights,
            Terrain::new(12345)
                .heights([1000, 1000], 64, 16, false, &|| true)
                .unwrap()
        );
        assert!(heights.iter().all(|h| (-64..320).contains(h)), "{heights:?}");
        // land here, not a flat sea
        assert!(heights.iter().min() != heights.iter().max());
        assert!(terrain.heights([0, 0], 4, 16, false, &|| false).is_none());
    }
}
