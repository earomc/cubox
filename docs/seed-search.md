# Seed search

`cubox find` tries seeds until it has found ones that pass all your conditions. Each line is one condition:

```sh
cubox find "cluster swamp_hut count>=2 radius 128 within 3000 of 0,0"
cubox find --file search.txt --limit 20
```

Matches go to stdout, one per line: the seed, its score, and the `x,z` of what was found. How it is going goes to
stderr, with an estimate of how rare a match is. The order of the conditions doesn't matter, the cheap ones are
checked first.

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

Areas are around `spawn`, a position `x,z`, or `here`, see below. Noise parameters are `temperature`, `humidity`,
`continentalness`, `erosion`, `weirdness` and `depth`. Structures have the names of their icons in `assets/icons`,
biomes their names in the game. `#` starts a comment.

## Options

- `--limit 10`: stop after this many matches.
- `--tested 100000 --top 10`: cover this many seeds, then show the best ones, by the score of the first condition
  that has one: the biggest area or share, the most in a cluster, the nearest structure.
- `--from 0`: count up from a seed instead of trying random ones. `--seeds file` tries the seeds of a file.
- `--threads 4`: use fewer threads.
- `--in-seed 12345`: find locations in one seed instead of seeds. `here` means the spot being tried, every 512 blocks.

## How accurate it is

Structures are only reported if they really generate, as on the map. Biomes are sampled on a grid that gets coarser
with the size of the area, so a small patch in a big area can be missed, and an area is about right, not exact.
Spawn is the point the game starts looking for a spawn from, it then moves it within about 80 blocks.

Random seeds use that where structures are depends only on the lower 48 bits of a seed: a seed with no chance turns
down all of those, and up to 1024 seeds are tried for one that has a chance. Those share all their structure positions.
