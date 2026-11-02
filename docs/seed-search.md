# Seed search

The Seed search section of the desktop app and `cubox find` do the same. The web version doesn't have it yet.

`cubox find` tries seeds until it has found ones that pass all your conditions. Each line is one condition:

```sh
cubox find "cluster swamp_hut count>=2 radius 128 within 3000 of 0,0"
cubox find --file search.txt --limit 20
```

Matches go to stdout, one per line: the seed, its score, and the `x,z` of what was found. How it is going goes to
stderr, and after 5 matches an estimate of how rare they are. The order of the conditions doesn't change what passes, the
cheap ones are checked first. It only decides the score, see `--tested`.

## Conditions

| Condition | Passes when |
|---|---|
| `structure village count>=2 within 500 of spawn` | at least 2 are that close |
| `cluster swamp_hut count>=2 radius 128 within 3000 of 0,0` | that many fit in one circle of the radius, somewhere in the area. A radius of 128 means one spot is within 128 blocks of them all, which is what a farm needs |
| `biome mushroom_fields area>=1000000 within 5000 of spawn` | one connected patch that starts in the area has that many square blocks |
| `biome jungle coverage>=20% within 1000 of spawn` | it covers that share of the area |
| `biome ocean present within 300 of spawn`, `absent` | it is or isn't in the area |
| `noise continentalness -2..-1.05 coverage>=30% within 1000 of spawn` | the overworld's value is in the range, for that share of the area or just anywhere without `coverage` |
| `not <condition>` | the condition doesn't pass |
| `any`, conditions on their own lines, `end` | one of them passes |

Areas are around `spawn` or a position `x,z`, in blocks of the dimension the condition is about. Spawn is the world's
spawn in the overworld and the platform at 100,0 in the end. The nether has none, name a position there. Noise parameters are `temperature`, `humidity`,
`continentalness`, `erosion`, `weirdness` and `depth`. Structures have the names of their icons in `assets/icons`, among them `copper_vein`, `iron_vein`, `end_gateway` (the 20 the dragon opens), and `geode` and `end_gateway_scattered`, which are maybe there: their random numbers are exact, but terrain the game looks at can still stop them,
biomes their names in the game. `#` starts a comment.

Biomes are looked at from above, at y 256 in the overworld, so caves can't be searched for: `deep_dark`, `dripstone_caves`,
`lush_caves`, `sulfur_caves` and `ice_caves` are refused, and so is `the_void`, which doesn't generate. Noise values are
also those at y 256, which matters for `depth`.

## Options

- `--limit 10`: stop after this many matches. Without it, 10 seeds or all locations.
- `--tested 100000 --top 10`: cover this many seeds, then show the best ones, by the score of the first condition
  that has one: the biggest area or share, the most in a cluster, the nearest structure (its distance, below 0 so that more is better).
- `--from 0`: count up from a seed instead of trying random ones. `--seeds file` tries the seeds of a file, one per
  line, numbers or text.
- `--threads 4`: use fewer threads.
- `--in-seed 12345`: find locations in one seed instead of seeds, see below.

## Finding locations in one seed

With `--in-seed` or "Find locations in this seed" you search one seed for places. Where to look is a setting, and
conditions have no `of`: they are around each spot of that area, and a cluster only has its radius.

```sh
cubox find --in-seed 12345 --within 10000 --of 0,0 "structure village count>=1 within 200
biome desert present within 300"
```

`--within` is 10000 and `--of` 0,0 if you leave them out. It goes through the area once, reports each place once and
then ends. Spots are as far apart as the smallest distance allows, so nothing is missed. The area is in blocks of the
dimension the conditions are about, a nether area is in nether blocks.

## How accurate it is

Structures are only reported if they really generate, as on the map. Biomes are sampled on a grid that gets coarser
with the size of the area, so a small patch in a big area can be missed, and an area is about right, not exact.
Spawn is the point the game starts looking for a spawn from, it then moves it within about 80 blocks.

Random seeds use that where structures are depends only on the lower 48 bits of a seed: a seed with no chance turns
down all of those, and up to 1024 seeds are tried for one that has a chance. Those share all their structure positions.
