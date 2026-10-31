use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

use crate::{
    gui::{
        executor::Executor,
        map_renderer::{MapRenderer, TileTexture},
        resources,
        world::{Biomes, Estimates, Task, WorldParams},
    },
    tile_renderer::{Layer, TileKey},
};

/// Small tiles pop in quickly one by one and waste little work on parts outside the screen.
pub const TILE_SIZE: usize = 128;

/// How many levels down a tile may be built from cached tiles, through the levels in between.
const MAX_BUILD_DEPTH: u32 = 4;

struct Entry {
    texture: Rc<TileTexture>,
    /// when the texture arrived, for fading it in
    arrived: f64,
    /// used to build the parent tile
    estimates: Estimates,
    biomes: Biomes,
    /// the biome numbers on the GPU, for highlighting
    mask: Option<Rc<TileTexture>>,
    mask_arrived: f64,
    last_used_frame: u64,
}

/// Not the number of a biome, so the table of `MapRenderer::set_darkening` darkens it.
const UNKNOWN_BIOME: u8 = 255;

/// Caches tile textures and generates missing tiles with the platform's `Executor`.
pub struct TileCache {
    executor: Executor,
    entries: HashMap<TileKey, Entry>,
    /// tiles that have a task queued or running
    pending: HashSet<TileKey>,
    /// darkens areas no biome numbers are known for yet, like a biome that isn't highlighted
    unknown_mask: Option<Rc<TileTexture>>,
    /// bumped on clear(), so results of tasks for an old seed get dropped
    generation: u64,
    frame: u64,
    max_tiles: usize,
    /// biomes are highlighted, so all tiles need their biome numbers
    highlight: bool,
    /// seconds a newly arrived texture takes to fade in over whatever was shown before
    fade_seconds: f64,
}

impl TileCache {
    pub fn new(params: WorldParams) -> Self {
        Self {
            executor: Executor::new(params),
            entries: HashMap::new(),
            pending: HashSet::new(),
            unknown_mask: None,
            generation: 0,
            frame: 0,
            max_tiles: resources::tiles_in_mib(resources::default_cache_mib()),
            highlight: false,
            fade_seconds: 0.15,
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

    /// Of the tiles waiting or being generated, how many are height map tiles.
    pub fn pending_relief(&self) -> usize {
        self.pending
            .iter()
            .filter(|key| matches!(key.layer, Layer::Relief { .. }))
            .count()
    }

    /// How many tiles may be generated at once.
    pub fn set_thread_limit(&mut self, threads: usize) {
        self.executor.set_limit(threads);
    }

    pub fn set_highlight(&mut self, highlight: bool) {
        self.highlight = highlight;
    }

    pub fn set_fade_seconds(&mut self, seconds: f64) {
        self.fade_seconds = seconds.max(0.001);
    }

    pub fn set_max_tiles(&mut self, tiles: usize) {
        self.max_tiles = tiles;
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
        // only highlighting needs what flat tiles were made without
        if self.highlight
            && allow_sampling
            && !self.pending.contains(&key)
            && self.entries.get(&key).is_some_and(|entry| entry.mask.is_none())
            && key.layer == Layer::Biome
        {
            self.queue(key, Task::Numbers);
            return;
        }
        if self.pending.contains(&key) || self.entries.contains_key(&key) {
            return;
        }
        if matches!(key.layer, Layer::Relief { .. }) {
            if allow_sampling {
                self.queue(key, Task::Relief);
            }
        } else if let Some((children, biomes)) = self.finished_children(&key) {
            self.queue(key, Task::Downsample(children, biomes));
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
        let alpha = ((now - entry.arrived) / self.fade_seconds).clamp(0.0, 1.0) as f32;
        Some((entry.texture.clone(), alpha))
    }

    /// The biome numbers of a tile on the GPU, if it is cached.
    pub fn mask(&mut self, key: &TileKey) -> Option<Rc<TileTexture>> {
        let entry = self.entries.get_mut(key)?;
        entry.last_used_frame = self.frame;
        entry.mask.clone()
    }

    /// How far the biome numbers of a tile have faded in.
    pub fn mask_alpha(&self, key: &TileKey, now: f64) -> f32 {
        self.entries.get(key).map_or(0.0, |entry| {
            let arrived = entry.arrived.max(entry.mask_arrived);
            ((now - arrived) / self.fade_seconds).clamp(0.0, 1.0) as f32
        })
    }

    pub fn unknown_mask(&self) -> Option<Rc<TileTexture>> {
        self.unknown_mask.clone()
    }

    fn finished_children(&self, key: &TileKey) -> Option<([Estimates; 4], [Biomes; 4])> {
        if key.scale == 0 {
            return None;
        }
        let [a, b, c, d] = children(key).map(|child| self.entries.get(&child));
        let [a, b, c, d] = [a?, b?, c?, d?];
        Some((
            [a, b, c, d].map(|e| e.estimates.clone()),
            [a, b, c, d].map(|e| e.biomes.clone()),
        ))
    }

    /// Uploads finished tiles as textures. `now` is the egui time in seconds. Returns true if anything changed.
    pub fn poll(&mut self, now: f64, device: &wgpu::Device, queue: &wgpu::Queue, renderer: &MapRenderer) -> bool {
        if self.highlight && self.unknown_mask.is_none() {
            self.unknown_mask = Some(Rc::new(renderer.create_mask_tile(device, queue, &[UNKNOWN_BIOME], 1)));
        }
        let mut changed = false;
        let finished: Vec<_> = self.executor.finished().collect();
        for done in finished {
            if done.generation != self.generation {
                continue;
            }
            self.pending.remove(&done.key);
            let Some(tile) = done.tile else { continue };
            // only the numbers of a tile that is there already
            if tile.rgb.is_empty() {
                if let Some(entry) = self.entries.get_mut(&done.key) {
                    entry.mask = Some(Rc::new(renderer.create_mask_tile(
                        device,
                        queue,
                        &tile.biomes,
                        TILE_SIZE as u32,
                    )));
                    entry.mask_arrived = now;
                    entry.biomes = Arc::new(tile.biomes);
                    changed = true;
                }
                continue;
            }
            // nearest at the finest level so single quarts stay crisp when zoomed in
            let texture = Rc::new(match done.key.layer {
                Layer::Biome => renderer.create_tile(device, queue, &tile.rgb, TILE_SIZE as u32, done.key.scale == 0),

                Layer::Relief { .. } => renderer.create_rgba_tile(device, queue, &tile.rgb, TILE_SIZE as u32, true),
            });
            let mask = (!tile.biomes.is_empty())
                .then(|| Rc::new(renderer.create_mask_tile(device, queue, &tile.biomes, TILE_SIZE as u32)));
            let estimates = Arc::new(tile.estimates);
            let biomes = Arc::new(tile.biomes);
            self.entries.insert(
                done.key,
                Entry {
                    texture,
                    arrived: now,
                    estimates,
                    biomes,
                    mask,
                    mask_arrived: now,
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
        if self.entries.len() <= self.max_tiles {
            return;
        }
        let mut by_age: Vec<(u64, TileKey)> = self.entries.iter().map(|(k, e)| (e.last_used_frame, *k)).collect();
        by_age.sort_unstable_by_key(|(used, _)| *used);
        let excess = self.entries.len() - self.max_tiles;
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
