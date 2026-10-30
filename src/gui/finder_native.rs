//! Runs finder jobs on threads.

use std::sync::{Arc, Mutex, mpsc};

use crate::gui::finder::{FinderWorld, Job, Reply, WorldQueries};

/// Placing structures recurses deeply, SteelMC's own tests need this much stack for it too.
const STACK_SIZE: usize = 16 << 20;

type Request = (FinderWorld, u64, Job);

pub struct Pool {
    tx: mpsc::Sender<Request>,
    rx: mpsc::Receiver<(u64, Job, Reply)>,
    threads: usize,
    running: usize,
}

impl Pool {
    pub fn new() -> Self {
        // half of the cores, the others are for the map
        let threads = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(2, 8));
        let (tx, requests) = mpsc::channel::<Request>();
        let (replies, rx) = mpsc::channel();
        let requests = Arc::new(Mutex::new(requests));
        let queries: Arc<Mutex<Option<Arc<WorldQueries>>>> = Default::default();
        for i in 0..threads {
            let (requests, replies, queries) = (requests.clone(), replies.clone(), queries.clone());
            std::thread::Builder::new()
                .name(format!("cubox-finder-{i}"))
                .stack_size(STACK_SIZE)
                .spawn(move || {
                    // ends when the pool is dropped
                    loop {
                        let Ok((world, generation, job)) = requests.lock().unwrap().recv() else {
                            return;
                        };
                        let current = {
                            let mut queries = queries.lock().unwrap();
                            match &*queries {
                                Some(current) if current.world == world => current.clone(),
                                _ => queries.insert(Arc::new(WorldQueries::new(world))).clone(),
                            }
                        };
                        let reply = current.run(&job);
                        if replies.send((generation, job, reply)).is_err() {
                            return;
                        }
                    }
                })
                .expect("creating a finder thread");
        }
        Self {
            tx,
            rx,
            threads,
            running: 0,
        }
    }

    /// Replies are taken once per frame, so threads have some jobs waiting not to run dry.
    pub fn has_idle(&self) -> bool {
        self.running < self.threads * 4
    }

    pub fn post(&mut self, world: FinderWorld, generation: u64, job: Job) {
        self.running += 1;
        // the threads only end when the pool is dropped
        let _ = self.tx.send((world, generation, job));
    }

    pub fn finished(&mut self) -> impl Iterator<Item = (u64, Job, Reply)> + '_ {
        let replies: Vec<_> = self.rx.try_iter().collect();
        self.running -= replies.len();
        replies.into_iter()
    }
}
