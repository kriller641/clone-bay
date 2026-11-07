//! Drawing the window. All decisions live in `state` and `list_model`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use clone_bay_core::discovery::{file_name, FileKind};
use clone_bay_core::platform::default_eve_root;
use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::copy_view;
use crate::edit_view;
use crate::formation_view;
use crate::list_model::{
    counted, display_name, format_age, format_size, kind_label, mains_first, name_cell,
    status_label, tally, visible_rows, KindFilter, SortColumn, Status,
};
use crate::restore_view;
use crate::state::{RowAction, Screen, State};
use crate::tab_view;
use crate::theme;
use crate::widgets::{marker, panel_frame, primary_button, sort_header};

/// The size the window opens at, in points: width, then height.
pub const DEFAULT_WINDOW_SIZE: [f32; 2] = [1560.0, 1280.0];

pub struct App {
    pub(crate) state: State,
}

impl App {
    /// `folder` overrides the automatically detected EVE settings folder.
    pub fn new(ctx: &egui::Context, folder: Option<PathBuf>) -> Self {
        theme::apply(ctx);
        let ctx = ctx.clone();
        let wake = Arc::new(move || ctx.request_repaint());
        Self {
            state: State::new(folder.or_else(default_eve_root), wake),
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        // What applies to the whole profile, then what narrows the list.
        ui.horizontal(|ui| {
            ui.label("Profile");
            let current = self
                .state
                .selected_profile
                .and_then(|index| self.state.profiles.get(index))
                .map_or_else(|| "None".to_owned(), profile_label);
            let mut chosen = None;
            egui::ComboBox::from_id_salt("profile")
                .selected_text(current)
                .width(360.0)
                .show_ui(ui, |ui| {
                    for (index, profile) in self.state.profiles.iter().enumerate() {
                        let selected = self.state.selected_profile == Some(index);
                        if ui
                            .selectable_label(selected, profile_label(profile))
                            .clicked()
                        {
                            chosen = Some(index);
                        }
                    }
                });
            if let Some(index) = chosen {
                self.state.select_profile(index);
            }
            if ui
                .add_enabled(
                    self.state.selected_profile.is_some(),
                    egui::Button::new("Open folder"),
                )
                .on_hover_text("Show this profile's folder in File Explorer")
                .clicked()
            {
                self.state.show_profile_folder();
            }
            if ui
                .button("Rescan")
                .on_hover_text("List this profile's files again")
                .clicked()
            {
                self.state.rescan();
            }
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.state.search)
                    .hint_text("Search name or ID")
                    .desired_width(200.0),
            );
            ui.selectable_value(&mut self.state.filter, KindFilter::All, "All");
            ui.selectable_value(&mut self.state.filter, KindFilter::Characters, "Characters");
            ui.selectable_value(&mut self.state.filter, KindFilter::Accounts, "Accounts");
        });
        ui.add_space(4.0);
    }

    fn folder_prompt(&mut self, ui: &mut egui::Ui) {
        ui.label("Folder that holds your EVE settings:");
        ui.horizontal(|ui| {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.state.folder_input)
                    .hint_text(r"%LOCALAPPDATA%\CCP\EVE")
                    .desired_width(480.0),
            );
            let entered =
                edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if ui.button("Open").clicked() || entered {
                let folder = PathBuf::from(self.state.folder_input.trim());
                self.state.open_folder(&folder);
            }
        });
    }

    fn file_table(&mut self, ui: &mut egui::Ui) {
        let state = &self.state;
        let visible = visible_rows(
            &state.rows,
            state.filter,
            &state.search,
            state.sort,
            &state.names,
        );
        // Until the user sorts the list themselves, their main character
        // and account stay at the top.
        let visible = if state.mains_on_top {
            mains_first(&state.rows, visible, &state.names)
        } else {
            visible
        };
        if visible.is_empty() {
            ui.weak(if state.rows.is_empty() {
                "This profile has no settings files."
            } else {
                "No files match the search."
            });
            return;
        }
        let now = SystemTime::now();
        // Text in a row would otherwise be selectable and swallow the click
        // that is meant to select the row.
        ui.style_mut().interaction.selectable_labels = false;
        let mut clicked_header = None;
        let mut clicked_file = None;
        let mut menu_action = None;
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            // Cells would otherwise stack what they hold, at the top of the row.
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(190.0).at_least(80.0).clip(true))
            .column(Column::auto().at_least(80.0))
            .column(Column::auto().at_least(120.0))
            .column(Column::auto().at_least(80.0))
            .column(Column::remainder().clip(true))
            .header(theme::ROW_HEIGHT, |mut header| {
                let columns = [
                    ("Name", SortColumn::Name),
                    ("Type", SortColumn::Kind),
                    ("ID", SortColumn::Id),
                    ("Size", SortColumn::Size),
                    ("Last changed", SortColumn::Modified),
                ];
                for (title, column) in columns {
                    header.col(|ui| {
                        if sort_header(ui, title, column, state.sort) {
                            clicked_header = Some(column);
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(theme::ROW_HEIGHT, visible.len(), |mut table_row| {
                    let row = &state.rows[visible[table_row.index()]];
                    let entry = &row.entry;
                    table_row.set_selected(state.selected_file.as_ref() == Some(&entry.path));
                    table_row.col(|ui| {
                        let name = name_cell(&state.names, entry.kind, entry.id);
                        let unusable = matches!(row.status, Status::Unreadable(_));
                        match (name, entry.kind) {
                            (Some(name), _) if unusable => ui.weak(name),
                            (Some(name), _) => ui.strong(name),
                            (None, FileKind::Account) => ui.weak("no label"),
                            (None, FileKind::Character) => ui.label(""),
                        };
                        if unusable {
                            let text = egui::RichText::new("Can't use").color(theme::PROBLEM);
                            marker(ui, theme::PROBLEM, text)
                                .on_hover_text(status_label(&row.status));
                        }
                    });
                    table_row.col(|ui| {
                        ui.weak(kind_label(entry.kind));
                    });
                    table_row.col(|ui| {
                        ui.label(egui::RichText::new(entry.id.to_string()).monospace().weak());
                    });
                    table_row.col(|ui| {
                        ui.weak(format_size(entry.size));
                    });
                    table_row.col(|ui| {
                        ui.weak(format_age(entry.modified, now));
                    });
                    let response = table_row.response();
                    // A right-click selects the row too, so the menu and the
                    // panel beside the list are about the same file.
                    if response.clicked() || response.secondary_clicked() {
                        clicked_file = Some(entry.path.clone());
                    }
                    response.context_menu(|ui| {
                        if let Some(action) = row_menu(ui, state, entry.kind, entry.id) {
                            menu_action = Some(action);
                            ui.close();
                        }
                    });
                });
            });
        if let Some(column) = clicked_header {
            // The user has chosen an order: stop holding the mains on top.
            self.state.mains_on_top = false;
            self.state.sort = self.state.sort.clicked(column);
        }
        if let Some(path) = clicked_file {
            self.state.selected_file = Some(path);
        }
        if let Some(action) = menu_action {
            self.state.act_on_selected(action);
        }
    }

    /// The "Mark as main" box for the selected file, and the question it
    /// asks before taking the mark away or moving it from another file.
    fn main_box(&mut self, ui: &mut egui::Ui, kind: FileKind, id: u64) {
        let what = kind_label(kind).to_lowercase();
        let mut marked = self.state.main_of(kind) == Some(id);
        let tick = ui
            .checkbox(&mut marked, "Mark as main")
            .on_hover_text(format!(
                "Your main {what} is listed first and has (Main) beside its name. There is one \
             main {what} at a time."
            ));
        if tick.changed() {
            self.state.toggle_main_selected();
        }
        let Some((_, _, make_main)) = self.state.main_prompt_for_selected() else {
            return;
        };
        let question = if make_main {
            let current = self
                .state
                .main_of(kind)
                .and_then(|main| name_cell(&self.state.names, kind, main))
                .unwrap_or_else(|| format!("Another {what} (Main)"));
            format!("{current} is your main {what} now. Make this one the main instead?")
        } else {
            format!("Stop marking this as your main {what}?")
        };
        ui.colored_label(theme::WARNING, question);
        ui.horizontal(|ui| {
            if ui.button("Yes").clicked() {
                self.state.confirm_main();
            }
            if ui.button("No").clicked() {
                self.state.cancel_main();
            }
        });
    }

    fn details(&mut self, ui: &mut egui::Ui) {
        let Some(row) = self.state.selected_row() else {
            ui.add_space(8.0);
            ui.weak("Select a file to see what it contains.");
            return;
        };
        let (kind, id) = (row.entry.kind, row.entry.id);
        ui.add_space(6.0);
        let kind_and_id = format!("{} {id}", kind_label(kind));
        match display_name(&self.state.names, kind, id) {
            Some(name) => {
                ui.heading(egui::RichText::new(name).strong());
                ui.label(kind_and_id);
            }
            None => {
                ui.heading(egui::RichText::new(kind_and_id).strong());
            }
        }
        ui.label(
            egui::RichText::new(file_name(&row.entry.path))
                .monospace()
                .weak(),
        );
        ui.add_space(6.0);
        self.main_box(ui, kind, id);
        ui.add_space(6.0);

        let usable = self.state.can_copy_selected();
        ui.horizontal_wrapped(|ui| {
            let copy = ui.add_enabled(usable, primary_button("Copy to other files"));
            if copy
                .on_hover_text("Choose which files, then review before anything changes")
                .clicked()
            {
                self.state.begin_copy();
            }
            let edit = ui.add_enabled(usable, egui::Button::new("Edit settings"));
            if edit
                .on_hover_text("Change individual settings in this file")
                .clicked()
            {
                self.state.begin_edit();
            }
            let tabs = ui.add_enabled(usable, egui::Button::new(tab_view::button_label(kind)));
            if tabs
                .on_hover_text("Change the order the game shows these tabs in")
                .clicked()
            {
                self.state.begin_tabs();
            }
            if self.state.can_edit_formations()
                && ui
                    .button("Edit probe formations")
                    .on_hover_text("Add, change and remove this account's custom probe formations")
                    .clicked()
            {
                self.state.begin_formations();
            }
            let restore = ui.add_enabled(
                self.state.can_restore_selected(),
                egui::Button::new("Restore from a backup"),
            );
            if restore
                .on_hover_text("Put this file back the way one of its backups holds it")
                .clicked()
            {
                self.state.begin_restore();
            }
        });
        ui.add_space(6.0);
        if kind == FileKind::Account {
            // Account IDs can't be looked up, so the user names them.
            ui.horizontal(|ui| {
                ui.label("Label");
                let edit = egui::TextEdit::singleline(self.state.label_draft(id))
                    .hint_text("A name for this account")
                    .desired_width(180.0);
                if ui.add(edit).changed() {
                    self.state.commit_label_draft();
                }
            });
            ui.add_space(6.0);
        }
        let Some(row) = self.state.selected_row() else {
            return;
        };
        match &row.status {
            Status::Pending => {
                ui.weak("Checking…");
            }
            Status::Unreadable(error) => {
                let text = egui::RichText::new("This file can't be used.").color(theme::PROBLEM);
                ui.horizontal(|ui| marker(ui, theme::PROBLEM, text));
                ui.label(format!("Reason: {error}."));
                ui.weak("Nothing has been changed.");
            }
            Status::Readable(summary) => {
                let written_by = if summary.format_version == 0 {
                    "the EVE client"
                } else {
                    "a settings tool"
                };
                ui.horizontal(|ui| {
                    marker(ui, theme::OK, format!("Last written by {written_by}."));
                });
                ui.add_space(6.0);
                ui.strong(counted(summary.sections.len(), "section", "sections"));
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Grid::new("sections").striped(true).show(ui, |ui| {
                        for section in &summary.sections {
                            ui.monospace(&section.name);
                            match section.entries {
                                Some(count) => ui.label(counted(count, "entry", "entries")),
                                None => ui.weak("single setting"),
                            };
                            ui.end_row();
                        }
                    });
                });
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

/// The size of the window that says the app is already open.
pub const ALREADY_OPEN_WINDOW_SIZE: [f32; 2] = [520.0, 160.0];

/// The small window shown instead of the app when another window of it is
/// already open.
pub struct AlreadyOpen;

impl AlreadyOpen {
    pub fn new(ctx: &egui::Context) -> Self {
        theme::apply(ctx);
        Self
    }

    pub fn draw(ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Clone Bay is already open");
            ui.label("Switch to the window that is open, or close it before opening another.");
            ui.add_space(8.0);
            if ui.add(primary_button("Close")).clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }
}

impl eframe::App for AlreadyOpen {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        Self::draw(ui);
    }
}

impl App {
    /// Draws the whole window into `ui`.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        self.state.receive();

        let State { screen, names, .. } = &mut self.state;
        let close = match screen {
            Screen::Files => None,
            Screen::Formations(flow) => Some(
                egui::CentralPanel::default()
                    .show(ui, |ui| formation_view::draw(ui, flow))
                    .inner
                    == formation_view::Request::Close,
            ),
            Screen::Tabs(flow) => Some(
                egui::CentralPanel::default()
                    .show(ui, |ui| tab_view::draw(ui, flow))
                    .inner
                    == tab_view::Request::Close,
            ),
            Screen::Restore(flow) => Some(
                egui::CentralPanel::default()
                    .show(ui, |ui| restore_view::draw(ui, flow))
                    .inner
                    == restore_view::Request::Close,
            ),
            Screen::Edit(flow) => Some(
                egui::CentralPanel::default()
                    .show(ui, |ui| edit_view::draw(ui, flow, names))
                    .inner
                    == edit_view::Request::Close,
            ),
            Screen::Copy(flow) => Some(
                egui::CentralPanel::default()
                    .show(ui, |ui| copy_view::draw(ui, flow))
                    .inner
                    == copy_view::Request::Close,
            ),
        };
        if let Some(close) = close {
            if close {
                self.state.close_screen();
            }
            return;
        }

        let frame = panel_frame(ui);
        egui::Panel::top("toolbar")
            .frame(frame)
            .show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(tally(&self.state.rows));
                ui.separator();
                ui.weak("Files are only changed when you copy or save, and are backed up first.");
                if let Some(note) = self.state.names_note() {
                    ui.separator();
                    ui.weak(note);
                }
            });
        });
        if !self.state.profiles.is_empty() {
            egui::Panel::right("details")
                .frame(frame)
                // Not resizable: there is nothing to gain from a wider panel,
                // and dragging its edge only snapped back.
                .resizable(false)
                .default_size(320.0)
                .min_size(220.0)
                .show(ui, |ui| self.details(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(problem) = &self.state.problem {
                ui.colored_label(ui.visuals().warn_fg_color, problem);
                ui.add_space(6.0);
            }
            if self.state.profiles.is_empty() {
                self.folder_prompt(ui);
            } else {
                self.file_table(ui);
            }
        });
    }
}

/// A row's right-click menu: the same choices as the buttons beside the
/// list, for the file that was right-clicked (which is selected by then).
/// Returns what was chosen.
fn row_menu(ui: &mut egui::Ui, state: &State, kind: FileKind, id: u64) -> Option<RowAction> {
    let mut chosen = None;
    let mut marked = state.main_of(kind) == Some(id);
    if ui.checkbox(&mut marked, "Mark as main").changed() {
        chosen = Some(RowAction::ToggleMain);
    }
    ui.separator();
    let usable = state.can_copy_selected();
    let mut item = |ui: &mut egui::Ui, enabled: bool, label: &str, action: RowAction| {
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            chosen = Some(action);
        }
    };
    item(ui, usable, "Copy to other files", RowAction::Copy);
    item(ui, usable, "Edit settings", RowAction::Edit);
    item(ui, usable, tab_view::button_label(kind), RowAction::Tabs);
    if state.can_edit_formations() {
        item(ui, true, "Edit probe formations", RowAction::Formations);
    }
    item(
        ui,
        state.can_restore_selected(),
        "Restore from a backup",
        RowAction::Restore,
    );
    chosen
}

fn profile_label(profile: &clone_bay_core::discovery::Profile) -> String {
    format!("{}  ({})", profile.name, profile.install)
}

/// Helpers for tests that drive the real UI off-screen.
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::Path;
    use std::thread;
    use std::time::{Duration, Instant};

    use eframe::egui;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;

    use super::App;
    use crate::list_model::{Row, Status};
    use crate::services::Services;
    use crate::state::test_support::quiet;
    use crate::state::State;

    pub type Ui = Harness<'static, App>;

    /// The real UI showing `root`, with every file already checked.
    pub fn harness(root: &Path) -> Ui {
        harness_with(root, Services::for_tests())
    }

    pub fn harness_with(root: &Path, services: Services) -> Ui {
        let app = App {
            state: State::with_services(Some(root.to_path_buf()), quiet(), services),
        };
        let mut harness = Harness::builder()
            // The size the real window opens at.
            .with_size(egui::Vec2::from(crate::app::DEFAULT_WINDOW_SIZE))
            .build_ui_state(|ui, app: &mut App| app.draw(ui), app);
        crate::theme::apply(&harness.ctx);
        let pending = |row: &Row| row.status == Status::Pending;
        wait_until(&mut harness, |app| !app.state.rows.iter().any(pending));
        harness
    }

    /// Keeps drawing until `ready`, for work that finishes on another thread.
    pub fn wait_until(harness: &mut Ui, ready: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            harness.step();
            if ready(harness.state()) {
                break;
            }
            assert!(Instant::now() < deadline, "timed out waiting for the app");
            thread::sleep(Duration::from_millis(5));
        }
        settle(harness);
    }

    /// A few frames, so layout and input from the last change take effect.
    /// (`Harness::run` can't be used: it never returns while a spinner is
    /// animating.)
    pub fn settle(harness: &mut Ui) {
        harness.run_steps(3);
    }

    /// A real pointer click in the middle of the widget showing `label`,
    /// scrolling it into view first as a user would.
    pub fn click(harness: &mut Ui, label: &str) {
        harness.get_by_label(label).scroll_to_me();
        settle(harness);
        let at = harness.get_by_label(label).rect().center();
        harness.hover_at(at);
        harness.step();
        harness.drag_at(at);
        harness.step();
        harness.drop_at(at);
        settle(harness);
    }

    pub fn shows(harness: &Ui, label: &str) -> bool {
        harness.query_by_label(label).is_some()
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::{NodeT, Queryable};

    use super::test_support::{click, harness, harness_with};
    use super::*;
    use crate::list_model::{Sort, SortColumn};
    use crate::services::Services;
    use crate::state::test_support::eve_root;

    #[test]
    fn a_second_window_says_the_app_is_already_open() {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::from(ALREADY_OPEN_WINDOW_SIZE))
            .build_ui(AlreadyOpen::draw);
        AlreadyOpen::new(&harness.ctx);
        harness.run();
        harness.get_by_label("Clone Bay is already open");
        harness
            .get_by_label("Switch to the window that is open, or close it before opening another.");
        harness.get_by_label("Close").click();
        harness.run();
    }

    #[test]
    fn lists_each_file_with_its_status() {
        let root = eve_root();
        let harness = harness(root.path());
        harness.get_by_label("22");
        harness.get_by_label("11");
        // Only the file that can't be used is marked. Why is shown when it
        // is selected.
        harness.get_by_label("Can't use");
        assert!(harness.query_by_label("OK, 1 section").is_none());
        harness.get_by_label("2 files, 1 can't be used");
        harness.get_by_label("Select a file to see what it contains.");
    }

    #[test]
    fn clicking_a_row_shows_what_the_file_contains() {
        let root = eve_root();
        let mut harness = harness(root.path());
        click(&mut harness, "11");
        let selected = harness.state().state.selected_row().map(|row| row.entry.id);
        assert_eq!(selected, Some(11));
        harness.get_by_label("core_user_11.dat");
        harness.get_by_label("Account 11");
        harness.get_by_label("Last written by a settings tool.");
        harness.get_by_label("1 section");
        harness.get_by_label("ui");
        harness.get_by_label("0 entries");
        assert!(harness
            .query_by_label("Select a file to see what it contains.")
            .is_none());
    }

    #[test]
    fn clicking_a_damaged_file_explains_why_it_cannot_be_used() {
        let root = eve_root();
        let mut harness = harness(root.path());
        click(&mut harness, "22");
        harness.get_by_label("core_char_22.dat");
        harness.get_by_label("This file can't be used.");
        harness.get_by_label("Reason: can't decode it (not a marshal stream (bad signature)).");
        harness.get_by_label("Nothing has been changed.");
    }

    #[test]
    fn clicking_a_column_header_sorts_by_it() {
        let root = eve_root();
        let mut harness = harness(root.path());
        assert_eq!(harness.state().state.sort, Sort::default());
        click(&mut harness, "ID");
        let sort = harness.state().state.sort;
        assert_eq!((sort.column, sort.descending), (SortColumn::Id, false));
        // The header now carries the direction arrow.
        click(&mut harness, "ID \u{2B06}");
        let sort = harness.state().state.sort;
        assert_eq!((sort.column, sort.descending), (SortColumn::Id, true));
    }

    #[test]
    fn the_kind_filter_hides_the_other_kind() {
        let root = eve_root();
        let mut harness = harness(root.path());
        click(&mut harness, "Accounts");
        assert_eq!(harness.state().state.filter, KindFilter::Accounts);
        harness.get_by_label("11");
        assert!(harness.query_by_label("22").is_none());
        click(&mut harness, "Characters");
        harness.get_by_label("22");
        assert!(harness.query_by_label("11").is_none());
    }

    #[test]
    fn rescan_lists_a_file_added_since() {
        let root = eve_root();
        let mut harness = harness(root.path());
        let first = &harness.state().state.rows[0].entry.path;
        let folder = first.parent().unwrap().to_path_buf();
        std::fs::write(folder.join("core_char_77.dat"), b"new").unwrap();
        click(&mut harness, "Rescan");
        assert_eq!(harness.state().state.rows.len(), 3);
        harness.get_by_label("77");
    }

    #[test]
    fn open_folder_shows_the_chosen_profile_folder() {
        let root = eve_root();
        let shown = Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = Arc::clone(&shown);
        let services = Services {
            show_folder: Arc::new(move |folder| {
                record.lock().unwrap().push(folder.to_path_buf());
                Ok(())
            }),
            ..Services::for_tests()
        };
        let mut harness = harness_with(root.path(), services);
        click(&mut harness, "Open folder");
        let profile = harness.state().state.profiles[0].path.clone();
        assert_eq!(*shown.lock().unwrap(), [profile]);
        assert_eq!(harness.state().state.problem, None);
    }

    #[test]
    fn a_folder_that_cannot_be_shown_says_why() {
        let root = eve_root();
        let services = Services {
            show_folder: Arc::new(|_| Err("the folder X isn't there any more".to_owned())),
            ..Services::for_tests()
        };
        let mut harness = harness_with(root.path(), services);
        click(&mut harness, "Open folder");
        harness.get_by_label("Couldn't open the folder: the folder X isn't there any more.");
    }

    /// Right-clicks the row showing `row_label` and returns where, since the
    /// menu opens there.
    fn right_click(harness: &mut test_support::Ui, row_label: &str) -> egui::Pos2 {
        let at = harness.get_by_label(row_label).rect().center();
        harness.get_by_label(row_label).click_secondary();
        test_support::settle(harness);
        at
    }

    /// The menu item `label` in the menu opened at `at`. The panel beside
    /// the list has a button with the same text; the menu's is the one
    /// nearest where the menu opened.
    fn menu_item<'a>(
        harness: &'a test_support::Ui,
        at: egui::Pos2,
        label: &'a str,
    ) -> egui_kittest::Node<'a> {
        harness
            .get_all_by_label(label)
            .min_by(|a, b| {
                let (a, b) = (
                    a.rect().center().distance(at),
                    b.rect().center().distance(at),
                );
                a.total_cmp(&b)
            })
            .unwrap()
    }

    fn choose(harness: &mut test_support::Ui, at: egui::Pos2, label: &str) {
        menu_item(harness, at, label).click();
        test_support::settle(harness);
    }

    #[test]
    fn right_clicking_a_row_selects_it_and_offers_the_panel_buttons() {
        let root = eve_root();
        let mut harness = harness(root.path());
        let at = right_click(&mut harness, "11");
        let selected = harness.state().state.selected_row().map(|row| row.entry.id);
        assert_eq!(selected, Some(11));
        for label in [
            "Mark as main",
            "Copy to other files",
            "Edit settings",
            "Order overview tabs",
            "Edit probe formations",
            "Restore from a backup",
        ] {
            // Once in the panel, once in the menu.
            assert_eq!(harness.get_all_by_label(label).count(), 2, "{label}");
            assert!(!menu_item(&harness, at, label)
                .accesskit_node()
                .is_disabled());
        }
    }

    #[test]
    fn the_menu_opens_the_chosen_screen() {
        let root = eve_root();
        let mut harness = harness(root.path());
        let at = right_click(&mut harness, "11");
        choose(&mut harness, at, "Edit settings");
        assert!(matches!(harness.state().state.screen, Screen::Edit(_)));
        // The menu closed with the choice.
        assert!(harness.query_by_label("Copy to other files").is_none());
    }

    #[test]
    fn the_menu_opens_the_tab_and_formation_screens() {
        let root = eve_root();
        let mut harness = harness(root.path());
        let at = right_click(&mut harness, "11");
        choose(&mut harness, at, "Order overview tabs");
        assert!(matches!(harness.state().state.screen, Screen::Tabs(_)));
        harness.state_mut().state.close_screen();
        test_support::settle(&mut harness);
        let at = right_click(&mut harness, "11");
        choose(&mut harness, at, "Edit probe formations");
        assert!(matches!(
            harness.state().state.screen,
            Screen::Formations(_)
        ));
    }

    #[test]
    fn the_menu_acts_on_the_row_right_clicked_not_the_one_selected_before() {
        let root = eve_root();
        let mut harness = harness(root.path());
        click(&mut harness, "22");
        let at = right_click(&mut harness, "11");
        choose(&mut harness, at, "Copy to other files");
        let Screen::Copy(_) = &harness.state().state.screen else {
            panic!("the copy screen should be open");
        };
        assert_eq!(
            harness.state().state.selected_row().map(|row| row.entry.id),
            Some(11)
        );
    }

    #[test]
    fn mark_as_main_in_the_menu_marks_the_file() {
        let root = eve_root();
        let mut harness = harness(root.path());
        let at = right_click(&mut harness, "11");
        choose(&mut harness, at, "Mark as main");
        assert_eq!(harness.state().state.main_of(FileKind::Account), Some(11));
        assert!(harness.query_by_label("Edit settings").is_some());
        assert_eq!(harness.get_all_by_label("Edit settings").count(), 1);
    }

    #[test]
    fn a_damaged_file_can_only_be_restored_from_the_menu() {
        let root = eve_root();
        let mut harness = harness(root.path());
        let at = right_click(&mut harness, "22");
        for label in ["Copy to other files", "Edit settings", "Order chat tabs"] {
            assert!(
                menu_item(&harness, at, label)
                    .accesskit_node()
                    .is_disabled(),
                "{label}"
            );
        }
        assert!(harness.query_by_label("Edit probe formations").is_none());
        choose(&mut harness, at, "Restore from a backup");
        assert!(matches!(harness.state().state.screen, Screen::Restore(_)));
    }

    #[test]
    fn with_no_settings_found_it_asks_for_a_folder() {
        let empty = tempfile::tempdir().unwrap();
        let harness = harness(empty.path());
        harness.get_by_label("Folder that holds your EVE settings:");
        harness.get_by_label("Open");
        assert!(harness
            .query_by_label("Select a file to see what it contains.")
            .is_none());
    }
}
