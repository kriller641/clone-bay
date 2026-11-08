//! Drawing the tab order screen. All decisions live in `tab_flow`.

use clone_bay_core::discovery::FileKind;
use clone_bay_core::tab_order::TabLayout;
use eframe::egui;

use crate::list_model::kind_label;
use crate::tab_flow::TabFlow;
use crate::widgets::{
    problem_line, save_buttons, save_footer_height, save_messages, section, SaveButton,
};

/// The width kept for a tab's name, so the buttons line up.
const NAME_WIDTH: f32 = 260.0;

/// What the user asked for on this frame that the caller has to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    None,
    /// Leave the screen and go back to the file list.
    Close,
}

/// The heading of the screen: whose tabs these are.
pub fn title(flow: &TabFlow) -> String {
    let what = match flow.entry.kind {
        FileKind::Account => "Overview tabs",
        FileKind::Character => "Chat tabs",
    };
    let kind = kind_label(flow.entry.kind).to_lowercase();
    match &flow.name {
        Some(name) => format!("{what} for {name} ({kind} {})", flow.entry.id),
        None => format!("{what} for {kind} {}", flow.entry.id),
    }
}

/// One line under the heading saying what the lists are.
pub fn explanation(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Account => {
            "Each box is one overview window, with its tabs from left to right. Move a tab up \
             to put it further left."
        }
        FileKind::Character => {
            "Each box is one chat window, with its channels from left to right. Move a channel \
             up to put it further left."
        }
    }
}

/// What to say when a file holds no tabs to put in order.
pub fn nothing_to_order(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Account => "This file doesn't store any overview windows yet.",
        FileKind::Character => "This file doesn't store any chat channels in a chat window yet.",
    }
}

/// The line about which tab is selected when the game opens.
pub fn opens_on_text(position: i64) -> String {
    if position == 0 {
        "Overview windows open on their first tab.".to_owned()
    } else {
        format!(
            "Overview windows open on tab {} of each window, not the first.",
            position + 1
        )
    }
}

/// Something the user pressed in the lists.
enum Move {
    Up(usize, usize),
    Down(usize, usize),
    ToGroup(usize, usize, usize),
}

pub fn draw(ui: &mut egui::Ui, flow: &mut TabFlow) -> Request {
    let mut request = Request::None;
    let kind = flow.entry.kind;
    ui.add_space(4.0);
    ui.heading(egui::RichText::new(title(flow)).strong());
    ui.add_space(6.0);

    // Leave room below the lists for the messages and buttons shown.
    let below = save_footer_height(flow.problem.is_some(), &flow.saving);
    let list_height = (ui.available_height() - below).max(60.0);
    let mut pressed = None;
    let mut first_tab = false;
    egui::ScrollArea::vertical()
        .max_height(list_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let Some(layout) = &flow.layout else {
                return;
            };
            if layout.is_empty() {
                ui.label(nothing_to_order(kind));
                return;
            }
            ui.label(explanation(kind));
            ui.add_space(8.0);
            if let Some(position) = layout.opens_on {
                ui.horizontal(|ui| {
                    ui.label(opens_on_text(position));
                    if position != 0 && ui.button("Open on the first tab").clicked() {
                        first_tab = true;
                    }
                });
                ui.add_space(8.0);
            }
            for group in 0..layout.groups.len() {
                section(ui, Some(&layout.groups[group].title), |ui| {
                    pressed = pressed.take().or(tab_rows(ui, layout, group));
                });
            }
        });
    match pressed {
        Some(Move::Up(group, tab)) => flow.move_up(group, tab),
        Some(Move::Down(group, tab)) => flow.move_down(group, tab),
        Some(Move::ToGroup(group, tab, to)) => flow.move_to_group(group, tab, to),
        None => {}
    }
    if first_tab {
        flow.open_on_first_tab();
    }
    ui.add_space(6.0);

    problem_line(ui, flow.problem.as_deref());
    save_messages(ui, &mut flow.saving);
    let unsaved = flow.is_dirty().then(|| "New order not saved".to_owned());
    match save_buttons(ui, flow.busy(), flow.can_save(), flow.is_dirty(), unsaved) {
        Some(SaveButton::Save) => flow.save(),
        Some(SaveButton::Discard) => flow.discard(),
        Some(SaveButton::Back) => request = Request::Close,
        None => {}
    }
    request
}

/// Draws the tabs of one group. Returns what was pressed, if anything.
fn tab_rows(ui: &mut egui::Ui, layout: &TabLayout, group: usize) -> Option<Move> {
    let mut pressed = None;
    let tabs = &layout.groups[group].tabs;
    if tabs.is_empty() {
        ui.weak("No tabs");
    }
    for (index, tab) in tabs.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(NAME_WIDTH, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(NAME_WIDTH);
                    ui.add(egui::Label::new(format!("{}. {}", index + 1, tab.label)).truncate());
                },
            );
            if ui.add_enabled(index > 0, egui::Button::new("Up")).clicked() {
                pressed = Some(Move::Up(group, index));
            }
            if ui
                .add_enabled(index + 1 < tabs.len(), egui::Button::new("Down"))
                .clicked()
            {
                pressed = Some(Move::Down(group, index));
            }
            if layout.can_move_between_groups() && layout.groups.len() > 1 {
                let movable = tabs.len() > 1;
                ui.add_enabled_ui(movable, |ui| {
                    ui.menu_button("Move to", |ui| {
                        for to in 0..layout.groups.len() {
                            if layout.can_move_to_group(group, index, to)
                                && ui.button(&layout.groups[to].title).clicked()
                            {
                                pressed = Some(Move::ToGroup(group, index, to));
                            }
                        }
                    });
                });
            }
        });
    }
    pressed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_selected_tab_is_counted_from_one_for_the_user() {
        assert_eq!(
            opens_on_text(0),
            "Overview windows open on their first tab."
        );
        assert_eq!(
            opens_on_text(1),
            "Overview windows open on tab 2 of each window, not the first."
        );
    }

    #[test]
    fn each_kind_of_file_has_its_own_wording() {
        assert!(explanation(FileKind::Account).contains("overview window"));
        assert!(explanation(FileKind::Character).contains("chat window"));
        assert!(nothing_to_order(FileKind::Account).contains("overview"));
        assert!(nothing_to_order(FileKind::Character).contains("chat"));
    }
}
