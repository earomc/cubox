//! Checks cards against one seed. What it needs of the world is only made when a card gets that far.

use std::{collections::HashMap, sync::OnceLock};

use steel_worldgen::biomes::BiomeSourceKind;

use crate::{
    dimension::Dimension,
    noise::{NoiseCache, NoiseField},
    seedfind::spec::{Anchor, BiomeTest, Card},
    structures::{Finder, Found, STRUCTURE_KINDS, Spread, allowed_by_biomes, candidates, spread},
};

/// Samples of a card's area, whatever its size.
const AREA_SAMPLES_PER_AXIS: i32 = 64;
const MIN_STEP: i32 = 16;
const MAX_STEP: i32 = 256;
/// A connected patch is followed this far, in samples.
const MAX_PATCH_SAMPLES: usize = 1 << 16;
/// How much of a wanted patch the coarse look has to see before the fine one is done.
const COARSE_SHARE: f64 = 0.5;

/// Room for the grid cells and chunk middles of candidates, see `candidates`.
pub type Cells = Vec<([i32; 2], [i32; 2])>;

/// Something a card found.
#[derive(Clone, PartialEq, Debug)]
pub struct Hit {
    /// the kind of a structure, nothing for a biome or noise value
    pub kind: Option<usize>,
    pub dimension: Dimension,
    pub x: i32,
    pub z: i32,
}

/// What passing a card came to.
#[derive(Default)]
struct Passed {
    /// how well it did, more is better
    score: Option<f64>,
    hits: Vec<Hit>,
}

/// What a seed needs for checking cards, made when first needed.
pub struct SeedWorld {
    pub seed: i64,
    sources: [OnceLock<BiomeSourceKind>; 3],
    finders: [OnceLock<Option<Finder>>; 3],
    spawn: OnceLock<[i32; 2]>,
    climate: OnceLock<Option<NoiseField>>,
}

impl SeedWorld {
    pub fn new(seed: i64) -> Self {
        Self {
            seed,
            sources: Default::default(),
            finders: Default::default(),
            spawn: OnceLock::new(),
            climate: OnceLock::new(),
        }
    }

    fn source(&self, dimension: Dimension) -> &BiomeSourceKind {
        self.sources[dimension as usize].get_or_init(|| dimension.biome_source(self.seed))
    }

    fn finder(&self, dimension: Dimension) -> Option<&Finder> {
        self.finders[dimension as usize]
            .get_or_init(|| match Finder::new(self.seed, dimension) {
                Ok(finder) => Some(finder),
                Err(err) => {
                    log::error!("setting up structures: {err:#}");
                    None
                }
            })
            .as_ref()
    }

    pub fn spawn(&self) -> [i32; 2] {
        *self.spawn.get_or_init(|| {
            let spawn = self.source(Dimension::Overworld).initial_spawn_search_origin();
            [spawn.x(), spawn.z()]
        })
    }

    fn center(&self, anchor: Anchor, here: [i32; 2]) -> [i32; 2] {
        match anchor {
            Anchor::Spawn => self.spawn(),
            Anchor::At(x, z) => [x, z],
            Anchor::Here => here,
        }
    }
}

/// The dimension a biome is in.
fn biome_dimension(biome: &str) -> Dimension {
    static NAMES: OnceLock<[Vec<String>; 3]> = OnceLock::new();
    let names = NAMES.get_or_init(|| {
        Dimension::ALL.map(|dimension| {
            dimension
                .biome_source(0)
                .possible_biome_refs()
                .iter()
                .map(|biome| biome.key.path.to_string())
                .collect()
        })
    });
    Dimension::ALL
        .into_iter()
        .find(|dimension| names[*dimension as usize].iter().any(|name| name == biome))
        .unwrap_or(Dimension::Overworld)
}

/// The largest number of `points` that one circle of `radius` can hold, and its middle.
pub fn best_circle(points: &[[i32; 2]], radius: i32) -> (usize, [i32; 2]) {
    let r = radius as f64;
    let count_at = |x: f64, z: f64| {
        points
            .iter()
            .filter(|p| {
                let (dx, dz) = (p[0] as f64 - x, p[1] as f64 - z);
                // a little room, the middle of two points is rounded
                dx * dx + dz * dz <= (r + 1e-6) * (r + 1e-6)
            })
            .count()
    };
    let mut best = (0, [0, 0]);
    let mut consider = |x: f64, z: f64| {
        let count = count_at(x, z);
        if count > best.0 {
            best = (count, [x.round() as i32, z.round() as i32]);
        }
    };
    // if a circle holds some points it can be moved until it touches two, or sits on one
    for (i, a) in points.iter().enumerate() {
        consider(a[0] as f64, a[1] as f64);
        for b in &points[i + 1..] {
            let (dx, dz) = ((b[0] - a[0]) as f64, (b[1] - a[1]) as f64);
            let d2 = dx * dx + dz * dz;
            if d2 == 0.0 || d2 > 4.0 * r * r {
                continue;
            }
            let (mx, mz) = ((a[0] + b[0]) as f64 / 2.0, (a[1] + b[1]) as f64 / 2.0);
            let h = (r * r - d2 / 4.0).max(0.0).sqrt() / d2.sqrt();
            consider(mx - dz * h, mz + dx * h);
            consider(mx + dz * h, mz - dx * h);
        }
    }
    best
}

/// Drops candidates without enough others close enough to share a circle of `radius` with.
fn keep_clusterable(cells: &mut Cells, radius: i32, count: u32) {
    cells.sort_unstable_by_key(|(_, middle)| middle[0]);
    let reach = 2 * radius as i64;
    let middles: Vec<[i32; 2]> = cells.iter().map(|(_, middle)| *middle).collect();
    let mut first = 0;
    let mut index = 0;
    cells.retain(|_| {
        let at = middles[index];
        while (at[0] - middles[first][0]) as i64 > reach {
            first += 1;
        }
        let close = middles[first..]
            .iter()
            .take_while(|other| (other[0] - at[0]) as i64 <= reach)
            .filter(|other| {
                let (dx, dz) = ((other[0] - at[0]) as i64, (other[1] - at[1]) as i64);
                dx * dx + dz * dz <= reach * reach
            })
            .count();
        index += 1;
        close >= count as usize
    });
}

/// Whether `count` of the candidates can be in a circle of `radius`. Drops those that can't.
fn could_cluster(cells: &mut Cells, radius: i32, count: u32) -> bool {
    keep_clusterable(cells, radius, count);
    let middles: Vec<[i32; 2]> = cells.iter().map(|(_, middle)| *middle).collect();
    best_circle(&middles, radius).0 >= count as usize
}

/// Spots on a grid in a circle, the closest to the middle first.
fn spots(center: [i32; 2], radius: i32, step: i32) -> Vec<[i32; 2]> {
    let n = radius / step;
    let mut spots = Vec::with_capacity(((2 * n + 1) * (2 * n + 1)) as usize);
    for dz in -n..=n {
        for dx in -n..=n {
            if (dx * dx + dz * dz) as i64 * (step as i64 * step as i64) <= radius as i64 * radius as i64 {
                spots.push([dx, dz]);
            }
        }
    }
    spots.sort_by_key(|[dx, dz]| dx * dx + dz * dz);
    spots
        .into_iter()
        .map(|[dx, dz]| [center[0] + dx * step, center[1] + dz * step])
        .collect()
}

fn area_step(radius: i32) -> i32 {
    (radius / AREA_SAMPLES_PER_AXIS).clamp(MIN_STEP, MAX_STEP)
}

/// The biome's connected patch through `start` on a grid of `step`: its square blocks and middle.
fn patch(
    sample: &mut dyn FnMut(i32, i32) -> bool,
    start: [i32; 2],
    step: i32,
    seen: &mut HashMap<[i32; 2], bool>,
) -> (i64, [i32; 2]) {
    let mut stack = vec![start];
    seen.insert(start, true);
    let (mut cells, mut sum) = (0i64, [0i64; 2]);
    while let Some(at) = stack.pop() {
        cells += 1;
        sum[0] += at[0] as i64;
        sum[1] += at[1] as i64;
        if cells as usize >= MAX_PATCH_SAMPLES {
            break;
        }
        for (dx, dz) in [(step, 0), (-step, 0), (0, step), (0, -step)] {
            let next = [at[0] + dx, at[1] + dz];
            if seen.contains_key(&next) {
                continue;
            }
            let inside = sample(next[0], next[1]);
            seen.insert(next, inside);
            if inside {
                stack.push(next);
            }
        }
    }
    (
        cells * step as i64 * step as i64,
        [(sum[0] / cells) as i32, (sum[1] / cells) as i32],
    )
}

fn check_biome(world: &SeedWorld, biome: &str, test: BiomeTest, radius: i32, center: [i32; 2]) -> Option<Passed> {
    let dimension = biome_dimension(biome);
    let source = world.source(dimension);
    let mut sampler = source.chunk_sampler();
    let qy = dimension.default_height() >> 2;
    let mut is_biome = |x: i32, z: i32| &*sampler.sample(x >> 2, qy, z >> 2).key.path == biome;
    let hit = |x, z| Hit {
        kind: None,
        dimension,
        x,
        z,
    };
    match test {
        BiomeTest::Present => {
            let at = spots(center, radius, area_step(radius))
                .into_iter()
                .find(|at| is_biome(at[0], at[1]))?;
            Some(Passed {
                score: None,
                hits: vec![hit(at[0], at[1])],
            })
        }
        BiomeTest::Absent => spots(center, radius, area_step(radius))
            .into_iter()
            .all(|at| !is_biome(at[0], at[1]))
            .then(Passed::default),
        BiomeTest::Coverage(percent) => {
            let spots = spots(center, radius, area_step(radius));
            let wanted = (percent as f64 / 100.0 * spots.len() as f64).ceil() as usize;
            let (mut inside, mut first) = (0, None);
            for (i, at) in spots.iter().enumerate() {
                if is_biome(at[0], at[1]) {
                    inside += 1;
                    first.get_or_insert(*at);
                }
                // it can't get there anymore
                if inside + spots.len() - i - 1 < wanted {
                    return None;
                }
            }
            (inside >= wanted.max(1)).then(|| Passed {
                score: Some(inside as f64 * 100.0 / spots.len() as f64),
                hits: first.map(|at| hit(at[0], at[1])).into_iter().collect(),
            })
        }
        BiomeTest::Area(area) => {
            // coarse enough to be quick, fine enough not to miss a patch of this size. Exact to
            // the fine grid, which is 4 times finer.
            let step = (((area as f64).sqrt() / 8.0) as i32)
                .clamp(MIN_STEP, MAX_STEP)
                .min((radius / 4).max(MIN_STEP));
            let mut seen = HashMap::new();
            let mut best: Option<(i64, [i32; 2])> = None;
            for at in spots(center, radius, step) {
                let inside = match seen.get(&at) {
                    Some(_) => continue,
                    None => is_biome(at[0], at[1]),
                };
                seen.insert(at, inside);
                if !inside {
                    continue;
                }
                let (coarse, middle) = patch(&mut is_biome, at, step, &mut seen);
                if coarse as f64 >= area as f64 * COARSE_SHARE {
                    let (fine, middle_fine) = patch(&mut is_biome, at, (step / 4).max(4), &mut HashMap::new());
                    if fine >= area && best.is_none_or(|(size, _)| fine > size) {
                        best = Some((fine, if fine > 0 { middle_fine } else { middle }));
                    }
                }
            }
            let (size, middle) = best?;
            Some(Passed {
                score: Some(size as f64),
                hits: vec![hit(middle[0], middle[1])],
            })
        }
    }
}

impl SeedWorld {
    fn check(&self, card: &Card, here: [i32; 2], cells: &mut Cells) -> Option<Passed> {
        match card {
            Card::Structure {
                kind,
                count,
                radius,
                anchor,
            } => {
                let center = self.center(*anchor, here);
                if spread(*kind) != Some(Spread::Rings) {
                    cells.clear();
                    candidates(self.seed, *kind, center, *radius, cells);
                    if cells.len() < *count as usize {
                        return None;
                    }
                    let source = self.source(STRUCTURE_KINDS[*kind].dimension);
                    allowed_by_biomes(source, *kind, cells);
                    if cells.len() < *count as usize {
                        return None;
                    }
                }
                let found = self
                    .finder(STRUCTURE_KINDS[*kind].dimension)?
                    .around(*kind, center, *radius);
                if found.len() < *count as usize {
                    return None;
                }
                let nearest = found.iter().map(|f| distance(f, center)).fold(f64::INFINITY, f64::min);
                Some(Passed {
                    score: Some(-nearest),
                    hits: found.iter().map(found_hit).collect(),
                })
            }
            Card::Cluster {
                kind,
                count,
                radius,
                within,
                anchor,
            } => {
                let center = self.center(*anchor, here);
                if spread(*kind) != Some(Spread::Rings) {
                    cells.clear();
                    candidates(self.seed, *kind, center, *within, cells);
                    let margin = crate::structures::marker_margin(*kind);
                    if !could_cluster(cells, radius + margin, *count) {
                        return None;
                    }
                    allowed_by_biomes(self.source(STRUCTURE_KINDS[*kind].dimension), *kind, cells);
                    if !could_cluster(cells, radius + margin, *count) {
                        return None;
                    }
                }
                let found = self
                    .finder(STRUCTURE_KINDS[*kind].dimension)?
                    .around(*kind, center, *within);
                let points: Vec<[i32; 2]> = found.iter().map(|f| [f.x, f.z]).collect();
                let (best, middle) = best_circle(&points, *radius);
                if best < *count as usize {
                    return None;
                }
                let held = |f: &&Found| {
                    let (dx, dz) = ((f.x - middle[0]) as i64, (f.z - middle[1]) as i64);
                    dx * dx + dz * dz <= (*radius as i64 + 1) * (*radius as i64 + 1)
                };
                Some(Passed {
                    score: Some(best as f64),
                    hits: found.iter().filter(held).map(found_hit).collect(),
                })
            }
            Card::Biome {
                biome,
                test,
                radius,
                anchor,
            } => check_biome(self, biome, *test, *radius, self.center(*anchor, here)),
            Card::Noise {
                parameter,
                min,
                max,
                coverage,
                radius,
                anchor,
            } => {
                let center = self.center(*anchor, here);
                let climate = self
                    .climate
                    .get_or_init(|| NoiseField::new(Dimension::Overworld, self.seed))
                    .as_ref()?;
                let mut cache = NoiseCache::default();
                let qy = Dimension::Overworld.default_height() >> 2;
                // the climate changes slowly, and sampling it all takes longer than a biome
                let spots = spots(center, *radius, (area_step(*radius) * 2).min(MAX_STEP));
                let wanted = ((*coverage as f64 / 100.0 * spots.len() as f64).ceil() as usize).max(1);
                let (mut inside, mut first) = (0, None);
                for (i, at) in spots.iter().enumerate() {
                    let value = climate.value(&mut cache, *parameter, [at[0] >> 2, qy, at[1] >> 2]);
                    if (*min..=*max).contains(&value) {
                        inside += 1;
                        first.get_or_insert(*at);
                        if *coverage == 0.0 {
                            break;
                        }
                    }
                    if inside + spots.len() - i - 1 < wanted {
                        return None;
                    }
                }
                (inside >= wanted).then(|| Passed {
                    score: (*coverage > 0.0).then(|| inside as f64 * 100.0 / spots.len() as f64),
                    hits: first
                        .map(|at| Hit {
                            kind: None,
                            dimension: Dimension::Overworld,
                            x: at[0],
                            z: at[1],
                        })
                        .into_iter()
                        .collect(),
                })
            }
            Card::Not(card) => match self.check(card, here, cells) {
                Some(_) => None,
                None => Some(Passed::default()),
            },
            Card::Any(cards) => cards.iter().find_map(|card| self.check(card, here, cells)),
        }
    }
}

fn distance(found: &Found, center: [i32; 2]) -> f64 {
    let (dx, dz) = ((found.x - center[0]) as f64, (found.z - center[1]) as f64);
    (dx * dx + dz * dz).sqrt()
}

fn found_hit(found: &Found) -> Hit {
    Hit {
        kind: Some(found.kind),
        dimension: STRUCTURE_KINDS[found.kind].dimension,
        x: found.x,
        z: found.z,
    }
}

/// A seed or location that passed all cards.
#[derive(Clone, PartialEq, Debug)]
pub struct Match {
    pub seed: i64,
    /// the score of the first card that has one, more is better
    pub score: Option<f64>,
    pub hits: Vec<Hit>,
}

/// A search with its cards in the order they are checked.
pub struct Plan {
    /// card index in the search and card, the cheapest first
    cards: Vec<(usize, Card)>,
    /// cards that can turn a seed down from its lower 48 bits alone
    positional: Vec<usize>,
}

/// About how long a card takes, to check the cheap ones first.
fn cost(card: &Card) -> u32 {
    let anchored = |anchor: &Anchor| if *anchor == Anchor::Spawn { 3 } else { 0 };
    match card {
        Card::Structure { kind, anchor, .. } | Card::Cluster { kind, anchor, .. } => {
            if spread(*kind) == Some(Spread::Rings) {
                100
            } else {
                1 + anchored(anchor)
            }
        }
        Card::Biome { anchor, .. } | Card::Noise { anchor, .. } => 2 + anchored(anchor),
        Card::Not(card) => cost(card),
        Card::Any(cards) => cards.iter().map(cost).sum(),
    }
}

impl Plan {
    pub fn new(cards: &[Card]) -> Self {
        let mut ordered: Vec<(usize, Card)> = cards.iter().cloned().enumerate().collect();
        ordered.sort_by_key(|(_, card)| cost(card));
        let positional = ordered
            .iter()
            .enumerate()
            .filter(|(_, (_, card))| match card {
                Card::Structure { kind, anchor, .. } | Card::Cluster { kind, anchor, .. } => {
                    matches!(anchor, Anchor::At(..)) && spread(*kind) != Some(Spread::Rings)
                }
                _ => false,
            })
            .map(|(i, _)| i)
            .collect();
        Self {
            cards: ordered,
            positional,
        }
    }

    /// Whether going through upper bits of seeds that passed `lower_bits_pass` is worth it.
    pub fn has_positional(&self) -> bool {
        !self.positional.is_empty()
    }

    /// False if no seed with the same lower 48 bits can pass. Needs no world, takes nanoseconds.
    pub fn lower_bits_pass(&self, seed: i64, cells: &mut Cells) -> bool {
        self.positional.iter().all(|&i| match &self.cards[i].1 {
            Card::Structure {
                kind,
                count,
                radius,
                anchor: Anchor::At(x, z),
            } => {
                cells.clear();
                candidates(seed, *kind, [*x, *z], *radius, cells);
                cells.len() >= *count as usize
            }
            Card::Cluster {
                kind,
                count,
                radius,
                within,
                anchor: Anchor::At(x, z),
            } => {
                cells.clear();
                candidates(seed, *kind, [*x, *z], *within, cells);
                let reach = radius + crate::structures::marker_margin(*kind);
                could_cluster(cells, reach, *count)
            }
            _ => true,
        })
    }

    /// Checks all cards, `here` is what `Anchor::Here` means.
    pub fn check(&self, world: &SeedWorld, here: [i32; 2], cells: &mut Cells) -> Option<Match> {
        let mut passed = Vec::with_capacity(self.cards.len());
        for (index, card) in &self.cards {
            passed.push((*index, world.check(card, here, cells)?));
        }
        passed.sort_by_key(|(index, _)| *index);
        Some(Match {
            seed: world.seed,
            score: passed.iter().find_map(|(_, passed)| passed.score),
            hits: passed.into_iter().flat_map(|(_, passed)| passed.hits).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seedfind::spec::{NoiseParameter, parse};

    fn with_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn best_circle_finds_what_fits() {
        assert_eq!(best_circle(&[], 10).0, 0);
        assert_eq!(best_circle(&[[0, 0]], 10).0, 1);
        assert_eq!(best_circle(&[[0, 0], [20, 0]], 10).0, 2);
        assert_eq!(best_circle(&[[0, 0], [21, 0]], 10).0, 1);
        // the corners of a square of 20 fit in a radius of 14.15, not 14
        let square = [[0, 0], [20, 0], [0, 20], [20, 20]];
        assert_eq!(best_circle(&square, 15).0, 4);
        assert_eq!(best_circle(&square, 14).0, 2);
    }

    #[test]
    fn clusterable_keeps_what_can_share_a_circle() {
        let cells = |points: &[[i32; 2]]| points.iter().map(|p| ([0, 0], *p)).collect::<Vec<_>>();
        let mut near = cells(&[[0, 0], [10, 0], [500, 0], [0, 15], [1000, 1000]]);
        keep_clusterable(&mut near, 10, 3);
        assert_eq!(near.len(), 3);
        let mut lonely = cells(&[[0, 0], [100, 0]]);
        keep_clusterable(&mut lonely, 10, 2);
        assert!(lonely.is_empty());
    }

    /// The order and the shortcut from the lower bits must not change which seeds pass.
    #[test]
    fn plan_finds_the_same_as_checking_everything() {
        with_stack(|| {
            let cases = [
                "structure swamp_hut count>=1 within 1500 of 0,0\nbiome plains present within 800 of 0,0",
                "cluster swamp_hut count>=2 radius 200 within 3000 of 100,-100",
                "structure village count>=2 within 1500 of 0,0\nnot biome mushroom_fields present within 500 of 0,0",
                "any\n  structure igloo count>=1 within 2000 of 0,0\n  biome desert present within 300 of 0,0\nend",
            ];
            let mut matches = 0;
            for text in cases {
                let cards = parse(text).unwrap();
                let plan = Plan::new(&cards);
                let mut cells = Vec::new();
                for seed in 0..60i64 {
                    let seed = seed.wrapping_mul(0x0123_4567_89AB_CDEF);
                    let world = SeedWorld::new(seed);
                    // every card, no shortcut: `around` asks the finder about all candidates
                    let plain = cards.iter().all(|card| match card {
                        Card::Structure {
                            kind,
                            count,
                            radius,
                            anchor: Anchor::At(x, z),
                        } => world
                            .finder(STRUCTURE_KINDS[*kind].dimension)
                            .is_some_and(|f| f.around(*kind, [*x, *z], *radius).len() >= *count as usize),
                        card => world.check(card, [0, 0], &mut cells).is_some(),
                    });
                    let planned = plan.lower_bits_pass(seed, &mut cells)
                        && plan.check(&SeedWorld::new(seed), [0, 0], &mut cells).is_some();
                    assert_eq!(planned, plain, "{text} seed {seed}");
                    matches += planned as i32;
                }
            }
            assert!(matches > 3, "{matches}");
        });
    }

    #[test]
    fn lower_bits_never_turn_down_a_real_cluster() {
        with_stack(|| {
            let wide = Plan::new(&parse("cluster swamp_hut count>=2 radius 128 within 2500 of 0,0").unwrap());
            let (mut cells, mut passed) = (Vec::new(), 0);
            for seed in 0..800i64 {
                let seed = seed.wrapping_mul(0x09E3_779B_97F4_A7C1);
                if wide.check(&SeedWorld::new(seed), [0, 0], &mut cells).is_some() {
                    assert!(wide.lower_bits_pass(seed, &mut cells), "seed {seed}");
                    passed += 1;
                }
            }
            assert!(passed > 0);
            let narrow = Plan::new(&parse("cluster swamp_hut count>=3 radius 128 within 1500 of 0,0").unwrap());
            assert!((0..800i64).any(|seed| !narrow.lower_bits_pass(seed, &mut cells)));
        });
    }

    #[test]
    fn noise_cards_use_the_values_of_the_field() {
        with_stack(|| {
            // mushroom fields are where continentalness is lowest
            let source = Dimension::Overworld.biome_source(1);
            let mut sampler = source.chunk_sampler();
            let field = NoiseField::new(Dimension::Overworld, 1).unwrap();
            let mut cache = NoiseCache::default();
            let mut mushrooms = 0;
            for i in 0..40_000 {
                let (x, z) = ((i % 200 - 100) * 128, (i / 200 - 100) * 128);
                if &*sampler.sample(x, 64, z).key.path == "mushroom_fields" {
                    mushrooms += 1;
                    let value = field.value(&mut cache, NoiseParameter::Continentalness, [x, 64, z]);
                    assert!(value <= -1.0, "{value}");
                }
            }
            assert!(mushrooms > 0);
            let plan = Plan::new(&parse("noise continentalness -5..5 coverage>=100% within 100 of 0,0").unwrap());
            assert!(plan.check(&SeedWorld::new(1), [0, 0], &mut Vec::new()).is_some());
            let plan = Plan::new(&parse("noise continentalness 5..6 within 100 of 0,0").unwrap());
            assert!(plan.check(&SeedWorld::new(1), [0, 0], &mut Vec::new()).is_none());
        });
    }
}
