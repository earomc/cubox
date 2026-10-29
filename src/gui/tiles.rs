use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};

use steel_worldgen::biomes::BiomeSourceKind;

use crate::{
    gui::map_renderer::{MapRenderer, TileTexture},
    tile_renderer::{Estimate, Lab, Shading, Tile, TileKey, calibrate, downsample, render_tile, world_mean},
};

/// Small tiles pop in quickly one by one and waste little work on parts outside the screen.
pub const TILE_SIZE: usize = 128;
/// Seconds a newly arrived texture takes to fade in over whatever was shown before.
const FADE_SECONDS: f64 = 0.15;
/// 128x128 tiles are 64 KiB as RGBA texture plus 96 KiB of estimates kept for downsampling,
/// so this is about 320 MiB.
const MAX_TILES: usize = 2048;
/// The coarsest level, see `MapView`.
const MAX_SCALE: u8 = 16;
/// How many levels down a tile may be built from cached tiles, through the levels in between.
const MAX_BUILD_DEPTH: u32 = 4;

type Estimates = Arc<Vec<Estimate>>;

/// Work for one tile. The variants are ordered by priority, cheap work first.
enum Task {
    /// average 4 finished children, no sampling needed
    Downsample([Estimates; 4]),
    /// sample the tile
    Render,
}

impl Task {
    fn phase(&self) -> u8 {
        match self {
            Task::Downsample(_) => 0,
            Task::Render => 1,
        }
    }
}

/// Queue shared with the workers.
#[derive(Default)]
struct Queue {
    tasks: HashMap<TileKey, Task>,
    /// visible tiles by priority, lower is more important. Tasks for tiles not in here aren't run,
    /// and running ones are abandoned.
    rank: HashMap<TileKey, usize>,
    generation: u64,
}

/// A level's shading, filled in by whoever needs it first.
type ShadingCell = OnceLock<Shading>;

/// The world the tiles are generated for.
struct World {
    source: Arc<BiomeSourceKind>,
    sample_height: i32,
    /// average color of the world, computed by the first worker that needs it
    mean: OnceLock<Lab>,
    /// shading per scale and samples setting, each calibrated by the first worker that needs it
    shadings: Mutex<HashMap<(u8, u8), Arc<ShadingCell>>>,
}

impl World {
    /// Creates the world and starts calibrating all levels in the background, for `samples` and
    /// for the one sample cover tiles. Otherwise the first tiles of each level visited would wait
    /// for its calibration, about 0.1s.
    fn new(source: Arc<BiomeSourceKind>, sample_height: i32, samples: u8) -> Arc<Self> {
        let world = Arc::new(Self {
            source,
            sample_height,
            mean: OnceLock::new(),
            shadings: Default::default(),
        });
        let weak = Arc::downgrade(&world);
        // a plain thread, not a rayon job: workers wait for these results and must not be needed for them
        std::thread::spawn(move || {
            for scale in 0..=MAX_SCALE {
                for samples in [samples, 1] {
                    // stop once the world was replaced, e.g. by another seed
                    let Some(world) = weak.upgrade() else { return };
                    let key = TileKey {
                        scale,
                        sample_height,
                        tile_size_px: TILE_SIZE,
                        samples,
                        x: 0,
                        z: 0,
                    };
                    world.shading(key);
                }
            }
        });
        world
    }

    fn mean(&self) -> Lab {
        *self.mean.get_or_init(|| world_mean(&self.source, self.sample_height))
    }

    fn shading(&self, key: TileKey) -> Shading {
        let cell = self
            .shadings
            .lock()
            .unwrap()
            .entry((key.scale, key.samples))
            .or_default()
            .clone();
        *cell.get_or_init(|| calibrate(&self.source, key, self.mean()))
    }
}

struct Shared {
    queue: Mutex<Queue>,
    /// rayon jobs that were spawned but haven't picked a task yet
    idle_workers: AtomicUsize,
    world: Mutex<Arc<World>>,
    tx: mpsc::Sender<Finished>,
}

struct Finished {
    generation: u64,
    key: TileKey,
    /// None if the tile was abandoned because it went off screen
    tile: Option<Tile>,
}

struct Entry {
    texture: Rc<TileTexture>,
    /// when the texture arrived, for fading it in
    arrived: f64,
    /// used to build the parent tile
    estimates: Estimates,
    last_used_frame: u64,
}

/// Caches tile textures and generates missing tiles on the rayon thread pool.
pub struct TileCache {
    shared: Arc<Shared>,
    entries: HashMap<TileKey, Entry>,
    /// tiles that have a task queued or running
    pending: HashSet<TileKey>,
    /// bumped on clear(), so results of tasks for an old seed get dropped
    generation: u64,
    frame: u64,
    rx: mpsc::Receiver<Finished>,
}

impl TileCache {
    pub fn new(source: Arc<BiomeSourceKind>, sample_height: i32, samples: u8) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            shared: Arc::new(Shared {
                queue: Default::default(),
                idle_workers: AtomicUsize::new(0),
                world: Mutex::new(World::new(source, sample_height, samples)),
                tx,
            }),
            entries: HashMap::new(),
            pending: HashSet::new(),
            generation: 0,
            frame: 0,
            rx,
        }
    }

    /// Drops all tiles, e.g. after the seed changed.
    pub fn clear(&mut self, source: Arc<BiomeSourceKind>, sample_height: i32, samples: u8) {
        self.generation += 1;
        *self.shared.world.lock().unwrap() = World::new(source, sample_height, samples);
        let mut queue = self.shared.queue.lock().unwrap();
        queue.tasks.clear();
        queue.generation = self.generation;
        self.entries.clear();
        self.pending.clear();
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Starts a new frame. `visible` are the tiles the map view wants to show, most important
    /// first, and `cover` quick coarse tiles to show until they are there, which go first.
    /// Queues what is missing of them, drops queued work for tiles that aren't wanted anymore and
    /// makes running work for them stop. Visible tiles are only sampled if `allow_sampling`, the
    /// map view holds that back while the zoom level is still changing.
    pub fn update(&mut self, visible: &[TileKey], cover: &[TileKey], allow_sampling: bool) {
        self.frame += 1;
        let mut rank = HashMap::new();
        for (i, &key) in cover.iter().enumerate() {
            self.plan(key, i, true, &mut rank);
        }
        for (i, &key) in visible.iter().enumerate() {
            self.plan(key, cover.len() + i, allow_sampling, &mut rank);
        }

        let mut queue = self.shared.queue.lock().unwrap();
        let Queue {
            tasks,
            rank: queue_rank,
            ..
        } = &mut *queue;
        *queue_rank = rank;
        tasks.retain(|key, _| {
            let keep = queue_rank.contains_key(key);
            if !keep {
                self.pending.remove(key);
            }
            keep
        });
        drop(queue);
        // tasks skipped by workers while their tile was off screen may be runnable again
        Shared::ensure_workers(&self.shared);
    }

    /// Queues what `key` needs, at priority `rank`: building it from its children if they are
    /// cached, otherwise building the missing children first if they can be built from cached
    /// tiles further down (right after zooming out a few levels at once), otherwise sampling it.
    fn plan(&mut self, key: TileKey, rank: usize, allow_sampling: bool, ranks: &mut HashMap<TileKey, usize>) {
        ranks.entry(key).or_insert(rank);
        if self.pending.contains(&key) || self.entries.contains_key(&key) {
            return;
        }
        if let Some(children) = self.finished_children(&key) {
            self.queue(key, Task::Downsample(children));
        } else if self.buildable(&key, MAX_BUILD_DEPTH) {
            for child in children(&key) {
                self.plan(child, rank, false, ranks);
            }
        } else if allow_sampling {
            self.queue(key, Task::Render);
        }
    }

    /// Whether the tile is cached or can be built from cached tiles up to `depth` levels down.
    fn buildable(&self, key: &TileKey, depth: u32) -> bool {
        self.entries.contains_key(key)
            || (depth > 0 && key.scale > 0 && children(key).iter().all(|child| self.buildable(child, depth - 1)))
    }

    fn queue(&mut self, key: TileKey, task: Task) {
        self.pending.insert(key);
        self.shared.queue.lock().unwrap().tasks.insert(key, task);
    }

    pub fn is_finished(&self, key: &TileKey) -> bool {
        self.entries.contains_key(key)
    }

    /// Returns the texture of a tile and its fade in opacity if it is cached, and marks it as used
    /// this frame. `now` is the egui time in seconds.
    pub fn get(&mut self, key: &TileKey, now: f64) -> Option<(Rc<TileTexture>, f32)> {
        let entry = self.entries.get_mut(key)?;
        entry.last_used_frame = self.frame;
        let alpha = ((now - entry.arrived) / FADE_SECONDS).clamp(0.0, 1.0) as f32;
        Some((entry.texture.clone(), alpha))
    }

    fn finished_children(&self, key: &TileKey) -> Option<[Estimates; 4]> {
        if key.scale == 0 {
            return None;
        }
        let [a, b, c, d] = children(key).map(|child| self.entries.get(&child).map(|e| e.estimates.clone()));
        Some([a?, b?, c?, d?])
    }

    /// Uploads finished tiles as textures. `now` is the egui time in seconds. Returns true if anything changed.
    pub fn poll(&mut self, now: f64, device: &wgpu::Device, queue: &wgpu::Queue, renderer: &MapRenderer) -> bool {
        let mut changed = false;
        while let Ok(done) = self.rx.try_recv() {
            if done.generation != self.generation {
                continue;
            }
            self.pending.remove(&done.key);
            let Some(tile) = done.tile else { continue };
            // nearest at the finest level so single quarts stay crisp when zoomed in
            let texture =
                Rc::new(renderer.create_tile(device, queue, &tile.rgb, TILE_SIZE as u32, done.key.scale == 0));
            let estimates = Arc::new(tile.estimates);
            self.entries.insert(
                done.key,
                Entry {
                    texture,
                    arrived: now,
                    estimates,
                    last_used_frame: self.frame,
                },
            );
            changed = true;
        }
        self.evict();
        changed
    }

    /// Removes the least recently used tiles if there are too many.
    fn evict(&mut self) {
        if self.entries.len() <= MAX_TILES {
            return;
        }
        let mut by_age: Vec<(u64, TileKey)> = self.entries.iter().map(|(k, e)| (e.last_used_frame, *k)).collect();
        by_age.sort_unstable_by_key(|(used, _)| *used);
        let excess = self.entries.len() - MAX_TILES;
        for (used, key) in by_age.into_iter().take(excess) {
            if used == self.frame {
                break; // never evict what's on screen right now
            }
            self.entries.remove(&key); // dropping the handle frees the texture
        }
    }
}

/// The 4 tiles one scale finer covering `key`: top left, top right, bottom left, bottom right.
fn children(key: &TileKey) -> [TileKey; 4] {
    [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dz)| TileKey {
        scale: key.scale - 1,
        x: key.x * 2 + dx,
        z: key.z * 2 + dz,
        ..*key
    })
}

impl Shared {
    /// Makes sure there is a spawned rayon job for every queued task.
    /// The jobs don't own a task, each one picks the most important task when it starts running.
    fn ensure_workers(this: &Arc<Self>) {
        let tasks = this.queue.lock().unwrap().tasks.len();
        while this.idle_workers.load(Ordering::Acquire) < tasks {
            this.idle_workers.fetch_add(1, Ordering::AcqRel);
            let shared = this.clone();
            rayon::spawn(move || shared.work());
        }
    }

    fn work(self: Arc<Self>) {
        let (key, task, generation) = {
            let mut queue = self.queue.lock().unwrap();
            self.idle_workers.fetch_sub(1, Ordering::AcqRel);
            let best = queue
                .tasks
                .iter()
                .filter_map(|(key, task)| Some((task.phase(), *queue.rank.get(key)?, *key)))
                .min_by_key(|&(phase, rank, _)| (phase, rank));
            let Some((_, _, key)) = best else { return };
            let task = queue.tasks.remove(&key).unwrap();
            (key, task, queue.generation)
        };
        // if the seed changed meanwhile this is the new world, but then the result is dropped anyway
        let world = self.world.lock().unwrap().clone();
        let shading = world.shading(key);

        let tile = match task {
            Task::Downsample(children) => Some(downsample(children.each_ref().map(|c| c.as_slice()), key, &shading)),
            Task::Render => {
                // stop as soon as the tile is off screen or the world changed
                let keep_going = || {
                    let queue = self.queue.lock().unwrap();
                    queue.generation == generation && queue.rank.contains_key(&key)
                };
                render_tile(&world.source, key, &shading, &keep_going)
            }
        };
        // the receiver only goes away when the gui is closing
        let _ = self.tx.send(Finished { generation, key, tile });
    }
}
