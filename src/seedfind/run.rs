//! Goes through seeds on threads until told to stop.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

use crate::seedfind::{
    engine::{Match, Plan, SeedWorld},
    spec::Card,
};

/// Placing structures recurses deeply, see `gui::finder_pool`.
const STACK_SIZE: usize = 16 << 20;
/// Seeds with the same lower bits that are tried, when those pass. Their structures can be at the
/// same spots, it's the biomes that differ, and more of them would only be more of the same.
const UPPER_BITS_TRIED: u64 = 1024;
/// How far apart spots are tried when searching one seed, in blocks.
const LOCATION_STEP: i32 = 512;
/// How far out spots are tried when searching one seed, in steps.
const LOCATION_SHELLS: i32 = 1024;

/// Where the seeds come from.
#[derive(Clone, Debug)]
pub enum Seeds {
    /// random ones, as if with this start. Uses that all structures depend on the lower 48 bits.
    Random(u64),
    /// counting up
    From(i64),
    List(Vec<i64>),
    /// not seeds but spots in one seed, the closest to the origin first
    Locations(i64),
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
    /// the highest seed that is sure to be done when counting up
    pub position: AtomicU64,
}

impl Counters {
    /// How many seeds are tried per match on average so far, if one was found.
    pub fn one_in(&self) -> Option<f64> {
        let matches = self.matches.load(Ordering::Relaxed);
        (matches > 0).then(|| self.seeds_per_match(matches as f64))
    }

    /// Seeds per match, for a lower limit too if nothing was found yet.
    fn seeds_per_match(&self, matches: f64) -> f64 {
        let bases = self.bases.load(Ordering::Relaxed).max(1) as f64;
        let passed = self.bases_passed.load(Ordering::Relaxed).max(1) as f64;
        let seeds = self.seeds.load(Ordering::Relaxed).max(1) as f64;
        (bases / passed) * (seeds / matches)
    }

    /// At least this many seeds per match, while there are none.
    pub fn at_least_one_in(&self) -> f64 {
        self.seeds_per_match(1.0)
    }

    /// All seeds this search has covered, including the ones it could skip.
    pub fn covered(&self) -> u64 {
        let bases = self.bases.load(Ordering::Relaxed);
        let passed = self.bases_passed.load(Ordering::Relaxed);
        let seeds = self.seeds.load(Ordering::Relaxed);
        bases.saturating_sub(passed) + seeds
    }
}

pub struct Search {
    pub counters: Arc<Counters>,
    pub results: mpsc::Receiver<Match>,
    stop: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    threads: Arc<AtomicUsize>,
}

impl Search {
    pub fn start(cards: &[Card], seeds: Seeds, threads: usize) -> Self {
        let plan = Arc::new(Plan::new(cards));
        let (tx, results) = mpsc::channel();
        let search = Self {
            counters: Default::default(),
            results,
            stop: Default::default(),
            pause: Default::default(),
            threads: Arc::new(AtomicUsize::new(threads)),
        };
        let next = Arc::new(AtomicU64::new(0));
        let location_world = match seeds {
            Seeds::Locations(seed) => Some(Arc::new(SeedWorld::new(seed))),
            _ => None,
        };
        let spots = Arc::new(match seeds {
            Seeds::Locations(_) => location_spots(),
            _ => Vec::new(),
        });
        let seeds = Arc::new(seeds);
        let workers = std::thread::available_parallelism()
            .map_or(threads, |n| n.get())
            .max(threads);
        for number in 0..workers {
            let worker = Worker {
                number,
                plan: plan.clone(),
                seeds: seeds.clone(),
                next: next.clone(),
                counters: search.counters.clone(),
                tx: tx.clone(),
                stop: search.stop.clone(),
                pause: search.pause.clone(),
                threads: search.threads.clone(),
                location_world: location_world.clone(),
                spots: spots.clone(),
            };
            std::thread::Builder::new()
                .name(format!("cubox-search-{number}"))
                .stack_size(STACK_SIZE)
                .spawn(move || worker.run())
                .expect("creating a search thread");
        }
        search
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn set_paused(&self, paused: bool) {
        self.pause.store(paused, Ordering::Relaxed);
    }

    pub fn set_threads(&self, threads: usize) {
        self.threads.store(threads, Ordering::Relaxed);
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        self.stop();
    }
}

fn location_spots() -> Vec<[i32; 2]> {
    let mut spots = vec![[0, 0]];
    for shell in 1..=LOCATION_SHELLS {
        for i in -shell..shell {
            spots.push([i, -shell]);
            spots.push([shell, i]);
            spots.push([-i, shell]);
            spots.push([-shell, -i]);
        }
    }
    spots
        .into_iter()
        .map(|[x, z]| [x * LOCATION_STEP, z * LOCATION_STEP])
        .collect()
}

/// A number that looks random, the same for the same input.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

struct Worker {
    number: usize,
    plan: Arc<Plan>,
    seeds: Arc<Seeds>,
    next: Arc<AtomicU64>,
    counters: Arc<Counters>,
    tx: mpsc::Sender<Match>,
    stop: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    threads: Arc<AtomicUsize>,
    location_world: Option<Arc<SeedWorld>>,
    spots: Arc<Vec<[i32; 2]>>,
}

impl Worker {
    fn run(self) {
        let mut cells = Vec::new();
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return;
            }
            if self.pause.load(Ordering::Relaxed) || self.number >= self.threads.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            let index = self.next.fetch_add(1, Ordering::Relaxed);
            let more = match &*self.seeds {
                Seeds::Random(start) => {
                    self.random_base(mix(start ^ index), &mut cells);
                    true
                }
                Seeds::From(start) => {
                    self.one_seed(start.wrapping_add(index as i64), &mut cells);
                    self.counters.position.store(index, Ordering::Relaxed);
                    true
                }
                Seeds::List(list) => match list.get(index as usize) {
                    Some(seed) => {
                        self.one_seed(*seed, &mut cells);
                        true
                    }
                    None => false,
                },
                Seeds::Locations(_) => match self.spots.get(index as usize) {
                    Some(spot) => {
                        self.spot(*spot, &mut cells);
                        true
                    }
                    None => false,
                },
            };
            if !more {
                return;
            }
        }
    }

    fn found(&self, found: Option<Match>) {
        if let Some(found) = found {
            self.counters.matches.fetch_add(1, Ordering::Relaxed);
            // only fails when the search is being dropped
            let _ = self.tx.send(found);
        }
    }

    fn check_seed(&self, seed: i64, cells: &mut Vec<([i32; 2], [i32; 2])>) {
        self.counters.seeds.fetch_add(1, Ordering::Relaxed);
        self.found(self.plan.check(&SeedWorld::new(seed), [0, 0], cells));
    }

    fn one_seed(&self, seed: i64, cells: &mut Vec<([i32; 2], [i32; 2])>) {
        self.counters.bases.fetch_add(1, Ordering::Relaxed);
        if self.plan.lower_bits_pass(seed, cells) {
            self.counters.bases_passed.fetch_add(1, Ordering::Relaxed);
            self.check_seed(seed, cells);
        }
    }

    fn random_base(&self, random: u64, cells: &mut Vec<([i32; 2], [i32; 2])>) {
        if !self.plan.has_positional() {
            return self.one_seed(random as i64, cells);
        }
        let base = random & ((1 << 48) - 1);
        self.counters.bases.fetch_add(1, Ordering::Relaxed);
        if !self.plan.lower_bits_pass(base as i64, cells) {
            return;
        }
        self.counters.bases_passed.fetch_add(1, Ordering::Relaxed);
        for try_number in 0..UPPER_BITS_TRIED {
            if self.stop.load(Ordering::Relaxed) {
                return;
            }
            let upper = mix(random ^ try_number) << 48;
            self.check_seed((base | upper) as i64, cells);
        }
    }

    fn spot(&self, spot: [i32; 2], cells: &mut Vec<([i32; 2], [i32; 2])>) {
        self.counters.bases.fetch_add(1, Ordering::Relaxed);
        self.counters.bases_passed.fetch_add(1, Ordering::Relaxed);
        self.counters.seeds.fetch_add(1, Ordering::Relaxed);
        if let Some(world) = &self.location_world {
            self.found(self.plan.check(world, spot, cells).map(|mut found| {
                // nothing found by the cards means the spot itself
                if found.hits.is_empty() {
                    found.hits.push(crate::seedfind::engine::Hit {
                        kind: None,
                        dimension: crate::dimension::Dimension::Overworld,
                        x: spot[0],
                        z: spot[1],
                    });
                }
                found
            }));
        }
    }
}
