//! Options the client keeps in `prefs.ini`, a plain text file of
//! `key=value` lines beside the settings files in each profile folder.
//!
//! The only one handled here is "always show bracket text", which the game
//! has no menu for. The key and its values (`1` and `0`) are the ones EVE
//! Wrench writes.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::platform::EveStatus;
use crate::safe_write::{BackupSession, Fingerprint, WriteError};

const PREFS_FILE: &str = "prefs.ini";
const BRACKET_TEXT_KEY: &str = "bracketsAlwaysShowShipText";
/// Larger than any real `prefs.ini` by a wide margin.
const MAX_PREFS_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PrefsError {
    #[error("can't read prefs.ini ({0})")]
    Read(String),
    #[error("prefs.ini isn't the plain text file it should be")]
    NotText,
    #[error(
        "EVE is running. Close every EVE client first: the client would overwrite the change. \
         Nothing was changed"
    )]
    EveRunning,
    #[error(
        "Couldn't check whether EVE is running. Confirm that every EVE client is closed, then \
         try again. Nothing was changed"
    )]
    EveUnknown,
    #[error("self-check failed: the new prefs.ini isn't the old one with one line changed")]
    SelfCheck,
    #[error("{0}")]
    Write(#[from] WriteError),
}

fn prefs_path(profile_dir: &Path) -> PathBuf {
    profile_dir.join(PREFS_FILE)
}

/// The file's text, or `None` if there is no `prefs.ini`.
fn read_prefs(profile_dir: &Path) -> Result<Option<String>, PrefsError> {
    let bytes = match fs::read(prefs_path(profile_dir)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(PrefsError::Read(error.to_string())),
    };
    if bytes.len() > MAX_PREFS_BYTES || bytes.contains(&0) {
        return Err(PrefsError::NotText);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| PrefsError::NotText)
}

/// The value of `key` on a `key=value` line, if this line sets it.
fn value_of<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let (name, value) = line.split_once('=')?;
    (name.trim() == key).then_some(value.trim())
}

/// Whether the text turns the option on. The last line that sets it wins;
/// no line at all means off, the game's default.
pub fn bracket_text_in(text: &str) -> bool {
    text.lines()
        .filter_map(|line| value_of(line, BRACKET_TEXT_KEY))
        .next_back()
        .is_some_and(|value| value == "1")
}

/// `text` with the option set. Every other line, the kind of line ending,
/// and whether the file ends with one, are kept as they were.
pub fn with_bracket_text(text: &str, enabled: bool) -> String {
    let line_ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let new_line = format!("{BRACKET_TEXT_KEY}={}", u8::from(enabled));
    let ends_with_newline = text.ends_with('\n');
    let mut lines: Vec<&str> = text.lines().collect();
    let mut replaced = false;
    for line in &mut lines {
        if value_of(line, BRACKET_TEXT_KEY).is_some() {
            *line = &new_line;
            replaced = true;
        }
    }
    if !replaced {
        lines.push(&new_line);
    }
    let mut result = lines.join(line_ending);
    if ends_with_newline {
        result.push_str(line_ending);
    }
    result
}

/// Whether "always show bracket text" is on for a profile.
pub fn bracket_text(profile_dir: &Path) -> Result<bool, PrefsError> {
    Ok(read_prefs(profile_dir)?.is_some_and(|text| bracket_text_in(&text)))
}

/// Turns "always show bracket text" on or off for a profile.
///
/// Refuses if EVE is running, or might be and the user hasn't confirmed it
/// is closed. `backups` makes sure an existing `prefs.ini` has a verified
/// backup first; the folder holding it is returned. A profile with no
/// `prefs.ini` gets one holding just this line.
pub fn set_bracket_text(
    profile_dir: &Path,
    enabled: bool,
    eve: EveStatus,
    user_confirmed_eve_closed: bool,
    backups: &BackupSession,
) -> Result<Option<PathBuf>, PrefsError> {
    match eve {
        EveStatus::Running => return Err(PrefsError::EveRunning),
        EveStatus::Unknown if !user_confirmed_eve_closed => return Err(PrefsError::EveUnknown),
        EveStatus::Unknown | EveStatus::NotRunning => {}
    }
    let path = prefs_path(profile_dir);
    let Some(old) = read_prefs(profile_dir)? else {
        let text = with_bracket_text("", enabled);
        fs::write(&path, &text).map_err(|e| WriteError::CannotWrite(e.to_string()))?;
        return Ok(None);
    };
    let new = with_bracket_text(&old, enabled);
    if !is_the_intended_change(&old, &new, enabled) {
        return Err(PrefsError::SelfCheck);
    }
    if new == old {
        return Ok(None);
    }
    let fingerprint = Fingerprint::of(old.as_bytes());
    let backup_dir = backups.back_up(&path, fingerprint)?;
    backups.replace_file(&path, new.as_bytes(), fingerprint)?;
    Ok(Some(backup_dir))
}

/// The self-check: `new` must be `old` with nothing but the option's lines
/// different, and must leave the option in the wanted state.
fn is_the_intended_change(old: &str, new: &str, enabled: bool) -> bool {
    only_the_option_differs(old, new) && bracket_text_in(new) == enabled
}

/// Whether two versions of the file are the same apart from lines that set
/// the option.
fn only_the_option_differs(old: &str, new: &str) -> bool {
    let others = |text: &str| -> Vec<String> {
        text.lines()
            .filter(|line| value_of(line, BRACKET_TEXT_KEY).is_none())
            .map(str::to_owned)
            .collect()
    };
    others(old) == others(new)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = "clusterMode=LOCAL\r\ndebug=0\r\nport=26000";

    #[test]
    fn the_option_is_off_unless_a_line_turns_it_on() {
        assert!(!bracket_text_in(""));
        assert!(!bracket_text_in(REAL));
        assert!(bracket_text_in("bracketsAlwaysShowShipText=1"));
        assert!(bracket_text_in(
            "a=1\r\nbracketsAlwaysShowShipText=1\r\nb=2\r\n"
        ));
        assert!(bracket_text_in(" bracketsAlwaysShowShipText = 1 "));
        assert!(!bracket_text_in("bracketsAlwaysShowShipText=0"));
        assert!(!bracket_text_in("bracketsAlwaysShowShipText=true"));
        assert!(!bracket_text_in("bracketsAlwaysShowShipText"));
        // A different key that merely starts or ends the same way.
        assert!(!bracket_text_in("bracketsAlwaysShowShipTextToo=1"));
        assert!(!bracket_text_in("xbracketsAlwaysShowShipText=1"));
        // The last line wins.
        assert!(bracket_text_in(
            "bracketsAlwaysShowShipText=0\nbracketsAlwaysShowShipText=1"
        ));
        assert!(!bracket_text_in(
            "bracketsAlwaysShowShipText=1\nbracketsAlwaysShowShipText=0"
        ));
    }

    #[test]
    fn a_missing_line_is_added_at_the_end_in_the_files_own_style() {
        assert_eq!(
            with_bracket_text(REAL, true),
            "clusterMode=LOCAL\r\ndebug=0\r\nport=26000\r\nbracketsAlwaysShowShipText=1"
        );
        assert_eq!(
            with_bracket_text("a=1\nb=2\n", false),
            "a=1\nb=2\nbracketsAlwaysShowShipText=0\n"
        );
        assert_eq!(
            with_bracket_text("a=1\r\n", true),
            "a=1\r\nbracketsAlwaysShowShipText=1\r\n"
        );
        assert_eq!(with_bracket_text("", true), "bracketsAlwaysShowShipText=1");
    }

    #[test]
    fn an_existing_line_is_changed_in_place() {
        let text = "a=1\r\nbracketsAlwaysShowShipText=0\r\nb=2\r\n";
        assert_eq!(
            with_bracket_text(text, true),
            "a=1\r\nbracketsAlwaysShowShipText=1\r\nb=2\r\n"
        );
        assert_eq!(with_bracket_text(text, false), text);
        // Every line that sets it is brought into line.
        let twice = "bracketsAlwaysShowShipText=0\nx=1\nbracketsAlwaysShowShipText=1";
        assert_eq!(
            with_bracket_text(twice, true),
            "bracketsAlwaysShowShipText=1\nx=1\nbracketsAlwaysShowShipText=1"
        );
    }

    #[test]
    fn setting_it_round_trips_and_leaves_other_lines_alone() {
        for text in [
            REAL,
            "",
            "a=1\n",
            "x\r\n\r\ny=2\r\n",
            "bracketsAlwaysShowShipText=1\n",
        ] {
            for enabled in [true, false] {
                let new = with_bracket_text(text, enabled);
                assert_eq!(bracket_text_in(&new), enabled, "{text:?}");
                assert!(only_the_option_differs(text, &new), "{text:?}");
                assert_eq!(
                    with_bracket_text(&new, enabled),
                    new,
                    "setting twice changes nothing"
                );
            }
        }
        assert!(!only_the_option_differs("a=1\nb=2", "a=1\nb=3"));
        assert!(!only_the_option_differs("a=1\nb=2", "a=1"));
    }

    #[test]
    fn the_self_check_needs_both_the_right_state_and_untouched_lines() {
        let on = "a=1\nbracketsAlwaysShowShipText=1";
        assert!(is_the_intended_change("a=1", on, true));
        // The right lines but the wrong state.
        assert!(!is_the_intended_change("a=1", on, false));
        // The right state but another line changed.
        assert!(!is_the_intended_change("a=2", on, true));
        assert!(!is_the_intended_change("a=2", on, false));
        assert_eq!(MAX_PREFS_BYTES, 1_048_576);
    }

    fn profile(contents: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        if let Some(contents) = contents {
            fs::write(dir.path().join("prefs.ini"), contents).unwrap();
        }
        dir
    }

    #[test]
    fn the_option_is_read_from_and_written_to_the_profiles_prefs_file() {
        let dir = profile(Some(REAL));
        assert_eq!(bracket_text(dir.path()), Ok(false));

        let backup = set_bracket_text(
            dir.path(),
            true,
            EveStatus::NotRunning,
            false,
            &BackupSession::new("T"),
        )
        .unwrap();
        let backup = backup.expect("the old file should have been backed up");
        assert_eq!(backup, dir.path().join("clone-bay-backups").join("T"));
        assert_eq!(fs::read_to_string(backup.join("prefs.ini")).unwrap(), REAL);
        assert_eq!(
            fs::read_to_string(dir.path().join("prefs.ini")).unwrap(),
            format!("{REAL}\r\nbracketsAlwaysShowShipText=1")
        );
        assert_eq!(bracket_text(dir.path()), Ok(true));

        // Setting it to what it already is writes nothing and backs up nothing.
        let again = set_bracket_text(
            dir.path(),
            true,
            EveStatus::NotRunning,
            false,
            &BackupSession::new("U"),
        )
        .unwrap();
        assert_eq!(again, None);
        assert!(!dir.path().join("clone-bay-backups").join("U").exists());

        set_bracket_text(
            dir.path(),
            false,
            EveStatus::NotRunning,
            false,
            &BackupSession::new("V"),
        )
        .unwrap();
        assert_eq!(bracket_text(dir.path()), Ok(false));
    }

    #[test]
    fn a_profile_without_a_prefs_file_gets_one() {
        let dir = profile(None);
        assert_eq!(bracket_text(dir.path()), Ok(false));
        let backup = set_bracket_text(
            dir.path(),
            true,
            EveStatus::NotRunning,
            false,
            &BackupSession::new("T"),
        )
        .unwrap();
        assert_eq!(backup, None, "there was nothing to back up");
        assert_eq!(
            fs::read_to_string(dir.path().join("prefs.ini")).unwrap(),
            "bracketsAlwaysShowShipText=1"
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn nothing_is_written_while_eve_is_or_might_be_running() {
        let dir = profile(Some(REAL));
        for confirmed in [false, true] {
            assert_eq!(
                set_bracket_text(
                    dir.path(),
                    true,
                    EveStatus::Running,
                    confirmed,
                    &BackupSession::new("T")
                ),
                Err(PrefsError::EveRunning)
            );
        }
        assert_eq!(
            set_bracket_text(
                dir.path(),
                true,
                EveStatus::Unknown,
                false,
                &BackupSession::new("T")
            ),
            Err(PrefsError::EveUnknown)
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("prefs.ini")).unwrap(),
            REAL
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);

        assert!(set_bracket_text(
            dir.path(),
            true,
            EveStatus::Unknown,
            true,
            &BackupSession::new("T")
        )
        .is_ok());
        assert_eq!(bracket_text(dir.path()), Ok(true));
    }

    #[test]
    fn a_prefs_file_that_is_not_text_is_left_alone() {
        for contents in [&b"a=1\0b=2"[..], &[0xFF, 0xFE, b'a'][..]] {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("prefs.ini"), contents).unwrap();
            assert_eq!(bracket_text(dir.path()), Err(PrefsError::NotText));
            assert_eq!(
                set_bracket_text(
                    dir.path(),
                    true,
                    EveStatus::NotRunning,
                    false,
                    &BackupSession::new("T")
                ),
                Err(PrefsError::NotText)
            );
            assert_eq!(fs::read(dir.path().join("prefs.ini")).unwrap(), contents);
        }
        // Too large to be a real prefs file.
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("prefs.ini"),
            vec![b'a'; MAX_PREFS_BYTES + 1],
        )
        .unwrap();
        assert_eq!(bracket_text(dir.path()), Err(PrefsError::NotText));
        fs::write(dir.path().join("prefs.ini"), vec![b'a'; MAX_PREFS_BYTES]).unwrap();
        assert_eq!(bracket_text(dir.path()), Ok(false));
    }

    #[test]
    fn a_prefs_path_that_cannot_be_read_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("prefs.ini")).unwrap();
        assert!(matches!(bracket_text(dir.path()), Err(PrefsError::Read(_))));
    }
}
