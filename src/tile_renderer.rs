use std::sync::LazyLock;

use steel_worldgen::biomes::{BiomeSourceKind, ChunkBiomeSampler};

use crate::biome_colors::biome_color;

/// The world border is at ±29,999,984 blocks.
pub const WORLD_BORDER_QUARTS: i32 = 29_999_984 / 4;

/// A color in Oklab. Colors are averaged in Oklab because it is perceptual: mixed areas keep their
/// perceived lightness and color. Averaging linear light makes mixes pale, averaging sRGB bytes dark.
pub type Lab = [f32; 3];

/// What a tile shows.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Layer {
    Biome,
    /// the hillshade of the terrain, drawn over the biomes
    Relief,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TileKey {
    pub layer: Layer,
    pub scale: u8,           // scale of sampling
    pub sample_height: i32,  // the y coordinate in blocks
    pub tile_size_px: usize, // size of the tile in pixels
    pub samples: u8,         // samples per axis per pixel, averaged together (supersampling)

    // image coordinates of the tile
    pub x: i32,
    pub z: i32,
}

impl TileKey {
    pub fn stride(&self) -> i32 {
        1 << self.scale
    }

    /// Samples per axis that are actually taken. A pixel never covers less than one quart,
    /// so more samples than quarts are pointless.
    pub fn effective_samples(&self) -> i32 {
        (self.samples.max(1) as i32).min(self.stride())
    }

    /// World origin of the tile in quarts.
    pub fn origin(&self) -> (i32, i32) {
        let size = self.tile_size_px as i32 * self.stride();
        (self.x * size, self.z * size)
    }
}

/// Offsets of `n` sub samples per axis inside a pixel of `stride` quarts, spread evenly around
/// its center. For one sample this is stride / 2, the center of the pixel.
fn offsets(stride: i32, n: i32) -> Vec<i32> {
    (0..n).map(|k| stride * (2 * k + 1) / (2 * n)).collect()
}

/// sRGB byte -> linear light
static SRGB_TO_LINEAR: LazyLock<[f32; 256]> = LazyLock::new(|| {
    std::array::from_fn(|i| {
        let c = i as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    })
});

fn linear_to_srgb(c: f32) -> u8 {
    let c = if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

/// sRGB bytes -> Oklab, see https://bottosson.github.io/posts/oklab/
#[allow(clippy::excessive_precision)] // the published constants, as is
fn srgb_to_lab(c: &[u8]) -> Lab {
    let [r, g, b] = [0, 1, 2].map(|ch| SRGB_TO_LINEAR[c[ch] as usize]);
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

/// Oklab -> sRGB bytes
#[allow(clippy::excessive_precision)] // the published constants, as is
fn lab_to_srgb(c: Lab, pixel: &mut [u8]) {
    let l = (c[0] + 0.3963377774 * c[1] + 0.2158037573 * c[2]).powi(3);
    let m = (c[0] - 0.1055613458 * c[1] - 0.0638541728 * c[2]).powi(3);
    let s = (c[0] - 0.0894841775 * c[1] - 1.2914855480 * c[2]).powi(3);
    pixel[0] = linear_to_srgb(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s);
    pixel[1] = linear_to_srgb(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s);
    pixel[2] = linear_to_srgb(-0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s);
}

/// A pixel's sampled average color, before shading, in Oklab packed into 16 bits per channel.
/// Coarser tiles are built from these: averaging shaded colors would also average their clipping
/// to what sRGB can show, and get a slightly different color than rendering the tile directly.
pub type Estimate = [u16; 3];

fn pack(c: Lab) -> Estimate {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
    // L is 0..1, a and b of sRGB colors stay well within -0.5..0.5
    [q(c[0]), q(c[1] + 0.5), q(c[2] + 0.5)]
}

fn unpack(e: Estimate) -> Lab {
    let v = |q: u16| q as f32 / 65535.0;
    [v(e[0]), v(e[1]) - 0.5, v(e[2]) - 0.5]
}

/// A generated tile: its colors, and the estimates they were shaded from.
pub struct Tile {
    pub rgb: Vec<u8>,
    pub estimates: Vec<Estimate>,
}

/// Biome color at a quart position, in Oklab.
fn sample_lab(sampler: &mut ChunkBiomeSampler, x: i32, y: i32, z: i32) -> Lab {
    srgb_to_lab(&biome_color(&sampler.sample(x, y, z).key.path))
}

/// Average color of the samples at `offsets` x `offsets` from (x, z).
fn average(sampler: &mut ChunkBiomeSampler, x: i32, y: i32, z: i32, offsets: &[i32]) -> Lab {
    let mut sum = [0f32; 3];
    for &oz in offsets {
        for &ox in offsets {
            let c = sample_lab(sampler, x + ox, y, z + oz);
            (0..3).for_each(|ch| sum[ch] += c[ch]);
        }
    }
    sum.map(|s| s / (offsets.len() * offsets.len()) as f32)
}

/// Average color of the whole world at `sample_height`, from 64k samples spread evenly over it.
/// Zoomed far out, every pixel covers so many biomes that this is what it really looks like.
/// Takes about 0.2s. Deliberately single threaded: the gui computes it lazily inside a rayon job,
/// where waiting on nested parallel work could run another job that needs this very value.
pub fn world_mean(source: &BiomeSourceKind, sample_height: i32) -> Lab {
    const GRID: i32 = 256;
    let step = 2 * WORLD_BORDER_QUARTS / GRID;
    let mut sampler = source.chunk_sampler();
    let mut sum = [0f64; 3];
    for row in 0..GRID {
        let z = -WORLD_BORDER_QUARTS + row * step + step / 2;
        for col in 0..GRID {
            let x = -WORLD_BORDER_QUARTS + col * step + step / 2;
            let c = sample_lab(&mut sampler, x, sample_height >> 2, z);
            (0..3).for_each(|ch| sum[ch] += c[ch] as f64);
        }
    }
    sum.map(|s| (s / (GRID * GRID) as f64) as f32)
}

/// How the sampled pixel averages of one level are turned into colors.
///
/// A pixel's few samples only estimate the average color of its area. Once pixels cover many
/// biomes that estimate is mostly noise: shown as is, the map has far more contrast than the real
/// area averages, and looks like confetti. So estimates are pulled toward the world mean just enough
/// that the pixels vary exactly as much as the true area averages do (variance matching). The map
/// then has the contrast and saturation the real view has: unchanged where pixels are smaller than
/// biomes, a uniform haze where they cover thousands of them, and the real amount in between.
#[derive(Clone, Copy, Debug)]
pub struct Shading {
    pub mean: Lab,
    /// variance of the true pixel averages
    signal: f32,
    /// variance of the error of a pixel's estimate
    noise: f32,
}

/// Pixels sampled to calibrate a level, and the samples per axis of their reference averages.
const CALIBRATION_PIXELS: u64 = 512;
const REFERENCE_SAMPLES: i32 = 8;
/// Below this standard deviation (Oklab, about half a step of a color byte) a level is drawn
/// as the plain world mean without sampling.
const FLAT_DEVIATION: f32 = 0.002;

impl Shading {
    /// Shading that shows the estimates as they are.
    pub fn exact(mean: Lab) -> Self {
        Self {
            mean,
            signal: 1.0,
            noise: 0.0,
        }
    }

    /// How much of an estimate's deviation from the mean is kept, for estimates whose error
    /// variance is `noise`.
    fn weight(&self, noise: f32) -> f32 {
        let total = self.signal + noise;
        if total <= 0.0 {
            1.0
        } else {
            (self.signal / total).sqrt()
        }
    }

    /// Whether the true pixel averages of this level are practically all the world mean.
    pub fn is_flat(&self) -> bool {
        self.signal.sqrt() < FLAT_DEVIATION
    }
}

/// Measures how much the pixel averages of `key`'s level really vary (signal) and how far its
/// `samples` per pixel are off (noise), for this world: it samples pixels all over the world with
/// both the tile's samples and a much denser reference, which stands in for the true average.
/// Takes about 0.1s at 4 samples per pixel, single threaded like `world_mean`.
pub fn calibrate(source: &BiomeSourceKind, key: TileKey, mean: Lab) -> Shading {
    let stride = key.stride();
    let n = key.effective_samples();
    let reference_n = REFERENCE_SAMPLES.max(2 * n).min(stride);
    if reference_n <= n {
        // pixels this small are sampled completely, the estimates are exact
        return Shading::exact(mean);
    }
    let (samples, reference) = (offsets(stride, n), offsets(stride, reference_n));
    let qy = key.sample_height >> 2;
    let mut sampler = source.chunk_sampler();

    let mut error = 0f32;
    let mut truths = Vec::with_capacity(CALIBRATION_PIXELS as usize);
    let range = (2 * WORLD_BORDER_QUARTS - stride) as u64;
    for i in 0..CALIBRATION_PIXELS {
        // deterministic positions, so a level always gets the same shading
        let h = splitmix(i ^ ((key.scale as u64) << 32));
        let x = -WORLD_BORDER_QUARTS + (h % range) as i32;
        let z = -WORLD_BORDER_QUARTS + ((h >> 32) % range) as i32;
        let estimate = average(&mut sampler, x, qy, z, &samples);
        let truth = average(&mut sampler, x, qy, z, &reference);
        error += (0..3).map(|ch| (estimate[ch] - truth[ch]).powi(2)).sum::<f32>();
        truths.push(truth);
    }
    let noise = error / CALIBRATION_PIXELS as f32;
    let count = truths.len() as f32;
    let center: Lab = std::array::from_fn(|ch| truths.iter().map(|t| t[ch]).sum::<f32>() / count);
    let spread = truths
        .iter()
        .map(|t| (0..3).map(|ch| (t[ch] - center[ch]).powi(2)).sum::<f32>())
        .sum::<f32>()
        / (count - 1.0);
    // the reference has an error too, about noise * samples / reference samples if samples are independent
    let reference_noise = noise * (n * n) as f32 / (reference_n * reference_n) as f32;
    Shading {
        mean,
        signal: (spread - reference_noise).max(0.0),
        noise,
    }
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^ (x >> 31)
}

/// Shades estimates with `weight` (see `Shading::weight`) into sRGB colors.
fn shade(estimates: Vec<Estimate>, shading: &Shading, weight: f32) -> Tile {
    let mut rgb = vec![0u8; estimates.len() * 3];
    for (pixel, &e) in rgb.as_chunks_mut::<3>().0.iter_mut().zip(&estimates) {
        let avg = unpack(e);
        lab_to_srgb(
            std::array::from_fn(|ch| shading.mean[ch] + weight * (avg[ch] - shading.mean[ch])),
            pixel,
        );
    }
    Tile { rgb, estimates }
}

/// Renders one tile, TILE * TILE pixels.
/// Each pixel is the average of `samples * samples` evenly spaced biome samples over the area the
/// pixel covers, shaded by `shading` (from `calibrate` for this key's level).
/// `keep_going` is asked before every row, if it says no the tile is abandoned and None returned.
pub fn render_tile(
    source: &BiomeSourceKind,
    key: TileKey,
    shading: &Shading,
    keep_going: &dyn Fn() -> bool,
) -> Option<Tile> {
    let size = key.tile_size_px;
    if shading.is_flat() {
        return Some(shade(vec![pack(shading.mean); size * size], shading, 0.0));
    }
    let stride = key.stride();
    let (origin_x, origin_z) = key.origin();
    let offsets = offsets(stride, key.effective_samples());
    let qy = key.sample_height >> 2; // converting block height to quart height dividing by 4

    let mut sampler = source.chunk_sampler();
    let mut estimates = Vec::with_capacity(size * size);
    for row in 0..size {
        if !keep_going() {
            return None;
        }
        let pz = origin_z + row as i32 * stride;
        for col in 0..size {
            let px = origin_x + col as i32 * stride;
            estimates.push(pack(average(&mut sampler, px, qy, pz, &offsets)));
        }
    }
    Some(shade(estimates, shading, shading.weight(shading.noise)))
}

/// Builds a tile from the estimates of its 4 children one scale finer by averaging 2x2 pixels.
/// That is exactly what a mipmap level is, and costs no biome sampling at all. The averages have
/// 4x the samples of a directly rendered tile, so less noise, and are shaded accordingly: they end
/// up varying just as much as the true averages, like rendered tiles.
/// `children` are ordered top left, top right, bottom left, bottom right.
pub fn downsample(children: [&[Estimate]; 4], key: TileKey, shading: &Shading) -> Tile {
    let size = key.tile_size_px;
    let half = size / 2;
    let mut estimates = Vec::with_capacity(size * size);
    for z in 0..size {
        for x in 0..size {
            let child = children[(z / half) * 2 + x / half];
            // top left of the 2x2 block in the child
            let (cx, cz) = ((x % half) * 2, (z % half) * 2);
            let mut sum = [0f32; 3];
            for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let c = unpack(child[(cz + dz) * size + cx + dx]);
                (0..3).for_each(|ch| sum[ch] += c[ch]);
            }
            estimates.push(pack(sum.map(|s| s * 0.25)));
        }
    }
    shade(estimates, shading, shading.weight(shading.noise / 4.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Children with n samples per axis sample exactly the grid of the parent with 2n samples,
    /// so downsampling them must give the same pixel estimates.
    /// Not exactly the same though: the sampler's R-tree lookup starts from the last result,
    /// which breaks ties between equally close biomes depending on the sampling order.
    /// That flips a tiny fraction of samples, so allow a few pixels to differ.
    #[test]
    fn downsample_matches_direct_render() {
        let source = BiomeSourceKind::overworld(12345);
        let exact = Shading::exact([0.5, 0.0, -0.02]);
        for (scale, size) in [(3, 32), (6, 32), (9, 16)] {
            let parent = TileKey {
                layer: Layer::Biome,
                scale,
                sample_height: 256,
                tile_size_px: size,
                samples: 4,
                x: -1,
                z: 2,
            };
            let children: Vec<Vec<Estimate>> = [(0, 0), (1, 0), (0, 1), (1, 1)]
                .iter()
                .map(|&(dx, dz)| {
                    let child = TileKey {
                        scale: scale - 1,
                        samples: 2,
                        x: parent.x * 2 + dx,
                        z: parent.z * 2 + dz,
                        ..parent
                    };
                    render_tile(&source, child, &exact, &|| true).unwrap().estimates
                })
                .collect();
            let down = downsample(std::array::from_fn(|i| children[i].as_slice()), parent, &exact).estimates;
            let direct = render_tile(&source, parent, &exact, &|| true).unwrap().estimates;
            let differing = down
                .iter()
                .zip(&direct)
                .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 2))
                .count();
            let pixels = parent.tile_size_px * parent.tile_size_px;
            assert!(
                differing * 100 <= pixels,
                "scale {scale}: {differing} of {pixels} pixels differ"
            );
        }
    }

    /// Close up the samples are the truth, far out the pixel averages barely vary.
    #[test]
    fn calibration_fades_out_with_distance() {
        let source = BiomeSourceKind::overworld(12345);
        let key = |scale| TileKey {
            layer: Layer::Biome,
            scale,
            sample_height: 256,
            tile_size_px: 128,
            samples: 2,
            x: 0,
            z: 0,
        };
        let near = calibrate(&source, key(2), [0.0; 3]);
        let far = calibrate(&source, key(13), [0.0; 3]);
        assert!(near.weight(near.noise) > 0.9, "{near:?}");
        assert!(far.weight(far.noise) < 0.2, "{far:?}");
    }
}
