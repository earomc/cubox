//! Goes through seeds, or the spots of an area of one seed, on threads.

use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use crate::seedfind::{
    engine::{Cells, Hit, Match, Plan, SeedWorld, spots},
    spec::{self, Card},
};

/// Placing structures recurses deeply, see `gui::finder_pool`.
const STACK_SIZE: usize = 16 << 20;
/// Seeds with the same lower bits that are tried, when those pass. Their structures can be at the
/// same spots, it's the biomes that differ, and more of them would only be more of the same.
const UPPER_BITS_TRIED: u64 = 1024;
/// Matches waiting to be looked at. Beyond that threads wait, a hidden window doesn't look at them
/// and they must not fill the memory.
const MAX_WAITING_MATCHES: usize = 4096;
/// Spots of an area tried at most.
const MAX_SPOTS: i64 = 4_000_000;
/// Places closer than this are the same one, if they aren't structures, in blocks.
const SAME_PLACE: i32 = 256;
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
    /// not seeds but the spots of the `area` card in one seed
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
    /// items of a list or spots of an area that are finished
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

pub struct Search {
    pub counters: Arc<Counters>,
    pub results: mpsc::Receiver<Match>,
    stop: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    threads: Arc<AtomicUsize>,
    /// how many spots or items there are, if it isn't endless
    total: Option<u64>,
    /// where the spots are around, in one seed
    center: Option<[i32; 2]>,
    started: Instant,
    paused_since: Option<Instant>,
    paused_for: Duration,
}

impl Search {
    /// Starts it, or says what is wrong with the cards for these seeds.
    pub fn start(cards: &[Card], seeds: Seeds, threads: usize) -> Result<Self, String> {
        let in_seed = match seeds {
            Seeds::Locations(seed) => Some(seed),
            _ => None,
        };
        spec::validate(cards, in_seed.is_some())?;
        let location_world = in_seed.map(|seed| Arc::new(SeedWorld::new(seed)));
        let (center, spots) = match &location_world {
            Some(world) => {
                let (center, spots) = area_spots(cards, world)?;
                (Some(center), spots)
            }
            None => (None, Vec::new()),
        };
        let total = match &seeds {
            Seeds::Locations(_) => Some(spots.len() as u64),
            Seeds::List(list) => Some(list.len() as u64),
            _ => None,
        };
        let plan = Arc::new(Plan::new(cards));
        let (tx, results) = mpsc::sync_channel(MAX_WAITING_MATCHES);
        let search = Self {
            counters: Default::default(),
            results,
            stop: Default::default(),
            pause: Default::default(),
            threads: Arc::new(AtomicUsize::new(threads)),
            total,
            center,
            started: Instant::now(),
            paused_since: None,
            paused_for: Duration::ZERO,
        };
        let next = Arc::new(AtomicU64::new(0));
        let spots = Arc::new(spots);
        let seen = Arc::new(Mutex::new(HashSet::new()));
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
                seen: seen.clone(),
            };
            std::thread::Builder::new()
                .name(format!("cubox-search-{number}"))
                .stack_size(STACK_SIZE)
                .spawn(move || worker.run())
                .expect("creating a search thread");
        }
        Ok(search)
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.pause.store(paused, Ordering::Relaxed);
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
        self.threads.store(threads, Ordering::Relaxed);
    }

    /// Whether all there is to do is done. An endless search never is.
    pub fn is_done(&self) -> bool {
        self.total
            .is_some_and(|total| self.counters.finished.load(Ordering::Relaxed) >= total)
    }

    /// Where the spots of one seed are around.
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
            let finished = self.counters.finished.load(Ordering::Relaxed);
            let total = self.total.unwrap_or(0);
            return if self.is_done() {
                format!("Done: {matches} location{plural} in {total} spots")
            } else {
                format!("{finished} of {total} spots, {matches} location{plural}")
            };
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

/// The middle of the area and the spots to try, nearest first. Conditions don't depend on the spot
/// without `here`, so one is enough then. The grid is as fine as the closest `here` needs: all
/// that is within a card's radius of some spot is then within it of a spot of the grid.
fn area_spots(cards: &[Card], world: &SeedWorld) -> Result<([i32; 2], Vec<[i32; 2]>), String> {
    let (center, radius) = cards
        .iter()
        .find_map(|card| match card {
            Card::Area { radius, anchor } => Some((world.center(*anchor, [0, 0]), *radius)),
            _ => None,
        })
        .unwrap_or(([0, 0], 0));
    let step = cards.iter().filter_map(spec::smallest_here).min();
    let Some(step) = step else {
        return Ok((center, vec![center]));
    };
    let step = step.max(16);
    let across = 2 * (radius / step) as i64 + 1;
    if across * across > MAX_SPOTS {
        return Err(format!(
            "that would be {} spots to try: make the area smaller or `within ... of here` larger",
            rounded((across * across) as f64)
        ));
    }
    Ok((center, spots(center, radius, step)))
}

/// A number that looks random, the same for the same input.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// What makes a hit the same place as another.
type Place = (Option<usize>, i32, i32);

fn place_of(hit: &Hit) -> Place {
    match hit.kind {
        Some(_) => (hit.kind, hit.x, hit.z),
        None => (None, hit.x.div_euclid(SAME_PLACE), hit.z.div_euclid(SAME_PLACE)),
    }
}

/// Keeps a location if it is at a place that wasn't reported yet, which then comes first. Its
/// structures are what makes it that place if it has any: the same village with another part of a
/// biome next to it is the same location.
fn new_location(found: &mut Match, seen: &Mutex<HashSet<Place>>) -> bool {
    let has_structures = found.hits.iter().any(|hit| hit.kind.is_some());
    let counts = |hit: &Hit| hit.kind.is_some() || !has_structures;
    let mut seen = seen.lock().unwrap();
    let new = found
        .hits
        .iter()
        .position(|hit| counts(hit) && !seen.contains(&place_of(hit)));
    seen.extend(found.hits.iter().filter(|hit| counts(hit)).map(place_of));
    match new {
        Some(index) => {
            found.hits.swap(0, index);
            true
        }
        None => false,
    }
}

struct Worker {
    number: usize,
    plan: Arc<Plan>,
    seeds: Arc<Seeds>,
    next: Arc<AtomicU64>,
    counters: Arc<Counters>,
    tx: mpsc::SyncSender<Match>,
    stop: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    threads: Arc<AtomicUsize>,
    location_world: Option<Arc<SeedWorld>>,
    spots: Arc<Vec<[i32; 2]>>,
    seen: Arc<Mutex<HashSet<Place>>>,
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
                        self.counters.finished.fetch_add(1, Ordering::Relaxed);
                        true
                    }
                    None => false,
                },
                Seeds::Locations(_) => match self.spots.get(index as usize) {
                    Some(spot) => {
                        self.spot(*spot, &mut cells);
                        self.counters.finished.fetch_add(1, Ordering::Relaxed);
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
        let Some(mut found) = found else { return };
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

    fn check_seed(&self, seed: i64, cells: &mut Cells) {
        self.counters.seeds.fetch_add(1, Ordering::Relaxed);
        self.found(self.plan.check(&SeedWorld::new(seed), [0, 0], cells));
    }

    fn one_seed(&self, seed: i64, cells: &mut Cells) {
        self.counters.bases.fetch_add(1, Ordering::Relaxed);
        if self.plan.lower_bits_pass(seed, cells) {
            self.counters.bases_passed.fetch_add(1, Ordering::Relaxed);
            self.check_seed(seed, cells);
        }
    }

    fn random_base(&self, random: u64, cells: &mut Cells) {
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

    fn spot(&self, spot: [i32; 2], cells: &mut Cells) {
        let Some(world) = &self.location_world else { return };
        let Some(mut found) = self.plan.check(world, spot, cells) else {
            return;
        };
        // nothing found by the cards means the spot itself
        if found.hits.is_empty() {
            found.hits.push(Hit {
                kind: None,
                dimension: crate::dimension::Dimension::Overworld,
                x: spot[0],
                z: spot[1],
            });
        }
        if new_location(&mut found, &self.seen) {
            self.found(Some(found));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_to_end(text: &str, seed: i64) -> (Vec<Match>, String) {
        let cards = spec::parse(text).unwrap();
        let search = Search::start(&cards, Seeds::Locations(seed), 4).unwrap();
        let start = Instant::now();
        let mut found = Vec::new();
        while !search.is_done() {
            found.extend(search.results.try_iter());
            assert!(start.elapsed().as_secs() < 120, "{}", search.status());
            std::thread::sleep(Duration::from_millis(10));
        }
        // what a worker has yet to send comes right after it says it is done
        std::thread::sleep(Duration::from_millis(50));
        found.extend(search.results.try_iter());
        (found, search.status())
    }

    #[test]
    fn an_area_is_gone_through_once_without_repeats() {
        let (found, status) = run_to_end(
            "area within 4000 of 0,0\nstructure village count>=1 within 400 of here",
            12345,
        );
        assert!(status.starts_with("Done"), "{status}");
        assert!(found.len() > 1, "{found:?}");
        let firsts: Vec<_> = found.iter().map(|f| (f.hits[0].x, f.hits[0].z)).collect();
        let unique: HashSet<_> = firsts.iter().collect();
        assert_eq!(unique.len(), firsts.len(), "{firsts:?}");
        // and it found them all
        let finder = crate::structures::Finder::new(12345, crate::dimension::Dimension::Overworld).unwrap();
        let village = crate::structures::STRUCTURE_KINDS
            .iter()
            .position(|k| k.id == "village")
            .unwrap();
        let reported: HashSet<_> = found.iter().flat_map(|f| f.hits.iter().map(|h| (h.x, h.z))).collect();
        for real in finder.around(village, [0, 0], 4000) {
            assert!(reported.contains(&(real.x, real.z)), "{real:?}");
        }
    }

    #[test]
    fn a_structure_next_to_a_biome_is_one_location() {
        let (found, _) = run_to_end(
            "area within 3000 of 0,0\nstructure village count>=1 within 300 of here\nbiome plains present within 600 of here",
            12345,
        );
        let finder = crate::structures::Finder::new(12345, crate::dimension::Dimension::Overworld).unwrap();
        let village = crate::structures::STRUCTURE_KINDS
            .iter()
            .position(|k| k.id == "village")
            .unwrap();
        let real = finder.around(village, [0, 0], 3300).len();
        assert!(!found.is_empty() && found.len() <= real, "{} of {real}", found.len());
        assert!(found.iter().all(|f| f.hits[0].kind == Some(village)));
    }

    #[test]
    fn conditions_without_here_are_checked_once() {
        let (found, status) = run_to_end(
            "area within 90000 of 0,0\nstructure village count>=1 within 3000 of 0,0",
            12345,
        );
        assert_eq!(found.len(), 1);
        assert!(status.contains("in 1 spots"), "{status}");
    }

    #[test]
    fn too_big_areas_are_refused() {
        let cards = spec::parse("area within 30000000 of 0,0\nbiome desert present within 16 of here").unwrap();
        assert!(
            Search::start(&cards, Seeds::Locations(1), 1)
                .err()
                .unwrap()
                .contains("spots")
        );
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
