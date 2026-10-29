# Changelog

All notable changes to cubox are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and cubox follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The release workflow publishes the section of a version as its release notes.

## [Unreleased]

## [0.1.0] - 2026-10-29

First release.

### Added

- Interactive map: drag to pan, scroll to zoom, from single quarts out to the whole world.
- Accurate colors at every zoom level: pixels average several biome samples in Oklab, and every
  level is calibrated so the map has the contrast of the real area averages. Zoomed far out the
  world fades into its true average color instead of turning into noise.
- Overworld, nether and end, the biome sample height, samples per pixel, go to coordinates and
  the biome under the cursor.
- Fast multithreaded tile generation, centre first, with quick coarse covers while loading.
- `cubox img` renders a map to a PNG. `cubox gui --script` drives the view and takes screenshots.
- Builds for Linux, macOS and Windows on x86_64 and ARM.

[Unreleased]: https://github.com/earomc/cubox/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/earomc/cubox/releases/tag/v0.1.0
