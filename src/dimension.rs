//! The three dimensions of a world.

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

    /// The name the game's commands know it by.
    fn game_id(self) -> &'static str {
        match self {
            Dimension::Overworld => "minecraft:overworld",
            Dimension::Nether => "minecraft:the_nether",
            Dimension::End => "minecraft:the_end",
        }
    }

    /// The command that takes a player to a block of this dimension, from any dimension.
    pub fn teleport_command(self, [x, z]: [i32; 2], y: i32) -> String {
        format!("/execute in {} run tp @p {x} {y} {z}", self.game_id())
    }

    /// Where a nether portal at a block leads, before the game looks for a portal or room nearby:
    /// its position in the middle of the block, scaled by 8 and rounded down.
    pub fn portal_exit(self, [x, z]: [i32; 2]) -> Option<(Dimension, [i32; 2])> {
        match self {
            Dimension::Overworld => Some((Dimension::Nether, [x.div_euclid(8), z.div_euclid(8)])),
            Dimension::Nether => Some((Dimension::Overworld, [x * 8 + 4, z * 8 + 4])),
            Dimension::End => None,
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
        // same lock forever. So it is built here, before any worker needs it. Browsers have no such
        // workers, and there rayon would make this thread its pool, which `Finder::new` needs it for.
        #[cfg(not(target_arch = "wasm32"))]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_name_the_dimension_like_the_game() {
        assert_eq!(
            Dimension::Nether.teleport_command([-5, 12], 64),
            "/execute in minecraft:the_nether run tp @p -5 64 12"
        );
        assert!(Dimension::End.teleport_command([0, 0], 0).contains("minecraft:the_end"));
    }

    #[test]
    fn portals_scale_by_8_and_round_down() {
        let exit = |dimension: Dimension, block| dimension.portal_exit(block).unwrap();
        assert_eq!(exit(Dimension::Overworld, [800, 15]), (Dimension::Nether, [100, 1]));
        assert_eq!(exit(Dimension::Overworld, [-1, -9]), (Dimension::Nether, [-1, -2]));
        assert_eq!(exit(Dimension::Nether, [100, -1]), (Dimension::Overworld, [804, -4]));
        assert_eq!(Dimension::End.portal_exit([1, 1]), None);
        // and back to the same block of the nether
        assert_eq!(
            exit(Dimension::Overworld, exit(Dimension::Nether, [-7, 3]).1).1,
            [-7, 3]
        );
    }
}
