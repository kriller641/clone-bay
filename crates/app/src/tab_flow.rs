//! Changing the order of tabs in one file: overview tabs for an account,
//! chat tabs for a character. No drawing happens here.

use clone_bay_core::discovery::SettingsFileEntry;
use clone_bay_core::edit::timestamp_now;
use clone_bay_core::safe_write::Fingerprint;
use clone_bay_core::tab_order::{open_file, save, TabLayout, TabOrderError};

use crate::saving::Saving;
use crate::services::Services;
use crate::state::Wake;

pub struct TabFlow {
    pub entry: SettingsFileEntry,
    /// The character's name or the account's label, when there is one.
    pub name: Option<String>,
    /// The order as the user has arranged it. `None` when the file couldn't
    /// be read.
    pub layout: Option<TabLayout>,
    /// The order the file held when it was opened or last saved.
    original: Option<TabLayout>,
    /// What the file held then. A save is refused once it holds anything
    /// else.
    opened_as: Option<Fingerprint>,
    /// Why the file couldn't be opened, or the last save was refused.
    pub problem: Option<String>,
    pub saving: Saving<TabOrderError>,
}

impl TabFlow {
    /// Opens `entry` to reorder its tabs. If it can't be read there is no
    /// layout and `problem` says why.
    pub fn new(
        entry: &SettingsFileEntry,
        name: Option<String>,
        services: Services,
        wake: Wake,
    ) -> Self {
        let mut flow = Self {
            entry: entry.clone(),
            name,
            layout: None,
            original: None,
            opened_as: None,
            problem: None,
            saving: Saving::new(services, wake),
        };
        flow.reload();
        flow
    }

    /// Reads the file again, dropping any unsaved changes.
    fn reload(&mut self) {
        match open_file(&self.entry.path) {
            Ok((layout, opened_as)) => {
                self.original = Some(layout.clone());
                self.layout = Some(layout);
                self.opened_as = Some(opened_as);
            }
            Err(error) => {
                self.original = None;
                self.layout = None;
                self.opened_as = None;
                self.problem = Some(error.to_string());
            }
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.layout.is_some() && self.layout != self.original
    }

    /// Whether a save is running, during which the screen can't be left.
    pub fn busy(&self) -> bool {
        self.saving.busy()
    }

    /// Whether "Save changes" may be pressed.
    pub fn can_save(&self) -> bool {
        self.is_dirty() && self.saving.ready()
    }

    /// Whether a save has changed the file since the screen was opened.
    pub fn changed_file(&self) -> bool {
        self.saving.changed_file()
    }

    /// Puts the order back to what the file holds.
    pub fn discard(&mut self) {
        if !self.busy() {
            self.problem = None;
            self.reload();
        }
    }

    /// The layout, when it may be changed: not while a save is running.
    fn editable(&mut self) -> Option<&mut TabLayout> {
        if self.busy() {
            return None;
        }
        self.layout.as_mut()
    }

    pub fn move_up(&mut self, group: usize, tab: usize) {
        if let Some(layout) = self.editable() {
            layout.move_up(group, tab);
        }
    }

    pub fn move_down(&mut self, group: usize, tab: usize) {
        if let Some(layout) = self.editable() {
            layout.move_down(group, tab);
        }
    }

    pub fn move_to_group(&mut self, group: usize, tab: usize, to: usize) {
        if let Some(layout) = self.editable() {
            layout.move_to_group(group, tab, to);
        }
    }

    pub fn open_on_first_tab(&mut self) {
        if let Some(layout) = self.editable() {
            layout.open_on_first_tab();
        }
    }

    /// Saves the order in the background.
    pub fn save(&mut self) {
        if !self.can_save() {
            return;
        }
        let (Some(layout), Some(opened_as)) = (self.layout.clone(), self.opened_as) else {
            return;
        };
        let path = self.entry.path.clone();
        self.problem = None;
        self.saving.start(move |context| {
            save(
                &path,
                opened_as,
                &layout,
                context.eve,
                context.user_confirmed_eve_closed,
                context.backups,
                &timestamp_now(),
            )
        });
    }

    /// Applies the result of a save, if it has finished.
    pub fn poll(&mut self) {
        match self.saving.poll() {
            // Show what the file holds now.
            Some(Ok(())) => self.reload(),
            Some(Err(error)) => self.problem = Some(error.to_string()),
            None => {}
        }
    }
}
