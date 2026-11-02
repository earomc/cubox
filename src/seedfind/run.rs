//! Goes through seeds, or what is to be found in an area of one seed, on threads.

use std::{
    collections::{BTreeMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use crate::{
    noise::NoiseCache,
    seedfind::{
        engine::{Cells, Hit, Match, Patch, PatchScan, Plan, SeedWorld, best_circle_with, row_runs},
        spec::{self, Condition, LocationSearch, Target},
    },
    structures::{Found, Spread, marker_margin, spread},
};

/// Placing structures recurses deeply, see `gui::finder_pool`.
const STACK_SIZE: usize = 16 << 20;
/// Seeds with the same lower bits that are tried, when those pass. Their structures can be at the
/// same spots, it's the biomes that differ, and more of them would only be more of the same.
const UPPER_BITS_TRIED: u64 = 1024;
/// Matches waiting to be looked at. Beyond that threads wait, a hidden window doesn't look at them
/// and they must not fill the memory.
const MAX_WAITING_MATCHES: usize = 4096;
/// Matches before there is an estimate of how rare they are.
const MATCHES_FOR_ODDS: u64 = 5;

/// Where the seeds come from.
#[derive(Clone, Debug)]
pub enum Seeds {
    /// random ones, as if with this start. Uses that all structures depend on the lower 48 bits.
    Random(u64),
    /// counting up
    From(i64),
    List(Vec<i64>),
}

/// The part of one seed that a search for locations goes through.
#[derive(Clone, Copy, Debug)]
pub struct Area {
    pub seed: i64,
    pub radius: i32,
    pub center: [i32; 2],
}

/// How a search is going.
#[derive(Default)]
pub struct Counters {
    /// lower 48 bits tried, and how many of them passed
    pub bases: AtomicU64,
    pub bases_passed: AtomicU64,
    /// seeds checked with a world
    pub seeds: AtomicU64,
    pub matches: AtomicU64,
    /// items of a list or parts of an area that are finished
    finished: AtomicU64,
}

impl Counters {
    /// How many seeds there are per match on average so far.
    pub fn one_in(&self) -> Option<f64> {
        let matches = self.matches.load(Ordering::Relaxed);
        let bases = self.bases.load(Ordering::Relaxed).max(1) as f64;
        let passed = self.bases_passed.load(Ordering::Relaxed).max(1) as f64;
        let seeds = self.seeds.load(Ordering::Relaxed).max(1) as f64;
        (matches > 0).then(|| (bases / passed) * (seeds / matches as f64))
    }

    /// All seeds this search has covered, including the ones it could skip.
    pub fn covered(&self) -> u64 {
        let bases = self.bases.load(Ordering::Relaxed);
        let passed = self.bases_passed.load(Ordering::Relaxed);
        let seeds = self.seeds.load(Ordering::Relaxed);
        bases.saturating_sub(passed) + seeds
    }
}

/// Two significant digits and separators: 12,000.
fn rounded(value: f64) -> String {
    let magnitude = 10f64.powi((value.max(1.0).log10().floor() as i32 - 1).max(0));
    let digits = ((value / magnitude).round() * magnitude) as u64;
    let text = digits.to_string();
    let mut out = String::new();
    for (i, digit) in text.chars().enumerate() {
        if i > 0 && (text.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn duration_text(seconds: f64) -> String {
    match seconds {
        s if s < 1.0 => "less than a second".to_owned(),
        s if s < 120.0 => format!("{s:.0} s"),
        s if s < 7200.0 => format!("{:.0} min", s / 60.0),
        s => format!("{:.0} h", s / 3600.0),
    }
}

/// What all threads of a search are told, and what they report to.
struct Control {
    stop: Arc<AtomicBool>,
    pause: AtomicBool,
    threads: AtomicUsize,
    counters: Arc<Counters>,
    tx: mpsc::SyncSender<Match>,
}

impl Control {
    /// Waits while this thread isn't to work. False if the search is over.
    fn may_work(&self, number: usize) -> bool {
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return false;
            }
            if !self.pause.load(Ordering::Relaxed) && number < self.threads.load(Ordering::Relaxed) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn send(&self, mut found: Match) {
        self.counters.matches.fetch_add(1, Ordering::Relaxed);
        loop {
            match self.tx.try_send(found) {
                Err(mpsc::TrySendError::Full(again)) if !self.stop.load(Ordering::Relaxed) => {
                    found = again;
                    std::thread::sleep(Duration::from_millis(20));
                }
                // the rest only happens when the search is being dropped
                _ => return,
            }
        }
    }
}

pub struct Search {
    pub counters: Arc<Counters>,
    pub results: mpsc::Receiver<Match>,
    control: Arc<Control>,
    /// how many parts or items there are, if it isn't endless
    total: Option<u64>,
    /// where a search for locations is around
    center: Option<[i32; 2]>,
    started: Instant,
    paused_since: Option<Instant>,
    paused_for: Duration,
}

/// How many threads to start. Only as many as allowed work, but that can change later.
fn thread_count(threads: usize) -> usize {
    std::thread::available_parallelism()
        .map_or(threads, |n| n.get())
        .max(threads)
}

fn spawn_thread(number: usize, work: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .name(format!("cubox-search-{number}"))
        .stack_size(STACK_SIZE)
        .spawn(work)
        .expect("creating a search thread");
}

impl Search {
    fn new(threads: usize, total: Option<u64>, center: Option<[i32; 2]>) -> Self {
        let (tx, results) = mpsc::sync_channel(MAX_WAITING_MATCHES);
        let counters: Arc<Counters> = Default::default();
        Self {
            counters: counters.clone(),
            results,
            control: Arc::new(Control {
                stop: Default::default(),
                pause: AtomicBool::new(false),
                threads: AtomicUsize::new(threads),
                counters,
                tx,
            }),
            total,
            center,
            started: Instant::now(),
            paused_since: None,
            paused_for: Duration::ZERO,
        }
    }

    /// Starts looking for seeds, or says what is wrong with the conditions. Biomes and noises are
    /// looked at every `step` blocks.
    pub fn seeds(conditions: &[Condition], seeds: Seeds, threads: usize, step: i32) -> Result<Self, String> {
        spec::validate_seeds(conditions)?;
        let total = match &seeds {
            Seeds::List(list) => Some(list.len() as u64),
            _ => None,
        };
        let search = Self::new(threads, total, None);
        let plan = Arc::new(Plan::new(conditions));
        let next = Arc::new(AtomicU64::new(0));
        let seeds = Arc::new(seeds);
        for number in 0..thread_count(threads) {
            let worker = SeedWorker {
                number,
                step,
                plan: plan.clone(),
                seeds: seeds.clone(),
                next: next.clone(),
                control: search.control.clone(),
            };
            spawn_thread(number, move || worker.run());
        }
        Ok(search)
    }

    /// Starts looking for locations, or says what is wrong with the search. Biomes and noises are
    /// looked at every `step` blocks.
    pub fn locations(search: &LocationSearch, area: Area, threads: usize, step: i32) -> Result<Self, String> {
        spec::validate_locations(search)?;
        let parts = Parts::of(&search.target, area, step);
        let this = Self::new(threads, Some(parts.total()), Some(area.center));
        let shared = Arc::new(LocationShared {
            plan: Plan::new(&search.conditions),
            target: search.target.clone(),
            world: SeedWorld::new(area.seed, step).stopped_by(this.control.stop.clone()),
            area,
            step,
            parts,
            total: this.total.unwrap_or(0),
            next: AtomicU64::new(0),
            control: this.control.clone(),
            clusters: Default::default(),
        });
        for number in 0..thread_count(threads) {
            let shared = shared.clone();
            spawn_thread(number, move || shared.work(number));
        }
        Ok(this)
    }

    pub fn stop(&self) {
        self.control.stop.store(true, Ordering::Relaxed);
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.control.pause.store(paused, Ordering::Relaxed);
        match (paused, self.paused_since) {
            (true, None) => self.paused_since = Some(Instant::now()),
            (false, Some(since)) => {
                self.paused_for += since.elapsed();
                self.paused_since = None;
            }
            _ => {}
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused_since.is_some()
    }

    pub fn set_threads(&self, threads: usize) {
        self.control.threads.store(threads, Ordering::Relaxed);
    }

    /// Whether all there is to do is done. An endless search never is.
    pub fn is_done(&self) -> bool {
        self.total
            .is_some_and(|total| self.counters.finished.load(Ordering::Relaxed) >= total)
    }

    /// Where a search for locations is around.
    pub fn center(&self) -> Option<[i32; 2]> {
        self.center
    }

    /// How long it has been working, without pauses.
    fn active_seconds(&self) -> f64 {
        let paused = self.paused_for + self.paused_since.map_or(Duration::ZERO, |since| since.elapsed());
        self.started.elapsed().saturating_sub(paused).as_secs_f64()
    }

    /// How it is going, in words.
    pub fn status(&self) -> String {
        let matches = self.counters.matches.load(Ordering::Relaxed);
        if self.center.is_some() {
            let plural = if matches == 1 { "" } else { "s" };
            if self.is_done() {
                return format!("Done: {matches} location{plural}");
            }
            let finished = self.counters.finished.load(Ordering::Relaxed) as f64;
            let percent = finished * 100.0 / self.total.unwrap_or(1).max(1) as f64;
            return format!("{percent:.1}% of the area, {matches} location{plural}");
        }
        let covered = self.counters.covered();
        let active = self.active_seconds().max(0.001);
        let seeds = format!(
            "{} seeds, {}/s",
            rounded(covered as f64),
            rounded(covered as f64 / active)
        );
        let odds = match (matches, self.counters.one_in()) {
            (0, _) => "no match yet".to_owned(),
            (matches, Some(one_in)) if matches >= MATCHES_FOR_ODDS => format!(
                "{matches} matches, about 1 in {}, one every {}",
                rounded(one_in),
                duration_text(active / matches as f64)
            ),
            (matches, _) => format!("{matches} match{}", if matches == 1 { "" } else { "es" }),
        };
        format!("{seeds}\n{odds}")
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A number that looks random, the same for the same input.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

struct SeedWorker {
    number: usize,
    step: i32,
    plan: Arc<Plan>,
    seeds: Arc<Seeds>,
    next: Arc<AtomicU64>,
    control: Arc<Control>,
}

impl SeedWorker {
    fn run(self) {
        let mut cells = Vec::new();
        while self.control.may_work(self.number) {
            let index = self.next.fetch_add(1, Ordering::Relaxed);
            match &*self.seeds {
                Seeds::Random(start) => self.random_base(mix(start ^ index), &mut cells),
                Seeds::From(start) => self.one_seed(start.wrapping_add(index as i64), &mut cells),
                Seeds::List(list) => {
                    let Some(seed) = list.get(index as usize) else { return };
                    self.one_seed(*seed, &mut cells);
                    self.control.counters.finished.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    fn check_seed(&self, seed: i64, cells: &mut Cells) {
        self.control.counters.seeds.fetch_add(1, Ordering::Relaxed);
        let world = SeedWorld::new(seed, self.step).stopped_by(self.control.stop.clone());
        if let Some(found) = self.plan.check(&world, [0, 0], cells) {
            self.control.send(found);
        }
    }

    fn one_seed(&self, seed: i64, cells: &mut Cells) {
        self.control.counters.bases.fetch_add(1, Ordering::Relaxed);
        if self.plan.lower_bits_pass(seed, cells) {
            self.control.counters.bases_passed.fetch_add(1, Ordering::Relaxed);
            self.check_seed(seed, cells);
        }
    }

    fn random_base(&self, random: u64, cells: &mut Cells) {
        if !self.plan.has_positional() {
            return self.one_seed(random as i64, cells);
        }
        let base = random & ((1 << 48) - 1);
        self.control.counters.bases.fetch_add(1, Ordering::Relaxed);
        if !self.plan.lower_bits_pass(base as i64, cells) {
            return;
        }
        self.control.counters.bases_passed.fetch_add(1, Ordering::Relaxed);
        for try_number in 0..UPPER_BITS_TRIED {
            if !self.control.may_work(self.number) {
                return;
            }
            let upper = mix(random ^ try_number) << 48;
            self.check_seed((base | upper) as i64, cells);
        }
    }
}

/// How the work of a search for locations is divided, so that threads can each take the next part.
enum Parts {
    /// the cells of a kind's grid, in rows
    Cells {
        kind: usize,
        first: [i32; 2],
        columns: u64,
        rows: u64,
    },
    /// all of a kind that isn't on a grid at once
    Rings { kind: usize },
    /// the rows of a grid of the area, which then are connected in order
    Rows { rows: u64, connect: Mutex<Connect> },
}

/// Rows that are done, waiting for those before them.
struct Connect {
    scan: PatchScan,
    next_row: u64,
    waiting: BTreeMap<u64, Vec<(i32, i32)>>,
}

impl Parts {
    fn of(target: &Target, area: Area, step: i32) -> Self {
        let kind = match target {
            Target::Poi { kind } | Target::Cluster { kind, .. } => *kind,
            Target::Biome { .. } | Target::Noise { .. } => {
                return Self::Rows {
                    rows: 2 * (area.radius / step) as u64 + 1,
                    connect: Mutex::new(Connect {
                        scan: PatchScan::new(area.center, step),
                        next_row: 0,
                        waiting: BTreeMap::new(),
                    }),
                };
            }
        };
        let Some(Spread::Grid(chunks)) = spread(kind) else {
            return Self::Rings { kind };
        };
        let size = chunks as i64 * 16;
        let reach = area.radius as i64 + marker_margin(kind) as i64;
        let cells = |middle: i32| {
            let (low, high) = (middle as i64 - reach, middle as i64 + reach);
            (
                low.div_euclid(size) as i32,
                (high.div_euclid(size) - low.div_euclid(size) + 1) as u64,
            )
        };
        let ((first_x, columns), (first_z, rows)) = (cells(area.center[0]), cells(area.center[1]));
        Self::Cells {
            kind,
            first: [first_x, first_z],
            columns,
            rows,
        }
    }

    fn total(&self) -> u64 {
        match self {
            Self::Cells { columns, rows, .. } => columns * rows,
            Self::Rings { .. } => 1,
            Self::Rows { rows, .. } => *rows,
        }
    }
}

/// What the threads of a search for locations share.
struct LocationShared {
    plan: Plan,
    target: Target,
    world: SeedWorld,
    area: Area,
    step: i32,
    parts: Parts,
    total: u64,
    next: AtomicU64,
    control: Arc<Control>,
    /// the groups that were reported, a group is found from each of its members
    clusters: Mutex<HashSet<Vec<[i32; 2]>>>,
}

impl LocationShared {
    fn work(&self, number: usize) {
        let mut cells = Vec::new();
        while self.control.may_work(number) {
            let index = self.next.fetch_add(1, Ordering::Relaxed);
            if index >= self.total {
                return;
            }
            match &self.parts {
                Parts::Cells {
                    kind, first, columns, ..
                } => {
                    let column = first[0] + (index % columns) as i32;
                    let row = first[1] + (index / columns) as i32;
                    let finder = self.world.finder(crate::structures::MARKER_KINDS[*kind].dimension);
                    if let Some(found) = finder.and_then(|finder| finder.in_world(*kind, column, row)) {
                        self.found(found, &mut cells);
                    }
                }
                Parts::Rings { kind } => {
                    let finder = self.world.finder(crate::structures::MARKER_KINDS[*kind].dimension);
                    for found in finder.map_or(Vec::new(), |finder| finder.rings(*kind)) {
                        self.found(found, &mut cells);
                    }
                }
                Parts::Rows { connect, .. } => {
                    let runs = self.row(index);
                    for patch in self.connect_row(connect, index, runs) {
                        self.found_patch(patch, &mut cells);
                    }
                }
            }
            self.control.counters.finished.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The runs of a row where the target is.
    fn row(&self, index: u64) -> Vec<(i32, i32)> {
        let dz = index as i32 - self.area.radius / self.step;
        let (center, radius, step) = (self.area.center, self.area.radius, self.step);
        let stop = &self.control.stop;
        match &self.target {
            Target::Biome { biome, .. } => {
                let dimension = self.target.dimension();
                let mut sampler = self.world.source(dimension).chunk_sampler();
                let qy = dimension.default_height() >> 2;
                row_runs(center, radius, step, dz, &mut |x, z| {
                    !stop.load(Ordering::Relaxed) && &*sampler.sample(x >> 2, qy, z >> 2).key.path == biome
                })
            }
            Target::Noise {
                parameter, min, max, ..
            } => {
                let Some(climate) = self.world.climate() else {
                    return Vec::new();
                };
                let mut cache = NoiseCache::default();
                let qy = self.target.dimension().default_height() >> 2;
                row_runs(center, radius, step, dz, &mut |x, z| {
                    let value = climate.value(&mut cache, *parameter, [x >> 2, qy, z >> 2]);
                    !stop.load(Ordering::Relaxed) && (*min..=*max).contains(&value)
                })
            }
            Target::Poi { .. } | Target::Cluster { .. } => Vec::new(),
        }
    }

    /// Hands in a row and connects all rows that are ready. Returns the patches this finishes.
    fn connect_row(&self, connect: &Mutex<Connect>, index: u64, runs: Vec<(i32, i32)>) -> Vec<Patch> {
        let mut connect = connect.lock().unwrap();
        connect.waiting.insert(index, runs);
        let mut finished = Vec::new();
        loop {
            let next_row = connect.next_row;
            let Some(runs) = connect.waiting.remove(&next_row) else {
                return finished;
            };
            let dz = next_row as i32 - self.area.radius / self.step;
            finished.extend(connect.scan.push_row(dz, &runs));
            connect.next_row += 1;
            if connect.next_row == self.total {
                finished.extend(connect.scan.finish());
            }
        }
    }

    fn in_area(&self, x: i32, z: i32) -> bool {
        let (dx, dz) = ((x - self.area.center[0]) as i64, (z - self.area.center[1]) as i64);
        dx * dx + dz * dz <= self.area.radius as i64 * self.area.radius as i64
    }

    fn found(&self, found: Found, cells: &mut Cells) {
        if !self.in_area(found.x, found.z) {
            return;
        }
        let hit = |found: &Found| Hit {
            kind: Some(found.kind),
            dimension: self.target.dimension(),
            x: found.x,
            z: found.z,
        };
        let Target::Cluster { count, radius, .. } = self.target else {
            return self.report([found.x, found.z], vec![hit(&found)], None, cells);
        };
        let Some(finder) = self.world.finder(self.target.dimension()) else {
            return;
        };
        // all that can share a circle with this one are at most twice the radius away
        let near = finder.around(found.kind, [found.x, found.z], radius * 2);
        let points: Vec<[i32; 2]> = near.iter().map(|near| [near.x, near.z]).collect();
        let (held, middle) = best_circle_with(&points, radius, Some([found.x, found.z]));
        if held < count as usize {
            return;
        }
        let in_circle = |near: &&Found| {
            let (dx, dz) = ((near.x - middle[0]) as i64, (near.z - middle[1]) as i64);
            dx * dx + dz * dz <= (radius as i64 + 1) * (radius as i64 + 1)
        };
        let members: Vec<&Found> = near.iter().filter(in_circle).collect();
        let mut group: Vec<[i32; 2]> = members.iter().map(|near| [near.x, near.z]).collect();
        group.sort_unstable();
        if self.clusters.lock().unwrap().insert(group) {
            let mut hits = vec![Hit {
                kind: None,
                dimension: self.target.dimension(),
                x: middle[0],
                z: middle[1],
            }];
            hits.extend(members.iter().map(|near| hit(near)));
            self.report(middle, hits, Some(held as f64), cells);
        }
    }

    fn found_patch(&self, patch: Patch, cells: &mut Cells) {
        let (Target::Biome { min_area, .. } | Target::Noise { min_area, .. }) = &self.target else {
            return;
        };
        let area = patch.area(self.step);
        if min_area.is_some_and(|min_area| area < min_area) {
            return;
        }
        let [x, z] = patch.middle();
        let hit = Hit {
            kind: None,
            dimension: self.target.dimension(),
            x,
            z,
        };
        self.report([x, z], vec![hit], Some(area as f64), cells);
    }

    /// Reports what was found if the conditions around it pass. It comes first in the hits.
    fn report(&self, at: [i32; 2], mut hits: Vec<Hit>, score: Option<f64>, cells: &mut Cells) {
        if let Some(mut found) = self.plan.check(&self.world, at, cells) {
            hits.append(&mut found.hits);
            found.hits = hits;
            found.score = score.or(found.score);
            self.control.send(found);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dimension::Dimension, structures::kind_index};

    /// A coarser grid than the default keeps these tests quick.
    const STEP: i32 = 16;

    fn run_to_end(text: &str, seed: i64, radius: i32, step: i32) -> (Vec<Match>, String) {
        let locations = spec::parse_locations(text).unwrap();
        let area = Area {
            seed,
            radius,
            center: [0, 0],
        };
        let search = Search::locations(&locations, area, 4, step).unwrap();
        let start = Instant::now();
        let mut found = Vec::new();
        while !search.is_done() {
            found.extend(search.results.try_iter());
            assert!(start.elapsed().as_secs() < 200, "{}", search.status());
            std::thread::sleep(Duration::from_millis(10));
        }
        // what a worker has yet to send comes right after it says it is done
        std::thread::sleep(Duration::from_millis(50));
        found.extend(search.results.try_iter());
        (found, search.status())
    }

    fn finder() -> crate::structures::Finder {
        crate::structures::Finder::new(12345, Dimension::Overworld).unwrap()
    }

    fn firsts(found: &[Match]) -> Vec<(i32, i32)> {
        let mut firsts: Vec<_> = found.iter().map(|f| (f.hits[0].x, f.hits[0].z)).collect();
        firsts.sort_unstable();
        firsts
    }

    #[test]
    fn each_poi_is_one_result_and_none_are_missing() {
        for (id, radius) in [
            ("village", 6000),
            ("swamp_hut", 12_000),
            ("geode", 400),
            ("copper_vein", 3000),
        ] {
            let (found, status) = run_to_end(&format!("poi {id}"), 12345, radius, STEP);
            assert!(status.starts_with("Done"), "{status}");
            let mut real: Vec<_> = finder()
                .around(kind_index(id), [0, 0], radius)
                .iter()
                .map(|f| (f.x, f.z))
                .collect();
            real.sort_unstable();
            assert!(!real.is_empty(), "{id}");
            assert_eq!(firsts(&found), real, "{id}");
        }
    }

    #[test]
    fn rings_are_results_too() {
        let (found, _) = run_to_end("poi stronghold", 12345, 10_000, STEP);
        let mut real: Vec<_> = finder()
            .around(kind_index("stronghold"), [0, 0], 10_000)
            .iter()
            .map(|f| (f.x, f.z))
            .collect();
        real.sort_unstable();
        assert!(!real.is_empty());
        assert_eq!(firsts(&found), real);
    }

    #[test]
    fn conditions_are_about_what_was_found() {
        let (all, _) = run_to_end("poi village", 12345, 5000, STEP);
        let (near, _) = run_to_end("poi village\nbiome plains within 200", 12345, 5000, STEP);
        let (not_near, _) = run_to_end("poi village\nnot biome plains within 200", 12345, 5000, STEP);
        assert!(!near.is_empty() && !not_near.is_empty());
        assert_eq!(near.len() + not_near.len(), all.len());
        let mut together = firsts(&near);
        together.extend(firsts(&not_near));
        together.sort_unstable();
        assert_eq!(together, firsts(&all));
    }

    /// Every structure with another close enough to share a circle is in a reported group.
    #[test]
    fn clusters_are_all_reported_once() {
        let radius = 150;
        let (found, _) = run_to_end(
            &format!("cluster village count>=2 radius {radius}"),
            12345,
            12_000,
            STEP,
        );
        let reported: HashSet<(i32, i32)> = found
            .iter()
            .flat_map(|f| f.hits.iter().filter(|hit| hit.kind.is_some()).map(|hit| (hit.x, hit.z)))
            .collect();
        let real = finder().around(kind_index("village"), [0, 0], 12_000);
        let mut paired = 0;
        for a in &real {
            let close = real.iter().any(|b| {
                let (dx, dz) = ((a.x - b.x) as i64, (a.z - b.z) as i64);
                a != b && dx * dx + dz * dz <= (2 * radius as i64).pow(2)
            });
            if close {
                paired += 1;
                assert!(reported.contains(&(a.x, a.z)), "{a:?}");
            }
        }
        assert!(paired > 0);
        let groups: HashSet<Vec<(i32, i32)>> = found
            .iter()
            .map(|f| {
                let mut group: Vec<_> = f.hits.iter().filter(|h| h.kind.is_some()).map(|h| (h.x, h.z)).collect();
                group.sort_unstable();
                group
            })
            .collect();
        assert_eq!(groups.len(), found.len());
    }

    /// A slow flood fill of the same grid, in one go.
    fn flood_fill(biome: &str, seed: i64, radius: i32, step: i32) -> Vec<(i64, [i32; 2])> {
        let source = Dimension::Overworld.biome_source(seed);
        let mut sampler = source.chunk_sampler();
        let n = radius / step;
        let mut inside = HashSet::new();
        for dz in -n..=n {
            for dx in crate::seedfind::engine::disc_row(radius, step, dz) {
                if &*sampler.sample((dx * step) >> 2, 64, (dz * step) >> 2).key.path == biome {
                    inside.insert([dx, dz]);
                }
            }
        }
        let mut seen = HashSet::new();
        let mut patches = Vec::new();
        for start in &inside {
            if !seen.insert(*start) {
                continue;
            }
            let (mut stack, mut points, mut sum) = (vec![*start], 0i64, [0i64; 2]);
            while let Some([x, z]) = stack.pop() {
                points += 1;
                sum[0] += (x * step) as i64;
                sum[1] += (z * step) as i64;
                for next in [[x + 1, z], [x - 1, z], [x, z + 1], [x, z - 1]] {
                    if inside.contains(&next) && seen.insert(next) {
                        stack.push(next);
                    }
                }
            }
            let middle = sum.map(|sum| (sum / points) as i32);
            patches.push((points * (step * step) as i64, middle));
        }
        patches.sort_unstable();
        patches
    }

    #[test]
    fn each_patch_of_a_biome_is_one_result() {
        let (found, status) = run_to_end("biome plains", 12345, 3000, STEP);
        assert!(status.starts_with("Done"), "{status}");
        let mut patches: Vec<(i64, [i32; 2])> = found
            .iter()
            .map(|f| (f.score.unwrap() as i64, [f.hits[0].x, f.hits[0].z]))
            .collect();
        patches.sort_unstable();
        let wanted = flood_fill("plains", 12345, 3000, STEP);
        assert!(wanted.len() > 1, "{wanted:?}");
        assert_eq!(patches, wanted);
    }

    #[test]
    fn a_minimum_area_leaves_out_small_patches() {
        let (all, _) = run_to_end("biome plains", 12345, 3000, STEP);
        let sizes: Vec<i64> = all.iter().map(|f| f.score.unwrap() as i64).collect();
        let limit = sizes.iter().sum::<i64>() / sizes.len() as i64;
        let (big, _) = run_to_end(&format!("biome plains area>={limit}"), 12345, 3000, STEP);
        assert_eq!(big.len(), sizes.iter().filter(|size| **size >= limit).count());
    }

    /// The grids agree on how much there is, and only the exact one sees what is smaller than the
    /// other's step.
    #[test]
    fn the_exact_grid_sees_patches_a_coarse_one_cannot() {
        let (exact, _) = run_to_end("biome river", 12345, 1500, spec::EXACT_STEP);
        let (coarse, _) = run_to_end("biome river", 12345, 1500, STEP);
        let total = |found: &[Match]| found.iter().map(|f| f.score.unwrap() as i64).sum::<i64>();
        let difference = (total(&exact) - total(&coarse)).abs() as f64 / total(&exact) as f64;
        assert!(difference < 0.1, "{} {}", total(&exact), total(&coarse));
        let smallest = |found: &[Match]| found.iter().map(|f| f.score.unwrap() as i64).min().unwrap();
        assert!(smallest(&coarse) >= (STEP * STEP) as i64);
        assert!(smallest(&exact) < (STEP * STEP) as i64, "{}", smallest(&exact));
    }

    #[test]
    fn noise_regions_are_results() {
        let (found, status) = run_to_end("noise erosion 0.2..0.4\nbiome plains within 2000", 12345, 3000, STEP);
        assert!(status.starts_with("Done"), "{status}");
        assert!(!found.is_empty());
        assert!(found.iter().all(|f| f.score.is_some()));
    }

    #[test]
    fn seeds_of_a_list_are_each_checked() {
        let conditions = spec::parse_seeds("poi village within 3000 of spawn").unwrap();
        let search = Search::seeds(&conditions, Seeds::List(vec![1, 2, 3]), 2, STEP).unwrap();
        let start = Instant::now();
        while !search.is_done() {
            assert!(start.elapsed().as_secs() < 60);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            search.results.try_iter().map(|f| f.seed).collect::<HashSet<_>>().len(),
            3
        );
    }

    #[test]
    fn stopping_ends_a_long_check() {
        let conditions = spec::parse_seeds("biome mushroom_fields within 2000000 of 0,0").unwrap();
        let search = Search::seeds(&conditions, Seeds::From(1), 2, spec::EXACT_STEP).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let control = search.control.clone();
        search.stop();
        let start = Instant::now();
        // the threads hold the other references, and let go when they end
        while Arc::strong_count(&control) > 2 {
            assert!(start.elapsed().as_secs() < 20, "{}", Arc::strong_count(&control));
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn numbers_are_rounded_for_reading() {
        assert_eq!(rounded(1234.0), "1,200");
        assert_eq!(rounded(7.0), "7");
        assert_eq!(rounded(1_560_000.0), "1,600,000");
        assert_eq!(duration_text(90.0), "90 s");
        assert_eq!(duration_text(600.0), "10 min");
    }
}
