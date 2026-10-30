//! Runs tile tasks on threads.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};

use crate::{
    gui::{
        tiles::TILE_SIZE,
        world::{Finished, Queue, Task, World, WorldParams},
    },
    tile_renderer::TileKey,
};

/// The coarsest level, see `MapView`.
const MAX_SCALE: u8 = 16;

pub struct Executor {
    shared: Arc<Shared>,
    rx: mpsc::Receiver<Finished>,
}

struct Shared {
    queue: Mutex<Queue>,
    /// The tile workers run in their own pool, not rayon's global one. While a world's mean and
    /// shadings are computed, workers block waiting for them. The first biome sample builds
    /// SteelMC's biome lookup tree, in parallel on the global pool: if the workers were the global
    /// pool, nothing could run that and everything would wait forever.
    pool: rayon::ThreadPool,
    /// jobs that were spawned but haven't picked a task yet
    idle_workers: AtomicUsize,
    world: Mutex<Arc<World>>,
    tx: mpsc::Sender<Finished>,
}

impl Executor {
    pub fn new(params: WorldParams) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            shared: Arc::new(Shared {
                queue: Default::default(),
                pool: rayon::ThreadPoolBuilder::new()
                    .thread_name(|i| format!("cubox-tiles-{i}"))
                    .build()
                    .expect("creating the tile worker threads"),
                idle_workers: AtomicUsize::new(0),
                world: Mutex::new(new_world(params)),
                tx,
            }),
            rx,
        }
    }

    /// Switches to another world. Tasks of the old one are dropped, results of running ones are
    /// sent with the old generation.
    pub fn set_world(&mut self, params: WorldParams, generation: u64) {
        *self.shared.world.lock().unwrap() = new_world(params);
        let mut queue = self.shared.queue.lock().unwrap();
        queue.tasks.clear();
        queue.generation = generation;
    }

    pub fn queue(&mut self, key: TileKey, task: Task) {
        self.shared.queue.lock().unwrap().tasks.insert(key, task);
    }

    /// Sets the wanted tiles (see `Queue::set_rank`) and starts work on the queued tasks.
    pub fn update(&mut self, rank: HashMap<TileKey, usize>, pending: &mut HashSet<TileKey>) {
        self.shared.queue.lock().unwrap().set_rank(rank, pending);
        // tasks skipped by workers while their tile was off screen may be runnable again
        Shared::ensure_workers(&self.shared);
    }

    pub fn finished(&mut self) -> impl Iterator<Item = Finished> + '_ {
        self.rx.try_iter()
    }
}

/// Creates the world and starts calibrating all levels in the background, for the samples setting
/// and for the one sample cover tiles. Otherwise the first tiles of each level visited would wait
/// for its calibration, about 0.1s.
fn new_world(params: WorldParams) -> Arc<World> {
    let world = Arc::new(World::new(params));
    let weak = Arc::downgrade(&world);
    // a plain thread, not a rayon job: workers wait for these results and must not be needed for them
    std::thread::spawn(move || {
        for scale in 0..=MAX_SCALE {
            // the setting and the one sample cover tiles, once if they are the same
            for samples in if params.samples == 1 {
                vec![1]
            } else {
                vec![params.samples, 1]
            } {
                // stop once the world was replaced, e.g. by another seed
                let Some(world) = weak.upgrade() else { return };
                world.shading(TileKey {
                    scale,
                    sample_height: params.sample_height,
                    tile_size_px: TILE_SIZE,
                    samples,
                    x: 0,
                    z: 0,
                });
            }
        }
    });
    world
}

impl Shared {
    /// Makes sure there is a spawned job for every queued task.
    /// The jobs don't own a task, each one picks the most important task when it starts running.
    fn ensure_workers(this: &Arc<Self>) {
        let tasks = this.queue.lock().unwrap().tasks.len();
        while this.idle_workers.load(Ordering::Acquire) < tasks {
            this.idle_workers.fetch_add(1, Ordering::AcqRel);
            let shared = this.clone();
            this.pool.spawn(move || shared.work());
        }
    }

    fn work(self: Arc<Self>) {
        let taken = {
            let mut queue = self.queue.lock().unwrap();
            self.idle_workers.fetch_sub(1, Ordering::AcqRel);
            queue.take_best()
        };
        let Some((key, task, generation)) = taken else { return };
        // if the seed changed meanwhile this is the new world, but then the result is dropped anyway
        let world = self.world.lock().unwrap().clone();
        // stop as soon as the tile is off screen or the world changed
        let keep_going = || {
            let queue = self.queue.lock().unwrap();
            queue.generation == generation && queue.rank.contains_key(&key)
        };
        let tile = world.run(key, task, &keep_going);
        // the receiver only goes away when the gui is closing
        let _ = self.tx.send(Finished { generation, key, tile });
    }
}
