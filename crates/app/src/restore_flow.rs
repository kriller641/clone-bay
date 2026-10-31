//! Putting one file back the way one of its backups holds it. No drawing
//! happens here.

use clone_bay_core::discovery::SettingsFileEntry;
use clone_bay_core::inspect::read_settings_bytes;
use clone_bay_core::restore::{list_backups, restore, Backup, RestoreError};
use clone_bay_core::safe_write::Fingerprint;

use crate::list_model::format_size;
use crate::saving::Saving;
use crate::services::Services;
use crate::state::Wake;

pub struct RestoreFlow {
    pub entry: SettingsFileEntry,
    /// The character's name or the account's label, when there is one.
    pub name: Option<String>,
    /// The file's backups, newest first.
    pub backups: Vec<Backup>,
    /// The backup the user picked, by its place in `backups`.
    pub selected: Option<usize>,
    /// What the file held when the list was made. A restore is refused once
    /// it holds anything else. `None` when the file couldn't be read.
    opened_as: Option<Fingerprint>,
    /// Why the file can't be restored, or the last restore was refused.
    pub problem: Option<String>,
    pub saving: Saving<RestoreError>,
}

impl RestoreFlow {
    pub fn new(
        entry: &SettingsFileEntry,
        name: Option<String>,
        services: Services,
        wake: Wake,
    ) -> Self {
        let mut flow = Self {
            entry: entry.clone(),
            name,
            backups: Vec::new(),
            selected: None,
            opened_as: None,
            problem: None,
            saving: Saving::new(services, wake),
        };
        flow.reload();
        flow
    }

    /// Reads the file and its list of backups again.
    fn reload(&mut self) {
        self.selected = None;
        self.backups = list_backups(&self.entry.path);
        match read_settings_bytes(&self.entry.path) {
            Ok(bytes) => self.opened_as = Some(Fingerprint::of(&bytes)),
            Err(error) => {
                self.opened_as = None;
                self.problem = Some(format!("the file can't be read ({error})"));
            }
        }
    }

    /// Picks the backup at `index` in the list.
    pub fn select(&mut self, index: usize) {
        if !self.busy() && index < self.backups.len() {
            self.selected = Some(index);
        }
    }

    /// Whether a restore is running, during which the screen can't be left.
    pub fn busy(&self) -> bool {
        self.saving.busy()
    }

    /// Whether "Restore this backup" may be pressed.
    pub fn can_restore(&self) -> bool {
        self.selected.is_some() && self.opened_as.is_some() && self.saving.ready()
    }

    /// Whether a restore has changed the file since the screen was opened.
    pub fn changed_file(&self) -> bool {
        self.saving.changed_file()
    }

    /// Restores the chosen backup in the background.
    pub fn restore(&mut self) {
        if !self.can_restore() {
            return;
        }
        let (Some(backup), Some(opened_as)) = (
            self.selected
                .and_then(|index| self.backups.get(index))
                .cloned(),
            self.opened_as,
        ) else {
            return;
        };
        let path = self.entry.path.clone();
        self.problem = None;
        self.saving.start(move |context| {
            restore(
                &path,
                opened_as,
                &backup,
                context.eve,
                context.user_confirmed_eve_closed,
                context.backups,
            )
        });
    }

    /// Applies the result of a restore, if it has finished.
    pub fn poll(&mut self) {
        match self.saving.poll() {
            // The file, and the list, are different now.
            Some(Ok(())) => self.reload(),
            Some(Err(error)) => self.problem = Some(error.to_string()),
            None => {}
        }
    }
}

/// What a backup is called in the list: when it was taken and its size.
pub fn backup_label(backup: &Backup) -> String {
    let when = match backup.taken {
        Some(taken) => taken.format("%-d %b %Y, %H:%M:%S").to_string(),
        None => "Unknown time".to_owned(),
    };
    format!("{when}  ({})", format_size(backup.size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backup_is_labelled_with_its_time_and_size() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("core_user_1.dat");
        for folder in ["20261003-070405", "undated"] {
            let folder = dir.path().join("clone-bay-backups").join(folder);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("core_user_1.dat"), vec![0; 2048]).unwrap();
        }
        let backups = list_backups(&file);
        let labels: Vec<String> = backups.iter().map(backup_label).collect();
        let size = format_size(2048);
        assert_eq!(
            labels,
            [
                format!("3 Oct 2026, 07:04:05  ({size})"),
                format!("Unknown time  ({size})")
            ]
        );
    }
}
