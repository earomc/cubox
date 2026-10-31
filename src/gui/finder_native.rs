//! Runs finder jobs on threads.

use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

use crate::gui::{
    finder::{FinderWorld, Job, Reply, WorldQueries},
    resources,
};

/// Placing structures recurses deeply, SteelMC's own tests need this much stack for it too.
const STACK_SIZE: usize = 16 << 20;
/// Replies are taken once per frame, so threads have this many jobs waiting each not to run dry.
const JOBS_PER_THREAD: usize = 32;

type Request = (FinderWorld, u64, Job);

pub struct Pool {
    tx: mpsc::Sender<Request>,
    rx: mpsc::Receiver<(u64, Job, Reply)>,
    limit: Arc<Limit>,
    running: usize,
}

/// How many threads may work at once. All of them exist from the start and wait here.
struct Limit {
    threads: AtomicUsize,
    working: Mutex<usize>,
    changed: Condvar,
}

impl Limit {
    fn enter(&self) {
        let mut working = self.working.lock().unwrap();
        while *working >= self.threads.load(Ordering::Relaxed) {
            // also to notice the limit went up, which doesn't happen often
            working = self.changed.wait_timeout(working, Duration::from_millis(50)).unwrap().0;
        }
        *working += 1;
    }

    fn leave(&self) {
        *self.working.lock().unwrap() -= 1;
        self.changed.notify_one();
    }
}

impl Pool {
    pub fn new() -> Self {
        let threads = resources::max_threads();
        let limit = Arc::new(Limit {
            threads: AtomicUsize::new(threads),
            working: Mutex::new(0),
            changed: Condvar::new(),
        });
        let (tx, requests) = mpsc::channel::<Request>();
        let (replies, rx) = mpsc::channel();
        let requests = Arc::new(Mutex::new(requests));
        let queries: Arc<Mutex<Option<Arc<WorldQueries>>>> = Default::default();
        for i in 0..threads {
            let (requests, replies, queries, limit) =
                (requests.clone(), replies.clone(), queries.clone(), limit.clone());
            std::thread::Builder::new()
                .name(format!("cubox-finder-{i}"))
                .stack_size(STACK_SIZE)
                .spawn(move || {
                    // ends when the pool is dropped
                    loop {
                        let Ok((world, generation, job)) = requests.lock().unwrap().recv() else {
                            return;
                        };
                        limit.enter();
                        let current = {
                            let mut queries = queries.lock().unwrap();
                            match &*queries {
                                Some(current) if current.world == world => current.clone(),
                                _ => queries.insert(Arc::new(WorldQueries::new(world))).clone(),
                            }
                        };
                        let reply = current.run(&job);
                        limit.leave();
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
            limit,
            running: 0,
        }
    }

    pub fn set_limit(&mut self, threads: usize) {
        self.limit.threads.store(threads, Ordering::Relaxed);
        self.limit.changed.notify_all();
    }

    pub fn has_idle(&self) -> bool {
        self.running < self.limit.threads.load(Ordering::Relaxed) * JOBS_PER_THREAD
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
