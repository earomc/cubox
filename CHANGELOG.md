# Changelog

All notable changes to cubox are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and cubox follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The release workflow publishes the section of a version as its release notes.

## [Unreleased]

### Added

- Slime chunks on the map: "Slime chunks" in the panel, or `slime on` in scripts, in the overworld. They show up the
  moment they are on the screen, until a pixel is bigger than a chunk.

### Changed

- Structure markers show Minecraft textures instead of text labels: a compass for spawn, an ender eye for strongholds,
  a bell for villages and so on. They stay sharp at any size and have a dark and a light outline, so they stand out on
  any biome. The textures belong to Mojang and aren't under cubox's license, see `assets/icons/NOTICE`.
- Exact relief loads about a third faster when zoomed out, from SteelMC's faster chunk generation. Zoomed in it is
  only slightly faster.

### Fixed

- Debug builds of the gui, like `cargo run -- gui`, crashed on the first frame. Release builds weren't affected.

## [0.8.0] - 2026-11-04

### Added

- "Give way to other programs" in the Performance menu and `--low-priority`: searching and generating run at a low
  priority, so that a game in the front runs as if cubox wasn't there. Off by default. A long search could leave other
  programs waiting for half a minute when memory was short.

### Fixed

- A search that matches nearly every seed could keep the window from drawing for good.

### Changed

- Searching for strongholds is much faster: seeds whose rings can't put one close enough are turned down at once.
- Changing a search or searching again keeps what was found, the same result isn't listed twice. Clear results empties
  the list.
- The status of a search shows its numbers exactly, not rounded to two digits.

## [0.7.0] - 2026-11-02

### Changed

- Finding seeds and finding locations are two searches, with two tabs and `cubox seeds` and `cubox locations <seed>`
  instead of `cubox find`. **`cubox find` is gone.**
- A location search's first line is what you are looking for, and each result is one of those: every village, every
  group of witch huts, each connected patch of a biome with its size. The other lines are conditions around it, and
  `within` only means how far from it. Before, results were spots near something, and the number decided how many spots
  were tried.
- Biomes and noises are looked at every 4 blocks everywhere, which is exact and misses nothing, instead of on a grid that
  gets coarser with the area. **Searches with large biome areas take much longer**, `--step` or "Biomes every" in the
  panel trades that back. See `docs/seed-search.md`.
- `area>=` counts the part of a patch inside the area.
- `present` and `absent` are gone: `biome desert within 300`, `not biome desert within 300`. `count>=1` can be left out.
- Stop and Pause work in the middle of a seed.
- Results can be removed from the list, one by one or all at once.

## [0.6.0] - 2026-11-02

### Added

- A menu on the map, with a right click or a long press: copy the location, copy a command that teleports there in the
  right dimension, and go to the coordinates a nether portal there leads to.

### Changed

- Finding locations in one seed has one way of saying where: the area is a setting next to the checkbox, `--within`
  and `--of` with `cubox find`, and conditions have no `of`. **`area` lines and `of here` no longer work**, write
  `poi village count>=1 within 200` and `cluster swamp_hut count>=2 radius 128`.
- `of spawn` means the platform at 100,0 for end conditions, and is refused for nether ones, which have no spawn.
- The Markers menu has a filter and is sorted by name, and its size slider moved there from Extras.
- Samples are per axis in the Extras menu too, like everywhere else.
- Presets are for finding seeds, so they aren't offered while finding locations.
- The search keyword `structure` is now `poi`, as it also finds geodes, ore veins and gateways:
  `poi village count>=1 within 500 of spawn`. **Saved searches need this change.**
- `cubox img --scale` is now `--level`, what the viewer calls it. In scripts `structure` is now `marker`.

### Fixed

- `cubox img -y` takes negative heights.
- `cubox find` refuses what could never match instead of searching forever: cave biomes, which it can't see, and
  impossible values like `coverage>=200%` or a noise range the wrong way round. `count>=0`, a negative `area>=` or
  positions outside the world are refused too.
- `--threads 0` is refused instead of hanging. Flags that exclude each other, like `--from` and `--seeds`, say so.
- `--in-seed` and files for `--seeds` take text seeds.
- Clusters near the middle of two spots aren't missed when finding locations.
- Highlighted biomes flickered while tiles were generated: tiles don't fade in while biomes are highlighted.
- Tiles built from ones further in looked duller than those next to them. They now have the same contrast.
- Far out, detail that isn't real came back at some levels, most of all in the end. Once a level is the plain average
  of the world, so are all coarser ones. In the end that is from level 9 on.
- Text in the window could stay garbled after a frame was skipped.
- Old markers stayed up to half a second after changing the seed.
- Moving the height slider restarted the search for markers.
- Values in the text of a seed search are no longer changed by the panel showing them.
- In the web version a worker that failed to load no longer leaves the map waiting forever.
- `cubox gui` ends with an error code if it fails.

## [0.5.1] - 2026-11-01

### Changed

- Several noises can be shown at once, each with its own opacity, and one slider changes all of them.
- Ore veins are markers at real ore instead of something drawn over the map. The `vein` search condition is gone,
  `structure copper_vein` and `structure iron_vein` do it.
- Geodes and scattered end gateways are also checked against the terrain.
- Zoom buttons are off by default, their switch is in the Advanced menu, now called Extras.
- The settings window can be resized.

## [0.5.0] - 2026-11-01

### Added

- `cubox find` searches for seeds: structures near a spot or spawn, structures close together, biomes with a
  minimum size or share of an area, and noise values. See `docs/seed-search.md`.
- A Seed search section on the desktop with presets, finding locations in one seed too. Clicking a result goes there
  and shows its markers.
- End gateways, in the order the dragon opens them, and geodes and scattered end gateways, which are maybe there, as
  markers and search conditions.
- Large ore veins on the map and as a `vein` search condition.
- Finding locations in one seed goes through an `area` once, reports each place once and ends.
- A Noise section shows temperature, humidity, continentalness, erosion, weirdness or depth as colors over the map.

### Removed

- The Search section, Seed search has taken its place. The web version has no search for now.

## [0.4.2] - 2026-10-31

### Fixed

- Markers appear in the web version again, 0.4.1 broke them.
- Flicker when zooming with highlighted biomes.

## [0.4.1] - 2026-10-31

### Changed

- End city markers are found about 2.7 times as fast, with the same results.

### Fixed

- `cubox img` refuses sizes of 0 and images over 32768 pixels per side with an error, instead of crashing or
  trying to allocate more memory than there is.
- A rare possible deadlock when the first biome sample happened on a tile worker.
- A crash of the gui now ends with its error message instead of a segmentation fault on Linux.

## [0.4.0] - 2026-10-31

### Added

- Highlight biomes: choose any in the Biomes menu and the others are darkened, instantly.
- A button for a random seed.
- The spawn point of a seed as a marker. The Structures menu is now called Markers.
- The distance grid can be drawn over the structure markers.

### Changed

- Samples per pixel moved to the Advanced menu.
- Fixed grid spacing stays the same at every zoom, where lines get too close they fade out.
- Markers may overlap by default. Without overlap their number doesn't drop past seeing the whole world, the
  space between them is set in pixels and can be 0.

## [0.3.0] - 2026-10-31

### Added

- Structures on the map, with real positions from the game's generation code: a sub menu with a switch and
  an icon for every kind, all off by default, including abandoned camps and end cities with a ship.
  - They show at every zoom. Close up that is all of them, far out a steady number of real ones.
  - Markers don't overlap unless you let them. A slider sets the space between them, the rarest kinds come
    first unless you switch that off, and with overlap on a limit sets how many are shown.
- A search for the nearest structures and biomes, which can be cleared again.
- A height map in the overworld, off by default, with the sea floor shown under water. It uses the game's
  quick estimate of the surface, exact heights can be switched on with a quality setting.
- A distance grid with coordinates and a scale bar, with automatic or fixed spacing, off by default.
- A Performance menu for how many threads cubox may use and how much memory finished tiles may take, and an
  Advanced menu with more values to adjust.
- The status line says how many biome tiles, height map tiles and structure areas are being generated.
- Arrow keys pan and + and - zoom the map. Ctrl and + or - scale the interface.
- Zoom buttons that can be hidden, and tapping the map shows the biome on touch screens.
- A nightly web version of the newest commit on main, at `/nightly/`.

### Changed

- Maps now match Minecraft 26.3.
- The settings window shows the license and links to the source code.
- The web download is bigger, about 23 MB, because of the structure data.

### Fixed

- Pinching to zoom didn't work in browsers.
- On phones and tablets numbers and seeds couldn't be typed. A text bar now opens the keyboard.
- The settings window scrolls and starts closed on small screens.

## [0.2.0] - 2026-10-30

### Added

- cubox runs in the browser, generating tiles in web workers on all cores, with WebGPU or WebGL2.
  `?seed=...&dimension=...` in the address picks the world.
- A Docker image that builds the web version and serves it: `docker compose up --build`.
- Releases also deploy the web version to GitHub Pages.

## [0.1.1] - 2026-10-30

### Fixed

- cubox could deadlock at startup, leaving the map black and sometimes the whole window unresponsive.
  The tile workers no longer share a thread pool with the world generator's own parallel setup.
- The window could freeze on Linux ("not responding"): cubox kept drawing frames even when nothing
  changed, and on Wayland a hidden window could block it. It now only draws when something changes,
  lets the compositor pace frames and pauses while hidden.
- The window is named "cubox" on Linux instead of "Unknown".

## [0.1.0] - 2026-10-30

First release.

### Added

- Interactive map: drag to pan, scroll to zoom, from quarts (4×4 blocks) out to the whole world.
- Seeds like in Minecraft: numbers, negative ones too, or any text, hashed like the game does.
- Accurate colors at every zoom level: pixels average several biome samples in Oklab, and every
  level is calibrated so the map has the contrast of the real area averages. Zoomed far out the
  world fades into its true average color instead of turning into noise.
- Overworld, nether and end, the biome sample height, samples per pixel, go to coordinates and
  the biome under the cursor.
- Fast multithreaded tile generation, centre first, with quick coarse covers while loading.
- `cubox img` renders a map to a PNG. `cubox gui --script` drives the view and takes screenshots.
- Builds for Linux, macOS and Windows on x86_64 and ARM.

[Unreleased]: https://github.com/earomc/cubox/compare/v0.8.0...HEAD
[0.8.0]: https://github.com/earomc/cubox/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/earomc/cubox/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/earomc/cubox/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/earomc/cubox/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/earomc/cubox/compare/v0.4.2...v0.5.0
[0.4.2]: https://github.com/earomc/cubox/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/earomc/cubox/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/earomc/cubox/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/earomc/cubox/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/earomc/cubox/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/earomc/cubox/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/earomc/cubox/releases/tag/v0.1.0
