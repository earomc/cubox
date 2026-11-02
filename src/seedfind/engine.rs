//! Checks conditions against one seed. What it needs of the world is only made when a condition gets that far.

use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use steel_worldgen::biomes::BiomeSourceKind;

use crate::{
    dimension::Dimension,
    noise::{NoiseCache, NoiseField},
    seedfind::spec::{Anchor, BiomeTest, Condition},
    structures::{Finder, Found, MARKER_KINDS, Spread, allowed_by_biomes, candidates, depends_on_lower_bits, spread},
};

/// The obsidian platform everyone arrives on in the end.
const END_SPAWN: [i32; 2] = [100, 0];

/// Room for the grid cells and chunk middles of candidates, see `candidates`.
pub type Cells = Vec<([i32; 2], [i32; 2])>;

/// Something a condition found.
#[derive(Clone, PartialEq, Debug)]
pub struct Hit {
    /// the kind of a structure, nothing for a biome or noise value
    pub kind: Option<usize>,
    pub dimension: Dimension,
    pub x: i32,
    pub z: i32,
}

/// What passing a condition came to.
#[derive(Default)]
struct Passed {
    /// how well it did, more is better
    score: Option<f64>,
    hits: Vec<Hit>,
}

/// What a seed needs for checking conditions, made when first needed.
pub struct SeedWorld {
    pub seed: i64,
    /// how far apart biomes and noises are sampled, in blocks
    step: i32,
    sources: [OnceLock<BiomeSourceKind>; 3],
    finders: [OnceLock<Option<Finder>>; 3],
    spawn: OnceLock<[i32; 2]>,
    climate: OnceLock<Option<NoiseField>>,
    /// checks that take long end early once this is set, their answers don't matter then
    stop: Arc<AtomicBool>,
}

impl SeedWorld {
    pub fn new(seed: i64, step: i32) -> Self {
        Self {
            seed,
            step,
            sources: Default::default(),
            finders: Default::default(),
            spawn: OnceLock::new(),
            climate: OnceLock::new(),
            stop: Default::default(),
        }
    }

    /// Makes long checks end early when `stop` is set.
    pub fn stopped_by(mut self, stop: Arc<AtomicBool>) -> Self {
        self.stop = stop;
        self
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// The biomes of a dimension.
    pub fn source(&self, dimension: Dimension) -> &BiomeSourceKind {
        self.sources[dimension as usize].get_or_init(|| dimension.biome_source(self.seed))
    }

    /// The climate of the overworld.
    pub fn climate(&self) -> Option<&NoiseField> {
        self.climate
            .get_or_init(|| NoiseField::new(Dimension::Overworld, self.seed))
            .as_ref()
    }

    pub fn finder(&self, dimension: Dimension) -> Option<&Finder> {
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

    fn overworld_spawn(&self) -> [i32; 2] {
        *self.spawn.get_or_init(|| {
            let spawn = self.source(Dimension::Overworld).initial_spawn_search_origin();
            [spawn.x(), spawn.z()]
        })
    }

    /// Where a condition about `dimension` is around. `validate_seeds` keeps spawn out of the nether.
    fn center(&self, anchor: Anchor, dimension: Dimension, spot: [i32; 2]) -> [i32; 2] {
        match (anchor, dimension) {
            (Anchor::At(x, z), _) => [x, z],
            (Anchor::Target, _) => spot,
            (Anchor::Spawn, Dimension::End) => END_SPAWN,
            (Anchor::Spawn, _) => self.overworld_spawn(),
        }
    }
}

/// The dimension a biome is in.
pub fn biome_dimension(biome: &str) -> Dimension {
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
    best_circle_with(points, radius, None)
}

/// The same for circles that hold the point `with`, if given.
pub fn best_circle_with(points: &[[i32; 2]], radius: i32, with: Option<[i32; 2]>) -> (usize, [i32; 2]) {
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
        let holds = with.is_none_or(|p| {
            let (dx, dz) = (p[0] as f64 - x, p[1] as f64 - z);
            dx * dx + dz * dz <= (r + 1e-6) * (r + 1e-6)
        });
        let count = count_at(x, z);
        if holds && count > best.0 {
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

/// The columns of grid row `dz` that are in a circle, as offsets from its middle.
pub fn disc_row(radius: i32, step: i32, dz: i32) -> std::ops::RangeInclusive<i32> {
    let (radius, z) = (radius as i64, dz as i64 * step as i64);
    let half = ((radius * radius - z * z).max(0) as f64).sqrt() as i64 / step as i64;
    -(half as i32)..=half as i32
}

/// The points of a grid in a circle, from the middle outwards ring by ring. Needs no memory, however
/// many there are.
fn disc_points(center: [i32; 2], radius: i32, step: i32) -> impl Iterator<Item = [i32; 2]> {
    let limit = radius as i64 * radius as i64;
    (0..=radius / step).flat_map(move |ring| {
        (-ring..=ring).flat_map(move |dz| {
            // the whole first and last row of a ring, and only the ends of the others
            let gap = if dz.abs() == ring { 1 } else { (2 * ring) as usize };
            (-ring..=ring).step_by(gap).filter_map(move |dx| {
                let (x, z) = (dx as i64 * step as i64, dz as i64 * step as i64);
                (x * x + z * z <= limit).then_some([center[0] + dx * step, center[1] + dz * step])
            })
        })
    })
}

/// How many points of a grid are in a circle.
fn disc_size(radius: i32, step: i32) -> usize {
    (-(radius / step)..=radius / step)
        .map(|dz| disc_row(radius, step, dz).count())
        .sum()
}

/// A connected part of a grid.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Patch {
    points: i64,
    sum: [i64; 2],
}

impl Patch {
    /// Its square blocks.
    pub fn area(&self, step: i32) -> i64 {
        self.points * step as i64 * step as i64
    }

    /// The average of its points, which can be outside of it if it is a ring.
    pub fn middle(&self) -> [i32; 2] {
        self.sum.map(|sum| (sum / self.points) as i32)
    }
}

/// Connects grid points given row by row, in order, into patches, and gives each patch as soon as
/// the row after it has shown that it ends. Only remembers one row, whatever the size of the area.
pub struct PatchScan {
    center: [i32; 2],
    step: i32,
    next_label: usize,
    open: HashMap<usize, Patch>,
    /// the runs of the last row: first column, column after the last, and patch
    last_row: Vec<(i32, i32, usize)>,
}

impl PatchScan {
    pub fn new(center: [i32; 2], step: i32) -> Self {
        Self {
            center,
            step,
            next_label: 0,
            open: HashMap::new(),
            last_row: Vec::new(),
        }
    }

    /// Adds a row: its runs of points that are inside, from left to right, as first column and the
    /// column after the last. Returns the patches that ended before it.
    pub fn push_row(&mut self, dz: i32, runs: &[(i32, i32)]) -> Vec<Patch> {
        fn root(merged: &HashMap<usize, usize>, mut label: usize) -> usize {
            while let Some(next) = merged.get(&label) {
                label = *next;
            }
            label
        }
        let z = self.center[1] as i64 + dz as i64 * self.step as i64;
        let (x0, step) = (self.center[0] as i64, self.step as i64);
        let mut merged = HashMap::new();
        let mut row = Vec::with_capacity(runs.len());
        let mut first_above = 0;
        for &(start, end) in runs {
            let label = self.next_label;
            self.next_label += 1;
            let points = (end - start) as i64;
            // the sum of the columns from start to end - 1
            let columns = points * (start as i64 + end as i64 - 1) / 2;
            self.open.insert(
                label,
                Patch {
                    points,
                    sum: [points * x0 + columns * step, points * z],
                },
            );
            while self.last_row.get(first_above).is_some_and(|run| run.1 <= start) {
                first_above += 1;
            }
            let mut above = first_above;
            while let Some(&(above_start, _, other)) = self.last_row.get(above) {
                if above_start >= end {
                    break;
                }
                let (mine, other) = (root(&merged, label), root(&merged, other));
                if mine != other {
                    let gone = self.open.remove(&other).expect("patches of a row stay open");
                    let kept = self.open.get_mut(&mine).expect("patches of a row stay open");
                    kept.points += gone.points;
                    kept.sum = [kept.sum[0] + gone.sum[0], kept.sum[1] + gone.sum[1]];
                    merged.insert(other, mine);
                }
                above += 1;
            }
            row.push((start, end, label));
        }
        for run in &mut row {
            run.2 = root(&merged, run.2);
        }
        let ongoing: std::collections::HashSet<usize> = row.iter().map(|run| run.2).collect();
        let ended: Vec<usize> = self.open.keys().copied().filter(|l| !ongoing.contains(l)).collect();
        self.last_row = row;
        ended.iter().filter_map(|label| self.open.remove(label)).collect()
    }

    /// The patches still going after the last row.
    pub fn finish(&mut self) -> Vec<Patch> {
        self.last_row.clear();
        self.open.drain().map(|(_, patch)| patch).collect()
    }
}

/// The runs of a row of a circle where `is_inside` says so, see `PatchScan::push_row`.
pub fn row_runs(
    center: [i32; 2],
    radius: i32,
    step: i32,
    dz: i32,
    is_inside: &mut dyn FnMut(i32, i32) -> bool,
) -> Vec<(i32, i32)> {
    let z = center[1] + dz * step;
    let mut runs = Vec::new();
    let mut start = None;
    let columns = disc_row(radius, step, dz);
    let end = *columns.end() + 1;
    for dx in columns {
        match (is_inside(center[0] + dx * step, z), start) {
            (true, None) => start = Some(dx),
            (false, Some(from)) => {
                runs.push((from, dx));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        runs.push((from, end));
    }
    runs
}

/// The largest patch of at least `area` square blocks inside a circle, looking at all of it.
fn largest_patch(
    is_inside: &mut dyn FnMut(i32, i32) -> bool,
    stopped: &dyn Fn() -> bool,
    area: i64,
    radius: i32,
    center: [i32; 2],
    step: i32,
) -> Option<Patch> {
    let mut scan = PatchScan::new(center, step);
    let mut best: Option<Patch> = None;
    let mut consider = |patches: Vec<Patch>| {
        for patch in patches {
            if patch.area(step) >= area && best.is_none_or(|b| patch.points > b.points) {
                best = Some(patch);
            }
        }
    };
    for dz in -(radius / step)..=radius / step {
        if stopped() {
            return None;
        }
        let runs = row_runs(center, radius, step, dz, is_inside);
        consider(scan.push_row(dz, &runs));
    }
    consider(scan.finish());
    best
}

fn check_biome(
    world: &SeedWorld,
    biome: &str,
    test: BiomeTest,
    radius: i32,
    anchor: Anchor,
    spot: [i32; 2],
) -> Option<Passed> {
    let dimension = biome_dimension(biome);
    let center = world.center(anchor, dimension, spot);
    let step = world.step;
    let source = world.source(dimension);
    let mut sampler = source.chunk_sampler();
    let qy = dimension.default_height() >> 2;
    let mut is_biome = |x: i32, z: i32| &*sampler.sample(x >> 2, qy, z >> 2).key.path == biome;
    let points = || disc_points(center, radius, step).take_while(|_| !world.stopped());
    let hit = |x, z| Hit {
        kind: None,
        dimension,
        x,
        z,
    };
    match test {
        BiomeTest::Present => {
            let at = points().find(|at| is_biome(at[0], at[1]))?;
            Some(Passed {
                score: None,
                hits: vec![hit(at[0], at[1])],
            })
        }
        BiomeTest::Coverage(percent) => {
            let total = disc_size(radius, step);
            let wanted = (percent as f64 / 100.0 * total as f64).ceil() as usize;
            let (mut inside, mut first) = (0, None);
            for (i, at) in points().enumerate() {
                if is_biome(at[0], at[1]) {
                    inside += 1;
                    first.get_or_insert(at);
                }
                // it can't get there anymore
                if inside + total - i - 1 < wanted {
                    return None;
                }
            }
            (inside >= wanted.max(1)).then(|| Passed {
                score: Some(inside as f64 * 100.0 / total as f64),
                hits: first.map(|at| hit(at[0], at[1])).into_iter().collect(),
            })
        }
        BiomeTest::Area(area) => {
            let patch = largest_patch(&mut is_biome, &|| world.stopped(), area, radius, center, step)?;
            let middle = patch.middle();
            Some(Passed {
                score: Some(patch.area(step) as f64),
                hits: vec![hit(middle[0], middle[1])],
            })
        }
    }
}

impl SeedWorld {
    fn check(&self, condition: &Condition, here: [i32; 2], cells: &mut Cells) -> Option<Passed> {
        match condition {
            Condition::Poi {
                kind,
                count,
                radius,
                anchor,
            } => {
                let center = self.center(*anchor, MARKER_KINDS[*kind].dimension, here);
                if spread(*kind) != Some(Spread::Rings) {
                    cells.clear();
                    candidates(self.seed, *kind, center, *radius, cells);
                    if cells.len() < *count as usize {
                        return None;
                    }
                    let source = self.source(MARKER_KINDS[*kind].dimension);
                    allowed_by_biomes(source, *kind, cells);
                    if cells.len() < *count as usize {
                        return None;
                    }
                }
                let found = self
                    .finder(MARKER_KINDS[*kind].dimension)?
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
            Condition::Cluster {
                kind,
                count,
                radius,
                within,
                anchor,
            } => {
                let center = self.center(*anchor, MARKER_KINDS[*kind].dimension, here);
                let within = *within;
                if spread(*kind) != Some(Spread::Rings) {
                    cells.clear();
                    candidates(self.seed, *kind, center, within, cells);
                    let margin = crate::structures::marker_margin(*kind);
                    if !could_cluster(cells, radius + margin, *count) {
                        return None;
                    }
                    allowed_by_biomes(self.source(MARKER_KINDS[*kind].dimension), *kind, cells);
                    if !could_cluster(cells, radius + margin, *count) {
                        return None;
                    }
                }
                let found = self
                    .finder(MARKER_KINDS[*kind].dimension)?
                    .around(*kind, center, within);
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
            Condition::Biome {
                biome,
                test,
                radius,
                anchor,
            } => check_biome(self, biome, *test, *radius, *anchor, here),
            Condition::Noise {
                parameter,
                min,
                max,
                coverage,
                radius,
                anchor,
            } => {
                let center = self.center(*anchor, Dimension::Overworld, here);
                let climate = self.climate()?;
                let mut cache = NoiseCache::default();
                let qy = Dimension::Overworld.default_height() >> 2;
                let total = disc_size(*radius, self.step);
                let wanted = ((*coverage as f64 / 100.0 * total as f64).ceil() as usize).max(1);
                let (mut inside, mut first) = (0, None);
                for (i, at) in disc_points(center, *radius, self.step).enumerate() {
                    if self.stopped() {
                        return None;
                    }
                    let value = climate.value(&mut cache, *parameter, [at[0] >> 2, qy, at[1] >> 2]);
                    if (*min..=*max).contains(&value) {
                        inside += 1;
                        first.get_or_insert(at);
                        if *coverage == 0.0 {
                            break;
                        }
                    }
                    if inside + total - i - 1 < wanted {
                        return None;
                    }
                }
                (inside >= wanted).then(|| Passed {
                    score: (*coverage > 0.0).then(|| inside as f64 * 100.0 / total as f64),
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
            Condition::Not(condition) => match self.check(condition, here, cells) {
                Some(_) => None,
                None => Some(Passed::default()),
            },
            Condition::Any(conditions) => conditions
                .iter()
                .find_map(|condition| self.check(condition, here, cells)),
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
        dimension: MARKER_KINDS[found.kind].dimension,
        x: found.x,
        z: found.z,
    }
}

/// A seed or location that passed all conditions.
#[derive(Clone, PartialEq, Debug)]
pub struct Match {
    pub seed: i64,
    /// the score of the first condition that has one, more is better
    pub score: Option<f64>,
    pub hits: Vec<Hit>,
}

/// A search with its conditions in the order they are checked.
pub struct Plan {
    /// condition index in the search and condition, the cheapest first
    conditions: Vec<(usize, Condition)>,
    /// conditions that can turn a seed down from its lower 48 bits alone
    positional: Vec<usize>,
}

/// About how long a condition takes, to check the cheap ones first.
fn cost(condition: &Condition) -> u32 {
    let anchored = |anchor: &Anchor| if *anchor == Anchor::Spawn { 3 } else { 0 };
    match condition {
        Condition::Poi { kind, anchor, .. } | Condition::Cluster { kind, anchor, .. } => {
            if spread(*kind) == Some(Spread::Rings) {
                100
            } else {
                1 + anchored(anchor)
            }
        }
        Condition::Biome { anchor, .. } | Condition::Noise { anchor, .. } => 2 + anchored(anchor),
        Condition::Not(condition) => cost(condition),
        Condition::Any(conditions) => conditions.iter().map(cost).sum(),
    }
}

impl Plan {
    pub fn new(conditions: &[Condition]) -> Self {
        let mut ordered: Vec<(usize, Condition)> = conditions.iter().cloned().enumerate().collect();
        ordered.sort_by_key(|(_, condition)| cost(condition));
        let positional = ordered
            .iter()
            .enumerate()
            .filter(|(_, (_, condition))| match condition {
                Condition::Poi { kind, anchor, .. } | Condition::Cluster { kind, anchor, .. } => {
                    matches!(anchor, Anchor::At(..))
                        && spread(*kind) != Some(Spread::Rings)
                        && depends_on_lower_bits(*kind)
                }
                _ => false,
            })
            .map(|(i, _)| i)
            .collect();
        Self {
            conditions: ordered,
            positional,
        }
    }

    /// Whether going through upper bits of seeds that passed `lower_bits_pass` is worth it.
    pub fn has_positional(&self) -> bool {
        !self.positional.is_empty()
    }

    /// False if no seed with the same lower 48 bits can pass. Needs no world, takes nanoseconds.
    pub fn lower_bits_pass(&self, seed: i64, cells: &mut Cells) -> bool {
        self.positional.iter().all(|&i| match &self.conditions[i].1 {
            Condition::Poi {
                kind,
                count,
                radius,
                anchor: Anchor::At(x, z),
            } => {
                cells.clear();
                candidates(seed, *kind, [*x, *z], *radius, cells);
                cells.len() >= *count as usize
            }
            Condition::Cluster {
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

    /// Checks all conditions, `here` is what `Anchor::Target` means.
    pub fn check(&self, world: &SeedWorld, here: [i32; 2], cells: &mut Cells) -> Option<Match> {
        let mut passed = Vec::with_capacity(self.conditions.len());
        for (index, condition) in &self.conditions {
            passed.push((*index, world.check(condition, here, cells)?));
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
    use crate::{noise::NoiseParameter, seedfind::spec::parse_seeds as parse};

    fn with_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap();
    }

    /// Patches against a slow flood fill, on random grids of all densities.
    #[test]
    fn patches_are_those_a_flood_fill_finds() {
        let (center, radius, step) = ([100, -50], 200, 8);
        let mut state = 12345u64;
        let mut random = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as u32
        };
        for round in 0..300 {
            let density = round % 10 + 1;
            let n = radius / step;
            let inside: std::collections::HashSet<[i32; 2]> = (-n..=n)
                .flat_map(|dz| disc_row(radius, step, dz).map(move |dx| [dx, dz]))
                .filter(|_| random() % 11 < density as u32)
                .collect();
            let mut scan = PatchScan::new(center, step);
            let mut found = Vec::new();
            for dz in -n..=n {
                let mut is_inside = |x: i32, z: i32| inside.contains(&[(x - center[0]) / step, (z - center[1]) / step]);
                let runs = row_runs(center, radius, step, dz, &mut is_inside);
                found.extend(scan.push_row(dz, &runs));
            }
            found.extend(scan.finish());
            // the flood fill
            let mut seen = std::collections::HashSet::new();
            let mut wanted = Vec::new();
            for start in &inside {
                if !seen.insert(*start) {
                    continue;
                }
                let mut stack = vec![*start];
                let (mut points, mut sum) = (0i64, [0i64; 2]);
                while let Some([x, z]) = stack.pop() {
                    points += 1;
                    sum[0] += (center[0] + x * step) as i64;
                    sum[1] += (center[1] + z * step) as i64;
                    for next in [[x + 1, z], [x - 1, z], [x, z + 1], [x, z - 1]] {
                        if inside.contains(&next) && seen.insert(next) {
                            stack.push(next);
                        }
                    }
                }
                wanted.push(Patch { points, sum });
            }
            let key = |patch: &Patch| (patch.points, patch.sum);
            found.sort_by_key(key);
            wanted.sort_by_key(key);
            assert_eq!(found, wanted, "round {round}");
        }
    }

    #[test]
    fn disc_points_are_each_point_once_nearest_rings_first() {
        for (radius, step) in [(100, 4), (1000, 16), (17, 16), (5, 16)] {
            let points: Vec<[i32; 2]> = disc_points([7, -3], radius, step).collect();
            assert_eq!(points.len(), disc_size(radius, step), "{radius} {step}");
            let unique: std::collections::HashSet<_> = points.iter().collect();
            assert_eq!(unique.len(), points.len());
            assert_eq!(points[0], [7, -3]);
            let ring = |p: &[i32; 2]| ((p[0] - 7).abs()).max((p[1] + 3).abs());
            assert!(points.windows(2).all(|pair| ring(&pair[0]) <= ring(&pair[1])));
        }
    }

    #[test]
    fn circles_can_be_made_to_hold_a_point() {
        let points = [[0, 0], [10, 0], [12, 0], [30, 0]];
        assert_eq!(best_circle(&points, 6).0, 3);
        assert_eq!(best_circle_with(&points, 9, Some([30, 0])).0, 2);
    }

    #[test]
    fn spawn_is_where_the_dimension_has_it() {
        let world = SeedWorld::new(12345, 16);
        let spawn = Dimension::Overworld.biome_source(12345).initial_spawn_search_origin();
        let center = |dimension| world.center(Anchor::Spawn, dimension, [7, 7]);
        assert_eq!(center(Dimension::Overworld), [spawn.x(), spawn.z()]);
        assert_eq!(center(Dimension::End), END_SPAWN);
    }

    /// Their search would run forever, so the parser refuses them.
    #[test]
    fn biomes_refused_by_the_parser_are_really_never_sampled() {
        let source = Dimension::Overworld.biome_source(1);
        let mut sampler = source.chunk_sampler();
        let qy = Dimension::Overworld.default_height() >> 2;
        for i in 0..250_000 {
            let biome = sampler.sample((i % 500 - 250) * 64, qy, (i / 500 - 250) * 64);
            assert!(
                crate::seedfind::spec::searchable_biomes().any(|name| name == &*biome.key.path),
                "{}",
                biome.key.path
            );
        }
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
                "poi swamp_hut count>=1 within 1500 of 0,0\nbiome plains within 800 of 0,0",
                "cluster swamp_hut count>=2 radius 200 within 3000 of 100,-100",
                "poi village count>=2 within 1500 of 0,0\nnot biome mushroom_fields within 500 of 0,0",
                "any\n  poi igloo count>=1 within 2000 of 0,0\n  biome desert within 300 of 0,0\nend",
            ];
            let mut matches = 0;
            for text in cases {
                let conditions = parse(text).unwrap();
                let plan = Plan::new(&conditions);
                let mut cells = Vec::new();
                for seed in 0..60i64 {
                    let seed = seed.wrapping_mul(0x0123_4567_89AB_CDEF);
                    let world = SeedWorld::new(seed, 16);
                    // every condition, no shortcut: `around` asks the finder about all candidates
                    let plain = conditions.iter().all(|condition| match condition {
                        Condition::Poi {
                            kind,
                            count,
                            radius,
                            anchor: Anchor::At(x, z),
                        } => world
                            .finder(MARKER_KINDS[*kind].dimension)
                            .is_some_and(|f| f.around(*kind, [*x, *z], *radius).len() >= *count as usize),
                        condition => world.check(condition, [0, 0], &mut cells).is_some(),
                    });
                    let planned = plan.lower_bits_pass(seed, &mut cells)
                        && plan.check(&SeedWorld::new(seed, 16), [0, 0], &mut cells).is_some();
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
                if wide.check(&SeedWorld::new(seed, 16), [0, 0], &mut cells).is_some() {
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
            assert!(plan.check(&SeedWorld::new(1, 16), [0, 0], &mut Vec::new()).is_some());
            let plan = Plan::new(&parse("noise continentalness 5..6 within 100 of 0,0").unwrap());
            assert!(plan.check(&SeedWorld::new(1, 16), [0, 0], &mut Vec::new()).is_none());
        });
    }
}
