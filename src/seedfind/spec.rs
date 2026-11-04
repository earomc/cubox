//! What searches look for, as text, one line each. Finding seeds, every condition says where:
//!
//! ```text
//! poi village count>=2 within 500 of spawn
//! cluster swamp_hut count>=2 radius 128 within 3000 of 0,0
//! biome mushroom_fields area>=4000000 within 5000 of spawn
//! not biome ocean within 300 of spawn
//! noise continentalness -1.2..-1.05 coverage>=5% within 1000 of spawn
//! slime count>=3 within 100 of 0,0
//! cluster slime count>=4 square 2 within 1000 of 0,0
//! any
//!   biome desert within 500 of spawn
//!   biome badlands within 500 of spawn
//! end
//! ```
//!
//! Finding locations in one seed, the first line is what to find, each result is one of those. The
//! other lines are conditions around it, so they have no `of`:
//!
//! ```text
//! poi village
//! biome desert within 300
//! not poi pillager_outpost within 500
//! ```

use std::fmt;

use crate::noise::NoiseParameter;

use crate::{biome_colors::BIOMES, dimension::Dimension, seedfind::engine::biome_dimension, structures::MARKER_KINDS};

/// Where the area of a condition is around.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anchor {
    Spawn,
    At(i32, i32),
    /// what was found, when finding locations in one seed
    Target,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BiomeTest {
    /// anywhere in the area
    Present,
    /// percent of the area
    Coverage(f32),
    /// one connected patch of at least this many square blocks
    Area(i64),
}

/// Squares of `size` chunks per side with at least `count` slime chunks in them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SlimeSquare {
    pub size: i32,
    pub count: u32,
}

/// The largest square of chunks a slime cluster can be about.
pub const MAX_SLIME_SQUARE: i32 = 16;

/// What a location search finds, each result being one of these.
#[derive(Clone, PartialEq, Debug)]
pub enum Target {
    Poi {
        kind: usize,
    },
    /// each group of `count` of a kind that fits in a circle of `radius`
    Cluster {
        kind: usize,
        count: u32,
        radius: i32,
    },
    /// each connected patch of a biome, of at least this many square blocks
    Biome {
        biome: String,
        min_area: Option<i64>,
    },
    /// each connected region where a noise is in a range, of at least this many square blocks
    Noise {
        parameter: NoiseParameter,
        min: f32,
        max: f32,
        min_area: Option<i64>,
    },
    /// each slime chunk
    Slime,
    /// each square with that many slime chunks
    SlimeCluster(SlimeSquare),
}

impl Target {
    /// The dimension of what is found.
    pub fn dimension(&self) -> Dimension {
        match self {
            Target::Poi { kind } | Target::Cluster { kind, .. } => MARKER_KINDS[*kind].dimension,
            Target::Biome { biome, .. } => biome_dimension(biome),
            Target::Noise { .. } | Target::Slime | Target::SlimeCluster(_) => Dimension::Overworld,
        }
    }
}

/// What to find in one seed, and what has to be around each of them.
#[derive(Clone, PartialEq, Debug)]
pub struct LocationSearch {
    pub target: Target,
    pub conditions: Vec<Condition>,
}

/// How far apart biomes and noises are sampled by default, in blocks. Biomes exist at this
/// resolution, so nothing is missed.
pub const EXACT_STEP: i32 = 4;

#[derive(Clone, PartialEq, Debug)]
pub enum Condition {
    /// `count` of a kind in the area
    Poi {
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
        /// how far the area reaches
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
    /// `count` slime chunks with their middle in the area
    Slime {
        count: u32,
        radius: i32,
        anchor: Anchor,
    },
    /// a square of chunks with that many slime chunks, whose middle is in the area
    SlimeCluster {
        square: SlimeSquare,
        within: i32,
        anchor: Anchor,
    },
    Not(Box<Condition>),
    Any(Vec<Condition>),
}

impl Condition {
    /// Whether this is about slime chunks, or has a condition inside that is.
    pub fn mentions_slime(&self) -> bool {
        match self {
            Condition::Slime { .. } | Condition::SlimeCluster { .. } => true,
            Condition::Not(condition) => condition.mentions_slime(),
            Condition::Any(conditions) => conditions.iter().any(Condition::mentions_slime),
            _ => false,
        }
    }
}

/// The dimension a condition's place is in, for those about one.
fn dimension_of(condition: &Condition) -> Option<Dimension> {
    match condition {
        Condition::Poi { kind, .. } | Condition::Cluster { kind, .. } => Some(MARKER_KINDS[*kind].dimension),
        Condition::Biome { biome, .. } => Some(biome_dimension(biome)),
        Condition::Noise { .. } | Condition::Slime { .. } | Condition::SlimeCluster { .. } => {
            Some(Dimension::Overworld)
        }
        Condition::Not(_) | Condition::Any(_) => None,
    }
}

fn anchor_of(condition: &Condition) -> Option<Anchor> {
    match condition {
        Condition::Poi { anchor, .. }
        | Condition::Cluster { anchor, .. }
        | Condition::Biome { anchor, .. }
        | Condition::Noise { anchor, .. }
        | Condition::Slime { anchor, .. }
        | Condition::SlimeCluster { anchor, .. } => Some(*anchor),
        Condition::Not(_) | Condition::Any(_) => None,
    }
}

/// Says what is wrong with a condition, for one around what was found or else for finding seeds.
fn validate_condition(condition: &Condition, around_target: bool) -> Result<(), String> {
    match condition {
        Condition::Not(condition) => return validate_condition(condition, around_target),
        Condition::Any(conditions) => {
            return conditions
                .iter()
                .try_for_each(|condition| validate_condition(condition, around_target));
        }
        _ => {}
    }
    match anchor_of(condition) {
        Some(Anchor::Target) if !around_target => Err(format!("`{condition}` needs `of spawn` or `of x,z`")),
        Some(Anchor::Spawn | Anchor::At(..)) if around_target => {
            Err(format!("`{condition}`: this is around what was found, leave out `of`"))
        }
        Some(Anchor::Spawn) if dimension_of(condition) == Some(Dimension::Nether) => {
            Err(format!("`{condition}`: the nether has no spawn, use `of x,z`"))
        }
        _ => Ok(()),
    }
}

/// Says what is wrong with conditions for finding seeds.
pub fn validate_seeds(conditions: &[Condition]) -> Result<(), String> {
    if conditions.is_empty() {
        return Err("nothing to look for".to_owned());
    }
    conditions
        .iter()
        .try_for_each(|condition| validate_condition(condition, false))
}

/// Says what is wrong with a search for locations.
pub fn validate_locations(search: &LocationSearch) -> Result<(), String> {
    search
        .conditions
        .iter()
        .try_for_each(|condition| validate_condition(condition, true))
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
        Some(kind) if !crate::structures::is_searchable(kind) => {
            Err(format!("{id} can't be searched for, it is what `of spawn` means"))
        }
        Some(kind) => Ok(kind),
        None => Err(format!("no poi called {id}")),
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

/// Reads what follows the rest of a condition: `within 500 of spawn`, or `within 500` around what
/// was found.
fn area(words: &[&str]) -> Result<(i32, Anchor), String> {
    let anchor = match words {
        ["within", _] => Anchor::Target,
        ["within", _, "of", "spawn"] => Anchor::Spawn,
        ["within", _, "of", at] => {
            let [x, z] = position(at)?;
            Anchor::At(x, z)
        }
        _ => {
            return Err(format!(
                "expected `within <blocks>` or `within <blocks> of spawn|x,z`, got `{}`",
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

/// `area>=1000000`, at least 1.
fn min_area(word: &str) -> Result<i64, String> {
    number_after(word, "area>=")
        .ok()
        .filter(|area| *area >= 1)
        .ok_or_else(|| format!("expected `area>=<square blocks>`, got `{word}`"))
}

/// `count>=4` and `square 2` of a slime cluster: at least 1, and no more than the square has chunks.
fn slime_square(count: &str, size: &str) -> Result<SlimeSquare, String> {
    let size: i32 = size
        .parse()
        .ok()
        .filter(|size| (1..=MAX_SLIME_SQUARE).contains(size))
        .ok_or_else(|| format!("`square {size}`: a square is 1 to {MAX_SLIME_SQUARE} chunks per side"))?;
    let count = self::count(count)?;
    if count > (size * size) as u32 {
        return Err(format!(
            "a square of {size} by {size} chunks has only {} of them",
            size * size
        ));
    }
    Ok(SlimeSquare { size, count })
}

fn biome_named(biome: &str) -> Result<String, String> {
    if !BIOMES.contains(&biome) {
        return Err(format!("no biome called {biome}"));
    }
    if !searchable_biomes().any(|name| name == biome) {
        return Err(format!("{biome} can't be searched for, see docs/seed-search.md"));
    }
    Ok(biome.to_owned())
}

fn noise_named(parameter: &str) -> Result<NoiseParameter, String> {
    NoiseParameter::ALL
        .into_iter()
        .find(|p| p.name() == parameter)
        .ok_or_else(|| format!("no noise parameter called {parameter}"))
}

fn noise_range(range: &str) -> Result<(f32, f32), String> {
    range
        .split_once("..")
        .and_then(|(min, max)| Some((min.parse::<f32>().ok()?, max.parse::<f32>().ok()?)))
        .filter(|(min, max)| min <= max)
        .ok_or_else(|| format!("expected a range like -1.2..-1.05, got `{range}`"))
}

impl Target {
    fn parse_line(words: &[&str]) -> Result<Self, String> {
        match words {
            ["poi", kind] => Ok(Target::Poi {
                kind: kind_named(kind)?,
            }),
            ["slime"] => Ok(Target::Slime),
            ["cluster", "slime", count, "square", size] => Ok(Target::SlimeCluster(slime_square(count, size)?)),
            ["cluster", kind, count, "radius", radius] => Ok(Target::Cluster {
                kind: kind_named(kind)?,
                count: self::count(count)?,
                radius: distance(radius)?,
            }),
            ["biome", biome] => Ok(Target::Biome {
                biome: biome_named(biome)?,
                min_area: None,
            }),
            ["biome", biome, area] => Ok(Target::Biome {
                biome: biome_named(biome)?,
                min_area: Some(min_area(area)?),
            }),
            ["noise", parameter, range] | ["noise", parameter, range, _] => {
                let (min, max) = noise_range(range)?;
                Ok(Target::Noise {
                    parameter: noise_named(parameter)?,
                    min,
                    max,
                    min_area: words.get(3).map(|area| min_area(area)).transpose()?,
                })
            }
            _ => Err(format!(
                "the first line is what to find, like `poi village` or `biome desert`, got `{}`",
                words.join(" ")
            )),
        }
    }
}

impl Condition {
    fn parse_line(words: &[&str]) -> Result<Self, String> {
        match words {
            ["not", rest @ ..] => Ok(Condition::Not(Box::new(Self::parse_line(rest)?))),
            ["poi", kind, rest @ ..] => {
                let (count, rest) = match rest {
                    [word, rest @ ..] if word.starts_with("count>=") => (self::count(word)?, rest),
                    rest => (1, rest),
                };
                let (radius, anchor) = area(rest)?;
                Ok(Condition::Poi {
                    kind: kind_named(kind)?,
                    count,
                    radius,
                    anchor,
                })
            }
            ["slime", rest @ ..] => {
                let (count, rest) = match rest {
                    [word, rest @ ..] if word.starts_with("count>=") => (self::count(word)?, rest),
                    rest => (1, rest),
                };
                let (radius, anchor) = area(rest)?;
                Ok(Condition::Slime { count, radius, anchor })
            }
            ["cluster", "slime", count, "square", size, rest @ ..] => {
                let (within, anchor) = area(rest)?;
                Ok(Condition::SlimeCluster {
                    square: slime_square(count, size)?,
                    within,
                    anchor,
                })
            }
            ["cluster", kind, count, "radius", radius, rest @ ..] => {
                let (within, anchor) = area(rest)?;
                Ok(Condition::Cluster {
                    kind: kind_named(kind)?,
                    count: self::count(count)?,
                    radius: distance(radius)?,
                    within,
                    anchor,
                })
            }
            ["biome", biome, rest @ ..] => {
                let (test, rest) = match rest {
                    [word, rest @ ..] if word.starts_with("coverage>=") => (BiomeTest::Coverage(coverage(word)?), rest),
                    [word, rest @ ..] if word.starts_with("area>=") => (BiomeTest::Area(min_area(word)?), rest),
                    rest => (BiomeTest::Present, rest),
                };
                let (radius, anchor) = area(rest)?;
                Ok(Condition::Biome {
                    biome: biome_named(biome)?,
                    test,
                    radius,
                    anchor,
                })
            }
            ["noise", parameter, range, rest @ ..] => {
                let parameter = noise_named(parameter)?;
                let (min, max) = noise_range(range)?;
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

/// The words of a line, without its comment.
fn words_of(line: &str) -> Vec<&str> {
    line.split('#').next().unwrap_or("").split_whitespace().collect()
}

/// Reads conditions from the lines after the first `skip`. Errors say the line and what is wrong.
fn parse_conditions(text: &str, skip: usize) -> Result<Vec<Condition>, String> {
    let mut conditions = Vec::new();
    let mut any: Option<Vec<Condition>> = None;
    for (number, line) in text.lines().enumerate().skip(skip) {
        let words = words_of(line);
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

/// Reads a search for seeds.
pub fn parse_seeds(text: &str) -> Result<Vec<Condition>, String> {
    let conditions = parse_conditions(text, 0)?;
    validate_seeds(&conditions)?;
    Ok(conditions)
}

/// Reads a search for locations: the first line that says something is what to find.
pub fn parse_locations(text: &str) -> Result<LocationSearch, String> {
    let (number, words) = text
        .lines()
        .map(words_of)
        .enumerate()
        .find(|(_, words)| !words.is_empty())
        .ok_or("nothing to look for")?;
    let target = Target::parse_line(&words).map_err(|err| format!("line {}: {err}", number + 1))?;
    let search = LocationSearch {
        target,
        conditions: parse_conditions(text, number + 1)?,
    };
    validate_locations(&search)?;
    Ok(search)
}

impl fmt::Display for Anchor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Anchor::Spawn => write!(f, "spawn"),
            Anchor::Target => Ok(()),
            Anchor::At(x, z) => write!(f, "{x},{z}"),
        }
    }
}

/// `within 500 of spawn`, without `of` around what was found.
struct Within(i32, Anchor);

impl fmt::Display for Within {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "within {}", self.0)?;
        if self.1 != Anchor::Target {
            write!(f, " of {}", self.1)?;
        }
        Ok(())
    }
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Condition::Poi {
                kind,
                count,
                radius,
                anchor,
            } => {
                write!(f, "poi {} ", MARKER_KINDS[*kind].id)?;
                if *count != 1 {
                    write!(f, "count>={count} ")?;
                }
                write!(f, "{}", Within(*radius, *anchor))
            }
            Condition::Cluster {
                kind,
                count,
                radius,
                within,
                anchor,
            } => write!(
                f,
                "cluster {} count>={count} radius {radius} {}",
                MARKER_KINDS[*kind].id,
                Within(*within, *anchor)
            ),
            Condition::Biome {
                biome,
                test,
                radius,
                anchor,
            } => {
                write!(f, "biome {biome} ")?;
                match test {
                    BiomeTest::Present => {}
                    BiomeTest::Coverage(percent) => write!(f, "coverage>={percent}% ")?,
                    BiomeTest::Area(area) => write!(f, "area>={area} ")?,
                }
                write!(f, "{}", Within(*radius, *anchor))
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
            Condition::Slime { count, radius, anchor } => {
                write!(f, "slime ")?;
                if *count != 1 {
                    write!(f, "count>={count} ")?;
                }
                write!(f, "{}", Within(*radius, *anchor))
            }
            Condition::SlimeCluster { square, within, anchor } => write!(
                f,
                "cluster slime count>={} square {} {}",
                square.count,
                square.size,
                Within(*within, *anchor)
            ),
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

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Target::Poi { kind } => write!(f, "poi {}", MARKER_KINDS[*kind].id),
            Target::Cluster { kind, count, radius } => {
                write!(f, "cluster {} count>={count} radius {radius}", MARKER_KINDS[*kind].id)
            }
            Target::Biome { biome, min_area } => {
                write!(f, "biome {biome}")?;
                min_area.map_or(Ok(()), |area| write!(f, " area>={area}"))
            }
            Target::Noise {
                parameter,
                min,
                max,
                min_area,
            } => {
                write!(f, "noise {} {min}..{max}", parameter.name())?;
                min_area.map_or(Ok(()), |area| write!(f, " area>={area}"))
            }
            Target::Slime => write!(f, "slime"),
            Target::SlimeCluster(square) => {
                write!(f, "cluster slime count>={} square {}", square.count, square.size)
            }
        }
    }
}

/// The text of a search for seeds.
pub fn print_seeds(conditions: &[Condition]) -> String {
    conditions.iter().map(|condition| format!("{condition}\n")).collect()
}

/// The text of a search for locations.
pub fn print_locations(search: &LocationSearch) -> String {
    format!("{}\n{}", search.target, print_seeds(&search.conditions))
}

/// Ready made searches for seeds: name and text.
pub const SEED_PRESETS: &[(&str, &str)] = &[
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
        "poi village within 200 of spawn\npoi ruined_portal within 200 of spawn\npoi stronghold within 2000 of spawn\n",
    ),
    (
        "Mushroom island",
        "biome mushroom_fields area>=1000000 within 5000 of spawn\n",
    ),
    ("Slime farm", "cluster slime count>=4 square 2 within 300 of 0,0\n"),
];

/// Ready made searches for locations: name and text.
pub const LOCATION_PRESETS: &[(&str, &str)] = &[
    ("Witch farm", "cluster swamp_hut count>=2 radius 128\n"),
    ("Mushroom island", "biome mushroom_fields\n"),
    ("Desert village", "poi village\nbiome desert within 100\n"),
    ("Slime farm", "cluster slime count>=4 square 2\n"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_reads_back_the_same() {
        let seeds = "poi village count>=2 within 500 of spawn
poi igloo within 500 of 3,4
cluster swamp_hut count>=2 radius 128 within 3000 of 0,-5
biome mushroom_fields area>=4000000 within 5000 of spawn
biome jungle coverage>=20% within 1000 of 7,7
noise continentalness -1.2..-1.05 coverage>=5% within 1000 of spawn
noise erosion 0..0.5 within 10 of 1,2
slime within 50 of spawn
slime count>=3 within 100 of 0,0
cluster slime count>=4 square 2 within 1000 of -16,32
not poi stronghold within 2000 of spawn
any
  biome desert within 500 of spawn
  not biome badlands within 500 of spawn
end
";
        let conditions = parse_seeds(seeds).unwrap();
        assert_eq!(print_seeds(&conditions), seeds);
        for locations in [
            "poi village\nbiome desert within 300\nnot poi igloo count>=2 within 500\ncluster igloo count>=2 radius 9 within 50\n",
            "cluster swamp_hut count>=2 radius 128\n",
            "biome desert\n",
            "biome desert area>=5000\nnoise erosion 0..1 coverage>=5% within 9\n",
            "noise erosion -1..0.5 area>=100\n",
            "slime\nbiome swamp within 50\n",
            "cluster slime count>=7 square 3\nnot slime count>=2 within 30\ncluster slime count>=2 square 2 within 64\n",
        ] {
            let search = parse_locations(locations).unwrap();
            assert_eq!(print_locations(&search), locations);
        }
    }

    #[test]
    fn presets_are_valid() {
        for (name, text) in SEED_PRESETS {
            parse_seeds(text).unwrap_or_else(|err| panic!("{name}: {err}"));
        }
        for (name, text) in LOCATION_PRESETS {
            parse_locations(text).unwrap_or_else(|err| panic!("{name}: {err}"));
        }
    }

    #[test]
    fn each_search_has_one_way_of_saying_where() {
        assert!(
            parse_seeds("biome desert within 500")
                .unwrap_err()
                .contains("needs `of")
        );
        assert!(parse_seeds("not biome desert within 500").is_err());
        assert!(parse_seeds("").is_err());
        let error = parse_locations("poi village\nbiome desert within 500 of 0,0").unwrap_err();
        assert!(error.contains("leave out `of`"), "{error}");
        assert!(parse_locations("poi village\nany\nbiome desert within 5 of 0,0\nend").is_err());
        assert!(parse_locations("").is_err());
        // what to find has no distance, that only means something for what is around it
        let error = parse_locations("biome desert within 500").unwrap_err();
        assert!(error.contains("first line"), "{error}");
        assert!(parse_locations("# only a comment\n\npoi village").is_ok());
    }

    #[test]
    fn the_nether_has_no_spawn_but_the_end_does() {
        assert!(
            parse_seeds("poi fortress within 500 of spawn")
                .unwrap_err()
                .contains("nether")
        );
        assert!(parse_seeds("biome crimson_forest within 500 of spawn").is_err());
        assert!(parse_seeds("not poi bastion within 500 of spawn").is_err());
        assert!(parse_seeds("poi fortress within 500 of 8,8").is_ok());
        assert!(parse_seeds("poi end_city within 500 of spawn").is_ok());
        assert!(parse_seeds("biome the_end within 500 of spawn").is_ok());
    }

    #[test]
    fn errors_say_where() {
        for (text, wanted) in [
            ("poi nothing within 5 of spawn", "no poi"),
            ("poi spawn within 5 of spawn", "can't be searched"),
            ("\nbiome nothing within 5 of spawn", "line 2: no biome"),
            ("poi village count>=x within 5 of spawn", "count>="),
            ("poi village within 0 of spawn", "not a distance"),
            ("poi village count>=0 within 5 of spawn", "count>="),
            ("poi village count>=3% within 5 of spawn", "count>="),
            ("cluster swamp_hut count>=2 radius -5 within 5 of 0,0", "not a distance"),
            (
                "cluster swamp_hut count>=2 radius 2147483647 within 5 of 0,0",
                "not a distance",
            ),
            ("biome desert within 5 of 2147483000,0", "not a position"),
            ("biome desert coverage>=200% within 5 of 0,0", "0 to 100"),
            ("biome desert area>=-5 within 5 of 0,0", "area>="),
            ("noise erosion NaN..1 within 5 of 0,0", "range"),
            ("noise erosion 1..-1 within 5 of 0,0", "range"),
            ("biome lush_caves within 5 of 0,0", "can't be searched"),
            ("biome the_void within 5 of 0,0", "can't be searched"),
            ("biome desert present within 5 of 0,0", "expected `within"),
            ("biome desert absent within 5 of 0,0", "expected `within"),
            ("area within 5 of 0,0", "don't know"),
            ("poi village near spawn", "within"),
            ("hello", "don't know"),
            ("any\nbiome desert within 5 of spawn", "without `end`"),
            ("end", "without `any`"),
            ("slime count>=0 within 5 of 0,0", "count>="),
            ("slime within 5", "needs `of"),
            ("cluster slime count>=5 square 2 within 5 of 0,0", "only 4"),
            ("cluster slime count>=1 square 0 within 5 of 0,0", "1 to 16"),
            ("cluster slime count>=1 square 17 within 5 of 0,0", "1 to 16"),
            ("cluster slime count>=4 radius 2 within 5 of 0,0", "no poi called slime"),
        ] {
            let error = parse_seeds(text).unwrap_err();
            assert!(error.contains(wanted), "{text}: {error}");
        }
    }
}
