//! The sub menus of the settings window.

use crate::{
    dimension::Dimension,
    gui::{
        Ui,
        finder::Target,
        overlays::{GridSpacing, MARKER_GAPS},
        resources,
    },
    structures::STRUCTURE_KINDS,
    terrain::Terrain,
    tile_renderer::Detail,
};

const ICON_SIZE: f32 = 18.0;

/// The coarsest level with exact heights at the manual qualities. Beyond it even Full would take
/// a long time for what are single columns far apart.
const MAX_EXACT_LEVEL: i32 = 3;

/// Values most people never need to change.
pub struct Advanced {
    pub marker_size: f32,
    /// candidates an empty spot is given
    pub search_tries: u32,
    /// spots searched per marker distance, per axis
    pub search_fineness: u32,
    pub max_relief_level: i32,
    pub relief_strength: f32,
    pub fade_seconds: f32,
}

impl Default for Advanced {
    fn default() -> Self {
        Self {
            marker_size: 22.0,
            search_tries: 8192,
            search_fineness: 2,
            max_relief_level: 5,
            relief_strength: 1.0,
            fade_seconds: 0.15,
        }
    }
}

/// How many exact heights the height map is made of.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ReliefQuality {
    Auto,
    Low,
    Medium,
    High,
    Full,
}

impl ReliefQuality {
    pub const ALL: [Self; 5] = [Self::Auto, Self::Low, Self::Medium, Self::High, Self::Full];

    /// Where the heights of a tile at this level come from.
    pub fn detail(self, level: i32) -> Detail {
        let step = match self {
            Self::Auto => match level {
                0 => 2,
                1 => 4,
                _ => return Detail::Estimate,
            },
            Self::Low => 8,
            Self::Medium => 4,
            Self::High => 2,
            Self::Full => 1,
        };
        if level > MAX_EXACT_LEVEL {
            Detail::Estimate
        } else {
            Detail::Exact(step)
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Full => "Full",
        }
    }

    /// Measured on a desktop core.
    fn hint(self) -> &'static str {
        match self {
            Self::Auto => "Exact heights close up, the estimate further out",
            Self::Low => "An exact height every 8 pixels: 0.3 seconds per tile",
            Self::Medium => "An exact height every 4 pixels: 1 second per tile",
            Self::High => "An exact height every 2 pixels: 3 seconds per tile",
            Self::Full => "An exact height for every pixel: 13 seconds per tile",
        }
    }
}

impl Ui {
    fn kinds_here(&self) -> impl Iterator<Item = usize> + use<'_> {
        (0..STRUCTURE_KINDS.len()).filter(|&kind| STRUCTURE_KINDS[kind].dimension == self.dimension)
    }

    pub(super) fn structures_menu(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Structures").show(ui, |ui| {
            ui.horizontal(|ui| {
                let kinds: Vec<usize> = self.kinds_here().collect();
                if ui.button("All").clicked() {
                    kinds.iter().for_each(|&kind| self.shown_kinds[kind] = true);
                }
                if ui.button("None").clicked() {
                    kinds.iter().for_each(|&kind| self.shown_kinds[kind] = false);
                }
            });
            ui.add_enabled_ui(!self.marker_overlap, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Spacing");
                    ui.add(
                        egui::Slider::new(&mut self.marker_gap, MARKER_GAPS)
                            .logarithmic(true)
                            .fixed_decimals(0)
                            .suffix(" pt"),
                    )
                    .on_hover_text(
                        "The space between markers at least. Where there is room and a structure, it is shown, \
                         the rarest kinds first.",
                    );
                });
            });
            ui.checkbox(&mut self.prioritize_rare, "Prioritize rare").on_hover_text(
                "Where markers compete for space, the rarest kinds get it. Off: every kind has the same chance.",
            );
            ui.checkbox(&mut self.marker_overlap, "Let markers overlap").on_hover_text(
                "As many markers as you allow below, even on top of each other. Without this, as many as there is room for.",
            );
            ui.add_enabled_ui(self.marker_overlap, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Max markers");
                    ui.add(egui::Slider::new(&mut self.max_markers, 1000..=200_000).logarithmic(true))
                        .on_hover_text("How many are looked for and drawn. More takes longer and can make the map slow.");
                });
            });
            for kind in self.kinds_here().collect::<Vec<_>>() {
                ui.horizontal(|ui| {
                    if let Some(icons) = &self.icons {
                        ui.add(icons.image(kind, ICON_SIZE));
                    }
                    ui.checkbox(&mut self.shown_kinds[kind], STRUCTURE_KINDS[kind].name);
                    if self.sampled[kind] {
                        ui.weak("some shown")
                            .on_hover_text("Too many to look at all of them: zoom in to see every one");
                    }
                });
            }
        });
    }

    pub(super) fn search_menu(&mut self, ui: &mut egui::Ui) {
        let mut new_target = None;
        let mut go_to = None;
        let mut clear = false;
        egui::CollapsingHeader::new("Search").show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.search_filter)
                    .hint_text("Structure or biome")
                    .desired_width(f32::INFINITY),
            );
            let filter = self.search_filter.trim().to_lowercase();
            let matches = |name: &str| name.to_lowercase().contains(&filter);
            let active = self.queries.search.as_ref().map(|search| search.target.clone());
            let mut pick = |target: Target| {
                if active.as_ref() == Some(&target) {
                    clear = true;
                } else {
                    new_target = Some(target);
                }
            };
            egui::ScrollArea::vertical()
                .id_salt("search list")
                .max_height(140.0)
                .show(ui, |ui| {
                    for kind in self.kinds_here() {
                        let def = &STRUCTURE_KINDS[kind];
                        let target = Target::Structure(kind);
                        if matches(def.name)
                            && ui
                                .selectable_label(active.as_ref() == Some(&target), def.name)
                                .clicked()
                        {
                            pick(target);
                        }
                    }
                    let mut biomes: Vec<String> = self
                        .biome_source
                        .possible_biome_refs()
                        .iter()
                        .map(|biome| biome.key.path.to_string())
                        .collect();
                    biomes.sort();
                    for biome in biomes {
                        let name = biome.replace('_', " ");
                        let target = Target::Biome(biome);
                        if matches(&name)
                            && ui
                                .selectable_label(active.as_ref() == Some(&target), format!("{name} (biome)"))
                                .clicked()
                        {
                            pick(target);
                        }
                    }
                });
            ui.weak("Biome patches under 32 blocks wide can be missed");

            let (mut cancel, mut clear_button) = (false, false);
            if let Some(search) = &self.queries.search {
                ui.separator();
                let center = search.center;
                if search.done {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}: {} found", search.target.name(), search.results.len()));
                        clear_button = ui.button("Clear").clicked();
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("{}: {} blocks out", search.target.name(), search.reached));
                        cancel = ui.button("Cancel").clicked();
                    });
                }
                for result in &search.results {
                    let distance = ((result[0] - center[0]) as f64).hypot((result[1] - center[1]) as f64);
                    let text = format!("x {} z {} ({:.0} blocks)", result[0], result[1], distance);
                    if ui.selectable_label(false, text).clicked() {
                        go_to = Some(*result);
                    }
                }
            }
            if cancel && let Some(search) = &mut self.queries.search {
                search.done = true;
            }
            clear |= clear_button;
        });
        if clear {
            self.queries.clear_search();
        }
        if let Some(target) = new_target {
            let center = self.map.camera.center;
            self.queries
                .start_search(target, [(center[0] * 4.0) as i32, (center[1] * 4.0) as i32]);
        }
        if let Some(block) = go_to {
            self.map.camera.center = [block[0] as f64 / 4.0, block[1] as f64 / 4.0];
        }
    }

    pub(super) fn terrain_menu(&mut self, ui: &mut egui::Ui, level: i32) {
        egui::CollapsingHeader::new("Terrain").show(ui, |ui| {
            ui.add_enabled_ui(self.dimension == Dimension::Overworld, |ui| {
                ui.checkbox(&mut self.relief, "Show height map")
                    .on_hover_text("The real terrain height, shaded with the light in the north west");
                ui.add_enabled_ui(self.relief, |ui| {
                    ui.checkbox(&mut self.relief_exact, "Exact heights").on_hover_text(
                        "Replace the game's quick estimate with the real heights when zoomed in. Much slower.",
                    );
                    ui.add_enabled_ui(self.relief_exact, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Quality");
                            for quality in ReliefQuality::ALL {
                                ui.selectable_value(&mut self.relief_quality, quality, quality.name())
                                    .on_hover_text(quality.hint());
                            }
                        });
                    });
                    ui.checkbox(&mut self.sea_floor, "Show sea floor")
                        .on_hover_text("Shade the ground under water. Without this water is flat.");
                });
            });
            if self.dimension != Dimension::Overworld {
                ui.weak("Only in the overworld");
            } else if self.relief && level > self.advanced.max_relief_level {
                ui.weak("Zoom in to see terrain");
            } else if self.relief && (!self.relief_exact || self.relief_quality.detail(level) == Detail::Estimate) {
                ui.weak("Estimated heights, usually 5 to 15 blocks off");
            }
        });
    }

    pub(super) fn advanced_menu(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Advanced").show(ui, |ui| {
            let advanced = &mut self.advanced;
            let row = |ui: &mut egui::Ui, name: &str, slider: egui::Slider, hint: &str| {
                ui.horizontal(|ui| {
                    ui.label(name);
                    ui.add(slider).on_hover_text(hint);
                });
            };
            row(
                ui,
                "Marker size",
                egui::Slider::new(&mut advanced.marker_size, 10.0..=64.0)
                    .fixed_decimals(0)
                    .suffix(" pt"),
                "How big markers are on the screen",
            );
            row(
                ui,
                "Search tries",
                egui::Slider::new(&mut advanced.search_tries, 16..=65536).logarithmic(true),
                "How many candidates an empty spot is given to find a structure. More fills more gaps, more slowly.",
            );
            row(
                ui,
                "Search detail",
                egui::Slider::new(&mut advanced.search_fineness, 1..=4),
                "How many candidates are searched for each marker that fits. More fills more gaps, with much more work.",
            );
            row(
                ui,
                "Height map zoom",
                egui::Slider::new(&mut advanced.max_relief_level, 0..=10).prefix("level "),
                "The furthest zoom level with a height map. From about level 6 on it is mostly noise.",
            );
            row(
                ui,
                "Height map strength",
                egui::Slider::new(&mut advanced.relief_strength, 0.1..=1.8).fixed_decimals(1),
                "How strongly the height map is drawn over the biomes",
            );
            row(
                ui,
                "Tile fade",
                egui::Slider::new(&mut advanced.fade_seconds, 0.0..=1.0)
                    .fixed_decimals(2)
                    .suffix(" s"),
                "How long new tiles take to fade in",
            );
            if ui.button("Reset").clicked() {
                *advanced = Advanced::default();
            }
        });
    }

    /// The plus and minus keys zoom the map, like the zoom buttons, unless they are being typed. With
    /// Ctrl they are egui's, which scale the interface.
    pub(super) fn zoom_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let pressed = |key| ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, key));
        if pressed(egui::Key::Plus) || pressed(egui::Key::Equals) {
            self.map.zoom_by(-1.0);
        }
        if pressed(egui::Key::Minus) {
            self.map.zoom_by(1.0);
        }
    }

    pub(super) fn performance_menu(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Performance").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Threads");
                ui.add(egui::Slider::new(&mut self.threads, 1..=resources::max_threads()))
                    .on_hover_text("How many cores cubox uses at most. Fewer leave more for other programs.");
            });
            ui.horizontal(|ui| {
                let (min, max) = resources::CACHE_MIB;
                ui.label("Tile memory");
                ui.add(
                    egui::Slider::new(&mut self.cache_mib, min..=max)
                        .logarithmic(true)
                        .suffix(" MiB"),
                )
                .on_hover_text("Finished tiles kept for going back, the view itself is always kept");
            });
            #[cfg(target_arch = "wasm32")]
            ui.weak("Workers already started keep their memory until the page is reloaded");
        });
    }

    pub(super) fn grid_menu(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Grid").show(ui, |ui| {
            ui.checkbox(&mut self.grid.enabled, "Show grid");
            ui.add_enabled_ui(self.grid.enabled, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Lines every");
                    for (spacing, name) in [
                        (GridSpacing::Auto, "auto"),
                        (GridSpacing::Chunks, "chunk"),
                        (GridSpacing::Regions, "region"),
                        (GridSpacing::Custom, "custom"),
                    ] {
                        ui.selectable_value(&mut self.grid.spacing, spacing, name);
                    }
                });
                if self.grid.spacing == GridSpacing::Custom {
                    ui.horizontal(|ui| {
                        ui.label("Blocks");
                        ui.add(
                            egui::DragValue::new(&mut self.grid.custom_spacing)
                                .range(1..=10_000_000)
                                .speed(16.0),
                        );
                    });
                }
                ui.checkbox(&mut self.grid.labels, "Coordinates");
                ui.horizontal(|ui| {
                    ui.label("Opacity");
                    ui.add(egui::Slider::new(&mut self.grid.opacity, 0.1..=1.0));
                });
            });
        });
    }

    /// The terrain height at a quart while relief is on.
    pub(super) fn hovered_height(&mut self, quart: [i32; 2]) -> Option<i32> {
        if !self.relief || self.dimension != Dimension::Overworld {
            return None;
        }
        if let Some((at, sea_floor, height)) = self.hovered_height
            && at == quart
            && sea_floor == self.sea_floor
        {
            return Some(height);
        }
        let terrain = self.terrain.get_or_insert_with(|| Terrain::new(self.seed));
        let height = terrain.heights([quart[0] * 4, quart[1] * 4], 4, 1, self.sea_floor, &|| true)?[0];
        self.hovered_height = Some((quart, self.sea_floor, height));
        Some(height)
    }

    /// `structure` of a script.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn script_structure(&mut self, which: &str) {
        match which {
            "all" => self.shown_kinds.fill(true),
            "none" => self.shown_kinds.fill(false),
            _ => match STRUCTURE_KINDS.iter().position(|kind| kind.id == which) {
                Some(kind) => self.shown_kinds[kind] = true,
                None => log::error!("no structure {which}"),
            },
        }
    }

    /// `search` of a script.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn script_search(&mut self, what: &str) {
        let target = match STRUCTURE_KINDS.iter().position(|kind| kind.id == what) {
            Some(kind) => Target::Structure(kind),
            None => Target::Biome(what.to_string()),
        };
        let center = self.map.camera.center;
        self.queries
            .start_search(target, [(center[0] * 4.0) as i32, (center[1] * 4.0) as i32]);
    }
}
