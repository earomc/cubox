//! The sub menus of the settings window.

use crate::{
    dimension::Dimension,
    gui::{Ui, finder::Target, map_view::MAX_RELIEF_LEVEL, overlays::GridSpacing},
    structures::STRUCTURE_KINDS,
    terrain::Terrain,
};

const ICON_SIZE: f32 = 18.0;

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
                    if self.too_far[kind] {
                        ui.weak("zoom in");
                    }
                });
            }
        });
    }

    pub(super) fn search_menu(&mut self, ui: &mut egui::Ui) {
        let mut new_target = None;
        let mut go_to = None;
        egui::CollapsingHeader::new("Search").show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.search_filter)
                    .hint_text("Structure or biome")
                    .desired_width(f32::INFINITY),
            );
            let filter = self.search_filter.trim().to_lowercase();
            let matches = |name: &str| name.to_lowercase().contains(&filter);
            egui::ScrollArea::vertical()
                .id_salt("search list")
                .max_height(140.0)
                .show(ui, |ui| {
                    for kind in self.kinds_here() {
                        let def = &STRUCTURE_KINDS[kind];
                        if matches(def.name) && ui.selectable_label(false, def.name).clicked() {
                            new_target = Some(Target::Structure(kind));
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
                        if matches(&name) && ui.selectable_label(false, format!("{name} (biome)")).clicked() {
                            new_target = Some(Target::Biome(biome));
                        }
                    }
                });
            ui.weak("Biome patches under 32 blocks wide can be missed");

            let mut cancel = false;
            if let Some(search) = &self.queries.search {
                ui.separator();
                let center = search.center;
                if search.done {
                    ui.label(format!("{}: {} found", search.target.name(), search.results.len()));
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
        });
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
        if let Some((at, height)) = self.hovered_height
            && at == quart
        {
            return Some(height);
        }
        let terrain = self.terrain.get_or_insert_with(|| Terrain::new(self.seed));
        let height = terrain.heights([quart[0] * 4, quart[1] * 4], 4, 1, &|| true)?[0];
        self.hovered_height = Some((quart, height));
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
