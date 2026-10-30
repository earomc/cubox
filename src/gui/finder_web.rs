//! Runs finder jobs in web workers of their own: they load all of the structure data, which the
//! tile workers don't need. They only start when first needed, as each is a copy of the wasm
//! module. Jobs travel as text, see `Job::encode`.

use std::{cell::RefCell, rc::Rc};

use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{MessageEvent, Worker, WorkerOptions, WorkerType};

use crate::{
    dimension::Dimension,
    gui::finder::{FinderWorld, Job, Reply, WorldQueries},
};

const WORKERS: usize = 2;

pub struct Pool {
    inner: Rc<RefCell<Inner>>,
}

struct Inner {
    workers: Vec<Slot>,
    finished: Vec<(u64, Job, Reply)>,
}

struct Slot {
    worker: Worker,
    busy: Option<(u64, Job)>,
}

impl Pool {
    pub fn new() -> Self {
        let inner = Rc::new(RefCell::new(Inner {
            workers: Vec::new(),
            finished: Vec::new(),
        }));
        Self { inner }
    }

    pub fn has_idle(&self) -> bool {
        let inner = self.inner.borrow();
        inner.workers.len() < WORKERS || inner.workers.iter().any(|slot| slot.busy.is_none())
    }

    pub fn post(&mut self, world: FinderWorld, generation: u64, job: Job) {
        let all_busy = self.inner.borrow().workers.iter().all(|slot| slot.busy.is_some());
        if all_busy {
            let index = self.inner.borrow().workers.len();
            match spawn_worker(&self.inner, index) {
                Ok(worker) => self.inner.borrow_mut().workers.push(Slot { worker, busy: None }),
                Err(err) => log::error!("starting a finder worker: {err:?}"),
            }
        }
        let mut inner = self.inner.borrow_mut();
        let Some(slot) = inner.workers.iter_mut().find(|slot| slot.busy.is_none()) else {
            return;
        };
        let message = format!(
            "{} {} {}\n{}",
            world.seed,
            world.dimension.name(),
            world.sample_height,
            job.encode()
        );
        match slot.worker.post_message(&JsValue::from_str(&message)) {
            Ok(()) => slot.busy = Some((generation, job)),
            Err(err) => log::error!("sending a finder job: {err:?}"),
        }
    }

    pub fn finished(&mut self) -> impl Iterator<Item = (u64, Job, Reply)> + '_ {
        std::mem::take(&mut self.inner.borrow_mut().finished).into_iter()
    }
}

fn spawn_worker(inner: &Rc<RefCell<Inner>>, index: usize) -> Result<Worker, JsValue> {
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    let worker = Worker::new_with_options("./worker.js", &options)?;
    let weak = Rc::downgrade(inner);
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(inner) = weak.upgrade() else { return };
        let mut inner = inner.borrow_mut();
        let Some((generation, job)) = inner.workers.get_mut(index).and_then(|slot| slot.busy.take()) else {
            return;
        };
        // a failed job counts as nothing found, so that it isn't asked again and again
        let reply = event.data().as_string().and_then(|line| Reply::decode(&line));
        let reply = reply.unwrap_or_else(|| {
            log::error!("a finder worker failed: {:?}", event.data());
            match job {
                Job::Biome { .. } => Reply::Biome(Vec::new()),
                _ => Reply::Found(Vec::new()),
            }
        });
        inner.finished.push((generation, job, reply));
    });
    worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    // lives as long as the page, like the worker
    on_message.forget();
    Ok(worker)
}

thread_local! {
    static QUERIES: RefCell<Option<Rc<WorldQueries>>> = const { RefCell::new(None) };
}

/// Runs one message from `Pool::post` in a worker.
pub fn worker_run(message: &str) -> Result<String, JsValue> {
    let (world, job) = message.split_once('\n').ok_or("bad finder job")?;
    let mut words = world.split(' ');
    let world = (|| {
        Some(FinderWorld {
            seed: words.next()?.parse().ok()?,
            dimension: Dimension::from_name(words.next()?)?,
            sample_height: words.next()?.parse().ok()?,
        })
    })()
    .ok_or("bad finder world")?;
    let job = Job::decode(job).ok_or("bad finder job")?;
    let queries = QUERIES.with_borrow_mut(|queries| {
        if queries.as_ref().is_none_or(|q| q.world != world) {
            *queries = Some(Rc::new(WorldQueries::new(world)));
        }
        queries.clone().unwrap()
    });
    Ok(queries.run(&job).encode())
}
