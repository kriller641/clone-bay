//! Drawing the probe formation editor. All decisions live in
//! `formation_flow` and in the core crate.

use clone_bay_core::formations::{
    Direction, Formation, Preset, View, AU, KM, MAX_PROBES, RANGE_OPTIONS,
};
use eframe::egui;

use crate::formation_flow::{
    format_km, format_number, nudged, parse_degrees, parse_distance_km, parse_factor, parse_km,
    FormationFlow, NUDGE,
};
use crate::list_model::counted;
use crate::viewer::{self, Camera, Picked};
use crate::widgets::{problem_line, save_buttons, save_messages, typed_number, SaveButton};

/// What the user asked for on this frame that the caller has to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    None,
    /// Leave the editor and go back to the file list.
    Close,
}

/// The picture of the formation fills the room under the list, within
/// these sizes.
/// How much of the room under the list the picture takes, each way.
const VIEW_SHARE: f32 = 0.9;
const VIEW_MIN_HEIGHT: f32 = 180.0;
const VIEW_MAX_HEIGHT: f32 = 900.0;
const VIEW_MAX_WIDTH: f32 = 900.0;

/// The heading of the editor: whose formations these are.
pub fn title(flow: &FormationFlow) -> String {
    match &flow.name {
        Some(name) => format!("Probe formations for {name} (account {})", flow.entry.id),
        None => format!("Probe formations for account {}", flow.entry.id),
    }
}

/// A scan range in metres, the way the game shows it.
pub fn range_label(range: f64) -> String {
    match RANGE_OPTIONS.iter().find(|option| *option * AU == range) {
        Some(option) => format!("{option} AU"),
        None => format!("{:.3} AU (not a range the game offers)", range / AU),
    }
}

/// One line about whether a formation will launch where it is drawn.
pub fn balance_note(formation: &Formation) -> String {
    if formation.is_balanced() {
        "Launches exactly as drawn.".to_owned()
    } else {
        format!(
            "The game centres a formation on its probes' average position, so this one will \
             launch shifted by {:.0} km.",
            formation.launch_shift() / KM
        )
    }
}

/// The arrows on the buttons either side of a coordinate.
pub const LEFT_ARROW: &str = "\u{25C0}";
pub const RIGHT_ARROW: &str = "\u{25B6}";

/// What "Balance" and "Centre" do, shown when the pointer rests on them.
const BALANCE_HINT: &str = "Keeps your probes where they are and adds one counterweight probe \
                            opposite them, so the formation launches where it is drawn. If all \
                            eight probes are used, the last one is moved to be the counterweight.";
const CENTRE_HINT: &str = "Adds no probe. Moves every probe by the same amount so the formation \
                           launches where it is drawn. The shape stays the same; only the \
                           numbers shift.";

/// The width of the boxes for a scale factor and a number of degrees.
const TOOL_BOX_WIDTH: f32 = 56.0;

/// How the side a turn is seen from is named in its list.
pub fn view_label(view: View) -> &'static str {
    match view {
        View::Above => "above",
        View::South => "the south",
        View::East => "the east",
    }
}

/// The width of a position or distance box: room for a signed number of
/// kilometres with seven or so digits, and narrow enough that a probe's whole
/// row fits the window at the size it opens at.
const COORDINATE_BOX_WIDTH: f32 = 70.0;

pub fn draw(ui: &mut egui::Ui, flow: &mut FormationFlow) -> Request {
    let mut request = Request::None;
    ui.add_space(4.0);
    ui.heading(egui::RichText::new(title(flow)).strong());
    ui.add_space(6.0);

    // Ctrl+Z, and Ctrl+Y or Ctrl+Shift+Z, unless a text box is being typed
    // in, which has an undo of its own.
    if ui.memory(|memory| memory.focused().is_none()) {
        let command = egui::Modifiers::COMMAND;
        let redo = ui.input_mut(|input| {
            input.consume_key(command | egui::Modifiers::SHIFT, egui::Key::Z)
                || input.consume_key(command, egui::Key::Y)
        });
        if redo {
            flow.redo();
        } else if ui.input_mut(|input| input.consume_key(command, egui::Key::Z)) {
            flow.undo();
        }
    }

    // Whatever this frame changes becomes one step that Undo can take back.
    let before = flow.formations.clone();
    let mut typing = false;
    let mut step = None;
    let body_height = (ui.available_height() - 110.0).max(120.0);
    if flow.readable() {
        ui.columns(2, |columns| {
            columns[0].set_max_height(body_height);
            formation_list(&mut columns[0], flow);
            columns[0].add_space(10.0);
            picture(&mut columns[0], flow);
            // Both ways: in a narrow window a probe's row is wider than
            // the column.
            egui::ScrollArea::both()
                .id_salt("formation")
                .max_height(body_height)
                .auto_shrink([false, false])
                .show(&mut columns[1], |ui| {
                    formation_editor(ui, flow, &mut typing, &mut step);
                });
        });
    }
    if flow.formations != before {
        let now = ui.input(|input| input.time);
        flow.record_change(before, typing, now);
    }
    // Only now, so that undoing isn't itself recorded as a change.
    match step {
        Some(Step::Undo) => flow.undo(),
        Some(Step::Redo) => flow.redo(),
        None => {}
    }
    ui.add_space(6.0);

    problem_line(ui, flow.problem.as_deref());
    if let Some(invalid) = flow.invalid() {
        ui.colored_label(ui.visuals().warn_fg_color, format!("{invalid}."));
    }
    if let Some(note) = &flow.share_note {
        ui.label(format!("{note}."));
    }
    save_messages(ui, &mut flow.saving);
    let unsaved = flow.is_dirty().then(|| "Changes not saved".to_owned());
    match save_buttons(ui, flow.busy(), flow.can_save(), flow.is_dirty(), unsaved) {
        Some(SaveButton::Save) => flow.save(),
        Some(SaveButton::Discard) => flow.discard(),
        Some(SaveButton::Back) => request = Request::Close,
        None => {}
    }
    request
}

/// The left half: the formations, and adding and removing them.
fn formation_list(ui: &mut egui::Ui, flow: &mut FormationFlow) {
    ui.strong("Formations");
    let in_use = flow.formations.selected;
    let mut chosen = None;
    if flow.formations.list().is_empty() {
        ui.weak("This account has no custom formations yet.");
    }
    for formation in flow.formations.list() {
        let mut label = format!(
            "{}  ({})",
            formation.name,
            counted(formation.probes.len(), "probe", "probes")
        );
        if in_use == Some(formation.id) {
            label.push_str("  default");
        }
        if ui
            .selectable_label(flow.current == Some(formation.id), label)
            .clicked()
        {
            chosen = Some(formation.id);
        }
    }
    if let Some(id) = chosen {
        flow.current = Some(id);
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let mut preset = None;
        egui::ComboBox::from_id_salt("add formation")
            .selected_text("Add a formation")
            .show_ui(ui, |ui| {
                for option in Preset::ALL {
                    if ui.selectable_label(false, option.label()).clicked() {
                        preset = Some(option);
                    }
                }
            });
        if let Some(preset) = preset {
            flow.add_preset(preset);
        }
        let has_current = flow.current.is_some();
        if ui
            .add_enabled(has_current, egui::Button::new("Duplicate"))
            .clicked()
        {
            flow.duplicate_current();
        }
        if ui
            .add_enabled(has_current, egui::Button::new("Delete"))
            .clicked()
        {
            flow.delete_current();
        }
    });
    // Sharing formations with other players through the clipboard, apart
    // from the settings file.
    ui.horizontal(|ui| {
        let has_current = flow.current.is_some();
        let has_any = !flow.formations.list().is_empty();
        if ui
            .add_enabled(has_current, egui::Button::new("Copy this one"))
            .on_hover_text(
                "Copies the formation being shown to the clipboard as a few lines of text, \
                 to paste to another player. Only its name and probes are copied.",
            )
            .clicked()
        {
            flow.copy_current();
        }
        if ui
            .add_enabled(has_any, egui::Button::new("Copy all"))
            .on_hover_text("Copies every formation in the list to the clipboard as text.")
            .clicked()
        {
            flow.copy_all();
        }
        if ui
            .button("Paste")
            .on_hover_text(
                "Adds the formations in the text on the clipboard, copied from Clone Bay \
                 by you or another player. Nothing here is replaced.",
            )
            .clicked()
        {
            flow.paste();
        }
    });
}

/// Under the list: the formation being shown, as a picture that can be
/// turned. It sits here, and not under the table of probes, so it stays in
/// sight however long the table is.
fn picture(ui: &mut egui::Ui, flow: &mut FormationFlow) {
    let Some(id) = flow.current else {
        return;
    };
    let picked = flow.picked_in(id);
    let Some(formation) = flow.formations.get(id) else {
        return;
    };
    let camera = &mut flow.camera;
    ui.horizontal(|ui| {
        ui.strong("Picture");
        let views = [
            (
                "Top",
                Camera::TOP,
                "From above: north at the top, west on the left",
            ),
            (
                "Front",
                Camera::FRONT,
                "From the south: up at the top, west on the left",
            ),
            (
                "Side",
                Camera::SIDE,
                "From the east: up at the top, north on the right",
            ),
            (
                "Reset view",
                Camera::default(),
                "Back to the angle it started at",
            ),
        ];
        for (label, view, hint) in views {
            if ui.button(label).on_hover_text(hint).clicked() {
                *camera = view;
            }
        }
    });
    // Leave room underneath for the line of help.
    let room = egui::vec2(ui.available_width(), ui.available_height() - 24.0);
    let size = egui::vec2(
        (room.x * VIEW_SHARE).min(VIEW_MAX_WIDTH),
        (room.y * VIEW_SHARE).clamp(VIEW_MIN_HEIGHT, VIEW_MAX_HEIGHT),
    );
    match viewer::show(ui, size, formation, camera, picked) {
        Picked::Probe(index) => flow.picked_probe = Some((id, index)),
        Picked::Background => flow.picked_probe = None,
        Picked::Nothing => {}
    }
    ui.weak("Drag to turn, scroll to zoom, click a probe to pick it out in the table.");
}

/// A small arrow button that moves a coordinate by `step` metres, towards
/// `direction`. Returns the new coordinate when clicked. It is greyed out
/// when the move would take the probe further than one can be placed.
fn nudge_button(
    ui: &mut egui::Ui,
    arrow: &str,
    direction: &str,
    coordinate: f64,
    step: f64,
) -> Option<f64> {
    let moved = nudged(coordinate, step);
    let hint = format!("Move {:.0} km {direction}", step.abs() / KM);
    ui.add_enabled(moved.is_some(), egui::Button::new(arrow).small())
        .on_hover_text(hint)
        .clicked()
        .then_some(moved)
        .flatten()
}

/// A step back or forward through the changes made so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Undo,
    Redo,
}

/// The Undo and Redo buttons. They sit with the tools that change a
/// formation, not with Save at the bottom.
fn history_buttons(ui: &mut egui::Ui, can_undo: bool, can_redo: bool) -> Option<Step> {
    let mut step = None;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(can_undo, egui::Button::new("Undo"))
            .on_hover_text("Take back the last change (Ctrl+Z)")
            .clicked()
        {
            step = Some(Step::Undo);
        }
        if ui
            .add_enabled(can_redo, egui::Button::new("Redo"))
            .on_hover_text("Put back what was undone (Ctrl+Y)")
            .clicked()
        {
            step = Some(Step::Redo);
        }
    });
    step
}

/// The right half: the formation being shown. `typing` is set when a change
/// on this frame came from typing or dragging, as opposed to a button.
/// `step` is set when Undo or Redo was pressed.
fn formation_editor(
    ui: &mut egui::Ui,
    flow: &mut FormationFlow,
    typing: &mut bool,
    step: &mut Option<Step>,
) {
    let (can_undo, can_redo) = (flow.can_undo(), flow.can_redo());
    let Some(id) = flow.current else {
        ui.weak("Add a formation, or pick one on the left.");
        // Still offered here, so deleting the last formation can be undone.
        *step = history_buttons(ui, can_undo, can_redo);
        return;
    };
    let stores_choice = flow.formations.selected.is_some();
    let in_use = flow.formations.selected == Some(id);
    let mut make_in_use = false;
    let mut add_probe = false;
    let mut picked = flow.picked_in(id);
    let tools = &mut flow.tools;
    let Some(formation) = flow.formations.get_mut(id) else {
        return;
    };

    ui.horizontal(|ui| {
        ui.label("Name");
        let name = egui::TextEdit::singleline(&mut formation.name).desired_width(220.0);
        *typing |= ui.add(name).changed();
        if stores_choice && !in_use && ui.button("Default in game")
            .on_hover_text("Makes this the formation picked in the probe scanner the next time the game loads these settings. Takes effect when saved; it does not change a running game.")
            .clicked() {
            make_in_use = true;
        }
    });
    ui.add_space(4.0);

    let mut remove = None;
    egui::Grid::new("probes").striped(true).show(ui, |ui| {
        ui.strong("#");
        ui.strong("N/S (km)")
            .on_hover_text("Kilometres north of the centre; negative is south");
        ui.strong("W/E (km)")
            .on_hover_text("Kilometres west of the centre; negative is east");
        ui.strong("U/D (km)")
            .on_hover_text("Kilometres above the centre; negative is below");
        ui.strong("Distance (km)").on_hover_text(
            "How far the probe is from the centre. Type a new distance to move it straight \
             towards or away from the centre, without changing its direction.",
        );
        ui.strong("Scan range");
        ui.end_row();
        for (index, probe) in formation.probes.iter_mut().enumerate() {
            let number = ui
                .selectable_label(picked == Some(index), (index + 1).to_string())
                .on_hover_text("Pick this probe out in the picture");
            if number.clicked() {
                picked = (picked != Some(index)).then_some(index);
            }
            // North is +z, west is +x and up is +y.
            // Each has the direction its number grows in, then the opposite.
            let axes = [
                ("n", &mut probe.z, "north", "south"),
                ("w", &mut probe.x, "west", "east"),
                ("u", &mut probe.y, "up", "down"),
            ];
            for (axis, coordinate, grows, shrinks) in axes {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    // The arrows are in the order of the heading: the left
                    // one moves towards the direction named first.
                    let mut moved = nudge_button(ui, LEFT_ARROW, grows, *coordinate, NUDGE);
                    let box_id = ui.id().with(("probe", id, index, axis));
                    let shown = format_km(*coordinate / KM);
                    let (_, typed) =
                        typed_number(ui, box_id, COORDINATE_BOX_WIDTH, shown, parse_km);
                    // Only write back a value the user actually typed, so
                    // untouched positions keep every digit they were stored
                    // with.
                    if let Some(km) = typed {
                        if km * KM != *coordinate {
                            *coordinate = km * KM;
                            *typing = true;
                        }
                    }
                    moved = moved.or(nudge_button(ui, RIGHT_ARROW, shrinks, *coordinate, -NUDGE));
                    if let Some(moved) = moved {
                        *coordinate = moved;
                    }
                });
            }
            if probe.distance() == 0.0 {
                // At the centre there is no direction to move along.
                ui.weak("0").on_hover_text(
                    "This probe is at the centre, so it has no direction to move in. Give it \
                     a position first.",
                );
            } else {
                let box_id = ui.id().with(("probe", id, index, "distance"));
                let shown = format_km(probe.distance() / KM);
                let (_, typed) = typed_number(
                    ui,
                    box_id,
                    COORDINATE_BOX_WIDTH,
                    shown.clone(),
                    parse_distance_km,
                );
                // As with the positions: only a distance the user typed, and
                // only one that differs from what is shown, moves the probe.
                if let Some(km) = typed {
                    if format_km(km) != shown && probe.set_distance(km * KM) {
                        *typing = true;
                    }
                }
            }
            egui::ComboBox::from_id_salt(("range", index))
                .selected_text(range_label(probe.range))
                .show_ui(ui, |ui| {
                    for option in RANGE_OPTIONS {
                        ui.selectable_value(&mut probe.range, option * AU, format!("{option} AU"));
                    }
                });
            if ui.small_button("Remove").clicked() {
                remove = Some(index);
            }
            ui.end_row();
        }
    });
    if let Some(index) = remove {
        formation.probes.remove(index);
        picked = None;
    }
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        let room = formation.probes.len() < MAX_PROBES;
        if ui
            .add_enabled(room, egui::Button::new("Add a probe"))
            .clicked()
        {
            add_probe = true;
        }
        if ui.button("Half size").clicked() {
            formation.scale(0.5);
        }
        if ui.button("Double size").clicked() {
            formation.scale(2.0);
        }
        let factor_id = ui.id().with("scale factor");
        let shown = format_number(tools.scale_factor);
        let (_, typed) = typed_number(ui, factor_id, TOOL_BOX_WIDTH, shown, parse_factor);
        if let Some(factor) = typed {
            tools.scale_factor = factor;
        }
        if ui
            .button("Scale")
            .on_hover_text(
                "Multiplies every position by the number in the box: 2 doubles the size, \
                 0.5 halves it. Scan ranges are not changed.",
            )
            .clicked()
        {
            formation.scale(tools.scale_factor);
        }
        egui::ComboBox::from_id_salt("every range")
            .selected_text("Set every range")
            .show_ui(ui, |ui| {
                for option in RANGE_OPTIONS {
                    if ui.selectable_label(false, format!("{option} AU")).clicked() {
                        formation.set_range(option);
                    }
                }
            });
    });
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Turn by");
        let degrees_id = ui.id().with("turn degrees");
        let shown = format_number(tools.turn_degrees);
        let (_, typed) = typed_number(ui, degrees_id, TOOL_BOX_WIDTH, shown, parse_degrees);
        if let Some(degrees) = typed {
            tools.turn_degrees = degrees;
        }
        ui.label("degrees clockwise, seen from");
        egui::ComboBox::from_id_salt("turn view")
            .selected_text(view_label(tools.turn_view))
            .show_ui(ui, |ui| {
                for view in [View::Above, View::South, View::East] {
                    ui.selectable_value(&mut tools.turn_view, view, view_label(view));
                }
            });
        if ui
            .button("Turn")
            .on_hover_text(
                "Turns the whole formation around its middle. The side it is seen from \
                 matches the picture's Top, Front and Side views. A negative number turns \
                 it anticlockwise.",
            )
            .clicked()
        {
            formation.turn(tools.turn_view, tools.turn_degrees);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Mirror");
        let flips = [
            ("Flip N/S", Direction::NorthSouth, "Swaps north and south"),
            ("Flip W/E", Direction::WestEast, "Swaps west and east"),
            ("Flip U/D", Direction::UpDown, "Swaps up and down"),
        ];
        for (label, direction, hint) in flips {
            if ui.button(label).on_hover_text(hint).clicked() {
                formation.mirror(direction);
            }
        }
    });
    *step = history_buttons(ui, can_undo, can_redo);
    ui.add_space(4.0);

    // The note gets a line of its own so a long one wraps instead of being
    // cut off at the edge.
    ui.label(balance_note(formation));
    // In a row of its own so the button keeps its natural width.
    ui.horizontal(|ui| {
        // The hints show whether or not the buttons can be pressed, so
        // they can be read on a formation that is already balanced.
        let off_balance = !formation.is_balanced();
        let balance = ui
            .add_enabled(off_balance, egui::Button::new("Balance"))
            .on_hover_text(BALANCE_HINT)
            .on_disabled_hover_text(BALANCE_HINT);
        if balance.clicked() {
            formation.balance();
        }
        let centre = ui
            .add_enabled(off_balance, egui::Button::new("Centre"))
            .on_hover_text(CENTRE_HINT)
            .on_disabled_hover_text(CENTRE_HINT);
        if centre.clicked() {
            formation.recentre();
        }
    });
    ui.add_space(6.0);

    flow.picked_probe = picked.map(|index| (id, index));

    if make_in_use {
        flow.formations.selected = Some(id);
    }
    if add_probe {
        flow.add_probe();
    }
}

#[cfg(test)]
mod tests {
    use clone_bay_core::formations::Probe;

    use super::*;

    #[test]
    fn views_are_named_plainly() {
        assert_eq!(view_label(View::Above), "above");
        assert_eq!(view_label(View::South), "the south");
        assert_eq!(view_label(View::East), "the east");
    }

    #[test]
    fn the_button_hints_read_as_plain_sentences() {
        // Both once had runs of spaces where a long line had been split.
        for hint in [BALANCE_HINT, CENTRE_HINT] {
            assert!(!hint.contains("  "), "{hint:?}");
            assert!(hint.ends_with('.'), "{hint:?}");
        }
    }

    #[test]
    fn ranges_are_labelled_the_way_the_game_shows_them() {
        assert_eq!(range_label(0.25 * AU), "0.25 AU");
        assert_eq!(range_label(32.0 * AU), "32 AU");
        assert_eq!(
            range_label(3.0 * AU),
            "3.000 AU (not a range the game offers)"
        );
    }

    #[test]
    fn the_balance_note_says_how_far_a_formation_will_shift() {
        let centred = Formation::new(1, "f", vec![Probe::at_km(0.0, 0.0, 0.0, 1.0)]);
        assert_eq!(balance_note(&centred), "Launches exactly as drawn.");
        let off = Formation::new(1, "f", vec![Probe::at_km(300.0, 400.0, 0.0, 1.0)]);
        assert!(
            balance_note(&off).ends_with("launch shifted by 500 km."),
            "{}",
            balance_note(&off)
        );
    }
}
