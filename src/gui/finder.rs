//! What structures and search ask of background workers, and which questions to ask when.
//! Where the work runs differs, see `finder_native.rs` and `finder_web.rs`. Workers are stateless
//! besides caching the world, so stopping a search is just not asking any further.

use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    sync::OnceLock,
};

use steel_worldgen::biomes::BiomeSourceKind;

use crate::{
    dimension::Dimension,
    gui::finder_pool::Pool,
    structures::{Finder, Found, STRUCTURE_KINDS, Spread, bin_rounds, rarity, real_rate, spread},
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
/// Bins remembered at most, about 100 bytes each. Beyond that they are looked for again if needed.
const MAX_BINS: usize = 1_000_000;
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
        /// which candidates to try, all of them if empty
        tries: Range<u32>,
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
                tries,
            } => format!("bins {kind} {level} {x} {z} {side} {} {}", tries.start, tries.end),
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
                    tries: *n.get(5)? as u32..*n.get(6)? as u32,
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
                tries,
            } => {
                let found = self.finder().map_or(Vec::new(), |finder| {
                    (*z..z + side)
                        .flat_map(|z| (*x..x + side).map(move |x| (x, z)))
                        .flat_map(|(x, z)| {
                            if tries.is_empty() {
                                finder.all_in_bin(*kind, *level, x, z)
                            } else {
                                finder.in_bin(*kind, *level, x, z, tries.clone()).into_iter().collect()
                            }
                        })
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

/// Level 0 looks at every cell, from there on a job is one bin, as each one is looked through for
/// as long as it takes to find something.
fn job_side_at(level: u32, spacing: i32) -> i32 {
    if level == 0 { job_side(spacing) } else { 1 }
}

/// Which bins to look at for about `target` structures among `cells` grid cells, of which `rate`
/// really generate: a level, and the share of its bins. The coarsest level that can still give as
/// many, as that is the least work and spreads them evenly. The expected number is the target, or
/// everything if there aren't that many, so it doesn't jump while zooming.
fn plan_bins(cells: f64, rate: f64, target: f64, tries: u32) -> (u32, f64) {
    let expected = |level: u32| {
        let candidates = 4f64.powi(level as i32);
        let found_in_bin = 1.0 - (1.0 - rate).powf(candidates.min(tries as f64));
        cells / candidates * found_in_bin
    };
    if expected(0) <= target {
        return (0, 1.0);
    }
    let mut level = 0;
    while level < 24 && expected(level + 1) >= target {
        level += 1;
    }
    (level, target / expected(level))
}

/// Whether a bin is in the `share` of its level's bins looked at. A growing share only adds bins.
fn bin_is_chosen(kind: usize, level: u32, x: i64, z: i64, share: f64) -> bool {
    if share >= 1.0 {
        return true;
    }
    let mixed = (x as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((z as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
        .wrapping_add(((kind as u64) << 8) | level as u64);
    let mixed = (mixed ^ (mixed >> 29)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let mixed = mixed ^ (mixed >> 32);
    (mixed as f64 / u64::MAX as f64) < share
}

/// How markers are looked for at this zoom.
#[derive(Clone, Copy)]
pub struct MarkerSearch {
    /// about how many structures of a kind to find in view
    pub target: f64,
    /// how many candidates a square is given
    pub max_tries: u32,
}

/// What a job for bins has found so far.
#[derive(Default)]
struct BinState {
    found: Vec<Found>,
    /// how many candidates were tried, of which job `tries` is next
    tried: u32,
}

/// The markers that were placed on the screen, for not looking further where that can't change them.
pub trait Occupied {
    /// Whether a marker of this rarity or rarer is inside these blocks.
    fn has_marker(&self, rarity: usize, min: [f64; 2], max: [f64; 2]) -> bool;
}

/// What is known and wanted of a kind in the visible part of the world.
#[derive(Default)]
pub struct Visible {
    /// the jobs of each round of looking for structures, nearest to the center first
    pub jobs: Vec<Vec<Job>>,
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
    bins: HashMap<(usize, u32, i32, i32), BinState>,
    rings: HashMap<usize, Vec<Found>>,
    in_flight: HashSet<Job>,
    pub search: Option<Search>,
    /// jobs for the view that aren't answered yet
    waiting: usize,
    /// per kind the view and level last looked at, and the bins looked at there
    chosen: HashMap<usize, ChosenView>,
    /// changes when there is something new, see `revision`
    revision: u64,
}

type ChosenView = ((u32, f64, [f64; 2], [f64; 2]), Vec<(i32, i32)>);

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
            waiting: 0,
            chosen: HashMap::new(),
            revision: 0,
        }
    }

    /// Changes when an answer came in, until then nothing about the markers does.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// How many threads may work at once.
    pub fn set_thread_limit(&mut self, threads: usize) {
        self.pool.set_limit(threads);
    }

    /// How many areas of the view structures are being looked for in.
    pub fn waiting(&self) -> usize {
        self.waiting
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
        self.waiting = 0;
        self.revision += 1;
        self.search = None;
        if !same_structures {
            self.bins.clear();
            self.rings.clear();
        }
    }

    pub fn busy(&self) -> bool {
        self.waiting > 0 || !self.in_flight.is_empty() || self.search.as_ref().is_some_and(|search| !search.done)
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
    pub fn visible(
        &mut self,
        kind: usize,
        min: [f64; 2],
        max: [f64; 2],
        search: MarkerSearch,
        occupied: &dyn Occupied,
    ) -> Visible {
        match spread(kind) {
            None => Visible::default(),
            Some(Spread::Rings) => Visible {
                jobs: if self.rings.contains_key(&kind) {
                    Vec::new()
                } else {
                    vec![vec![Job::Rings { kind }]]
                },
                known: self.rings.get(&kind).cloned().unwrap_or_default(),
                sampled: false,
            },
            Some(Spread::Grid(spacing)) => {
                let cell = (spacing * 16) as f64;
                let (lo, hi) = (min.map(|v| v.max(-WORLD_EDGE)), max.map(|v| v.min(WORLD_EDGE)));
                let cells = ((hi[0] - lo[0]).max(0.0) / cell) * ((hi[1] - lo[1]).max(0.0) / cell);
                let rounds = bin_rounds(spacing, search.max_tries);
                let (level, share) = plan_bins(cells, real_rate(kind), search.target, rounds[rounds.len() - 1]);
                let side = job_side_at(level, spacing);
                let bins = self.chosen_bins(kind, spacing, level, share, side, min, max);
                let block = spacing as f64 * (1i64 << level) as f64 * side as f64 * 16.0;
                let mut visible = Visible {
                    sampled: level > 0 || share < 1.0,
                    ..Default::default()
                };
                let candidates = 4u64.saturating_pow(level);
                let rarity = rarity(kind);
                for (x, z) in bins {
                    let state = self.bins.get(&(kind, level, x, z));
                    if let Some(state) = state {
                        visible.known.extend_from_slice(&state.found);
                    }
                    let tried = state.map_or(0, |state| state.tried);
                    if level == 0 {
                        if state.is_none() {
                            if visible.jobs.is_empty() {
                                visible.jobs.push(Vec::new());
                            }
                            visible.jobs[0].push(Job::Bins {
                                kind,
                                level,
                                x,
                                z,
                                side,
                                tries: 0..0,
                            });
                            self.nearby_levels(kind, level, [x, z], side, spacing, &mut visible.known);
                        }
                        continue;
                    }
                    if state.is_some_and(|state| !state.found.is_empty()) {
                        continue;
                    }
                    let Some(round) = rounds
                        .iter()
                        .position(|&tries| tried < tries && (tried as u64) < candidates)
                    else {
                        continue;
                    };
                    if round == 0 {
                        self.nearby_levels(kind, level, [x, z], side, spacing, &mut visible.known);
                    } else {
                        // no use looking further where nothing more could be shown
                        let size = block / side as f64;
                        let (lo, hi) = (
                            [x as f64 * size, z as f64 * size],
                            [(x + 1) as f64 * size, (z + 1) as f64 * size],
                        );
                        if occupied.has_marker(rarity, lo, hi) {
                            continue;
                        }
                    }
                    if visible.jobs.len() <= round {
                        visible.jobs.resize(round + 1, Vec::new());
                    }
                    visible.jobs[round].push(Job::Bins {
                        kind,
                        level,
                        x,
                        z,
                        side,
                        tries: tried..(rounds[round] as u64).min(candidates) as u32,
                    });
                }
                visible
            }
        }
    }

    /// The bins in view that are looked at, nearest to the center first. Going through a lot of
    /// them takes a while, so they are remembered until the view changes.
    #[allow(clippy::too_many_arguments)]
    fn chosen_bins(
        &mut self,
        kind: usize,
        spacing: i32,
        level: u32,
        share: f64,
        side: i32,
        min: [f64; 2],
        max: [f64; 2],
    ) -> Vec<(i32, i32)> {
        let view = (level, share, min, max);
        if let Some((known_view, bins)) = self.chosen.get(&kind)
            && *known_view == view
        {
            return bins.clone();
        }
        let block = spacing as f64 * (1i64 << level) as f64 * side as f64 * 16.0;
        let range = |lo: f64, hi: f64| {
            (lo.max(-WORLD_EDGE) / block).floor() as i64..=(hi.min(WORLD_EDGE) / block).floor() as i64
        };
        let (xs, zs) = (range(min[0], max[0]), range(min[1], max[1]));
        let center = [(min[0] + max[0]) / 2.0 / block, (min[1] + max[1]) / 2.0 / block];
        let mut blocks: Vec<(i64, i64)> = zs
            .flat_map(|z| xs.clone().map(move |x| (x, z)))
            .filter(|&(x, z)| bin_is_chosen(kind, level, x, z, share))
            .collect();
        let distance =
            |&(x, z): &(i64, i64)| (x as f64 + 0.5 - center[0]).powi(2) + (z as f64 + 0.5 - center[1]).powi(2);
        blocks.sort_unstable_by(|a, b| distance(a).total_cmp(&distance(b)));
        let bins: Vec<(i32, i32)> = blocks
            .into_iter()
            .map(|(x, z)| (x as i32 * side, z as i32 * side))
            .collect();
        self.chosen.insert(kind, (view, bins.clone()));
        bins
    }

    /// What other zoom levels know of a job that isn't answered yet, so markers stay while zooming.
    fn nearby_levels(&self, kind: usize, level: u32, at: [i32; 2], side: i32, spacing: i32, out: &mut Vec<Found>) {
        // one level coarser
        let coarse_side = job_side_at(level + 1, spacing);
        let align = |v: i32, side: i32| v.div_euclid(side) * side;
        let coarse = (align(at[0] >> 1, coarse_side), align(at[1] >> 1, coarse_side));
        if let Some(state) = self.bins.get(&(kind, level + 1, coarse.0, coarse.1)) {
            out.extend_from_slice(&state.found);
        }
        // one level finer
        if level > 0 {
            let fine_side = job_side_at(level - 1, spacing);
            let (from, to) = (at.map(|v| align(v * 2, fine_side)), side * 2);
            for x in (from[0]..from[0] + to).step_by(fine_side as usize) {
                for z in (from[1]..from[1] + to).step_by(fine_side as usize) {
                    if let Some(state) = self.bins.get(&(kind, level - 1, x, z)) {
                        out.extend_from_slice(&state.found);
                    }
                }
            }
        }
    }

    /// Takes the workers' answers and gives idle workers the next jobs: for the search first, then
    /// `wanted`, most important first.
    pub fn update(&mut self, wanted: &[Job]) {
        self.waiting = wanted.len();
        let replies: Vec<_> = self.pool.finished().collect();
        for (generation, job, reply) in replies {
            if generation != self.generation {
                continue;
            }
            self.in_flight.remove(&job);
            self.store(job, reply);
            self.revision += 1;
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
            Job::Bins {
                kind,
                level,
                x,
                z,
                tries,
                ..
            } => self
                .bins
                .get(&(*kind, *level, *x, *z))
                .is_some_and(|state| state.tried >= tries.end),
            Job::Rings { kind } => self.rings.contains_key(kind),
            Job::Biome { .. } => false,
        }
    }

    fn store(&mut self, job: Job, reply: Reply) {
        match (job, reply) {
            (
                Job::Bins {
                    kind,
                    level,
                    x,
                    z,
                    tries,
                    ..
                },
                Reply::Found(found),
            ) => {
                if self.bins.len() >= MAX_BINS {
                    self.bins.clear();
                }
                let state = self.bins.entry((kind, level, x, z)).or_default();
                state.found.extend(found);
                state.tried = state.tried.max(tries.end);
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
                                tries: 0..0,
                            })
                            .collect::<Vec<_>>();
                        if !jobs.is_empty() {
                            return jobs;
                        }
                        // the shell is answered
                        for (dx, dz) in shell_blocks(search.shell) {
                            let key = (kind, 0, (center[0] + dx) * side, (center[1] + dz) * side);
                            search.results.extend(self.bins[&key].found.iter().map(|f| [f.x, f.z]));
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

    fn near(blocks_per_point: f64) -> MarkerSearch {
        // about as many as there are squares of 24 points in the views of these tests
        MarkerSearch {
            target: 625.0 / (blocks_per_point / 1000.0).powi(2).max(1.0),
            max_tries: 8192,
        }
    }

    struct Empty;

    impl Occupied for Empty {
        fn has_marker(&self, _: usize, _: [f64; 2], _: [f64; 2]) -> bool {
            false
        }
    }

    struct Full;

    impl Occupied for Full {
        fn has_marker(&self, _: usize, _: [f64; 2], _: [f64; 2]) -> bool {
            true
        }
    }

    /// Answers until nothing is wanted, and returns how many jobs that took.
    fn answer_all(queries: &mut Queries, kind: usize, occupied: &dyn Occupied) -> usize {
        let (min, max, zoom) = ([-300_000.0, -300_000.0], [300_000.0, 300_000.0], 1000.0);
        let mut seen = HashSet::new();
        for _ in 0..200_000 {
            let jobs = queries.visible(kind, min, max, near(zoom), occupied).jobs.concat();
            seen.extend(jobs.iter().cloned());
            queries.update(&jobs);
            if !queries.busy() {
                return seen.len();
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("still {} waiting", queries.waiting());
    }

    #[test]
    fn the_expected_number_rises_then_stays_at_the_target_without_jumps() {
        let expected = |cells: f64, rate: f64, target: f64| {
            let (level, share) = plan_bins(cells, rate, target, 8192);
            let candidates = 4f64.powi(level as i32);
            cells / candidates * share * (1.0 - (1.0 - rate).powf(candidates.min(8192.0)))
        };
        for (rate, target) in [(0.24, 1000.0), (0.012, 1000.0), (0.0003, 300.0), (1.0, 30000.0)] {
            let mut cells = 100.0;
            while cells < 1e11 {
                let number = expected(cells, rate, target);
                let everything = cells * rate;
                assert!(
                    (number - everything.min(target)).abs() < 1e-6 * target,
                    "{cells} cells of {rate}: {number}"
                );
                cells *= 1.1;
            }
        }
    }

    #[test]
    fn a_growing_share_only_adds_bins() {
        let chosen = |share| {
            (0..200i64)
                .flat_map(|x| (0..200i64).map(move |z| (x, z)))
                .filter(|&(x, z)| bin_is_chosen(3, 5, x, z, share))
                .collect::<HashSet<_>>()
        };
        let (few, more) = (chosen(0.3), chosen(0.6));
        assert!(few.is_subset(&more));
        let got = few.len() as f64 / 40000.0;
        assert!((0.27..0.33).contains(&got), "{got}");
    }

    #[test]
    fn later_rounds_only_look_where_there_is_space() {
        let world = FinderWorld {
            seed: 12345,
            dimension: Dimension::Overworld,
            sample_height: 256,
        };
        let kind = STRUCTURE_KINDS.iter().position(|kind| kind.id == "swamp_hut").unwrap();
        let bins = |round: usize, queries: &mut Queries, occupied: &dyn Occupied| {
            let (min, max) = ([-300_000.0, -300_000.0], [300_000.0, 300_000.0]);
            queries
                .visible(kind, min, max, near(1000.0), occupied)
                .jobs
                .get(round)
                .map_or(0, Vec::len)
        };
        let mut queries = Queries::new(world);
        let first_round = bins(0, &mut queries, &Empty);
        assert!(first_round > 100, "{first_round}");
        // with markers everywhere only the first round happens
        let full = answer_all(&mut queries, kind, &Full);
        assert_eq!(full, first_round);
        assert_eq!(
            queries
                .visible(kind, [-300_000.0; 2], [300_000.0; 2], near(1000.0), &Full)
                .jobs
                .concat()
                .len(),
            0
        );
        // where there is space it goes on, and every bin with something found stops
        let later = bins(1, &mut queries, &Empty);
        assert!(later > 0 && later < first_round, "{later} of {first_round}");
        let found = queries
            .visible(kind, [-300_000.0; 2], [300_000.0; 2], near(1000.0), &Empty)
            .known
            .len();
        assert_eq!(later + found, first_round);
    }

    #[test]
    fn messages_survive_encoding() {
        for job in [
            Job::Bins {
                kind: 3,
                level: 2,
                x: -4,
                z: 7,
                side: 16,
                tries: 16..128,
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
    fn nothing_is_waiting_once_everything_is_answered() {
        let mut queries = Queries::new(FinderWorld {
            seed: 12345,
            dimension: Dimension::Overworld,
            sample_height: 256,
        });
        let stronghold = STRUCTURE_KINDS.iter().position(|kind| kind.id == "stronghold").unwrap();
        let village = STRUCTURE_KINDS.iter().position(|kind| kind.id == "village").unwrap();
        let (min, max) = ([-1000.0, -1000.0], [1000.0, 1000.0]);
        for _ in 0..30_000 {
            let mut jobs = queries.visible(stronghold, min, max, near(16.0), &Empty).jobs;
            jobs.extend(queries.visible(village, min, max, near(16.0), &Empty).jobs);
            queries.update(&jobs.concat());
            if !queries.busy() {
                assert_eq!(queries.waiting(), 0);
                assert_eq!(
                    queries.visible(stronghold, min, max, near(16.0), &Empty).known.len(),
                    128
                );
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("still {} waiting", queries.waiting());
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
