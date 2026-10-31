# Changelog

All notable changes to cubox are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and cubox follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The release workflow publishes the section of a version as its release notes.

## [Unreleased]

### Added

- Structures on the map: a sub menu with a switch and an icon for every kind, all off by default.
  Their positions come from the game's real generation code.
  End cities with a ship have their own switch and icon. They show at every zoom, all of them until they
  get very dense and a sample beyond that. Markers don't overlap unless you let them, with a
  slider for the space between them: where there is room and a structure it is shown, the rarest kinds first, and free spots keep
  being searched in rounds.
- A search for the nearest structures and biomes.
- A height map of the real terrain when zoomed in, in the overworld, off by default, with a quality
  setting and the sea floor shown under water. Each height is one visible square. It uses the game's own
  quick estimate of the surface, and exact heights can be switched on for when zoomed in.
- + and - zoom the map, and an Advanced menu has more values to adjust.
- A Performance menu with how many threads cubox may use and how much memory finished tiles may take.
- The status line says how many biome tiles, height map tiles and structure areas are being generated.
- A nightly web version of the newest commit on main, at `/nightly/`.
- A distance grid with coordinates and a scale bar, with automatic or fixed spacing, off by default.
- Zoom buttons that can be hidden, and tapping the map shows the biome on touch screens.

### Fixed

- Pinching to zoom didn't work in browsers, and on phones numbers and seeds couldn't be typed.
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

[Unreleased]: https://github.com/earomc/cubox/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/earomc/cubox/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/earomc/cubox/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/earomc/cubox/releases/tag/v0.1.0
