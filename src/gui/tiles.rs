use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

use crate::{
    gui::{
        executor::Executor,
        map_renderer::{MapRenderer, TileTexture},
        world::{Estimates, Task, WorldParams},
    },
    tile_renderer::{Layer, TileKey},
};

/// Small tiles pop in quickly one by one and waste little work on parts outside the screen.
pub const TILE_SIZE: usize = 128;
/// Seconds a newly arrived texture takes to fade in over whatever was shown before.
const FADE_SECONDS: f64 = 0.15;
/// 128x128 tiles are 64 KiB as RGBA texture plus 96 KiB of estimates kept for downsampling,
/// so this is about 320 MiB. Browsers get less memory.
const MAX_TILES: usize = if cfg!(target_arch = "wasm32") { 512 } else { 2048 };
/// How many levels down a tile may be built from cached tiles, through the levels in between.
const MAX_BUILD_DEPTH: u32 = 4;

struct Entry {
    texture: Rc<TileTexture>,
    /// when the texture arrived, for fading it in
    arrived: f64,
    /// used to build the parent tile
    estimates: Estimates,
    last_used_frame: u64,
}

/// Caches tile textures and generates missing tiles with the platform's `Executor`.
pub struct TileCache {
    executor: Executor,
    entries: HashMap<TileKey, Entry>,
    /// tiles that have a task queued or running
    pending: HashSet<TileKey>,
    /// bumped on clear(), so results of tasks for an old seed get dropped
    generation: u64,
    frame: u64,
}

impl TileCache {
    pub fn new(params: WorldParams) -> Self {
        Self {
            executor: Executor::new(params),
            entries: HashMap::new(),
            pending: HashSet::new(),
            generation: 0,
            frame: 0,
        }
    }

    /// Drops all tiles, e.g. after the seed changed.
    pub fn clear(&mut self, params: WorldParams) {
        self.generation += 1;
        self.executor.set_world(params, self.generation);
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
    /// map view holds that back while the zoom level is still changing. `relief` tiles come last.
    pub fn update(&mut self, visible: &[TileKey], cover: &[TileKey], relief: &[TileKey], allow_sampling: bool) {
        self.frame += 1;
        let mut rank = HashMap::new();
        for (i, &key) in cover.iter().enumerate() {
            self.plan(key, i, true, &mut rank);
        }
        for (i, &key) in visible.iter().enumerate() {
            self.plan(key, cover.len() + i, allow_sampling, &mut rank);
        }
        for (i, &key) in relief.iter().enumerate() {
            self.plan(key, cover.len() + visible.len() + i, allow_sampling, &mut rank);
        }
        self.executor.update(rank, &mut self.pending);
    }

    /// Queues what `key` needs, at priority `rank`: building it from its children if they are
    /// cached, otherwise building the missing children first if they can be built from cached
    /// tiles further down (right after zooming out a few levels at once), otherwise sampling it.
    fn plan(&mut self, key: TileKey, rank: usize, allow_sampling: bool, ranks: &mut HashMap<TileKey, usize>) {
        ranks.entry(key).or_insert(rank);
        if self.pending.contains(&key) || self.entries.contains_key(&key) {
            return;
        }
        if key.layer == Layer::Relief {
            if allow_sampling {
                self.queue(key, Task::Relief);
            }
        } else if let Some(children) = self.finished_children(&key) {
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
        self.executor.queue(key, task);
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
        let finished: Vec<_> = self.executor.finished().collect();
        for done in finished {
            if done.generation != self.generation {
                continue;
            }
            self.pending.remove(&done.key);
            let Some(tile) = done.tile else { continue };
            // nearest at the finest level so single quarts stay crisp when zoomed in
            let texture = Rc::new(match done.key.layer {
                Layer::Biome => renderer.create_tile(device, queue, &tile.rgb, TILE_SIZE as u32, done.key.scale == 0),
                Layer::Relief => renderer.create_rgba_tile(device, queue, &tile.rgb, TILE_SIZE as u32, false),
            });
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
