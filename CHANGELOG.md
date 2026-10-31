# Changelog

All notable changes to cubox are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and cubox follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The release workflow publishes the section of a version as its release notes.

## [Unreleased]

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

[Unreleased]: https://github.com/earomc/cubox/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/earomc/cubox/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/earomc/cubox/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/earomc/cubox/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/earomc/cubox/releases/tag/v0.1.0
