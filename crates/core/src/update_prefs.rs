//! Whether the app looks for updates, and when it last did. Kept in the
//! app's own folder, never in a settings file.
//!
//! One record a line, tab-separated: `check` then `1` or `0`, and
//! `last_check` then seconds since 1970. A line that can't be read is
//! passed over.

use std::fs;
use std::io;
use std::path::Path;

use crate::app_file;

const SECONDS_BETWEEN_CHECKS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePrefs {
    /// Look for updates when the app opens.
    pub check: bool,
    /// When it last looked, in seconds since 1970.
    pub last_check: Option<u64>,
}

impl Default for UpdatePrefs {
    fn default() -> Self {
        Self {
            check: true,
            last_check: None,
        }
    }
}

impl UpdatePrefs {
    /// Reads what was saved. A file that doesn't exist yet holds the
    /// defaults.
    pub fn load(path: &Path) -> io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut text = format!("check\t{}\n", u8::from(self.check));
        if let Some(last) = self.last_check {
            text.push_str(&format!("last_check\t{last}\n"));
        }
        app_file::replace(path, &text)
    }

    /// Whether to look for an update now.
    pub fn due(&self, now: u64) -> bool {
        self.check
            && self
                .last_check
                .is_none_or(|last| now < last || now - last >= SECONDS_BETWEEN_CHECKS)
    }

    fn parse(text: &str) -> Self {
        let mut prefs = Self::default();
        for line in text.lines() {
            match line.split_once('\t') {
                Some(("check", "1")) => prefs.check = true,
                Some(("check", "0")) => prefs.check = false,
                Some(("last_check", value)) => {
                    if let Ok(last) = value.parse() {
                        prefs.last_check = Some(last);
                    }
                }
                _ => {}
            }
        }
        prefs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 24 * 60 * 60;

    #[test]
    fn with_nothing_saved_checking_is_on_and_due() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = UpdatePrefs::load(&dir.path().join("update.tsv")).unwrap();
        assert_eq!(
            prefs,
            UpdatePrefs {
                check: true,
                last_check: None
            }
        );
        assert!(prefs.due(1_000_000));
    }

    #[test]
    fn what_is_saved_is_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("update.tsv");
        let prefs = UpdatePrefs {
            check: false,
            last_check: Some(123),
        };
        prefs.save(&path).unwrap();
        assert_eq!(UpdatePrefs::load(&path).unwrap(), prefs);
    }

    #[test]
    fn a_check_is_due_once_a_day() {
        let prefs = UpdatePrefs {
            check: true,
            last_check: Some(1_000_000),
        };
        assert!(!prefs.due(1_000_000));
        assert!(!prefs.due(1_000_000 + DAY - 1));
        assert!(prefs.due(1_000_000 + DAY));
        // The clock was set back: check, so a wrong clock can't stop
        // checks for ever.
        assert!(prefs.due(999_999));
    }

    #[test]
    fn switched_off_it_is_never_due() {
        let prefs = UpdatePrefs {
            check: false,
            last_check: None,
        };
        assert!(!prefs.due(1_000_000));
    }

    #[test]
    fn the_last_line_for_the_tick_box_wins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update.tsv");
        std::fs::write(&path, "check\t0\ncheck\t1\n").unwrap();
        assert!(UpdatePrefs::load(&path).unwrap().check);
    }

    #[test]
    fn a_file_that_exists_but_cannot_be_read_is_an_error_not_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        // A folder where the file should be.
        assert!(UpdatePrefs::load(dir.path()).is_err());
    }

    #[test]
    fn lines_that_cannot_be_read_are_passed_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update.tsv");
        std::fs::write(
            &path,
            "check\tmaybe\nlast_check\tsoon\nnonsense\ncheck\t0\n",
        )
        .unwrap();
        assert_eq!(
            UpdatePrefs::load(&path).unwrap(),
            UpdatePrefs {
                check: false,
                last_check: None
            }
        );
    }
}
