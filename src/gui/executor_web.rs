//! Runs tile tasks in web workers. Each worker is its own copy of the wasm module with its own
//! world generator, so all cores are used without shared memory, which would need special headers
//! from the server. The main thread decides what runs next, like the threads do on the desktop.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

use js_sys::{Array, Uint8Array, Uint16Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{MessageEvent, Worker, WorkerOptions, WorkerType};

use crate::{
    dimension::Dimension,
    gui::world::{Finished, Queue, Task, World, WorldParams},
    tile_renderer::{Estimate, Tile, TileKey},
};

/// The worker script next to the page, see `web/worker.js`.
const WORKER_SCRIPT: &str = "./worker.js";

pub struct Executor {
    inner: Rc<RefCell<Inner>>,
}

struct Inner {
    queue: Queue,
    params: WorldParams,
    workers: Vec<Slot>,
    finished: Vec<Finished>,
}

struct Slot {
    worker: Worker,
    /// the tile it works on and the generation it was asked for
    busy: Option<(TileKey, u64)>,
}

impl Executor {
    pub fn new(params: WorldParams) -> Self {
        let inner = Rc::new(RefCell::new(Inner {
            queue: Queue::default(),
            params,
            workers: Vec::new(),
            finished: Vec::new(),
        }));
        let count = web_sys::window().map_or(4.0, |w| w.navigator().hardware_concurrency()) as usize;
        for index in 0..count.clamp(1, 16) {
            match spawn_worker(&inner, index) {
                Ok(worker) => inner.borrow_mut().workers.push(Slot { worker, busy: None }),
                Err(err) => log::error!("starting a tile worker: {err:?}"),
            }
        }
        Self { inner }
    }

    /// Switches to another world. Tasks of the old one are dropped, results of running ones come
    /// back with the old generation.
    pub fn set_world(&mut self, params: WorldParams, generation: u64) {
        let mut inner = self.inner.borrow_mut();
        inner.params = params;
        inner.queue.tasks.clear();
        inner.queue.generation = generation;
    }

    pub fn queue(&mut self, key: TileKey, task: Task) {
        self.inner.borrow_mut().queue.tasks.insert(key, task);
    }

    /// Sets the wanted tiles (see `Queue::set_rank`) and gives idle workers the most important tasks.
    pub fn update(&mut self, rank: HashMap<TileKey, usize>, pending: &mut HashSet<TileKey>) {
        let mut inner = self.inner.borrow_mut();
        inner.queue.set_rank(rank, pending);
        let Inner {
            queue, params, workers, ..
        } = &mut *inner;
        for slot in workers.iter_mut().filter(|slot| slot.busy.is_none()) {
            let Some((key, task, generation)) = queue.take_best() else {
                break;
            };
            let (request, transfer) = encode_request(params, key, &task);
            match slot.worker.post_message_with_transfer(&request, &transfer) {
                Ok(()) => slot.busy = Some((key, generation)),
                Err(err) => log::error!("sending a tile task: {err:?}"),
            }
        }
    }

    pub fn finished(&mut self) -> impl Iterator<Item = Finished> + '_ {
        std::mem::take(&mut self.inner.borrow_mut().finished).into_iter()
    }
}

fn spawn_worker(inner: &Rc<RefCell<Inner>>, index: usize) -> Result<Worker, JsValue> {
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    let worker = Worker::new_with_options(WORKER_SCRIPT, &options)?;
    let weak = Rc::downgrade(inner);
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(inner) = weak.upgrade() else { return };
        let mut inner = inner.borrow_mut();
        let Some((key, generation)) = inner.workers.get_mut(index).and_then(|slot| slot.busy.take()) else {
            return;
        };
        let tile = decode_tile(&event.data());
        if tile.is_none() {
            log::error!("a tile worker failed: {:?}", event.data());
        }
        inner.finished.push(Finished { generation, key, tile });
    });
    worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    // lives as long as the page, like the worker
    on_message.forget();
    Ok(worker)
}

/// `[seed, dimension, sample height, samples setting, scale, x, z, tile size, samples, children]`.
/// The seed is a string, JavaScript numbers can't hold all 64 bits. `children` are the estimates
/// of the 4 children for downsampling, or null to sample the tile.
fn encode_request(params: &WorldParams, key: TileKey, task: &Task) -> (Array, Array) {
    let transfer = Array::new();
    let children = match task {
        Task::Render => JsValue::NULL,
        Task::Downsample(children) => {
            let array = Array::new();
            for child in children {
                let estimates = Uint16Array::from(child.as_flattened());
                transfer.push(&estimates.buffer());
                array.push(&estimates);
            }
            array.into()
        }
    };
    let request = Array::of(&[
        params.seed.to_string().into(),
        params.dimension.name().into(),
        params.sample_height.into(),
        params.samples.into(),
        key.scale.into(),
        key.x.into(),
        key.z.into(),
        (key.tile_size_px as u32).into(),
        key.samples.into(),
        children,
    ]);
    (request, transfer)
}

/// `[rgb, estimates]` from `worker_run`, or None if the worker reported an error.
fn decode_tile(data: &JsValue) -> Option<Tile> {
    let reply: Array = data.clone().dyn_into().ok()?;
    let rgb: Uint8Array = reply.get(0).dyn_into().ok()?;
    let estimates: Uint16Array = reply.get(1).dyn_into().ok()?;
    let estimates = estimates.to_vec();
    Some(Tile {
        rgb: rgb.to_vec(),
        estimates: estimates.as_chunks::<3>().0.to_vec(),
    })
}

thread_local! {
    /// The world of this worker, kept as long as the requests are for it.
    static WORLD: RefCell<Option<Rc<World>>> = const { RefCell::new(None) };
}

/// Runs one request from `encode_request` in a worker. Returns `[rgb, estimates]`.
pub fn worker_run(request: &Array) -> Result<Array, JsValue> {
    let number = |i: u32| {
        request
            .get(i)
            .as_f64()
            .ok_or_else(|| JsValue::from_str("bad tile request"))
    };
    let seed: i64 = request
        .get(0)
        .as_string()
        .and_then(|s| s.parse().ok())
        .ok_or("bad seed")?;
    let dimension = request
        .get(1)
        .as_string()
        .and_then(|d| Dimension::from_name(&d))
        .ok_or("bad dimension")?;
    let params = WorldParams {
        seed,
        dimension,
        sample_height: number(2)? as i32,
        samples: number(3)? as u8,
    };
    let key = TileKey {
        scale: number(4)? as u8,
        x: number(5)? as i32,
        z: number(6)? as i32,
        tile_size_px: number(7)? as usize,
        samples: number(8)? as u8,
        sample_height: params.sample_height,
    };
    let children = request.get(9);
    let task = if children.is_null() {
        Task::Render
    } else {
        let children: Array = children.dyn_into()?;
        let child = |i: u32| -> Result<Arc<Vec<Estimate>>, JsValue> {
            let estimates: Uint16Array = children.get(i).dyn_into()?;
            Ok(Arc::new(estimates.to_vec().as_chunks::<3>().0.to_vec()))
        };
        Task::Downsample([child(0)?, child(1)?, child(2)?, child(3)?])
    };

    let world = WORLD.with_borrow_mut(|world| {
        if world.as_ref().is_none_or(|w| w.params != params) {
            *world = Some(Rc::new(World::new(params)));
        }
        world.clone().unwrap()
    });
    let tile = world.run(key, task, &|| true).ok_or("tile abandoned")?;
    Ok(Array::of(&[
        Uint8Array::from(tile.rgb.as_slice()).into(),
        Uint16Array::from(tile.estimates.as_flattened()).into(),
    ]))
}
