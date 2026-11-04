//! Slime chunks: the overworld chunks where slimes spawn underground at any light level.

/// Java's `Random`, as `Random.nextInt(10)` uses it.
const LCG_A: i64 = 0x5_DEEC_E66D;
const LCG_C: i64 = 0xB;
const LCG_MASK: i64 = (1 << 48) - 1;

/// Whether the chunk at chunk coordinates `x`, `z` is a slime chunk, like the game's
/// `WorldgenRandom.seedSlimeChunk(x, z, seed, 987234911).nextInt(10) == 0`. The `int`
/// multiplications wrap at 32 bits as they do in Java.
#[inline]
pub fn is_slime_chunk(seed: i64, x: i32, z: i32) -> bool {
    let mixed = seed
        .wrapping_add(x.wrapping_mul(x).wrapping_mul(4_987_142) as i64)
        .wrapping_add(x.wrapping_mul(5_947_611) as i64)
        .wrapping_add((z.wrapping_mul(z) as i64).wrapping_mul(4_392_871))
        .wrapping_add(z.wrapping_mul(389_711) as i64)
        ^ 987_234_911;
    let mut state = (mixed ^ LCG_A) & LCG_MASK;
    // nextInt(10): 31 bits, again for the 8 values at the very top that would make the last
    // round of 10 incomplete
    loop {
        state = state.wrapping_mul(LCG_A).wrapping_add(LCG_C) & LCG_MASK;
        let bits = (state >> 17) as i32;
        let value = bits % 10;
        if bits.wrapping_sub(value).wrapping_add(9) >= 0 {
            return value == 0;
        }
    }
}

/// The first and last chunk inside the world border, on either axis.
const WORLD_CHUNKS: std::ops::RangeInclusive<i64> = -1_875_000..=1_874_999;

/// The middle of a chunk, in blocks.
pub fn chunk_middle(x: i32, z: i32) -> [i32; 2] {
    [x * 16 + 8, z * 16 + 8]
}

/// The largest whole number whose square is at most `value`.
fn isqrt(value: i64) -> i64 {
    let mut root = (value as f64).sqrt() as i64;
    while root * root > value {
        root -= 1;
    }
    while (root + 1) * (root + 1) <= value {
        root += 1;
    }
    root
}

/// Rows of chunks whose point `offset` blocks into them (both ways) is within `radius` blocks of
/// `center`, and inside the world: each row is its z and the x that are in, in chunks. With an offset
/// of 8 these are the chunks whose middle is in the circle, and with `8 * size` the squares of `size`
/// chunks whose middle is, by their first chunk.
pub fn rows_in_disc(
    center: [i32; 2],
    radius: i32,
    offset: i32,
) -> impl Iterator<Item = (i32, std::ops::RangeInclusive<i32>)> {
    let [cx, cz] = center.map(i64::from);
    let (r, offset) = (i64::from(radius), i64::from(offset));
    let ceil = |v: i64| -(-v).div_euclid(16);
    let clamp = |lo: i64, hi: i64| (lo.max(*WORLD_CHUNKS.start()), hi.min(*WORLD_CHUNKS.end()));
    let (first, last) = clamp(ceil(cz - r - offset), (cz + r - offset).div_euclid(16));
    (first..=last).filter_map(move |z| {
        let dz = z * 16 + offset - cz;
        let half = isqrt(r * r - dz * dz);
        let (lo, hi) = clamp(ceil(cx - half - offset), (cx + half - offset).div_euclid(16));
        (lo <= hi).then_some((z as i32, lo as i32..=hi as i32))
    })
}

/// The middles of the slime chunks whose middle is within `radius` blocks of `center`, nearest first.
/// None if `stopped` said so, which is asked every row.
pub fn slime_in_disc(seed: i64, center: [i32; 2], radius: i32, stopped: &dyn Fn() -> bool) -> Option<Vec<[i32; 2]>> {
    let mut found = Vec::new();
    for (z, xs) in rows_in_disc(center, radius, 8) {
        if stopped() {
            return None;
        }
        found.extend(xs.filter(|&x| is_slime_chunk(seed, x, z)).map(|x| chunk_middle(x, z)));
    }
    let distance = |p: &[i32; 2]| {
        let (dx, dz) = (i64::from(p[0] - center[0]), i64::from(p[1] - center[1]));
        dx * dx + dz * dz
    };
    found.sort_by_key(distance);
    Some(found)
}

/// A square of chunks and how many of them are slime chunks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Square {
    /// its chunk with the smallest x and z
    pub first: [i32; 2],
    /// chunks per side
    pub size: i32,
    pub count: u32,
}

impl Square {
    /// Its middle in blocks.
    pub fn middle(&self) -> [i32; 2] {
        self.first.map(|chunk| chunk * 16 + 8 * self.size)
    }

    /// The middles of its slime chunks.
    pub fn slime_chunks(&self, seed: i64) -> Vec<[i32; 2]> {
        let [x0, z0] = self.first;
        (z0..z0 + self.size)
            .flat_map(|z| (x0..x0 + self.size).map(move |x| (x, z)))
            .filter(|&(x, z)| is_slime_chunk(seed, x, z))
            .map(|(x, z)| chunk_middle(x, z))
            .collect()
    }
}

/// Counts the slime chunks of the squares of `size` chunks per side whose first chunk is in row `z`
/// and column `xs`, and calls `found` with each. Each chunk is only looked at `size` times.
pub fn squares_in_row(seed: i64, size: i32, z: i32, xs: std::ops::RangeInclusive<i32>, found: &mut dyn FnMut(Square)) {
    let (first, last) = (*xs.start(), *xs.end());
    if first > last {
        return;
    }
    // slime chunks per column of the rows of the squares, from the first square's first column
    // to the last one's last
    let mut columns = vec![0u32; (last - first + size) as usize];
    for row in z..z + size {
        for (i, column) in columns.iter_mut().enumerate() {
            *column += u32::from(is_slime_chunk(seed, first + i as i32, row));
        }
    }
    let size = size as usize;
    let mut count: u32 = columns[..size - 1].iter().sum();
    for (i, x) in (first..=last).enumerate() {
        count += columns[i + size - 1];
        found(Square {
            first: [x, z],
            size: size as i32,
            count,
        });
        count -= columns[i];
    }
}

/// The square of `size` chunks per side with the most slime chunks, of those whose middle is within
/// `radius` blocks of `center`. The first of them in rows on a tie. None if there is no such square or
/// `stopped` said so, which is asked every row.
pub fn best_square(seed: i64, size: i32, center: [i32; 2], radius: i32, stopped: &dyn Fn() -> bool) -> Option<Square> {
    let full = (size * size) as u32;
    let mut best: Option<Square> = None;
    for (z, xs) in rows_in_disc(center, radius, 8 * size) {
        if stopped() {
            return None;
        }
        squares_in_row(seed, size, z, xs, &mut |square| {
            if best.is_none_or(|best| square.count > best.count) {
                best = Some(square);
            }
        });
        if best.is_some_and(|best| best.count == full) {
            break;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use steel_utils::random::{Random, legacy_random::LegacyRandom};

    use super::*;

    /// The game's way, with SteelMC's port of Java's `Random`.
    fn reference(seed: i64, x: i32, z: i32) -> bool {
        let mixed = seed
            .wrapping_add(i64::from(x.wrapping_mul(x).wrapping_mul(0x4c_1906)))
            .wrapping_add(i64::from(x.wrapping_mul(0x5a_c0db)))
            .wrapping_add(i64::from(z.wrapping_mul(z)).wrapping_mul(0x43_07a7))
            .wrapping_add(i64::from(z.wrapping_mul(0x5_f24f)))
            ^ 0x3ad8_025f;
        LegacyRandom::from_seed(mixed as u64).next_i32_bounded(10) == 0
    }

    #[test]
    fn agrees_with_javas_random() {
        for seed in [0, 1, -1, 12345, -4172144997902289642, i64::MAX, i64::MIN] {
            for z in (-300..300).chain([-1_875_000, 1_874_999, i32::MIN, i32::MAX]) {
                for x in (-300..300).chain([-1_875_000, 1_874_999, i32::MIN, i32::MAX]) {
                    assert_eq!(
                        is_slime_chunk(seed, x, z),
                        reference(seed, x, z),
                        "seed {seed} chunk {x} {z}"
                    );
                }
            }
        }
    }

    /// The 8 values of 31 bits nextInt(10) draws again for: the inline version has to as well.
    #[test]
    fn redraws_like_java() {
        // a state whose next 31 bits are 2^31 - 1, found by stepping the LCG back
        let wanted_bits = (i32::MAX as i64) << 17;
        let inverse_a: i64 = 0xDFE0_5BCB_1365; // LCG_A * this = 1 mod 2^48
        let previous = (wanted_bits - LCG_C).wrapping_mul(inverse_a) & LCG_MASK;
        let mut random = LegacyRandom::from_seed((previous ^ LCG_A) as u64);
        let expected = random.next_i32_bounded(10) == 0;
        // the seed the slime formula has to produce for that state, with x = z = 0
        let seed = previous ^ LCG_A ^ 987_234_911;
        let mut check = LegacyRandom::from_seed((previous ^ LCG_A) as u64);
        assert_eq!(
            check.next_i32() as u32 >> 1,
            i32::MAX as u32,
            "the state isn't one that redraws"
        );
        assert_eq!(is_slime_chunk(seed, 0, 0), expected);
        assert_eq!(reference(seed, 0, 0), expected);
    }

    #[test]
    fn about_one_in_ten() {
        let count = (0..1000)
            .flat_map(|z| (0..1000).map(move |x| (x, z)))
            .filter(|&(x, z)| is_slime_chunk(42, x, z))
            .count();
        assert!((95_000..105_000).contains(&count), "{count}");
    }

    #[test]
    fn discs_hold_the_chunks_whose_middle_is_in_them() {
        for (center, radius) in [([0, 0], 100), ([-37, 1003], 250), ([5, 5], 8), ([5, 5], 1)] {
            let from_rows: Vec<[i32; 2]> = rows_in_disc(center, radius, 8)
                .flat_map(|(z, xs)| xs.map(move |x| [x, z]))
                .collect();
            let mut direct = Vec::new();
            for z in -100..100 {
                for x in -100..100 {
                    let [mx, mz] = chunk_middle(x, z);
                    let (dx, dz) = (i64::from(mx - center[0]), i64::from(mz - center[1]));
                    if dx * dx + dz * dz <= i64::from(radius) * i64::from(radius) {
                        direct.push([x, z]);
                    }
                }
            }
            direct.sort_by_key(|p| (p[1], p[0]));
            assert_eq!(from_rows, direct, "{center:?} {radius}");
        }
    }

    #[test]
    fn discs_stop_at_the_world_border() {
        let rows: Vec<_> = rows_in_disc([29_999_990, 0], 1000, 8).collect();
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|(_, xs)| *xs.end() == 1_874_999));
    }

    #[test]
    fn squares_count_like_looking_at_each_chunk() {
        let seed = 99;
        for size in 1..=4 {
            let mut squares = Vec::new();
            squares_in_row(seed, size, -7, -40..=40, &mut |square| squares.push(square));
            assert_eq!(squares.len(), 81);
            for square in squares {
                assert_eq!(square.slime_chunks(seed).len() as u32, square.count, "{square:?}");
            }
        }
    }

    #[test]
    fn the_best_square_has_the_most() {
        let seed = 4;
        let best = best_square(seed, 2, [0, 0], 1500, &|| false).unwrap();
        let mut most = 0;
        for (z, xs) in rows_in_disc([0, 0], 1500, 16) {
            squares_in_row(seed, 2, z, xs, &mut |square| most = most.max(square.count));
        }
        assert_eq!(best.count, most);
        // full ones are 1 in 10000 squares, there are about 200000 here
        let full = best_square(seed, 2, [0, 0], 4000, &|| false).unwrap();
        assert_eq!(full.count, 4);
        assert_eq!(full.slime_chunks(seed).len(), 4);
        let [mx, mz] = full.middle();
        assert!(i64::from(mx).pow(2) + i64::from(mz).pow(2) <= 4000 * 4000);
    }

    #[test]
    fn slime_chunks_in_a_disc_are_nearest_first() {
        let found = slime_in_disc(1, [100, 100], 600, &|| false).unwrap();
        assert!(found.len() > 30);
        let distance = |p: &[i32; 2]| (p[0] - 100).pow(2) + (p[1] - 100).pow(2);
        assert!(found.windows(2).all(|pair| distance(&pair[0]) <= distance(&pair[1])));
        assert!(found.iter().all(|p| distance(p) <= 600 * 600));
        assert!(slime_in_disc(1, [100, 100], 600, &|| true).is_none());
    }
}
