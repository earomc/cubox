const fn rgb(hex: u32) -> [u8; 3] {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

#[inline]
pub fn biome_color(path: &str) -> [u8; 3] {
    rgb(match path {
        // ---- Offshore ----
        "ocean" => 0x000070,
        "deep_ocean" => 0x000030,
        "warm_ocean" => 0x0000AC,
        "lukewarm_ocean" => 0x000090,
        "deep_lukewarm_ocean" => 0x000040,
        "cold_ocean" => 0x202070,
        "deep_cold_ocean" => 0x202038,
        "frozen_ocean" => 0x7070D6,
        "deep_frozen_ocean" => 0x404090,
        "mushroom_fields" => 0xB05AA0,

        // ---- Highland ----
        "jagged_peaks" => 0xDCE4EE,
        "frozen_peaks" => 0xB4C8E6,
        "stony_peaks" => 0x8C8C8C,
        "meadow" => 0x83BB6D,
        "cherry_grove" => 0xF4B6D0,
        "grove" => 0x5E8A6E,
        "snowy_slopes" => 0xE8F0F0,
        "windswept_hills" => 0x606060,
        "windswept_gravelly_hills" => 0x888888,
        "windswept_forest" => 0x507050,

        // ---- Woodland ----
        "forest" => 0x056621,
        "flower_forest" => 0x2D8E49,
        "taiga" => 0x0B6659,
        "old_growth_pine_taiga" => 0x596651,
        "old_growth_spruce_taiga" => 0x818E79,
        "snowy_taiga" => 0x31554A,
        "birch_forest" => 0x307444,
        "old_growth_birch_forest" => 0x589C6C,
        "dark_forest" => 0x40511A,
        "pale_garden" => 0xA8B0A0,
        "jungle" => 0x537B09,
        "sparse_jungle" => 0x628B17,
        "bamboo_jungle" => 0x768E14,
        "dappled_forest" => 0xC86A1E,

        // ---- Wetland ----
        "river" => 0x0000FF,
        "frozen_river" => 0xA0A0FF,
        "swamp" => 0x07F9B2,
        "mangrove_swamp" => 0x67B86A,
        "beach" => 0xFADE55,
        "snowy_beach" => 0xFAF0C0,
        "stony_shore" => 0xA2A284,

        // ---- Flatland ----
        "plains" => 0x8DB360,
        "sunflower_plains" => 0xB5DB88,
        "snowy_plains" => 0xFFFFFF,
        "ice_spikes" => 0xB4DCDC,

        // ---- Arid land ----
        "desert" => 0xFA9418,
        "savanna" => 0xBDB25F,
        "savanna_plateau" => 0xA79D64,
        "windswept_savanna" => 0xE5DA87,
        "badlands" => 0xD94515,
        "wooded_badlands" => 0xB09765,
        "eroded_badlands" => 0xFF6D3D,

        // ---- Caves ----
        "deep_dark" => 0x0A2E3A,
        "dripstone_caves" => 0x8B6F4E,
        "lush_caves" => 0x6FA83C,
        "sulfur_caves" => 0xC8C03A,
        "ice_caves" => 0x9FD8F0,

        // ---- Void ----
        "the_void" => 0x000000,

        // ---- Nether ----
        "nether_wastes" => 0xBF3B3B,
        "soul_sand_valley" => 0x5E3830,
        "crimson_forest" => 0xDD0808,
        "warped_forest" => 0x49907B,
        "basalt_deltas" => 0x403636,

        // ---- End ----
        "the_end" => 0x8080FF,
        "small_end_islands" => 0x9A9AF0,
        "end_midlands" => 0xB8B880,
        "end_highlands" => 0xE0E0A8,
        "end_barrens" => 0x909060,

        _ => 0xFF00FF,
    })
}

/// All biomes. Their place here is the number they have in biome mask tiles, see `biome_number`.
pub const BIOMES: &[&str] = &[
    "ocean",
    "deep_ocean",
    "warm_ocean",
    "lukewarm_ocean",
    "deep_lukewarm_ocean",
    "cold_ocean",
    "deep_cold_ocean",
    "frozen_ocean",
    "deep_frozen_ocean",
    "mushroom_fields",
    "jagged_peaks",
    "frozen_peaks",
    "stony_peaks",
    "meadow",
    "cherry_grove",
    "grove",
    "snowy_slopes",
    "windswept_hills",
    "windswept_gravelly_hills",
    "windswept_forest",
    "forest",
    "flower_forest",
    "taiga",
    "old_growth_pine_taiga",
    "old_growth_spruce_taiga",
    "snowy_taiga",
    "birch_forest",
    "old_growth_birch_forest",
    "dark_forest",
    "pale_garden",
    "jungle",
    "sparse_jungle",
    "bamboo_jungle",
    "dappled_forest",
    "river",
    "frozen_river",
    "swamp",
    "mangrove_swamp",
    "beach",
    "snowy_beach",
    "stony_shore",
    "plains",
    "sunflower_plains",
    "snowy_plains",
    "ice_spikes",
    "desert",
    "savanna",
    "savanna_plateau",
    "windswept_savanna",
    "badlands",
    "wooded_badlands",
    "eroded_badlands",
    "deep_dark",
    "dripstone_caves",
    "lush_caves",
    "sulfur_caves",
    "ice_caves",
    "the_void",
    "nether_wastes",
    "soul_sand_valley",
    "crimson_forest",
    "warped_forest",
    "basalt_deltas",
    "the_end",
    "small_end_islands",
    "end_midlands",
    "end_highlands",
    "end_barrens",
];

/// The number of a biome in mask tiles, `UNKNOWN_BIOME` if it isn't known.
pub fn biome_number(path: &str) -> u8 {
    static NUMBERS: std::sync::LazyLock<std::collections::HashMap<&str, u8>> =
        std::sync::LazyLock::new(|| BIOMES.iter().enumerate().map(|(i, name)| (*name, i as u8)).collect());
    NUMBERS.get(path).copied().unwrap_or(UNKNOWN_BIOME)
}

pub const UNKNOWN_BIOME: u8 = 255;
