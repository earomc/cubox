//! What structures and search ask of background workers, and which questions to ask when.
//! Where the work runs differs, see `finder_native.rs` and `finder_web.rs`. Workers are stateless
//! besides caching the world, so stopping a search is just not asking any further.

use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
};

use steel_worldgen::biomes::BiomeSourceKind;

use crate::{
    dimension::Dimension,
    gui::finder_pool::Pool,
    structures::{Finder, Found, STRUCTURE_KINDS, Spread, bin_budget, spread},
};

/// Which world questions are about.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FinderWorld {
    pub seed: i64,
    pub dimension: Dimension,
    pub sample_height: i32,
}

/// Blocks searched per biome job, and how far apart its samples are.
const BIOME_BAND: i32 = 1024;
const BIOME_STEP: i32 = 32;
/// How far a search for a biome goes in blocks.
const BIOME_LIMIT: i32 = 100_000;
/// Hits closer than this are the same patch of biome.
const BIOME_PATCH: i32 = 512;
/// How far a search for a structure goes in blocks.
const STRUCTURE_LIMIT: i64 = 300_000;
/// Results a search ends at.
const SEARCH_RESULTS: usize = 10;
/// Structures shown are at least this far apart when zoomed out, in points. What this keeps from
/// the screen is the work: it stays the same however much of the world is visible.
const MIN_BIN_POINTS: f64 = 48.0;
/// Bins per axis of a job when zoomed out.
const FAR_JOB_SIDE: i32 = 4;
/// The edge of the world in blocks.
const WORLD_EDGE: f64 = 30_000_000.0;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Job {
    /// a square of `side` bins of `4^level` cells of a kind's grid, up to one structure per bin
    Bins {
        kind: usize,
        level: u32,
        x: i32,
        z: i32,
        side: i32,
    },
    Rings {
        kind: usize,
    },
    /// samples on the square shells from `from` up to `to` blocks around `center`
    Biome {
        biome: String,
        center: [i32; 2],
        from: i32,
        to: i32,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Reply {
    Found(Vec<Found>),
    Biome(Vec<[i32; 2]>),
}

#[cfg(any(target_arch = "wasm32", test))]
fn numbers(words: &[&str]) -> Option<Vec<i32>> {
    words.iter().map(|word| word.parse().ok()).collect()
}

#[cfg(any(target_arch = "wasm32", test))]
impl Job {
    /// One line of text, for sending to web workers.
    pub fn encode(&self) -> String {
        match self {
            Job::Bins {
                kind,
                level,
                x,
                z,
                side,
            } => format!("bins {kind} {level} {x} {z} {side}"),
            Job::Rings { kind } => format!("rings {kind}"),
            Job::Biome {
                biome,
                center,
                from,
                to,
            } => {
                format!("biome {biome} {} {} {from} {to}", center[0], center[1])
            }
        }
    }

    pub fn decode(line: &str) -> Option<Self> {
        let words: Vec<&str> = line.split(' ').collect();
        match *words.first()? {
            "bins" => {
                let n = numbers(&words[1..])?;
                Some(Job::Bins {
                    kind: *n.first()? as usize,
                    level: *n.get(1)? as u32,
                    x: *n.get(2)?,
                    z: *n.get(3)?,
                    side: *n.get(4)?,
                })
            }
            "rings" => Some(Job::Rings {
                kind: words.get(1)?.parse().ok()?,
            }),
            "biome" => {
                let n = numbers(words.get(2..)?)?;
                Some(Job::Biome {
                    biome: words.get(1)?.to_string(),
                    center: [*n.first()?, *n.get(1)?],
                    from: *n.get(2)?,
                    to: *n.get(3)?,
                })
            }
            _ => None,
        }
    }
}

#[cfg(any(target_arch = "wasm32", test))]
impl Reply {
    pub fn encode(&self) -> String {
        let mut line = String::new();
        match self {
            Reply::Found(found) => {
                line.push('f');
                for f in found {
                    line += &format!(" {} {} {}", f.kind, f.x, f.z);
                }
            }
            Reply::Biome(hits) => {
                line.push('b');
                for hit in hits {
                    line += &format!(" {} {}", hit[0], hit[1]);
                }
            }
        }
        line
    }

    pub fn decode(line: &str) -> Option<Self> {
        let words: Vec<&str> = line.split(' ').collect();
        let n = numbers(&words[1..])?;
        match words[0] {
            "f" => Some(Reply::Found(
                n.as_chunks::<3>()
                    .0
                    .iter()
                    .map(|c| Found {
                        kind: c[0] as usize,
                        x: c[1],
                        z: c[2],
                    })
                    .collect(),
            )),
            "b" => Some(Reply::Biome(n.as_chunks::<2>().0.to_vec())),
            _ => None,
        }
    }
}

/// Answers jobs for one world, shared by the workers that have shared memory.
pub struct WorldQueries {
    pub world: FinderWorld,
    finder: OnceLock<Option<Finder>>,
    source: OnceLock<BiomeSourceKind>,
}

impl WorldQueries {
    pub fn new(world: FinderWorld) -> Self {
        Self {
            world,
            finder: OnceLock::new(),
            source: OnceLock::new(),
        }
    }

    fn finder(&self) -> Option<&Finder> {
        self.finder
            .get_or_init(|| match Finder::new(self.world.seed, self.world.dimension) {
                Ok(finder) => Some(finder),
                Err(err) => {
                    log::error!("setting up structures: {err:#}");
                    None
                }
            })
            .as_ref()
    }

    pub fn run(&self, job: &Job) -> Reply {
        match job {
            Job::Bins {
                kind,
                level,
                x,
                z,
                side,
            } => {
                let found = self.finder().map_or(Vec::new(), |finder| {
                    let budget = spread(*kind).map_or(0, |spread| match spread {
                        Spread::Grid(spacing) => bin_budget(spacing),
                        Spread::Rings => 0,
                    });
                    (*z..z + side)
                        .flat_map(|z| (*x..x + side).map(move |x| (x, z)))
                        .filter_map(|(x, z)| finder.in_bin(*kind, *level, x, z, budget))
                        .collect()
                });
                Reply::Found(found)
            }
            Job::Rings { kind } => Reply::Found(self.finder().map_or(Vec::new(), |finder| finder.rings(*kind))),
            Job::Biome {
                biome,
                center,
                from,
                to,
            } => Reply::Biome(self.scan_biome(biome, *center, *from, *to)),
        }
    }

    fn scan_biome(&self, biome: &str, center: [i32; 2], from: i32, to: i32) -> Vec<[i32; 2]> {
        let source = self
            .source
            .get_or_init(|| self.world.dimension.biome_source(self.world.seed));
        let mut sampler = source.chunk_sampler();
        let qy = self.world.sample_height >> 2;
        let mut hits = Vec::new();
        for radius in (from..to).step_by(BIOME_STEP as usize) {
            let steps = (radius / BIOME_STEP).max(1);
            for i in 0..steps * 8 {
                let (dx, dz) = shell_position(i, steps);
                let (x, z) = (center[0] + dx * BIOME_STEP, center[1] + dz * BIOME_STEP);
                if &*sampler.sample(x >> 2, qy, z >> 2).key.path == biome {
                    hits.push([x, z]);
                }
            }
        }
        hits
    }
}

/// Position `i` of `radius * 8` around a square shell of that radius.
fn shell_position(i: i32, radius: i32) -> (i32, i32) {
    let side = radius * 2;
    match i / side {
        0 => (-radius + i % side, -radius),
        1 => (radius, -radius + i % side),
        2 => (radius - i % side, radius),
        _ => (-radius, radius - i % side),
    }
}

/// How many cells of a kind's grid one job covers per axis when all of them are looked at. A job
/// has to last longer than a frame, workers only get new ones once per frame. Tight grids are
/// cheap but many.
fn job_side(spacing: i32) -> i32 {
    (16 / spacing).max(4)
}

/// The level of bins for a kind at this zoom: the smallest with bins of at least `MIN_BIN_POINTS`.
fn bin_level(spacing: i32, blocks_per_point: f64) -> u32 {
    let cell_points = (spacing * 16) as f64 / blocks_per_point;
    (MIN_BIN_POINTS / cell_points).log2().ceil().max(0.0) as u32
}

/// What is known and wanted of a kind in the visible part of the world.
#[derive(Default)]
pub struct Visible {
    /// nearest to the center first
    pub jobs: Vec<Job>,
    pub known: Vec<Found>,
    /// only some of the structures are shown at this zoom
    pub sampled: bool,
}

/// What a search looks for.
#[derive(Clone, PartialEq, Debug)]
pub enum Target {
    Structure(usize),
    Biome(String),
}

impl Target {
    pub fn name(&self) -> String {
        match self {
            Target::Structure(kind) => STRUCTURE_KINDS[*kind].name.to_string(),
            Target::Biome(biome) => biome.replace('_', " "),
        }
    }
}

pub struct Search {
    pub target: Target,
    /// where it started from, in blocks
    pub center: [i32; 2],
    /// nearest first, in blocks
    pub results: Vec<[i32; 2]>,
    /// how far it has looked in blocks
    pub reached: i64,
    pub done: bool,
    /// the shell being worked on, in cells for structures and bands for biomes
    shell: i32,
    /// the shell after results were enough, in which something closer might still be
    last_shell: Option<i32>,
}

impl Search {
    /// The biome job of the band being searched.
    fn band_job(&self) -> Option<Job> {
        let Target::Biome(biome) = &self.target else {
            return None;
        };
        let from = self.shell * BIOME_BAND;
        Some(Job::Biome {
            biome: biome.clone(),
            center: self.center,
            from,
            to: from + BIOME_BAND,
        })
    }
}

/// Asks the workers what is wanted and remembers the answers.
pub struct Queries {
    pool: Pool,
    world: FinderWorld,
    /// bumped when the world changes, replies of older ones are dropped
    generation: u64,
    /// the replies to `Job::Bins` by kind, level, x and z
    bins: HashMap<(usize, u32, i32, i32), Vec<Found>>,
    rings: HashMap<usize, Vec<Found>>,
    in_flight: HashSet<Job>,
    pub search: Option<Search>,
}

impl Queries {
    pub fn new(world: FinderWorld) -> Self {
        Self {
            pool: Pool::new(),
            world,
            generation: 0,
            bins: HashMap::new(),
            rings: HashMap::new(),
            in_flight: HashSet::new(),
            search: None,
        }
    }

    /// Forgets everything if the world changed. The height only changes what biome searches see.
    pub fn set_world(&mut self, world: FinderWorld) {
        let same_structures = self.world.seed == world.seed && self.world.dimension == world.dimension;
        if self.world == world {
            return;
        }
        self.world = world;
        self.generation += 1;
        self.in_flight.clear();
        self.search = None;
        if !same_structures {
            self.bins.clear();
            self.rings.clear();
        }
    }

    pub fn busy(&self) -> bool {
        !self.in_flight.is_empty() || self.search.as_ref().is_some_and(|search| !search.done)
    }

    pub fn clear_search(&mut self) {
        self.search = None;
    }

    pub fn start_search(&mut self, target: Target, center: [i32; 2]) {
        self.search = Some(Search {
            target,
            center,
            results: Vec::new(),
            reached: 0,
            done: false,
            shell: 0,
            last_shell: None,
        });
    }

    /// The visible part of a kind. `min` and `max` are in blocks.
    pub fn visible(&self, kind: usize, min: [f64; 2], max: [f64; 2], blocks_per_point: f64) -> Visible {
        match spread(kind) {
            None => Visible::default(),
            Some(Spread::Rings) => Visible {
                jobs: vec![Job::Rings { kind }],
                known: self.rings.get(&kind).cloned().unwrap_or_default(),
                sampled: false,
            },
            Some(Spread::Grid(spacing)) => {
                let level = bin_level(spacing, blocks_per_point);
                let side = if level == 0 { job_side(spacing) } else { FAR_JOB_SIDE };
                let block = spacing as f64 * (1i64 << level) as f64 * side as f64 * 16.0;
                let range = |lo: f64, hi: f64| {
                    (lo.max(-WORLD_EDGE) / block).floor() as i64..=(hi.min(WORLD_EDGE) / block).floor() as i64
                };
                let (xs, zs) = (range(min[0], max[0]), range(min[1], max[1]));
                let center = [(min[0] + max[0]) / 2.0 / block, (min[1] + max[1]) / 2.0 / block];
                let mut blocks: Vec<(i64, i64)> = zs.flat_map(|z| xs.clone().map(move |x| (x, z))).collect();
                let distance =
                    |&(x, z): &(i64, i64)| (x as f64 + 0.5 - center[0]).powi(2) + (z as f64 + 0.5 - center[1]).powi(2);
                blocks.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
                let mut visible = Visible {
                    sampled: 4u64.saturating_pow(level) > bin_budget(spacing) as u64,
                    ..Default::default()
                };
                for (x, z) in blocks {
                    let (x, z) = (x as i32 * side, z as i32 * side);
                    match self.bins.get(&(kind, level, x, z)) {
                        Some(found) => visible.known.extend_from_slice(found),
                        None => {
                            visible.jobs.push(Job::Bins {
                                kind,
                                level,
                                x,
                                z,
                                side,
                            });
                            self.nearby_levels(kind, level, [x, z], side, spacing, &mut visible.known);
                        }
                    }
                }
                visible
            }
        }
    }

    /// What other zoom levels know of a job that isn't answered yet, so markers stay while zooming.
    fn nearby_levels(&self, kind: usize, level: u32, at: [i32; 2], side: i32, spacing: i32, out: &mut Vec<Found>) {
        let side_at = |level: u32| if level == 0 { job_side(spacing) } else { FAR_JOB_SIDE };
        // one level coarser
        let coarse_side = side_at(level + 1);
        let align = |v: i32| (v >> 1).div_euclid(coarse_side) * coarse_side;
        if let Some(found) = self.bins.get(&(kind, level + 1, align(at[0]), align(at[1]))) {
            out.extend_from_slice(found);
        }
        // one level finer
        if level > 0 {
            let fine_side = side_at(level - 1);
            let (from, to) = ((at[0] * 2, at[1] * 2), side * 2);
            for x in (from.0..from.0 + to).step_by(fine_side as usize) {
                for z in (from.1..from.1 + to).step_by(fine_side as usize) {
                    if let Some(found) = self.bins.get(&(kind, level - 1, x, z)) {
                        out.extend_from_slice(found);
                    }
                }
            }
        }
    }

    /// Takes the workers' answers and gives idle workers the next jobs: for the search first, then
    /// `wanted`, most important first.
    pub fn update(&mut self, wanted: &[Job]) {
        let replies: Vec<_> = self.pool.finished().collect();
        for (generation, job, reply) in replies {
            if generation != self.generation {
                continue;
            }
            self.in_flight.remove(&job);
            self.store(job, reply);
        }
        let search_jobs = self.search_jobs();
        for job in search_jobs.iter().chain(wanted) {
            if !self.pool.has_idle() {
                break;
            }
            if self.in_flight.contains(job) || self.is_known(job) {
                continue;
            }
            self.pool.post(self.world, self.generation, job.clone());
            self.in_flight.insert(job.clone());
        }
    }

    fn is_known(&self, job: &Job) -> bool {
        match job {
            Job::Bins { kind, level, x, z, .. } => self.bins.contains_key(&(*kind, *level, *x, *z)),
            Job::Rings { kind } => self.rings.contains_key(kind),
            Job::Biome { .. } => false,
        }
    }

    fn store(&mut self, job: Job, reply: Reply) {
        match (job, reply) {
            (Job::Bins { kind, level, x, z, .. }, Reply::Found(found)) => {
                self.bins.insert((kind, level, x, z), found);
            }
            (Job::Rings { kind }, Reply::Found(found)) => {
                self.rings.insert(kind, found);
            }
            (job @ Job::Biome { .. }, Reply::Biome(hits)) => {
                let Job::Biome { to, .. } = job else { unreachable!() };
                // a reply of a search that was replaced has no place here
                if let Some(search) = &mut self.search
                    && search.band_job().as_ref() == Some(&job)
                {
                    search.reached = to as i64;
                    for hit in hits {
                        let far_enough = search
                            .results
                            .iter()
                            .all(|r| (r[0] - hit[0]).abs().max((r[1] - hit[1]).abs()) >= BIOME_PATCH);
                        if far_enough {
                            search.results.push(hit);
                        }
                    }
                    sort_by_distance(&mut search.results, search.center);
                    if search.results.len() >= SEARCH_RESULTS || to >= BIOME_LIMIT {
                        search.results.truncate(SEARCH_RESULTS);
                        search.done = true;
                    }
                    search.shell += 1;
                }
            }
            _ => log::error!("a reply that doesn't fit its job"),
        }
    }

    /// What the running search needs next. Advances it when its shell is answered.
    fn search_jobs(&mut self) -> Vec<Job> {
        let Some(search) = &mut self.search else {
            return Vec::new();
        };
        if search.done {
            return Vec::new();
        }
        match search.target.clone() {
            Target::Biome(_) => search.band_job().into_iter().collect(),
            Target::Structure(kind) => match spread(kind) {
                Some(Spread::Rings) => {
                    let Some(found) = self.rings.get(&kind) else {
                        return vec![Job::Rings { kind }];
                    };
                    search.results = found.iter().map(|f| [f.x, f.z]).collect();
                    sort_by_distance(&mut search.results, search.center);
                    search.results.truncate(SEARCH_RESULTS);
                    search.done = true;
                    Vec::new()
                }
                Some(Spread::Grid(spacing)) => {
                    let side = job_side(spacing);
                    let block = spacing * side * 16;
                    let center = [search.center[0].div_euclid(block), search.center[1].div_euclid(block)];
                    loop {
                        let jobs = shell_blocks(search.shell)
                            .into_iter()
                            .map(|(dx, dz)| ((center[0] + dx) * side, (center[1] + dz) * side))
                            .filter(|&(x, z)| !self.bins.contains_key(&(kind, 0, x, z)))
                            .map(|(x, z)| Job::Bins {
                                kind,
                                level: 0,
                                x,
                                z,
                                side,
                            })
                            .collect::<Vec<_>>();
                        if !jobs.is_empty() {
                            return jobs;
                        }
                        // the shell is answered
                        for (dx, dz) in shell_blocks(search.shell) {
                            let key = (kind, 0, (center[0] + dx) * side, (center[1] + dz) * side);
                            search.results.extend(self.bins[&key].iter().map(|f| [f.x, f.z]));
                        }
                        sort_by_distance(&mut search.results, search.center);
                        search.reached = search.shell as i64 * block as i64;
                        if search.results.len() >= SEARCH_RESULTS && search.last_shell.is_none() {
                            search.last_shell = Some(search.shell + 1);
                        }
                        if search.last_shell.is_some_and(|last| search.shell >= last)
                            || search.reached >= STRUCTURE_LIMIT
                        {
                            search.results.truncate(SEARCH_RESULTS);
                            search.done = true;
                            return Vec::new();
                        }
                        search.shell += 1;
                    }
                }
                None => {
                    search.done = true;
                    Vec::new()
                }
            },
        }
    }
}

fn sort_by_distance(results: &mut [[i32; 2]], center: [i32; 2]) {
    results.sort_by_key(|r| {
        let (dx, dz) = ((r[0] - center[0]) as i64, (r[1] - center[1]) as i64);
        dx * dx + dz * dz
    });
}

/// The cells at distance `shell` from the center cell.
fn shell_blocks(shell: i32) -> Vec<(i32, i32)> {
    if shell == 0 {
        return vec![(0, 0)];
    }
    (0..shell * 8).map(|i| shell_position(i, shell)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_survive_encoding() {
        for job in [
            Job::Bins {
                kind: 3,
                level: 2,
                x: -4,
                z: 7,
                side: 16,
            },
            Job::Rings { kind: 2 },
            Job::Biome {
                biome: "dark_forest".into(),
                center: [-10, 20],
                from: 0,
                to: 1024,
            },
        ] {
            assert_eq!(Job::decode(&job.encode()), Some(job));
        }
        for reply in [
            Reply::Found(vec![]),
            Reply::Found(vec![Found { kind: 1, x: -5, z: 6 }]),
            Reply::Biome(vec![[1, 2], [-3, 4]]),
        ] {
            assert_eq!(Reply::decode(&reply.encode()), Some(reply));
        }
    }

    fn run_search(target: Target) -> Vec<[i32; 2]> {
        let mut queries = Queries::new(FinderWorld {
            seed: 12345,
            dimension: Dimension::Overworld,
            sample_height: 256,
        });
        queries.start_search(target, [1000, 1000]);
        while queries.busy() {
            queries.update(&[]);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        queries.search.unwrap().results
    }

    #[test]
    fn searches_find_the_nearest_first() {
        let biomes = run_search(Target::Biome("dark_forest".into()));
        println!("{biomes:?}");
        assert_eq!(biomes.len(), SEARCH_RESULTS);
        let source = Dimension::Overworld.biome_source(12345);
        let mut sampler = source.chunk_sampler();
        for hit in &biomes {
            assert_eq!(&*sampler.sample(hit[0] >> 2, 64, hit[1] >> 2).key.path, "dark_forest");
        }
        let kind = STRUCTURE_KINDS.iter().position(|kind| kind.id == "village").unwrap();
        let villages = run_search(Target::Structure(kind));
        println!("{villages:?}");
        assert_eq!(villages.len(), SEARCH_RESULTS);
        let distance = |r: &[i32; 2]| ((r[0] - 1000) as f64).hypot((r[1] - 1000) as f64);
        assert!(villages.is_sorted_by_key(|r| distance(r) as i64));
        let kind = STRUCTURE_KINDS.iter().position(|kind| kind.id == "stronghold").unwrap();
        assert_eq!(run_search(Target::Structure(kind)).len(), SEARCH_RESULTS);
    }

    #[test]
    fn shells_visit_every_position_once() {
        for radius in 1..5 {
            let mut shell = shell_blocks(radius);
            assert_eq!(shell.len() as i32, radius * 8);
            assert!(shell.iter().all(|&(x, z)| x.abs().max(z.abs()) == radius));
            shell.sort();
            shell.dedup();
            assert_eq!(shell.len() as i32, radius * 8);
        }
    }
}
