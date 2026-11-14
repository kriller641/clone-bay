//! Putting a downloaded exe in place of the running one.
//!
//! A running exe can't be overwritten on Windows, but it can be renamed.
//! So the running exe is renamed aside, the new one takes its name, and the
//! file set aside is deleted the next time the app starts.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

/// Starts the exe only to prove it runs and is the version given next.
pub const SELF_TEST_FLAG: &str = "--self-test";
/// Says the exe was started by the version given next, which it replaced.
pub const AFTER_UPDATE_FLAG: &str = "--after-update";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InstallError {
    #[error("the new version couldn't be started to check it ({0})")]
    SelfTestStart(String),
    #[error("the new version didn't run properly when it was checked")]
    SelfTestFailed,
    #[error("the app's file couldn't be replaced ({0})")]
    Replace(String),
    #[error("the new version couldn't be started ({0})")]
    Start(String),
}

/// Where a downloaded exe waits, beside the running one. It ends in
/// `.exe` so it can be run to check it.
pub fn staged_path(exe: &Path) -> PathBuf {
    exe.with_extension("new.exe")
}

/// Where the replaced exe is kept until the new one has started.
pub fn old_path(exe: &Path) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

/// Puts `staged` in place of `exe`, keeping the old exe at [`old_path`].
/// If the new one can't be put in place the old one is put back.
pub fn swap_in(exe: &Path, staged: &Path) -> Result<(), InstallError> {
    let replace = |error: std::io::Error| InstallError::Replace(error.to_string());
    let old = old_path(exe);
    fs::rename(exe, &old).map_err(replace)?;
    if let Err(error) = fs::rename(staged, exe) {
        let _ = fs::rename(&old, exe);
        return Err(replace(error));
    }
    Ok(())
}

/// Puts the old exe back after a swap, replacing the new one.
pub fn undo_swap(exe: &Path) -> Result<(), InstallError> {
    fs::rename(old_path(exe), exe).map_err(|error| InstallError::Replace(error.to_string()))
}

/// Deletes what an update left beside `exe`: the old exe and a download
/// that was never used. The old exe can't be deleted while the version it
/// belongs to is still closing, so this tries up to `tries` times, `pause`
/// apart. What can't be deleted is left for next time.
pub fn clean_leftovers(exe: &Path, tries: u32, pause: Duration) {
    let leftovers = [old_path(exe), staged_path(exe)];
    for attempt in 0..tries {
        let mut remaining = false;
        for path in &leftovers {
            if path.exists() && fs::remove_file(path).is_err() {
                remaining = true;
            }
        }
        if !remaining {
            return;
        }
        if attempt + 1 < tries {
            thread::sleep(pause);
        }
    }
}

/// Runs the downloaded exe to prove it starts and is `version`.
pub fn self_test(staged: &Path, version: &str, limit: Duration) -> Result<(), InstallError> {
    let mut command = Command::new(staged);
    command.arg(SELF_TEST_FLAG).arg(version);
    run_check(command, limit)
}

fn run_check(mut command: Command, limit: Duration) -> Result<(), InstallError> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| InstallError::SelfTestStart(error.to_string()))?;
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => return Err(InstallError::SelfTestFailed),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(InstallError::SelfTestFailed);
            }
        }
    }
}

/// Starts the new exe, telling it which version it replaced and passing on
/// the folder the app was opened with.
pub fn start_updated(
    exe: &Path,
    old_version: &str,
    folder: Option<&OsStr>,
) -> Result<(), InstallError> {
    let mut command = Command::new(exe);
    command.arg(AFTER_UPDATE_FLAG).arg(old_version);
    if let Some(folder) = folder {
        command.arg(folder);
    }
    command
        .spawn()
        .map(drop)
        .map_err(|error| InstallError::Start(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(windows)]
    use std::process::Command;
    use std::time::Duration;

    use super::*;

    const NO_WAIT: Duration = Duration::from_millis(0);

    #[test]
    fn the_staged_and_old_files_sit_beside_the_exe() {
        let exe = Path::new("C:/apps/clone-bay-v0.1.4-windows-x64.exe");
        assert_eq!(
            staged_path(exe),
            Path::new("C:/apps/clone-bay-v0.1.4-windows-x64.new.exe")
        );
        assert_eq!(
            old_path(exe),
            Path::new("C:/apps/clone-bay-v0.1.4-windows-x64.exe.old")
        );
    }

    /// A folder holding an "exe" and a staged new one.
    fn installed() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("clone-bay.exe");
        let staged = staged_path(&exe);
        fs::write(&exe, b"old").unwrap();
        fs::write(&staged, b"new").unwrap();
        (dir, exe, staged)
    }

    #[test]
    fn swapping_puts_the_new_exe_in_place_and_keeps_the_old_one_aside() {
        let (_dir, exe, staged) = installed();
        swap_in(&exe, &staged).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new");
        assert_eq!(fs::read(old_path(&exe)).unwrap(), b"old");
        assert!(!staged.exists());
    }

    #[test]
    fn swapping_replaces_an_old_file_left_from_last_time() {
        let (_dir, exe, staged) = installed();
        fs::write(old_path(&exe), b"older").unwrap();
        swap_in(&exe, &staged).unwrap();
        assert_eq!(fs::read(old_path(&exe)).unwrap(), b"old");
    }

    #[test]
    fn a_swap_with_nothing_staged_leaves_the_exe_as_it_was() {
        let (_dir, exe, staged) = installed();
        fs::remove_file(&staged).unwrap();
        assert!(matches!(
            swap_in(&exe, &staged),
            Err(InstallError::Replace(_))
        ));
        assert_eq!(fs::read(&exe).unwrap(), b"old");
        assert!(!old_path(&exe).exists());
    }

    #[test]
    fn undoing_a_swap_puts_the_old_exe_back() {
        let (_dir, exe, staged) = installed();
        swap_in(&exe, &staged).unwrap();
        undo_swap(&exe).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"old");
        assert!(!old_path(&exe).exists());
    }

    #[test]
    fn cleaning_up_removes_both_leftovers_and_nothing_else() {
        let (_dir, exe, staged) = installed();
        fs::write(old_path(&exe), b"older").unwrap();
        clean_leftovers(&exe, 1, NO_WAIT);
        assert!(!staged.exists());
        assert!(!old_path(&exe).exists());
        assert_eq!(fs::read(&exe).unwrap(), b"old");
        // With nothing to clean it does nothing.
        clean_leftovers(&exe, 3, NO_WAIT);
    }

    #[cfg(windows)]
    fn cmd(script: &str) -> Command {
        let mut command = Command::new("cmd");
        command.args(["/C", script]);
        command
    }

    #[cfg(windows)]
    #[test]
    fn a_check_passes_only_when_the_program_exits_cleanly() {
        let limit = Duration::from_secs(20);
        assert_eq!(run_check(cmd("exit 0"), limit), Ok(()));
        assert_eq!(
            run_check(cmd("exit 3"), limit),
            Err(InstallError::SelfTestFailed)
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_check_that_takes_too_long_is_stopped_and_fails() {
        let slow = cmd("ping -n 30 127.0.0.1 >NUL");
        assert_eq!(
            run_check(slow, Duration::from_millis(300)),
            Err(InstallError::SelfTestFailed)
        );
    }

    #[test]
    fn a_program_that_cannot_be_started_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.exe");
        assert!(matches!(
            self_test(&missing, "0.1.5", Duration::from_secs(1)),
            Err(InstallError::SelfTestStart(_))
        ));
        assert!(matches!(
            start_updated(&missing, "0.1.4", None),
            Err(InstallError::Start(_))
        ));
    }
}
