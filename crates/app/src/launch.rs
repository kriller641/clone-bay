//! What the app was started to do, read from its command line.

use std::ffi::OsString;
use std::path::PathBuf;

use clone_bay_core::install::{AFTER_UPDATE_FLAG, SELF_TEST_FLAG};

#[derive(Debug, PartialEq, Eq)]
pub enum Launch {
    /// Started only to prove this exe runs and is the version given.
    SelfTest(String),
    /// Open the window.
    Open {
        /// The version this one has just replaced.
        updated_from: Option<String>,
        /// A folder to open instead of the detected one.
        folder: Option<PathBuf>,
    },
}

/// Reads the arguments that follow the program's name.
pub fn parse(mut args: impl Iterator<Item = OsString>) -> Launch {
    let first = args.next();
    match first.as_ref().and_then(|arg| arg.to_str()) {
        Some(SELF_TEST_FLAG) => Launch::SelfTest(
            args.next()
                .and_then(|version| version.into_string().ok())
                .unwrap_or_default(),
        ),
        Some(AFTER_UPDATE_FLAG) => Launch::Open {
            updated_from: args.next().and_then(|version| version.into_string().ok()),
            folder: args.next().map(PathBuf::from),
        },
        _ => Launch::Open {
            updated_from: None,
            folder: first.map(PathBuf::from),
        },
    }
}

/// What a self-test exits with: 0 only if this exe is the version wanted.
pub fn self_test_code(wanted: &str) -> i32 {
    i32::from(wanted != env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Launch {
        parse(args.iter().map(OsString::from))
    }

    fn open(updated_from: Option<&str>, folder: Option<&str>) -> Launch {
        Launch::Open {
            updated_from: updated_from.map(str::to_owned),
            folder: folder.map(PathBuf::from),
        }
    }

    #[test]
    fn with_no_arguments_it_just_opens() {
        assert_eq!(parsed(&[]), open(None, None));
    }

    #[test]
    fn a_first_argument_is_the_folder_to_open() {
        assert_eq!(parsed(&["C:/eve"]), open(None, Some("C:/eve")));
    }

    #[test]
    fn after_an_update_it_knows_the_version_it_replaced_and_keeps_the_folder() {
        assert_eq!(
            parsed(&["--after-update", "0.1.4"]),
            open(Some("0.1.4"), None)
        );
        assert_eq!(
            parsed(&["--after-update", "0.1.4", "C:/eve"]),
            open(Some("0.1.4"), Some("C:/eve"))
        );
        // The flag with nothing after it: open as usual.
        assert_eq!(parsed(&["--after-update"]), open(None, None));
    }

    #[test]
    fn a_self_test_names_the_version_wanted() {
        assert_eq!(
            parsed(&["--self-test", "0.1.5"]),
            Launch::SelfTest("0.1.5".to_owned())
        );
        assert_eq!(parsed(&["--self-test"]), Launch::SelfTest(String::new()));
    }

    #[test]
    fn the_self_test_passes_only_for_this_version() {
        assert_eq!(self_test_code(env!("CARGO_PKG_VERSION")), 0);
        assert_eq!(self_test_code("0.0.0"), 1);
        assert_eq!(self_test_code(""), 1);
    }
}
