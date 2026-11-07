//! Measures distances on the map like a ruler in an online map: points are placed with clicks, the
//! path between them shows the length of each part and the whole.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, vec2};

use crate::{
    dimension::Dimension,
    gui::{map_view::MapView, overlays::format_blocks},
};

/// Radius of the dot at each point, in points.
const DOT_RADIUS: f32 = 5.0;
/// How far from a point it can still be grabbed, in points. Fingers need more than a mouse.
const GRAB_RADIUS: f32 = 10.0;
/// Overworld blocks per Nether block, the game's coordinate scale between them.
const NETHER_SCALE: f64 = 8.0;

#[derive(Default)]
pub struct Ruler {
    /// clicks on the map place points
    pub active: bool,
    /// in block coordinates
    pub points: Vec<[i64; 2]>,
}

impl Ruler {
    /// Starts measuring, with a first point if given.
    pub fn start(&mut self, first: Option<[i64; 2]>) {
        self.active = true;
        self.points.clear();
        self.points.extend(first);
    }

    /// Stops measuring and forgets the points.
    pub fn stop(&mut self) {
        self.active = false;
        self.points.clear();
    }

    /// The length of the whole path in blocks.
    pub fn total(&self) -> f64 {
        self.points.windows(2).map(|pair| distance(pair[0], pair[1])).sum()
    }

    /// Paints the path over the map and handles what is done with it. `map` is the map's response
    /// of this frame, `rect` the area it covers.
    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        map_view: &MapView,
        map: &egui::Response,
        dimension: Dimension,
        offset_for_zoom_buttons: bool,
    ) {
        let to_block = |p: Pos2| map_view.screen_to_block(rect, p).map(|b| b.floor() as i64);
        // the middle of the block, at close zoom the point sits in it
        let to_screen = |b: [i64; 2]| map_view.block_to_screen(rect, b.map(|b| b as f64 + 0.5));

        if self.active {
            // points are grabbed before the map: egui gives the press to what is on top
            let mut remove = None;
            for (i, point) in self.points.iter_mut().enumerate() {
                let at = to_screen(*point);
                let grab = Rect::from_center_size(at, vec2(GRAB_RADIUS, GRAB_RADIUS) * 2.0);
                let response = ui
                    .interact(grab, ui.make_persistent_id(("ruler point", i)), Sense::click_and_drag())
                    .on_hover_cursor(egui::CursorIcon::Grab);
                if response.dragged()
                    && let Some(p) = response.interact_pointer_pos()
                {
                    *point = to_block(p);
                } else if response.clicked() {
                    remove = Some(i);
                }
            }
            if let Some(i) = remove {
                self.points.remove(i);
            } else if map.clicked()
                && let Some(p) = map.interact_pointer_pos()
            {
                self.points.push(to_block(p));
            }
            self.paint(ui, rect, map, &to_screen);
            self.info(ui, rect, dimension, offset_for_zoom_buttons);
        } else {
            self.button(ui, rect, offset_for_zoom_buttons);
        }
    }

    fn paint(&self, ui: &egui::Ui, rect: Rect, map: &egui::Response, to_screen: &dyn Fn([i64; 2]) -> Pos2) {
        let painter = ui.painter_at(rect);
        let screen: Vec<Pos2> = self.points.iter().map(|&p| to_screen(p)).collect();
        let outline = Stroke::new(5.0, Color32::from_black_alpha(160));
        let line = Stroke::new(2.5, Color32::WHITE);

        // where the next point would go, with a mouse. touches don't hover
        let touching = ui.input(|i| i.any_touches());
        if let (Some(&last), Some(hover)) = (screen.last(), map.hover_pos().filter(|_| !touching)) {
            painter.line_segment([last, hover], Stroke::new(1.5, Color32::from_white_alpha(140)));
        }
        if screen.len() >= 2 {
            painter.line(screen.clone(), outline);
            painter.line(screen.clone(), line);
        }
        for &at in &screen {
            painter.circle(at, DOT_RADIUS, Color32::WHITE, Stroke::new(2.0, Color32::BLACK));
        }

        let font = FontId::proportional(13.0);
        let label = |at: Pos2, align: Align2, text: String| {
            let galley = painter.layout_no_wrap(text, font.clone(), Color32::WHITE);
            let background = align.anchor_size(at, galley.size());
            painter.rect_filled(background.expand(3.0), 3.0, Color32::from_black_alpha(200));
            painter.galley(background.min, galley, Color32::WHITE);
        };
        // each part at its middle, if it is long enough on the screen for its number
        if screen.len() >= 3 {
            for (pair, blocks) in screen.windows(2).zip(self.points.windows(2)) {
                let length = format_distance(distance(blocks[0], blocks[1]));
                let width = painter
                    .layout_no_wrap(length.clone(), font.clone(), Color32::WHITE)
                    .size()
                    .x;
                if (pair[1] - pair[0]).length() > width + 2.0 * GRAB_RADIUS + 12.0 {
                    label(pair[0] + (pair[1] - pair[0]) / 2.0, Align2::CENTER_CENTER, length);
                }
            }
        }
        if let Some(&last) = screen.last().filter(|_| screen.len() >= 2) {
            label(
                last + vec2(DOT_RADIUS + 6.0, 0.0),
                Align2::LEFT_CENTER,
                format_distance(self.total()),
            );
        }
    }

    /// The total, what it is in the other dimension, and the buttons.
    fn info(&mut self, ui: &egui::Ui, rect: Rect, dimension: Dimension, offset: bool) {
        Self::area(rect, offset).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                ui.strong("Measure distance");
                if self.points.len() < 2 {
                    ui.label("Click the map to place points.\nDrag a point to move it, click it to remove it.");
                } else {
                    let total = self.total();
                    let parts = self.points.len() - 1;
                    let plural = if parts == 1 { "" } else { "s" };
                    ui.label(format!("Total: {} ({parts} part{plural})", format_distance(total)));
                    if let Some((other, blocks)) = other_dimension(dimension, total) {
                        let other = match other {
                            Dimension::Nether => "Nether",
                            _ => "Overworld",
                        };
                        ui.label(format!("= {} in the {other}", format_distance(blocks)))
                            .on_hover_text("Each Nether block is 8 Overworld blocks, along both x and z");
                    }
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.points.is_empty(), egui::Button::new("Clear"))
                        .clicked()
                    {
                        self.points.clear();
                    }
                    if ui.button("Done").on_hover_text("Esc").clicked() {
                        self.stop();
                    }
                });
            });
        });
    }

    fn button(&mut self, ui: &egui::Ui, rect: Rect, offset: bool) {
        Self::area(rect, offset).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                let response = ui
                    .add(egui::Button::new("").min_size(vec2(36.0, 36.0)))
                    .on_hover_text("Measure distance (R), or right-click the map");
                paint_ruler_icon(ui.painter(), response.rect, ui.visuals().text_color());
                if response.clicked() {
                    self.start(None);
                }
            });
        });
    }

    fn area(rect: Rect, offset_for_zoom_buttons: bool) -> egui::Area {
        // above the zoom buttons when they are there
        let up = if offset_for_zoom_buttons { 104.0 } else { 0.0 };
        egui::Area::new(egui::Id::unique("ruler"))
            .anchor(Align2::RIGHT_BOTTOM, vec2(-12.0, -12.0 - up))
            .constrain_to(rect)
    }
}

/// A ruler lying at a slant, with marks along one edge. The font has no ruler.
fn paint_ruler_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let center = rect.center();
    let along = vec2(1.0, -1.0).normalized();
    let across = vec2(along.y, -along.x);
    let (length, width) = (24.0, 9.0);
    let corner = |a: f32, b: f32| center + along * a + across * b;
    let outline = vec![
        corner(-length / 2.0, -width / 2.0),
        corner(length / 2.0, -width / 2.0),
        corner(length / 2.0, width / 2.0),
        corner(-length / 2.0, width / 2.0),
    ];
    let stroke = Stroke::new(1.5, color);
    painter.add(egui::Shape::closed_line(outline, stroke));
    for i in 1..6 {
        let a = -length / 2.0 + i as f32 * length / 6.0;
        let mark = if i % 2 == 0 { 4.5 } else { 2.5 };
        painter.line_segment([corner(a, -width / 2.0), corner(a, -width / 2.0 + mark)], stroke);
    }
}

/// The straight distance between two block positions.
pub fn distance(a: [i64; 2], b: [i64; 2]) -> f64 {
    let (dx, dz) = ((b[0] - a[0]) as f64, (b[1] - a[1]) as f64);
    dx.hypot(dz)
}

/// `12.5 blocks`, `1 234 blocks`: tenths only where they still matter.
pub fn format_distance(blocks: f64) -> String {
    if blocks < 99.95 {
        format!("{blocks:.1} blocks")
    } else {
        format!("{} blocks", format_blocks(blocks.round() as i64))
    }
}

/// The same distance travelled in the dimension linked to this one, if there is one.
pub fn other_dimension(dimension: Dimension, blocks: f64) -> Option<(Dimension, f64)> {
    match dimension {
        Dimension::Overworld => Some((Dimension::Nether, blocks / NETHER_SCALE)),
        Dimension::Nether => Some((Dimension::Overworld, blocks * NETHER_SCALE)),
        Dimension::End => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_add_up_to_the_total() {
        let ruler = Ruler {
            active: true,
            points: vec![[0, 0], [3000, 4000], [3000, 0]],
        };
        assert_eq!(distance([0, 0], [3000, 4000]), 5000.0);
        assert_eq!(distance([3000, 4000], [3000, 0]), 4000.0);
        assert_eq!(ruler.total(), 9000.0);
        assert_eq!(distance([5, -5], [-5, 5]), distance([-5, 5], [5, -5]));
    }

    #[test]
    fn removing_a_point_joins_its_neighbours() {
        let mut ruler = Ruler {
            active: true,
            points: vec![[0, 0], [100, 100], [0, 300]],
        };
        ruler.points.remove(1);
        assert_eq!(ruler.total(), 300.0);
    }

    #[test]
    fn nothing_to_measure_is_zero() {
        let mut ruler = Ruler::default();
        assert_eq!(ruler.total(), 0.0);
        ruler.start(Some([7, 7]));
        assert_eq!(ruler.total(), 0.0);
        ruler.stop();
        assert!(!ruler.active && ruler.points.is_empty());
    }

    #[test]
    fn nether_distance_is_an_eighth_both_ways() {
        assert_eq!(
            other_dimension(Dimension::Overworld, 8000.0),
            Some((Dimension::Nether, 1000.0))
        );
        assert_eq!(
            other_dimension(Dimension::Nether, 1000.0),
            Some((Dimension::Overworld, 8000.0))
        );
        assert_eq!(other_dimension(Dimension::End, 1000.0), None);
    }

    #[test]
    fn distances_are_readable() {
        assert_eq!(format_distance(0.0), "0.0 blocks");
        assert_eq!(format_distance(2f64.sqrt()), "1.4 blocks");
        assert_eq!(format_distance(99.94), "99.9 blocks");
        assert_eq!(format_distance(99.96), "100 blocks");
        assert_eq!(format_distance(12_345.6), "12 346 blocks");
    }
}
