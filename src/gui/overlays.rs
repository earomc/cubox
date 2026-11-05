//! What is painted over the map: slime chunks, structure markers, the distance grid and the zoom buttons.

use std::collections::HashMap;

use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, TextureHandle, pos2, vec2};

use crate::{
    dimension::Dimension,
    features::ring_gateways,
    gui::{
        Ui,
        finder::{Job, MarkerSearch, Occupied},
    },
    slime::is_slime_chunk,
    structures::{Found, MARKER_KINDS, ore_of, rarity},
    tile_renderer::WORLD_BORDER_QUARTS,
};

/// Grid lines fade out from this far apart in points, and aren't drawn at all from the second.
const LINE_FADE_POINTS: [f64; 2] = [8.0, 3.0];
/// No grid step is larger than the distance from the middle of the world to its border: the lines at
/// 0 and on both sides of it are then in the world, and the scale bar is a distance between two lines.
const MAX_GRID_STEP: i64 = WORLD_BORDER_QUARTS as i64 * 4;
/// Minor lines fade in from where they are this share of the gap between major lines apart, and are at
/// their full `minor_opacity` when they reach it. Then they become the major lines and jump to full
/// opacity, so the two kinds stay easy to tell apart.
const MINOR_FADE_FROM: f64 = 0.25;
/// Coordinates of grid lines keep this far apart in points, the lines between go without.
const LABEL_POINTS: f64 = 70.0;

const SLIME_GREEN: [u8; 3] = [110, 230, 60];
/// Slime chunks are drawn from textures of this many chunks per side, one pixel per chunk, made the
/// first time they come on the screen. Then a frame only draws a few hundred of them, however many
/// chunks there are.
const SLIME_BLOCK: i32 = 64;
/// How many of those textures are kept, 16 KiB each. A 4K screen at a pixel per chunk needs about 2000.
const SLIME_BLOCKS_KEPT: usize = 4096;
/// Textures made in one frame at most, about 1.5 ms. Zoomed far out a screen needs hundreds at once,
/// they then come in over a few frames from the middle out instead of holding up one.
const SLIME_BLOCKS_PER_FRAME: usize = 96;

/// The space between markers, in points.
pub const MARKER_GAPS: std::ops::RangeInclusive<f32> = 0.0..=100.0;
pub const DEFAULT_MARKER_GAP: f32 = 2.0;

/// While answers come in, markers are placed anew at most this often per marker, and after at most
/// this many seconds.
const MARKERS_PLACED_PER_SECOND: f64 = 100_000.0;
const MAX_PLACING_WAIT: f64 = 0.5;

/// What the markers on the screen depend on besides the answers about them.
#[derive(PartialEq)]
pub struct MarkerView {
    seed: i64,
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

/// The structure icons in `assets/icons`. Replace those files and rebuild to change them. They are
/// Minecraft textures owned by Mojang and not under cubox's license, see `assets/icons/NOTICE`.
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
    "spawn",
    "end_gateway",
    "end_gateway_scattered",
    "geode",
    "copper_vein",
    "iron_vein",
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

/// The icon two pixels larger on every side, with two outlines around everything visible: the dark
/// of egui's windows and around that the gray of its buttons, so it stands out on any biome.
fn outlined(icon: &image::RgbaImage) -> image::RgbaImage {
    let visuals = egui::Visuals::dark();
    let rings =
        [visuals.window_fill(), visuals.widgets.inactive.fg_stroke.color].map(|color| image::Rgba(color.to_array()));
    let visible = |x: i64, z: i64| {
        x >= 0
            && z >= 0
            && icon
                .get_pixel_checked(x as u32, z as u32)
                .is_some_and(|pixel| pixel[3] > 0)
    };
    // how many pixels away the nearest visible one is, diagonals count as one
    let near = |x: i64, z: i64, distance: i64| {
        (-distance..=distance).any(|dx| (-distance..=distance).any(|dz| visible(x + dx, z + dz)))
    };
    let pad = rings.len() as u32;
    image::RgbaImage::from_fn(icon.width() + 2 * pad, icon.height() + 2 * pad, |x, z| {
        let (x, z) = (x as i64 - pad as i64, z as i64 - pad as i64);
        if visible(x, z) {
            return *icon.get_pixel(x as u32, z as u32);
        }
        (1..)
            .zip(rings)
            .find(|&(distance, _)| near(x, z, distance))
            .map_or(image::Rgba([0, 0, 0, 0]), |(_, ring)| ring)
    })
}

pub struct Icons(Vec<TextureHandle>);

impl Icons {
    pub fn load(ctx: &egui::Context) -> Self {
        Self(
            MARKER_KINDS
                .iter()
                .map(|kind| {
                    let image = image::load_from_memory(icon_file(kind.id))
                        .unwrap_or_else(|err| panic!("icon of {}: {err}", kind.id))
                        .into_rgba8();
                    let image = outlined(&image);
                    let size = [image.width() as usize, image.height() as usize];
                    let image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                    ctx.load_texture(kind.id, image, egui::TextureOptions::NEAREST)
                })
                .collect(),
        )
    }

    pub fn image(&self, kind: usize, size: f32) -> egui::Image<'_> {
        egui::Image::new(&self.0[kind]).fit_to_exact_size(vec2(size, size))
    }
}

/// The distances auto spacing chooses from, in blocks. Zooming picks the smallest that keeps the lines
/// far enough apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GridSteps {
    OneTwoFive,
    PowersOfTwo,
    PowersOfTen,
    ChunksAndRegions,
}

impl GridSteps {
    pub const ALL: [Self; 4] = [
        Self::OneTwoFive,
        Self::PowersOfTwo,
        Self::PowersOfTen,
        Self::ChunksAndRegions,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::OneTwoFive => "1, 2, 5",
            Self::PowersOfTwo => "Powers of 2",
            Self::PowersOfTen => "Powers of 10",
            Self::ChunksAndRegions => "Chunks & regions",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::OneTwoFive => "1, 2, 5, 10, 20, 50 ... blocks: round coordinates",
            Self::PowersOfTwo => "1, 2, 4, 8, 16 ... blocks: always twice as far, on chunk borders from 16",
            Self::PowersOfTen => "1, 10, 100, 1000 ... blocks",
            Self::ChunksAndRegions => "1, 4, 16 (a chunk), 64, 512 (a region), then 8 times as far each",
        }
    }

    /// Its steps in blocks, smallest first, up to `MAX_GRID_STEP`.
    fn steps(self) -> Vec<i64> {
        let next = |step: i64| match self {
            Self::OneTwoFive => {
                let mut digit = step;
                while digit % 10 == 0 {
                    digit /= 10;
                }
                if digit == 2 { step / 2 * 5 } else { step * 2 }
            }
            Self::PowersOfTwo => step * 2,
            Self::PowersOfTen => step * 10,
            Self::ChunksAndRegions => match step {
                1 => 4,
                4 => 16,
                16 => 64,
                64 => 512,
                _ => step * 8,
            },
        };
        std::iter::successors(Some(1), |&step| Some(next(step)).filter(|&step| step <= MAX_GRID_STEP)).collect()
    }

    /// The smallest step whose lines are at least `min_points` apart, or the largest.
    fn spacing_at(self, blocks_per_point: f64, min_points: f64) -> i64 {
        let steps = self.steps();
        steps
            .iter()
            .copied()
            .find(|&step| step as f64 / blocks_per_point >= min_points)
            .unwrap_or(steps[steps.len() - 1])
    }

    /// The largest smaller step that divides `major`, for minor lines that fall on the major ones.
    fn minor_of(self, major: i64) -> Option<i64> {
        self.steps()
            .into_iter()
            .rev()
            .find(|&step| step < major && major % step == 0)
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum GridSpacing {
    /// round numbers fitting the zoom
    Auto,
    /// the same at every zoom
    Fixed,
}

pub struct SlimeChunks {
    pub shown: bool,
    pub opacity: f32,
    /// textures of `SLIME_BLOCK` chunks per side by seed and block, with when they were last drawn
    blocks: HashMap<(i64, i32, i32), (TextureHandle, u64)>,
    frame: u64,
    /// textures made this frame
    made: usize,
}

impl Default for SlimeChunks {
    fn default() -> Self {
        Self {
            shown: false,
            opacity: 0.6,
            blocks: HashMap::new(),
            frame: 0,
            made: 0,
        }
    }
}

impl SlimeChunks {
    /// The texture of a block of chunks, made if it isn't there yet and this frame may make more.
    fn block(&mut self, ctx: &egui::Context, seed: i64, bx: i32, bz: i32) -> Option<egui::TextureId> {
        let frame = self.frame;
        let key = (seed, bx, bz);
        if !self.blocks.contains_key(&key) {
            if self.made >= SLIME_BLOCKS_PER_FRAME {
                return None;
            }
            self.made += 1;
        }
        let (texture, used) = self.blocks.entry(key).or_insert_with(|| {
            let [r, g, b] = SLIME_GREEN;
            let mut rgba = vec![0u8; (SLIME_BLOCK * SLIME_BLOCK * 4) as usize];
            for (i, pixel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let (x, z) = (i as i32 % SLIME_BLOCK, i as i32 / SLIME_BLOCK);
                if is_slime_chunk(seed, bx * SLIME_BLOCK + x, bz * SLIME_BLOCK + z) {
                    pixel.copy_from_slice(&[r, g, b, 255]);
                }
            }
            let size = [SLIME_BLOCK as usize; 2];
            let image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
            (
                ctx.load_texture(format!("slime {seed} {bx} {bz}"), image, egui::TextureOptions::NEAREST),
                frame,
            )
        });
        *used = frame;
        Some(texture.id())
    }

    /// Drops the textures drawn longest ago if there are too many, never one of this frame.
    fn evict(&mut self) {
        if self.blocks.len() <= SLIME_BLOCKS_KEPT {
            return;
        }
        let mut by_age: Vec<(u64, (i64, i32, i32))> = self.blocks.iter().map(|(k, (_, used))| (*used, *k)).collect();
        by_age.sort_unstable_by_key(|(used, _)| *used);
        let excess = self.blocks.len() - SLIME_BLOCKS_KEPT;
        for (used, key) in by_age.into_iter().take(excess) {
            if used == self.frame {
                break;
            }
            self.blocks.remove(&key);
        }
    }
}

/// Whether slime chunks this many pixels wide are drawn: as long as a pixel isn't bigger than a chunk.
pub fn slime_shown_at(chunk_pixels: f64) -> bool {
    chunk_pixels >= 1.0
}

pub struct Grid {
    pub enabled: bool,
    pub spacing: GridSpacing,
    /// what auto spacing chooses from
    pub steps: GridSteps,
    pub fixed_spacing: i64,
    /// how far apart in points auto lines are at least
    pub min_distance: f64,
    pub labels: bool,
    pub opacity: f32,
    /// how strong minor lines get at most, as a share of `opacity`
    pub minor_opacity: f32,
    /// drawn after the structure markers instead of before them
    pub over_markers: bool,
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            enabled: false,
            spacing: GridSpacing::Auto,
            steps: GridSteps::PowersOfTwo,
            fixed_spacing: 512,
            min_distance: 80.0,
            labels: true,
            opacity: 0.5,
            minor_opacity: 0.3,
            over_markers: false,
        }
    }
}

impl Grid {
    /// Blocks between lines at this zoom.
    fn spacing_at(&self, blocks_per_point: f64) -> i64 {
        match self.spacing {
            GridSpacing::Auto => self.steps.spacing_at(blocks_per_point, self.min_distance),
            GridSpacing::Fixed => self.fixed_spacing.max(1),
        }
    }

    /// The minor lines between major lines `spacing` apart and how visible they are, in auto spacing.
    fn minor_at(&self, spacing: i64, blocks_per_point: f64) -> Option<(i64, f32)> {
        if self.spacing != GridSpacing::Auto {
            return None;
        }
        let minor = self.steps.minor_of(spacing)?;
        let points = minor as f64 / blocks_per_point;
        let fade = ((points / self.min_distance - MINOR_FADE_FROM) / (1.0 - MINOR_FADE_FROM)).clamp(0.0, 1.0);
        Some((minor, fade as f32 * self.minor_opacity * line_visibility(points)))
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_fixed_spacing(&mut self, blocks: i64) {
        self.spacing = GridSpacing::Fixed;
        self.fixed_spacing = blocks;
    }
}

/// How visible grid lines this many points apart are drawn, from 1 to 0 where they would only be a
/// blur.
pub fn line_visibility(points: f64) -> f32 {
    let [full, none] = LINE_FADE_POINTS;
    ((points - none) / (full - none)).clamp(0.0, 1.0) as f32
}

/// Scale of the markers: past where the whole world is visible they don't get further apart in
/// the world, or fewer would fit the less of the screen the world takes. In blocks per point.
fn marker_blocks_per_point(blocks_per_point: f64, view_points: [f32; 2]) -> f64 {
    let whole_world = WORLD_BORDER_QUARTS as f64 * 8.0 / view_points[0].min(view_points[1]).max(1.0) as f64;
    blocks_per_point.min(whole_world)
}

/// How many markers fit in the world between `min` and `max` in blocks, `distance` apart in blocks.
fn markers_that_fit(min: [f64; 2], max: [f64; 2], distance: f64) -> f64 {
    let edge = WORLD_BORDER_QUARTS as f64 * 4.0;
    let side = |lo: f64, hi: f64| (hi.min(edge) - lo.max(-edge)).max(0.0);
    side(min[0], max[0]) * side(min[1], max[1]) / (distance * distance)
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
        if self.slime.shown && self.dimension == Dimension::Overworld {
            self.paint_slime(ui, rect);
        }
        if self.grid.enabled && !self.grid.over_markers {
            self.paint_grid(ui, rect);
        }
        self.update_structures(ui.ctx(), rect);
        self.paint_markers(ui, rect);
        if self.grid.enabled && self.grid.over_markers {
            self.paint_grid(ui, rect);
        }
        if self.zoom_buttons_shown {
            self.zoom_buttons(ui, rect);
        }
    }

    fn update_structures(&mut self, ctx: &egui::Context, rect: Rect) {
        let (min, max) = self.map.visible_blocks(rect);
        let blocks_per_point = self.map.blocks_per_point();
        let distance_points = (self.extras.marker_size + self.marker_gap) as f64;
        let marker_scale = marker_blocks_per_point(blocks_per_point, rect.size().into());
        // what is just off the screen can still be in the way of what is on it
        let margin = distance_points * blocks_per_point;
        let (min, max) = (min.map(|v| v - margin), max.map(|v| v + margin));
        let kinds = MARKER_KINDS
            .iter()
            .enumerate()
            .filter(|(kind, def)| self.shown_kinds[*kind] && def.dimension == self.dimension)
            .count();
        let target = if self.marker_overlap {
            // as many as asked for, whatever there is room for
            self.max_markers as f64 / kinds.max(1) as f64
        } else {
            // what fits on the part of the screen with world on it, and some more to choose from
            let fit = markers_that_fit(min, max, distance_points * marker_scale);
            let choice = (self.extras.marker_fineness * self.extras.marker_fineness) as f64;
            // with more kinds the others fill the gaps: half the work for 2% fewer markers
            fit * if kinds >= 3 { choice / 2.0 } else { choice }
        };
        let view = MarkerView {
            seed: self.seed,
            min,
            max,
            target,
            kinds: self.shown_kinds.clone(),
            dimension: self.dimension,
            overlap: self.marker_overlap,
            rare_first: self.prioritize_rare,
            distance: distance_points * marker_scale,
            max_tries: self.extras.marker_tries,
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
        for (kind, def) in MARKER_KINDS.iter().enumerate() {
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
        let size = self.extras.marker_size;
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
                let name = MARKER_KINDS[found.kind].name;
                // the game then looks for a suitable block nearby, in chunks that aren't built here
                let about = match MARKER_KINDS[found.kind].id {
                    "spawn" => " (within about 80 blocks)".to_owned(),
                    "copper_vein" | "iron_vein" => match ore_of(MARKER_KINDS[found.kind].id) {
                        Some(ore) => format!("\nore at y {} to {}", ore.heights().start, ore.heights().end),
                        None => String::new(),
                    },
                    "end_gateway" => ring_gateways(self.seed)
                        .iter()
                        .position(|spot| *spot == [found.x, found.z])
                        .map_or(String::new(), |number| {
                            format!("\nopens as number {} of 20", number + 1)
                        }),
                    _ => String::new(),
                };
                let text = format!("{name}{about}\nx {} z {}", found.x, found.z);
                label = Some((marker.right_top(), text));
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

    /// The slime chunks on the screen, from textures of blocks of them.
    fn paint_slime(&mut self, ui: &egui::Ui, rect: Rect) {
        let chunk_pixels = 16.0 / self.map.blocks_per_point() * ui.pixels_per_point() as f64;
        if !slime_shown_at(chunk_pixels) {
            return;
        }
        let alpha = (self.slime.opacity * 255.0) as u8;
        self.slime.frame += 1;
        self.slime.made = 0;

        // not past the world border, which is the edge of a chunk
        let border = WORLD_BORDER_QUARTS as f64 * 4.0;
        let world = Rect::from_min_max(
            self.map.block_to_screen(rect, [-border; 2]),
            self.map.block_to_screen(rect, [border; 2]),
        );
        let painter = ui.painter_at(rect.intersect(world));
        let (min, max) = self.map.visible_blocks(rect);
        let last_chunk = (border / 16.0) as i32 - 1;
        let chunks = |lo: f64, hi: f64| {
            ((lo / 16.0).floor() as i32).max(-last_chunk - 1)..=((hi / 16.0).floor() as i32).min(last_chunk)
        };
        let (xs, zs) = (chunks(min[0], max[0]), chunks(min[1], max[1]));
        let chunk_screen = |x: i32, z: i32| self.map.block_to_screen(rect, [x as f64 * 16.0, z as f64 * 16.0]);

        let tint = Color32::from_white_alpha(alpha);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let blocks =
            |r: &std::ops::RangeInclusive<i32>| r.start().div_euclid(SLIME_BLOCK)..=r.end().div_euclid(SLIME_BLOCK);
        let mut visible: Vec<(i32, i32)> = blocks(&zs).flat_map(|bz| blocks(&xs).map(move |bx| (bx, bz))).collect();
        // the middle first, the textures that aren't there yet are made over the next frames
        let middle = [(xs.start() + xs.end()) / 2, (zs.start() + zs.end()) / 2].map(|c| c.div_euclid(SLIME_BLOCK));
        visible.sort_by_key(|&(bx, bz)| (bx - middle[0]).abs().max((bz - middle[1]).abs()));
        let mut missing = false;
        for (bx, bz) in visible {
            let Some(texture) = self.slime.block(ui.ctx(), self.seed, bx, bz) else {
                missing = true;
                continue;
            };
            let screen = Rect::from_min_max(
                chunk_screen(bx * SLIME_BLOCK, bz * SLIME_BLOCK),
                chunk_screen((bx + 1) * SLIME_BLOCK, (bz + 1) * SLIME_BLOCK),
            );
            painter.image(texture, screen, uv, tint);
        }
        if missing {
            ui.ctx().request_repaint();
        }
        self.slime.evict();
    }

    fn paint_grid(&self, ui: &egui::Ui, rect: Rect) {
        let painter = ui.painter_at(rect);
        let blocks_per_point = self.map.blocks_per_point();
        let spacing = self.grid.spacing_at(blocks_per_point);
        let border = WORLD_BORDER_QUARTS as f64 * 4.0;
        let (min, max) = self.map.visible_blocks(rect);
        let points = spacing as f64 / blocks_per_point;
        let visibility = line_visibility(points);
        let line_alpha = (self.grid.opacity * visibility * 255.0) as u8;
        // only every so many get their coordinates when they are close together
        let label_every = (LABEL_POINTS / points).ceil().max(1.0) as i64;
        let line = Color32::from_white_alpha(line_alpha);
        let axis = Stroke::new(2.0, Color32::from_white_alpha(line_alpha.saturating_add(60)));
        let thin = Stroke::new(1.0, line);
        let font = FontId::monospace(11.0);
        // the lines every `every` blocks between `lo` and `hi`, not past the world border
        let range = |every: i64, lo: f64, hi: f64| {
            let (lo, hi) = (lo.max(-border), hi.min(border));
            ((lo / every as f64).ceil() as i64)..=((hi / every as f64).floor() as i64)
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
        // minor lines under the major ones, without the ones that are major
        if let Some((minor, minor_visibility)) = self.grid.minor_at(spacing, blocks_per_point)
            && minor_visibility > 0.0
        {
            let stroke = Stroke::new(
                1.0,
                Color32::from_white_alpha((self.grid.opacity * minor_visibility * 255.0) as u8),
            );
            for i in range(minor, min[0], max[0]).filter(|i| (i * minor) % spacing != 0) {
                let x = self.map.block_to_screen(rect, [(i * minor) as f64, 0.0]).x;
                painter.line_segment([pos2(x, lo_y), pos2(x, hi_y)], stroke);
            }
            for i in range(minor, min[1], max[1]).filter(|i| (i * minor) % spacing != 0) {
                let y = self.map.block_to_screen(rect, [0.0, (i * minor) as f64]).y;
                painter.line_segment([pos2(lo_x, y), pos2(hi_x, y)], stroke);
            }
        }
        // past where lines are drawn there is only the scale bar, however many they would be
        if visibility > 0.0 {
            for i in range(spacing, min[0], max[0]) {
                let block = i * spacing;
                let x = self.map.block_to_screen(rect, [block as f64, 0.0]).x;
                let stroke = if block == 0 { axis } else { thin };
                painter.line_segment([pos2(x, lo_y), pos2(x, hi_y)], stroke);
                if self.grid.labels && i.rem_euclid(label_every) == 0 {
                    label(pos2(x + 2.0, lo_y + 2.0), Align2::LEFT_TOP, format_blocks(block));
                }
            }
            for i in range(spacing, min[1], max[1]) {
                let block = i * spacing;
                let y = self.map.block_to_screen(rect, [0.0, block as f64]).y;
                let stroke = if block == 0 { axis } else { thin };
                painter.line_segment([pos2(lo_x, y), pos2(hi_x, y)], stroke);
                if self.grid.labels && i.rem_euclid(label_every) == 0 {
                    label(pos2(lo_x + 2.0, y + 2.0), Align2::LEFT_TOP, format_blocks(block));
                }
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
            kind: crate::structures::kind_index(id),
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
        // a whole world of candidates: quickly, however far apart the corners are
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
    fn markers_can_touch_but_not_overlap() {
        let (a, touching, overlapping) = (
            found("village", 0, 0),
            found("village", 100, 0),
            found("village", 99, 300),
        );
        assert_eq!(Placed::new(vec![a, touching], 100.0, true, 100).markers.len(), 2);
        let near = Placed::new(vec![a, found("village", 99, 0)], 100.0, true, 100).markers;
        assert_eq!(near.len(), 1);
        assert_eq!(Placed::new(vec![a, overlapping], 100.0, true, 100).markers.len(), 2);
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
    fn auto_steps_keep_lines_apart_and_stay_in_the_world() {
        assert_eq!(GridSteps::OneTwoFive.spacing_at(1.0, 80.0), 100);
        assert_eq!(GridSteps::OneTwoFive.spacing_at(4.0, 80.0), 500);
        assert_eq!(GridSteps::PowersOfTwo.spacing_at(1.0, 80.0), 128);
        assert_eq!(GridSteps::PowersOfTen.spacing_at(1.0, 80.0), 100);
        assert_eq!(GridSteps::ChunksAndRegions.spacing_at(1.0, 80.0), 512);
        assert_eq!(GridSteps::OneTwoFive.spacing_at(0.01, 80.0), 1);
        let border = WORLD_BORDER_QUARTS as i64 * 4;
        for steps in GridSteps::ALL {
            let all = steps.steps();
            assert!(all.windows(2).all(|pair| pair[0] < pair[1]), "{steps:?}");
            let largest = all[all.len() - 1];
            for zoom in -6..20 {
                let blocks_per_point = f64::from(zoom).exp2();
                for gap in [8.0, 80.0, 300.0] {
                    let spacing = steps.spacing_at(blocks_per_point, gap);
                    assert!(spacing <= MAX_GRID_STEP, "{steps:?}");
                    assert!(
                        spacing as f64 / blocks_per_point >= gap || spacing == largest,
                        "{steps:?}"
                    );
                }
            }
            // fully zoomed out the lines at 0 and both sides of it are in the world, not only the one at 0
            let spacing = steps.spacing_at(4.0 * 15f64.exp2(), 300.0);
            assert!((border / 4..=border).contains(&spacing), "{steps:?} {spacing}");
        }
    }

    #[test]
    fn minor_lines_fall_on_the_major_ones() {
        for steps in GridSteps::ALL {
            assert_eq!(steps.minor_of(1), None);
            for major in steps.steps().into_iter().skip(1) {
                let minor = steps.minor_of(major).unwrap();
                assert!(minor < major && major % minor == 0, "{steps:?} {major} {minor}");
            }
        }
        assert_eq!(GridSteps::OneTwoFive.minor_of(5), Some(1));
        assert_eq!(GridSteps::OneTwoFive.minor_of(50), Some(10));
        assert_eq!(GridSteps::ChunksAndRegions.minor_of(512), Some(64));
        // powers of 2 and the ladder are on chunk borders from a chunk up
        for steps in [GridSteps::PowersOfTwo, GridSteps::ChunksAndRegions] {
            assert!(
                steps
                    .steps()
                    .iter()
                    .filter(|&&step| step >= 16)
                    .all(|step| step % 16 == 0)
            );
        }
    }

    #[test]
    fn minor_lines_fade_in_until_they_become_major() {
        let grid = Grid::default();
        // 64 blocks per point: majors at 8192 (128 points), minors at 4096 (64 points, 0.8 of the gap)
        let (minor, almost) = grid.minor_at(8192, 64.0).unwrap();
        assert_eq!(minor, 4096);
        assert!(
            (0.5 * grid.minor_opacity..grid.minor_opacity).contains(&almost),
            "{almost}"
        );
        // just before the minors take over they are at their full opacity, still fainter than the majors
        let (_, full) = grid.minor_at(8192, 4096.0 / 80.0).unwrap();
        assert_eq!(full, grid.minor_opacity);
        assert!(full < 1.0);
        let hidden = Grid {
            minor_opacity: 0.0,
            ..Default::default()
        };
        assert_eq!(hidden.minor_at(8192, 64.0).unwrap().1, 0.0);
        let (_, none) = grid.minor_at(8192, 4096.0 / 15.0).unwrap();
        assert_eq!(none, 0.0);
        let fixed = Grid {
            spacing: GridSpacing::Fixed,
            ..Default::default()
        };
        assert_eq!(fixed.minor_at(8192, 64.0), None);
    }

    #[test]
    fn fixed_spacing_stays_the_same_and_lines_fade_when_too_close() {
        let grid = Grid {
            spacing: GridSpacing::Fixed,
            fixed_spacing: 512,
            ..Default::default()
        };
        for blocks_per_point in [0.1, 1.0, 100.0, 100_000.0] {
            assert_eq!(grid.spacing_at(blocks_per_point), 512);
        }
        assert_eq!(line_visibility(100.0), 1.0);
        assert!((0.0..1.0).contains(&line_visibility(5.0)));
        assert_eq!(line_visibility(2.0), 0.0);
    }

    #[test]
    fn past_the_whole_world_as_many_markers_fit() {
        let view = [1000.0, 700.0];
        let whole_world = WORLD_BORDER_QUARTS as f64 * 8.0 / 700.0;
        let number = |blocks_per_point: f64| {
            let half = [500.0 * blocks_per_point, 350.0 * blocks_per_point];
            let scale = marker_blocks_per_point(blocks_per_point, view);
            markers_that_fit([-half[0], -half[1]], [half[0], half[1]], 24.0 * scale)
        };
        let at_whole_world = number(whole_world);
        for further in [1.01, 1.5, 4.0] {
            assert!((number(whole_world * further) - at_whole_world).abs() < at_whole_world * 1e-9);
        }
        // before that more than one screen of world is what fits
        assert!(number(whole_world / 2.0) > 1000.0);
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
        for kind in MARKER_KINDS {
            assert!(image::load_from_memory(icon_file(kind.id)).is_ok(), "{}", kind.id);
        }
    }
}
