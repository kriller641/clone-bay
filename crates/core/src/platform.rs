//! Everything that differs between operating systems lives here, so a port
//! to another OS only has to touch this module.

use std::path::{Path, PathBuf};

use thiserror::Error;

/// The folder that holds one sub-folder per EVE install, or `None` when it
/// can't be worked out on this system.
///
/// Only Windows is supported for now.
pub fn default_eve_root() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .filter(|value| !value.is_empty())
            .map(|value| eve_root_under(Path::new(&value)))
    } else {
        None
    }
}

/// The folder this app keeps its own data in (remembered names and
/// labels), or `None` when it can't be worked out on this system.
pub fn app_data_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA")
            .filter(|value| !value.is_empty())
            .map(|value| app_data_dir_under(Path::new(&value)))
    } else {
        None
    }
}

/// This app's data folder beneath the user's roaming app data folder.
pub fn app_data_dir_under(app_data: &Path) -> PathBuf {
    app_data.join("clone-bay")
}

/// Where EVE keeps its settings beneath the user's local app data folder.
pub fn eve_root_under(local_app_data: &Path) -> PathBuf {
    local_app_data.join("CCP").join("EVE")
}

/// The file in the app's data folder that the open window holds locked.
const WINDOW_LOCK_FILE: &str = "window.lock";

/// Whether this is the only window of the app.
#[derive(Debug)]
pub enum SingleWindow {
    /// It is. The lock lasts as long as this file is kept open, and the
    /// system releases it when the app ends, however it ends.
    Claimed(std::fs::File),
    /// Another window of the app is open.
    AlreadyOpen,
    /// It couldn't be checked, for this reason.
    Unchecked(String),
}

/// Takes the lock that only one window of the app may hold, in `folder`
/// (the app's data folder), creating the folder if need be.
pub fn claim_single_window(folder: &Path) -> SingleWindow {
    let opened = std::fs::create_dir_all(folder).and_then(|()| {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(folder.join(WINDOW_LOCK_FILE))
    });
    let file = match opened {
        Ok(file) => file,
        Err(error) => return SingleWindow::Unchecked(error.to_string()),
    };
    match file.try_lock() {
        Ok(()) => SingleWindow::Claimed(file),
        Err(std::fs::TryLockError::WouldBlock) => SingleWindow::AlreadyOpen,
        Err(std::fs::TryLockError::Error(error)) => SingleWindow::Unchecked(error.to_string()),
    }
}

/// Whether the EVE client is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EveStatus {
    Running,
    NotRunning,
    /// It couldn't be checked. Treat as "might be running".
    Unknown,
}

/// The EVE client's process name. The launcher is a different process and is
/// harmless.
const CLIENT_PROCESS: &str = "exefile";

/// Checks whether an EVE client is running.
///
/// The client keeps settings in memory and writes them when it closes, so
/// anything written while it runs would be overwritten.
pub fn eve_status() -> EveStatus {
    match process_list() {
        Some(list) => status_from_process_list(&list),
        None => EveStatus::Unknown,
    }
}

/// Reads the answer out of a process listing. An empty listing means the
/// listing itself failed: there is always at least one process.
pub fn status_from_process_list(list: &str) -> EveStatus {
    if list.trim().is_empty() {
        EveStatus::Unknown
    } else if list.to_ascii_lowercase().contains(CLIENT_PROCESS) {
        EveStatus::Running
    } else {
        EveStatus::NotRunning
    }
}

/// The names of all running processes, or `None` if they can't be listed.
#[cfg(windows)]
fn process_list() -> Option<String> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    // Stops a console window flashing up behind the app.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // Use the system's own tasklist, not whatever is first on PATH.
    let tasklist = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32").join("tasklist.exe"))
        .filter(|path| path.is_file())?;
    let output = Command::new(tasklist)
        .args(["/FO", "CSV", "/NH"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(not(windows))]
fn process_list() -> Option<String> {
    use std::process::{Command, Stdio};

    let output = Command::new("ps")
        .args(["-A", "-o", "comm="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Why a folder couldn't be shown.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ShowFolderError {
    #[error("the folder {0} isn't there any more")]
    Missing(PathBuf),
    #[error("the file manager couldn't be opened ({0})")]
    CannotStart(String),
}

/// Opens `folder` in the system's file manager.
pub fn show_folder(folder: &Path) -> Result<(), ShowFolderError> {
    if !folder.is_dir() {
        return Err(ShowFolderError::Missing(folder.to_path_buf()));
    }
    file_manager(folder)
        .spawn()
        .map(drop)
        .map_err(|error| ShowFolderError::CannotStart(error.to_string()))
}

#[cfg(windows)]
fn file_manager(folder: &Path) -> std::process::Command {
    use std::os::windows::process::CommandExt;

    // The system's own Explorer, not whatever is first on PATH. Explorer
    // reads a comma as a separator unless the path is quoted, and a Windows
    // path can't hold a quote, so quoting it is safe.
    let explorer = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("explorer.exe"))
        .unwrap_or_else(|| PathBuf::from("explorer.exe"));
    let mut command = std::process::Command::new(explorer);
    command.raw_arg(format!("\"{}\"", folder.display()));
    command
}

#[cfg(target_os = "macos")]
fn file_manager(folder: &Path) -> std::process::Command {
    let mut command = std::process::Command::new("open");
    command.arg(folder);
    command
}

#[cfg(not(any(windows, target_os = "macos")))]
fn file_manager(folder: &Path) -> std::process::Command {
    let mut command = std::process::Command::new("xdg-open");
    command.arg(folder);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_data_is_its_own_folder_under_roaming_app_data() {
        assert_eq!(
            app_data_dir_under(Path::new("base")),
            Path::new("base").join("clone-bay")
        );
    }

    #[cfg(windows)]
    #[test]
    fn default_app_data_follows_the_environment() {
        let expected = app_data_dir_under(Path::new(&std::env::var_os("APPDATA").unwrap()));
        assert_eq!(app_data_dir(), Some(expected));
    }

    #[test]
    fn eve_root_is_ccp_eve_under_local_app_data() {
        let root = eve_root_under(Path::new("base"));
        assert_eq!(root, Path::new("base").join("CCP").join("EVE"));
    }

    #[cfg(windows)]
    #[test]
    fn default_root_follows_local_app_data() {
        // LOCALAPPDATA is always set for a Windows user session.
        let expected = eve_root_under(Path::new(&std::env::var_os("LOCALAPPDATA").unwrap()));
        assert_eq!(default_eve_root(), Some(expected));
    }

    #[test]
    fn only_one_window_holds_the_lock_until_it_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        // The folder is made if it isn't there yet.
        let folder = dir.path().join("clone-bay");
        let first = claim_single_window(&folder);
        assert!(matches!(first, SingleWindow::Claimed(_)), "{first:?}");
        let second = claim_single_window(&folder);
        assert!(matches!(second, SingleWindow::AlreadyOpen), "{second:?}");

        drop(first);
        let again = claim_single_window(&folder);
        assert!(matches!(again, SingleWindow::Claimed(_)), "{again:?}");
    }

    #[test]
    fn a_lock_that_cannot_be_made_is_reported_not_treated_as_open() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the folder should be.
        let not_a_folder = dir.path().join("clone-bay");
        std::fs::write(&not_a_folder, b"x").unwrap();
        let result = claim_single_window(&not_a_folder);
        let SingleWindow::Unchecked(reason) = &result else {
            panic!("{result:?}");
        };
        assert!(reason.contains("os error"), "{reason}");
    }

    #[test]
    fn the_client_is_found_in_a_process_listing_whatever_its_case() {
        let running = "\"System Idle Process\",\"0\"\n\"ExeFile.exe\",\"4120\",\"Console\"\n";
        assert_eq!(status_from_process_list(running), EveStatus::Running);
        let not_running = "\"System Idle Process\",\"0\"\n\"evelauncher.exe\",\"77\"\n";
        assert_eq!(status_from_process_list(not_running), EveStatus::NotRunning);
    }

    #[test]
    fn an_empty_listing_means_the_check_failed() {
        assert_eq!(status_from_process_list(""), EveStatus::Unknown);
        assert_eq!(status_from_process_list(" \r\n"), EveStatus::Unknown);
    }

    #[cfg(windows)]
    #[test]
    fn the_real_process_list_can_be_read() {
        // This test process is itself in the list.
        let list = process_list().expect("tasklist should work on Windows");
        assert!(list.to_ascii_lowercase().contains(".exe"), "{list}");
        assert_ne!(eve_status(), EveStatus::Unknown);
    }

    #[test]
    fn a_folder_that_has_gone_is_reported_without_starting_anything() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("settings_Gone");
        assert_eq!(show_folder(&gone), Err(ShowFolderError::Missing(gone)));
        // A file isn't a folder either.
        let file = dir.path().join("core_user_1.dat");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(show_folder(&file), Err(ShowFolderError::Missing(file)));
    }

    #[cfg(windows)]
    #[test]
    fn explorer_gets_the_folder_quoted() {
        let command = file_manager(Path::new(r"C:\EVE,settings\settings_Default"));
        assert!(command
            .get_program()
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with("explorer.exe"));
        // A raw argument isn't listed by `get_args`, so check the debug form.
        assert!(
            format!("{command:?}").contains(r#""C:\EVE,settings\settings_Default""#),
            "{command:?}"
        );
    }
}
