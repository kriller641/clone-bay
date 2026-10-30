//! Everything that differs between operating systems lives here, so a port
//! to another OS only has to touch this module.

use std::path::{Path, PathBuf};

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
}
