//! Drawing the restore screen. All decisions live in `restore_flow`.

use eframe::egui;

use crate::list_model::kind_label;
use crate::restore_flow::{backup_label, RestoreFlow};
use crate::widgets::{eve_closed_box, primary_button, problem_line, section};

/// What the user asked for on this frame that the caller has to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    None,
    /// Leave the screen and go back to the file list.
    Close,
}

/// The heading of the screen: which file this is.
pub fn title(flow: &RestoreFlow) -> String {
    let kind = kind_label(flow.entry.kind).to_lowercase();
    match &flow.name {
        Some(name) => format!("Restore {name} ({kind} {})", flow.entry.id),
        None => format!("Restore {kind} {}", flow.entry.id),
    }
}

pub fn draw(ui: &mut egui::Ui, flow: &mut RestoreFlow) -> Request {
    let mut request = Request::None;
    ui.add_space(4.0);
    ui.heading(egui::RichText::new(title(flow)).strong());
    ui.add_space(6.0);
    ui.label(
        "Pick a backup to put back. What the file holds now is backed up first, so a restore \
         can be undone the same way.",
    );
    ui.add_space(8.0);

    // Leave room below the list for the messages and buttons.
    let mut below = 56.0;
    if flow.problem.is_some() {
        below += 30.0;
    }
    if flow.saving.saved_to.is_some() {
        below += 30.0;
    }
    if flow.saving.needs_eve_confirmation {
        below += 44.0;
    }
    let list_height = (ui.available_height() - below).max(60.0);
    let mut picked = None;
    egui::ScrollArea::vertical()
        .max_height(list_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            section(ui, Some("Backups, newest first"), |ui| {
                if flow.backups.is_empty() {
                    ui.label(
                        "There are no backups of this file. One is made the first time the app \
                         changes it.",
                    );
                }
                for (index, backup) in flow.backups.iter().enumerate() {
                    let chosen = flow.selected == Some(index);
                    let row = ui
                        .selectable_label(chosen, backup_label(backup))
                        .on_hover_text(backup.path.display().to_string());
                    if row.clicked() {
                        picked = Some(index);
                    }
                }
            });
        });
    if let Some(index) = picked {
        flow.select(index);
    }
    ui.add_space(6.0);

    problem_line(ui, flow.problem.as_deref());
    if let Some(folder) = &flow.saving.saved_to {
        ui.label(format!(
            "Restored. What the file held before was backed up in {}",
            folder.display()
        ));
    }
    if flow.saving.needs_eve_confirmation {
        eve_closed_box(ui, &mut flow.saving.eve_closed_confirmed);
    }
    ui.horizontal(|ui| {
        if flow.busy() {
            ui.spinner();
            ui.label("Restoring");
            return;
        }
        if ui
            .add_enabled(flow.can_restore(), primary_button("Restore this backup"))
            .on_hover_text("Backs up what the file holds now first")
            .clicked()
        {
            flow.restore();
        }
        if ui.button("Back to the file list").clicked() {
            request = Request::Close;
        }
    });
    request
}
