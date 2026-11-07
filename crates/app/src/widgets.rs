//! Small pieces of drawing shared by the screens. No decisions are made
//! here.

use eframe::egui;

use crate::list_model::{Sort, SortColumn};
use crate::saving::{SaveError, Saving};
use crate::state::capitalise;
use crate::theme;

/// The frame of the toolbar, the details panel and the status bar.
pub fn panel_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::side_top_panel(ui.style()).fill(theme::PANEL)
}

/// The one button on a screen that moves the user forward.
pub fn primary_button(text: &str) -> egui::Button<'_> {
    let text = egui::RichText::new(text).color(theme::ON_ACCENT).strong();
    egui::Button::new(text).fill(theme::ACCENT)
}

/// A coloured dot followed by text, for a state that should stand out.
/// Both go into the row being laid out, so call it inside a horizontal one.
pub fn marker(
    ui: &mut egui::Ui,
    colour: egui::Color32,
    text: impl Into<egui::WidgetText>,
) -> egui::Response {
    dot(ui, colour);
    ui.label(text)
}

/// A small coloured dot, vertically centred in the row.
pub fn dot(ui: &mut egui::Ui, colour: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, colour);
}

/// A tinted band across the screen holding something the user should read.
pub fn note(ui: &mut egui::Ui, fill: egui::Color32, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(fill)
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(contents);
        });
}

/// Why something was refused, in the error colour, as a sentence.
pub fn problem_line(ui: &mut egui::Ui, problem: Option<&str>) {
    if let Some(problem) = problem {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!("{}.", capitalise(problem)),
        );
    }
}

/// The box the user ticks to vouch that EVE is closed, when the app couldn't
/// check for itself.
pub fn eve_closed_box(ui: &mut egui::Ui, confirmed: &mut bool) {
    note(ui, theme::CAUTION, |ui| {
        ui.checkbox(confirmed, "I have closed every EVE client");
    });
}

/// What a screen that saves one file shows above its buttons: where the last
/// save put the original, and the "EVE is closed" box when it is needed.
pub fn save_messages<E: SaveError>(ui: &mut egui::Ui, saving: &mut Saving<E>) {
    if let Some(backup) = &saving.saved_to {
        ui.label(format!(
            "Saved. The original was backed up in {}",
            backup.display()
        ));
    }
    if saving.needs_eve_confirmation {
        eve_closed_box(ui, &mut saving.eve_closed_confirmed);
    }
}

/// How much room [`problem_line`], [`save_messages`] and [`save_buttons`]
/// take together.
pub fn save_footer_height<E: SaveError>(problem: bool, saving: &Saving<E>) -> f32 {
    let mut height = 48.0;
    if problem {
        height += 30.0;
    }
    if saving.saved_to.is_some() {
        height += 30.0;
    }
    if saving.needs_eve_confirmation {
        height += 44.0;
    }
    height
}

/// A button pressed in [`save_buttons`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveButton {
    Save,
    Discard,
    Back,
}

/// The buttons of a screen that saves one file: "Save changes", "Discard
/// changes" and "Back to the file list", then `unsaved` with a marker when
/// there is something not saved. While a save runs there is only a spinner.
pub fn save_buttons(
    ui: &mut egui::Ui,
    busy: bool,
    can_save: bool,
    dirty: bool,
    unsaved: Option<String>,
) -> Option<SaveButton> {
    let mut pressed = None;
    ui.horizontal(|ui| {
        if busy {
            ui.spinner();
            ui.label("Saving");
            return;
        }
        if ui
            .add_enabled(can_save, primary_button("Save changes"))
            .on_hover_text("Backs the file up first")
            .clicked()
        {
            pressed = Some(SaveButton::Save);
        }
        if ui
            .add_enabled(dirty, egui::Button::new("Discard changes"))
            .clicked()
        {
            pressed = Some(SaveButton::Discard);
        }
        if ui.button("Back to the file list").clicked() {
            pressed = Some(SaveButton::Back);
        }
        if let Some(unsaved) = unsaved {
            marker(ui, theme::ACCENT, unsaved);
        }
    });
    pressed
}

/// Where `remember_typing` notes whether a box had keyboard focus.
fn typing_id() -> egui::Id {
    egui::Id::new("clone-bay-typing-last-frame")
}

/// Whether Esc or the mouse's back button asked to go back to the file list,
/// like "Back to the file list". Esc pressed while typing in a box, or with
/// a list open, only leaves the box or closes the list, as it always has.
/// Call it before the screen's widgets, and `remember_typing` after them.
pub fn back_requested(ui: &egui::Ui) -> bool {
    let ctx = ui.ctx();
    // egui takes focus away on Esc before anything is drawn, so whether the
    // user was typing has to come from the end of the last frame.
    let was_typing = ctx.data(|data| data.get_temp::<bool>(typing_id()) == Some(true));
    let busy_with_esc = was_typing || egui::Popup::is_any_open(ctx);
    ui.input(|input| {
        input.pointer.button_pressed(egui::PointerButton::Extra1)
            || (!busy_with_esc && input.key_pressed(egui::Key::Escape))
    })
}

/// Notes whether a box has keyboard focus now, for `back_requested` on the
/// next frame.
pub fn remember_typing(ui: &egui::Ui) {
    let ctx = ui.ctx();
    let typing = ctx.memory(|memory| memory.focused().is_some());
    ctx.data_mut(|data| data.insert_temp(typing_id(), typing));
}

/// A bordered card that sets one group of controls apart from the next.
/// With a title, the title sits at the top with a line under it.
pub fn section(ui: &mut egui::Ui, title: Option<&str>, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(theme::PANEL)
        .stroke(egui::Stroke::new(1.0, theme::LINE))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(title) = title {
                section_title(ui, title);
                ui.separator();
                ui.add_space(2.0);
            }
            contents(ui);
        });
    // Clear space between one card and the next.
    ui.add_space(12.0);
}

/// The name of a section, larger than body text and in the accent colour.
pub fn section_title(ui: &mut egui::Ui, title: &str) {
    ui.label(
        egui::RichText::new(title)
            .size(15.0)
            .strong()
            .color(theme::ACCENT),
    );
}

/// A fixed-width box for typing a number, with the digits against its right
/// edge so a column of them lines up. There is deliberately no dragging to
/// change the value; numbers are only ever typed.
///
/// `shown` is the current value as text. While the box has focus it holds
/// exactly what has been typed; `parse` decides whether that is a value.
/// Text that isn't one is drawn in the problem colour and changes nothing,
/// and when the box loses focus it goes back to `shown`. Returns the new
/// value on the frame the typed text becomes (or changes to) a valid one.
pub fn typed_number<T>(
    ui: &mut egui::Ui,
    id: egui::Id,
    width: f32,
    shown: String,
    parse: impl Fn(&str) -> Option<T>,
) -> (egui::Response, Option<T>) {
    let typed: Option<String> = ui.data(|data| data.get_temp(id));
    let is_typing = typed.is_some();
    let mut text = typed.unwrap_or(shown);
    let valid = !is_typing || parse(&text).is_some();
    let mut edit = egui::TextEdit::singleline(&mut text)
        .id(id)
        .desired_width(width)
        .horizontal_align(egui::Align::RIGHT);
    if !valid {
        edit = edit.text_color(theme::PROBLEM);
    }
    // The box is given exactly `width`, whatever it holds.
    let response = ui.add_sized([width, theme::ROW_HEIGHT - 6.0], edit);
    let value = if response.changed() {
        parse(&text)
    } else {
        None
    };
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(id, text));
    } else {
        // Done typing: show the value itself again.
        ui.data_mut(|data| data.remove::<String>(id));
    }
    (response, value)
}

/// A column header that sorts by its column when clicked. Returns whether
/// it was clicked.
pub fn sort_header(ui: &mut egui::Ui, title: &str, column: SortColumn, sort: Sort) -> bool {
    let arrow = match (sort.column == column, sort.descending) {
        (false, _) => "",
        (true, false) => " ⬆",
        (true, true) => " ⬇",
    };
    let label = egui::RichText::new(format!("{title}{arrow}")).strong();
    ui.add(egui::Button::new(label).frame(false))
        .on_hover_text("Sort by this column")
        .clicked()
}

/// Where the user is in a run of steps: the current one stands out.
/// `current` counts from 1.
pub fn steps(ui: &mut egui::Ui, names: &[&str], current: usize) {
    ui.horizontal(|ui| {
        let mut first = true;
        for (text, is_current) in step_labels(names, current) {
            if !first {
                ui.weak("›");
            }
            first = false;
            if is_current {
                ui.label(egui::RichText::new(text).color(theme::ACCENT).strong());
            } else {
                ui.weak(text);
            }
        }
    });
}

/// Each step's numbered label, and whether it is the current one.
fn step_labels(names: &[&str], current: usize) -> Vec<(String, bool)> {
    names
        .iter()
        .zip(1..)
        .map(|(name, number)| (format!("{number} {name}"), number == current))
        .collect()
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;

    use super::*;

    #[test]
    fn a_sort_header_shows_which_way_its_column_is_sorted() {
        let sorted = |descending| Sort {
            column: SortColumn::Size,
            descending,
        };
        let draw = |sort: Sort| {
            Harness::new_ui(move |ui| {
                sort_header(ui, "Size", SortColumn::Size, sort);
                sort_header(ui, "ID", SortColumn::Id, sort);
            })
        };
        let up = draw(sorted(false));
        up.get_by_label("Size \u{2B06}");
        up.get_by_label("ID");
        draw(sorted(true)).get_by_label("Size \u{2B07}");
    }

    #[test]
    fn steps_are_numbered_in_order_with_a_mark_between_each() {
        let harness = Harness::new_ui(|ui| steps(ui, &["Choose", "Review", "Done"], 2));
        harness.get_by_label("1 Choose");
        harness.get_by_label("2 Review");
        harness.get_by_label("3 Done");
        assert_eq!(harness.query_all_by_label("›").count(), 2);
    }

    #[test]
    fn only_the_current_step_stands_out() {
        let current = |current| -> Vec<bool> {
            step_labels(&["Choose", "Review", "Done"], current)
                .into_iter()
                .map(|(_, is_current)| is_current)
                .collect()
        };
        assert_eq!(current(1), [true, false, false]);
        assert_eq!(current(2), [false, true, false]);
        assert_eq!(current(3), [false, false, true]);
        assert_eq!(step_labels(&["Only"], 1), [("1 Only".to_owned(), true)]);
    }

    #[test]
    fn a_dot_takes_up_room_in_its_row() {
        let width = std::cell::Cell::new(0.0);
        let _harness = Harness::new_ui(|ui| {
            ui.horizontal(|ui| {
                dot(ui, theme::ACCENT);
                width.set(ui.min_rect().width());
            });
        });
        assert!(width.get() >= 8.0, "the dot took {} points", width.get());
    }

    #[test]
    fn panels_are_filled_with_the_panel_colour() {
        let fill = std::cell::Cell::new(egui::Color32::TRANSPARENT);
        let _harness = Harness::new_ui(|ui| fill.set(panel_frame(ui).fill));
        assert_eq!(fill.get(), theme::PANEL);
    }
}
