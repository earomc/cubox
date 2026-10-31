//! What a search looks for, as cards and as the text they are saved as, one card per line:
//!
//! ```text
//! structure village count>=2 within 500 of spawn
//! cluster swamp_hut count>=2 radius 128 within 3000 of 0,0
//! biome mushroom_fields area>=4000000 within 5000 of spawn
//! biome ocean absent within 300 of spawn
//! biome jungle coverage>=20% within 1000 of here
//! noise continentalness -1.2..-1.05 coverage>=5% within 1000 of spawn
//! not structure stronghold count>=1 within 2000 of spawn
//! any
//!   biome desert present within 500 of spawn
//!   biome badlands present within 500 of spawn
//! end
//! ```

use std::fmt;

use crate::structures::STRUCTURE_KINDS;

/// Where the area of a card is around.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anchor {
    Spawn,
    At(i32, i32),
    /// the spot being tried when searching one seed for locations, the origin otherwise
    Here,
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoiseParameter {
    Temperature,
    Humidity,
    Continentalness,
    Erosion,
    Weirdness,
    Depth,
}

impl NoiseParameter {
    pub const ALL: [Self; 6] = [
        Self::Temperature,
        Self::Humidity,
        Self::Continentalness,
        Self::Erosion,
        Self::Weirdness,
        Self::Depth,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Temperature => "temperature",
            Self::Humidity => "humidity",
            Self::Continentalness => "continentalness",
            Self::Erosion => "erosion",
            Self::Weirdness => "weirdness",
            Self::Depth => "depth",
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Card {
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
    Not(Box<Card>),
    Any(Vec<Card>),
}

fn kind_named(id: &str) -> Result<usize, String> {
    match STRUCTURE_KINDS.iter().position(|kind| kind.id == id) {
        Some(kind) if !crate::structures::is_structure(kind) => Err(format!("{id} is not a structure")),
        Some(kind) => Ok(kind),
        None => Err(format!("no structure called {id}")),
    }
}

/// Reads what follows `within`: `500 of spawn`.
fn area(words: &[&str]) -> Result<(i32, Anchor), String> {
    let [within, radius, of, anchor] = words else {
        return Err(format!(
            "expected `within <blocks> of spawn|here|x,z`, got `{}`",
            words.join(" ")
        ));
    };
    if *within != "within" || *of != "of" {
        return Err(format!(
            "expected `within <blocks> of spawn|here|x,z`, got `{}`",
            words.join(" ")
        ));
    }
    let radius: i32 = radius
        .parse()
        .ok()
        .filter(|radius| (1..=30_000_000).contains(radius))
        .ok_or_else(|| format!("`{radius}` is not a distance of 1 to 30000000 blocks"))?;
    let anchor = match *anchor {
        "spawn" => Anchor::Spawn,
        "here" => Anchor::Here,
        at => {
            let at = at
                .split_once(',')
                .and_then(|(x, z)| Some(Anchor::At(x.parse().ok()?, z.parse().ok()?)));
            at.ok_or_else(|| format!("`{anchor}` is not spawn, here or x,z"))?
        }
    };
    Ok((radius, anchor))
}

fn number_after<T: std::str::FromStr>(word: &str, prefix: &str) -> Result<T, String> {
    word.strip_prefix(prefix)
        .and_then(|number| number.trim_end_matches('%').parse().ok())
        .ok_or_else(|| format!("expected `{prefix}<number>`, got `{word}`"))
}

impl Card {
    fn parse_line(words: &[&str]) -> Result<Self, String> {
        match words {
            ["not", rest @ ..] => Ok(Card::Not(Box::new(Self::parse_line(rest)?))),
            ["structure", kind, count, rest @ ..] => {
                let (radius, anchor) = area(rest)?;
                Ok(Card::Structure {
                    kind: kind_named(kind)?,
                    count: number_after(count, "count>=")?,
                    radius,
                    anchor,
                })
            }
            ["cluster", kind, count, "radius", radius, rest @ ..] => {
                let (within, anchor) = area(rest)?;
                Ok(Card::Cluster {
                    kind: kind_named(kind)?,
                    count: number_after(count, "count>=")?,
                    radius: radius.parse().map_err(|_| format!("`{radius}` is not a distance"))?,
                    within,
                    anchor,
                })
            }
            ["biome", biome, test, rest @ ..] => {
                let (radius, anchor) = area(rest)?;
                let test = match *test {
                    "present" => BiomeTest::Present,
                    "absent" => BiomeTest::Absent,
                    test if test.starts_with("coverage>=") => BiomeTest::Coverage(number_after(test, "coverage>=")?),
                    test => BiomeTest::Area(number_after(test, "area>=")?),
                };
                if !crate::biome_colors::BIOMES.contains(biome) {
                    return Err(format!("no biome called {biome}"));
                }
                Ok(Card::Biome {
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
                    .and_then(|(min, max)| Some((min.parse().ok()?, max.parse().ok()?)))
                    .ok_or_else(|| format!("expected a range like -1.2..-1.05, got `{range}`"))?;
                let (coverage, rest) = match rest {
                    [word, rest @ ..] if word.starts_with("coverage>=") => (number_after(word, "coverage>=")?, rest),
                    rest => (0.0, rest),
                };
                let (radius, anchor) = area(rest)?;
                Ok(Card::Noise {
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
pub fn parse(text: &str) -> Result<Vec<Card>, String> {
    let mut cards = Vec::new();
    let mut any: Option<Vec<Card>> = None;
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
                    return Err(error("`any` needs cards".to_owned()));
                }
                cards.push(Card::Any(group));
            }
            (["end"], None) => return Err(error("`end` without `any`".to_owned())),
            (words, Some(group)) => group.push(Card::parse_line(words).map_err(error)?),
            (words, None) => cards.push(Card::parse_line(words).map_err(error)?),
        }
    }
    if any.is_some() {
        return Err("`any` without `end`".to_owned());
    }
    Ok(cards)
}

impl fmt::Display for Anchor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Anchor::Spawn => write!(f, "spawn"),
            Anchor::Here => write!(f, "here"),
            Anchor::At(x, z) => write!(f, "{x},{z}"),
        }
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Card::Structure {
                kind,
                count,
                radius,
                anchor,
            } => write!(
                f,
                "structure {} count>={count} within {radius} of {anchor}",
                STRUCTURE_KINDS[*kind].id
            ),
            Card::Cluster {
                kind,
                count,
                radius,
                within,
                anchor,
            } => write!(
                f,
                "cluster {} count>={count} radius {radius} within {within} of {anchor}",
                STRUCTURE_KINDS[*kind].id
            ),
            Card::Biome {
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
                write!(f, " within {radius} of {anchor}")
            }
            Card::Noise {
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
                write!(f, " within {radius} of {anchor}")
            }
            Card::Not(card) => write!(f, "not {card}"),
            Card::Any(cards) => {
                writeln!(f, "any")?;
                for card in cards {
                    writeln!(f, "  {card}")?;
                }
                write!(f, "end")
            }
        }
    }
}

/// The text of a search.
pub fn print(cards: &[Card]) -> String {
    cards.iter().map(|card| format!("{card}\n")).collect()
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
        let text = "structure village count>=2 within 500 of spawn
cluster swamp_hut count>=2 radius 128 within 3000 of 0,-5
biome mushroom_fields area>=4000000 within 5000 of here
biome ocean absent within 300 of spawn
biome jungle coverage>=20% within 1000 of here
noise continentalness -1.2..-1.05 coverage>=5% within 1000 of spawn
noise erosion 0..0.5 within 10 of 1,2
not structure stronghold count>=1 within 2000 of spawn
any
  biome desert present within 500 of spawn
  not biome badlands present within 500 of spawn
end
";
        let cards = parse(text).unwrap();
        assert_eq!(print(&cards), text);
        assert_eq!(parse(&print(&cards)).unwrap(), cards);
    }

    #[test]
    fn presets_are_valid() {
        for (name, text) in PRESETS {
            assert!(parse(text).is_ok(), "{name}");
        }
    }

    #[test]
    fn errors_say_where() {
        for (text, wanted) in [
            ("structure nothing count>=1 within 5 of spawn", "no structure"),
            ("structure spawn count>=1 within 5 of spawn", "not a structure"),
            ("\nbiome nothing present within 5 of spawn", "line 2: no biome"),
            ("structure village count>=x within 5 of spawn", "count>="),
            ("structure village count>=1 within 0 of spawn", "not a distance"),
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
