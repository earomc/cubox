use steel_worldgen::biomes::BiomeSourceKind;

use crate::seed::generator_seed;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dimension {
    Overworld,
    Nether,
    End,
}

impl Dimension {
    pub const ALL: [Dimension; 3] = [Dimension::Overworld, Dimension::Nether, Dimension::End];

    pub fn name(self) -> &'static str {
        match self {
            Dimension::Overworld => "overworld",
            Dimension::Nether => "nether",
            Dimension::End => "end",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.name() == name)
    }

    pub fn biome_source(self, seed: i64) -> BiomeSourceKind {
        let seed = generator_seed(seed);
        let source = match self {
            Dimension::Overworld => BiomeSourceKind::overworld(seed),
            Dimension::Nether => BiomeSourceKind::nether(seed),
            Dimension::End => BiomeSourceKind::end(seed),
        };
        // The first sample builds SteelMC's biome lookup tree with rayon, behind a lock. A worker of
        // a rayon pool doing that could pick up another job while it waits, which then waits for the
        // same lock forever. So it is built here, before any worker needs it.
        source.chunk_sampler().sample(0, 0, 0);
        source
    }

    /// Height (y in blocks) to sample biomes at by default. The overworld is sampled above the
    /// terrain, so caves don't show. The nether has biomes all the way through, the end ignores y.
    pub fn default_height(self) -> i32 {
        match self {
            Dimension::Overworld => 256,
            Dimension::Nether => 64,
            Dimension::End => 64,
        }
    }
}
