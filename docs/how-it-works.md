# How it works

The map is a pyramid of 128×128 px tiles: level *n* has 2<sup>n</sup> quarts per pixel. The viewer shows the level
whose pixels are closest to screen pixels and generates missing tiles in the background. A coarser tile is built from
four finer ones when they exist, which costs no sampling at all.

Each pixel averages several biome samples spread over its area. Up close that is the exact biome map. Far out a
pixel covers thousands of biomes and its few samples are mostly noise, so shown as is the map would look like
confetti. cubox measures, per world and zoom level, how much the true area averages vary and how far the samples are
off, and pulls the samples toward the world's average color just enough that the map varies as much as the real
averages do. That's why zooming out ends in a uniform blue-green: the whole world averaged.
