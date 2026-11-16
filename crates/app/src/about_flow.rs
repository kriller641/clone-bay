//! The About page: what the app is, where the profile's backups are kept,
//! and the credits. No drawing happens here, and nothing on it changes a
//! file.

use std::path::PathBuf;

use clone_bay_core::discovery::Profile;
use clone_bay_core::restore::{backup_runs, backups_folder, BackupRuns};

use crate::list_model::counted;
use crate::services::Services;

/// The version that is running.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The project's page on GitHub, from `repository` in the root `Cargo.toml`.
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// Shown greyed out on "Open the backup folder" when there is nothing to
/// open.
pub const NO_BACKUPS: &str = "No backups yet: one is made the first time a file is changed.";

pub struct AboutFlow {
    /// The profile shown on the file list, by its name. `None` when no
    /// settings were found.
    pub profile_name: Option<String>,
    /// Where that profile's backups go.
    pub backups_folder: Option<PathBuf>,
    /// What that folder holds.
    pub backups: BackupRuns,
    /// Why the last button press didn't work.
    pub problem: Option<String>,
    services: Services,
}

impl AboutFlow {
    /// The page for `profile`, the one the file list shows. Lists its
    /// backup folders, which is quick: only their names are read.
    pub fn new(profile: Option<&Profile>, services: Services) -> Self {
        let backups = profile.map_or(
            BackupRuns {
                count: 0,
                newest: None,
            },
            |profile| backup_runs(&profile.path),
        );
        Self {
            profile_name: profile.map(|profile| profile.name.clone()),
            backups_folder: profile.map(|profile| backups_folder(&profile.path)),
            backups,
            problem: None,
            services,
        }
    }

    /// Whether there is a backup folder to open.
    pub fn has_backups(&self) -> bool {
        self.backups.count > 0
    }

    /// One line about the profile's backups.
    pub fn backups_line(&self) -> String {
        if !self.has_backups() {
            return NO_BACKUPS.to_owned();
        }
        let folders = counted(self.backups.count, "backup folder", "backup folders");
        match self.backups.newest {
            Some(newest) => format!(
                "{folders}, the newest from {}",
                newest.format("%-d %b %Y, %H:%M")
            ),
            None => folders,
        }
    }

    /// Opens the project's page on GitHub.
    pub fn open_github(&mut self) {
        self.problem = (self.services.open_link)(REPOSITORY)
            .err()
            .map(|error| format!("couldn't open the page: {error}"));
    }

    /// Opens the profile's backup folder in the system's file manager.
    pub fn open_backups(&mut self) {
        let Some(folder) = self.backups_folder.as_ref().filter(|_| self.has_backups()) else {
            return;
        };
        self.problem = (self.services.show_folder)(folder)
            .err()
            .map(|error| format!("couldn't open the folder: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// The page for a profile whose backup folder holds `folders`.
    fn flow_with(folders: &[&str]) -> (tempfile::TempDir, AboutFlow) {
        let dir = tempfile::tempdir().unwrap();
        for folder in folders {
            fs::create_dir_all(backups_folder(dir.path()).join(folder)).unwrap();
        }
        let profile = Profile {
            install: "c_eve_tq_tranquility".to_owned(),
            name: "Default".to_owned(),
            path: dir.path().to_path_buf(),
        };
        let flow = AboutFlow::new(Some(&profile), Services::for_tests());
        (dir, flow)
    }

    #[test]
    fn the_backups_line_says_how_many_and_when_the_newest_was_made() {
        assert_eq!(flow_with(&[]).1.backups_line(), NO_BACKUPS);
        let (_dir, flow) = flow_with(&["20261031-090000", "20261106-140209", "undated"]);
        assert_eq!(flow.profile_name.as_deref(), Some("Default"));
        assert!(flow.has_backups());
        assert_eq!(
            flow.backups_line(),
            "3 backup folders, the newest from 6 Nov 2026, 14:02"
        );
        assert_eq!(flow_with(&["undated"]).1.backups_line(), "1 backup folder");
    }

    #[test]
    fn with_no_profile_there_is_no_folder_to_open() {
        let mut flow = AboutFlow::new(None, Services::for_tests());
        assert_eq!(flow.profile_name, None);
        assert_eq!(flow.backups_folder, None);
        assert!(!flow.has_backups());
        // Pressing it anyway does nothing and says nothing.
        flow.open_backups();
        assert_eq!(flow.problem, None);
    }

    #[test]
    fn the_github_link_is_the_repository() {
        assert!(
            REPOSITORY.starts_with("https://github.com/"),
            "{REPOSITORY}"
        );
    }
}
