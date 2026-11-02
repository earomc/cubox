//! What a search looks for, as conditions and as the text they are saved as, one condition per line:
//!
//! ```text
//! structure village count>=2 within 500 of spawn
//! cluster swamp_hut count>=2 radius 128 within 3000 of 0,0
//! biome mushroom_fields area>=4000000 within 5000 of spawn
//! biome ocean absent within 300 of spawn
//! noise continentalness -1.2..-1.05 coverage>=5% within 1000 of spawn
//! not structure stronghold count>=1 within 2000 of spawn
//! any
//!   biome desert present within 500 of spawn
//!   biome badlands present within 500 of spawn
//! end
//! ```
//!
//! When finding locations in one seed, the same without `of`, which then is each spot of the area
//! being searched, and clusters only have their radius:
//!
//! ```text
//! structure village count>=1 within 200
//! cluster swamp_hut count>=2 radius 128
//! ```

use std::fmt;

use crate::noise::NoiseParameter;

use crate::{biome_colors::BIOMES, dimension::Dimension, seedfind::engine::biome_dimension, structures::MARKER_KINDS};

/// Where the area of a condition is around.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anchor {
    Spawn,
    At(i32, i32),
    /// the spot being tried when finding locations in one seed
    Spot,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BiomeTest {
    Present,
    Absent,
    /// percent of the area
    Coverage(f32),
    /// one connected patch of at least this many square blocks
    Area(i64),
}

#[derive(Clone, PartialEq, Debug)]
pub enum Condition {
    Structure {
        kind: usize,
        count: u32,
        radius: i32,
        anchor: Anchor,
    },
    /// `count` of a kind that all are within `radius` of one spot, somewhere in the area
    Cluster {
        kind: usize,
        count: u32,
        radius: i32,
        /// the size of the area, unused around `Anchor::Spot`, see `cluster_reach`
        within: i32,
        anchor: Anchor,
    },
    Biome {
        biome: String,
        test: BiomeTest,
        radius: i32,
        anchor: Anchor,
    },
    Noise {
        parameter: NoiseParameter,
        min: f32,
        max: f32,
        /// percent of the area that has to be in the range, 0 means anywhere
        coverage: f32,
        radius: i32,
        anchor: Anchor,
    },
    Not(Box<Condition>),
    Any(Vec<Condition>),
}

/// How far around a spot a cluster of `radius` is looked for. With spots `2 * radius` apart the
/// middle of any cluster is at most `1.42 * radius` from a spot, and its structures `radius` further.
pub fn cluster_reach(radius: i32) -> i32 {
    radius * 5 / 2
}

/// How far apart spots can be for this condition not to miss anything: everything within its radius of
/// some place is then within its radius of a spot.
pub fn spot_spacing(condition: &Condition) -> Option<i32> {
    match condition {
        Condition::Structure { radius, .. } | Condition::Biome { radius, .. } | Condition::Noise { radius, .. } => {
            Some(*radius)
        }
        Condition::Cluster { radius, .. } => Some(radius * 2),
        Condition::Not(condition) => spot_spacing(condition),
        Condition::Any(conditions) => conditions.iter().filter_map(spot_spacing).min(),
    }
}

/// The dimension a condition's place is in, for those about one.
fn dimension_of(condition: &Condition) -> Option<Dimension> {
    match condition {
        Condition::Structure { kind, .. } | Condition::Cluster { kind, .. } => Some(MARKER_KINDS[*kind].dimension),
        Condition::Biome { biome, .. } => Some(biome_dimension(biome)),
        Condition::Noise { .. } => Some(Dimension::Overworld),
        Condition::Not(_) | Condition::Any(_) => None,
    }
}

fn anchor_of(condition: &Condition) -> Option<Anchor> {
    match condition {
        Condition::Structure { anchor, .. }
        | Condition::Cluster { anchor, .. }
        | Condition::Biome { anchor, .. }
        | Condition::Noise { anchor, .. } => Some(*anchor),
        Condition::Not(_) | Condition::Any(_) => None,
    }
}

/// Says what is wrong with a condition for finding seeds, or locations `in_seed`.
fn validate_condition(condition: &Condition, in_seed: bool) -> Result<(), String> {
    match condition {
        Condition::Not(condition) => return validate_condition(condition, in_seed),
        Condition::Any(conditions) => {
            return conditions
                .iter()
                .try_for_each(|condition| validate_condition(condition, in_seed));
        }
        _ => {}
    }
    match anchor_of(condition) {
        Some(Anchor::Spot) if !in_seed => Err(format!("`{condition}` needs `of spawn` or `of x,z`")),
        Some(Anchor::Spawn | Anchor::At(..)) if in_seed => {
            Err(format!("`{condition}`: locations are around each spot, leave out `of`"))
        }
        Some(Anchor::Spawn) if dimension_of(condition) == Some(Dimension::Nether) => {
            Err(format!("`{condition}`: the nether has no spawn, use `of x,z`"))
        }
        _ => Ok(()),
    }
}

/// Says what is wrong with conditions for finding seeds, or locations `in_seed`.
pub fn validate(conditions: &[Condition], in_seed: bool) -> Result<(), String> {
    if conditions.is_empty() {
        return Err("nothing to look for".to_owned());
    }
    conditions
        .iter()
        .try_for_each(|condition| validate_condition(condition, in_seed))
}

/// The biomes a search can look for. It samples at each dimension's default height, above all caves
/// in the overworld, and `the_void` isn't generated.
pub fn searchable_biomes() -> impl Iterator<Item = &'static str> {
    const NEVER_SAMPLED: [&str; 6] = [
        "deep_dark",
        "dripstone_caves",
        "lush_caves",
        "sulfur_caves",
        "ice_caves",
        "the_void",
    ];
    BIOMES.iter().copied().filter(|biome| !NEVER_SAMPLED.contains(biome))
}

fn kind_named(id: &str) -> Result<usize, String> {
    match MARKER_KINDS.iter().position(|kind| kind.id == id) {
        Some(kind) if !crate::structures::is_searchable(kind) => Err(format!("{id} is not a structure")),
        Some(kind) => Ok(kind),
        None => Err(format!("no structure called {id}")),
    }
}

/// The farthest a place can be from the middle of the world, in blocks, and so any distance.
pub const MAX_BLOCKS: i32 = 30_000_000;

fn distance(word: &str) -> Result<i32, String> {
    word.parse()
        .ok()
        .filter(|distance| (1..=MAX_BLOCKS).contains(distance))
        .ok_or_else(|| format!("`{word}` is not a distance of 1 to {MAX_BLOCKS} blocks"))
}

/// A position in the world: `x,z`.
pub fn position(word: &str) -> Result<[i32; 2], String> {
    word.split_once(',')
        .and_then(|(x, z)| Some([x.parse().ok()?, z.parse().ok()?]))
        .filter(|at: &[i32; 2]| at.iter().all(|v| v.abs() <= MAX_BLOCKS))
        .ok_or_else(|| format!("`{word}` is not a position x,z in the world"))
}

/// Reads what follows `within`: `500 of spawn`, or `500` around each spot.
fn area(words: &[&str]) -> Result<(i32, Anchor), String> {
    let anchor = match words {
        ["within", _] => Anchor::Spot,
        ["within", _, "of", "spawn"] => Anchor::Spawn,
        ["within", _, "of", at] => {
            let [x, z] = position(at)?;
            Anchor::At(x, z)
        }
        _ => {
            return Err(format!(
                "expected `within <blocks> of spawn|x,z`, got `{}`",
                words.join(" ")
            ));
        }
    };
    Ok((distance(words[1])?, anchor))
}

fn number_after<T: std::str::FromStr>(word: &str, prefix: &str) -> Result<T, String> {
    word.strip_prefix(prefix)
        .and_then(|number| number.parse().ok())
        .ok_or_else(|| format!("expected `{prefix}<number>`, got `{word}`"))
}

/// `count>=2`, at least 1.
fn count(word: &str) -> Result<u32, String> {
    number_after(word, "count>=")
        .ok()
        .filter(|count| *count >= 1)
        .ok_or_else(|| format!("expected `count>=<at least 1>`, got `{word}`"))
}

/// `coverage>=20%`, the sign is optional.
fn coverage(word: &str) -> Result<f32, String> {
    number_after(word.trim_end_matches('%'), "coverage>=")
        .ok()
        .filter(|percent| (0.0..=100.0).contains(percent))
        .ok_or_else(|| format!("expected `coverage>=<0 to 100>%`, got `{word}`"))
}

impl Condition {
    fn parse_line(words: &[&str]) -> Result<Self, String> {
        match words {
            ["not", rest @ ..] => Ok(Condition::Not(Box::new(Self::parse_line(rest)?))),
            ["structure", kind, count, rest @ ..] => {
                let (radius, anchor) = area(rest)?;
                Ok(Condition::Structure {
                    kind: kind_named(kind)?,
                    count: self::count(count)?,
                    radius,
                    anchor,
                })
            }
            ["cluster", kind, count, "radius", radius] => Ok(Condition::Cluster {
                kind: kind_named(kind)?,
                count: self::count(count)?,
                radius: distance(radius)?,
                within: 0,
                anchor: Anchor::Spot,
            }),
            ["cluster", kind, count, "radius", radius, rest @ ..] => {
                let (within, anchor) = area(rest)?;
                if anchor == Anchor::Spot {
                    return Err("a cluster around each spot only has its radius".to_owned());
                }
                Ok(Condition::Cluster {
                    kind: kind_named(kind)?,
                    count: self::count(count)?,
                    radius: distance(radius)?,
                    within,
                    anchor,
                })
            }
            ["biome", biome, test, rest @ ..] => {
                let (radius, anchor) = area(rest)?;
                let test = match *test {
                    "present" => BiomeTest::Present,
                    "absent" => BiomeTest::Absent,
                    test if test.starts_with("coverage>=") => BiomeTest::Coverage(coverage(test)?),
                    test => BiomeTest::Area(
                        number_after(test, "area>=")
                            .ok()
                            .filter(|area| *area >= 1)
                            .ok_or_else(|| format!("expected `area>=<square blocks>`, got `{test}`"))?,
                    ),
                };
                if !BIOMES.contains(biome) {
                    return Err(format!("no biome called {biome}"));
                }
                if !searchable_biomes().any(|name| name == *biome) {
                    return Err(format!("{biome} can't be searched for, see docs/seed-search.md"));
                }
                Ok(Condition::Biome {
                    biome: biome.to_string(),
                    test,
                    radius,
                    anchor,
                })
            }
            ["noise", parameter, range, rest @ ..] => {
                let parameter = NoiseParameter::ALL
                    .into_iter()
                    .find(|p| p.name() == *parameter)
                    .ok_or_else(|| format!("no noise parameter called {parameter}"))?;
                let (min, max) = range
                    .split_once("..")
                    .and_then(|(min, max)| Some((min.parse::<f32>().ok()?, max.parse::<f32>().ok()?)))
                    .filter(|(min, max)| min <= max)
                    .ok_or_else(|| format!("expected a range like -1.2..-1.05, got `{range}`"))?;
                let (coverage, rest) = match rest {
                    [word, rest @ ..] if word.starts_with("coverage>=") => (coverage(word)?, rest),
                    rest => (0.0, rest),
                };
                let (radius, anchor) = area(rest)?;
                Ok(Condition::Noise {
                    parameter,
                    min,
                    max,
                    coverage,
                    radius,
                    anchor,
                })
            }
            _ => Err(format!("don't know what `{}` means", words.join(" "))),
        }
    }
}

/// Reads a search. Errors say the line and what is wrong with it.
pub fn parse(text: &str) -> Result<Vec<Condition>, String> {
    let mut conditions = Vec::new();
    let mut any: Option<Vec<Condition>> = None;
    for (number, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("");
        let words: Vec<&str> = line.split_whitespace().collect();
        let error = |message: String| format!("line {}: {message}", number + 1);
        match (words.as_slice(), &mut any) {
            ([], _) => {}
            (["any"], None) => any = Some(Vec::new()),
            (["any"], Some(_)) => return Err(error("`any` can't be inside `any`".to_owned())),
            (["end"], Some(_)) => {
                let group = any.take().unwrap_or_default();
                if group.is_empty() {
                    return Err(error("`any` needs conditions".to_owned()));
                }
                conditions.push(Condition::Any(group));
            }
            (["end"], None) => return Err(error("`end` without `any`".to_owned())),
            (words, Some(group)) => group.push(Condition::parse_line(words).map_err(error)?),
            (words, None) => conditions.push(Condition::parse_line(words).map_err(error)?),
        }
    }
    if any.is_some() {
        return Err("`any` without `end`".to_owned());
    }
    Ok(conditions)
}

impl fmt::Display for Anchor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Anchor::Spawn => write!(f, "spawn"),
            Anchor::Spot => Ok(()),
            Anchor::At(x, z) => write!(f, "{x},{z}"),
        }
    }
}

/// `within 500 of spawn`, without `of` around each spot.
struct Within(i32, Anchor);

impl fmt::Display for Within {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "within {}", self.0)?;
        if self.1 != Anchor::Spot {
            write!(f, " of {}", self.1)?;
        }
        Ok(())
    }
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Condition::Structure {
                kind,
                count,
                radius,
                anchor,
            } => write!(
                f,
                "structure {} count>={count} {}",
                MARKER_KINDS[*kind].id,
                Within(*radius, *anchor)
            ),
            Condition::Cluster {
                kind,
                count,
                radius,
                within,
                anchor,
            } => {
                write!(f, "cluster {} count>={count} radius {radius}", MARKER_KINDS[*kind].id)?;
                if *anchor != Anchor::Spot {
                    write!(f, " {}", Within(*within, *anchor))?;
                }
                Ok(())
            }
            Condition::Biome {
                biome,
                test,
                radius,
                anchor,
            } => {
                write!(f, "biome {biome} ")?;
                match test {
                    BiomeTest::Present => write!(f, "present")?,
                    BiomeTest::Absent => write!(f, "absent")?,
                    BiomeTest::Coverage(percent) => write!(f, "coverage>={percent}%")?,
                    BiomeTest::Area(area) => write!(f, "area>={area}")?,
                }
                write!(f, " {}", Within(*radius, *anchor))
            }
            Condition::Noise {
                parameter,
                min,
                max,
                coverage,
                radius,
                anchor,
            } => {
                write!(f, "noise {} {min}..{max}", parameter.name())?;
                if *coverage > 0.0 {
                    write!(f, " coverage>={coverage}%")?;
                }
                write!(f, " {}", Within(*radius, *anchor))
            }
            Condition::Not(condition) => write!(f, "not {condition}"),
            Condition::Any(conditions) => {
                writeln!(f, "any")?;
                for condition in conditions {
                    writeln!(f, "  {condition}")?;
                }
                write!(f, "end")
            }
        }
    }
}

/// The text of a search.
pub fn print(conditions: &[Condition]) -> String {
    conditions.iter().map(|condition| format!("{condition}\n")).collect()
}

/// Ready made searches: name and text.
pub const PRESETS: &[(&str, &str)] = &[
    (
        "Witch farm",
        "cluster swamp_hut count>=2 radius 128 within 3000 of 0,0\n",
    ),
    (
        "Quad hut",
        "cluster swamp_hut count>=4 radius 128 within 10000 of 0,0\n",
    ),
    (
        "Speedrun",
        "structure village count>=1 within 200 of spawn\nstructure ruined_portal count>=1 within 200 of spawn\nstructure stronghold count>=1 within 2000 of spawn\n",
    ),
    (
        "Mushroom island",
        "biome mushroom_fields area>=1000000 within 5000 of spawn\n",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_reads_back_the_same() {
        for text in [
            "structure village count>=2 within 500 of spawn
cluster swamp_hut count>=2 radius 128 within 3000 of 0,-5
biome mushroom_fields area>=4000000 within 5000 of spawn
biome ocean absent within 300 of spawn
biome jungle coverage>=20% within 1000 of 7,7
noise continentalness -1.2..-1.05 coverage>=5% within 1000 of spawn
noise erosion 0..0.5 within 10 of 1,2
not structure stronghold count>=1 within 2000 of spawn
any
  biome desert present within 500 of spawn
  not biome badlands present within 500 of spawn
end
",
            "structure village count>=1 within 200
cluster swamp_hut count>=2 radius 128
not biome desert present within 300
",
        ] {
            let conditions = parse(text).unwrap();
            assert_eq!(print(&conditions), text);
            assert_eq!(parse(&print(&conditions)).unwrap(), conditions);
        }
    }

    #[test]
    fn presets_are_valid() {
        for (name, text) in PRESETS {
            validate(&parse(text).unwrap(), false).unwrap_or_else(|err| panic!("{name}: {err}"));
        }
    }

    #[test]
    fn each_mode_has_one_way_of_saying_where() {
        let check = |text: &str, in_seed| validate(&parse(text).unwrap(), in_seed);
        let spot = "biome desert present within 500";
        let fixed = "biome desert present within 500 of 0,0";
        assert!(check(spot, true).is_ok());
        assert!(check(fixed, false).is_ok());
        assert!(check(spot, false).unwrap_err().contains("needs `of"));
        assert!(check(fixed, true).unwrap_err().contains("leave out"));
        assert!(check(&format!("not {spot}"), false).is_err());
        assert!(check(&format!("any\n{fixed}\nend"), true).is_err());
        assert!(check("", false).is_err());
        assert!(check("", true).is_err());
    }

    #[test]
    fn the_nether_has_no_spawn_but_the_end_does() {
        let check = |text: &str| validate(&parse(text).unwrap(), false);
        assert!(
            check("structure fortress count>=1 within 500 of spawn")
                .unwrap_err()
                .contains("nether")
        );
        assert!(check("biome crimson_forest present within 500 of spawn").is_err());
        assert!(check("not structure bastion count>=1 within 500 of spawn").is_err());
        assert!(check("structure fortress count>=1 within 500 of 8,8").is_ok());
        assert!(check("structure end_city count>=1 within 500 of spawn").is_ok());
        assert!(check("biome the_end present within 500 of spawn").is_ok());
    }

    #[test]
    fn errors_say_where() {
        for (text, wanted) in [
            ("structure nothing count>=1 within 5 of spawn", "no structure"),
            ("structure spawn count>=1 within 5 of spawn", "not a structure"),
            ("\nbiome nothing present within 5 of spawn", "line 2: no biome"),
            ("structure village count>=x within 5 of spawn", "count>="),
            ("structure village count>=1 within 0 of spawn", "not a distance"),
            ("structure village count>=0 within 5 of spawn", "count>="),
            ("structure village count>=3% within 5 of spawn", "count>="),
            ("cluster swamp_hut count>=2 radius -5 within 5 of 0,0", "not a distance"),
            ("cluster swamp_hut count>=2 radius 2147483647", "not a distance"),
            ("cluster swamp_hut count>=2 radius 9 within 5", "only has its radius"),
            ("biome desert present within 5 of 2147483000,0", "not a position"),
            ("biome desert coverage>=200% within 5 of 0,0", "0 to 100"),
            ("biome desert area>=-5 within 5 of 0,0", "area>="),
            ("noise erosion NaN..1 within 5 of 0,0", "range"),
            ("noise erosion 1..-1 within 5 of 0,0", "range"),
            ("biome lush_caves present within 5 of 0,0", "can't be searched"),
            ("biome the_void present within 5 of 0,0", "can't be searched"),
            ("area within 5 of 0,0", "don't know"),
            ("structure village count>=1 near spawn", "within"),
            ("hello", "don't know"),
            ("any\nbiome desert present within 5 of spawn", "without `end`"),
            ("end", "without `any`"),
        ] {
            let error = parse(text).unwrap_err();
            assert!(error.contains(wanted), "{text}: {error}");
        }
    }
}
