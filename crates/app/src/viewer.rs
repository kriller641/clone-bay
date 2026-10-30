//! A view of a probe formation that can be turned and zoomed.
//!
//! Nothing here uses a 3D engine: each position is rotated by the camera's
//! angles with a little trigonometry and painted on a flat canvas, which is
//! plenty for eight probes.
//!
//! Directions follow EVE Wrench (not yet confirmed in the client): in a
//! stored position `+x` is west, `+y` is up and `+z` is north. The view
//! works in east, north and up.

use clone_bay_core::formations::{Formation, KM};
use eframe::egui;

use crate::theme;

/// Where the view is looking from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Turn around the up axis, in radians. Zero looks north.
    pub yaw: f32,
    /// Height of the viewpoint above the horizon, in radians. Zero looks
    /// level; a quarter turn looks straight down.
    pub pitch: f32,
    /// Magnification. One fits the whole formation in the view.
    pub zoom: f32,
}

impl Camera {
    /// Looking straight down, north at the top and west on the left.
    pub const TOP: Camera = Camera {
        yaw: 0.0,
        pitch: std::f32::consts::FRAC_PI_2,
        zoom: 1.0,
    };
    /// Looking north from the south, level: up at the top, west on the left.
    pub const FRONT: Camera = Camera {
        yaw: 0.0,
        pitch: 0.0,
        zoom: 1.0,
    };
    /// Looking west from the east, level: up at the top, north on the right.
    pub const SIDE: Camera = Camera {
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: 0.0,
        zoom: 1.0,
    };

    const MIN_ZOOM: f32 = 0.2;
    const MAX_ZOOM: f32 = 20.0;
    /// Radians turned per pixel dragged.
    const TURN_SPEED: f32 = 0.01;

    /// Turns the view by a drag of `(dx, dy)` pixels. Dragging right turns
    /// the scene the way a globe turns when its near side is pushed right;
    /// dragging down tips it towards the viewer,
    /// between level and straight down.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw = (self.yaw + dx * Self::TURN_SPEED).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + dy * Self::TURN_SPEED).clamp(0.0, std::f32::consts::FRAC_PI_2);
    }

    /// Zooms by a factor, within sensible limits. Factors that aren't
    /// positive, finite numbers are ignored.
    pub fn zoom_by(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        }
    }

    /// Where a point `(east, north, up)` lands, as `(right, up, depth)` in
    /// the same units. `right` and `up` are across and up the screen; a
    /// larger `depth` is further from the viewer.
    pub fn project(&self, east: f64, north: f64, up: f64) -> (f64, f64, f64) {
        let (yaw, pitch) = (f64::from(self.yaw), f64::from(self.pitch));
        let turned_east = east * yaw.cos() - north * yaw.sin();
        let turned_north = east * yaw.sin() + north * yaw.cos();
        (
            turned_east,
            up * pitch.cos() + turned_north * pitch.sin(),
            turned_north * pitch.cos() - up * pitch.sin(),
        )
    }
}

impl Default for Camera {
    /// From the south-east and above: enough of an angle to read all three
    /// directions at once.
    fn default() -> Self {
        Self {
            yaw: -0.5,
            pitch: 0.55,
            zoom: 1.0,
        }
    }
}

/// A stored position as `(east, north, up)`.
pub fn east_north_up(x: f64, y: f64, z: f64) -> (f64, f64, f64) {
    (-x, z, y)
}

/// How far, in metres, the view has to reach from the middle to fit every
/// probe and the launch centre whichever way it is turned. Never less than
/// a kilometre, so an all-zero formation still has a scale.
pub fn extent(formation: &Formation) -> f64 {
    let centre = formation.centre();
    formation
        .probes
        .iter()
        .map(|probe| (probe.x, probe.y, probe.z))
        .chain([centre])
        .map(|(x, y, z)| (x * x + y * y + z * z).sqrt())
        .fold(KM, f64::max)
}

/// A round distance, in metres, to space the rings by so that about three
/// fit within `extent` metres: 1, 2 or 5 times a power of ten kilometres.
pub fn ring_step(extent: f64) -> f64 {
    let target = (extent / 3.0).max(KM);
    let power = 10_f64.powf(target.log10().floor());
    // How many of that power of ten the target is: from 1 up to 10.
    let multiple = match target / power {
        m if m <= 1.0 => 1.0,
        m if m <= 2.0 => 2.0,
        m if m <= 5.0 => 5.0,
        _ => 10.0,
    };
    multiple * power
}

/// The radius, in metres, of the outermost ring drawn to reach `reach`
/// metres: the first whole number of ring steps at or beyond it.
pub fn outer_ring(reach: f64) -> f64 {
    let step = ring_step(reach);
    step * (reach / step).ceil().max(1.0)
}

/// The probe drawn nearest to `pointer`, if one is within reach. When two
/// overlap, the one nearer the viewer (drawn on top) wins. Each entry of
/// `drawn` is a probe's place on screen and its depth.
pub fn probe_under(drawn: &[(egui::Pos2, f64)], pointer: egui::Pos2) -> Option<usize> {
    /// How close, in pixels, the pointer must be to a dot.
    const REACH: f32 = 9.0;
    drawn
        .iter()
        .enumerate()
        .filter(|(_, (at, _))| at.distance(pointer) <= REACH)
        .min_by(|(_, (_, a)), (_, (_, b))| a.total_cmp(b))
        .map(|(index, _)| index)
}

/// What the user did in the view on this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picked {
    Nothing,
    /// A probe was clicked.
    Probe(usize),
    /// Empty space was clicked.
    Background,
}

/// Maps view coordinates to the screen for one frame.
pub struct Scene {
    camera: Camera,
    centre: egui::Pos2,
    /// Pixels per metre.
    scale: f64,
}

impl Scene {
    /// The scene that draws a formation reaching `extent` metres from the
    /// middle inside `rect`, looked at from `camera`. At a zoom of one the
    /// outermost ring fits the shorter side of `rect`, with room around it
    /// for the compass letters.
    pub fn new(rect: egui::Rect, camera: Camera, extent: f64) -> Self {
        let half = f64::from(rect.width().min(rect.height())) / 2.0;
        Self {
            camera,
            centre: rect.center(),
            scale: half * 0.8 / outer_ring(extent) * f64::from(camera.zoom),
        }
    }

    /// The screen position and depth of a point `(east, north, up)`.
    pub fn place(&self, east: f64, north: f64, up: f64) -> (egui::Pos2, f64) {
        let (right, screen_up, depth) = self.camera.project(east, north, up);
        let at = egui::pos2(
            self.centre.x + (right * self.scale) as f32,
            self.centre.y - (screen_up * self.scale) as f32,
        );
        (at, depth)
    }
}

/// Draws the view and handles turning, zooming and picking. `selected` is
/// the probe to highlight.
pub fn show(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    formation: &Formation,
    camera: &mut Camera,
    selected: Option<usize>,
) -> Picked {
    let extent = extent(formation);
    // Sense the pointer over exactly what is painted. (A widget placed
    // the usual way is stretched to the width of its column.)
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let id = ui.id().with("formation view");
    let response = ui.interact(rect, id, egui::Sense::click_and_drag());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Formation view"));
    if response.dragged() {
        let delta = response.drag_delta();
        camera.orbit(delta.x, delta.y);
    }
    if response.hovered() {
        // Take the wheel for zooming, so the list around the view doesn't
        // scroll at the same time.
        let scrolled = ui.input_mut(|input| {
            let scrolled = input.smooth_scroll_delta.y;
            input.smooth_scroll_delta.y = 0.0;
            scrolled
        });
        if scrolled != 0.0 {
            // Scrolling down zooms in.
            camera.zoom_by((-scrolled * 0.004).exp());
        }
    }
    if response.double_clicked() {
        *camera = Camera::default();
    }

    let scene = Scene::new(rect, *camera, extent);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);

    ground(&painter, &scene, extent / f64::from(camera.zoom));

    // Probes, furthest first so nearer ones are painted over them.
    let mut drawn = Vec::with_capacity(formation.probes.len());
    for probe in &formation.probes {
        let (east, north, up) = east_north_up(probe.x, probe.y, probe.z);
        drawn.push((
            scene.place(east, north, up),
            scene.place(east, north, 0.0).0,
        ));
    }
    let hovered = response
        .hover_pos()
        .and_then(|pointer| probe_under(&places(&drawn), pointer));
    let mut order: Vec<usize> = (0..drawn.len()).collect();
    order.sort_by(|&a, &b| drawn[b].0 .1.total_cmp(&drawn[a].0 .1));
    for index in order {
        let ((at, _), foot) = drawn[index];
        // A line down (or up) to the flat plane shows how high the probe is.
        painter.line_segment([foot, at], egui::Stroke::new(1.0, theme::LINE));
        painter.circle_filled(foot, 1.5, theme::DIM);
        let chosen = selected == Some(index);
        let colour = if chosen {
            theme::WARNING
        } else {
            theme::ACCENT
        };
        painter.circle_filled(at, if chosen { 6.5 } else { 5.0 }, colour);
        if chosen || hovered == Some(index) {
            painter.circle_stroke(at, 9.0, egui::Stroke::new(1.5, colour));
        }
        painter.text(
            at + egui::vec2(8.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            (index + 1).to_string(),
            egui::FontId::proportional(12.0),
            theme::TEXT,
        );
    }

    // Where the game will centre the formation, when that isn't the middle.
    if !formation.is_balanced() {
        let (x, y, z) = formation.centre();
        let (east, north, up) = east_north_up(x, y, z);
        let (at, _) = scene.place(east, north, up);
        let diamond = [
            at + egui::vec2(0.0, -6.0),
            at + egui::vec2(6.0, 0.0),
            at + egui::vec2(0.0, 6.0),
            at + egui::vec2(-6.0, 0.0),
            at + egui::vec2(0.0, -6.0),
        ];
        painter.line(diamond.to_vec(), egui::Stroke::new(1.5, theme::PROBLEM));
        painter.text(
            at + egui::vec2(9.0, 5.0),
            egui::Align2::LEFT_TOP,
            "launch centre",
            egui::FontId::proportional(11.0),
            theme::PROBLEM,
        );
    }

    if response.clicked() {
        match hovered {
            Some(index) => Picked::Probe(index),
            None => Picked::Background,
        }
    } else {
        Picked::Nothing
    }
}

fn places(drawn: &[((egui::Pos2, f64), egui::Pos2)]) -> Vec<(egui::Pos2, f64)> {
    drawn.iter().map(|(place, _)| *place).collect()
}

/// The flat plane the formation sits on: distance rings with their radius
/// in kilometres, the four compass directions, and the up and down axis.
fn ground(painter: &egui::Painter, scene: &Scene, reach: f64) {
    const SEGMENTS: usize = 72;
    let step = ring_step(reach);
    let rings = ((reach / step).ceil() as usize).clamp(1, 12);
    // Seen edge-on the rings are one line, and their labels would pile up.
    let label_rings = scene.camera.pitch > 0.2;
    let line = egui::Stroke::new(1.0, theme::LINE);
    for ring in 1..=rings {
        let radius = step * ring as f64;
        let outline: Vec<egui::Pos2> = (0..=SEGMENTS)
            .map(|segment| {
                let angle = std::f64::consts::TAU * segment as f64 / SEGMENTS as f64;
                scene
                    .place(radius * angle.cos(), radius * angle.sin(), 0.0)
                    .0
            })
            .collect();
        painter.line(outline, line);
        // Label each ring where it crosses the line to the south-east, out
        // of the way of the compass letters.
        let diagonal = radius * std::f64::consts::FRAC_1_SQRT_2;
        let (at, _) = scene.place(diagonal, -diagonal, 0.0);
        if label_rings {
            painter.text(
                at,
                egui::Align2::LEFT_TOP,
                format!("{:.0} km", radius / KM),
                egui::FontId::proportional(10.0),
                theme::DIM,
            );
        }
    }
    let edge = step * rings as f64;
    // Letter, then the direction it marks as east, north and up.
    let mut axes = vec![
        ("N", (0.0, 1.0, 0.0), theme::TEXT),
        ("S", (0.0, -1.0, 0.0), theme::DIM),
        ("E", (1.0, 0.0, 0.0), theme::DIM),
        ("W", (-1.0, 0.0, 0.0), theme::DIM),
    ];
    // Seen from straight above, up and down are one point in the middle.
    if scene.camera.pitch < 1.4 {
        axes.push(("U", (0.0, 0.0, 1.0), theme::DIM));
        axes.push(("D", (0.0, 0.0, -1.0), theme::DIM));
    }
    let (origin, _) = scene.place(0.0, 0.0, 0.0);
    for (letter, (east, north, up), colour) in axes {
        let (end, _) = scene.place(east * edge, north * edge, up * edge);
        painter.line_segment([origin, end], line);
        let reach = edge * 1.08;
        let (label, _) = scene.place(east * reach, north * reach, up * reach);
        painter.text(
            label,
            egui::Align2::CENTER_CENTER,
            letter,
            egui::FontId::proportional(12.0),
            colour,
        );
    }
}

#[cfg(test)]
mod tests {
    // Written with the viewer and not yet run when committed.
    use super::*;

    fn close(a: (f64, f64, f64), b: (f64, f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6 && (a.2 - b.2).abs() < 1e-6
    }

    #[test]
    fn stored_positions_become_east_north_up() {
        // +x is west, +y is up, +z is north.
        assert_eq!(east_north_up(100.0, 0.0, 0.0), (-100.0, 0.0, 0.0));
        assert_eq!(east_north_up(0.0, 7.0, 0.0), (0.0, 0.0, 7.0));
        assert_eq!(east_north_up(0.0, 0.0, 3.0), (0.0, 3.0, 0.0));
    }

    #[test]
    fn the_top_view_has_north_up_and_west_on_the_left() {
        let top = Camera::TOP;
        // North goes up the screen, east goes right, height is invisible.
        assert!(close(top.project(0.0, 1.0, 0.0), (0.0, 1.0, 0.0)));
        assert!(close(top.project(1.0, 0.0, 0.0), (1.0, 0.0, 0.0)));
        let (right, up, _) = top.project(0.0, 0.0, 1.0);
        assert!(right.abs() < 1e-6 && up.abs() < 1e-6);
        // West, being minus east, is on the left.
        let (east, north, height) = east_north_up(1.0, 0.0, 0.0);
        assert!(top.project(east, north, height).0 < 0.0);
        // Something higher is nearer the viewer looking down.
        assert!(top.project(0.0, 0.0, 1.0).2 < top.project(0.0, 0.0, 0.0).2);
    }

    #[test]
    fn the_front_view_has_up_up_and_north_away() {
        let front = Camera::FRONT;
        assert!(close(front.project(0.0, 0.0, 1.0), (0.0, 1.0, 0.0)));
        assert!(close(front.project(1.0, 0.0, 0.0), (1.0, 0.0, 0.0)));
        // North is straight ahead: no movement on screen, only further away.
        assert!(close(front.project(0.0, 1.0, 0.0), (0.0, 0.0, 1.0)));
    }

    #[test]
    fn the_side_view_looks_west_with_north_on_the_right() {
        let side = Camera::SIDE;
        assert!(close(side.project(0.0, 1.0, 0.0), (1.0, 0.0, 0.0)));
        assert!(close(side.project(0.0, 0.0, 1.0), (0.0, 1.0, 0.0)));
        // West is straight ahead, so east is nearer.
        assert!(side.project(1.0, 0.0, 0.0).2 < 0.0);
    }

    #[test]
    fn turning_keeps_distances_the_same() {
        let camera = Camera {
            yaw: 1.1,
            pitch: 0.4,
            zoom: 3.0,
        };
        let (right, up, depth) = camera.project(3.0, -4.0, 12.0);
        let length = (right * right + up * up + depth * depth).sqrt();
        assert!((length - 13.0).abs() < 1e-6, "{length}");
    }

    #[test]
    fn dragging_turns_the_view_within_level_and_straight_down() {
        let mut camera = Camera::FRONT;
        camera.orbit(0.0, 50.0);
        assert!((camera.pitch - 0.5).abs() < 1e-6);
        camera.orbit(0.0, 10_000.0);
        assert_eq!(camera.pitch, std::f32::consts::FRAC_PI_2);
        camera.orbit(0.0, -10_000.0);
        assert_eq!(camera.pitch, 0.0);
        // Dragging left takes yaw down, wrapping; dragging right brings it back.
        camera.orbit(-100.0, 0.0);
        assert!(
            (camera.yaw - (std::f32::consts::TAU - 1.0)).abs() < 1e-5,
            "{}",
            camera.yaw
        );
        camera.orbit(100.0, 0.0);
        assert!(camera.yaw.abs() < 1e-5 || (camera.yaw - std::f32::consts::TAU).abs() < 1e-5);
    }

    #[test]
    fn zoom_stays_within_its_limits() {
        let mut camera = Camera::default();
        camera.zoom_by(2.0);
        assert_eq!(camera.zoom, 2.0);
        // Factors that make no sense leave the zoom where it was.
        for ignored in [0.0, -2.0, f32::NAN, f32::INFINITY] {
            camera.zoom_by(ignored);
            assert_eq!(camera.zoom, 2.0, "{ignored}");
        }
        camera.zoom_by(1_000.0);
        assert_eq!(camera.zoom, 20.0);
        camera.zoom_by(0.000_1);
        assert_eq!(camera.zoom, 0.2);
        for ignored in [0.0, -2.0, f32::NAN, f32::INFINITY] {
            camera.zoom_by(ignored);
            assert_eq!(camera.zoom, 0.2, "{ignored}");
        }
    }

    #[test]
    fn the_view_reaches_the_furthest_probe_or_the_launch_centre() {
        use clone_bay_core::formations::Probe;
        let formation = Formation::new(
            1,
            "f",
            vec![
                Probe::at_km(300.0, -400.0, 1200.0, 1.0),
                Probe::at_km(-300.0, 400.0, -1200.0, 1.0),
            ],
        );
        assert!((extent(&formation) - 1300.0 * KM).abs() < 1e-3);
        // Nothing but zeros still gets a scale.
        let centred = Formation::new(1, "f", vec![Probe::at_km(0.0, 0.0, 0.0, 1.0)]);
        assert_eq!(extent(&centred), KM);
        assert_eq!(extent(&Formation::new(1, "f", vec![])), KM);
    }

    #[test]
    fn rings_are_spaced_by_round_numbers_of_kilometres() {
        assert_eq!(ring_step(600.0 * KM), 200.0 * KM);
        assert_eq!(ring_step(1_500.0 * KM), 500.0 * KM);
        assert_eq!(ring_step(3_000.0 * KM), 1_000.0 * KM);
        assert_eq!(ring_step(6_440.0 * KM), 5_000.0 * KM);
        assert_eq!(ring_step(30.0 * KM), 10.0 * KM);
        // Just past a round number moves up to the next one.
        assert_eq!(ring_step(3_000.1 * KM), 2_000.0 * KM);
        assert_eq!(ring_step(6_000.1 * KM), 5_000.0 * KM);
        assert_eq!(ring_step(15_000.1 * KM), 10_000.0 * KM);
        assert_eq!(ring_step(15_000.0 * KM), 5_000.0 * KM);
        // Never finer than a kilometre.
        assert_eq!(ring_step(KM), KM);
        assert_eq!(ring_step(0.0), KM);
    }

    #[test]
    fn the_outermost_ring_is_the_first_at_or_beyond_the_formation() {
        assert_eq!(outer_ring(500.0 * KM), 600.0 * KM);
        assert_eq!(outer_ring(600.0 * KM), 600.0 * KM);
        assert_eq!(outer_ring(5_600.0 * KM), 6_000.0 * KM);
        assert_eq!(outer_ring(0.0), KM);
    }

    #[test]
    fn points_are_placed_from_the_middle_of_the_view() {
        // A view 400 wide and 300 tall: the shorter side decides the scale.
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(400.0, 300.0));
        // 500 km out needs rings to 600 km, which sit 0.8 of the way to the
        // edge: 120 of the 150 pixels from the middle.
        let scene = Scene::new(rect, Camera::TOP, 500.0 * KM);
        let at = |east: f64, north: f64, up: f64| scene.place(east * KM, north * KM, up * KM);
        assert_eq!(at(0.0, 0.0, 0.0).0, egui::pos2(300.0, 200.0));
        assert_eq!(at(600.0, 0.0, 0.0).0, egui::pos2(420.0, 200.0));
        assert_eq!(at(-300.0, 0.0, 0.0).0, egui::pos2(240.0, 200.0));
        // North is up the screen, where y is smaller.
        assert_eq!(at(0.0, 300.0, 0.0).0, egui::pos2(300.0, 140.0));
        assert_eq!(at(0.0, -600.0, 0.0).0, egui::pos2(300.0, 320.0));
        // Looking down, something higher is nearer.
        assert!(at(0.0, 0.0, 100.0).1 < at(0.0, 0.0, 0.0).1);
        assert!((at(0.0, 0.0, 100.0).1 + 100.0 * KM).abs() < 1.0);

        // Zooming in spreads everything out from the middle.
        let zoomed = Camera {
            zoom: 2.0,
            ..Camera::TOP
        };
        let scene = Scene::new(rect, zoomed, 500.0 * KM);
        assert_eq!(
            scene.place(300.0 * KM, 0.0, 0.0).0,
            egui::pos2(420.0, 200.0)
        );
        // From the front, height is up the screen.
        let scene = Scene::new(rect, Camera::FRONT, 500.0 * KM);
        assert_eq!(
            scene.place(0.0, 0.0, 300.0 * KM).0,
            egui::pos2(300.0, 140.0)
        );
        // A taller view than it is wide is scaled by its width.
        let tall = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 900.0));
        let scene = Scene::new(tall, Camera::TOP, 500.0 * KM);
        assert_eq!(
            scene.place(600.0 * KM, 0.0, 0.0).0,
            egui::pos2(270.0, 450.0)
        );
    }

    #[test]
    fn the_probe_under_the_pointer_is_the_nearest_one_drawn_there() {
        let drawn = [
            (egui::pos2(10.0, 10.0), 5.0),
            (egui::pos2(100.0, 100.0), 1.0),
            // On top of the first, and nearer the viewer.
            (egui::pos2(12.0, 10.0), -3.0),
        ];
        assert_eq!(probe_under(&drawn, egui::pos2(11.0, 10.0)), Some(2));
        assert_eq!(probe_under(&drawn, egui::pos2(104.0, 100.0)), Some(1));
        assert_eq!(probe_under(&drawn, egui::pos2(50.0, 50.0)), None);
        // Exactly at the edge of reach counts; just beyond doesn't.
        assert_eq!(probe_under(&drawn, egui::pos2(109.0, 100.0)), Some(1));
        assert_eq!(probe_under(&drawn, egui::pos2(109.5, 100.0)), None);
        assert_eq!(probe_under(&[], egui::pos2(0.0, 0.0)), None);
    }
}
