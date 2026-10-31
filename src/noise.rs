//! The climate noises that decide biomes, for looking at them and for searching.

use steel_worldgen::{
    biomes::{NetherClimateSampler, OverworldClimateSampler},
    density_functions::{nether::NetherColumnCache, overworld::OverworldColumnCache},
};

use crate::{dimension::Dimension, seed::generator_seed, seedfind::spec::NoiseParameter, tile_renderer::TileKey};

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
            scale: 3,
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
    fn the_end_has_no_noise() {
        assert!(NoiseField::new(Dimension::End, 1).is_none());
        assert!(parameters_of(Dimension::End).is_empty());
    }
}
