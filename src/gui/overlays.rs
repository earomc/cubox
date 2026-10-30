//! What is painted over the map: structure markers, the distance grid and the zoom buttons.

use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, TextureHandle, pos2, vec2};

use crate::{
    gui::{Ui, finder::Job},
    structures::STRUCTURE_KINDS,
    tile_renderer::WORLD_BORDER_QUARTS,
};

/// Markers are this big on the screen whatever the zoom, in points.
const MARKER_SIZE: f32 = 22.0;
/// Auto grid lines stay at least this far apart, in points.
const MIN_LINE_DISTANCE: f64 = 80.0;
/// Fixed grid spacings get coarser when lines would be closer than this, in points.
const MIN_FIXED_LINE_DISTANCE: f64 = 12.0;
const SEARCH_COLOR: Color32 = Color32::from_rgb(255, 225, 60);

/// The structure icons in `assets/icons`. Replace those files and rebuild to change them.
macro_rules! icon_files {
    ($($id:literal),* $(,)?) => {
        fn icon_file(id: &str) -> &'static [u8] {
            match id {
                $($id => include_bytes!(concat!("../../assets/icons/", $id, ".png")),)*
                _ => panic!("no icon for {id}"),
            }
        }
    };
}

icon_files![
    "village",
    "pillager_outpost",
    "stronghold",
    "mineshaft",
    "desert_pyramid",
    "jungle_temple",
    "swamp_hut",
    "igloo",
    "ocean_monument",
    "woodland_mansion",
    "ocean_ruin",
    "shipwreck",
    "buried_treasure",
    "ruined_portal",
    "ancient_city",
    "trail_ruins",
    "trial_chambers",
    "fortress",
    "bastion",
    "nether_fossil",
    "ruined_portal_nether",
    "end_city",
];

pub struct Icons(Vec<TextureHandle>);

impl Icons {
    pub fn load(ctx: &egui::Context) -> Self {
        Self(
            STRUCTURE_KINDS
                .iter()
                .map(|kind| {
                    let image = image::load_from_memory(icon_file(kind.id))
                        .unwrap_or_else(|err| panic!("icon of {}: {err}", kind.id))
                        .into_rgba8();
                    let size = [image.width() as usize, image.height() as usize];
                    let image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                    ctx.load_texture(kind.id, image, egui::TextureOptions::LINEAR)
                })
                .collect(),
        )
    }

    pub fn image(&self, kind: usize, size: f32) -> egui::Image<'_> {
        egui::Image::new(&self.0[kind]).fit_to_exact_size(vec2(size, size))
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum GridSpacing {
    /// round numbers fitting the zoom
    Auto,
    /// 16 blocks
    Chunks,
    /// 512 blocks
    Regions,
    Custom,
}

pub struct Grid {
    pub enabled: bool,
    pub spacing: GridSpacing,
    pub custom_spacing: i64,
    pub labels: bool,
    pub opacity: f32,
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            enabled: false,
            spacing: GridSpacing::Auto,
            custom_spacing: 1000,
            labels: true,
            opacity: 0.5,
        }
    }
}

/// The smallest of 1, 2, 5, 10, 20, 50, ... blocks that keeps lines `min_points` apart.
fn round_spacing(blocks_per_point: f64, min_points: f64) -> i64 {
    let mut magnitude = 1i64;
    loop {
        for factor in [1, 2, 5] {
            let spacing = factor * magnitude;
            if spacing as f64 / blocks_per_point >= min_points {
                return spacing;
            }
        }
        magnitude *= 10;
    }
}

impl Grid {
    /// Blocks between lines at this zoom. Fixed spacings are multiplied when zoomed out too far to
    /// show them, the scale bar shows what it is then.
    fn spacing_at(&self, blocks_per_point: f64) -> i64 {
        let fixed = match self.spacing {
            GridSpacing::Auto => return round_spacing(blocks_per_point, MIN_LINE_DISTANCE),
            GridSpacing::Chunks => 16,
            GridSpacing::Regions => 512,
            GridSpacing::Custom => self.custom_spacing.max(1),
        };
        let mut spacing = fixed;
        while (spacing as f64 / blocks_per_point) < MIN_FIXED_LINE_DISTANCE {
            spacing *= 2;
        }
        spacing
    }
}

/// `1 500` for 1500: coordinates get long and these are read at a glance.
fn format_blocks(blocks: i64) -> String {
    let digits = blocks.abs().to_string();
    let mut out = String::new();
    if blocks < 0 {
        out.push('-');
    }
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

impl Ui {
    /// Paints everything over the map, and asks for what is needed of it. `ui` covers the map.
    pub(super) fn show_overlays(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        if self.grid.enabled {
            self.paint_grid(ui, rect);
        }
        self.update_structures(rect);
        self.paint_markers(ui, rect);
        self.zoom_buttons(ui, rect);
    }

    fn update_structures(&mut self, rect: Rect) {
        let (min, max) = self.map.visible_blocks(rect);
        let mut jobs: Vec<Job> = Vec::new();
        self.markers.clear();
        for (kind, def) in STRUCTURE_KINDS.iter().enumerate() {
            self.too_far[kind] = false;
            if !self.shown_kinds[kind] || def.dimension != self.dimension {
                continue;
            }
            match self.queries.visible(kind, min, max) {
                Some((wanted, known)) => {
                    jobs.extend(wanted);
                    self.markers.extend(known);
                }
                None => self.too_far[kind] = true,
            }
        }
        // rings first, they are few and the rest is nearest first per kind
        jobs.sort_by_key(|job| !matches!(job, Job::Rings { .. }));
        self.queries.update(&jobs);
    }

    fn paint_markers(&self, ui: &egui::Ui, rect: Rect) {
        let Some(icons) = &self.icons else { return };
        let painter = ui.painter_at(rect);
        // a mouse or the last touch shows what is under it
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let mut label = None;
        for found in &self.markers {
            let at = self.map.block_to_screen(rect, [found.x as f64, found.z as f64]);
            if !rect.expand(MARKER_SIZE).contains(at) {
                continue;
            }
            let marker = Rect::from_center_size(at, vec2(MARKER_SIZE, MARKER_SIZE));
            icons.image(found.kind, MARKER_SIZE).paint_at(ui, marker);
            if pointer.is_some_and(|p| marker.contains(p)) {
                let text = format!("{}\nx {} z {}", STRUCTURE_KINDS[found.kind].name, found.x, found.z);
                label = Some((marker.right_top(), text));
            }
        }
        if let Some(search) = &self.queries.search {
            for result in &search.results {
                let at = self.map.block_to_screen(rect, [result[0] as f64, result[1] as f64]);
                painter.circle_stroke(at, MARKER_SIZE * 0.75, Stroke::new(2.5, SEARCH_COLOR));
            }
        }
        if let Some((at, text)) = label {
            let font = FontId::proportional(13.0);
            let galley = painter.layout_no_wrap(text, font, Color32::WHITE);
            let background = Rect::from_min_size(at + vec2(4.0, 0.0), galley.size()).expand(3.0);
            painter.rect_filled(background, 3.0, Color32::from_black_alpha(200));
            painter.galley(at + vec2(4.0, 0.0), galley, Color32::WHITE);
        }
    }

    fn paint_grid(&self, ui: &egui::Ui, rect: Rect) {
        let painter = ui.painter_at(rect);
        let blocks_per_point = self.map.blocks_per_point();
        let spacing = self.grid.spacing_at(blocks_per_point);
        let border = (WORLD_BORDER_QUARTS as f64 * 4.0).min(i32::MAX as f64);
        let (min, max) = self.map.visible_blocks(rect);
        let line_alpha = (self.grid.opacity * 255.0) as u8;
        let line = Color32::from_white_alpha(line_alpha);
        let axis = Stroke::new(2.0, Color32::from_white_alpha(line_alpha.saturating_add(60)));
        let thin = Stroke::new(1.0, line);
        let font = FontId::monospace(11.0);
        // not past the world border
        let range = |lo: f64, hi: f64| {
            let (lo, hi) = (lo.max(-border), hi.min(border));
            ((lo / spacing as f64).ceil() as i64)..=((hi / spacing as f64).floor() as i64)
        };
        let [lo_y, hi_y] = [
            self.map.block_to_screen(rect, [0.0, min[1].max(-border)]).y,
            self.map.block_to_screen(rect, [0.0, max[1].min(border)]).y,
        ];
        let [lo_x, hi_x] = [
            self.map.block_to_screen(rect, [min[0].max(-border), 0.0]).x,
            self.map.block_to_screen(rect, [max[0].min(border), 0.0]).x,
        ];
        let label = |at: Pos2, align: Align2, text: String| {
            let shape = painter.layout_no_wrap(text, font.clone(), Color32::WHITE);
            let background = align.anchor_size(at, shape.size());
            painter.rect_filled(background.expand(1.0), 2.0, Color32::from_black_alpha(140));
            painter.galley(background.min, shape, Color32::WHITE);
        };
        for i in range(min[0], max[0]) {
            let block = i * spacing;
            let x = self.map.block_to_screen(rect, [block as f64, 0.0]).x;
            let stroke = if block == 0 { axis } else { thin };
            painter.line_segment([pos2(x, lo_y), pos2(x, hi_y)], stroke);
            if self.grid.labels {
                label(pos2(x + 2.0, lo_y + 2.0), Align2::LEFT_TOP, format_blocks(block));
            }
        }
        for i in range(min[1], max[1]) {
            let block = i * spacing;
            let y = self.map.block_to_screen(rect, [0.0, block as f64]).y;
            let stroke = if block == 0 { axis } else { thin };
            painter.line_segment([pos2(lo_x, y), pos2(hi_x, y)], stroke);
            if self.grid.labels {
                label(pos2(lo_x + 2.0, y + 2.0), Align2::LEFT_TOP, format_blocks(block));
            }
        }
        // a scale bar for the distance between lines
        let length = (spacing as f64 / blocks_per_point) as f32;
        let start = rect.left_bottom() + vec2(16.0, -18.0);
        let end = start + vec2(length.min(rect.width() - 32.0), 0.0);
        painter.line_segment([start, end], Stroke::new(3.0, Color32::WHITE));
        label(
            start + vec2(0.0, -4.0),
            Align2::LEFT_BOTTOM,
            format!("{} blocks", format_blocks(spacing)),
        );
    }

    fn zoom_buttons(&mut self, ui: &egui::Ui, rect: Rect) {
        egui::Area::new(egui::Id::unique("zoom buttons"))
            .anchor(Align2::RIGHT_BOTTOM, vec2(-12.0, -12.0))
            .constrain_to(rect)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let size = vec2(36.0, 36.0);
                    let button = |ui: &mut egui::Ui, text: &str| {
                        ui.add_sized(size, egui::Button::new(egui::RichText::new(text).size(20.0)))
                            .clicked()
                    };
                    if button(ui, "+") {
                        self.map.zoom_by(-1.0);
                    }
                    if button(ui, "−") {
                        self.map.zoom_by(1.0);
                    }
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_spacing_is_round_and_keeps_lines_apart() {
        assert_eq!(round_spacing(1.0, 80.0), 100);
        assert_eq!(round_spacing(2.0, 80.0), 200);
        assert_eq!(round_spacing(4.0, 80.0), 500);
        assert_eq!(round_spacing(0.01, 80.0), 1);
        for zoom in -6..20 {
            let blocks_per_point = (zoom as f64).exp2();
            assert!(round_spacing(blocks_per_point, 80.0) as f64 / blocks_per_point >= 80.0);
        }
    }

    #[test]
    fn fixed_spacing_gets_coarser_only_when_needed() {
        let grid = Grid {
            spacing: GridSpacing::Regions,
            ..Default::default()
        };
        assert_eq!(grid.spacing_at(1.0), 512);
        assert_eq!(grid.spacing_at(100.0), 2048);
    }

    #[test]
    fn numbers_are_readable() {
        assert_eq!(format_blocks(0), "0");
        assert_eq!(format_blocks(-512), "-512");
        assert_eq!(format_blocks(1500), "1 500");
        assert_eq!(format_blocks(-29999984), "-29 999 984");
    }

    #[test]
    fn every_kind_has_an_icon() {
        for kind in STRUCTURE_KINDS {
            assert!(image::load_from_memory(icon_file(kind.id)).is_ok(), "{}", kind.id);
        }
    }
}
