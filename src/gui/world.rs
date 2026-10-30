//! What tile generation needs on every platform: the world, the work and its priorities.
//! Where the work runs differs, see `executor_native.rs` and `executor_web.rs`.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, OnceLock},
};

use steel_worldgen::biomes::BiomeSourceKind;

use crate::{
    dimension::Dimension,
    terrain::Terrain,
    tile_renderer::{Estimate, Lab, Layer, Shading, Tile, TileKey, calibrate, downsample, render_tile, world_mean},
};

/// Everything that decides how a tile looks besides its key.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WorldParams {
    pub seed: i64,
    pub dimension: Dimension,
    pub sample_height: i32,
    /// the samples setting, cover tiles use 1 regardless
    pub samples: u8,
}

pub type Estimates = Arc<Vec<Estimate>>;

/// Work for one tile. The variants are ordered by priority, cheap work first.
pub enum Task {
    /// average 4 finished children, no sampling needed
    Downsample([Estimates; 4]),
    /// sample the tile
    Render,
    /// sample the terrain for a relief tile, the most expensive work
    Relief,
}

impl Task {
    fn phase(&self) -> u8 {
        match self {
            Task::Downsample(_) => 0,
            Task::Render => 1,
            Task::Relief => 2,
        }
    }
}

pub struct Finished {
    pub generation: u64,
    pub key: TileKey,
    /// None if the tile was abandoned because it went off screen
    pub tile: Option<Tile>,
}

/// Tasks waiting to run, and which tiles are wanted how much.
#[derive(Default)]
pub struct Queue {
    pub tasks: HashMap<TileKey, Task>,
    /// visible tiles by priority, lower is more important. Tasks for tiles not in here aren't run,
    /// and running ones are abandoned.
    pub rank: HashMap<TileKey, usize>,
    pub generation: u64,
}

impl Queue {
    /// Sets the wanted tiles and drops the tasks of the others, removing them from `pending`.
    pub fn set_rank(&mut self, rank: HashMap<TileKey, usize>, pending: &mut HashSet<TileKey>) {
        self.rank = rank;
        let rank = &self.rank;
        self.tasks.retain(|key, _| {
            let keep = rank.contains_key(key);
            if !keep {
                pending.remove(key);
            }
            keep
        });
    }

    /// Takes the most important task: cheap ones first, then by rank.
    pub fn take_best(&mut self) -> Option<(TileKey, Task, u64)> {
        let (_, _, key) = self
            .tasks
            .iter()
            .filter_map(|(key, task)| Some((task.phase(), *self.rank.get(key)?, *key)))
            .min_by_key(|&(phase, rank, _)| (phase, rank))?;
        let task = self.tasks.remove(&key).unwrap();
        Some((key, task, self.generation))
    }
}

/// A level's shading, filled in by whoever needs it first.
type ShadingCell = OnceLock<Shading>;

/// The world the tiles are generated for.
pub struct World {
    pub params: WorldParams,
    source: BiomeSourceKind,
    /// average color of the world, computed by the first worker that needs it
    mean: OnceLock<Lab>,
    /// shading per scale and samples setting, each calibrated by the first worker that needs it
    shadings: Mutex<HashMap<(u8, u8), Arc<ShadingCell>>>,
    /// only made when relief tiles are wanted
    terrain: OnceLock<Terrain>,
}

impl World {
    pub fn new(params: WorldParams) -> Self {
        Self {
            source: params.dimension.biome_source(params.seed),
            params,
            mean: OnceLock::new(),
            shadings: Default::default(),
            terrain: OnceLock::new(),
        }
    }

    pub fn mean(&self) -> Lab {
        *self
            .mean
            .get_or_init(|| world_mean(&self.source, self.params.sample_height))
    }

    pub fn shading(&self, key: TileKey) -> Shading {
        let cell = self
            .shadings
            .lock()
            .unwrap()
            .entry((key.scale, key.samples))
            .or_default()
            .clone();
        *cell.get_or_init(|| calibrate(&self.source, key, self.mean()))
    }

    /// Runs a task. `keep_going` is asked while sampling, a tile it says no to is abandoned.
    pub fn run(&self, key: TileKey, task: Task, keep_going: &dyn Fn() -> bool) -> Option<Tile> {
        if matches!(task, Task::Relief) {
            return self.relief(key, keep_going);
        }
        let shading = self.shading(key);
        match task {
            Task::Downsample(children) => Some(downsample(children.each_ref().map(|c| c.as_slice()), key, &shading)),
            Task::Render => render_tile(&self.source, key, &shading, keep_going),
            Task::Relief => unreachable!(),
        }
    }

    /// A relief tile: its `rgb` are RGBA pixels and it has no estimates.
    fn relief(&self, key: TileKey, keep_going: &dyn Fn() -> bool) -> Option<Tile> {
        let terrain = self.terrain.get_or_init(|| Terrain::new(self.params.seed));
        let Layer::Relief { step, sea_floor } = key.layer else {
            return None;
        };
        let (x, z) = key.origin();
        let rgb = terrain.relief(
            [x * 4, z * 4],
            key.stride() * 4,
            key.tile_size_px,
            step as usize,
            sea_floor,
            keep_going,
        )?;
        Some(Tile {
            rgb,
            estimates: Vec::new(),
        })
    }
}
