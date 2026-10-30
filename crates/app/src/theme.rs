//! The app's colours and spacing, set once when the window opens.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

/// Behind the lists and forms.
pub const BACKGROUND: Color32 = Color32::from_rgb(0x12, 0x18, 0x21);
/// Behind the toolbar, the details panel and the status bar.
pub const PANEL: Color32 = Color32::from_rgb(0x18, 0x20, 0x2b);
/// Buttons, tick boxes and slider tracks.
const RAISED: Color32 = Color32::from_rgb(0x26, 0x32, 0x42);
const RAISED_HOVERED: Color32 = Color32::from_rgb(0x30, 0x40, 0x54);
const RAISED_PRESSED: Color32 = Color32::from_rgb(0x3a, 0x4c, 0x63);
/// Behind text boxes.
const FIELD: Color32 = Color32::from_rgb(0x0e, 0x13, 0x1a);
pub const LINE: Color32 = Color32::from_rgb(0x2a, 0x36, 0x46);
const BUTTON_LINE: Color32 = Color32::from_rgb(0x3a, 0x4a, 0x5e);
pub const TEXT: Color32 = Color32::from_rgb(0xdb, 0xe3, 0xec);
/// Secondary text: IDs, sizes, dates and hints.
pub const DIM: Color32 = Color32::from_rgb(0x87, 0x96, 0xa8);
/// The one button on a screen that moves the user forward, and marks on
/// what they have changed.
pub const ACCENT: Color32 = Color32::from_rgb(0x5a, 0xa9, 0xee);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x08, 0x13, 0x1f);
const SELECTED: Color32 = Color32::from_rgb(0x1a, 0x35, 0x50);
pub const OK: Color32 = Color32::from_rgb(0x62, 0xc4, 0x8f);
pub const WARNING: Color32 = Color32::from_rgb(0xe9, 0xb0, 0x4a);
pub const PROBLEM: Color32 = Color32::from_rgb(0xf4, 0x77, 0x6c);
/// Behind a note the user should read before going on.
pub const NOTE: Color32 = Color32::from_rgb(0x1b, 0x2a, 0x3b);
/// Behind a note about something that could go wrong.
pub const CAUTION: Color32 = Color32::from_rgb(0x33, 0x29, 0x0f);

const CORNER: CornerRadius = CornerRadius::same(4);

/// The height of a row in the file lists.
pub const ROW_HEIGHT: f32 = 26.0;

/// Gives the whole app its look. The app is always dark, whatever the
/// system setting, so it looks the same on every PC.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.visuals = visuals();
        style.spacing.item_spacing = egui::vec2(8.0, 5.0);
        style.spacing.button_padding = egui::vec2(9.0, 3.0);
        style.spacing.interact_size.y = 24.0;
    });
}

pub fn visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = PANEL;
    visuals.window_stroke = Stroke::new(1.0, LINE);
    visuals.extreme_bg_color = FIELD;
    // Added on top of whatever is behind, so stripes show on both the
    // background and the panels.
    visuals.faint_bg_color = Color32::from_additive_luminance(7);
    visuals.weak_text_color = Some(DIM);
    visuals.hyperlink_color = ACCENT;
    visuals.warn_fg_color = WARNING;
    visuals.error_fg_color = PROBLEM;
    visuals.selection.bg_fill = SELECTED;
    visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.slider_trailing_fill = true;

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = PANEL;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    for (state, fill, line, text) in [
        (&mut widgets.inactive, RAISED, BUTTON_LINE, TEXT),
        (&mut widgets.hovered, RAISED_HOVERED, ACCENT, Color32::WHITE),
        (&mut widgets.active, RAISED_PRESSED, ACCENT, Color32::WHITE),
        (&mut widgets.open, RAISED, BUTTON_LINE, TEXT),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, line);
        state.fg_stroke = Stroke::new(1.0, text);
    }
    for state in [
        &mut widgets.noninteractive,
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        state.corner_radius = CORNER;
    }
    visuals
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How bright a colour looks, from 0 (black) to 1 (white), by the WCAG
    /// definition.
    fn luminance(colour: Color32) -> f32 {
        let channel = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(colour.r()) + 0.7152 * channel(colour.g()) + 0.0722 * channel(colour.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn every_text_colour_is_easy_to_read_on_every_surface() {
        // 4.5 is the WCAG AA level for normal-sized text.
        for surface in [BACKGROUND, PANEL, SELECTED, NOTE, CAUTION] {
            for text in [TEXT, OK, WARNING, PROBLEM] {
                let ratio = contrast(text, surface);
                assert!(ratio >= 4.5, "{text:?} on {surface:?} is only {ratio:.1}");
            }
        }
        for surface in [BACKGROUND, PANEL] {
            let ratio = contrast(DIM, surface);
            assert!(
                ratio >= 4.5,
                "secondary text on {surface:?} is only {ratio:.1}"
            );
        }
        assert!(contrast(ON_ACCENT, ACCENT) >= 4.5);
        assert!(contrast(TEXT, RAISED) >= 4.5);
    }

    #[test]
    fn applying_the_theme_makes_the_app_dark_with_its_own_colours_and_spacing() {
        let ctx = egui::Context::default();
        ctx.set_theme(egui::Theme::Light);
        apply(&ctx);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        let style = ctx.global_style();
        assert_eq!(style.visuals.panel_fill, BACKGROUND);
        assert_eq!(style.spacing.interact_size.y, 24.0);
    }

    #[test]
    fn the_theme_uses_the_app_colours() {
        let visuals = visuals();
        assert!(visuals.dark_mode);
        assert_eq!(visuals.panel_fill, BACKGROUND);
        assert_eq!(visuals.text_color(), TEXT);
        assert_eq!(visuals.weak_text_color(), DIM);
        assert_eq!(visuals.error_fg_color, PROBLEM);
        assert_eq!(visuals.warn_fg_color, WARNING);
        assert_eq!(visuals.selection.bg_fill, SELECTED);
    }
}
