/// Turns the text typed as a seed into a seed, exactly like Minecraft's `WorldOptions.parseSeed`:
/// a number (negative ones included) is used as is, any other text becomes the Java `hashCode`
/// of the string. Returns None for blank text, where Minecraft would pick a random seed.
///
/// Unlike Java's `Long.parseLong`, numbers written with non-ASCII digits (e.g. Arabic-Indic)
/// are treated as text.
pub fn parse_seed(text: &str) -> Option<i64> {
    // Java's String.trim only strips characters up to U+0020, not all unicode whitespace
    let text = text.trim_matches(|c| c <= ' ');
    if text.is_empty() {
        return None;
    }
    Some(text.parse::<i64>().unwrap_or_else(|_| java_string_hash(text) as i64))
}

/// Java's `String.hashCode()`: over UTF-16 code units, wrapping at 32 bits.
fn java_string_hash(text: &str) -> i32 {
    text.encode_utf16()
        .fold(0i32, |hash, unit| hash.wrapping_mul(31).wrapping_add(unit as i32))
}

/// The seed as the biome generator takes it, the same 64 bits.
pub fn generator_seed(seed: i64) -> u64 {
    seed as u64
}

/// A random seed, like Minecraft picks when the seed is left empty.
pub fn random_seed() -> i64 {
    let mut bytes = [0; 8];
    match getrandom::fill(&mut bytes) {
        Ok(()) => i64::from_le_bytes(bytes),
        Err(err) => {
            log::error!("getting a random seed: {err}");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(parse_seed("12345"), Some(12345));
        assert_eq!(parse_seed("-4172144997902289642"), Some(-4172144997902289642));
        assert_eq!(parse_seed("+7"), Some(7));
        assert_eq!(parse_seed("  42\t"), Some(42));
        assert_eq!(parse_seed("9223372036854775807"), Some(i64::MAX));
        assert_eq!(parse_seed("-9223372036854775808"), Some(i64::MIN));
    }

    /// Expected values are from Java's String.hashCode().
    #[test]
    fn text() {
        assert_eq!(parse_seed("test"), Some(3556498));
        assert_eq!(parse_seed("hello"), Some(99162322));
        // the famous string whose hash is Integer.MIN_VALUE: wraps at 32 bits, then sign extends
        assert_eq!(parse_seed("polygenelubricants"), Some(i32::MIN as i64));
        // one past i64::MAX isn't a number to Java either
        assert_eq!(
            parse_seed("9223372036854775808"),
            Some(java_string_hash("9223372036854775808") as i64)
        );
        // UTF-16 code units, not UTF-8 bytes
        assert_eq!(parse_seed("ä"), Some(0xE4));
        assert_eq!(parse_seed("😀"), Some(0xD83D * 31 + 0xDE00));
        // trimmed like Java, which keeps non-breaking spaces
        assert_eq!(parse_seed(" a "), Some('a' as i64));
        assert_eq!(parse_seed("\u{a0}a"), Some(0xA0 * 31 + 'a' as i64));
    }

    #[test]
    fn blank() {
        assert_eq!(parse_seed(""), None);
        assert_eq!(parse_seed("   "), None);
    }

    #[test]
    fn negative_seeds_keep_their_bits() {
        assert_eq!(generator_seed(-1), u64::MAX);
    }

    #[test]
    fn random_seeds_differ_and_can_be_typed_back() {
        let (a, b) = (random_seed(), random_seed());
        assert_ne!(a, b);
        assert_eq!(parse_seed(&a.to_string()), Some(a));
    }
}
