# Search

There are two searches. The Search section of the desktop app and the command line do the same. The web version doesn't
have them yet.

- **Find seeds** tries seeds until it has found ones where all your conditions hold: `cubox seeds`.
- **Find locations** goes through an area of one seed and lists everything of what you ask for: `cubox locations`.

## Find seeds

Each line is one condition, and says where it is about:

```sh
cubox seeds "poi village count>=2 within 500 of spawn
biome mushroom_fields area>=1000000 within 5000 of spawn"
cubox seeds --file search.txt --limit 20
```

Matches go to stdout, one per line: the seed, its score, and the `x,z` of what was found. How it is going goes to stderr,
and after 5 matches an estimate of how rare they are.

Places are around `spawn` or a position `x,z`, in blocks of the dimension the condition is about. Spawn is the world's
spawn in the overworld and the platform at 100,0 in the end. The nether has none, name a position there. Spawn in the
overworld is the point the game starts looking for a spawn from, it then moves it within about 80 blocks.

- `--limit 10`: stop after this many matches.
- `--tested 100000 --top 10`: cover this many seeds, then show the best ones, by the score of the first condition that
  has one: the biggest area or share, the most in a cluster, the nearest structure (its distance, below 0 so that more is
  better). This is the only thing the order of the conditions changes, the cheap ones are checked first anyway.
- `--from 0`: count up from a seed instead of trying random ones. `--seeds file` tries the seeds of a file, one per line,
  numbers or text.

## Find locations

The first line is what you are looking for, and each result is one of those. The other lines are conditions around it:

```sh
cubox locations 12345 --within 10000 --of 0,0 "poi village
biome desert within 300
not poi pillager_outpost within 500"
```

That finds every village within 10000 blocks of 0,0 that has a desert within 300 blocks of it and no outpost within 500.

| First line | Each result is |
|---|---|
| `poi village` | one of them. All are found, however far apart, and the whole world can be searched |
| `cluster swamp_hut count>=2 radius 128` | one group that many of them fit in a circle of the radius |
| `biome mushroom_fields` | one connected patch, at its middle, with its size in square blocks |
| `biome mushroom_fields area>=1000000` | the same, of at least that size |
| `noise erosion -1..-0.5` | one connected region where the value is in the range, `area>=` works like for biomes |
| `slime` | one slime chunk, at its middle |
| `cluster slime count>=4 square 2` | one square of 2 by 2 chunks with at least 4 slime chunks, at its middle, followed by its slime chunks |

Results go to stdout as they are found: the `x,z` of the result followed by those of what was found around it, and its size or the
number in the group. It goes through the area once and then ends, or stops at `--limit`. `--within` is 10000 and `--of`
0,0 if you leave them out.

The area and all distances are blocks of the dimension of what you look for, so a nether search is in nether blocks.
A patch that reaches out of the area counts with the part inside of it. Parts of a biome that only touch at a corner are
separate results, and islands often have small pieces around them: use `area>=` to see only the big ones.

## Conditions

| Condition | Passes when |
|---|---|
| `poi village within 500 of spawn` | one is that close |
| `poi village count>=2 within 500 of spawn` | at least that many are |
| `cluster swamp_hut count>=2 radius 128 within 3000 of 0,0` | that many fit in one circle of the radius, somewhere in the area. A radius of 128 means one spot is within 128 blocks of them all, which is what a farm needs |
| `biome ocean within 300 of spawn` | it is in the area |
| `biome jungle coverage>=20% within 1000 of spawn` | it covers that share of the area |
| `biome mushroom_fields area>=1000000 within 5000 of spawn` | one connected patch has that many square blocks in the area |
| `noise continentalness -2..-1.05 within 1000 of spawn` | the overworld's value is in the range somewhere in the area |
| `noise continentalness -2..-1.05 coverage>=30% within 1000 of spawn` | for that share of the area |
| `slime within 100 of spawn` | a slime chunk has its middle in the area |
| `slime count>=6 within 100 of 0,0` | at least that many do |
| `cluster slime count>=4 square 2 within 300 of 0,0` | a square of 2 by 2 chunks with its middle in the area has at least 4 slime chunks. A square is 1 to 16 chunks per side, `count>=7 square 3` is 7 of the 9 |
| `not <condition>` | the condition doesn't pass: `not biome ocean within 300 of spawn` |
| `any`, conditions on their own lines, `end` | one of them passes |

When finding locations they have no `of`, as they are around what was found: `biome ocean within 300`.

Noise parameters are `temperature`, `humidity`, `continentalness`, `erosion`, `weirdness` and `depth`. A `poi` is
anything with a marker, named like its icon in `assets/icons`: structures, and also `copper_vein`, `iron_vein`,
`end_gateway` (the 20 the dragon opens), and `geode` and `end_gateway_scattered`, which are maybe there: their random
numbers are exact, but terrain the game looks at can still stop them. Biomes have their names in the game. `#` starts a
comment.

Slime chunks are the overworld chunks where slimes spawn below y 40 at any light level, about one in ten, the same as
"Slime chunks" on the map. They are exact and take nanoseconds each. Like structures they only depend on the lower 48
bits of a seed, so a search for slime chunks around `x,z` turns down all seeds with those bits at once, while one around
spawn has to find spawn first, which needs the biomes.

Biomes are looked at from above, at y 256 in the overworld, so caves can't be searched for: `deep_dark`, `dripstone_caves`,
`lush_caves`, `sulfur_caves` and `ice_caves` are refused, and so is `the_void`, which doesn't generate. Noise values are
also those at y 256, which matters for `depth`.

## How exact it is, and how long it takes

Structures are only reported if they really generate, as on the map.

Biomes and noises are looked at on a grid, by default every 4 blocks. Biomes exist at that resolution, so that is exact
and nothing is missed, whatever the size of the area. It also takes as long as it has to, and you can trade that away with
`--step` (or "Biomes every" in the panel): a step of 16 is 16 times faster, and can miss what is smaller than about 16 by
16 blocks. These are times on one thread, divide by the threads you have:

| | step 4 | step 16 |
|---|---|---|
| A seed that doesn't have the biome, `within 1000` | 0.5 s | 0.08 s |
| The same, `within 5000` | 12 s | 0.75 s |
| The same, `within 10000` | 47 s | 3 s |
| Finding locations, all biomes of an area of radius 10000 | 47 s | 3 s |
| The same, radius 100000 | 80 min | 5 min |

In the end all of this takes about 2.5 times as long. A seed that has what you ask for near the middle stops early, and
structures aren't affected: they are checked first and cost the same at any step, so a search with both only looks at
biomes for the seeds that are left.

Random seeds use that where structures are depends only on the lower 48 bits of a seed: a seed with no chance turns down
all of those, and up to 1024 seeds are tried for one that has a chance. Those share all their structure positions.
