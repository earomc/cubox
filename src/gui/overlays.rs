//! What is painted over the map: structure markers, the distance grid and the zoom buttons.

use std::collections::HashMap;

use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, TextureHandle, pos2, vec2};

use crate::{
    dimension::Dimension,
    gui::{
        Ui,
        finder::{Job, MarkerSearch, Occupied},
    },
    structures::{Found, STRUCTURE_KINDS, rarity},
    tile_renderer::WORLD_BORDER_QUARTS,
};

/// Auto grid lines stay at least this far apart, in points.
const MIN_LINE_DISTANCE: f64 = 80.0;
/// Fixed grid spacings get coarser when lines would be closer than this, in points.
const MIN_FIXED_LINE_DISTANCE: f64 = 12.0;
const SEARCH_COLOR: Color32 = Color32::from_rgb(255, 225, 60);

/// The space between markers, in points.
pub const MARKER_GAPS: std::ops::RangeInclusive<f32> = 1.0..=100.0;
pub const DEFAULT_MARKER_GAP: f32 = 2.0;

/// While answers come in, markers are placed anew at most this often per marker, and after at most
/// this many seconds.
const MARKERS_PLACED_PER_SECOND: f64 = 100_000.0;
const MAX_PLACING_WAIT: f64 = 0.5;

/// What the markers on the screen depend on besides the answers about them.
#[derive(PartialEq)]
pub struct MarkerView {
    min: [f64; 2],
    max: [f64; 2],
    target: f64,
    kinds: Vec<bool>,
    dimension: Dimension,
    overlap: bool,
    rare_first: bool,
    distance: f64,
    max_tries: u32,
    max_markers: usize,
}

/// The markers that are shown: those with space around them, the rarest kinds first. Depends only
/// on the structures and the distance, so moving the view doesn't shuffle them.
#[derive(Default)]
pub struct Placed {
    pub markers: Vec<Found>,
    /// the rarity of the markers in each square of `cell` blocks
    grid: HashMap<(i64, i64), Vec<(usize, Found)>>,
    cell: f64,
}

impl Placed {
    /// `distance` is in blocks. 0 lets them overlap, which keeps all but exact duplicates, up to
    /// `max_overlapping`. Without `rare_first` all kinds have the same chance at a spot.
    pub fn new(mut candidates: Vec<Found>, distance: f64, rare_first: bool, max_overlapping: usize) -> Self {
        let rank = |found: &Found| if rare_first { rarity(found.kind) } else { 0 };
        let hash = |found: &Found| {
            (found.x as u32 as u64)
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .wrapping_add((found.z as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
        };
        candidates.sort_unstable_by_key(|found| (rank(found), hash(found), found.x, found.z, found.kind));
        candidates.dedup();
        if distance <= 0.0 {
            candidates.truncate(max_overlapping);
            return Self {
                markers: candidates,
                ..Default::default()
            };
        }
        let cell = distance;
        let at = |x: f64, z: f64| ((x / cell).floor() as i64, (z / cell).floor() as i64);
        let mut placed = Self {
            markers: Vec::with_capacity(candidates.len()),
            grid: HashMap::new(),
            cell,
        };
        for found in candidates {
            let (cx, cz) = at(found.x as f64, found.z as f64);
            let free = (cx - 1..=cx + 1).all(|x| {
                (cz - 1..=cz + 1).all(|z| {
                    placed.grid.get(&(x, z)).is_none_or(|near| {
                        near.iter().all(|(_, other)| {
                            ((other.x - found.x).abs().max((other.z - found.z).abs()) as f64) >= distance
                        })
                    })
                })
            });
            if free {
                placed.grid.entry((cx, cz)).or_default().push((rank(&found), found));
                placed.markers.push(found);
            }
        }
        placed
    }
}

impl Occupied for Placed {
    fn has_marker(&self, rarity: usize, min: [f64; 2], max: [f64; 2]) -> bool {
        // markers may overlap, nothing is in the way of any other
        if self.grid.is_empty() {
            return false;
        }
        let range = |lo: f64, hi: f64| (lo / self.cell).floor() as i64..=(hi / self.cell).floor() as i64;
        range(min[0], max[0]).any(|x| {
            range(min[1], max[1]).any(|z| {
                self.grid.get(&(x, z)).is_some_and(|near| {
                    near.iter().any(|(rank, found)| {
                        *rank <= rarity
                            && (min[0]..max[0]).contains(&(found.x as f64))
                            && (min[1]..max[1]).contains(&(found.z as f64))
                    })
                })
            })
        })
    }
}

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
    "abandoned_camp",
    "fortress",
    "bastion",
    "nether_fossil",
    "ruined_portal_nether",
    "end_city",
    "end_city_ship",
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
        self.update_structures(ui.ctx(), rect);
        self.paint_markers(ui, rect);
        if self.zoom_buttons_shown {
            self.zoom_buttons(ui, rect);
        }
    }

    fn update_structures(&mut self, ctx: &egui::Context, rect: Rect) {
        let (min, max) = self.map.visible_blocks(rect);
        let blocks_per_point = self.map.blocks_per_point();
        let distance_points = (self.advanced.marker_size + self.marker_gap) as f64;
        // what is just off the screen can still be in the way of what is on it
        let margin = distance_points * blocks_per_point;
        let (min, max) = (min.map(|v| v - margin), max.map(|v| v + margin));
        let kinds = STRUCTURE_KINDS
            .iter()
            .enumerate()
            .filter(|(kind, def)| self.shown_kinds[*kind] && def.dimension == self.dimension)
            .count();
        let target = if self.marker_overlap {
            // as many as asked for, whatever there is room for
            self.max_markers as f64 / kinds.max(1) as f64
        } else {
            // what fits on the part of the screen with world on it, and some more to choose from
            let edge = WORLD_BORDER_QUARTS as f64 * 4.0;
            let side = |lo: f64, hi: f64| (hi.min(edge) - lo.max(-edge)).max(0.0) / blocks_per_point;
            let fit = side(min[0], max[0]) * side(min[1], max[1]) / (distance_points * distance_points);
            let choice = (self.advanced.search_fineness * self.advanced.search_fineness) as f64;
            // with more kinds the others fill the gaps: half the work for 2% fewer markers
            fit * if kinds >= 3 { choice / 2.0 } else { choice }
        };
        let view = MarkerView {
            min,
            max,
            target,
            kinds: self.shown_kinds.clone(),
            dimension: self.dimension,
            overlap: self.marker_overlap,
            rare_first: self.prioritize_rare,
            distance: distance_points * blocks_per_point,
            max_tries: self.advanced.search_tries,
            max_markers: self.max_markers,
        };
        // only when something changed, there can be a lot of them. Answers keep coming in while they
        // are looked for, going through all markers for each would take the time from looking.
        let now = ctx.input(|input| input.time);
        let revision = self.queries.revision();
        let wait = (self.markers.len() as f64 / MARKERS_PLACED_PER_SECOND).min(MAX_PLACING_WAIT);
        let answered = revision != self.placed_revision && now - self.placed_at >= wait;
        if answered || self.marker_view.as_ref() != Some(&view) {
            self.place_markers(&view);
            self.marker_view = Some(view);
            self.placed_revision = revision;
            self.placed_at = now;
        }
        self.queries.update(&self.marker_jobs);
    }

    fn place_markers(&mut self, view: &MarkerView) {
        let search = MarkerSearch {
            target: view.target,
            max_tries: view.max_tries,
        };
        let mut rounds: Vec<Vec<std::vec::IntoIter<Job>>> = Vec::new();
        let mut known = Vec::new();
        for (kind, def) in STRUCTURE_KINDS.iter().enumerate() {
            self.sampled[kind] = false;
            if !self.shown_kinds[kind] || def.dimension != self.dimension {
                continue;
            }
            let visible = self.queries.visible(kind, view.min, view.max, search, &self.placed);
            self.sampled[kind] = visible.sampled;
            known.extend(visible.known);
            for (round, jobs) in visible.jobs.into_iter().enumerate() {
                if rounds.len() <= round {
                    rounds.resize_with(round + 1, Vec::new);
                }
                rounds[round].push(jobs.into_iter());
            }
        }
        // a round for all first. kinds take turns, or those further down the list would wait for
        // all of the others
        self.marker_jobs.clear();
        for mut per_kind in rounds {
            loop {
                let before = self.marker_jobs.len();
                self.marker_jobs.extend(per_kind.iter_mut().filter_map(Iterator::next));
                if self.marker_jobs.len() == before {
                    break;
                }
            }
        }
        let distance = if view.overlap { 0.0 } else { view.distance };
        self.placed = Placed::new(known, distance, view.rare_first, view.max_markers);
        self.markers.clear();
        self.markers.extend_from_slice(&self.placed.markers);
    }

    fn paint_markers(&self, ui: &egui::Ui, rect: Rect) {
        let Some(icons) = &self.icons else { return };
        let size = self.advanced.marker_size;
        let painter = ui.painter_at(rect);
        // a mouse or the last touch shows what is under it
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let mut label = None;
        // the rarest last, on top if they overlap
        for found in self.markers.iter().rev() {
            let at = self.map.block_to_screen(rect, [found.x as f64, found.z as f64]);
            if !rect.expand(size).contains(at) {
                continue;
            }
            let marker = Rect::from_center_size(at, vec2(size, size));
            icons.image(found.kind, size).paint_at(ui, marker);
            if pointer.is_some_and(|p| marker.contains(p)) {
                let text = format!("{}\nx {} z {}", STRUCTURE_KINDS[found.kind].name, found.x, found.z);
                label = Some((marker.right_top(), text));
            }
        }
        if let Some(search) = &self.queries.search {
            for result in &search.results {
                let at = self.map.block_to_screen(rect, [result[0] as f64, result[1] as f64]);
                painter.circle_stroke(at, size * 0.75, Stroke::new(2.5, SEARCH_COLOR));
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

    fn found(id: &str, x: i32, z: i32) -> Found {
        Found {
            kind: STRUCTURE_KINDS.iter().position(|kind| kind.id == id).unwrap(),
            x,
            z,
        }
    }

    #[test]
    fn markers_keep_their_distance_and_use_the_space_there_is() {
        let candidates: Vec<Found> = (0..3000)
            .map(|i: i32| found("village", i.wrapping_mul(7919) % 4000, i.wrapping_mul(104_729) % 4000))
            .collect();
        let distance = 100.0;
        let placed = Placed::new(candidates.clone(), distance, true, 100).markers;
        for (i, a) in placed.iter().enumerate() {
            for b in &placed[i + 1..] {
                assert!((a.x - b.x).abs().max((a.z - b.z).abs()) as f64 >= distance);
            }
        }
        // whatever is left out is next to one that is in: nothing with space around it is missing
        for candidate in &candidates {
            assert!(
                placed
                    .iter()
                    .any(|p| (p.x - candidate.x).abs().max((p.z - candidate.z).abs()) < 100)
            );
        }
        // the same whatever order they come in
        let mut reversed = candidates;
        reversed.reverse();
        assert_eq!(Placed::new(reversed, distance, true, 100).markers, placed);
    }

    #[test]
    fn the_rare_kind_wins_the_space_but_far_apart_both_stay() {
        let (rare, common) = (found("woodland_mansion", 0, 0), found("village", 10, 10));
        assert_eq!(Placed::new(vec![common, rare], 100.0, true, 100).markers, vec![rare]);
        let far = found("village", 500, 500);
        assert_eq!(Placed::new(vec![far, rare], 100.0, true, 100).markers, vec![rare, far]);
        // may overlap
        assert_eq!(Placed::new(vec![common, rare], 0.0, true, 100).markers.len(), 2);
    }

    #[test]
    fn overlapping_markers_are_limited_and_stop_nothing() {
        let many: Vec<Found> = (0..3100).map(|i| found("village", i, 0)).collect();
        let placed = Placed::new(many, 0.0, true, 3000);
        assert_eq!(placed.markers.len(), 3000);
        // a whole world of a search: quickly, however far apart the corners are
        assert!(!placed.has_marker(0, [-3e7; 2], [3e7; 2]));
    }

    #[test]
    fn without_priority_all_kinds_get_their_share() {
        let candidates: Vec<Found> = (0..400)
            .flat_map(|i: i32| {
                let (x, z) = (i.wrapping_mul(7919) % 3000, i.wrapping_mul(104_729) % 3000);
                [found("woodland_mansion", x, z), found("village", x + 7, z + 3)]
            })
            .collect();
        let share = |rare_first| {
            let placed = Placed::new(candidates.clone(), 100.0, rare_first, 100).markers;
            let villages = placed.iter().filter(|p| p.kind == found("village", 0, 0).kind).count();
            villages as f64 / placed.len() as f64
        };
        assert!(share(true) < 0.1, "{}", share(true));
        assert!((0.3..0.7).contains(&share(false)), "{}", share(false));
        // and markers of any kind stop the looking then
        let placed = Placed::new(vec![found("village", 50, 50)], 100.0, false, 100);
        assert!(placed.has_marker(0, [0.0; 2], [100.0; 2]));
    }

    #[test]
    fn only_as_rare_markers_stop_looking_further() {
        let placed = Placed::new(vec![found("village", 50, 50)], 100.0, true, 100);
        let village = rarity(found("village", 0, 0).kind);
        let mansion = rarity(found("woodland_mansion", 0, 0).kind);
        assert!(placed.has_marker(village, [0.0; 2], [100.0; 2]));
        assert!(!placed.has_marker(mansion, [0.0; 2], [100.0; 2]));
        assert!(!placed.has_marker(village, [200.0; 2], [300.0; 2]));
    }

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
