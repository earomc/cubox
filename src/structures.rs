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
        GenerationContext, StructureGenerator, StructurePiece, StructurePiecePayload,
        placement::{
            PlacementKind, StructurePlacement, StructureSet, generate_ring_positions, load_vanilla_structure_sets,
        },
    },
};

use crate::{
    dimension::Dimension,
    features::{FeatureRoll, feature_of, ring_gateways},
    noise::{NoiseCache, Ore, Underground},
    terrain::{Noises, init_registry},
    tile_renderer::WORLD_BORDER_QUARTS,
};

/// A kind of marker as a player thinks of it: a structure, which can be several in the game, or
/// something else with a place on the map, like the spawn point or an ore vein.
pub struct MarkerKind {
    /// also the name of its icon in `assets/icons`
    pub id: &'static str,
    pub name: &'static str,
    pub dimension: Dimension,
    /// the game's structures of this kind
    structures: &'static [&'static str],
    /// only those with this piece, which needs all their pieces to be known
    with_piece: Option<&'static str>,
}

impl MarkerKind {
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
) -> MarkerKind {
    MarkerKind {
        id,
        name,
        dimension,
        structures,
        with_piece: None,
    }
}

use Dimension::{End, Nether, Overworld};

pub const MARKER_KINDS: &[MarkerKind] = &[
    // not a structure, but marked like one. No game structures: it is found in another way
    kind("spawn", "Spawn point", Overworld, &[]),
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
    kind(
        "abandoned_camp",
        "Abandoned camp",
        Overworld,
        &[
            "abandoned_camp_bamboo_jungle",
            "abandoned_camp_birch_forest",
            "abandoned_camp_cherry_grove",
            "abandoned_camp_dappled_forest",
            "abandoned_camp_flower_forest",
            "abandoned_camp_forest",
            "abandoned_camp_meadow",
            "abandoned_camp_old_growth_birch_forest",
            "abandoned_camp_old_growth_pine_taiga",
            "abandoned_camp_old_growth_spruce_taiga",
            "abandoned_camp_pale_garden",
            "abandoned_camp_savanna",
            "abandoned_camp_snowy_taiga",
            "abandoned_camp_sparse_jungle",
            "abandoned_camp_swamp",
            "abandoned_camp_taiga",
            "abandoned_camp_windswept_forest",
            "abandoned_camp_wooded_badlands",
        ],
    ),
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
    kind("end_gateway", "End gateway after the dragon", End, &[]),
    kind("end_gateway_scattered", "Scattered end gateway (maybe there)", End, &[]),
    kind("geode", "Geode (maybe there)", Overworld, &[]),
    kind("copper_vein", "Copper ore vein", Overworld, &[]),
    kind("iron_vein", "Iron ore vein", Overworld, &[]),
    MarkerKind {
        with_piece: Some("end_city/ship"),
        ..kind("end_city_ship", "End city with ship", End, &["end_city"])
    },
];

/// The index in `MARKER_KINDS` of the kind with this id.
pub fn kind_index(id: &str) -> usize {
    MARKER_KINDS
        .iter()
        .position(|kind| kind.id == id)
        .unwrap_or_else(|| panic!("no kind {id}"))
}

/// A structure that really generates. `kind` is an index into `MARKER_KINDS`, x and z are blocks.
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
        MARKER_KINDS
            .iter()
            .map(|kind| {
                if feature_of(kind.id).is_some() {
                    return Some(Spread::Grid(1));
                }
                if ore_of(kind.id).is_some() {
                    return Some(Spread::Grid(VEIN_CELL_CHUNKS));
                }
                if kind.structures.is_empty() {
                    return Some(Spread::Rings);
                }
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

/// The grid of vein markers, in chunks: one marker at most in each cell.
const VEIN_CELL_CHUNKS: i32 = 32;
/// Spots looked at per side of a cell.
const VEIN_SAMPLES: i32 = 4;
/// Blocks of ore a look around a spot has to find, of what it looks at. See `Underground::ore_near`.
const VEIN_MIN_ORE: u32 = 6;
/// How many of those spots need veins for a marker.
const VEIN_MIN_SAMPLES: usize = 4;

pub fn ore_of(id: &str) -> Option<Ore> {
    match id {
        "copper_vein" => Some(Ore::Copper),
        "iron_vein" => Some(Ore::Iron),
        _ => None,
    }
}

/// Whether `candidates` knows where a kind can be: the kinds on a grid, and the stronghold rings.
pub fn has_candidates(kind: usize) -> bool {
    matches!(spread(kind), Some(Spread::Grid(_))) || is_ring_structure(kind)
}

fn is_ring_structure(kind: usize) -> bool {
    kind_sets()[kind]
        .as_ref()
        .is_some_and(|set| matches!(set.placement.kind, PlacementKind::ConcentricRings { .. }))
}

/// Whether the `candidates` of a kind only depend on the lower 48 bits of the seed. Kinds without
/// candidates, like the spawn point, trivially do.
pub fn depends_on_lower_bits(kind: usize) -> bool {
    feature_of(MARKER_KINDS[kind].id).is_none() && ore_of(MARKER_KINDS[kind].id).is_none()
}

/// Whether a search can look for a kind, unlike the spawn point it is measured from.
pub fn is_searchable(kind: usize) -> bool {
    MARKER_KINDS[kind].id != "spawn"
}

/// How far from the middle of its chunk a structure's marker can be, in blocks. See the test
/// `markers_stay_within_their_margin`.
pub fn marker_margin(kind: usize) -> i32 {
    match MARKER_KINDS[kind].id {
        "swamp_hut" | "geode" | "end_gateway_scattered" | "end_gateway" => 16,
        "copper_vein" | "iron_vein" => 380,
        "desert_pyramid" | "jungle_temple" | "igloo" | "buried_treasure" | "ocean_monument" => 64,
        _ => 512,
    }
}

/// Where a kind might be around a spot, from the seed alone: the grid cell and middle of the
/// chunk of each candidate within `radius` blocks of `center` or `marker_margin` further. A kind
/// that isn't on a grid has none. Structures depend on nothing but the lower 48 bits of the seed,
/// see `depends_on_lower_bits`.
pub fn candidates(seed: i64, kind: usize, center: [i32; 2], radius: i32, out: &mut Vec<([i32; 2], [i32; 2])>) {
    if ore_of(MARKER_KINDS[kind].id).is_some() {
        // only the noise says where they are, every cell in reach is a candidate
        let size = (VEIN_CELL_CHUNKS * 16) as i64;
        let reach = radius as i64 + marker_margin(kind) as i64;
        let cells = |middle: i32| {
            ((middle as i64 - reach).div_euclid(size) as i32)..=((middle as i64 + reach).div_euclid(size) as i32)
        };
        for z in cells(center[1]) {
            for x in cells(center[0]) {
                let middle = [x * size as i32 + size as i32 / 2, z * size as i32 + size as i32 / 2];
                let (dx, dz) = ((middle[0] - center[0]) as i64, (middle[1] - center[1]) as i64);
                if dx * dx + dz * dz <= reach * reach {
                    out.push(([x, z], middle));
                }
            }
        }
        return;
    }
    if let Some(feature) = feature_of(MARKER_KINDS[kind].id) {
        let reach = radius as i64 + marker_margin(kind) as i64;
        let chunks = |middle: i32| {
            ((middle as i64 - reach).div_euclid(16) as i32)..=((middle as i64 + reach).div_euclid(16) as i32)
        };
        for z in chunks(center[1]) {
            for x in chunks(center[0]) {
                if feature.in_chunk(seed, [x, z]).is_some() {
                    let middle = [x * 16 + 8, z * 16 + 8];
                    let (dx, dz) = ((middle[0] - center[0]) as i64, (middle[1] - center[1]) as i64);
                    if dx * dx + dz * dz <= reach * reach {
                        out.push(([x, z], middle));
                    }
                }
            }
        }
        return;
    }
    let Some(set) = &kind_sets()[kind] else { return };
    let placement = &set.placement;
    let reach = radius as i64 + marker_margin(kind) as i64;
    let in_reach = |middle: [i32; 2]| {
        let (dx, dz) = ((middle[0] - center[0]) as i64, (middle[1] - center[1]) as i64);
        dx * dx + dz * dz <= reach * reach
    };
    let (spacing, separation, spread_type) = match &placement.kind {
        PlacementKind::RandomSpread {
            spacing,
            separation,
            spread_type,
        } => (*spacing, *separation, *spread_type),
        PlacementKind::ConcentricRings {
            distance,
            spread,
            count,
            ..
        } => {
            // where the rings put them before a biome search moves them, see `marker_margin`
            type NoSnap = fn(i32, i32, &mut steel_utils::random::legacy_random::LegacyRandom) -> Option<(i32, i32)>;
            let chunks = POOL.with(|pool| match pool {
                Ok(pool) => generate_ring_positions::<NoSnap>(seed, *distance, *spread, *count, None, pool),
                // then there is no finder either, which would find nothing
                Err(_) => Vec::new(),
            });
            for (index, chunk) in chunks.into_iter().enumerate() {
                let middle = [chunk.0.x * 16 + 8, chunk.0.y * 16 + 8];
                if in_reach(middle) {
                    out.push(([index as i32, 0], middle));
                }
            }
            return;
        }
    };
    let cell_of = |block: i64| block.div_euclid(spacing as i64 * 16) as i32;
    let cells = |middle: i32| cell_of(middle as i64 - reach)..=cell_of(middle as i64 + reach);
    for cell_z in cells(center[1]) {
        for cell_x in cells(center[0]) {
            let chunk = StructurePlacement::get_potential_structure_chunk(
                seed,
                placement.salt,
                cell_x * spacing,
                cell_z * spacing,
                spacing,
                separation,
                spread_type,
            );
            let middle = [chunk.0.x * 16 + 8, chunk.0.y * 16 + 8];
            if in_reach(middle) && placement.is_structure_chunk(seed, chunk.0.x, chunk.0.y, None) {
                out.push(([cell_x, cell_z], middle));
            }
        }
    }
}

/// How many candidates of a bin are tried by the end of each round of looking for a structure, for
/// a grid of this spacing and up to about `max_tries`. Those that fail mostly do so at the cheap
/// checks. Tight grids have many candidates of which few are real.
pub fn bin_rounds(spacing: i32, max_tries: u32) -> [u32; 5] {
    let tries = [16, 128, 1024, 8192, 65536].map(|tries: u32| tries.min(max_tries));
    if spacing <= 4 { tries.map(|n| n * 8) } else { tries }
}

/// What the game requires of the lowest corner of an end city.
const END_CITY_MIN_HEIGHT: i32 = 60;

/// All kinds, the rarest in the world first, measured as real structures per area on a seed. Where
/// markers compete for space the rare ones are placed first, the others would crowd them out.
const RARITY_ORDER: &[&str] = &[
    "spawn",
    "stronghold",
    "woodland_mansion",
    "buried_treasure",
    "swamp_hut",
    "desert_pyramid",
    "jungle_temple",
    "pillager_outpost",
    "igloo",
    "ancient_city",
    "ocean_monument",
    "trail_ruins",
    "village",
    "abandoned_camp",
    "shipwreck",
    "ruined_portal",
    "ocean_ruin",
    "trial_chambers",
    "mineshaft",
    "fortress",
    "bastion",
    "ruined_portal_nether",
    "nether_fossil",
    "end_gateway",
    "end_gateway_scattered",
    "end_city_ship",
    "end_city",
    "geode",
    "iron_vein",
    "copper_vein",
];

/// How many of a grid's candidates really generate, measured over thousands of cells on several
/// seeds. Depends a little on the seed, it is used to know how many to look for.
const REAL_RATES: &[(&str, f64)] = &[
    ("geode", 0.041),
    ("copper_vein", 0.1),
    ("iron_vein", 0.1),
    ("end_gateway_scattered", 0.0004),
    ("village", 0.233),
    ("pillager_outpost", 0.032),
    ("mineshaft", 0.0037),
    ("desert_pyramid", 0.012),
    ("jungle_temple", 0.019),
    ("swamp_hut", 0.010),
    ("igloo", 0.057),
    ("ocean_monument", 0.091),
    ("woodland_mansion", 0.021),
    ("ocean_ruin", 0.30),
    ("shipwreck", 0.33),
    ("buried_treasure", 0.00029),
    ("ruined_portal", 0.99),
    ("ancient_city", 0.028),
    ("trail_ruins", 0.11),
    ("trial_chambers", 0.96),
    ("abandoned_camp", 0.34),
    ("fortress", 0.49),
    ("bastion", 0.51),
    ("nether_fossil", 0.083),
    ("ruined_portal_nether", 1.0),
    ("end_city", 0.097),
    ("end_city_ship", 0.054),
];

/// How many of a kind's grid candidates really generate.
pub fn real_rate(kind: usize) -> f64 {
    REAL_RATES
        .iter()
        .find(|(id, _)| *id == MARKER_KINDS[kind].id)
        .map_or(1.0, |(_, rate)| *rate)
}

/// The place of a kind in `RARITY_ORDER`, lower is rarer.
pub fn rarity(kind: usize) -> usize {
    static RANKS: LazyLock<Vec<usize>> = LazyLock::new(|| {
        MARKER_KINDS
            .iter()
            .map(|kind| RARITY_ORDER.iter().position(|id| *id == kind.id).unwrap_or(usize::MAX))
            .collect()
    });
    RANKS[kind]
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
    /// assemble all pieces like the game, only to test that it comes to the same
    assemble_all: bool,
    dimension: Dimension,
    /// only made when vein markers are wanted
    veins: OnceLock<Underground>,
}

struct KindSet {
    placement: StructurePlacement,
    /// the biomes the game requires, see `biome_allows`, if checking them first is worth it
    biomes: Option<FxHashSet<Identifier>>,
    /// offsets from the chunk's corner where the game can check the biome, in blocks
    biome_offsets: &'static [i32],
}

/// What is known about each kind without a seed.
fn kind_sets() -> &'static [Option<KindSet>] {
    static SETS: LazyLock<Vec<Option<KindSet>>> = LazyLock::new(|| {
        init_registry();
        let sets = load_vanilla_structure_sets();
        MARKER_KINDS
            .iter()
            .map(|kind| {
                let (_, set) = sets.iter().find(|(_, set)| kind.is_in(set))?;
                Some(KindSet {
                    placement: set.placement.clone(),
                    biome_offsets: if CHECKED_AWAY_FROM_MIDDLE.contains(&kind.id) {
                        AROUND_MIDDLE
                    } else {
                        MIDDLE
                    },
                    biomes: (!USUALLY_REAL.contains(&kind.id) && !CHECKED_ANYWHERE.contains(&kind.id)).then(|| {
                        REGISTRY
                            .structures
                            .iter()
                            .filter(|(_, structure)| kind.structures.contains(&&*structure.key.path))
                            .flat_map(|(_, structure)| structure.allowed_biomes.iter().cloned())
                            .collect()
                    }),
                })
            })
            .collect()
    });
    &SETS
}

/// False if `kind` can't generate in the chunk because of the biome around its middle. The game
/// finds this out too, but only after scanning heights or assembling pieces, which most candidates
/// would be wasted on. `shortcuts_change_nothing` makes sure this comes to the same.
fn biome_allows(source: &BiomeSourceKind, set: &KindSet, chunk: [i32; 2]) -> bool {
    let Some(biomes) = &set.biomes else { return true };
    let mut sampler = source.chunk_sampler();
    let (x, z) = (chunk[0] * 16, chunk[1] * 16);
    set.biome_offsets.iter().any(|dx| {
        set.biome_offsets.iter().any(|dz| {
            CHECK_HEIGHTS
                .iter()
                .any(|y| biomes.contains(&sampler.sample((x + dx) >> 2, y >> 2, (z + dz) >> 2).key))
        })
    })
}

/// Which of `candidates` of `kind` the biomes allow, from the world's biomes alone.
pub fn allowed_by_biomes(source: &BiomeSourceKind, kind: usize, cells: &mut Vec<([i32; 2], [i32; 2])>) {
    // rings are moved to a biome after, it isn't checked where their candidates are
    if let Some(set) = &kind_sets()[kind]
        && !is_ring_structure(kind)
    {
        cells.retain(|(_, middle)| biome_allows(source, set, [(middle[0] - 8) / 16, (middle[1] - 8) / 16]));
    }
}

/// Nearly all candidates of these really generate, a check up front would only cost time.
const USUALLY_REAL: &[&str] = &["ruined_portal", "ruined_portal_nether", "trial_chambers"];
/// This checks the biome at a random spot of its chunk, and nearly all of it is one biome anyway.
const CHECKED_ANYWHERE: &[&str] = &["nether_fossil"];
/// These check the biome at a piece that can be away from the middle of the chunk.
const CHECKED_AWAY_FROM_MIDDLE: &[&str] = &[
    "village",
    "pillager_outpost",
    "ancient_city",
    "trail_ruins",
    "abandoned_camp",
    "bastion",
    "woodland_mansion",
];
const MIDDLE: &[i32] = &[8];
const AROUND_MIDDLE: &[i32] = &[-32, -8, 8, 24, 48];
/// Heights the game can check the biome at, which depend on terrain it hasn't looked at yet. The
/// biome changes slowly with the height. In blocks, the likeliest first.
const CHECK_HEIGHTS: [i32; 21] = [
    64, 80, 48, 96, 32, 112, 16, 128, 0, 144, -16, 160, -32, 176, -48, 192, -64, 208, 224, 240, 256,
];

impl Finder {
    pub fn new(seed: i64, dimension: Dimension) -> anyhow::Result<Self> {
        init_registry();
        let source = dimension.biome_source(seed);
        let without_rings = load_vanilla_structure_sets()
            .into_iter()
            .filter(|(_, set)| !matches!(set.placement.kind, PlacementKind::ConcentricRings { .. }))
            .collect();
        let generator = POOL.with(|pool| {
            let pool = pool.as_ref().map_err(|err| anyhow::anyhow!("{err}"))?;
            anyhow::Ok(StructureGenerator::vanilla_with_structure_sets(
                seed,
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
        Ok(Self {
            seed,
            source,
            noises,
            generator,
            ring_generator: OnceLock::new(),
            assemble_all: false,
            dimension,
            veins: OnceLock::new(),
        })
    }

    fn set(&self, kind: usize) -> Option<&KindSet> {
        kind_sets()[kind]
            .as_ref()
            .filter(|_| MARKER_KINDS[kind].dimension == self.dimension)
    }

    /// The structure of `kind` in a grid cell of its `Spread::Grid`, if one really generates.
    pub fn in_cell(&self, kind: usize, cell_x: i32, cell_z: i32) -> Option<Found> {
        if let Some(ore) = ore_of(MARKER_KINDS[kind].id) {
            if self.dimension != Overworld {
                return None;
            }
            let veins = self.veins.get_or_init(|| Underground::new(self.seed));
            let (size, step) = (VEIN_CELL_CHUNKS * 16, VEIN_CELL_CHUNKS * 16 / VEIN_SAMPLES);
            let mut hits = Vec::new();
            for dz in 0..VEIN_SAMPLES {
                for dx in 0..VEIN_SAMPLES {
                    let (x, z) = (
                        cell_x * size + dx * step + step / 2,
                        cell_z * size + dz * step + step / 2,
                    );
                    if veins.at(x, z) == Some(ore) {
                        hits.push([x, z]);
                    }
                }
            }
            if hits.len() < VEIN_MIN_SAMPLES {
                return None;
            }
            // the spot nearest their middle, the middle itself can be beside a curved vein
            let count = hits.len() as i64;
            let middle = [0, 1].map(|axis| hits.iter().map(|hit| hit[axis] as i64).sum::<i64>() / count);
            hits.sort_by_key(|hit| {
                let (dx, dz) = (hit[0] as i64 - middle[0], hit[1] as i64 - middle[1]);
                dx * dx + dz * dz
            });
            // where real ore is: much of a vein's area can be cave or sea, and its blocks are random
            let mut cache = NoiseCache::default();
            let [x, z] = hits
                .into_iter()
                .find(|hit| veins.ore_near(&mut cache, ore, hit[0], hit[1]) >= VEIN_MIN_ORE)?;
            return Some(Found { kind, x, z });
        }
        if let Some(feature) = feature_of(MARKER_KINDS[kind].id) {
            if feature.dimension != self.dimension {
                return None;
            }
            let (block, y) = feature.in_chunk(self.seed, [cell_x, cell_z])?;
            if !feature.biome_allows(&self.source, block, y) || !self.terrain_allows(kind, cell_x, cell_z, block) {
                return None;
            }
            return Some(Found {
                kind,
                x: block[0],
                z: block[1],
            });
        }
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
        // the cheapest first: this is only a random number for kinds that are rare on purpose
        if !set.placement.is_structure_chunk(self.seed, chunk.0.x, chunk.0.y, None)
            || !biome_allows(&self.source, set, [chunk.0.x, chunk.0.y])
        {
            return None;
        }
        self.at_chunk(&self.generator, kind, chunk)
    }

    /// All structures of `kind` within `radius` blocks of `center`.
    pub fn around(&self, kind: usize, center: [i32; 2], radius: i32) -> Vec<Found> {
        let is_near = |found: &Found| {
            let (dx, dz) = ((found.x - center[0]) as i64, (found.z - center[1]) as i64);
            dx * dx + dz * dz <= radius as i64 * radius as i64
        };
        if spread(kind) == Some(Spread::Rings) {
            return self.rings(kind).into_iter().filter(is_near).collect();
        }
        let mut cells = Vec::new();
        candidates(self.seed, kind, center, radius, &mut cells);
        cells
            .into_iter()
            .filter_map(|(cell, _)| self.in_world(kind, cell[0], cell[1]))
            .filter(is_near)
            .collect()
    }

    /// One structure of `kind` in a square bin of `4^level` grid cells, if one of the candidates
    /// `tries` really generates. The candidates of a bin are spread over all of it, and those in a
    /// part of it are in the same order as that part's own, so that part finds the same structure.
    pub fn in_bin(&self, kind: usize, level: u32, x: i32, z: i32, tries: std::ops::Range<u32>) -> Option<Found> {
        tries.into_iter().find_map(|index| {
            let (cell_x, cell_z) = bin_candidate(level, x, z, index as u64);
            self.in_world(kind, cell_x, cell_z)
        })
    }

    /// All structures of `kind` in a bin of `4^level` grid cells.
    pub fn all_in_bin(&self, kind: usize, level: u32, x: i32, z: i32) -> Vec<Found> {
        (0..1u64 << (2 * level))
            .filter_map(|index| {
                let (cell_x, cell_z) = bin_candidate(level, x, z, index);
                self.in_world(kind, cell_x, cell_z)
            })
            .collect()
    }

    /// What the game looks at in the terrain before placing a feature, as far as noise can tell.
    /// Caves carved later aren't known.
    fn terrain_allows(&self, kind: usize, cell_x: i32, cell_z: i32, block: [i32; 2]) -> bool {
        match (&self.noises, MARKER_KINDS[kind].id) {
            (AnyNoises::Overworld(_), "geode") => {
                let Some((_, points)) = FeatureRoll::geode_points(self.seed, [cell_x, cell_z]) else {
                    return false;
                };
                let veins = self.veins.get_or_init(|| Underground::new(self.seed));
                let mut cache = NoiseCache::default();
                let not_solid = points
                    .iter()
                    .filter(|point| !veins.is_solid(&mut cache, point[0], point[1], point[2]))
                    .count();
                not_solid <= 1
            }
            // a gateway needs ground below it, the end has lots of nothing
            (AnyNoises::End(noises), "end_gateway_scattered") => {
                let mut cache: <EndNoises as DimensionNoises>::ColumnCache = Default::default();
                cache.ensure(block[0], block[1], &noises.noises);
                (0..128)
                    .step_by(4)
                    .any(|y| noises.noises.router_final_density(&mut cache, block[0], y, block[1]) > 0.0)
            }
            _ => true,
        }
    }

    /// `in_cell` without what is past the edge of the world, which bins at the edge reach into.
    pub fn in_world(&self, kind: usize, cell_x: i32, cell_z: i32) -> Option<Found> {
        self.in_cell(kind, cell_x, cell_z)
            .filter(|found| found.x.abs().max(found.z.abs()) as i64 <= WORLD_BORDER_QUARTS as i64 * 4)
    }

    /// All structures of a kind with `Spread::Rings`, which really generate.
    pub fn rings(&self, kind: usize) -> Vec<Found> {
        if MARKER_KINDS[kind].id == "end_gateway" {
            return ring_gateways(self.seed)
                .into_iter()
                .map(|[x, z]| Found { kind, x, z })
                .collect();
        }
        if MARKER_KINDS[kind].structures.is_empty() {
            let spawn = self.source.initial_spawn_search_origin();
            return vec![Found {
                kind,
                x: spawn.x(),
                z: spawn.z(),
            }];
        }
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
        let structures: Vec<_> = MARKER_KINDS[kind]
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

    /// Runs the game's structure start generation for a chunk. The position of structures made of
    /// jigsaw pieces is that of their center piece, like a village's meeting point.
    fn at_chunk(&self, generator: &StructureGenerator, kind: usize, chunk: ChunkPos) -> Option<Found> {
        let structures = MARKER_KINDS[kind].structures;
        let starts = match &self.noises {
            AnyNoises::Overworld(noises) => self.starts(generator, noises, chunk, kind),
            AnyNoises::Nether(noises) => self.starts(generator, noises, chunk, kind),
            AnyNoises::End(noises) => self.starts(generator, noises, chunk, kind),
        };
        starts
            .into_iter()
            .find(|start| !start.pieces.is_empty() && structures.contains(&&*start.structure.path))
            .filter(|start| {
                MARKER_KINDS[kind]
                    .with_piece
                    .is_none_or(|piece| start.pieces.iter().any(|p| has_template(p, piece)))
            })
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
        kind: usize,
    ) -> Vec<steel_worldgen::structure::StructureStart> {
        let (chunk_x, chunk_z) = (chunk.0.x, chunk.0.y);
        let mut sampler = self.source.chunk_sampler();
        let mut height_cache = Default::default();
        let mut aquifer = LazyAquifer::new(
            chunk_x * 16,
            chunk_z * 16,
            &noises.splitter,
            &*noises.noises,
            &noises.preliminary_surface,
        );
        let (mut surface_y_cache, mut height_cache_grid_ready) = (None, false);
        let ctx = GenerationContext::<'_, '_, N>::new(
            self.seed,
            chunk_x,
            chunk_z,
            N::Settings::SEA_LEVEL,
            &noises.noises,
            &noises.splitter,
            &noises.preliminary_surface,
            generator.template_pools(),
            generator.templates(),
            &mut sampler,
            &mut height_cache,
            &mut aquifer,
            &mut surface_y_cache,
            &mut height_cache_grid_ready,
        );
        // a marker needs no more than where it starts
        let mut ctx = if self.assemble_all { ctx } else { ctx.with_locate_only() };
        // The game measures an end city's 4 corners before it looks at any of them. The first is
        // always the middle of the chunk, and most candidates that fail already fail there.
        if !self.assemble_all
            && MARKER_KINDS[kind].structures == ["end_city"]
            && ctx.base_height_full(chunk_x * 16 + 7, chunk_z * 16 + 7, false) - 1 < END_CITY_MIN_HEIGHT
        {
            return Vec::new();
        }
        generator.generate_starts_for_chunk(&mut ctx, |_| false)
    }
}

fn has_template(piece: &StructurePiece, template: &str) -> bool {
    matches!(&piece.payload, StructurePiecePayload::Template(data) if &*data.template_id.path == template)
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
            let kind = kind_index("shipwreck");
            let mut found = 0;
            for (x, z) in (-8..8).flat_map(|x| (-8..8).map(move |z| (x, z))) {
                let Some(far) = finder.in_bin(kind, 3, x, z, 0..16) else {
                    continue;
                };
                found += 1;
                // whichever part it is in finds it too
                let at_part = (0..4).find_map(|part: i32| {
                    let (part_x, part_z) = (x * 2 + (part & 1), z * 2 + (part >> 1));
                    finder.in_bin(kind, 2, part_x, part_z, 0..16).filter(|f| *f == far)
                });
                assert_eq!(at_part, Some(far));
            }
            assert!(found > 100, "{found}");
        });
    }

    /// Compares with the game's code without shortcuts: the full set of structures, no checks up
    /// front. A lot of candidates, because a check that is wrong now and then would lose structures.
    #[test]
    fn markers_stay_within_their_margin() {
        with_big_stack(|| {
            for dimension in Dimension::ALL {
                let finder = Finder::new(12345, dimension).unwrap();
                for (kind, def) in MARKER_KINDS.iter().enumerate() {
                    if def.dimension != dimension || spread(kind) == Some(Spread::Rings) {
                        continue;
                    }
                    let mut cells = Vec::new();
                    candidates(finder.seed, kind, [0, 0], 40_000, &mut cells);
                    let mut real = 0;
                    for (cell, middle) in cells.into_iter().take(1500) {
                        if let Some(found) = finder.in_cell(kind, cell[0], cell[1]) {
                            let (dx, dz) = ((found.x - middle[0]) as i64, (found.z - middle[1]) as i64);
                            let margin = marker_margin(kind) as i64;
                            assert!(dx * dx + dz * dz <= margin * margin, "{} {dx} {dz}", def.id);
                            real += 1;
                        }
                    }
                    assert!(real > 0, "{}", def.id);
                }
            }
        });
    }

    /// Every real stronghold is near the position its ring gives it, so that a search can turn down
    /// a seed from the rings alone. Prints how far they get.
    #[test]
    fn strongholds_are_near_where_their_rings_put_them() {
        with_big_stack(|| {
            let kind = kind_index("stronghold");
            let mut farthest = 0i64;
            for seed in [1, 2, 12345, -987_654_321, 77, 4242] {
                let finder = Finder::new(seed, Dimension::Overworld).unwrap();
                let mut rings = Vec::new();
                candidates(seed, kind, [0, 0], 30_000_000, &mut rings);
                let real = finder.rings(kind);
                assert_eq!(real.len(), rings.len());
                for found in real {
                    let nearest = rings
                        .iter()
                        .map(|(_, middle)| {
                            let (dx, dz) = ((found.x - middle[0]) as i64, (found.z - middle[1]) as i64);
                            dx * dx + dz * dz
                        })
                        .min()
                        .unwrap();
                    farthest = farthest.max(nearest);
                }
            }
            let farthest = (farthest as f64).sqrt();
            println!("strongholds are at most {farthest:.0} blocks from their ring position");
            assert!(farthest <= marker_margin(kind) as f64, "{farthest}");
        });
    }

    #[test]
    fn candidates_depend_on_the_lower_48_bits_only() {
        for (kind, _) in MARKER_KINDS.iter().enumerate() {
            if !depends_on_lower_bits(kind) {
                continue;
            }
            for seed in 0..20i64 {
                let seed = seed.wrapping_mul(0x5DEE_CE66_D123_4567u64 as i64);
                let other = seed ^ (seed.rotate_left(13) << 48) ^ (1 << 63);
                let (mut a, mut b) = (Vec::new(), Vec::new());
                candidates(seed, kind, [0, 0], 5000, &mut a);
                candidates(other, kind, [0, 0], 5000, &mut b);
                assert_eq!(a, b, "{}", MARKER_KINDS[kind].id);
            }
        }
    }

    #[test]
    fn geodes_look_at_points_around_them() {
        let mut seen = 0;
        for i in 0..20_000 {
            let chunk = [i % 200 - 100, i / 200 - 50];
            let roll = feature_of("geode").unwrap().in_chunk(7, chunk);
            let points = FeatureRoll::geode_points(7, chunk);
            assert_eq!(roll.is_some(), points.is_some());
            if let (Some(([x, z], y)), Some((origin, points))) = (roll, points) {
                assert_eq!(origin, [x, y.unwrap(), z]);
                assert!((3..=4).contains(&points.len()));
                for point in points {
                    for axis in 0..3 {
                        assert!((4..=6).contains(&(point[axis] - origin[axis])));
                    }
                }
                seen += 1;
            }
        }
        assert!(seen > 100);
    }

    #[test]
    fn vein_markers_are_on_veins_of_both_ores_on_every_seed() {
        with_big_stack(|| {
            for seed in [12345, 1, -987_654_321, 77] {
                let finder = Finder::new(seed, Dimension::Overworld).unwrap();
                let veins = Underground::new(seed);
                for (id, ore) in [("copper_vein", Ore::Copper), ("iron_vein", Ore::Iron)] {
                    let kind = kind_index(id);
                    let found = finder.around(kind, [0, 0], 10_000);
                    assert!(found.len() > 50, "{id} of {seed}: {}", found.len());
                    for marker in found {
                        assert_eq!(veins.at(marker.x, marker.z), Some(ore), "{marker:?}");
                        let mut cache = NoiseCache::default();
                        assert!(veins.ore_near(&mut cache, ore, marker.x, marker.z) >= VEIN_MIN_ORE);
                    }
                }
            }
        });
    }

    #[test]
    fn shortcuts_change_nothing() {
        let threads: Vec<_> = Dimension::ALL
            .into_iter()
            .map(|dimension| {
                std::thread::Builder::new()
                    .stack_size(16 << 20)
                    .spawn(move || {
                        let finder = Finder::new(12345, dimension).unwrap();
                        let full = POOL.with(|pool| {
                            StructureGenerator::vanilla(finder.seed, None, &finder.source, pool.as_ref().unwrap())
                        });
                        let mut real = 0;
                        for (kind, def) in MARKER_KINDS.iter().enumerate() {
                            if def.dimension != dimension {
                                continue;
                            }
                            let Some(Spread::Grid(_)) = spread(kind) else { continue };
                            if !depends_on_lower_bits(kind) {
                                continue;
                            }
                            let set = finder.set(kind).unwrap();
                            let PlacementKind::RandomSpread {
                                spacing,
                                separation,
                                spread_type,
                            } = set.placement.kind
                            else {
                                continue;
                            };
                            let n = match def.id {
                                "village" => 40,
                                "mineshaft" | "buried_treasure" | "nether_fossil" => 30,
                                _ => 50,
                            };
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
                        println!("{dimension:?}: {real} real");
                        assert!(real > 100, "{dimension:?}: {real}");
                    })
                    .unwrap()
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }

    #[test]
    fn low_end_city_shortcut_finds_the_same() {
        with_big_stack(|| {
            let mut real = 0;
            for seed in [12345, 1, -987654321] {
                let finder = Finder::new(seed, Dimension::End).unwrap();
                let mut whole = Finder::new(seed, Dimension::End).unwrap();
                whole.assemble_all = true;
                for id in ["end_city", "end_city_ship"] {
                    let kind = kind_index(id);
                    for (x, z) in (20..50).flat_map(|x| (20..50).map(move |z| (x, z))) {
                        let ours = finder.in_cell(kind, x, z);
                        assert_eq!(ours, whole.in_cell(kind, x, z), "{id} of {seed} at {x} {z}");
                        real += ours.is_some() as i32;
                    }
                }
            }
            assert!(real > 100, "{real}");
        });
    }

    #[test]
    fn stopping_at_the_center_piece_finds_the_same() {
        with_big_stack(|| {
            let finder = Finder::new(12345, Dimension::Overworld).unwrap();
            let mut whole = Finder::new(12345, Dimension::Overworld).unwrap();
            whole.assemble_all = true;
            let mut real = 0;
            for id in ["village", "pillager_outpost", "ancient_city", "trail_ruins"] {
                let kind = kind_index(id);
                for (x, z) in (0..14).flat_map(|x| (0..14).map(move |z| (x * 13 - 90, z * 11 + 50))) {
                    let (center, assembled) = (finder.in_cell(kind, x, z), whole.in_cell(kind, x, z));
                    assert_eq!(center.is_some(), assembled.is_some(), "{id} at {x} {z}");
                    if let (Some(center), Some(assembled)) = (center, assembled) {
                        real += 1;
                        // no further than a big structure reaches
                        let distance = (center.x - assembled.x).abs().max((center.z - assembled.z).abs());
                        assert!(distance < 128, "{id} at {x} {z}: {distance}");
                    }
                }
            }
            assert!(real > 20, "{real}");
        });
    }

    #[test]
    fn rounds_find_the_same_as_one_long_search() {
        with_big_stack(|| {
            let finder = Finder::new(12345, Dimension::Overworld).unwrap();
            let kind = kind_index("swamp_hut");
            let mut found = 0;
            for (x, z) in (-5..5).flat_map(|x| (-5..5).map(move |z| (x, z))) {
                let long = finder.in_bin(kind, 8, x, z, 0..256);
                let rounds = finder
                    .in_bin(kind, 8, x, z, 0..16)
                    .or_else(|| finder.in_bin(kind, 8, x, z, 16..128))
                    .or_else(|| finder.in_bin(kind, 8, x, z, 128..256));
                assert_eq!(long, rounds);
                found += long.is_some() as i32;
            }
            assert!(found > 10, "{found}");
        });
    }

    /// On a seed the table wasn't measured on.
    #[test]
    fn real_rates_are_about_right() {
        for dimension in Dimension::ALL {
            with_big_stack(move || {
                let finder = Finder::new(987_654, dimension).unwrap();
                for (kind, def) in MARKER_KINDS.iter().enumerate() {
                    if def.dimension != dimension {
                        continue;
                    }
                    let Some(Spread::Grid(spacing)) = spread(kind) else {
                        continue;
                    };
                    // far too rare to measure quickly
                    if def.id == "buried_treasure" || def.id == "end_gateway_scattered" {
                        continue;
                    }
                    let n: i32 = if spacing <= 4 { 250 } else { 50 };
                    let cells = (0..n).flat_map(|x| (0..n).map(move |z| (x * 29 - 700, z * 31 - 800)));
                    let real = cells.filter(|&(x, z)| finder.in_cell(kind, x, z).is_some()).count();
                    let rate = real as f64 / (n * n) as f64;
                    let table = real_rate(kind);
                    assert!(
                        (table / 2.2..table * 2.2).contains(&rate),
                        "{}: {rate} and not {table}",
                        def.id
                    );
                }
            });
        }
    }

    #[test]
    fn the_spawn_point_is_one_marker_near_the_origin() {
        with_big_stack(|| {
            let kind = kind_index("spawn");
            assert_eq!(spread(kind), Some(Spread::Rings));
            for seed in [1, 12345, -987] {
                let finder = Finder::new(seed, Dimension::Overworld).unwrap();
                let found = finder.rings(kind);
                assert_eq!(found.len(), 1);
                let spawn = Dimension::Overworld.biome_source(seed).initial_spawn_search_origin();
                assert_eq!((found[0].x, found[0].z), (spawn.x(), spawn.z()));
                // the game looks up to 2048 blocks from the origin, then 512 from what it found
                assert!(found[0].x.abs().max(found[0].z.abs()) <= 2560, "{found:?}");
            }
        });
    }

    #[test]
    fn every_kind_has_a_rarity() {
        let mut ranks: Vec<usize> = (0..MARKER_KINDS.len()).map(rarity).collect();
        ranks.sort();
        ranks.dedup();
        assert_eq!(ranks.len(), MARKER_KINDS.len());
        assert!(ranks.iter().all(|rank| *rank != usize::MAX));
        let rank = |id| rarity(kind_index(id));
        assert!(rank("end_city_ship") < rank("end_city"));
    }

    #[test]
    fn ships_are_some_of_the_end_cities() {
        with_big_stack(|| {
            let finder = Finder::new(12345, Dimension::End).unwrap();
            let index = |id| kind_index(id);
            let (city, ship) = (index("end_city"), index("end_city_ship"));
            let (mut cities, mut ships) = (0, 0);
            for (x, z) in (-30..30).flat_map(|x| (-30..30).map(move |z| (x, z))) {
                let at = finder.in_cell(city, x, z);
                cities += at.is_some() as i32;
                if let Some(found) = finder.in_cell(ship, x, z) {
                    ships += 1;
                    let at = at.expect("a ship without a city");
                    assert_eq!((found.x, found.z), (at.x, at.z));
                }
            }
            println!("{ships} ships of {cities} cities");
            assert!(ships > 0 && ships < cities, "{ships} of {cities}");
        });
    }

    #[test]
    fn a_thread_can_find_in_several_worlds() {
        with_big_stack(|| {
            let kind = kind_index("stronghold");
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
                    MARKER_KINDS.iter().any(|kind| kind.structures.contains(&path)),
                    "{key}: {path} is missing in MARKER_KINDS"
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
            let index = |id| kind_index(id);
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
