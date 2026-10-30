//! Where structures are, found with Minecraft's real generation code: placement, exclusion zones,
//! biome and terrain checks and jigsaw assembly. A candidate chunk costs from microseconds up to
//! some milliseconds, so this is only asked for the part of the world being looked at.

use std::sync::{LazyLock, OnceLock};

use rustc_hash::FxHashSet;
use steel_registry::REGISTRY;
use steel_utils::{BlockPos, ChunkPos, Identifier};
use steel_worldgen::{
    biomes::BiomeSourceKind,
    density::{DimensionNoises, NoiseSettings},
    density_functions::{end::EndNoises, nether::NetherNoises, overworld::OverworldNoises},
    noise::LazyAquifer,
    structure::{
        GenerationContext, StructureGenerator,
        placement::{PlacementKind, StructurePlacement, StructureSet, load_vanilla_structure_sets},
    },
};

use crate::{
    dimension::Dimension,
    seed::generator_seed,
    terrain::{Noises, init_registry},
    tile_renderer::WORLD_BORDER_QUARTS,
};

/// A kind of structure as a player thinks of it, which can be several structures in the game.
pub struct StructureKind {
    /// also the name of its icon in `assets/icons`
    pub id: &'static str,
    pub name: &'static str,
    pub dimension: Dimension,
    /// the game's structures of this kind
    structures: &'static [&'static str],
}

impl StructureKind {
    fn is_in(&self, set: &StructureSet) -> bool {
        set.structures
            .iter()
            .any(|entry| self.structures.contains(&&*entry.structure.path))
    }
}

const fn kind(
    id: &'static str,
    name: &'static str,
    dimension: Dimension,
    structures: &'static [&'static str],
) -> StructureKind {
    StructureKind {
        id,
        name,
        dimension,
        structures,
    }
}

use Dimension::{End, Nether, Overworld};

pub const STRUCTURE_KINDS: &[StructureKind] = &[
    kind(
        "village",
        "Village",
        Overworld,
        &[
            "village_plains",
            "village_desert",
            "village_savanna",
            "village_snowy",
            "village_taiga",
        ],
    ),
    kind("pillager_outpost", "Pillager outpost", Overworld, &["pillager_outpost"]),
    kind("stronghold", "Stronghold", Overworld, &["stronghold"]),
    kind("mineshaft", "Mineshaft", Overworld, &["mineshaft", "mineshaft_mesa"]),
    kind("desert_pyramid", "Desert pyramid", Overworld, &["desert_pyramid"]),
    kind("jungle_temple", "Jungle temple", Overworld, &["jungle_pyramid"]),
    kind("swamp_hut", "Swamp hut", Overworld, &["swamp_hut"]),
    kind("igloo", "Igloo", Overworld, &["igloo"]),
    kind("ocean_monument", "Ocean monument", Overworld, &["monument"]),
    kind("woodland_mansion", "Woodland mansion", Overworld, &["mansion"]),
    kind(
        "ocean_ruin",
        "Ocean ruin",
        Overworld,
        &["ocean_ruin_cold", "ocean_ruin_warm"],
    ),
    kind("shipwreck", "Shipwreck", Overworld, &["shipwreck", "shipwreck_beached"]),
    kind("buried_treasure", "Buried treasure", Overworld, &["buried_treasure"]),
    kind(
        "ruined_portal",
        "Ruined portal",
        Overworld,
        &[
            "ruined_portal",
            "ruined_portal_desert",
            "ruined_portal_jungle",
            "ruined_portal_swamp",
            "ruined_portal_mountain",
            "ruined_portal_ocean",
        ],
    ),
    kind("ancient_city", "Ancient city", Overworld, &["ancient_city"]),
    kind("trail_ruins", "Trail ruins", Overworld, &["trail_ruins"]),
    kind("trial_chambers", "Trial chambers", Overworld, &["trial_chambers"]),
    kind("fortress", "Nether fortress", Nether, &["fortress"]),
    kind("bastion", "Bastion remnant", Nether, &["bastion_remnant"]),
    kind("nether_fossil", "Nether fossil", Nether, &["nether_fossil"]),
    kind(
        "ruined_portal_nether",
        "Ruined portal",
        Nether,
        &["ruined_portal_nether"],
    ),
    kind("end_city", "End city", End, &["end_city"]),
];

/// A structure that really generates. `kind` is an index into `STRUCTURE_KINDS`, x and z are blocks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Found {
    pub kind: usize,
    pub x: i32,
    pub z: i32,
}

/// How a kind's candidates are spread over the world.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Spread {
    /// one candidate per square grid cell of this many chunks
    Grid(i32),
    /// a fixed list of candidates around the origin
    Rings,
}

enum AnyNoises {
    Overworld(Noises<OverworldNoises>),
    Nether(Noises<NetherNoises>),
    End(Noises<EndNoises>),
}

/// How the candidates of a kind are spread.
pub fn spread(kind: usize) -> Option<Spread> {
    static SPREADS: LazyLock<Vec<Option<Spread>>> = LazyLock::new(|| {
        init_registry();
        let sets = load_vanilla_structure_sets();
        STRUCTURE_KINDS
            .iter()
            .map(|kind| {
                let (_, set) = sets.iter().find(|(_, set)| kind.is_in(set))?;
                Some(match set.placement.kind {
                    PlacementKind::RandomSpread { spacing, .. } => Spread::Grid(spacing),
                    PlacementKind::ConcentricRings { .. } => Spread::Rings,
                })
            })
            .collect()
    });
    SPREADS[kind]
}

/// Candidates tried at most per bin of `Finder::in_bin` for a grid of this spacing. Tight grids
/// have cheap but rarely real candidates.
pub fn bin_budget(spacing: i32) -> u32 {
    if spacing <= 4 { 256 } else { 16 }
}

thread_local! {
    /// A thread can only be part of one such pool for all of its life, even after dropping it.
    /// It needs none to spawn: browsers have no threads to spawn, on the desktop this is one already.
    static POOL: Result<rayon::ThreadPool, String> = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .use_current_thread()
        .build()
        .map_err(|err| err.to_string());
}

/// Answers where structures are in one world.
pub struct Finder {
    seed: i64,
    source: BiomeSourceKind,
    noises: AnyNoises,
    /// everything but the stronghold rings, which take over a second to find
    generator: StructureGenerator,
    /// only made when strongholds are wanted
    ring_generator: OnceLock<Option<StructureGenerator>>,
    /// the placement of each kind that exists in this dimension
    sets: Vec<KindSet>,
}

struct KindSet {
    kind: usize,
    placement: StructurePlacement,
    /// the only biomes at which assembling is worth it, see `Finder::might_generate`
    biomes: Option<FxHashSet<Identifier>>,
}

/// Villages assemble their center before the game checks the biome, 23ms per candidate compared to
/// under 1ms of other kinds, and most candidates are in the wrong biome.
const CHECK_BIOME_FIRST: &[&str] = &["village"];
/// Where the center of a village can be: the middle of the chunk give or take its size, in blocks.
const CENTER_OFFSETS: [i32; 5] = [-32, -8, 8, 24, 48];
/// Heights the surface can be at, the biome barely changes between them. In blocks.
const SURFACE_HEIGHTS: [i32; 4] = [48, 80, 128, 192];

impl Finder {
    pub fn new(seed: i64, dimension: Dimension) -> anyhow::Result<Self> {
        init_registry();
        let raw_seed = generator_seed(seed);
        let source = dimension.biome_source(seed);
        let without_rings = load_vanilla_structure_sets()
            .into_iter()
            .filter(|(_, set)| !matches!(set.placement.kind, PlacementKind::ConcentricRings { .. }))
            .collect();
        let generator = POOL.with(|pool| {
            let pool = pool.as_ref().map_err(|err| anyhow::anyhow!("{err}"))?;
            anyhow::Ok(StructureGenerator::vanilla_with_structure_sets(
                raw_seed as i64,
                None,
                &source,
                without_rings,
                pool,
            ))
        })?;
        let noises = match dimension {
            Dimension::Overworld => AnyNoises::Overworld(Noises::new(seed)),
            Dimension::Nether => AnyNoises::Nether(Noises::new(seed)),
            Dimension::End => AnyNoises::End(Noises::new(seed)),
        };
        let sets = load_vanilla_structure_sets();
        let sets = STRUCTURE_KINDS
            .iter()
            .enumerate()
            .filter(|(_, kind)| kind.dimension == dimension)
            .filter_map(|(index, kind)| {
                let (_, set) = sets.iter().find(|(_, set)| kind.is_in(set))?;
                Some(KindSet {
                    kind: index,
                    placement: set.placement.clone(),
                    biomes: CHECK_BIOME_FIRST.contains(&kind.id).then(|| {
                        REGISTRY
                            .structures
                            .iter()
                            .filter(|(_, structure)| kind.structures.contains(&&*structure.key.path))
                            .flat_map(|(_, structure)| structure.allowed_biomes.iter().cloned())
                            .collect()
                    }),
                })
            })
            .collect();
        Ok(Self {
            seed: raw_seed as i64,
            source,
            noises,
            generator,
            ring_generator: OnceLock::new(),
            sets,
        })
    }

    fn set(&self, kind: usize) -> Option<&KindSet> {
        self.sets.iter().find(|set| set.kind == kind)
    }

    /// The structure of `kind` in a grid cell of its `Spread::Grid`, if one really generates.
    pub fn in_cell(&self, kind: usize, cell_x: i32, cell_z: i32) -> Option<Found> {
        let set = self.set(kind)?;
        let PlacementKind::RandomSpread {
            spacing,
            separation,
            spread_type,
        } = set.placement.kind
        else {
            return None;
        };
        let chunk = StructurePlacement::get_potential_structure_chunk(
            self.seed,
            set.placement.salt,
            cell_x * spacing,
            cell_z * spacing,
            spacing,
            separation,
            spread_type,
        );
        if !self.might_generate(set, chunk) {
            return None;
        }
        self.at_chunk(&self.generator, kind, chunk)
    }

    /// False if the kind can't generate in the chunk because of the biome around its middle.
    fn might_generate(&self, set: &KindSet, chunk: ChunkPos) -> bool {
        let Some(biomes) = &set.biomes else { return true };
        let mut sampler = self.source.chunk_sampler();
        let (x, z) = (chunk.0.x * 16, chunk.0.y * 16);
        CENTER_OFFSETS.iter().any(|dx| {
            CENTER_OFFSETS.iter().any(|dz| {
                SURFACE_HEIGHTS
                    .iter()
                    .any(|y| biomes.contains(&sampler.sample((x + dx) >> 2, y >> 2, (z + dz) >> 2).key))
            })
        })
    }

    /// One structure of `kind` in a square bin of `4^level` grid cells, if one of the first `budget`
    /// candidates really generates. The candidates of a bin are spread over all of it, and those in
    /// a part of it are in the same order as that part's own, so that part finds the same structure.
    pub fn in_bin(&self, kind: usize, level: u32, x: i32, z: i32, budget: u32) -> Option<Found> {
        let tries = 4u64.saturating_pow(level).min(budget as u64);
        (0..tries).find_map(|index| {
            let (cell_x, cell_z) = bin_candidate(level, x, z, index);
            // bins at the edge of the world reach past it
            self.in_cell(kind, cell_x, cell_z)
                .filter(|found| found.x.abs().max(found.z.abs()) as i64 <= WORLD_BORDER_QUARTS as i64 * 4)
        })
    }

    /// All structures of a kind with `Spread::Rings`, which really generate.
    pub fn rings(&self, kind: usize) -> Vec<Found> {
        let generator = self.ring_generator.get_or_init(|| {
            POOL.with(|pool| match pool {
                Ok(pool) => Some(StructureGenerator::vanilla(self.seed, None, &self.source, pool)),
                Err(err) => {
                    log::error!("setting up strongholds: {err}");
                    None
                }
            })
        });
        let Some(generator) = generator else {
            return Vec::new();
        };
        let structures: Vec<_> = STRUCTURE_KINDS[kind]
            .structures
            .iter()
            .map(|path| Identifier::vanilla_static(path))
            .collect();
        let Some(plan) = generator.locate_plan_for_structures(&structures) else {
            return Vec::new();
        };
        plan.ring_candidates(BlockPos::new(0, 0, 0))
            .into_iter()
            .filter_map(|candidate| self.at_chunk(generator, kind, candidate.chunk_pos))
            .collect()
    }

    /// Runs the game's structure start generation for a chunk.
    fn at_chunk(&self, generator: &StructureGenerator, kind: usize, chunk: ChunkPos) -> Option<Found> {
        let structures = STRUCTURE_KINDS[kind].structures;
        let starts = match &self.noises {
            AnyNoises::Overworld(noises) => self.starts(generator, noises, chunk),
            AnyNoises::Nether(noises) => self.starts(generator, noises, chunk),
            AnyNoises::End(noises) => self.starts(generator, noises, chunk),
        };
        starts
            .into_iter()
            .find(|start| !start.pieces.is_empty() && structures.contains(&&*start.structure.path))
            .and_then(|start| start.bounding_box)
            .map(|bb| Found {
                kind,
                x: (bb.min_x() + bb.max_x()).div_euclid(2),
                z: (bb.min_z() + bb.max_z()).div_euclid(2),
            })
    }

    fn starts<N: DimensionNoises>(
        &self,
        generator: &StructureGenerator,
        noises: &Noises<N>,
        chunk: ChunkPos,
    ) -> Vec<steel_worldgen::structure::StructureStart> {
        let (chunk_x, chunk_z) = (chunk.0.x, chunk.0.y);
        let mut sampler = self.source.chunk_sampler();
        let mut height_cache = Default::default();
        let mut aquifer = LazyAquifer::new(chunk_x * 16, chunk_z * 16, &noises.splitter, &*noises.noises);
        let (mut surface_y_cache, mut height_cache_grid_ready) = (None, false);
        let mut ctx = GenerationContext::<'_, '_, N>::new(
            self.seed,
            chunk_x,
            chunk_z,
            N::Settings::SEA_LEVEL,
            &noises.noises,
            &noises.splitter,
            generator.template_pools(),
            generator.templates(),
            &mut sampler,
            &mut height_cache,
            &mut aquifer,
            &mut surface_y_cache,
            &mut height_cache_grid_ready,
        );
        generator.generate_starts_for_chunk(&mut ctx, |_| false)
    }
}

/// Grid cell number `index` of a bin. The lowest 2 bits of the index choose which of the 4 parts of
/// the bin it is in, in an order particular to the bin, and the others its number in that part.
fn bin_candidate(level: u32, x: i32, z: i32, index: u64) -> (i32, i32) {
    let (mut x, mut z, mut index) = (x, z, index);
    for level in (1..=level).rev() {
        let mixed = (x as u32 as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add((z as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
            .wrapping_add(level as u64)
            .wrapping_mul(0x1656_67B1_9E37_79F9);
        // the 24 orders of 4 parts
        let mut parts = [0, 1, 2, 3];
        let mut choice = (mixed >> 32) as usize % 24;
        for i in 0..3 {
            let swap = i + choice % (4 - i);
            choice /= 4 - i;
            parts.swap(i, swap);
        }
        let part = parts[(index & 3) as usize];
        index >>= 2;
        x = x * 2 + (part & 1);
        z = z * 2 + (part >> 1);
    }
    (x, z)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Structures need more stack than test threads have, like in SteelMC's own tests.
    fn with_big_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn bins_try_each_cell_once_and_parts_find_the_same() {
        let mut cells: Vec<_> = (0..16).map(|index| bin_candidate(2, -3, 5, index)).collect();
        // the first 4 are spread over the 4 parts of the bin
        let mut parts: Vec<_> = cells[..4].iter().map(|c| (c.0 >> 1, c.1 >> 1)).collect();
        parts.sort();
        parts.dedup();
        assert_eq!(parts.len(), 4);
        cells.sort();
        cells.dedup();
        assert_eq!(cells.len(), 16);
        assert!(cells.iter().all(|c| c.0 >> 2 == -3 && c.1 >> 2 == 5));

        with_big_stack(|| {
            let finder = Finder::new(12345, Dimension::Overworld).unwrap();
            let kind = STRUCTURE_KINDS.iter().position(|k| k.id == "shipwreck").unwrap();
            let mut found = 0;
            for (x, z) in (-8..8).flat_map(|x| (-8..8).map(move |z| (x, z))) {
                let Some(far) = finder.in_bin(kind, 3, x, z, 16) else {
                    continue;
                };
                found += 1;
                // whichever part it is in finds it too
                let at_part = (0..4).find_map(|part: i32| {
                    let (part_x, part_z) = (x * 2 + (part & 1), z * 2 + (part >> 1));
                    finder.in_bin(kind, 2, part_x, part_z, 16).filter(|f| *f == far)
                });
                assert_eq!(at_part, Some(far));
            }
            assert!(found > 100, "{found}");
        });
    }

    /// Compares with the game's code without shortcuts: the full set of structures, no biome check
    /// up front.
    #[test]
    fn shortcuts_change_nothing() {
        with_big_stack(|| {
            let finder = Finder::new(12345, Dimension::Overworld).unwrap();
            let full = POOL
                .with(|pool| StructureGenerator::vanilla(finder.seed, None, &finder.source, pool.as_ref().unwrap()));
            let mut real = 0;
            for (kind, def) in STRUCTURE_KINDS.iter().enumerate() {
                if def.dimension != Dimension::Overworld {
                    continue;
                }
                let Some(Spread::Grid(_)) = spread(kind) else { continue };
                let set = finder.set(kind).unwrap();
                let PlacementKind::RandomSpread {
                    spacing,
                    separation,
                    spread_type,
                } = set.placement.kind
                else {
                    continue;
                };
                let n = if def.id == "village" { 30 } else { 12 };
                for (x, z) in (0..n).flat_map(|x| (0..n).map(move |z| (x, z))) {
                    let (x, z) = (x * 13 - 200, z * 11 + 50);
                    let chunk = StructurePlacement::get_potential_structure_chunk(
                        finder.seed,
                        set.placement.salt,
                        x * spacing,
                        z * spacing,
                        spacing,
                        separation,
                        spread_type,
                    );
                    let ours = finder.in_cell(kind, x, z);
                    assert_eq!(ours, finder.at_chunk(&full, kind, chunk), "{} at {x} {z}", def.id);
                    real += ours.is_some() as i32;
                }
            }
            assert!(real > 100, "{real}");
        });
    }

    #[test]
    fn a_thread_can_find_in_several_worlds() {
        with_big_stack(|| {
            let kind = STRUCTURE_KINDS.iter().position(|k| k.id == "stronghold").unwrap();
            for seed in [1, 2, 3] {
                assert_eq!(Finder::new(seed, Dimension::Overworld).unwrap().rings(kind).len(), 128);
            }
        });
    }

    #[test]
    fn table_covers_all_vanilla_structures() {
        init_registry();
        for (key, set) in load_vanilla_structure_sets() {
            for entry in &set.structures {
                let path = &*entry.structure.path;
                assert!(
                    STRUCTURE_KINDS.iter().any(|kind| kind.structures.contains(&path)),
                    "{key}: {path} is missing in STRUCTURE_KINDS"
                );
            }
        }
    }

    #[test]
    fn all_kinds_are_found_in_their_biomes() {
        with_big_stack(|| {
            let finder = Finder::new(12345, Dimension::Overworld).unwrap();
            let source = Dimension::Overworld.biome_source(12345);
            let mut sampler = source.chunk_sampler();
            let index = |id| STRUCTURE_KINDS.iter().position(|k| k.id == id).unwrap();
            assert_eq!(finder.rings(index("stronghold")).len(), 128);

            let cells = |kind: usize| {
                (-40..40)
                    .flat_map(|x| (-40..40).map(move |z| (x, z)))
                    .filter_map(|(x, z)| finder.in_cell(kind, x, z))
                    .collect::<Vec<_>>()
            };
            let pyramids = cells(index("desert_pyramid"));
            assert!(!pyramids.is_empty());
            // the game checks the biome at the start, the marker is at the center of the structure
            let in_desert = pyramids
                .iter()
                .filter(|p| &*sampler.sample(p.x >> 2, 16, p.z >> 2).key.path == "desert")
                .count();
            assert!(
                in_desert * 10 >= pyramids.len() * 9,
                "{in_desert} of {}",
                pyramids.len()
            );
        });
    }
}
