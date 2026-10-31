//! The seed search panel: conditions as sentences and text, and the seeds found.

use std::{sync::atomic::Ordering, time::Instant};

use crate::{
    biome_colors::BIOMES,
    dimension::Dimension,
    gui::Ui,
    seedfind::{
        engine::Match,
        run::{Search, Seeds},
        spec::{self, Anchor, BiomeTest, Card, NoiseParameter, PRESETS},
    },
    structures::{STRUCTURE_KINDS, is_structure},
};

/// Matches kept in the list, the best ones.
const MAX_RESULTS: usize = 200;

pub struct SeedSearch {
    cards: Vec<Card>,
    /// the cards as text, which is what can be saved and shared
    text: String,
    text_error: Option<String>,
    in_this_seed: bool,
    search: Option<Running>,
    results: Vec<Match>,
    print_results: bool,
}

struct Running {
    search: Search,
    started: Instant,
    paused: bool,
    /// the seed being searched for locations
    in_seed: Option<i64>,
}

impl Default for SeedSearch {
    fn default() -> Self {
        let text = PRESETS[0].1.to_owned();
        Self {
            cards: spec::parse(&text).unwrap_or_default(),
            text,
            text_error: None,
            in_this_seed: false,
            search: None,
            results: Vec::new(),
            print_results: false,
        }
    }
}

impl SeedSearch {
    /// Takes the matches found since the last frame. True while searching.
    pub fn poll(&mut self, threads: usize) -> bool {
        let Some(running) = &self.search else { return false };
        running.search.set_threads(threads);
        let mut sort = false;
        while let Ok(found) = running.search.results.try_recv() {
            if self.print_results {
                println!("seed search: {} {:?} {:?}", found.seed, found.score, found.hits.first());
            }
            self.results.push(found);
            sort = true;
        }
        if sort {
            // the best first, a location keeps its order: nearest first
            if running.in_seed.is_none() {
                self.results
                    .sort_by(|a, b| b.score.unwrap_or(f64::MIN).total_cmp(&a.score.unwrap_or(f64::MIN)));
            }
            self.results.truncate(MAX_RESULTS);
        }
        true
    }

    fn start(&mut self, seed: i64, threads: usize) {
        let in_seed = self.in_this_seed.then_some(seed);
        let seeds = match in_seed {
            Some(seed) => Seeds::Locations(seed),
            None => Seeds::Random(crate::seed::random_seed() as u64),
        };
        self.results.clear();
        self.search = Some(Running {
            search: Search::start(&self.cards, seeds, threads),
            started: Instant::now(),
            paused: false,
            in_seed,
        });
    }

    /// `search_seeds` of a script.
    pub fn script_start(&mut self, text: &str, seed: i64) {
        match spec::parse(text) {
            Ok(cards) => {
                self.cards = cards;
                self.print_results = true;
                self.start(seed, std::thread::available_parallelism().map_or(4, |n| n.get()));
            }
            Err(err) => log::error!("search_seeds: {err}"),
        }
    }

    fn sync_text(&mut self) {
        self.text = spec::print(&self.cards);
        self.text_error = None;
    }
}

fn anchor_ui(ui: &mut egui::Ui, salt: usize, anchor: &mut Anchor) -> bool {
    let mut changed = false;
    let name = |anchor: &Anchor| match anchor {
        Anchor::Spawn => "spawn",
        Anchor::Here => "here",
        Anchor::At(..) => "x z",
    };
    egui::ComboBox::from_id_salt(("anchor", salt))
        .selected_text(name(anchor))
        .width(52.0)
        .show_ui(ui, |ui| {
            for option in [Anchor::Spawn, Anchor::At(0, 0), Anchor::Here] {
                let same = std::mem::discriminant(anchor) == std::mem::discriminant(&option);
                if ui.selectable_label(same, name(&option)).clicked() && !same {
                    *anchor = option;
                    changed = true;
                }
            }
        })
        .response
        .on_hover_text("What the distance is from. `here` is the spot being tried when searching this seed");
    if let Anchor::At(x, z) = anchor {
        changed |= ui.add(egui::DragValue::new(x).speed(16.0)).changed();
        changed |= ui.add(egui::DragValue::new(z).speed(16.0)).changed();
    }
    changed
}

fn within_ui(ui: &mut egui::Ui, salt: usize, radius: &mut i32, anchor: &mut Anchor) -> bool {
    ui.label("within");
    let mut changed = ui
        .add(egui::DragValue::new(radius).range(1..=30_000_000).speed(16.0))
        .changed();
    ui.label("of");
    changed |= anchor_ui(ui, salt, anchor);
    changed
}

fn kind_ui(ui: &mut egui::Ui, salt: usize, kind: &mut usize) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(("kind", salt))
        .selected_text(STRUCTURE_KINDS[*kind].name)
        .width(120.0)
        .show_ui(ui, |ui| {
            for (index, def) in STRUCTURE_KINDS.iter().enumerate() {
                if is_structure(index) {
                    changed |= ui.selectable_value(kind, index, def.name).changed();
                }
            }
        });
    changed
}

/// One card as a sentence. True if it changed.
fn card_ui(ui: &mut egui::Ui, salt: usize, card: &mut Card) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        match card {
            Card::Structure {
                kind,
                count,
                radius,
                anchor,
            } => {
                changed |= ui.add(egui::DragValue::new(count).range(1..=64)).changed();
                changed |= kind_ui(ui, salt, kind);
                changed |= within_ui(ui, salt, radius, anchor);
            }
            Card::Cluster {
                kind,
                count,
                radius,
                within,
                anchor,
            } => {
                changed |= ui.add(egui::DragValue::new(count).range(2..=16)).changed();
                changed |= kind_ui(ui, salt, kind);
                ui.label("in a circle of");
                changed |= ui
                    .add(egui::DragValue::new(radius).range(1..=10_000).speed(4.0))
                    .changed();
                changed |= within_ui(ui, salt, within, anchor);
            }
            Card::Biome {
                biome,
                test,
                radius,
                anchor,
            } => {
                egui::ComboBox::from_id_salt(("biome", salt))
                    .selected_text(biome.replace('_', " "))
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        let mut names: Vec<&str> = BIOMES.to_vec();
                        names.sort_unstable();
                        for name in names {
                            if ui.selectable_label(biome == name, name.replace('_', " ")).clicked() {
                                *biome = name.to_owned();
                                changed = true;
                            }
                        }
                    });
                let name = match test {
                    BiomeTest::Present => "present",
                    BiomeTest::Absent => "absent",
                    BiomeTest::Coverage(_) => "covers %",
                    BiomeTest::Area(_) => "patch of",
                };
                egui::ComboBox::from_id_salt(("test", salt))
                    .selected_text(name)
                    .width(70.0)
                    .show_ui(ui, |ui| {
                        for (name, option) in [
                            ("present", BiomeTest::Present),
                            ("absent", BiomeTest::Absent),
                            ("covers %", BiomeTest::Coverage(20.0)),
                            ("patch of", BiomeTest::Area(1_000_000)),
                        ] {
                            let same = std::mem::discriminant(test) == std::mem::discriminant(&option);
                            if ui.selectable_label(same, name).clicked() && !same {
                                *test = option;
                                changed = true;
                            }
                        }
                    });
                match test {
                    BiomeTest::Coverage(percent) => {
                        changed |= ui.add(egui::DragValue::new(percent).range(0.0..=100.0)).changed();
                    }
                    BiomeTest::Area(area) => {
                        changed |= ui
                            .add(egui::DragValue::new(area).range(1..=1_000_000_000).speed(10_000.0))
                            .on_hover_text("square blocks")
                            .changed();
                    }
                    _ => {}
                }
                changed |= within_ui(ui, salt, radius, anchor);
            }
            Card::Noise {
                parameter,
                min,
                max,
                coverage,
                radius,
                anchor,
            } => {
                egui::ComboBox::from_id_salt(("noise", salt))
                    .selected_text(parameter.name())
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for option in NoiseParameter::ALL {
                            changed |= ui.selectable_value(parameter, option, option.name()).changed();
                        }
                    });
                ui.label("from");
                changed |= ui
                    .add(egui::DragValue::new(min).speed(0.01).range(-2.0..=2.0))
                    .changed();
                ui.label("to");
                changed |= ui
                    .add(egui::DragValue::new(max).speed(0.01).range(-2.0..=2.0))
                    .changed();
                ui.label("on %");
                changed |= ui
                    .add(egui::DragValue::new(coverage).range(0.0..=100.0))
                    .on_hover_text("How much of the area, 0 means anywhere")
                    .changed();
                changed |= within_ui(ui, salt, radius, anchor);
            }
            Card::Not(inner) => {
                ui.label("not:");
                changed |= card_ui(ui, salt, inner);
            }
            Card::Any(cards) => {
                ui.label(format!("any of {} conditions, change as text", cards.len()));
            }
        }
    });
    changed
}

fn odds_line(running: &Running) -> String {
    let counters = &running.search.counters;
    let covered = counters.covered();
    let rate = covered as f64 / running.started.elapsed().as_secs_f64().max(0.001);
    let odds = match counters.one_in() {
        Some(one_in) => format!("about 1 in {one_in:.0}, one every {:.0} s", one_in / rate.max(1.0)),
        None if counters.matches.load(Ordering::Relaxed) == 0 && covered > 1000 => {
            format!("rarer than 1 in {:.0} so far", counters.at_least_one_in())
        }
        None => "not enough data yet".to_owned(),
    };
    format!("{covered} seeds, {rate:.0}/s\n{odds}")
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
            let mut remove = None;
            for (index, card) in search.cards.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    if ui.small_button("x").on_hover_text("Remove").clicked() {
                        remove = Some(index);
                    }
                    ui.vertical(|ui| changed |= card_ui(ui, index, card));
                });
            }
            if let Some(index) = remove {
                search.cards.remove(index);
                changed = true;
            }
            ui.horizontal_wrapped(|ui| {
                let anchor = Anchor::Spawn;
                let village = STRUCTURE_KINDS
                    .iter()
                    .position(|kind| kind.id == "village")
                    .unwrap_or(0);
                let hut = STRUCTURE_KINDS
                    .iter()
                    .position(|kind| kind.id == "swamp_hut")
                    .unwrap_or(0);
                for (name, card) in [
                    (
                        "+ structure",
                        Card::Structure {
                            kind: village,
                            count: 1,
                            radius: 500,
                            anchor,
                        },
                    ),
                    (
                        "+ cluster",
                        Card::Cluster {
                            kind: hut,
                            count: 2,
                            radius: 128,
                            within: 3000,
                            anchor: Anchor::At(0, 0),
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
                search.sync_text();
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
                            search.text_error = None;
                        }
                        Err(err) => search.text_error = Some(err),
                    }
                }
                if let Some(error) = &search.text_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
            });
            ui.checkbox(&mut search.in_this_seed, "Find locations in this seed")
                .on_hover_text("`here` is every spot tried, 512 blocks apart");

            ui.horizontal(|ui| match &mut search.search {
                None => {
                    if ui
                        .add_enabled(!search.cards.is_empty(), egui::Button::new("Search"))
                        .clicked()
                    {
                        search.start(seed, threads);
                    }
                }
                Some(running) => {
                    let label = if running.paused { "Go on" } else { "Pause" };
                    if ui.button(label).clicked() {
                        running.paused = !running.paused;
                        running.search.set_paused(running.paused);
                    }
                    if ui.button("Stop").clicked() {
                        search.search = None;
                    }
                }
            });
            if let Some(running) = &search.search {
                ui.weak(odds_line(running));
            }
            egui::ScrollArea::vertical()
                .id_salt("seed results")
                .max_height(180.0)
                .show(ui, |ui| {
                    for found in &search.results {
                        let score = found.score.map_or(String::new(), |score| format!(" ({score:.0})"));
                        let where_ = found
                            .hits
                            .first()
                            .map_or(String::new(), |hit| format!(" x {} z {}", hit.x, hit.z));
                        let label = if search.in_this_seed {
                            format!("{where_}{score}")
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
