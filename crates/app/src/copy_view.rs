//! Drawing the copy screens. All decisions live in `copy_flow`.

use std::path::Path;
use std::time::SystemTime;

use clone_bay_core::copy::{Action, CopyPlan, CopyReport, Outcome};
use clone_bay_core::discovery::{file_name, FileKind};
use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::copy_flow::{ChatPicker, CopyFlow, Stage};
use crate::list_model::{counted, format_age, format_size, kind_label, SortColumn};
use crate::theme::{self, ROW_HEIGHT};
use crate::widgets::{eve_closed_box, marker, note, primary_button, sort_header, steps};

/// The steps of a copy, as shown at the top of every copy screen.
const STEPS: [&str; 3] = ["Choose", "Review", "Done"];

/// What the user asked for on this frame that the caller has to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    None,
    /// Leave the copy screens and go back to the file list.
    Close,
}

/// What each kind of file keeps, in the user's words.
pub fn kept_summary(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Account => "module rack layout, weapon groups and drone groups",
        FileKind::Character => "auto-repeat and auto-reload settings and chat channels",
    }
}

/// Shown on the review when the copy won't pass on the tab the source's
/// overview was left on.
pub const FIRST_TAB_NOTE: &str = "The source's overview was left on a tab other than the first. \
     The files copied onto will open on their first tab.";

/// One line saying what will happen to a target.
pub fn action_text(action: &Action, kind: FileKind) -> String {
    match action {
        Action::ExactCopy => "Exact copy".to_owned(),
        Action::KeepOwn { kept } if kept.is_empty() => format!(
            "Copy. It has no {} stored, so it gets none",
            kept_summary(kind)
        ),
        Action::KeepOwn { kept } => format!("Copy, keeping its own {}", kept.join(", ")),
        Action::Chosen { channels_added: 0 } => "Copy the chosen settings".to_owned(),
        Action::Chosen { channels_added } => format!(
            "Copy the chosen settings, adding {}",
            counted(*channels_added, "chat channel", "chat channels")
        ),
        Action::Skip { reason } => format!("Skipped: {reason}"),
    }
}

/// One line saying what happened to a target.
pub fn outcome_text(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Written => "Copied".to_owned(),
        Outcome::Skipped(reason) => format!("Skipped: {reason}"),
        Outcome::Failed(reason) => format!("NOT copied: {reason}"),
    }
}

/// The headline of the review screen.
pub fn review_headline(plan: &CopyPlan) -> String {
    let skipped = plan.targets.len() - plan.writable();
    let mut text = match plan.writable() {
        0 => "None of the chosen files can be overwritten".to_owned(),
        n => format!("{} will be overwritten", counted(n, "file", "files")),
    };
    if skipped > 0 {
        text.push_str(&format!(", {skipped} skipped"));
    }
    text.push('.');
    text
}

/// The headline of the result screen.
pub fn result_headline(report: &CopyReport) -> String {
    let mut text = format!("Copied to {}.", counted(report.written(), "file", "files"));
    let skipped = report
        .outcomes
        .iter()
        .filter(|(_, outcome)| matches!(outcome, Outcome::Skipped(_)))
        .count();
    if skipped > 0 {
        text.push_str(&format!(" {} skipped.", counted(skipped, "file", "files")));
    }
    if report.failed() > 0 {
        text.push_str(&format!(
            " {} could not be copied.",
            counted(report.failed(), "file", "files")
        ));
    }
    text
}

/// The heading of every copy screen: whose settings are being copied.
pub fn source_title(flow: &CopyFlow) -> String {
    let kind = kind_label(flow.source.kind).to_lowercase();
    match &flow.source_name {
        Some(name) => format!("Copy settings from {name} ({kind} {})", flow.source.id),
        None => format!("Copy settings from {kind} {}", flow.source.id),
    }
}

pub fn draw(ui: &mut egui::Ui, flow: &mut CopyFlow) -> Request {
    let mut request = Request::None;
    ui.add_space(4.0);
    steps(ui, &STEPS, flow.stage.step());
    ui.heading(egui::RichText::new(source_title(flow)).strong());
    ui.add_space(6.0);
    match &flow.stage {
        Stage::Picking => request = picking(ui, flow),
        Stage::Planning => working(ui, "Checking files", flow.progress()),
        Stage::Copying => working(ui, "Copying", flow.progress()),
        Stage::Reviewing(_) => request = reviewing(ui, flow),
        Stage::Done(result) => {
            let mut every_file = flow.show_every_file;
            match result {
                Ok(report) => done(ui, report, &|path| flow.name_of(path), &mut every_file),
                Err(reason) => {
                    let reason = crate::state::capitalise(reason);
                    ui.colored_label(ui.visuals().error_fg_color, format!("{reason}."));
                }
            }
            flow.show_every_file = every_file;
            ui.add_space(8.0);
            if ui.button("Back to the file list").clicked() {
                request = Request::Close;
            }
        }
    }
    request
}

fn working(ui: &mut egui::Ui, what: &str, (done, total): (usize, usize)) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(format!("{what}: {done} of {total}"));
    });
}

fn picking(ui: &mut egui::Ui, flow: &mut CopyFlow) -> Request {
    let mut request = Request::None;
    let kept = kept_summary(flow.source.kind);
    note(ui, theme::NOTE, |ui| {
        ui.label(format!(
            "Choose what to copy and which files to copy it onto. Each file always keeps its own \
             {kept}."
        ));
    });
    ui.add_space(6.0);
    if flow.candidates.is_empty() {
        ui.weak("There are no other files of this kind in this profile.");
        ui.add_space(8.0);
        if ui.button("Cancel").clicked() {
            request = Request::Close;
        }
        return request;
    }

    // Leave room below the two lists for the buttons.
    let list_height = (ui.available_height() - 48.0).max(ROW_HEIGHT * 3.0);
    ui.columns(2, |columns| {
        columns[0].set_max_height(list_height);
        scope_picker(&mut columns[0], flow);
        target_picker(&mut columns[1], flow, list_height);
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let check = ui.add_enabled(flow.can_check(), primary_button("Check files"));
        if check
            .on_hover_text("Looks at each file and shows what would happen. Changes nothing.")
            .clicked()
        {
            flow.start_plan();
        }
        if ui.button("Cancel").clicked() {
            request = Request::Close;
        }
        ui.label(format!(
            "{} ticked",
            counted(flow.ticked(), "file", "files")
        ));
        if !flow.scope.chooses_something() {
            ui.weak("Nothing is chosen to copy.");
        }
    });
    request
}

/// The left half of the picking screen: what to copy.
fn scope_picker(ui: &mut egui::Ui, flow: &mut CopyFlow) {
    let scope = &mut flow.scope;
    ui.strong("What to copy");
    ui.radio_value(&mut scope.everything, true, "Everything");
    ui.radio_value(&mut scope.everything, false, "Only what I tick");
    if scope.everything {
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("scope")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.indent("groups", |ui| {
                for (group, ticked) in &mut scope.groups {
                    ui.checkbox(ticked, group.label);
                }
                if let Some(chat) = &mut scope.chat {
                    chat_picker(ui, chat);
                }
                ui.checkbox(&mut scope.everything_else, "Everything else")
                    .on_hover_text("Every setting that isn't in one of the groups above");
            });
        });
}

fn chat_picker(ui: &mut egui::Ui, chat: &mut ChatPicker) {
    if let Some(problem) = &chat.problem {
        ui.add_enabled(false, egui::Checkbox::new(&mut false, "Chat channels"));
        ui.weak(format!(
            "Can't read this character's chat channels: {problem}."
        ));
        return;
    }
    if chat.channels.is_empty() {
        ui.add_enabled(false, egui::Checkbox::new(&mut false, "Chat channels"));
        ui.weak("This character has no player channels to copy.");
        return;
    }
    ui.checkbox(&mut chat.ticked, "Chat channels")
        .on_hover_text(
            "Adds the channels you tick to each file. Channels a file already has are kept.",
        );
    if chat.ticked {
        ui.indent("channels", |ui| {
            for (channel, ticked) in &mut chat.channels {
                ui.checkbox(ticked, channel.name.as_str());
            }
        });
    }
}

/// The right half of the picking screen: which files to copy onto.
fn target_picker(ui: &mut egui::Ui, flow: &mut CopyFlow, list_height: f32) {
    ui.strong("Copy onto");
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut flow.search)
                .hint_text("Search name or ID")
                .desired_width(150.0),
        );
        if ui.button("Tick all shown").clicked() {
            flow.tick_visible(true);
        }
        if ui.button("Untick all shown").clicked() {
            flow.tick_visible(false);
        }
        ui.weak("Shift-click ticks a range");
    });
    ui.add_space(4.0);
    let visible = flow.visible();
    let now = SystemTime::now();
    let (candidates, sort) = (&flow.candidates, flow.sort);
    let mut clicked_header = None;
    let mut toggled = None;
    // Text in a row would otherwise be selectable and swallow the click
    // that is meant to tick the row.
    ui.style_mut().interaction.selectable_labels = false;
    // Leave room for the heading, the search row and the column headers.
    let body_height = (list_height - 3.0 * ROW_HEIGHT - 16.0).max(ROW_HEIGHT * 2.0);
    TableBuilder::new(ui)
        .id_salt("targets")
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        // Cells would otherwise stack what they hold, at the top of the row.
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .auto_shrink(false)
        .max_scroll_height(body_height)
        .column(Column::exact(22.0))
        .column(Column::initial(170.0).at_least(80.0).clip(true))
        .column(Column::auto().at_least(100.0))
        .column(Column::auto().at_least(70.0))
        .column(Column::remainder().clip(true))
        .header(ROW_HEIGHT, |mut header| {
            header.col(|_| {});
            let columns = [
                ("Name", SortColumn::Name),
                ("ID", SortColumn::Id),
                ("Size", SortColumn::Size),
                ("Last changed", SortColumn::Modified),
            ];
            for (title, column) in columns {
                header.col(|ui| {
                    if sort_header(ui, title, column, sort) {
                        clicked_header = Some(column);
                    }
                });
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, visible.len(), |mut table_row| {
                let index = visible[table_row.index()];
                let candidate = &candidates[index];
                let usable = candidate.unusable.is_none();
                table_row.col(|ui| {
                    let mut ticked = candidate.ticked;
                    let tick = egui::Checkbox::without_text(&mut ticked);
                    if ui.add_enabled(usable, tick).changed() {
                        toggled = Some(index);
                    }
                });
                table_row.col(|ui| {
                    match (&candidate.name, candidate.entry.kind) {
                        (Some(name), _) if usable => ui.strong(name),
                        (Some(name), _) => ui.weak(name),
                        (None, FileKind::Account) => ui.weak("no label"),
                        (None, FileKind::Character) => ui.label(""),
                    };
                    if let Some(reason) = &candidate.unusable {
                        let text = egui::RichText::new("Can't use").color(theme::PROBLEM);
                        marker(ui, theme::PROBLEM, text)
                            .on_hover_text(format!("Can't be used: {reason}"));
                    }
                });
                table_row.col(|ui| {
                    ui.label(
                        egui::RichText::new(candidate.entry.id.to_string())
                            .monospace()
                            .weak(),
                    );
                });
                table_row.col(|ui| {
                    ui.weak(format_size(candidate.entry.size));
                });
                table_row.col(|ui| {
                    ui.weak(format_age(candidate.entry.modified, now));
                });
                // Clicking anywhere in a row ticks it.
                if table_row.response().clicked() {
                    toggled = Some(index);
                }
            });
        });
    if let Some(column) = clicked_header {
        flow.sort = flow.sort.clicked(column);
    }
    if let Some(index) = toggled {
        let shift = ui.input(|input| input.modifiers.shift);
        flow.click_row(index, shift);
    }
}

/// Which targets of a plan the review lists, by their place in the plan:
/// all of them, or only the ones that will be skipped.
pub fn review_rows(plan: &CopyPlan, every_file: bool) -> Vec<usize> {
    (0..plan.targets.len())
        .filter(|&index| every_file || matches!(plan.targets[index].action, Action::Skip { .. }))
        .collect()
}

fn reviewing(ui: &mut egui::Ui, flow: &mut CopyFlow) -> Request {
    let mut request = Request::None;
    let Stage::Reviewing(plan) = &flow.stage else {
        return request;
    };
    ui.strong(review_headline(plan));
    ui.label(format!("Copying: {}.", flow.scope.summary()));
    if plan.resets_overview_tab() {
        ui.label(FIRST_TAB_NOTE);
    }
    note(ui, theme::NOTE, |ui| {
        ui.strong("Nothing has been changed yet.");
        ui.label(
            "Before a file is overwritten, its original is saved in clone-bay-backups \
             inside the profile folder.",
        );
    });
    ui.add_space(6.0);

    // Leave room below the list for what is shown under it.
    let mut below = 44.0 + ROW_HEIGHT;
    if flow.refusal.is_some() {
        below += 30.0;
    }
    if flow.needs_eve_confirmation {
        below += 44.0;
    }
    // Only the files that won't be copied onto are listed, unless the user
    // asks for all of them: hundreds of lines saying the same thing hide the
    // few that matter.
    let rows = review_rows(plan, flow.show_every_file);
    ui.horizontal(|ui| {
        ui.checkbox(&mut flow.show_every_file, "Show every file");
        if rows.is_empty() {
            ui.weak("No problems found: every file ticked will be copied onto.");
        } else if !flow.show_every_file {
            ui.weak("Only the files that will be skipped are listed.");
        }
    });
    let list_height = (ui.available_height() - below).max(ROW_HEIGHT * 3.0);
    egui::ScrollArea::vertical()
        .max_height(list_height)
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
            egui::Grid::new("plan").striped(true).show(ui, |ui| {
                for target in rows[range].iter().map(|&index| &plan.targets[index]) {
                    ui.monospace(file_name(&target.path));
                    ui.label(flow.name_of(&target.path).unwrap_or_default());
                    let text = action_text(&target.action, plan.kind);
                    if matches!(target.action, Action::Skip { .. }) {
                        ui.colored_label(ui.visuals().warn_fg_color, text);
                    } else {
                        ui.label(text);
                    }
                    ui.end_row();
                }
            });
        });
    ui.add_space(6.0);

    if let Some(refusal) = &flow.refusal {
        ui.colored_label(ui.visuals().error_fg_color, format!("{refusal}."));
    }
    if flow.needs_eve_confirmation {
        eve_closed_box(ui, &mut flow.eve_closed_confirmed);
    }
    ui.horizontal(|ui| {
        let copy = ui.add_enabled(flow.can_copy(), primary_button("Copy now"));
        if copy.clicked() {
            flow.start_copy();
        }
        if ui.button("Back").clicked() {
            flow.back_to_picking();
        }
        if ui.button("Cancel").clicked() {
            request = Request::Close;
        }
    });
    request
}

/// Which files the result screen lists, by their place in the report: all
/// of them, or only the ones that were not copied onto.
pub fn result_rows(report: &CopyReport, every_file: bool) -> Vec<usize> {
    (0..report.outcomes.len())
        .filter(|&index| every_file || report.outcomes[index].1 != Outcome::Written)
        .collect()
}

fn done<'a>(
    ui: &mut egui::Ui,
    report: &CopyReport,
    name_of: &dyn Fn(&Path) -> Option<&'a str>,
    every_file: &mut bool,
) {
    if report.failed() > 0 {
        ui.colored_label(ui.visuals().error_fg_color, result_headline(report));
    } else {
        ui.strong(result_headline(report));
    }
    ui.label("The originals were saved in:");
    ui.monospace(report.backup_dir.display().to_string());
    ui.weak(
        "To undo: with EVE closed, copy the files from that folder back into the profile folder.",
    );
    ui.add_space(6.0);
    // As on the review: the headline is the summary, and only the files
    // that were not copied onto are listed unless all are asked for.
    let rows = result_rows(report, *every_file);
    ui.horizontal(|ui| {
        ui.checkbox(every_file, "Show every file");
        if rows.is_empty() {
            ui.weak("Every file was copied onto.");
        } else if !*every_file {
            ui.weak("Only the files that were not copied onto are listed.");
        }
    });
    let list_height = (ui.available_height() - 48.0 - ROW_HEIGHT).max(ROW_HEIGHT * 3.0);
    egui::ScrollArea::vertical()
        .max_height(list_height)
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
            egui::Grid::new("outcomes").striped(true).show(ui, |ui| {
                for (path, outcome) in rows[range].iter().map(|&index| &report.outcomes[index]) {
                    ui.monospace(file_name(path));
                    ui.label(name_of(path).unwrap_or_default());
                    let text = outcome_text(outcome);
                    match outcome {
                        Outcome::Written => ui.label(text),
                        Outcome::Skipped(_) => ui.colored_label(ui.visuals().warn_fg_color, text),
                        Outcome::Failed(_) => ui.colored_label(ui.visuals().error_fg_color, text),
                    };
                    ui.end_row();
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;

    use super::*;

    fn report(outcomes: &[Outcome]) -> CopyReport {
        CopyReport {
            backup_dir: PathBuf::from("backups"),
            outcomes: outcomes
                .iter()
                .enumerate()
                .map(|(n, outcome)| (PathBuf::from(format!("core_user_{n}.dat")), outcome.clone()))
                .collect(),
        }
    }

    #[test]
    fn the_result_headline_counts_copies_and_mentions_failures_only_when_there_are_some() {
        let failed = Outcome::Failed("x".into());
        let skipped = Outcome::Skipped("x".into());
        assert_eq!(
            result_headline(&report(&[Outcome::Written])),
            "Copied to 1 file."
        );
        assert_eq!(
            result_headline(&report(&[Outcome::Written, Outcome::Written, skipped])),
            "Copied to 2 files. 1 file skipped."
        );
        assert_eq!(
            result_headline(&report(&[Outcome::Written, failed.clone()])),
            "Copied to 1 file. 1 file could not be copied."
        );
        assert_eq!(
            result_headline(&report(&[failed.clone(), failed])),
            "Copied to 0 files. 2 files could not be copied."
        );
    }

    #[test]
    fn progress_is_shown_as_done_of_total() {
        let harness = Harness::new_ui(|ui| working(ui, "Copying", (3, 10)));
        harness.get_by_label("Copying: 3 of 10");
    }

    #[test]
    fn the_result_screen_lists_only_what_was_not_copied_until_asked() {
        let mixed = report(&[
            Outcome::Written,
            Outcome::Skipped("the file is read-only".into()),
            Outcome::Failed("denied".into()),
            Outcome::Written,
        ]);
        assert_eq!(result_rows(&mixed, false), [1, 2]);
        assert_eq!(result_rows(&mixed, true), [0, 1, 2, 3]);
        assert_eq!(
            result_headline(&mixed),
            "Copied to 2 files. 1 file skipped. 1 file could not be copied."
        );

        let harness = Harness::new_ui(|ui| done(ui, &mixed, &|_| None, &mut false));
        harness.get_by_label("Only the files that were not copied onto are listed.");
        harness.get_by_label("core_user_1.dat");
        harness.get_by_label("core_user_2.dat");
        assert!(harness.query_by_label("core_user_0.dat").is_none());
        assert!(harness.query_by_label("core_user_3.dat").is_none());
        assert!(harness.query_by_label("Copied").is_none());

        let clean = report(&[Outcome::Written, Outcome::Written]);
        assert!(result_rows(&clean, false).is_empty());
        let harness = Harness::new_ui(|ui| done(ui, &clean, &|_| None, &mut false));
        harness.get_by_label("Copied to 2 files.");
        harness.get_by_label("Every file was copied onto.");
        assert!(harness.query_by_label("core_user_0.dat").is_none());
    }

    #[test]
    fn the_result_screen_lists_every_file_with_what_happened() {
        let report = report(&[
            Outcome::Written,
            Outcome::Skipped("the file is read-only".into()),
            Outcome::Failed("it couldn't be written (denied), so it was left untouched".into()),
        ]);
        let name_of = |path: &Path| {
            path.ends_with("core_user_1.dat")
                .then_some("Second account")
        };
        // With every file asked for.
        let harness = Harness::new_ui(|ui| done(ui, &report, &name_of, &mut true));
        harness.get_by_label("Copied to 1 file. 1 file skipped. 1 file could not be copied.");
        harness.get_by_label("backups");
        harness.get_by_label("core_user_0.dat");
        harness.get_by_label("Copied");
        harness.get_by_label("core_user_1.dat");
        harness.get_by_label("Second account");
        harness.get_by_label("Skipped: the file is read-only");
        harness.get_by_label("core_user_2.dat");
        harness
            .get_by_label("NOT copied: it couldn't be written (denied), so it was left untouched");
    }
}
