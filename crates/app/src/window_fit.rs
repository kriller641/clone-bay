//! Where the window goes when the app opens: the full height of the screen,
//! above the taskbar. No drawing happens here.
//!
//! The app is told the size of the screen it opens on, but not where the
//! taskbar is, so a fixed allowance is left for it and the window's title
//! bar. Windows places a window exactly where it is asked to, even under the
//! taskbar, so without this the buttons at the bottom of each screen could
//! open out of sight.

use eframe::egui::{pos2, vec2, Pos2, Vec2};

/// Room left below the window's contents for its title bar (about 31
/// points) and a taskbar along the bottom of the screen (48 points on
/// Windows 11), with a little to spare. Both are in points, so they stay
/// the same at any display scaling.
pub const TITLE_AND_TASKBAR: f32 = 85.0;

/// Room kept on each side when the screen is narrower than the window.
pub const SIDE_MARGIN: f32 = 20.0;

/// The size of the window's contents and where its top-left corner goes,
/// in points from the top-left of the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub size: Vec2,
    pub position: Pos2,
}

/// The placement for a screen of `screen` points: as tall as fits above the
/// taskbar, `width` wide unless the screen is narrower, centred left to
/// right and at the top. Never smaller than `min`. `None` when the screen's
/// size isn't a real size, so the window is left as it opened.
pub fn full_height(screen: Vec2, width: f32, min: Vec2) -> Option<Placement> {
    if !(screen.x.is_finite() && screen.y.is_finite() && screen.x > 0.0 && screen.y > 0.0) {
        return None;
    }
    let width = width.min(screen.x - 2.0 * SIDE_MARGIN).max(min.x);
    let height = (screen.y - TITLE_AND_TASKBAR).max(min.y);
    let left = ((screen.x - width) / 2.0).max(0.0);
    Some(Placement {
        size: vec2(width, height),
        position: pos2(left, 0.0),
    })
}

/// Places the window once, as soon as the screen's size is known. After
/// that the user's own moving and resizing is left alone.
#[derive(Default)]
pub struct WindowFit {
    done: bool,
}

impl WindowFit {
    /// The placement to apply on this frame, if any: the first time the
    /// screen's size is known, and never again.
    pub fn placement(&mut self, screen: Option<Vec2>, width: f32, min: Vec2) -> Option<Placement> {
        if self.done {
            return None;
        }
        let placement = full_height(screen?, width, min)?;
        self.done = true;
        Some(placement)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: f32 = 1560.0;
    const MIN: Vec2 = vec2(720.0, 360.0);

    fn place(width: f32, height: f32) -> Placement {
        full_height(vec2(width, height), WIDTH, MIN).unwrap()
    }

    #[test]
    fn a_1080p_screen_gets_its_full_height() {
        let placement = place(1920.0, 1080.0);
        assert_eq!(placement.size, vec2(1560.0, 995.0));
        assert_eq!(placement.position, pos2(180.0, 0.0));
    }

    #[test]
    fn a_1440p_screen_gets_its_full_height() {
        let placement = place(2560.0, 1440.0);
        assert_eq!(placement.size, vec2(1560.0, 1355.0));
        assert_eq!(placement.position, pos2(500.0, 0.0));
    }

    #[test]
    fn display_scaling_is_already_in_the_points() {
        // 1920 x 1080 at 125% is 1536 x 864 points: narrower than the window.
        let placement = place(1536.0, 864.0);
        assert_eq!(placement.size, vec2(1496.0, 779.0));
        assert_eq!(placement.position, pos2(20.0, 0.0));
        // At 150%, 1280 x 720 points.
        let placement = place(1280.0, 720.0);
        assert_eq!(placement.size, vec2(1240.0, 635.0));
        assert_eq!(placement.position, pos2(20.0, 0.0));
    }

    #[test]
    fn a_small_screen_still_gets_the_smallest_usable_window() {
        let placement = place(700.0, 400.0);
        assert_eq!(placement.size, MIN);
        // Wider than the screen: it starts at the left edge.
        assert_eq!(placement.position, pos2(0.0, 0.0));
        // Exactly the smallest, with nothing to spare.
        let placement = place(760.0, 445.0);
        assert_eq!(placement.size, MIN);
        assert_eq!(placement.position, pos2(20.0, 0.0));
    }

    #[test]
    fn a_screen_size_that_is_not_real_leaves_the_window_alone() {
        for screen in [
            vec2(0.0, 1080.0),
            vec2(1920.0, 0.0),
            vec2(-1920.0, 1080.0),
            vec2(f32::NAN, 1080.0),
            vec2(1920.0, f32::INFINITY),
        ] {
            assert_eq!(full_height(screen, WIDTH, MIN), None, "{screen:?}");
        }
    }

    #[test]
    fn the_window_is_placed_once_when_the_screen_is_known() {
        let mut fit = WindowFit::default();
        // Not known yet: nothing, and it is tried again.
        assert_eq!(fit.placement(None, WIDTH, MIN), None);
        let first = fit.placement(Some(vec2(1920.0, 1080.0)), WIDTH, MIN);
        assert_eq!(first, Some(place(1920.0, 1080.0)));
        // Never again, so the user's own moving and resizing stays.
        assert_eq!(fit.placement(Some(vec2(1920.0, 1080.0)), WIDTH, MIN), None);
    }

    #[test]
    fn a_size_that_is_not_real_is_tried_again_later() {
        let mut fit = WindowFit::default();
        assert_eq!(fit.placement(Some(vec2(0.0, 0.0)), WIDTH, MIN), None);
        assert!(fit
            .placement(Some(vec2(1920.0, 1080.0)), WIDTH, MIN)
            .is_some());
    }
}
