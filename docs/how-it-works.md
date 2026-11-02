# How it works

The map is a pyramid of 128×128 px tiles: level *n* has 2<sup>n</sup> quarts per pixel. The viewer shows the level
whose pixels are closest to screen pixels and generates missing tiles in the background. A coarser tile is built from
four finer ones when they exist, which costs no sampling at all.

Each pixel averages several biome samples spread over its area. Up close that is the exact biome map. Far out a
pixel covers thousands of biomes and its few samples are mostly noise, so shown as is the map would look like
confetti. cubox measures, per world and zoom level, how much the true area averages vary and how far the samples are
off, and pulls the samples toward the world's average color just enough that the map varies as much as the real
averages do. That's why zooming out ends in a uniform blue-green: the whole world averaged. The end's biomes are so
small that it gets there soonest, from level 9 on.

Tiles built from finer ones have more samples and so less error, which each tile keeps track of, and are shaded
accordingly. They vary as much as tiles next to them that were sampled, but with more real structure in it.

## The code

The library has no window of its own in these parts:

| Module | What it does |
|---|---|
| `tile_renderer` | biome tiles, shading and downsampling |
| `terrain` | heights and the hillshade made from them |
| `noise` | climate noises, ore veins and what is solid underground |
| `structures` | the kinds of markers, and where they really generate, using the game's code |
| `features` | geodes and end gateways, from the game's random numbers |
| `seedfind` | seed search: `spec` reads conditions, `engine` checks them, `run` goes through seeds on threads |

`gui` is the viewer. Its work is the same on every platform, only where it runs differs:

- **Tiles:** `tiles` plans what is wanted each frame, `world` does the work, `executor_native` runs it on threads and
  `executor_web` in web workers.
- **Markers:** `finder` asks for markers in square bins of a kind's grid, `finder_native` and `finder_web` answer. Bins
  are coarser the further out you are, so a steady number of real markers fills the screen for little work.
- **Drawing:** `map_view` picks the tiles and stand ins for ones that aren't there yet, `map_renderer` draws them with
  wgpu, `overlays` paints markers and the grid with egui and `menus` and `seed_search` are the settings window.

Browsers have no threads that share memory without special server headers, so each web worker is its own copy of the
wasm module. That is why tile requests and finder jobs are encoded as messages. The seed search isn't compiled there.
