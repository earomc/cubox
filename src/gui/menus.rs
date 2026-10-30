//! The sub menus of the settings window.

use crate::{
    dimension::Dimension,
    gui::{Ui, finder::Target, map_view::MAX_RELIEF_LEVEL, overlays::GridSpacing},
    structures::STRUCTURE_KINDS,
    terrain::Terrain,
};

const ICON_SIZE: f32 = 18.0;

/// How many real heights the height map is made of.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ReliefQuality {
    Low,
    Medium,
    High,
    Full,
}

impl ReliefQuality {
    pub const ALL: [Self; 4] = [Self::Low, Self::Medium, Self::High, Self::Full];

    /// Pixels between real heights.
    pub fn step(self) -> u8 {
        match self {
            Self::Low => 8,
            Self::Medium => 4,
            Self::High => 2,
            Self::Full => 1,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Full => "Full",
        }
    }

    /// Measured on a desktop core, zoomed out. Close up, Full costs less.
    fn hint(self) -> &'static str {
        match self {
            Self::Low => "A real height every 8 pixels: 0.3 seconds per tile",
            Self::Medium => "A real height every 4 pixels: 1 second per tile",
            Self::High => "A real height every 2 pixels: 3 seconds per tile",
            Self::Full => "A real height for every pixel: 13 seconds per tile",
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
                    ui.horizontal(|ui| {
                        ui.label("Quality");
                        for quality in ReliefQuality::ALL {
                            ui.selectable_value(&mut self.relief_quality, quality, quality.name())
                                .on_hover_text(quality.hint());
                        }
                    });
                    ui.checkbox(&mut self.sea_floor, "Show sea floor")
                        .on_hover_text("Shade the ground under water. Without this water is flat.");
                });
            });
            if self.dimension != Dimension::Overworld {
                ui.weak("Only in the overworld");
            } else if self.relief && level > MAX_RELIEF_LEVEL {
                ui.weak("Zoom in to see terrain");
            }
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
