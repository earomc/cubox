//! Where structures are, found with Minecraft's real generation code: placement, exclusion zones,
//! biome and terrain checks and jigsaw assembly. A candidate chunk costs from microseconds up to
//! some milliseconds, so this is only asked for the part of the world being looked at.

use std::sync::LazyLock;

use steel_utils::{BlockPos, ChunkPos};
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

/// Answers where structures are in one world. Creating it finds the stronghold rings, which takes
/// a moment, so it happens on a background thread and only when structures are asked for.
pub struct Finder {
    seed: i64,
    source: BiomeSourceKind,
    noises: AnyNoises,
    generator: StructureGenerator,
    /// the placement of each kind that exists in this dimension
    sets: Vec<KindSet>,
}

struct KindSet {
    kind: usize,
    placement: StructurePlacement,
}

impl Finder {
    pub fn new(seed: i64, dimension: Dimension) -> anyhow::Result<Self> {
        init_registry();
        let raw_seed = generator_seed(seed);
        let source = dimension.biome_source(seed);
        // on the desktop this runs on its own thread already, browsers have no threads to spawn
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .use_current_thread()
            .build()?;
        let generator = StructureGenerator::vanilla(raw_seed as i64, None, &source, &pool);
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
                })
            })
            .collect();
        Ok(Self {
            seed: raw_seed as i64,
            source,
            noises,
            generator,
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
        self.at_chunk(kind, chunk)
    }

    /// All structures of a kind with `Spread::Rings`, which really generate.
    pub fn rings(&self, kind: usize) -> Vec<Found> {
        let structures: Vec<_> = STRUCTURE_KINDS[kind]
            .structures
            .iter()
            .map(|path| steel_utils::Identifier::vanilla_static(path))
            .collect();
        let Some(plan) = self.generator.locate_plan_for_structures(&structures) else {
            return Vec::new();
        };
        plan.ring_candidates(BlockPos::new(0, 0, 0))
            .into_iter()
            .filter_map(|candidate| self.at_chunk(kind, candidate.chunk_pos))
            .collect()
    }

    /// Runs the game's structure start generation for a chunk.
    fn at_chunk(&self, kind: usize, chunk: ChunkPos) -> Option<Found> {
        let structures = STRUCTURE_KINDS[kind].structures;
        let starts = match &self.noises {
            AnyNoises::Overworld(noises) => self.starts(noises, chunk),
            AnyNoises::Nether(noises) => self.starts(noises, chunk),
            AnyNoises::End(noises) => self.starts(noises, chunk),
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
            self.generator.template_pools(),
            self.generator.templates(),
            &mut sampler,
            &mut height_cache,
            &mut aquifer,
            &mut surface_y_cache,
            &mut height_cache_grid_ready,
        );
        self.generator.generate_starts_for_chunk(&mut ctx, |_| false)
    }
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
