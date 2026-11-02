//! The search panel, for finding seeds or finding locations in the seed shown: conditions that read
//! like their text, and what was found.

use std::time::{Duration, Instant};

use crate::{
    dimension::Dimension,
    gui::Ui,
    noise::NoiseParameter,
    seedfind::{
        engine::{Match, biome_dimension},
        run::{Area, Search, Seeds},
        spec::{
            self, Anchor, BiomeTest, Condition, EXACT_STEP, LOCATION_PRESETS, LocationSearch, MAX_BLOCKS, SEED_PRESETS,
            Target,
        },
    },
    structures::{MARKER_KINDS, is_searchable, kind_index},
};

/// Matches kept in the list, the best ones.
const MAX_RESULTS: usize = 200;
/// How long after the last change a running search starts over, so that changing a value by
/// dragging or typing doesn't restart it all the time.
const RESTART_AFTER: Duration = Duration::from_millis(500);

/// What both searches have.
#[derive(Default)]
struct Running {
    /// the search as text, which is what can be saved and shared
    text: String,
    /// what is wrong with the text or the search
    error: Option<String>,
    search: Option<Search>,
    /// the last change that a running search hasn't taken on yet
    changed_at: Option<Instant>,
    results: Vec<Match>,
}

impl Running {
    /// Takes the matches found since the last frame. True while there is something to update.
    fn poll(&mut self, threads: usize, print: bool) -> bool {
        let Some(search) = &self.search else {
            return self.changed_at.is_some();
        };
        search.set_threads(threads);
        // before taking the matches: all of them have been sent once it is done
        let done = search.is_done();
        let mut sort = false;
        while let Ok(found) = search.results.try_recv() {
            if print {
                println!("seed search: {} {:?} {:?}", found.seed, found.score, found.hits.first());
            }
            sort |= add_result(&mut self.results, found);
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

    /// Whether a running search is due to start over after a change.
    fn restart_due(&mut self) -> bool {
        if self.changed_at.is_some_and(|at| at.elapsed() >= RESTART_AFTER) {
            self.changed_at = None;
            return self.search.is_some() && self.error.is_none();
        }
        false
    }

    /// After a change of what is looked for, with what is wrong with it now.
    fn changed(&mut self, error: Option<String>) {
        self.error = error;
        self.changed_at = Some(Instant::now());
        if self.error.is_some() {
            self.search = None;
        }
    }

    /// A search has started over. What was found stays until it is removed, see `results_ui`.
    fn started(&mut self, search: Result<Search, String>) {
        self.search = None;
        match search {
            Ok(search) => self.search = Some(search),
            Err(err) => self.error = Some(err),
        }
    }
}

/// Lists what was found, unless it is listed already, as searching again finds the same. True if it
/// is new.
fn add_result(results: &mut Vec<Match>, found: Match) -> bool {
    if results.contains(&found) {
        return false;
    }
    results.push(found);
    true
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Seeds,
    Locations,
}

pub struct SeedSearch {
    tab: Tab,
    /// how far apart biomes and noises are looked at, for both searches
    step: i32,
    seed_conditions: Vec<Condition>,
    seeds: Running,
    location_search: LocationSearch,
    /// how far locations are looked for, and around where
    area_radius: i32,
    area_center: [i32; 2],
    locations: Running,
    print_results: bool,
}

impl Default for SeedSearch {
    fn default() -> Self {
        let seed_text = SEED_PRESETS[0].1.to_owned();
        let location_text = LOCATION_PRESETS[0].1.to_owned();
        Self {
            tab: Tab::Seeds,
            step: EXACT_STEP,
            seed_conditions: spec::parse_seeds(&seed_text).unwrap_or_default(),
            seeds: Running {
                text: seed_text,
                ..Default::default()
            },
            location_search: spec::parse_locations(&location_text).expect("the presets are tested"),
            area_radius: 10_000,
            area_center: [0, 0],
            locations: Running {
                text: location_text,
                ..Default::default()
            },
            print_results: false,
        }
    }
}

impl SeedSearch {
    /// Takes the matches found since the last frame. True while there is something to update.
    pub fn poll(&mut self, seed: i64, threads: usize) -> bool {
        if self.seeds.restart_due() {
            self.start_seeds(threads);
        }
        if self.locations.restart_due() {
            self.start_locations(seed, threads);
        }
        let print = self.print_results;
        // both, so that neither is left waiting with full hands while the other is shown
        self.seeds.poll(threads, print) | self.locations.poll(threads, print)
    }

    fn start_seeds(&mut self, threads: usize) {
        let seeds = Seeds::Random(crate::seed::random_seed() as u64);
        let search = Search::seeds(&self.seed_conditions, seeds, threads, self.step);
        self.seeds.started(search);
    }

    fn start_locations(&mut self, seed: i64, threads: usize) {
        let area = Area {
            seed,
            radius: self.area_radius,
            center: self.area_center,
        };
        let search = Search::locations(&self.location_search, area, threads, self.step);
        self.locations.started(search);
    }

    fn all_threads() -> usize {
        std::thread::available_parallelism().map_or(4, |n| n.get())
    }

    /// `search_seeds` of a script.
    pub fn script_seeds(&mut self, text: &str) {
        match spec::parse_seeds(text) {
            Ok(conditions) => {
                self.seed_conditions = conditions;
                self.tab = Tab::Seeds;
                self.print_results = true;
                self.seeds_changed();
                self.start_seeds(Self::all_threads());
            }
            Err(err) => log::error!("searching: {err}"),
        }
    }

    /// `search_locations` of a script.
    pub fn script_locations(&mut self, text: &str, seed: i64, radius: i32, center: [i32; 2]) {
        match spec::parse_locations(text) {
            Ok(search) => {
                self.location_search = search;
                (self.area_radius, self.area_center) = (radius, center);
                self.tab = Tab::Locations;
                self.print_results = true;
                self.locations_changed();
                self.start_locations(seed, Self::all_threads());
            }
            Err(err) => log::error!("searching: {err}"),
        }
    }

    /// `search_step` of a script.
    pub fn script_step(&mut self, step: i32) {
        self.step = step.max(EXACT_STEP);
    }

    /// `edit_search` of a script.
    pub fn script_edit(&mut self, text: &str) {
        let result = match self.tab {
            Tab::Seeds => spec::parse_seeds(text).map(|conditions| {
                self.seed_conditions = conditions;
                self.seeds_changed();
            }),
            Tab::Locations => spec::parse_locations(text).map(|search| {
                self.location_search = search;
                self.locations_changed();
            }),
        };
        if let Err(err) = result {
            log::error!("edit_search: {err}");
        }
    }

    pub fn script_status(&self) -> String {
        self.shown().search.as_ref().map_or(String::new(), Search::status)
    }

    fn shown(&self) -> &Running {
        match self.tab {
            Tab::Seeds => &self.seeds,
            Tab::Locations => &self.locations,
        }
    }

    fn seeds_changed(&mut self) {
        self.seeds.text = spec::print_seeds(&self.seed_conditions);
        let error = spec::validate_seeds(&self.seed_conditions).err();
        self.seeds.changed(error);
    }

    fn locations_changed(&mut self) {
        self.locations.text = spec::print_locations(&self.location_search);
        let error = spec::validate_locations(&self.location_search).err();
        self.locations.changed(error);
    }

    /// A change of the settings of both searches.
    fn step_changed(&mut self) {
        self.seeds.changed(self.seeds.error.clone());
        self.locations.changed(self.locations.error.clone());
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
        Anchor::At(..) | Anchor::Target => "x,z",
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

/// `within 500 of ...`, or only `within 500` around what was found.
fn within_ui(ui: &mut egui::Ui, salt: usize, radius: &mut i32, anchor: &mut Anchor, spawn_exists: bool) -> bool {
    ui.label("within");
    let mut changed = number_ui(ui, radius, 1..=MAX_BLOCKS, 16.0);
    if *anchor != Anchor::Target {
        ui.label("of");
        changed |= anchor_ui(ui, salt, anchor, spawn_exists);
    }
    changed
}

fn kind_ui(ui: &mut egui::Ui, salt: usize, kind: &mut usize) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(("kind", salt))
        .selected_text(MARKER_KINDS[*kind].id)
        .width(120.0)
        .show_ui(ui, |ui| {
            for (index, def) in MARKER_KINDS.iter().enumerate() {
                if is_searchable(index) {
                    changed |= ui.selectable_value(kind, index, def.id).changed();
                }
            }
        });
    changed
}

fn biome_ui(ui: &mut egui::Ui, salt: usize, biome: &mut String) -> bool {
    let mut changed = false;
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
    changed
}

fn noise_ui(ui: &mut egui::Ui, salt: usize, parameter: &mut NoiseParameter, min: &mut f32, max: &mut f32) -> bool {
    let mut changed = false;
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
    changed
}

/// A least area that is optional.
fn min_area_ui(ui: &mut egui::Ui, min_area: &mut Option<i64>) -> bool {
    let mut on = min_area.is_some();
    let mut changed = ui.checkbox(&mut on, "area>=").changed();
    if changed {
        *min_area = on.then_some(1_000_000);
    }
    if let Some(area) = min_area {
        changed |= number_ui(ui, area, 1..=1_000_000_000_000, 10_000.0);
    }
    changed
}

/// What a search for locations finds, word for word its text with the values to change.
fn target_ui(ui: &mut egui::Ui, target: &mut Target) -> bool {
    let mut changed = false;
    let name = |target: &Target| match target {
        Target::Poi { .. } => "poi",
        Target::Cluster { .. } => "cluster",
        Target::Biome { .. } => "biome",
        Target::Noise { .. } => "noise",
    };
    egui::ComboBox::from_id_salt("target")
        .selected_text(format!("find {}", name(target)))
        .width(70.0)
        .show_ui(ui, |ui| {
            for option in [
                Target::Poi {
                    kind: kind_index("village"),
                },
                Target::Cluster {
                    kind: kind_index("swamp_hut"),
                    count: 2,
                    radius: 128,
                },
                Target::Biome {
                    biome: "mushroom_fields".to_owned(),
                    min_area: None,
                },
                Target::Noise {
                    parameter: NoiseParameter::Continentalness,
                    min: -1.2,
                    max: -1.05,
                    min_area: None,
                },
            ] {
                let same = std::mem::discriminant(target) == std::mem::discriminant(&option);
                if ui.selectable_label(same, name(&option)).clicked() && !same {
                    *target = option;
                    changed = true;
                }
            }
        });
    const SALT: usize = usize::MAX;
    match target {
        Target::Poi { kind } => changed |= kind_ui(ui, SALT, kind),
        Target::Cluster { kind, count, radius } => {
            changed |= kind_ui(ui, SALT, kind);
            ui.label("count>=");
            changed |= number_ui(ui, count, 1..=64, 0.1);
            ui.label("radius");
            changed |= number_ui(ui, radius, 1..=MAX_BLOCKS, 4.0);
        }
        Target::Biome { biome, min_area } => {
            changed |= biome_ui(ui, SALT, biome);
            changed |= min_area_ui(ui, min_area);
        }
        Target::Noise {
            parameter,
            min,
            max,
            min_area,
        } => {
            changed |= noise_ui(ui, SALT, parameter, min, max);
            changed |= min_area_ui(ui, min_area);
        }
    }
    changed
}

/// The conditions of a search, to change, remove and add to. Those added are around `anchor`.
fn conditions_ui(ui: &mut egui::Ui, conditions: &mut Vec<Condition>, anchor: Anchor) -> bool {
    let mut changed = false;
    let mut remove = None;
    for (index, condition) in conditions.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if ui.small_button("x").on_hover_text("Remove").clicked() {
                remove = Some(index);
            }
            // `any` is a block of lines, which has no `not`
            if !matches!(condition, Condition::Any(_)) {
                let is_not = matches!(condition, Condition::Not(_));
                if ui.selectable_label(is_not, "not").clicked() {
                    let taken = std::mem::replace(condition, Condition::Any(Vec::new()));
                    *condition = match taken {
                        Condition::Not(inner) => *inner,
                        other => Condition::Not(Box::new(other)),
                    };
                    changed = true;
                }
            }
            changed |= condition_ui(ui, index, condition);
        });
    }
    if let Some(index) = remove {
        conditions.remove(index);
        changed = true;
    }
    ui.horizontal_wrapped(|ui| {
        // around a place it has to be a position, where `spawn` isn't what a cluster needs
        let far = if anchor == Anchor::Target {
            anchor
        } else {
            Anchor::At(0, 0)
        };
        for (name, condition) in [
            (
                "+ poi",
                Condition::Poi {
                    kind: kind_index("village"),
                    count: 1,
                    radius: 500,
                    anchor,
                },
            ),
            (
                "+ cluster",
                Condition::Cluster {
                    kind: kind_index("swamp_hut"),
                    count: 2,
                    radius: 128,
                    within: 3000,
                    anchor: far,
                },
            ),
            (
                "+ biome",
                Condition::Biome {
                    biome: "plains".to_owned(),
                    test: BiomeTest::Present,
                    radius: 500,
                    anchor,
                },
            ),
            (
                "+ noise",
                Condition::Noise {
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
                conditions.push(condition);
                changed = true;
            }
        }
    });
    changed
}

/// The buttons that start and stop a search, and how it is going. True if it is to start.
fn controls_ui(ui: &mut egui::Ui, running: &mut Running) -> bool {
    if let Some(error) = &running.error {
        ui.colored_label(egui::Color32::LIGHT_RED, error);
    }
    let mut start = false;
    let done = running.search.as_ref().is_none_or(Search::is_done);
    ui.horizontal(|ui| {
        if done {
            let label = if running.search.is_some() {
                "Search again"
            } else {
                "Search"
            };
            if ui
                .add_enabled(running.error.is_none(), egui::Button::new(label))
                .clicked()
            {
                running.changed_at = None;
                start = true;
            }
        } else {
            if let Some(search) = &mut running.search {
                let paused = search.is_paused();
                if ui.button(if paused { "Go on" } else { "Pause" }).clicked() {
                    search.set_paused(!paused);
                }
            }
            if ui.button("Stop").clicked() {
                running.search = None;
            }
        }
    });
    if let Some(search) = &running.search {
        ui.weak(search.status());
    }
    start
}

/// The list of what was found, whose entries can be removed. Returns the one clicked.
fn results_ui(ui: &mut egui::Ui, running: &mut Running, label: impl Fn(&Match) -> String) -> Option<Match> {
    if running.results.is_empty() {
        return None;
    }
    if ui.small_button("Clear results").clicked() {
        running.results.clear();
    }
    let mut pick = None;
    let mut remove = None;
    egui::ScrollArea::vertical()
        .id_salt("seed results")
        .max_height(180.0)
        .show(ui, |ui| {
            for (index, found) in running.results.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.small_button("x").on_hover_text("Remove from the list").clicked() {
                        remove = Some(index);
                    }
                    if ui
                        .selectable_label(false, label(found))
                        .on_hover_text("Go there and show its markers")
                        .clicked()
                    {
                        pick = Some(found.clone());
                    }
                });
            }
        });
    if let Some(index) = remove {
        running.results.remove(index);
    }
    pick
}

/// One condition, word for word its text with the values to change. True if it changed.
fn condition_ui(ui: &mut egui::Ui, salt: usize, condition: &mut Condition) -> bool {
    let mut changed = false;
    match condition {
        Condition::Poi {
            kind,
            count,
            radius,
            anchor,
        } => {
            ui.label("poi");
            changed |= kind_ui(ui, salt, kind);
            ui.label("count>=");
            changed |= number_ui(ui, count, 1..=64, 0.1);
            let spawn_exists = MARKER_KINDS[*kind].dimension != Dimension::Nether;
            changed |= within_ui(ui, salt, radius, anchor, spawn_exists);
        }
        Condition::Cluster {
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
            let spawn_exists = MARKER_KINDS[*kind].dimension != Dimension::Nether;
            changed |= within_ui(ui, salt, within, anchor, spawn_exists);
        }
        Condition::Biome {
            biome,
            test,
            radius,
            anchor,
        } => {
            ui.label("biome");
            changed |= biome_ui(ui, salt, biome);
            let name = |test: &BiomeTest| match test {
                BiomeTest::Present => "anywhere",
                BiomeTest::Coverage(_) => "coverage>=",
                BiomeTest::Area(_) => "area>=",
            };
            egui::ComboBox::from_id_salt(("test", salt))
                .selected_text(name(test))
                .width(70.0)
                .show_ui(ui, |ui| {
                    for option in [
                        BiomeTest::Present,
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
        Condition::Noise {
            parameter,
            min,
            max,
            coverage,
            radius,
            anchor,
        } => {
            changed |= noise_ui(ui, salt, parameter, min, max);
            ui.label("coverage>=");
            changed |= number_ui(ui, coverage, 0.0..=100.0, 1.0);
            ui.label("%");
            changed |= within_ui(ui, salt, radius, anchor, true);
        }
        Condition::Not(inner) => changed |= condition_ui(ui, salt, inner),
        Condition::Any(conditions) => {
            ui.label(format!("any of {} conditions, see the text", conditions.len()));
        }
    }
    changed
}

/// The search as text. True if it was changed.
fn text_ui(ui: &mut egui::Ui, text: &mut String) -> bool {
    ui.add(
        egui::TextEdit::multiline(text)
            .font(egui::TextStyle::Monospace)
            .desired_rows(3)
            .desired_width(f32::INFINITY),
    )
    .changed()
}

impl Ui {
    pub(super) fn seed_search_menu(&mut self, ui: &mut egui::Ui) {
        let seed = self.seed;
        let threads = self.threads;
        let open = self.seed_search.print_results.then_some(true);
        let mut pick = None;
        egui::CollapsingHeader::new("Search").open(open).show(ui, |ui| {
            let search = &mut self.seed_search;
            ui.horizontal(|ui| {
                ui.selectable_value(&mut search.tab, Tab::Seeds, "Find seeds")
                    .on_hover_text("Seeds where all conditions hold around spawn or a position");
                ui.selectable_value(&mut search.tab, Tab::Locations, "Find locations")
                    .on_hover_text("Everything of what you ask for in an area of the seed shown");
            });
            ui.horizontal(|ui| {
                ui.label("Biomes every");
                if number_ui(ui, &mut search.step, EXACT_STEP..=1024, 4.0) {
                    search.step_changed();
                }
                ui.label("blocks").on_hover_text(
                    "How far apart biomes and noises are looked at. 4 is exact and misses nothing. A larger step is much faster, and can miss what is smaller than it.",
                );
            });
            match search.tab {
                Tab::Seeds => {
                    let mut changed = false;
                    egui::ComboBox::from_id_salt("seed presets")
                        .selected_text("Presets")
                        .show_ui(ui, |ui| {
                            for (name, text) in SEED_PRESETS {
                                if ui.selectable_label(false, *name).clicked()
                                    && let Ok(conditions) = spec::parse_seeds(text)
                                {
                                    search.seed_conditions = conditions;
                                    changed = true;
                                }
                            }
                        });
                    changed |= conditions_ui(ui, &mut search.seed_conditions, Anchor::Spawn);
                    if changed {
                        search.seeds_changed();
                    }
                    egui::CollapsingHeader::new("As text").show(ui, |ui| {
                        if text_ui(ui, &mut search.seeds.text) {
                            match spec::parse_seeds(&search.seeds.text) {
                                Ok(conditions) => {
                                    search.seed_conditions = conditions;
                                    search.seeds.changed(None);
                                }
                                Err(err) => search.seeds.changed(Some(err)),
                            }
                        }
                    });
                    if controls_ui(ui, &mut search.seeds) {
                        search.start_seeds(threads);
                    }
                    pick = results_ui(ui, &mut search.seeds, |found| {
                        // the nearest structure counts as the score below 0
                        let score = found.score.map_or(String::new(), |score| {
                            if score < 0.0 {
                                format!(" ({:.0} blocks away)", -score)
                            } else {
                                format!(" ({score:.0})")
                            }
                        });
                        format!("{}{score}", found.seed)
                    });
                }
                Tab::Locations => {
                    let mut changed = false;
                    ui.horizontal(|ui| {
                        ui.label("in this seed, within");
                        changed |= number_ui(ui, &mut search.area_radius, 1..=MAX_BLOCKS, 16.0);
                        ui.label("of");
                        let [x, z] = &mut search.area_center;
                        changed |= position_ui(ui, x, z);
                    })
                    .response
                    .on_hover_text("The area to search, in blocks of the dimension of what you are looking for");
                    egui::ComboBox::from_id_salt("location presets")
                        .selected_text("Presets")
                        .show_ui(ui, |ui| {
                            for (name, text) in LOCATION_PRESETS {
                                if ui.selectable_label(false, *name).clicked()
                                    && let Ok(preset) = spec::parse_locations(text)
                                {
                                    search.location_search = preset;
                                    changed = true;
                                }
                            }
                        });
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        changed |= target_ui(ui, &mut search.location_search.target);
                    });
                    ui.weak("with, around each of them:");
                    changed |= conditions_ui(ui, &mut search.location_search.conditions, Anchor::Target);
                    if changed {
                        search.locations_changed();
                    }
                    egui::CollapsingHeader::new("As text").show(ui, |ui| {
                        if text_ui(ui, &mut search.locations.text) {
                            match spec::parse_locations(&search.locations.text) {
                                Ok(parsed) => {
                                    search.location_search = parsed;
                                    search.locations.changed(None);
                                }
                                Err(err) => search.locations.changed(Some(err)),
                            }
                        }
                    });
                    if controls_ui(ui, &mut search.locations) {
                        search.start_locations(seed, threads);
                    }
                    pick = results_ui(ui, &mut search.locations, |found| {
                        found.hits.first().map_or(String::new(), |hit| {
                            let is_patch = hit.kind.is_none() && found.hits.len() == 1;
                            let size = found
                                .score
                                .filter(|_| is_patch)
                                .map_or(String::new(), |area| format!(" ({area:.0} square blocks)"));
                            format!("x {} z {}{size}", hit.x, hit.z)
                        })
                    });
                }
            }
        });
        if let Some(found) = pick {
            self.open_match(&found);
        }
    }

    /// `open_first_match` of a script.
    pub(super) fn script_open_first_match(&mut self) {
        match self.seed_search.shown().results.first().cloned() {
            Some(found) => self.open_match(&found),
            None => log::error!("no match to open"),
        }
    }

    /// Shows a match on the map with its markers.
    fn open_match(&mut self, found: &Match) {
        let dimension = found.hits.first().map_or(Dimension::Overworld, |hit| hit.dimension);
        if found.seed != self.seed {
            self.seed_text = found.seed.to_string();
        }
        if found.seed != self.seed || dimension != self.dimension {
            self.set_world(found.seed, dimension);
        }
        for kind in found.hits.iter().filter_map(|hit| hit.kind) {
            self.shown_kinds[kind] = true;
        }
        if let Some(hit) = found.hits.first() {
            self.map.camera.center = [hit.x as f64 / 4.0, hit.z as f64 / 4.0];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seedfind::engine::Hit;

    fn found(seed: i64, x: i32) -> Match {
        Match {
            seed,
            score: Some(1.0),
            hits: vec![Hit {
                kind: None,
                dimension: Dimension::Overworld,
                x,
                z: 0,
            }],
        }
    }

    #[test]
    fn results_stay_until_removed_and_are_listed_once() {
        let mut running = Running::default();
        assert!(add_result(&mut running.results, found(1, 5)));
        assert!(add_result(&mut running.results, found(2, 5)));
        assert!(!add_result(&mut running.results, found(1, 5)));
        running.changed(None);
        running.started(Err("a search that can't start".to_owned()));
        assert_eq!(running.results, vec![found(1, 5), found(2, 5)]);
    }
}
