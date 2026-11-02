//! The seed search panel: conditions that read like their text, and what was found.

use std::time::{Duration, Instant};

use crate::{
    dimension::Dimension,
    gui::Ui,
    seedfind::{
        engine::{Match, biome_dimension},
        run::{Search, Seeds},
        spec::{self, Anchor, BiomeTest, Card, MAX_BLOCKS, NoiseParameter, PRESETS},
    },
    structures::{STRUCTURE_KINDS, is_structure},
};

/// Matches kept in the list, the best ones.
const MAX_RESULTS: usize = 200;
/// How long after the last change a running search starts over, so that changing a value by
/// dragging or typing doesn't restart it all the time.
const RESTART_AFTER: Duration = Duration::from_millis(500);

pub struct SeedSearch {
    cards: Vec<Card>,
    /// the cards as text, which is what can be saved and shared
    text: String,
    /// what is wrong with the text or the cards
    error: Option<String>,
    in_this_seed: bool,
    /// how far locations are looked for, and around where
    area_radius: i32,
    area_center: [i32; 2],
    search: Option<Search>,
    /// the last change that a running search hasn't taken on yet
    changed_at: Option<Instant>,
    results: Vec<Match>,
    print_results: bool,
}

impl Default for SeedSearch {
    fn default() -> Self {
        let text = PRESETS[0].1.to_owned();
        Self {
            cards: spec::parse(&text).unwrap_or_default(),
            text,
            error: None,
            in_this_seed: false,
            area_radius: 10_000,
            area_center: [0, 0],
            search: None,
            changed_at: None,
            results: Vec::new(),
            print_results: false,
        }
    }
}

impl SeedSearch {
    /// Takes the matches found since the last frame. True while there is something to update.
    pub fn poll(&mut self, seed: i64, threads: usize) -> bool {
        if self.changed_at.is_some_and(|at| at.elapsed() >= RESTART_AFTER) {
            self.changed_at = None;
            if self.search.is_some() && self.error.is_none() {
                self.start(seed, threads);
            }
        }
        let Some(search) = &self.search else {
            return self.changed_at.is_some();
        };
        search.set_threads(threads);
        // before taking the matches: all of them have been sent once it is done
        let done = search.is_done();
        let mut sort = false;
        while let Ok(found) = search.results.try_recv() {
            if self.print_results {
                println!("seed search: {} {:?} {:?}", found.seed, found.score, found.hits.first());
            }
            self.results.push(found);
            sort = true;
        }
        if sort {
            match search.center() {
                // the nearest first
                Some(center) => self.results.sort_by_key(|found| {
                    found.hits.first().map_or(i64::MAX, |hit| {
                        let (dx, dz) = ((hit.x - center[0]) as i64, (hit.z - center[1]) as i64);
                        dx * dx + dz * dz
                    })
                }),
                // the best first
                None => self
                    .results
                    .sort_by(|a, b| b.score.unwrap_or(f64::MIN).total_cmp(&a.score.unwrap_or(f64::MIN))),
            }
            self.results.truncate(MAX_RESULTS);
        }
        !done || self.changed_at.is_some()
    }

    fn start(&mut self, seed: i64, threads: usize) {
        let seeds = if self.in_this_seed {
            Seeds::Locations {
                seed,
                radius: self.area_radius,
                center: self.area_center,
            }
        } else {
            Seeds::Random(crate::seed::random_seed() as u64)
        };
        self.results.clear();
        self.search = None;
        match Search::start(&self.cards, seeds, threads) {
            Ok(search) => self.search = Some(search),
            Err(err) => self.error = Some(err),
        }
    }

    /// `search_seeds` and `search_locations` of a script.
    pub fn script_start(&mut self, text: &str, seed: i64, area: Option<(i32, [i32; 2])>) {
        match spec::parse(text) {
            Ok(cards) => {
                self.cards = cards;
                self.in_this_seed = area.is_some();
                if let Some((radius, center)) = area {
                    (self.area_radius, self.area_center) = (radius, center);
                }
                self.print_results = true;
                self.start(seed, std::thread::available_parallelism().map_or(4, |n| n.get()));
            }
            Err(err) => log::error!("searching: {err}"),
        }
    }

    /// `edit_search` of a script.
    pub fn script_edit(&mut self, text: &str) {
        match spec::parse(text) {
            Ok(cards) => {
                self.cards = cards;
                self.changed();
            }
            Err(err) => log::error!("edit_search: {err}"),
        }
    }

    pub fn script_status(&self) -> String {
        self.search.as_ref().map_or(String::new(), Search::status)
    }

    /// After a change of the cards.
    fn changed(&mut self) {
        self.text = spec::print(&self.cards);
        self.validate();
    }

    fn validate(&mut self) {
        self.error = spec::validate(&self.cards, self.in_this_seed).err();
        self.changed_at = Some(Instant::now());
        if self.error.is_some() {
            self.search = None;
        }
    }

    /// Conditions of the other mode say where in the other way.
    fn set_in_this_seed(&mut self) {
        let in_seed = self.in_this_seed;
        self.cards.iter_mut().for_each(|card| convert_card(card, in_seed));
        self.changed();
    }
}

/// Makes a card fit finding locations `in_seed`, or else seeds.
fn convert_card(card: &mut Card, in_seed: bool) {
    let new_anchor = |anchor: &mut Anchor, spawn_exists: bool| {
        *anchor = match (in_seed, spawn_exists) {
            (true, _) => Anchor::Spot,
            (false, true) => Anchor::Spawn,
            (false, false) => Anchor::At(0, 0),
        };
    };
    match card {
        Card::Structure { kind, anchor, .. } => {
            new_anchor(anchor, STRUCTURE_KINDS[*kind].dimension != Dimension::Nether);
        }
        Card::Cluster { within, anchor, .. } => {
            new_anchor(anchor, false);
            if !in_seed {
                *within = 3000;
            }
        }
        Card::Biome { biome, anchor, .. } => new_anchor(anchor, biome_dimension(biome) != Dimension::Nether),
        Card::Noise { anchor, .. } => new_anchor(anchor, true),
        Card::Not(inner) => convert_card(inner, in_seed),
        Card::Any(cards) => cards.iter_mut().for_each(|card| convert_card(card, in_seed)),
    }
}

fn number_ui<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    speed: f64,
) -> bool {
    // a value from the text stays as it is until it is changed here
    ui.add(
        egui::DragValue::new(value)
            .range(range)
            .clamp_existing_to_range(false)
            .speed(speed),
    )
    .changed()
}

fn anchor_ui(ui: &mut egui::Ui, salt: usize, anchor: &mut Anchor, spawn_exists: bool) -> bool {
    let mut changed = false;
    let name = |anchor: &Anchor| match anchor {
        Anchor::Spawn => "spawn",
        Anchor::At(..) | Anchor::Spot => "x,z",
    };
    egui::ComboBox::from_id_salt(("anchor", salt))
        .selected_text(name(anchor))
        .width(52.0)
        .show_ui(ui, |ui| {
            let options = [Anchor::Spawn, Anchor::At(0, 0)];
            for option in options
                .into_iter()
                .filter(|option| spawn_exists || *option != Anchor::Spawn)
            {
                let same = std::mem::discriminant(anchor) == std::mem::discriminant(&option);
                if ui.selectable_label(same, name(&option)).clicked() && !same {
                    *anchor = option;
                    changed = true;
                }
            }
        });
    if let Anchor::At(x, z) = anchor {
        changed |= position_ui(ui, x, z);
    }
    changed
}

fn position_ui(ui: &mut egui::Ui, x: &mut i32, z: &mut i32) -> bool {
    let mut changed = number_ui(ui, x, -MAX_BLOCKS..=MAX_BLOCKS, 16.0);
    ui.label(",");
    changed |= number_ui(ui, z, -MAX_BLOCKS..=MAX_BLOCKS, 16.0);
    changed
}

/// `within 500 of ...`, or only `within 500` around each spot.
fn within_ui(ui: &mut egui::Ui, salt: usize, radius: &mut i32, anchor: &mut Anchor, spawn_exists: bool) -> bool {
    ui.label("within");
    let mut changed = number_ui(ui, radius, 1..=MAX_BLOCKS, 16.0);
    if *anchor != Anchor::Spot {
        ui.label("of");
        changed |= anchor_ui(ui, salt, anchor, spawn_exists);
    }
    changed
}

fn kind_ui(ui: &mut egui::Ui, salt: usize, kind: &mut usize) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(("kind", salt))
        .selected_text(STRUCTURE_KINDS[*kind].id)
        .width(120.0)
        .show_ui(ui, |ui| {
            for (index, def) in STRUCTURE_KINDS.iter().enumerate() {
                if is_structure(index) {
                    changed |= ui.selectable_value(kind, index, def.id).changed();
                }
            }
        });
    changed
}

/// One card, word for word its text with the values to change. True if it changed.
fn card_ui(ui: &mut egui::Ui, salt: usize, card: &mut Card) -> bool {
    let mut changed = false;
    match card {
        Card::Structure {
            kind,
            count,
            radius,
            anchor,
        } => {
            ui.label("structure");
            changed |= kind_ui(ui, salt, kind);
            ui.label("count>=");
            changed |= number_ui(ui, count, 1..=64, 0.1);
            let spawn_exists = STRUCTURE_KINDS[*kind].dimension != Dimension::Nether;
            changed |= within_ui(ui, salt, radius, anchor, spawn_exists);
        }
        Card::Cluster {
            kind,
            count,
            radius,
            within,
            anchor,
        } => {
            ui.label("cluster");
            changed |= kind_ui(ui, salt, kind);
            ui.label("count>=");
            changed |= number_ui(ui, count, 1..=64, 0.1);
            ui.label("radius");
            changed |= number_ui(ui, radius, 1..=MAX_BLOCKS, 4.0);
            if *anchor != Anchor::Spot {
                let spawn_exists = STRUCTURE_KINDS[*kind].dimension != Dimension::Nether;
                changed |= within_ui(ui, salt, within, anchor, spawn_exists);
            }
        }
        Card::Biome {
            biome,
            test,
            radius,
            anchor,
        } => {
            ui.label("biome");
            egui::ComboBox::from_id_salt(("biome", salt))
                .selected_text(biome.as_str())
                .width(120.0)
                .show_ui(ui, |ui| {
                    let mut names: Vec<&str> = spec::searchable_biomes().collect();
                    names.sort_unstable();
                    for name in names {
                        if ui.selectable_label(biome == name, name).clicked() {
                            *biome = name.to_owned();
                            changed = true;
                        }
                    }
                });
            let name = |test: &BiomeTest| match test {
                BiomeTest::Present => "present",
                BiomeTest::Absent => "absent",
                BiomeTest::Coverage(_) => "coverage>=",
                BiomeTest::Area(_) => "area>=",
            };
            egui::ComboBox::from_id_salt(("test", salt))
                .selected_text(name(test))
                .width(70.0)
                .show_ui(ui, |ui| {
                    for option in [
                        BiomeTest::Present,
                        BiomeTest::Absent,
                        BiomeTest::Coverage(20.0),
                        BiomeTest::Area(1_000_000),
                    ] {
                        let same = std::mem::discriminant(test) == std::mem::discriminant(&option);
                        if ui.selectable_label(same, name(&option)).clicked() && !same {
                            *test = option;
                            changed = true;
                        }
                    }
                });
            match test {
                BiomeTest::Coverage(percent) => {
                    changed |= number_ui(ui, percent, 0.0..=100.0, 1.0);
                    ui.label("%");
                }
                BiomeTest::Area(area) => changed |= number_ui(ui, area, 1..=1_000_000_000, 10_000.0),
                _ => {}
            }
            let spawn_exists = biome_dimension(biome) != Dimension::Nether;
            changed |= within_ui(ui, salt, radius, anchor, spawn_exists);
        }
        Card::Noise {
            parameter,
            min,
            max,
            coverage,
            radius,
            anchor,
        } => {
            ui.label("noise");
            egui::ComboBox::from_id_salt(("noise", salt))
                .selected_text(parameter.name())
                .width(110.0)
                .show_ui(ui, |ui| {
                    for option in NoiseParameter::ALL {
                        changed |= ui.selectable_value(parameter, option, option.name()).changed();
                    }
                });
            changed |= number_ui(ui, min, -2.0..=2.0, 0.01);
            ui.label("..");
            changed |= number_ui(ui, max, -2.0..=2.0, 0.01);
            ui.label("coverage>=");
            changed |= number_ui(ui, coverage, 0.0..=100.0, 1.0);
            ui.label("%");
            changed |= within_ui(ui, salt, radius, anchor, true);
        }
        Card::Not(inner) => changed |= card_ui(ui, salt, inner),
        Card::Any(cards) => {
            ui.label(format!("any of {} conditions, see the text", cards.len()));
        }
    }
    changed
}

impl Ui {
    pub(super) fn seed_search_menu(&mut self, ui: &mut egui::Ui) {
        let seed = self.seed;
        let threads = self.threads;
        let mut pick = None;
        let open = self.seed_search.print_results.then_some(true);
        egui::CollapsingHeader::new("Seed search").open(open).show(ui, |ui| {
            let search = &mut self.seed_search;
            let mut changed = false;
            if ui
                .checkbox(&mut search.in_this_seed, "Find locations in this seed")
                .on_hover_text("Goes through an area of this seed once. Conditions are around each spot in it")
                .changed()
            {
                search.set_in_this_seed();
                search.changed_at = None;
                search.search = None;
                search.results.clear();
            }
            let here = search.in_this_seed;
            if here {
                ui.horizontal(|ui| {
                    ui.label("within");
                    changed |= number_ui(ui, &mut search.area_radius, 1..=MAX_BLOCKS, 16.0);
                    ui.label("of");
                    let [x, z] = &mut search.area_center;
                    changed |= position_ui(ui, x, z);
                })
                .response
                .on_hover_text("Blocks of the dimension the conditions are about");
            } else {
                egui::ComboBox::from_id_salt("presets")
                    .selected_text("Presets")
                    .show_ui(ui, |ui| {
                        for (name, text) in PRESETS {
                            if ui.selectable_label(false, *name).clicked()
                                && let Ok(cards) = spec::parse(text)
                            {
                                search.cards = cards;
                                changed = true;
                            }
                        }
                    });
            }
            let mut remove = None;
            for (index, card) in search.cards.iter_mut().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if ui.small_button("x").on_hover_text("Remove").clicked() {
                        remove = Some(index);
                    }
                    // `any` is a block of lines, which has no `not`
                    if !matches!(card, Card::Any(_)) {
                        let is_not = matches!(card, Card::Not(_));
                        if ui.selectable_label(is_not, "not").clicked() {
                            let taken = std::mem::replace(card, Card::Any(Vec::new()));
                            *card = match taken {
                                Card::Not(inner) => *inner,
                                other => Card::Not(Box::new(other)),
                            };
                            changed = true;
                        }
                    }
                    changed |= card_ui(ui, index, card);
                });
            }
            if let Some(index) = remove {
                search.cards.remove(index);
                changed = true;
            }
            ui.horizontal_wrapped(|ui| {
                let anchor = if here { Anchor::Spot } else { Anchor::Spawn };
                let far = if here { Anchor::Spot } else { Anchor::At(0, 0) };
                let id_of = |id: &str| STRUCTURE_KINDS.iter().position(|kind| kind.id == id).unwrap_or(0);
                for (name, card) in [
                    (
                        "+ structure",
                        Card::Structure {
                            kind: id_of("village"),
                            count: 1,
                            radius: 500,
                            anchor,
                        },
                    ),
                    (
                        "+ cluster",
                        Card::Cluster {
                            kind: id_of("swamp_hut"),
                            count: 2,
                            radius: 128,
                            within: 3000,
                            anchor: far,
                        },
                    ),
                    (
                        "+ biome",
                        Card::Biome {
                            biome: "plains".to_owned(),
                            test: BiomeTest::Present,
                            radius: 500,
                            anchor,
                        },
                    ),
                    (
                        "+ noise",
                        Card::Noise {
                            parameter: NoiseParameter::Continentalness,
                            min: -1.0,
                            max: 1.0,
                            coverage: 0.0,
                            radius: 500,
                            anchor,
                        },
                    ),
                ] {
                    if ui.small_button(name).clicked() {
                        search.cards.push(card);
                        changed = true;
                    }
                }
            });
            if changed {
                search.changed();
            }
            egui::CollapsingHeader::new("As text").show(ui, |ui| {
                let edit = ui.add(
                    egui::TextEdit::multiline(&mut search.text)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
                if edit.changed() {
                    match spec::parse(&search.text) {
                        Ok(cards) => {
                            search.cards = cards;
                            search.validate();
                        }
                        Err(err) => {
                            search.error = Some(err);
                            search.search = None;
                        }
                    }
                }
            });
            if let Some(error) = &search.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }

            let done = search.search.as_ref().is_none_or(Search::is_done);
            ui.horizontal(|ui| {
                if done {
                    let label = if search.search.is_some() {
                        "Search again"
                    } else {
                        "Search"
                    };
                    if ui
                        .add_enabled(search.error.is_none(), egui::Button::new(label))
                        .clicked()
                    {
                        search.changed_at = None;
                        search.start(seed, threads);
                    }
                } else {
                    if let Some(running) = &mut search.search {
                        let paused = running.is_paused();
                        if ui.button(if paused { "Go on" } else { "Pause" }).clicked() {
                            running.set_paused(!paused);
                        }
                    }
                    if ui.button("Stop").clicked() {
                        search.search = None;
                    }
                }
            });
            if let Some(running) = &search.search {
                ui.weak(running.status());
            }
            egui::ScrollArea::vertical()
                .id_salt("seed results")
                .max_height(180.0)
                .show(ui, |ui| {
                    for found in &search.results {
                        // the nearest structure counts as the score below 0
                        let score = found.score.map_or(String::new(), |score| {
                            if score < 0.0 {
                                format!(" ({:.0} blocks away)", -score)
                            } else {
                                format!(" ({score:.0})")
                            }
                        });
                        let label = if search.in_this_seed {
                            found
                                .hits
                                .first()
                                .map_or(String::new(), |hit| format!("x {} z {}", hit.x, hit.z))
                        } else {
                            format!("{}{score}", found.seed)
                        };
                        if ui
                            .selectable_label(false, label)
                            .on_hover_text("Go there and show its markers")
                            .clicked()
                        {
                            pick = Some(found.clone());
                        }
                    }
                });
        });
        if let Some(found) = pick {
            self.open_match(&found);
        }
    }

    /// `open_first_match` of a script.
    pub(super) fn script_open_first_match(&mut self) {
        match self.seed_search.results.first().cloned() {
            Some(found) => self.open_match(&found),
            None => log::error!("no match to open"),
        }
    }

    /// Shows a match on the map with its markers.
    fn open_match(&mut self, found: &Match) {
        let dimension = found.hits.first().map_or(Dimension::Overworld, |hit| hit.dimension);
        if found.seed != self.seed {
            self.seed_text = found.seed.to_string();
            self.set_seed(found.seed);
            self.reset_tiles();
        }
        if dimension != self.dimension {
            self.set_dimension(dimension);
        }
        for kind in found.hits.iter().filter_map(|hit| hit.kind) {
            self.shown_kinds[kind] = true;
        }
        if let Some(hit) = found.hits.first() {
            self.map.camera.center = [hit.x as f64 / 4.0, hit.z as f64 / 4.0];
        }
    }
}
