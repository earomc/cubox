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

/// The slime chunks of a row of chunks, as runs of neighbours: `(first x, count)`.
pub fn slime_runs(seed: i64, z: i32, xs: std::ops::RangeInclusive<i32>, runs: &mut Vec<(i32, i32)>) {
    let mut run: Option<(i32, i32)> = None;
    for x in xs {
        if is_slime_chunk(seed, x, z) {
            match &mut run {
                Some((_, count)) => *count += 1,
                None => run = Some((x, 1)),
            }
        } else if let Some(done) = run.take() {
            runs.push(done);
        }
    }
    runs.extend(run);
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
    fn runs_cover_exactly_the_slime_chunks() {
        let mut runs = Vec::new();
        for z in -50..50 {
            runs.clear();
            slime_runs(7, z, -80..=80, &mut runs);
            let from_runs: Vec<i32> = runs.iter().flat_map(|&(x, n)| x..x + n).collect();
            let direct: Vec<i32> = (-80..=80).filter(|&x| is_slime_chunk(7, x, z)).collect();
            assert_eq!(from_runs, direct);
        }
    }
}
