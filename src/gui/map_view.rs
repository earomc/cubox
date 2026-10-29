use std::rc::Rc;

use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

use crate::{
    gui::{
        map_renderer::{Quad, TileTexture},
        tiles::{TILE_SIZE, TileCache},
    },
    tile_renderer::{self, TileKey},
};

const WORLD_BORDER_QUARTS: f64 = tile_renderer::WORLD_BORDER_QUARTS as f64;
const MIN_ZOOM: f64 = -4.0; // 16 screen points per quart
const MAX_ZOOM: f64 = 15.0;
const MAX_LEVEL: i32 = 16;
/// Seconds the zoom level has to stay unchanged before tiles get sampled.
const LEVEL_SETTLE_SECONDS: f64 = 0.12;
/// Cached tiles of other levels stand in for visible tiles that aren't there yet: coarser ones
/// up to this many levels up, finer ones down to this many levels (every level deeper is 4x the quads).
const MAX_STAND_IN_UP: i32 = 6;
const MAX_STAND_IN_DOWN: i32 = 3;
/// Visible tiles without any stand in first get a quick cover sampled one to this many levels
/// coarser with one sample per pixel, 1/16 to 1/64 of the work at the default samples. It keeps up
/// even while zooming out continuously.
const COVER_LEVELS: i32 = 2;

pub struct Camera {
    /// world position in the center of the view, in quarts
    pub center: [f64; 2],
    /// log2 of quarts per screen point
    pub zoom: f64,
}

impl Camera {
    fn quarts_per_point(&self) -> f64 {
        self.zoom.exp2()
    }

    fn screen_to_world(&self, rect: Rect, p: Pos2) -> [f64; 2] {
        let qpp = self.quarts_per_point();
        [
            self.center[0] + (p.x - rect.center().x) as f64 * qpp,
            self.center[1] + (p.y - rect.center().y) as f64 * qpp,
        ]
    }

    fn world_to_screen(&self, rect: Rect, w: [f64; 2]) -> Pos2 {
        let qpp = self.quarts_per_point();
        pos2(
            rect.center().x + ((w[0] - self.center[0]) / qpp) as f32,
            rect.center().y + ((w[1] - self.center[1]) / qpp) as f32,
        )
    }
}

/// Settings that every tile of the current map shares.
#[derive(Clone, Copy)]
pub struct MapSettings {
    pub sample_height: i32,
    pub samples: u8,
}

pub struct MapView {
    pub camera: Camera,
    /// quart position under the mouse cursor, if it is over the map
    pub hovered_quart: Option<[i32; 2]>,
    /// The level of the last frame and the egui time it was switched to. Tiles are only sampled
    /// once the level has settled, so levels flown through while zooming quickly cost nothing.
    level_since: (i32, f64),
}

impl MapView {
    pub fn new() -> Self {
        Self {
            camera: Camera {
                center: [0.0, 0.0],
                zoom: 2.0,
            },
            hovered_quart: None,
            level_since: (-1, 0.0),
        }
    }

    /// The pyramid level that is drawn: the one whose texels are closest to one physical pixel,
    /// so tiles are scaled by at most 1.41x either way. That needs about as many texels as the
    /// screen has pixels, so no samples are wasted on detail that is shrunk away.
    pub fn level(&self, pixels_per_point: f32) -> i32 {
        (self.camera.zoom - (pixels_per_point as f64).log2())
            .round()
            .clamp(0.0, MAX_LEVEL as f64) as i32
    }

    /// Handles input and fills `quads` with the tiles to draw, back to front.
    pub fn show(&mut self, ui: &mut egui::Ui, tiles: &mut TileCache, settings: MapSettings, quads: &mut Vec<Quad>) {
        let rect = ui.max_rect();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        self.handle_input(ui, rect, &response);

        let painter = ui.painter_at(rect);
        let border = Rect::from_min_max(
            self.camera.world_to_screen(rect, [-WORLD_BORDER_QUARTS; 2]),
            self.camera.world_to_screen(rect, [WORLD_BORDER_QUARTS; 2]),
        );
        let mut out = Quads {
            quads,
            pixels_per_point: ui.pixels_per_point(),
            clip: border,
        };

        let level = self.level(ui.pixels_per_point());
        let tile_quarts = (TILE_SIZE as f64) * (level as f64).exp2();
        let key_at = |level: i32, x: i32, z: i32| TileKey {
            scale: level as u8,
            sample_height: settings.sample_height,
            tile_size_px: TILE_SIZE,
            samples: settings.samples,
            x,
            z,
        };

        // range of visible tiles, limited to the world border
        let min = self.camera.screen_to_world(rect, rect.min);
        let max = self.camera.screen_to_world(rect, rect.max);
        let tile_range = |lo: f64, hi: f64| {
            let lo = lo.max(-WORLD_BORDER_QUARTS);
            let hi = hi.min(WORLD_BORDER_QUARTS);
            (lo / tile_quarts).floor() as i32..=(hi / tile_quarts).floor() as i32
        };
        let (xs, zs) = (tile_range(min[0], max[0]), tile_range(min[1], max[1]));

        // visible tiles, closest to the center of the screen first so they are generated first
        let center = [
            self.camera.center[0] / tile_quarts - 0.5,
            self.camera.center[1] / tile_quarts - 0.5,
        ];
        let mut visible: Vec<(i32, i32)> = zs.flat_map(|z| xs.clone().map(move |x| (x, z))).collect();
        visible.sort_by(|a, b| {
            let dist = |(x, z): &(i32, i32)| (*x as f64 - center[0]).powi(2) + (*z as f64 - center[1]).powi(2);
            dist(a).total_cmp(&dist(b))
        });
        let keys: Vec<TileKey> = visible.iter().map(|&(x, z)| key_at(level, x, z)).collect();
        let now = ui.input(|i| i.time);
        if self.level_since.0 != level {
            self.level_since = (level, now);
        }
        let unsettled_for = LEVEL_SETTLE_SECONDS - (now - self.level_since.1);
        if unsettled_for > 0.0 {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(unsettled_for));
        }
        // in steps of COVER_LEVELS, so the cover stays the same while the level changes by one and
        // its tiles aren't abandoned half done. by the time the zoom gets there they are finished.
        let cover_level = ((level / COVER_LEVELS + 1) * COVER_LEVELS).min(MAX_LEVEL);
        let mut cover: Vec<TileKey> = Vec::new();
        for key in keys
            .iter()
            .filter(|key| !tiles.is_finished(key) && !has_stand_in(tiles, key))
        {
            let up = cover_level - level;
            let coarse = TileKey {
                scale: cover_level as u8,
                x: key.x >> up,
                z: key.z >> up,
                ..*key
            }
            .cover();
            if coarse != *key && !cover.contains(&coarse) {
                cover.push(coarse);
            }
        }
        tiles.update(&keys, &cover, unsettled_for <= 0.0);

        let mut fading = false;
        for key in keys {
            let screen = Rect::from_min_max(
                self.camera
                    .world_to_screen(rect, [key.x as f64 * tile_quarts, key.z as f64 * tile_quarts]),
                self.camera.world_to_screen(
                    rect,
                    [(key.x + 1) as f64 * tile_quarts, (key.z + 1) as f64 * tile_quarts],
                ),
            );
            let tile = tiles.get(&key, now);
            // stand ins until the tile is there and has faded in: a coarser tile, and on top of it
            // finer ones, which are sharper but might only cover part of it
            if !tile.as_ref().is_some_and(|(_, alpha)| *alpha >= 1.0) {
                draw_ancestor(&mut out, tiles, now, key, screen);
                if let Some((tex, _)) = tiles.get(&key.cover(), now) {
                    out.push(tex, screen, FULL_UV, 1.0);
                }
                draw_descendants(&mut out, tiles, now, key, screen, MAX_STAND_IN_DOWN);
            }
            if let Some((tex, alpha)) = tile {
                fading |= alpha < 1.0;
                out.push(tex, screen, FULL_UV, alpha);
            }
        }
        if fading {
            ui.ctx().request_repaint();
        }

        painter.rect_stroke(border, 0.0, Stroke::new(1.5, Color32::RED), StrokeKind::Outside);

        self.hovered_quart = response.hover_pos().map(|p| {
            let w = self.camera.screen_to_world(rect, p);
            [w[0].floor() as i32, w[1].floor() as i32]
        });
    }

    fn handle_input(&mut self, ui: &egui::Ui, rect: Rect, response: &egui::Response) {
        let cam = &mut self.camera;
        let qpp = cam.quarts_per_point();

        let drag = response.drag_delta();
        cam.center[0] -= drag.x as f64 * qpp;
        cam.center[1] -= drag.y as f64 * qpp;

        if let Some(pointer) = response.hover_pos() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta(), i.zoom_delta()));
            // scroll up / pinch out zooms in
            let delta = -scroll.y as f64 * 0.005 - (pinch as f64).log2();
            if delta != 0.0 {
                // keep the world position under the cursor fixed
                let anchor = cam.screen_to_world(rect, pointer);
                cam.zoom = (cam.zoom + delta).clamp(MIN_ZOOM, MAX_ZOOM);
                let offset: Vec2 = pointer - rect.center();
                let qpp = cam.quarts_per_point();
                cam.center = [anchor[0] - offset.x as f64 * qpp, anchor[1] - offset.y as f64 * qpp];
            }
        }

        cam.center[0] = cam.center[0].clamp(-WORLD_BORDER_QUARTS, WORLD_BORDER_QUARTS);
        cam.center[1] = cam.center[1].clamp(-WORLD_BORDER_QUARTS, WORLD_BORDER_QUARTS);
    }
}

const FULL_UV: Rect = Rect {
    min: Pos2::ZERO,
    max: pos2(1.0, 1.0),
};

/// Collects the quads of the map in physical pixels.
struct Quads<'a> {
    quads: &'a mut Vec<Quad>,
    pixels_per_point: f32,
    /// the world border on screen, tiles don't extend past it
    clip: Rect,
}

impl Quads<'_> {
    fn push(&mut self, tex: Rc<TileTexture>, screen: Rect, uv: Rect, alpha: f32) {
        // cut off what is outside the world, and the same part of the texture
        let clipped = screen.intersect(self.clip);
        if !clipped.is_positive() {
            return;
        }
        let to_uv = |p: Pos2| uv.min + (p - screen.min) / screen.size() * uv.size();
        let (screen, uv) = (clipped, Rect::from_min_max(to_uv(clipped.min), to_uv(clipped.max)));

        let px = |p: Pos2| p * self.pixels_per_point;
        let (min, max) = (px(screen.min), px(screen.max));
        self.quads.push(Quad {
            rect: [min.x, min.y, max.x, max.y],
            uv: [uv.min.x, uv.min.y, uv.max.x, uv.max.y],
            alpha,
            tex,
        });
    }
}

trait Cover {
    /// The quick one sample per pixel version of a tile, see `COVER_LEVELS`.
    fn cover(self) -> Self;
}

impl Cover for TileKey {
    fn cover(self) -> Self {
        TileKey { samples: 1, ..self }
    }
}

/// The tile, or its cover version if only that is cached.
fn cached(tiles: &TileCache, key: TileKey) -> Option<TileKey> {
    [key, key.cover()].into_iter().find(|k| tiles.is_finished(k))
}

/// The closest cached coarser tile covering `key`, and how many levels up it is.
fn ancestor(tiles: &TileCache, key: &TileKey) -> Option<(TileKey, i32)> {
    (1..=MAX_STAND_IN_UP.min(MAX_LEVEL - key.scale as i32)).find_map(|up| {
        let parent = TileKey {
            scale: key.scale + up as u8,
            x: key.x >> up,
            z: key.z >> up,
            ..*key
        };
        cached(tiles, parent).map(|parent| (parent, up))
    })
}

/// Whether any cached tile of another level, or the cover of this one, covers (part of) `key`.
fn has_stand_in(tiles: &TileCache, key: &TileKey) -> bool {
    tiles.is_finished(&key.cover())
        || ancestor(tiles, key).is_some()
        || (1..=MAX_STAND_IN_DOWN.min(key.scale as i32)).any(|down| {
            let n = 1 << down;
            (0..n * n).any(|i| {
                cached(
                    tiles,
                    TileKey {
                        scale: key.scale - down as u8,
                        x: key.x * n + i % n,
                        z: key.z * n + i / n,
                        ..*key
                    },
                )
                .is_some()
            })
        })
}

/// Draws the part of the closest cached coarser tile that covers `key`.
fn draw_ancestor(out: &mut Quads, tiles: &mut TileCache, now: f64, key: TileKey, screen: Rect) {
    let Some((parent, up)) = ancestor(tiles, &key) else {
        return;
    };
    if let Some((tex, _)) = tiles.get(&parent, now) {
        let n = 1 << up;
        let sub = 1.0 / n as f32;
        let uv = Rect::from_min_size(
            pos2(key.x.rem_euclid(n) as f32 * sub, key.z.rem_euclid(n) as f32 * sub),
            vec2(sub, sub),
        );
        out.push(tex, screen, uv, 1.0);
    }
}

/// Draws the cached finer tiles inside `key`, up to `depth` levels down.
fn draw_descendants(out: &mut Quads, tiles: &mut TileCache, now: f64, key: TileKey, screen: Rect, depth: i32) {
    if key.scale == 0 || depth == 0 {
        return;
    }
    let half = screen.size() / 2.0;
    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let child = TileKey {
            scale: key.scale - 1,
            x: key.x * 2 + dx,
            z: key.z * 2 + dz,
            ..key
        };
        let child_screen = Rect::from_min_size(screen.min + vec2(dx as f32 * half.x, dz as f32 * half.y), half);
        match cached(tiles, child).and_then(|child| tiles.get(&child, now)) {
            Some((tex, _)) => out.push(tex, child_screen, FULL_UV, 1.0),
            None => draw_descendants(out, tiles, now, child, child_screen, depth - 1),
        }
    }
}
