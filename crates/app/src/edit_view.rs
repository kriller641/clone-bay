//! Drawing the settings edit screen. All decisions live in `edit_flow`.

use clone_bay_core::bundled::search_bundled_types;
use clone_bay_core::edit::{Content, Kind, ListedId, Reading, ScanFilter};
use clone_bay_core::names::NameBook;
use eframe::egui;

use crate::edit_flow::{
    clear_ships, distance_label, distance_order, format_decimal, group_choices, is_removable,
    parse_decimal, parse_distance, parse_whole, preset_choices, preset_label, removable, set_group,
    system_label, type_label, EditFlow, Row,
};
use crate::list_model::{counted, kind_label};
use crate::theme;
use crate::widgets::{
    dot, problem_line, save_buttons, save_footer_height, save_messages, section, section_title,
    typed_number, SaveButton,
};

/// The width kept for a setting's name, so the controls line up.
const NAME_WIDTH: f32 = 270.0;

/// What the user asked for on this frame that the caller has to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    None,
    /// Leave the edit screen and go back to the file list.
    Close,
}

/// The heading of the edit screen: whose settings these are.
pub fn title(flow: &EditFlow) -> String {
    let kind = kind_label(flow.entry.kind).to_lowercase();
    match &flow.name {
        Some(name) => format!("Settings for {name} ({kind} {})", flow.entry.id),
        None => format!("Settings for {kind} {}", flow.entry.id),
    }
}

pub fn draw(ui: &mut egui::Ui, flow: &mut EditFlow, names: &NameBook) -> Request {
    let mut request = Request::None;
    ui.add_space(4.0);
    ui.heading(egui::RichText::new(title(flow)).strong());
    ui.add_space(6.0);

    // Leave room below the settings for the messages and buttons shown.
    let below = save_footer_height(flow.problem.is_some(), &flow.saving);
    let list_height = (ui.available_height() - below).max(60.0);
    // The ship picked from a distance list's "Add ship" on this frame.
    let mut add = None;
    egui::ScrollArea::vertical()
        .max_height(list_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Rows that share a heading are drawn together as one section.
            let mut start = 0;
            while start < flow.rows.len() {
                let heading = flow.rows[start].setting.heading;
                let length = flow.rows[start..]
                    .iter()
                    .take_while(|row| row.setting.heading == heading)
                    .count();
                let end = start + length;
                let lists = matches!(flow.rows[start].setting.kind, Kind::Distances { .. });
                if lists {
                    // Each distance list is long enough to be a card of its
                    // own, under the heading they share.
                    section_title(ui, heading);
                    ui.add_space(4.0);
                    for index in start..end {
                        section(ui, None, |ui| {
                            if let Some((type_id, name)) =
                                setting_row(ui, &mut flow.rows[index], names)
                            {
                                add = Some((index, type_id, name));
                            }
                        });
                    }
                } else {
                    section(ui, Some(heading), |ui| {
                        for index in start..end {
                            if let Some((type_id, name)) =
                                setting_row(ui, &mut flow.rows[index], names)
                            {
                                add = Some((index, type_id, name));
                            }
                        }
                    });
                }
                start = end;
            }
        });
    if let Some((row, type_id, name)) = add {
        flow.add_type(row, type_id, name);
    }
    ui.add_space(6.0);

    problem_line(ui, flow.problem.as_deref());
    save_messages(ui, &mut flow.saving);
    let changes = flow.changes().len();
    let unsaved =
        (changes > 0).then(|| format!("{} not saved", counted(changes, "change", "changes")));
    match save_buttons(ui, flow.busy(), flow.can_save(), flow.is_dirty(), unsaved) {
        Some(SaveButton::Save) => flow.save(),
        Some(SaveButton::Discard) => flow.discard(),
        Some(SaveButton::Back) => request = Request::Close,
        None => {}
    }
    request
}

/// Draws one setting. Returns the ship picked from its "Add ship", if one
/// was (only a distance list has one).
fn setting_row(ui: &mut egui::Ui, row: &mut Row, names: &NameBook) -> Option<(u64, &'static str)> {
    let label = row.setting.label;
    let changed = row.change().is_some();
    let Some(draft) = &mut row.draft else {
        ui.horizontal(|ui| {
            change_mark(ui, false);
            ui.label(label);
            ui.weak("can't be edited: it is stored in a form this app doesn't understand");
        });
        return None;
    };
    let kind = row.setting.kind;
    let mut touched = false;
    let mut picked = None;
    let mut filter_action = FilterAction::None;
    if let Content::Distances(entries) = draft {
        let extras = DistanceExtras {
            add_input: &mut row.add_input,
            add_note: row.add_note.as_deref(),
            confirm_clear: &mut row.confirm_clear,
        };
        let limits = match kind {
            Kind::Distances { min, max } => (min, max),
            _ => (0, i64::MAX),
        };
        (touched, picked) = distances(ui, label, changed, limits, entries, names, extras);
    } else if let Content::Systems(listed) = draft {
        touched = systems(ui, label, changed, listed, names);
    } else if let Content::Filters(filters) = draft {
        (touched, filter_action) = scan_filters(ui, label, changed, filters, row.open_filter);
    } else {
        ui.horizontal(|ui| {
            change_mark(ui, changed);
            touched = match (kind, &mut *draft) {
                (Kind::Bool | Kind::Flag, Content::Switch(on)) => ui.checkbox(on, label).changed(),
                (Kind::Whole { min, max }, Content::Whole(n)) => {
                    name(ui, label);
                    // Typed, never dragged; see `typed_number`. A value the
                    // file already holds outside the range is shown as it
                    // is: only what the user types is held to the range.
                    let box_id = ui.id().with(("whole", row.setting.id));
                    let (_, typed) =
                        typed_number(ui, box_id, NUMBER_BOX_WIDTH, n.to_string(), |text| {
                            parse_whole(text, min, max)
                        });
                    ui.weak(format!("{min} to {max}"));
                    match typed {
                        Some(typed) if typed != *n => {
                            *n = typed;
                            true
                        }
                        _ => false,
                    }
                }
                (Kind::Choice(options), Content::Whole(n)) => {
                    name(ui, label);
                    choice(ui, row.setting.id, options, n)
                }
                (Kind::Word(options), Content::Word(chosen)) => {
                    name(ui, label);
                    word_choice(ui, row.setting.id, options, chosen)
                }
                (Kind::Preset, Content::Word(chosen)) => {
                    name(ui, label);
                    preset_choice(ui, row.setting.id, &row.options, chosen)
                }
                (Kind::Scaled { min, max, unit, .. }, Content::Number(x)) => {
                    name(ui, label);
                    // Typed, never dragged; see `typed_number`.
                    let box_id = ui.id().with(("scaled", row.setting.id));
                    let (_, typed) =
                        typed_number(ui, box_id, NUMBER_BOX_WIDTH, format_decimal(*x), |text| {
                            parse_decimal(text, min, max)
                        });
                    ui.label(unit);
                    ui.weak(format!(
                        "{} to {}",
                        format_decimal(min),
                        format_decimal(max)
                    ));
                    match typed {
                        Some(typed) if typed != *x => {
                            *x = typed;
                            true
                        }
                        _ => false,
                    }
                }
                (Kind::Number { min, max }, Content::Number(x)) => {
                    name(ui, label);
                    // A typed box here too, not a slider: a slider's number
                    // can be dragged.
                    let box_id = ui.id().with(("number", row.setting.id));
                    let (_, typed) =
                        typed_number(ui, box_id, NUMBER_BOX_WIDTH, format_decimal(*x), |text| {
                            parse_decimal(text, min, max)
                        });
                    ui.weak(format!(
                        "{} to {}",
                        format_decimal(min),
                        format_decimal(max)
                    ));
                    match typed {
                        Some(typed) if typed != *x => {
                            *x = typed;
                            true
                        }
                        _ => false,
                    }
                }
                _ => {
                    name(ui, label);
                    false
                }
            };
            if row.original == Reading::NotStored && !row.touched && !touched {
                ui.weak("not stored in this file; the game is using its default");
            }
        });
    }
    row.touched |= touched;
    match filter_action {
        FilterAction::None => {}
        FilterAction::Add => row.add_filter(),
        FilterAction::Toggle(id) => row.toggle_filter(id),
    }
    picked
}

/// Something pressed in the scan filter list that the row has to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilterAction {
    None,
    /// "Add filter".
    Add,
    /// Show or hide what the filter with this ID shows.
    Toggle(i64),
}

/// A dot at the start of a setting that has a change waiting to be saved.
/// It takes the same room when there is none, so the settings stay in line.
fn change_mark(ui: &mut egui::Ui, changed: bool) {
    if changed {
        dot(ui, theme::ACCENT);
    } else {
        ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    }
}

/// A setting's name, in a column of fixed width so the control after it
/// lines up with the others.
fn name(ui: &mut egui::Ui, label: &str) {
    let size = egui::vec2(NAME_WIDTH, ui.spacing().interact_size.y);
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(size, layout, |ui| {
        ui.set_min_width(NAME_WIDTH);
        ui.label(label);
    });
}

/// A drop-down of fixed choices. Returns whether the choice changed.
fn choice(ui: &mut egui::Ui, id: &str, options: &[(i64, &str)], current: &mut i64) -> bool {
    let shown = options
        .iter()
        .find(|(value, _)| value == current)
        .map_or_else(|| current.to_string(), |(_, label)| (*label).to_owned());
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for (value, label) in options {
                changed |= ui.selectable_value(current, *value, *label).changed();
            }
        });
    changed
}

/// A drop-down of fixed words. Returns whether the choice changed.
fn word_choice(
    ui: &mut egui::Ui,
    id: &str,
    options: &[(&str, &str)],
    current: &mut String,
) -> bool {
    let shown = options
        .iter()
        .find(|(word, _)| *word == current.as_str())
        .map_or_else(|| current.clone(), |(_, label)| (*label).to_owned());
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for (word, label) in options {
                changed |= ui
                    .selectable_value(current, (*word).to_owned(), *label)
                    .changed();
            }
        });
    changed
}

/// A drop-down of the file's own presets. Returns whether the choice
/// changed.
fn preset_choice(ui: &mut egui::Ui, id: &str, options: &[String], current: &mut String) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(preset_label(current))
        .width(PRESET_BOX_WIDTH)
        .show_ui(ui, |ui| {
            for (label, name) in preset_choices(options) {
                changed |= ui
                    .selectable_value(current, name.to_owned(), label)
                    .changed();
            }
        });
    changed
}

/// The width of the preset drop-down: room for a long preset name.
const PRESET_BOX_WIDTH: f32 = 260.0;

/// The width kept for how much a filter holds, so the buttons line up.
const FILTER_COUNT_WIDTH: f32 = 120.0;

/// The width of the box a filter's name is typed into.
const FILTER_NAME_WIDTH: f32 = 220.0;

/// The probe scanner's result filters: each can be renamed or removed, and
/// the kinds of result it shows chosen; new ones can be added. `open` is
/// the filter whose kinds are showing. Returns whether anything changed,
/// and what was pressed that the row has to act on.
fn scan_filters(
    ui: &mut egui::Ui,
    label: &str,
    unsaved: bool,
    filters: &mut Vec<ScanFilter>,
    open: Option<i64>,
) -> (bool, FilterAction) {
    let mut changed = false;
    let mut remove = None;
    let mut action = FilterAction::None;
    ui.horizontal(|ui| {
        change_mark(ui, unsaved);
        name(
            ui,
            &format!("{label} ({})", counted(filters.len(), "filter", "filters")),
        );
        if ui.button("Add filter").clicked() {
            action = FilterAction::Add;
        }
    });
    for (index, filter) in filters.iter_mut().enumerate() {
        let is_open = open == Some(filter.id);
        ui.horizontal(|ui| {
            // In line with the controls of the settings above.
            ui.add_space(8.0 + ui.spacing().item_spacing.x);
            let edit = egui::TextEdit::singleline(&mut filter.name)
                .id(ui.id().with(("filter name", filter.id)))
                .desired_width(FILTER_NAME_WIDTH);
            changed |= ui
                .add_sized([FILTER_NAME_WIDTH, theme::ROW_HEIGHT - 6.0], edit)
                .changed();
            let size = egui::vec2(FILTER_COUNT_WIDTH, ui.spacing().interact_size.y);
            let layout = egui::Layout::left_to_right(egui::Align::Center);
            ui.allocate_ui_with_layout(size, layout, |ui| {
                ui.set_min_width(FILTER_COUNT_WIDTH);
                ui.weak(counted(
                    filter.groups.len(),
                    "kind of result",
                    "kinds of result",
                ));
            });
            let toggle = if is_open {
                "Done choosing"
            } else {
                "Choose what it shows"
            };
            if ui.button(toggle).clicked() {
                action = FilterAction::Toggle(filter.id);
            }
            if ui.button("Remove filter").clicked() {
                remove = Some(index);
            }
        });
        if is_open {
            changed |= filter_groups(ui, filter);
        }
    }
    if let Some(index) = remove {
        filters.remove(index);
    }
    (changed || remove.is_some(), action)
}

/// The tick boxes for what one filter shows, a block for each category.
/// Returns whether anything changed.
fn filter_groups(ui: &mut egui::Ui, filter: &mut ScanFilter) -> bool {
    let mut changed = false;
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, theme::LINE))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .outer_margin(egui::Margin {
            left: 16,
            ..Default::default()
        })
        .show(ui, |ui| {
            for (category, choices) in group_choices(filter) {
                ui.strong(category);
                ui.horizontal_wrapped(|ui| {
                    for (group, group_name, ticked) in choices {
                        let mut on = ticked;
                        if ui.checkbox(&mut on, group_name).changed() {
                            set_group(filter, group, on);
                            changed = true;
                        }
                    }
                });
                ui.add_space(4.0);
            }
        });
    changed
}

/// The solar systems the autopilot avoids. Systems can be taken off the
/// list here; they are put on it in the game. Returns whether anything
/// changed.
fn systems(
    ui: &mut egui::Ui,
    label: &str,
    unsaved: bool,
    listed: &mut Vec<ListedId>,
    names: &NameBook,
) -> bool {
    let mut remove = None;
    let mut clear = false;
    ui.horizontal(|ui| {
        change_mark(ui, unsaved);
        name(
            ui,
            &format!("{label} ({})", counted(listed.len(), "system", "systems")),
        );
        if listed.is_empty() {
            ui.weak("none; systems are added in the game");
        } else if ui.button("Remove all systems").clicked() {
            clear = true;
        }
    });
    for (index, system) in listed.iter().enumerate() {
        ui.horizontal(|ui| {
            // In line with the controls of the settings above.
            ui.add_space(8.0 + ui.spacing().item_spacing.x);
            name(ui, &system_label(names, system.id));
            if ui.button("Remove system").clicked() {
                remove = Some(index);
            }
        });
    }
    if clear {
        listed.clear();
    }
    if let Some(index) = remove {
        listed.remove(index);
    }
    clear || remove.is_some()
}

/// The width of the box for a plain number setting.
const NUMBER_BOX_WIDTH: f32 = 70.0;

/// The width of a distance box: room for seven digits.
const DISTANCE_BOX_WIDTH: f32 = 90.0;

/// A box for typing a distance in metres. Returns whether the distance
/// changed.
fn distance_box(ui: &mut egui::Ui, id: egui::Id, metres: &mut i64, min: i64, max: i64) -> bool {
    let (response, typed) = typed_number(ui, id, DISTANCE_BOX_WIDTH, metres.to_string(), |text| {
        parse_distance(text, min, max)
    });
    response.on_hover_text(format!("Type a distance from {min} to {max} metres"));
    match typed {
        Some(parsed) if parsed != *metres => {
            *metres = parsed;
            true
        }
        _ => false,
    }
}

/// The parts of a distance list's row that aren't the distances themselves.
struct DistanceExtras<'a> {
    /// The ship name being typed into "Add ship".
    add_input: &'a mut String,
    /// What happened to the last "Add ship", when it didn't simply work.
    add_note: Option<&'a str>,
    /// Whether "Clear all" is waiting for a yes or no.
    confirm_clear: &'a mut bool,
}

/// How many ships "Add ship" offers at once.
const SUGGESTIONS: usize = 8;

/// A list of per-type distances. Returns whether anything changed, and the
/// ship picked from "Add ship", if one was.
fn distances(
    ui: &mut egui::Ui,
    label: &str,
    unsaved: bool,
    (min, max): (i64, i64),
    entries: &mut Vec<(i64, i64)>,
    names: &NameBook,
    extras: DistanceExtras<'_>,
) -> (bool, Option<(u64, &'static str)>) {
    let mut changed = false;
    let mut picked = None;
    let mut remove = None;
    ui.horizontal(|ui| {
        change_mark(ui, unsaved);
        ui.label(
            egui::RichText::new(format!(
                "{label} ({})",
                counted(entries.len(), "entry", "entries")
            ))
            // The list's name: larger than body text and at full brightness,
            // which is as bold as the app's one font weight gets.
            .size(17.0)
            .strong()
            .color(egui::Color32::WHITE),
        );
        ui.weak(format!(
            "{} to {}",
            distance_label(min),
            distance_label(max)
        ));
        let clear = ui.add_enabled(
            removable(entries) > 0,
            egui::Button::new("Clear all").small(),
        );
        if clear
            .on_hover_text(
                "Removes every ship from this list, after asking. The default for \
                 everything else stays.",
            )
            .clicked()
        {
            *extras.confirm_clear = true;
        }
    });
    if *extras.confirm_clear {
        ui.horizontal(|ui| {
            ui.colored_label(
                theme::PROBLEM,
                format!(
                    "Remove all {} from {label}? The default stays.",
                    counted(removable(entries), "ship", "ships")
                ),
            );
            if ui.button("Yes, clear all").clicked() {
                clear_ships(entries);
                changed = true;
                *extras.confirm_clear = false;
            }
            if ui.button("No, keep them").clicked() {
                *extras.confirm_clear = false;
            }
        });
    }
    ui.indent(label, |ui| {
        egui::Grid::new(label).striped(true).show(ui, |ui| {
            for index in distance_order(entries, names) {
                let (type_id, metres) = &mut entries[index];
                ui.label(type_label(names, *type_id));
                let box_id = ui.id().with(("distance", label, *type_id));
                changed |= distance_box(ui, box_id, metres, min, max);
                ui.weak("m");
                ui.weak(distance_label(*metres));
                // The default for everything else is always there: it can be
                // changed but not removed.
                if !is_removable(*type_id) {
                    ui.label("");
                } else if ui
                    .small_button("Remove")
                    .on_hover_text("This kind of object goes back to the default distance")
                    .clicked()
                {
                    remove = Some(index);
                }
                ui.end_row();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Add ship");
            ui.add(
                egui::TextEdit::singleline(extras.add_input)
                    .hint_text("Type part of its name")
                    .desired_width(200.0),
            );
            // Beside the box, not a hover tooltip: a tooltip would cover the
            // ships offered just below it.
            ui.weak("Pick a ship below to add it at 500 m, then change the distance");
        });
        // Ships matching what has been typed; picking one adds it.
        let matches = search_bundled_types(extras.add_input, SUGGESTIONS);
        if !matches.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for (type_id, name) in matches {
                    if ui.small_button(name).clicked() {
                        picked = Some((type_id, name));
                    }
                }
            });
        } else if !extras.add_input.trim().is_empty() {
            ui.weak("No ship or structure has that in its name.");
        }
        if let Some(note) = extras.add_note {
            ui.colored_label(theme::PROBLEM, note);
        }
    });
    if let Some(index) = remove {
        entries.remove(index);
        changed = true;
    }
    (changed, picked)
}
