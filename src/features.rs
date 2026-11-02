//! Where features of the game are, like geodes: unlike structures they are decided chunk by chunk
//! while decorating it. This finds out what the game's random numbers say, not what the terrain
//! then makes of it, so a feature can still fail to appear. See `FeatureRoll`.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};

use rustc_hash::{FxHashMap, FxHashSet};
use steel_registry::{REGISTRY, RegistryExt};
use steel_utils::{
    Identifier,
    random::{Random, legacy_random::LegacyRandom, worldgen_random::WorldgenRandom},
};
use steel_worldgen::biomes::BiomeSourceKind;

use crate::dimension::Dimension;

/// How a feature's placement goes in the game: `rarity_filter`, `in_square`, and for those with a
/// height `height_range`.
pub struct FeatureRoll {
    placed_feature: &'static str,
    pub dimension: Dimension,
    /// one chunk in this many
    chance: u32,
    /// lowest and highest block
    heights: Option<(i32, i32)>,
}

const GEODE: FeatureRoll = FeatureRoll {
    placed_feature: "amethyst_geode",
    dimension: Dimension::Overworld,
    chance: 24,
    heights: Some((-58, 30)),
};
const SCATTERED_GATEWAY: FeatureRoll = FeatureRoll {
    placed_feature: "end_gateway_return",
    dimension: Dimension::End,
    chance: 700,
    heights: None,
};

/// The feature behind a kind.
pub fn feature_of(id: &str) -> Option<&'static FeatureRoll> {
    match id {
        "geode" => Some(&GEODE),
        "end_gateway_scattered" => Some(&SCATTERED_GATEWAY),
        _ => None,
    }
}

type Vertex = (usize, usize, usize);

/// Which number a placed feature has in its step of decorating, for all biomes of a dimension.
/// That goes into its random numbers. As the game's `FeatureSorter`: a topological sort of the
/// order the biomes list their features in.
fn feature_numbers(dimension: Dimension) -> FxHashMap<Identifier, (i32, i32)> {
    crate::terrain::init_registry();
    let biomes = dimension.biome_source(0).possible_biome_refs();
    let mut orders = FxHashMap::default();
    let mut edges = BTreeMap::<Vertex, BTreeSet<Vertex>>::new();
    for biome in &biomes {
        let mut in_biome = Vec::new();
        for (step, features) in biome.features.iter().enumerate() {
            for key in features {
                let id = REGISTRY
                    .placed_features
                    .id_from_key(key)
                    .unwrap_or_else(|| panic!("unknown placed feature {key}"));
                let next = orders.len();
                let order = *orders.entry(id).or_insert(next);
                let vertex = (step, order, id);
                edges.entry(vertex).or_default();
                in_biome.push(vertex);
            }
        }
        for pair in in_biome.windows(2) {
            edges.entry(pair[0]).or_default().insert(pair[1]);
        }
    }
    // depth first, each vertex after all it leads to, reversed at the end
    let mut sorted = Vec::with_capacity(edges.len());
    let mut discovered = BTreeSet::new();
    for vertex in edges.keys().copied().collect::<Vec<_>>() {
        visit(vertex, &edges, &mut discovered, &mut sorted);
    }
    sorted.reverse();

    let mut numbers = FxHashMap::default();
    let mut next_in_step = BTreeMap::<usize, i32>::new();
    for (step, _, id) in sorted {
        let number = next_in_step.entry(step).or_default();
        if let Some(feature) = REGISTRY.placed_features.by_id(id) {
            numbers.insert(feature.key.clone(), (*number, step as i32));
        }
        *number += 1;
    }
    numbers
}

fn visit(
    vertex: Vertex,
    edges: &BTreeMap<Vertex, BTreeSet<Vertex>>,
    discovered: &mut BTreeSet<Vertex>,
    sorted: &mut Vec<Vertex>,
) {
    if !discovered.insert(vertex) {
        return;
    }
    if let Some(next) = edges.get(&vertex) {
        for neighbor in next {
            visit(*neighbor, edges, discovered, sorted);
        }
    }
    sorted.push(vertex);
}

fn number_of(roll: &FeatureRoll) -> (i32, i32) {
    static NUMBERS: LazyLock<[FxHashMap<Identifier, (i32, i32)>; 3]> =
        LazyLock::new(|| Dimension::ALL.map(feature_numbers));
    NUMBERS[roll.dimension as usize][&Identifier::vanilla_static(roll.placed_feature)]
}

fn biomes_of(roll: &FeatureRoll) -> &'static FxHashSet<Identifier> {
    static BIOMES: LazyLock<Vec<(&'static str, FxHashSet<Identifier>)>> = LazyLock::new(|| {
        crate::terrain::init_registry();
        [&GEODE, &SCATTERED_GATEWAY]
            .into_iter()
            .map(|roll| {
                let key = Identifier::vanilla_static(roll.placed_feature);
                let biomes = roll
                    .dimension
                    .biome_source(0)
                    .possible_biome_refs()
                    .iter()
                    .filter(|biome| biome.features.iter().flatten().any(|feature| *feature == key))
                    .map(|biome| biome.key.clone())
                    .collect();
                (roll.placed_feature, biomes)
            })
            .collect()
    });
    &BIOMES
        .iter()
        .find(|(placed, _)| *placed == roll.placed_feature)
        .expect("all rolls are listed")
        .1
}

impl FeatureRoll {
    /// What a chunk's random numbers say: the block the feature is at, if any, with the height if
    /// it has its own, and the random numbers to go on with.
    fn roll(&self, seed: i64, chunk: [i32; 2]) -> Option<([i32; 2], Option<i32>, WorldgenRandom)> {
        let (number, step) = number_of(self);
        let mut random = WorldgenRandom::from_seed(0);
        let decoration_seed = random.set_decoration_seed(seed, chunk[0] * 16, chunk[1] * 16);
        random.set_feature_seed(decoration_seed, number, step);
        if random.next_f32() >= 1.0 / self.chance as f32 {
            return None;
        }
        let x = chunk[0] * 16 + random.next_i32_bounded(16);
        let z = chunk[1] * 16 + random.next_i32_bounded(16);
        let y = self
            .heights
            .map(|(lowest, highest)| random.next_i32_between(lowest, highest));
        Some(([x, z], y, random))
    }

    /// The block a chunk's random numbers say the feature is at, if any, with the height if it has
    /// its own. Takes about 100 ns and depends on all bits of the seed.
    pub fn in_chunk(&self, seed: i64, chunk: [i32; 2]) -> Option<([i32; 2], Option<i32>)> {
        self.roll(seed, chunk).map(|(block, y, _)| (block, y))
    }

    /// The blocks the game looks at for a geode at this chunk: it gives up if more than one of them
    /// isn't solid. They come next in the same random numbers.
    pub fn geode_points(seed: i64, chunk: [i32; 2]) -> Option<([i32; 3], Vec<[i32; 3]>)> {
        let ([x, z], y, mut random) = GEODE.roll(seed, chunk)?;
        let y = y?;
        // how `GeodeFeature` goes on, with its defaults: 3 to 4 points, 4 to 6 blocks away
        let points = random.next_i32_between(3, 4);
        random.next_f64(); // the size of its crack
        random.next_f32(); // whether it has one
        let points = (0..points)
            .map(|_| {
                let point = [
                    x + random.next_i32_between(4, 6),
                    y + random.next_i32_between(4, 6),
                    z + random.next_i32_between(4, 6),
                ];
                random.next_i32_between(1, 2); // its offset
                point
            })
            .collect();
        Some(([x, y, z], points))
    }

    /// Whether the feature can be in the biome at this block. The game shifts where it looks a
    /// little, which only matters at the edge of a biome.
    pub fn biome_allows(&self, source: &BiomeSourceKind, block: [i32; 2], y: Option<i32>) -> bool {
        let qy = y.unwrap_or(64) >> 2;
        let biome = source.chunk_sampler().sample(block[0] >> 2, qy, block[1] >> 2);
        biomes_of(self).contains(&biome.key)
    }
}

/// The 20 gateways around the end's middle, in the order killing the dragon opens them. Those are
/// fixed spots, the seed only shuffles them. As the game's `EndDragonFight` and `Util.shuffle`.
pub fn ring_gateways(seed: i64) -> [[i32; 2]; 20] {
    const SPOTS: [[i32; 2]; 20] = [
        [96, 0],
        [91, 29],
        [77, 56],
        [56, 77],
        [29, 91],
        [-1, 96],
        [-30, 91],
        [-57, 77],
        [-78, 56],
        [-92, 29],
        [-96, -1],
        [-92, -30],
        [-78, -57],
        [-57, -78],
        [-30, -92],
        [0, -96],
        [29, -92],
        [56, -78],
        [77, -57],
        [91, -30],
    ];
    let mut random = LegacyRandom::from_seed(seed as u64);
    let mut order = std::array::from_fn::<usize, 20, _>(|i| 19 - i);
    for i in 0..20 {
        let other = 19 - random.next_i32_bounded(20 - i as i32) as usize;
        order.swap(i, other);
    }
    order.map(|spot| SPOTS[spot])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateways_are_all_spots_in_an_order_of_the_seed() {
        let (one, two) = (ring_gateways(1), ring_gateways(2));
        assert_ne!(one, two);
        for spot in one {
            assert!(two.contains(&spot));
            let distance = ((spot[0] * spot[0] + spot[1] * spot[1]) as f64).sqrt();
            assert!((94.0..=98.0).contains(&distance), "{spot:?}");
        }
    }

    #[test]
    fn features_have_their_place_in_decoration() {
        let (number, step) = number_of(&GEODE);
        assert_eq!(step, 2);
        assert!(number >= 0);
        assert_eq!(number_of(&SCATTERED_GATEWAY).1, 4);
        // only one feature comes in that step of the end's biomes
        assert_eq!(number_of(&SCATTERED_GATEWAY).0, 0);
    }

    #[test]
    fn rolls_come_as_rarely_as_the_game_says() {
        let chunks = 200 * 200;
        let hits = (0..chunks)
            .filter(|i| GEODE.in_chunk(12345, [i % 200, i / 200]).is_some())
            .count() as f64;
        let wanted = chunks as f64 / GEODE.chance as f64;
        assert!((hits - wanted).abs() < wanted * 0.15, "{hits} {wanted}");
    }

    #[test]
    fn rolls_stay_in_their_chunk_and_heights() {
        for i in 0..20_000 {
            let chunk = [i % 200 - 100, i / 200 - 50];
            if let Some(([x, z], y)) = GEODE.in_chunk(7, chunk) {
                assert_eq!([x.div_euclid(16), z.div_euclid(16)], chunk);
                assert!((-58..=30).contains(&y.unwrap()));
            }
        }
    }

    #[test]
    fn biomes_are_those_with_the_feature() {
        assert!(biomes_of(&GEODE).contains(&Identifier::vanilla_static("plains")));
        assert!(!biomes_of(&GEODE).contains(&Identifier::vanilla_static("the_end")));
        assert_eq!(biomes_of(&SCATTERED_GATEWAY).len(), 1);
    }
}
