//! What the window is showing and how it changes. No drawing happens here.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;

use clone_bay_core::discovery::{
    default_profile_index, list_settings_files, locate_profiles, FileKind, Profile,
};
use clone_bay_core::names::{look_up, Lookup, NameBook};

use crate::copy_flow::{CopyFlow, Stage};
use crate::edit_flow::EditFlow;
use crate::formation_flow::FormationFlow;
use crate::list_model::{display_name, tabs_button_label, KindFilter, Row, Sort, Status};
use crate::loader::Loader;
use crate::restore_flow::RestoreFlow;
use crate::services::Services;
use crate::tab_flow::TabFlow;

/// Called from background threads to ask the window to redraw.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// What fills the window. One screen shows at a time.
pub enum Screen {
    /// The list of files.
    Files,
    /// The copy screens, for the file being copied from.
    Copy(CopyFlow),
    /// The edit screen, for the file being edited.
    Edit(EditFlow),
    /// The probe formation editor, for one account. Boxed: it is much larger
    /// than the other screens.
    Formations(Box<FormationFlow>),
    /// The tab order screen, for one file.
    Tabs(TabFlow),
    /// Putting one file back from a backup.
    Restore(RestoreFlow),
}

/// What a question about losing unsaved changes is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leaving {
    /// Going back to the file list.
    Screen,
    /// Closing the window.
    Window,
}

/// What a row's right-click menu offers: a second way to the buttons beside
/// the list, for the file that was right-clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    ToggleMain,
    Copy,
    Edit,
    Tabs,
    Formations,
    Restore,
}

/// One thing that can be done to the selected file. The panel beside the
/// list and a row's right-click menu both draw this one list, so they
/// always offer the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileChoice {
    pub action: RowAction,
    pub label: &'static str,
    /// Shown when the pointer rests on it.
    pub hover: &'static str,
    pub enabled: bool,
    /// Why it can't be used, shown when the pointer rests on it while it is
    /// greyed out. Empty when it can be used.
    pub why_not: &'static str,
    /// The step that moves the user forward: drawn as the filled button.
    pub primary: bool,
}

pub struct State {
    /// The folder the user typed, shown when nothing was found automatically.
    pub folder_input: String,
    pub profiles: Vec<Profile>,
    pub selected_profile: Option<usize>,
    /// A message for the user when a folder couldn't be used.
    pub problem: Option<String>,
    pub rows: Vec<Row>,
    pub filter: KindFilter,
    pub search: String,
    pub sort: Sort,
    /// Whether the user's main character and account are kept at the top of
    /// the list. True until the user sorts the list themselves.
    pub mains_on_top: bool,
    /// A change of main waiting for a yes or no: the file it is about, and
    /// whether it would become the main (as opposed to stop being it).
    pub main_prompt: Option<(FileKind, u64, bool)>,
    pub selected_file: Option<PathBuf>,
    pub screen: Screen,
    /// The user asked to leave a screen, or close the window, while it has
    /// changes not saved: waiting for them to say whether to lose the
    /// changes or stay.
    pub leave_prompt: Option<Leaving>,
    /// Why the window wasn't closed: work on the files was still under way.
    pub busy_notice: Option<String>,
    /// The user chose to close the window; `app` closes it.
    pub close_window: bool,
    types_task: Option<Receiver<Lookup>>,
    /// Character names and account labels.
    pub names: NameBook,
    /// Why names couldn't be looked up or saved, if they couldn't.
    pub names_problem: Option<String>,
    names_task: Option<Receiver<Lookup>>,
    /// The account label being typed, kept as typed so spaces aren't
    /// trimmed away mid-word.
    label_draft: Option<(u64, String)>,
    batch: u64,
    loader: Loader,
    services: Services,
    wake: Wake,
}

impl State {
    /// Opens `folder` if there is one, otherwise starts empty.
    pub fn new(folder: Option<PathBuf>, wake: Wake) -> Self {
        Self::with_services(folder, wake, Services::real())
    }

    pub fn with_services(folder: Option<PathBuf>, wake: Wake, services: Services) -> Self {
        let (names, names_problem) = match &services.names_file {
            Some(file) => match NameBook::load(file) {
                Ok(names) => (names, None),
                Err(error) => (
                    NameBook::default(),
                    Some(format!("Couldn't read the saved names ({error})")),
                ),
            },
            None => (NameBook::default(), None),
        };
        let mut state = Self {
            screen: Screen::Files,
            leave_prompt: None,
            busy_notice: None,
            close_window: false,
            types_task: None,
            names,
            names_problem,
            names_task: None,
            label_draft: None,
            folder_input: String::new(),
            profiles: Vec::new(),
            selected_profile: None,
            problem: None,
            rows: Vec::new(),
            filter: KindFilter::default(),
            search: String::new(),
            sort: Sort::default(),
            mains_on_top: true,
            main_prompt: None,
            selected_file: None,
            batch: 0,
            loader: Loader::new(),
            services,
            wake,
        };
        match folder {
            Some(folder) => state.open_folder(&folder),
            None => {
                state.problem = Some(
                    "Couldn't find where EVE keeps its settings on this PC. \
                     Enter the folder below."
                        .to_owned(),
                );
            }
        }
        state
    }

    /// Looks for settings profiles at `folder` and shows the first one.
    pub fn open_folder(&mut self, folder: &Path) {
        self.folder_input = folder.display().to_string();
        self.profiles.clear();
        self.selected_profile = None;
        self.show_files(Vec::new());
        match locate_profiles(folder) {
            Ok(profiles) if profiles.is_empty() => {
                self.problem = Some(format!(
                    "No EVE settings were found in {}.",
                    folder.display()
                ));
            }
            Ok(profiles) => {
                self.problem = None;
                self.profiles = profiles;
                if let Some(index) = default_profile_index(&self.profiles) {
                    self.select_profile(index);
                }
            }
            Err(error) => self.problem = Some(capitalise(&error.to_string())),
        }
    }

    /// Lists the files in a profile and starts checking them.
    pub fn select_profile(&mut self, index: usize) {
        let Some(profile) = self.profiles.get(index) else {
            return;
        };
        self.selected_profile = Some(index);
        match list_settings_files(&profile.path) {
            Ok(files) => {
                self.problem = None;
                self.show_files(
                    files
                        .into_iter()
                        .map(|entry| Row {
                            entry,
                            status: Status::Pending,
                        })
                        .collect(),
                );
            }
            Err(error) => {
                self.problem = Some(capitalise(&error.to_string()));
                self.show_files(Vec::new());
            }
        }
    }

    /// Opens the chosen profile's folder in the system's file manager.
    pub fn show_profile_folder(&mut self) {
        let Some(profile) = self
            .selected_profile
            .and_then(|index| self.profiles.get(index))
        else {
            return;
        };
        if let Err(error) = (self.services.show_folder)(&profile.path) {
            self.problem = Some(capitalise(&format!("couldn't open the folder: {error}.")));
        }
    }

    /// Lists the current profile's files again.
    pub fn rescan(&mut self) {
        if let Some(index) = self.selected_profile {
            self.select_profile(index);
        }
    }

    /// Shows `rows` and checks the files that need it. A file listed before
    /// with the same size and time of last change keeps the status it had,
    /// so a rescan only reads what is new or has changed.
    fn show_files(&mut self, mut rows: Vec<Row>) {
        // Keep the selection across a rescan when the file is still there.
        if let Some(selected) = &self.selected_file {
            if !rows.iter().any(|row| &row.entry.path == selected) {
                self.selected_file = None;
            }
        }
        for row in &mut rows {
            if let Some(known) = self
                .rows
                .iter()
                .find(|old| old.entry == row.entry && old.status != Status::Pending)
            {
                row.status = known.status.clone();
            }
        }
        let to_check = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.status == Status::Pending)
            .map(|(index, row)| (index, row.entry.path.clone()))
            .collect();
        self.rows = rows;
        let wake = Arc::clone(&self.wake);
        self.batch = self.loader.start(to_check, move || wake());
        self.start_name_lookup();
    }

    /// Looks up the names of listed characters that aren't known yet, in
    /// the background. Does nothing if a lookup is already running; the
    /// rest are picked up when it finishes.
    fn start_name_lookup(&mut self) {
        if self.names_task.is_some() {
            return;
        }
        let mut ids: Vec<u64> = self
            .rows
            .iter()
            .filter(|row| row.entry.kind == FileKind::Character)
            .map(|row| row.entry.id)
            .filter(|&id| self.names.needs_lookup(id))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        if ids.is_empty() {
            return;
        }
        let source = Arc::clone(&self.services.name_source);
        let wake = Arc::clone(&self.wake);
        let (sender, receiver) = mpsc::channel();
        self.names_task = Some(receiver);
        thread::spawn(move || {
            let lookup = look_up(source.as_ref(), &ids);
            // If the window has gone there is nobody to tell.
            let _ = sender.send(lookup);
            wake();
        });
    }

    fn receive_names(&mut self) {
        let Some(lookup) = self
            .names_task
            .as_ref()
            .and_then(|task| task.try_recv().ok())
        else {
            return;
        };
        self.names_task = None;
        let finished = lookup.stopped.is_none();
        let learned = !lookup.found.is_empty();
        for (id, name) in lookup.found {
            self.names.set_character(id, name);
        }
        self.names_problem = lookup.stopped.map(|error| capitalise(&error.to_string()));
        if learned {
            self.save_names();
        }
        if finished {
            // Another profile may have been opened in the meantime.
            self.start_name_lookup();
        }
    }

    fn save_names(&mut self) {
        if let Some(file) = &self.services.names_file {
            if let Err(error) = self.names.save(file) {
                self.names_problem = Some(format!("Couldn't save names ({error})"));
            }
        }
    }

    /// A short note about names for the status line, when there is
    /// something to say.
    pub fn names_note(&self) -> Option<String> {
        if self.names_task.is_some() {
            Some("Looking up character names…".to_owned())
        } else {
            self.names_problem.clone()
        }
    }

    /// The label being typed for account `id`, starting from the saved one.
    pub fn label_draft(&mut self, id: u64) -> &mut String {
        let stale = !matches!(&self.label_draft, Some((draft_id, _)) if *draft_id == id);
        if stale {
            let saved = self.names.account_label(id).unwrap_or_default().to_owned();
            self.label_draft = Some((id, saved));
        }
        &mut self
            .label_draft
            .get_or_insert_with(|| (id, String::new()))
            .1
    }

    /// Saves the label being typed.
    pub fn commit_label_draft(&mut self) {
        if let Some((id, label)) = &self.label_draft {
            self.names.set_account_label(*id, label);
            self.save_names();
        }
    }

    /// The ID of the main character or main account, if one is marked.
    pub fn main_of(&self, kind: FileKind) -> Option<u64> {
        match kind {
            FileKind::Character => self.names.main_character(),
            FileKind::Account => self.names.main_account(),
        }
    }

    fn set_main(&mut self, kind: FileKind, id: Option<u64>) {
        match kind {
            FileKind::Character => self.names.set_main_character(id),
            FileKind::Account => self.names.set_main_account(id),
        }
        self.save_names();
    }

    /// The "Mark as main" box of the selected file was clicked.
    ///
    /// Marking a file when there is no main of its kind happens at once.
    /// Unmarking the main, or marking a file when another is the main,
    /// waits for the user to confirm.
    pub fn toggle_main_selected(&mut self) {
        let Some((kind, id)) = self
            .selected_row()
            .map(|row| (row.entry.kind, row.entry.id))
        else {
            return;
        };
        match self.main_of(kind) {
            None => {
                self.main_prompt = None;
                self.set_main(kind, Some(id));
            }
            Some(main) if main == id => self.main_prompt = Some((kind, id, false)),
            Some(_) => self.main_prompt = Some((kind, id, true)),
        }
    }

    /// The change of main waiting for a yes or no, if it is about the
    /// selected file.
    pub fn main_prompt_for_selected(&self) -> Option<(FileKind, u64, bool)> {
        let row = self.selected_row()?;
        self.main_prompt
            .filter(|(kind, id, _)| (*kind, *id) == (row.entry.kind, row.entry.id))
    }

    /// The user said yes to the change of main that was waiting.
    pub fn confirm_main(&mut self) {
        if let Some((kind, id, make_main)) = self.main_prompt.take() {
            self.set_main(kind, make_main.then_some(id));
        }
    }

    /// The user said no to the change of main that was waiting.
    pub fn cancel_main(&mut self) {
        self.main_prompt = None;
    }

    /// Whether the selected file's settings can be copied onto others.
    pub fn can_copy_selected(&self) -> bool {
        self.selected_row()
            .is_some_and(|row| matches!(row.status, Status::Readable(_)))
    }

    /// Opens the copy screens for the selected file.
    pub fn begin_copy(&mut self) {
        if !self.can_copy_selected() {
            return;
        }
        if let Some(row) = self.selected_row() {
            let flow = CopyFlow::new(
                &row.entry,
                &self.rows,
                &self.names,
                self.services.clone(),
                Arc::clone(&self.wake),
            );
            self.screen = Screen::Copy(flow);
        }
    }

    /// Opens the edit screen for the selected file.
    pub fn begin_edit(&mut self) {
        if !self.can_copy_selected() {
            return;
        }
        if let Some(row) = self.selected_row() {
            let name = display_name(&self.names, row.entry.kind, row.entry.id).map(str::to_owned);
            let flow = EditFlow::new(
                &row.entry,
                name,
                self.services.clone(),
                Arc::clone(&self.wake),
            );
            self.screen = Screen::Edit(flow);
            self.start_type_lookup();
        }
    }

    /// Whether the selected file is an account whose formations can be
    /// edited.
    pub fn can_edit_formations(&self) -> bool {
        self.can_copy_selected()
            && self
                .selected_row()
                .is_some_and(|row| row.entry.kind == FileKind::Account)
    }

    /// Opens the probe formation editor for the selected account.
    pub fn begin_formations(&mut self) {
        if !self.can_edit_formations() {
            return;
        }
        if let Some(row) = self.selected_row() {
            let name = display_name(&self.names, row.entry.kind, row.entry.id).map(str::to_owned);
            let flow = FormationFlow::new(
                &row.entry,
                name,
                self.services.clone(),
                Arc::clone(&self.wake),
            );
            self.screen = Screen::Formations(Box::new(flow));
        }
    }

    /// Opens the tab order screen for the selected file.
    pub fn begin_tabs(&mut self) {
        if !self.can_copy_selected() {
            return;
        }
        if let Some(row) = self.selected_row() {
            let name = display_name(&self.names, row.entry.kind, row.entry.id).map(str::to_owned);
            let flow = TabFlow::new(
                &row.entry,
                name,
                self.services.clone(),
                Arc::clone(&self.wake),
            );
            self.screen = Screen::Tabs(flow);
        }
    }

    /// Whether a file is selected that a backup could be restored onto. A
    /// file that can't be used can be restored too: that is when it is most
    /// needed.
    pub fn can_restore_selected(&self) -> bool {
        self.selected_row()
            .is_some_and(|row| !matches!(row.status, Status::Pending))
    }

    /// Opens the restore screen for the selected file.
    pub fn begin_restore(&mut self) {
        if !self.can_restore_selected() {
            return;
        }
        if let Some(row) = self.selected_row() {
            let name = display_name(&self.names, row.entry.kind, row.entry.id).map(str::to_owned);
            let flow = RestoreFlow::new(
                &row.entry,
                name,
                self.services.clone(),
                Arc::clone(&self.wake),
            );
            self.screen = Screen::Restore(flow);
        }
    }

    /// What can be done to the selected file, in the order offered. "Mark
    /// as main" is not here: it is a tick box with its own question.
    pub fn file_choices(&self) -> Vec<FileChoice> {
        let Some(row) = self.selected_row() else {
            return Vec::new();
        };
        let kind = row.entry.kind;
        let why_not = match row.status {
            Status::Pending => "Still checking this file.",
            Status::Unreadable(_) => {
                "This file can't be used. Restoring it from a backup may help."
            }
            Status::Readable(_) => "",
        };
        let usable = self.can_copy_selected();
        let choice = |action, label, hover, enabled: bool| FileChoice {
            action,
            label,
            hover,
            enabled,
            why_not: if enabled { "" } else { why_not },
            primary: action == RowAction::Copy,
        };
        let mut choices = vec![
            choice(
                RowAction::Copy,
                "Copy to other files",
                "Choose which files, then review before anything changes",
                usable,
            ),
            choice(
                RowAction::Edit,
                "Edit settings",
                "Change individual settings in this file",
                usable,
            ),
            choice(
                RowAction::Tabs,
                tabs_button_label(kind),
                "Change the order the game shows these tabs in",
                usable,
            ),
        ];
        // Only accounts hold formations, so only they offer the editor.
        if self.can_edit_formations() {
            choices.push(choice(
                RowAction::Formations,
                "Edit probe formations",
                "Add, change and remove this account's custom probe formations",
                true,
            ));
        }
        choices.push(choice(
            RowAction::Restore,
            "Restore from a backup",
            "Put this file back the way one of its backups holds it",
            self.can_restore_selected(),
        ));
        choices
    }

    /// Does what was chosen from a row's right-click menu or the panel
    /// beside the list, to the selected file.
    pub fn act_on_selected(&mut self, action: RowAction) {
        match action {
            RowAction::ToggleMain => self.toggle_main_selected(),
            RowAction::Copy => self.begin_copy(),
            RowAction::Edit => self.begin_edit(),
            RowAction::Tabs => self.begin_tabs(),
            RowAction::Formations => self.begin_formations(),
            RowAction::Restore => self.begin_restore(),
        }
    }

    /// Whether the screen showing is saving or copying.
    fn screen_busy(&self) -> bool {
        match &self.screen {
            Screen::Files => false,
            Screen::Copy(flow) => flow.busy(),
            Screen::Edit(flow) => flow.busy(),
            Screen::Formations(flow) => flow.busy(),
            Screen::Tabs(flow) => flow.busy(),
            Screen::Restore(flow) => flow.busy(),
        }
    }

    /// Whether the screen showing holds changes that haven't been saved.
    fn screen_unsaved(&self) -> bool {
        match &self.screen {
            Screen::Edit(flow) => flow.is_dirty(),
            Screen::Formations(flow) => flow.is_dirty(),
            Screen::Tabs(flow) => flow.is_dirty(),
            Screen::Files | Screen::Copy(_) | Screen::Restore(_) => false,
        }
    }

    /// The user asked to leave the screen, with its button or with Esc or
    /// the mouse's back button. Changes not saved are never lost without
    /// asking: the user is asked first.
    pub fn request_close(&mut self) {
        if self.screen_busy() {
            // Nothing leaves while a save or copy runs; asking would only
            // offer a choice that can't be acted on.
            return;
        }
        if self.screen_unsaved() {
            self.leave_prompt = Some(Leaving::Screen);
        } else {
            self.close_screen();
        }
    }

    /// Esc or the mouse's back button, on any screen but the file list.
    /// While the leave question is showing it means "stay"; on the copy
    /// screen's review it goes back to choosing files, as its "Back" button
    /// does; while files are being checked or copied it does nothing;
    /// otherwise it leaves the screen as the screen's own button would.
    pub fn go_back(&mut self) {
        if self.leave_prompt.take().is_some() {
            return;
        }
        if self.busy_notice.take().is_some() {
            return;
        }
        match &mut self.screen {
            Screen::Files => {}
            Screen::Copy(flow) => match flow.stage {
                Stage::Reviewing(_) => flow.back_to_picking(),
                Stage::Planning | Stage::Copying => {}
                Stage::Picking | Stage::Done(_) => self.request_close(),
            },
            _ => self.request_close(),
        }
    }

    /// The user chose to leave and lose the changes not saved.
    pub fn confirm_leave(&mut self) {
        match self.leave_prompt.take() {
            Some(Leaving::Screen) => self.close_screen(),
            Some(Leaving::Window) => self.close_window = true,
            None => {}
        }
    }

    /// The user chose to stay on the screen.
    pub fn cancel_leave(&mut self) {
        self.leave_prompt = None;
    }

    /// The window's close button (or Alt+F4) was pressed. Returns whether
    /// the window may close now. It may not while files are being written:
    /// the user is told to wait, since stopping a copy partway would leave
    /// some files copied onto and others not. With changes not saved the
    /// user is asked first, as when leaving a screen. A copy screen's
    /// choice of what to copy is remembered, as on leaving the screen.
    pub fn window_close_requested(&mut self) -> bool {
        if self.close_window {
            return true;
        }
        if let Some(work) = self.work_under_way() {
            self.leave_prompt = None;
            self.busy_notice = Some(format!(
                "Clone Bay is still {work}. Wait for it to finish, then close it."
            ));
            return false;
        }
        if self.screen_unsaved() {
            self.leave_prompt = Some(Leaving::Window);
            return false;
        }
        if let Screen::Copy(flow) = &mut self.screen {
            flow.remember_choice();
        }
        true
    }

    /// What the screen is busy doing, in words, if it is.
    fn work_under_way(&self) -> Option<&'static str> {
        if !self.screen_busy() {
            return None;
        }
        Some(match &self.screen {
            Screen::Copy(flow) if matches!(flow.stage, Stage::Planning) => "checking the files",
            Screen::Copy(_) => "copying",
            Screen::Restore(_) => "restoring the file",
            _ => "saving",
        })
    }

    /// The user has read why the window didn't close.
    pub fn dismiss_busy_notice(&mut self) {
        self.busy_notice = None;
    }

    /// Goes back to the file list. Does nothing while the screen is saving
    /// or copying. If files were changed the list is refreshed, which reads
    /// only the files that changed.
    pub fn close_screen(&mut self) {
        let (busy, changed) = match &self.screen {
            Screen::Files => return,
            Screen::Copy(flow) => (flow.busy(), flow.changed_files()),
            Screen::Edit(flow) => (flow.busy(), flow.changed_file()),
            Screen::Formations(flow) => (flow.busy(), flow.changed_file()),
            Screen::Tabs(flow) => (flow.busy(), flow.changed_file()),
            Screen::Restore(flow) => (flow.busy(), flow.changed_file()),
        };
        if busy {
            return;
        }
        self.leave_prompt = None;
        let choice_problem = match &mut self.screen {
            Screen::Copy(flow) => {
                flow.remember_choice();
                flow.choice_problem.take()
            }
            _ => None,
        };
        self.screen = Screen::Files;
        if changed {
            // Sizes, times and contents may all be different now.
            self.rescan();
        }
        if let Some(problem) = choice_problem {
            self.problem = Some(format!("{}.", capitalise(&problem)));
        }
    }

    #[cfg(test)]
    pub fn copy(&self) -> Option<&CopyFlow> {
        match &self.screen {
            Screen::Copy(flow) => Some(flow),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn copy_mut(&mut self) -> Option<&mut CopyFlow> {
        match &mut self.screen {
            Screen::Copy(flow) => Some(flow),
            _ => None,
        }
    }

    pub fn edit(&self) -> Option<&EditFlow> {
        match &self.screen {
            Screen::Edit(flow) => Some(flow),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn edit_mut(&mut self) -> Option<&mut EditFlow> {
        match &mut self.screen {
            Screen::Edit(flow) => Some(flow),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn formations(&self) -> Option<&FormationFlow> {
        match &self.screen {
            Screen::Formations(flow) => Some(flow),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn formations_mut(&mut self) -> Option<&mut FormationFlow> {
        match &mut self.screen {
            Screen::Formations(flow) => Some(flow),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn restore(&self) -> Option<&RestoreFlow> {
        match &self.screen {
            Screen::Restore(flow) => Some(flow),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn tabs(&self) -> Option<&TabFlow> {
        match &self.screen {
            Screen::Tabs(flow) => Some(flow),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn tabs_mut(&mut self) -> Option<&mut TabFlow> {
        match &mut self.screen {
            Screen::Tabs(flow) => Some(flow),
            _ => None,
        }
    }

    /// Looks up the names of the item types on the edit screen that aren't
    /// known yet, in the background.
    fn start_type_lookup(&mut self) {
        if self.types_task.is_some() {
            return;
        }
        let Some(flow) = self.edit() else {
            return;
        };
        let ids: Vec<u64> = flow
            .type_ids()
            .into_iter()
            .filter(|&id| self.names.item_type_needs_lookup(id))
            // Ships and structures built into the app need no lookup.
            .filter(|&id| clone_bay_core::bundled::bundled_type_name(id).is_none())
            .collect();
        if ids.is_empty() {
            return;
        }
        let source = Arc::clone(&self.services.type_name_source);
        let wake = Arc::clone(&self.wake);
        let (sender, receiver) = mpsc::channel();
        self.types_task = Some(receiver);
        thread::spawn(move || {
            let lookup = look_up(source.as_ref(), &ids);
            // If the window has gone there is nobody to tell.
            let _ = sender.send(lookup);
            wake();
        });
    }

    fn receive_types(&mut self) {
        let Some(lookup) = self
            .types_task
            .as_ref()
            .and_then(|task| task.try_recv().ok())
        else {
            return;
        };
        self.types_task = None;
        let learned = !lookup.found.is_empty();
        for (id, name) in lookup.found {
            self.names.set_item_type(id, name);
        }
        if let Some(error) = lookup.stopped {
            self.names_problem = Some(capitalise(&error.to_string()));
        }
        if learned {
            self.save_names();
        }
    }

    /// Applies results that background threads have finished.
    pub fn receive(&mut self) {
        match &mut self.screen {
            Screen::Files => {}
            Screen::Copy(flow) => flow.poll(),
            Screen::Edit(flow) => flow.poll(),
            Screen::Formations(flow) => flow.poll(),
            Screen::Tabs(flow) => flow.poll(),
            Screen::Restore(flow) => flow.poll(),
        }
        self.receive_names();
        self.receive_types();
        for inspected in self.loader.drain(self.batch) {
            if let Some(row) = self.rows.get_mut(inspected.index) {
                row.status = match inspected.result {
                    Ok(summary) => Status::Readable(summary),
                    Err(error) => Status::Unreadable(error),
                };
            }
        }
    }

    pub fn selected_row(&self) -> Option<&Row> {
        let selected = self.selected_file.as_ref()?;
        self.rows.iter().find(|row| &row.entry.path == selected)
    }
}

pub fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A sample settings folder for tests in this crate.
#[cfg(test)]
pub mod test_support {
    use std::fs;
    use std::sync::Arc;

    use super::Wake;

    pub fn quiet() -> Wake {
        Arc::new(|| {})
    }

    /// A state that touches nothing real: no name service, nothing saved.
    pub fn state(folder: Option<std::path::PathBuf>) -> super::State {
        super::State::with_services(folder, quiet(), crate::services::Services::for_tests())
    }

    /// A small valid settings file: a version 1 stream holding `{"ui": {}}`.
    pub const SMALL_SETTINGS: [u8; 10] = [0x7D, 1, 22, 1, 22, 0, 19, 2, b'u', b'i'];

    /// One install with two profiles. `Default` holds a good account file
    /// (11), a damaged character file (22) and an unrelated file; `alt`
    /// holds one good character file (33).
    pub fn eve_root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let live = root.path().join("c_eve_tq_tranquility");
        for profile in ["settings_Default", "settings_alt"] {
            fs::create_dir_all(live.join(profile)).unwrap();
        }
        let default = live.join("settings_Default");
        fs::write(default.join("core_user_11.dat"), SMALL_SETTINGS).unwrap();
        fs::write(default.join("core_char_22.dat"), b"damaged").unwrap();
        fs::write(default.join("prefs.ini"), b"").unwrap();
        let alt = live.join("settings_alt");
        fs::write(alt.join("core_char_33.dat"), SMALL_SETTINGS).unwrap();
        root
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::test_support::{eve_root, state, SMALL_SETTINGS};
    use super::*;

    fn settle(state: &mut State) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.rows.iter().any(|row| row.status == Status::Pending) {
            assert!(Instant::now() < deadline, "timed out waiting for files");
            state.receive();
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_restore_needs_a_selected_file_that_has_been_checked() {
        let root = eve_root();
        let mut state = state(Some(root.path().to_path_buf()));
        assert!(!state.can_restore_selected());
        state.selected_file = Some(state.rows[0].entry.path.clone());
        assert!(!state.can_restore_selected());
        settle(&mut state);
        assert!(state.can_restore_selected());
    }

    fn offered(state: &State) -> Vec<(&'static str, bool)> {
        state
            .file_choices()
            .iter()
            .map(|choice| (choice.label, choice.enabled))
            .collect()
    }

    #[test]
    fn what_can_be_done_depends_on_the_file() {
        let root = eve_root();
        let mut state = state(Some(root.path().to_path_buf()));
        // Nothing selected, nothing offered.
        assert!(state.file_choices().is_empty());
        // Not checked yet: nothing can be done, and each says why.
        state.selected_file = Some(state.rows[1].entry.path.clone());
        assert!(offered(&state).iter().all(|(_, enabled)| !enabled));
        assert!(state
            .file_choices()
            .iter()
            .all(|choice| choice.why_not == "Still checking this file."));
        settle(&mut state);
        assert!(state
            .file_choices()
            .iter()
            .all(|choice| choice.why_not.is_empty()));

        // A good account: everything, the copy first and filled.
        assert_eq!(
            offered(&state),
            [
                ("Copy to other files", true),
                ("Edit settings", true),
                ("Order overview tabs", true),
                ("Edit probe formations", true),
                ("Restore from a backup", true),
            ]
        );
        let primary: Vec<_> = state
            .file_choices()
            .iter()
            .filter(|choice| choice.primary)
            .map(|choice| choice.action)
            .collect();
        assert_eq!(primary, [RowAction::Copy]);
        assert!(state
            .file_choices()
            .iter()
            .all(|choice| !choice.hover.is_empty()));

        // A damaged character: chat tabs, no formations, only a restore.
        state.selected_file = Some(state.rows[0].entry.path.clone());
        assert_eq!(
            offered(&state),
            [
                ("Copy to other files", false),
                ("Edit settings", false),
                ("Order chat tabs", false),
                ("Restore from a backup", true),
            ]
        );
        let reasons: Vec<_> = state
            .file_choices()
            .iter()
            .map(|choice| choice.why_not)
            .collect();
        let cant = "This file can't be used. Restoring it from a backup may help.";
        assert_eq!(reasons, [cant, cant, cant, ""]);
    }

    fn ids(state: &State) -> Vec<u64> {
        state.rows.iter().map(|row| row.entry.id).collect()
    }

    #[test]
    fn opens_the_default_profile_and_checks_every_file() {
        let root = eve_root();
        let mut state = state(Some(root.path().to_path_buf()));
        assert_eq!(state.problem, None);
        assert_eq!(state.profiles.len(), 2);
        let selected = &state.profiles[state.selected_profile.unwrap()];
        assert_eq!(selected.name, "Default");
        assert_eq!(ids(&state), [22, 11]);
        assert!(state.rows.iter().all(|row| row.status == Status::Pending));

        settle(&mut state);
        assert!(matches!(state.rows[0].status, Status::Unreadable(_)));
        assert!(matches!(state.rows[1].status, Status::Readable(_)));
    }

    #[test]
    fn switching_profile_replaces_the_list() {
        let root = eve_root();
        let mut state = state(Some(root.path().to_path_buf()));
        let alt = state.profiles.iter().position(|p| p.name == "alt").unwrap();
        state.select_profile(alt);
        assert_eq!(state.selected_profile, Some(alt));
        assert_eq!(ids(&state), [33]);
        settle(&mut state);
        assert!(matches!(state.rows[0].status, Status::Readable(_)));
    }

    #[test]
    fn selecting_a_profile_that_does_not_exist_changes_nothing() {
        let root = eve_root();
        let mut state = state(Some(root.path().to_path_buf()));
        let before = state.selected_profile;
        state.select_profile(99);
        assert_eq!(state.selected_profile, before);
        assert_eq!(ids(&state), [22, 11]);
    }

    #[test]
    fn rescan_picks_up_new_files_and_keeps_the_selection() {
        let root = eve_root();
        let mut state = state(Some(root.path().to_path_buf()));
        let chosen = state.rows[1].entry.path.clone();
        state.selected_file = Some(chosen.clone());
        let folder = chosen.parent().unwrap();
        fs::write(folder.join("core_user_44.dat"), SMALL_SETTINGS).unwrap();

        state.rescan();
        assert_eq!(ids(&state), [22, 11, 44]);
        assert_eq!(state.selected_file, Some(chosen.clone()));
        assert_eq!(state.selected_row().unwrap().entry.id, 11);

        fs::remove_file(&chosen).unwrap();
        state.rescan();
        assert_eq!(state.selected_file, None);
        assert!(state.selected_row().is_none());
    }

    #[test]
    fn a_rescan_reads_only_the_files_that_changed() {
        let root = eve_root();
        let mut state = state(Some(root.path().to_path_buf()));
        settle(&mut state);
        let before = state.rows.clone();

        // Nothing changed: every file keeps what was found, with no reading.
        state.rescan();
        assert_eq!(state.rows, before);

        // One file changed: only it is read again.
        let account = state.rows[1].entry.path.clone();
        fs::write(&account, b"now damaged, and a different size").unwrap();
        state.rescan();
        assert_eq!(state.rows[0], before[0]);
        assert_eq!(state.rows[1].status, Status::Pending);
        settle(&mut state);
        assert!(matches!(state.rows[1].status, Status::Unreadable(_)));
    }

    #[test]
    fn files_still_waiting_when_the_list_is_rescanned_are_still_checked() {
        let root = eve_root();
        let mut state = state(Some(root.path().to_path_buf()));
        // Straight away, before the first check has reported anything.
        state.rescan();
        settle(&mut state);
        assert!(matches!(state.rows[0].status, Status::Unreadable(_)));
        assert!(matches!(state.rows[1].status, Status::Readable(_)));
    }

    #[test]
    fn a_folder_without_settings_is_explained() {
        let empty = tempfile::tempdir().unwrap();
        let state = state(Some(empty.path().to_path_buf()));
        assert!(state.profiles.is_empty());
        assert!(state.rows.is_empty());
        let problem = state.problem.unwrap();
        assert!(
            problem.starts_with("No EVE settings were found in "),
            "{problem}"
        );
    }

    #[test]
    fn a_missing_folder_is_explained() {
        let empty = tempfile::tempdir().unwrap();
        let state = state(Some(empty.path().join("nope")));
        let problem = state.problem.unwrap();
        assert!(problem.starts_with("Can't read the folder "), "{problem}");
    }

    #[test]
    fn starting_with_no_folder_asks_for_one() {
        let state = state(None);
        assert!(state.problem.unwrap().contains("Enter the folder below"));
        assert!(state.profiles.is_empty());
    }

    #[test]
    fn opening_another_folder_clears_the_old_one() {
        let root = eve_root();
        let mut state = state(Some(root.path().to_path_buf()));
        let empty = tempfile::tempdir().unwrap();
        state.open_folder(empty.path());
        assert!(state.profiles.is_empty());
        assert_eq!(state.selected_profile, None);
        assert!(state.rows.is_empty());
        assert_eq!(state.folder_input, empty.path().display().to_string());
    }

    #[test]
    fn capitalise_handles_empty_and_non_ascii_text() {
        assert_eq!(capitalise(""), "");
        assert_eq!(capitalise("can't"), "Can't");
        assert_eq!(capitalise("école"), "École");
    }
}
