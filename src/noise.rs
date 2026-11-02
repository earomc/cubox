//! The climate noises that decide biomes, for looking at them and for searching.

use steel_utils::{
    BlockStateId,
    random::{Random, xoroshiro::Xoroshiro},
};
use steel_worldgen::{
    biomes::{NetherClimateSampler, OverworldClimateSampler},
    density::DimensionNoises,
    density_functions::{
        nether::NetherColumnCache,
        overworld::{self as generated, OverworldColumnCache, OverworldNoises},
    },
    noise::OreVeinifier,
    noise_parameters::get_noise_parameters,
};

use crate::{dimension::Dimension, seed::generator_seed, tile_renderer::TileKey};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum NoiseParameter {
    Temperature,
    Humidity,
    Continentalness,
    Erosion,
    Weirdness,
    Depth,
}

impl NoiseParameter {
    pub const ALL: [Self; 6] = [
        Self::Temperature,
        Self::Humidity,
        Self::Continentalness,
        Self::Erosion,
        Self::Weirdness,
        Self::Depth,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Temperature => "temperature",
            Self::Humidity => "humidity",
            Self::Continentalness => "continentalness",
            Self::Erosion => "erosion",
            Self::Weirdness => "weirdness",
            Self::Depth => "depth",
        }
    }
}

/// The ores of large veins.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ore {
    Copper,
    Iron,
}

impl Ore {
    /// The heights the game's rule has its veins at, in blocks.
    pub fn heights(self) -> std::ops::Range<i32> {
        match self {
            Self::Copper => 0..50,
            Self::Iron => -60..-8,
        }
    }
}

/// Noise tiles are sampled every this many pixels and blended between, as noise changes slowly.
const TILE_STEP: usize = 4;

/// The noises of a world, in a dimension that has them.
pub enum NoiseField {
    Overworld(Box<OverworldClimateSampler>),
    Nether(Box<NetherClimateSampler>),
}

/// Keeps what columns have in common, one per thread.
pub struct NoiseCache {
    overworld: OverworldColumnCache,
    nether: NetherColumnCache,
}

impl Default for NoiseCache {
    fn default() -> Self {
        Self {
            overworld: OverworldColumnCache::new(),
            nether: NetherColumnCache::new(),
        }
    }
}

/// The parameters a dimension has. The end picks its biomes without any.
pub fn parameters_of(dimension: Dimension) -> &'static [NoiseParameter] {
    match dimension {
        Dimension::Overworld => &NoiseParameter::ALL,
        Dimension::Nether => &[NoiseParameter::Temperature, NoiseParameter::Humidity],
        Dimension::End => &[],
    }
}

impl NoiseField {
    pub fn new(dimension: Dimension, seed: i64) -> Option<Self> {
        let seed = generator_seed(seed);
        match dimension {
            Dimension::Overworld => Some(Self::Overworld(Box::new(OverworldClimateSampler::new(seed)))),
            Dimension::Nether => Some(Self::Nether(Box::new(NetherClimateSampler::new(seed)))),
            Dimension::End => None,
        }
    }

    /// The value at a quart position, in the range the game uses.
    pub fn value(&self, cache: &mut NoiseCache, parameter: NoiseParameter, quart: [i32; 3]) -> f32 {
        let point = match self {
            Self::Overworld(sampler) => sampler.sample(quart[0], quart[1], quart[2], &mut cache.overworld),
            Self::Nether(sampler) => sampler.sample(quart[0], quart[1], quart[2], &mut cache.nether),
        };
        let value = match parameter {
            NoiseParameter::Temperature => point.temperature,
            NoiseParameter::Humidity => point.humidity,
            NoiseParameter::Continentalness => point.continentalness,
            NoiseParameter::Erosion => point.erosion,
            NoiseParameter::Weirdness => point.weirdness,
            NoiseParameter::Depth => point.depth,
        };
        value as f32 / 10_000.0
    }

    /// The RGBA pixels of a tile showing a parameter.
    pub fn tile(&self, parameter: NoiseParameter, key: TileKey, keep_going: &dyn Fn() -> bool) -> Option<Vec<u8>> {
        let size = key.tile_size_px;
        let (origin_x, origin_z) = key.origin();
        let stride = key.stride();
        let qy = key.sample_height >> 2;
        let samples = size / TILE_STEP + 2;
        let mut cache = NoiseCache::default();
        let mut values = Vec::with_capacity(samples * samples);
        for row in 0..samples {
            if !keep_going() {
                return None;
            }
            for column in 0..samples {
                let quart = [
                    origin_x + (column * TILE_STEP) as i32 * stride,
                    qy,
                    origin_z + (row * TILE_STEP) as i32 * stride,
                ];
                values.push(self.value(&mut cache, parameter, quart));
            }
        }
        let mut rgba = Vec::with_capacity(size * size * 4);
        for z in 0..size {
            let (row, fz) = (z / TILE_STEP, (z % TILE_STEP) as f32 / TILE_STEP as f32);
            for x in 0..size {
                let (column, fx) = (x / TILE_STEP, (x % TILE_STEP) as f32 / TILE_STEP as f32);
                let at = |c: usize, r: usize| values[r * samples + c];
                let top = at(column, row) * (1.0 - fx) + at(column + 1, row) * fx;
                let bottom = at(column, row + 1) * (1.0 - fx) + at(column + 1, row + 1) * fx;
                let [r, g, b] = color_of(top * (1.0 - fz) + bottom * fz);
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        Some(rgba)
    }
}

/// How much vein noise is needed for a vein, what the game calls veininess, see its `OreVeinifier`.
const VEININESS: f32 = 0.4;
/// The heights looked at for veins of each ore, inside what the game allows and away from its
/// ends, where it makes veins thinner. Copper is at y 0 to 50, iron at y -60 to -8.
const COPPER_HEIGHTS: [i32; 3] = [15, 25, 35];
const IRON_HEIGHTS: [i32; 3] = [-44, -34, -24];

/// Blocks of a vein looked at around a spot, every this many blocks, across all of the ore's heights.
const ORE_SCAN_RADIUS: i32 = 8;
const ORE_SCAN_STEP: usize = 2;

/// What is underground in an overworld, from its noises: ore veins and solid blocks. That is before caves are carved out of it
/// and features are placed, which can take things away again.
pub struct Underground {
    noises: Box<OverworldNoises>,
    veinifier: OreVeinifier,
}

impl Underground {
    pub fn new(seed: i64) -> Self {
        crate::terrain::init_registry();
        let seed = generator_seed(seed);
        let mut rng = Xoroshiro::from_seed(seed);
        let splitter = rng.next_positional();
        Self {
            noises: Box::new(OverworldNoises::create(seed, &splitter, &get_noise_parameters())),
            veinifier: OreVeinifier::new(&splitter),
        }
    }

    /// Whether the terrain is solid at a block, not air or water.
    pub fn is_solid(&self, cache: &mut NoiseCache, x: i32, y: i32, z: i32) -> bool {
        cache.overworld.ensure(x, z, &self.noises);
        self.noises.router_final_density(&mut cache.overworld, x, y, z) > 0.0
    }

    /// Whether the game's rule puts ore or a raw ore block of this ore at a block of solid stone.
    /// Copper and iron have the heights of their rule, the blocks of the rule itself don't matter.
    fn has_ore(&self, cache: &mut NoiseCache, ore: Ore, x: i32, y: i32, z: i32) -> bool {
        let density = self.vein_density(cache, ore, x, y, z);
        if density <= 0.0 || !self.is_solid(cache, x, y, z) {
            return false;
        }
        let at = (x as f64, y as f64, z as f64);
        let richness = match ore {
            Ore::Copper => {
                generated::router_material_ore_vein_0_richness(&self.noises, &cache.overworld, at.0, at.1, at.2)
            }
            Ore::Iron => {
                generated::router_material_ore_vein_1_richness(&self.noises, &cache.overworld, at.0, at.1, at.2)
            }
        };
        let (ore_block, filler) = (BlockStateId(1), BlockStateId(0));
        let gap = || match ore {
            Ore::Copper => {
                generated::router_material_ore_vein_0_filler_gap(&self.noises, &cache.overworld, at.0, at.1, at.2)
            }
            Ore::Iron => {
                generated::router_material_ore_vein_1_filler_gap(&self.noises, &cache.overworld, at.0, at.1, at.2)
            }
        };
        self.veinifier
            .try_apply_material_rule(x, y, z, density, richness, gap, ore_block, ore_block, filler, 0.02)
            == Some(ore_block)
    }

    /// How many blocks of ore a look around a spot finds. It looks at one block in 8.
    pub fn ore_near(&self, cache: &mut NoiseCache, ore: Ore, x: i32, z: i32) -> u32 {
        let heights = ore.heights();
        let mut found = 0;
        for dz in (-ORE_SCAN_RADIUS..ORE_SCAN_RADIUS).step_by(ORE_SCAN_STEP) {
            for dx in (-ORE_SCAN_RADIUS..ORE_SCAN_RADIUS).step_by(ORE_SCAN_STEP) {
                for y in heights.clone().step_by(ORE_SCAN_STEP) {
                    found += self.has_ore(cache, ore, x + dx, y, z + dz) as u32;
                }
            }
        }
        found
    }

    /// The ore whose veins run under a block position, if any. This is the noise that decides where
    /// veins are, the game then only keeps some blocks in them.
    pub fn at(&self, x: i32, z: i32) -> Option<Ore> {
        let veininess = |y: i32| {
            self.noises
                .n_ore_veininess
                .get_value(x as f64 * 1.5, y as f64 * 1.5, z as f64 * 1.5)
        };
        if COPPER_HEIGHTS.into_iter().any(|y| veininess(y) >= VEININESS) {
            Some(Ore::Copper)
        } else if IRON_HEIGHTS.into_iter().any(|y| veininess(y) <= -VEININESS) {
            Some(Ore::Iron)
        } else {
            None
        }
    }

    /// The game's own rule for an ore can place something at a block where this is above 0.
    fn vein_density(&self, cache: &mut NoiseCache, ore: Ore, x: i32, y: i32, z: i32) -> f32 {
        cache.overworld.ensure(x, z, &self.noises);
        let at = (x as f64, y as f64, z as f64);
        match ore {
            Ore::Copper => {
                generated::router_material_ore_vein_0_density(&self.noises, &cache.overworld, at.0, at.1, at.2)
            }
            Ore::Iron => {
                generated::router_material_ore_vein_1_density(&self.noises, &cache.overworld, at.0, at.1, at.2)
            }
        }
    }
}

/// The color of a value: blue for low, pale for 0, red for high.
pub fn color_of(value: f32) -> [u8; 3] {
    const LOW: [f32; 3] = [33.0, 72.0, 170.0];
    const MIDDLE: [f32; 3] = [240.0, 238.0, 225.0];
    const HIGH: [f32; 3] = [178.0, 30.0, 40.0];
    let t = value.clamp(-1.0, 1.0);
    let (end, t) = if t < 0.0 { (LOW, -t) } else { (HIGH, t) };
    [0, 1, 2].map(|i| (MIDDLE[i] + (end[i] - MIDDLE[i]) * t) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tile_renderer::Layer;

    #[test]
    fn values_are_those_the_biomes_are_chosen_with() {
        // where the sampler says deep ocean climate the biome is one of the oceans
        let source = Dimension::Overworld.biome_source(7);
        let mut sampler = source.chunk_sampler();
        let field = NoiseField::new(Dimension::Overworld, 7).unwrap();
        let mut cache = NoiseCache::default();
        let (mut land, mut ocean) = (0, 0);
        for i in 0..10_000 {
            let quart = [(i % 100 - 50) * 300, 64, (i / 100 - 50) * 300];
            let value = field.value(&mut cache, NoiseParameter::Continentalness, quart);
            let biome = sampler.sample(quart[0], quart[1], quart[2]);
            if value < -0.5 {
                ocean += 1;
                assert!(
                    biome.key.path.contains("ocean") || &*biome.key.path == "mushroom_fields",
                    "{} at {value}",
                    biome.key.path
                );
            } else {
                land += 1;
            }
        }
        assert!(land > 0 && ocean > 0);
    }

    #[test]
    fn a_tile_has_the_values_of_the_field() {
        let field = NoiseField::new(Dimension::Overworld, 7).unwrap();
        let key = TileKey {
            layer: Layer::Noise(NoiseParameter::Erosion),
            level: 3,
            sample_height: 256,
            tile_size_px: 128,
            samples: 1,
            x: 2,
            z: -1,
        };
        let rgba = field.tile(NoiseParameter::Erosion, key, &|| true).unwrap();
        assert_eq!(rgba.len(), 128 * 128 * 4);
        let (x, z) = key.origin();
        let value = field.value(&mut NoiseCache::default(), NoiseParameter::Erosion, [x, 64, z]);
        assert_eq!(rgba[..3], color_of(value));
    }

    #[test]
    fn veins_are_rare_and_of_both_ores() {
        let field = Underground::new(12345);
        let (mut copper, mut iron, mut none) = (0, 0, 0);
        for i in 0..40_000 {
            match field.at((i % 200 - 100) * 64, (i / 200 - 100) * 64) {
                Some(Ore::Copper) => copper += 1,
                Some(Ore::Iron) => iron += 1,
                None => none += 1,
            }
        }
        assert!(copper > 200 && iron > 200 && none > 20_000, "{copper} {iron} {none}");
    }

    /// Where the game's rule places a vein block, the map says there is a vein.
    #[test]
    fn veins_are_where_the_game_puts_them() {
        let field = Underground::new(12345);
        let mut cache = NoiseCache::default();
        let (mut placed, mut outside) = (0, 0);
        for i in 0..40_000 {
            let (x, z) = ((i % 200 - 100) * 37, (i / 200 - 100) * 37);
            for (ore, heights) in [(Ore::Copper, COPPER_HEIGHTS), (Ore::Iron, IRON_HEIGHTS)] {
                for y in heights {
                    if field.vein_density(&mut cache, ore, x, y, z) > 0.0 {
                        placed += 1;
                        outside += (field.at(x, z) != Some(ore)) as i32;
                    }
                }
            }
            assert!(field.vein_density(&mut cache, Ore::Copper, x, -30, z) <= 0.0);
            assert!(field.vein_density(&mut cache, Ore::Iron, x, 25, z) <= 0.0);
        }
        println!("placed {placed} outside {outside}");
        assert!(placed > 100);
        // the game's noise is interpolated between cells, which moves the edge by a few blocks
        assert!((outside as f64) < placed as f64 * 0.1, "{outside} of {placed}");
    }

    #[test]
    fn the_end_has_no_noise() {
        assert!(NoiseField::new(Dimension::End, 1).is_none());
        assert!(parameters_of(Dimension::End).is_empty());
    }
}
