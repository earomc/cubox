use steel_worldgen::biomes::BiomeSourceKind;

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

    pub fn biome_source(self, seed: u64) -> BiomeSourceKind {
        match self {
            Dimension::Overworld => BiomeSourceKind::overworld(seed),
            Dimension::Nether => BiomeSourceKind::nether(seed),
            Dimension::End => BiomeSourceKind::end(seed),
        }
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
