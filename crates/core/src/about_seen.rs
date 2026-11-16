//! Whether the About page has been shown, so it opens by itself only the
//! first time the app is opened. Kept in the app's own folder, never in a
//! settings file.
//!
//! The file's presence is the whole record; what it holds doesn't matter.

use std::fs;
use std::io;
use std::path::Path;

use crate::app_file;

/// Whether the About page has been shown before. A file that doesn't exist
/// means it hasn't.
pub fn seen(path: &Path) -> io::Result<bool> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Records that the About page has been shown. Makes the folder if it isn't
/// there.
pub fn mark_seen(path: &Path) -> io::Result<()> {
    app_file::replace(path, "seen\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_never_marked_has_not_been_seen() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!seen(&dir.path().join("about_seen.txt")).unwrap());
    }

    #[test]
    fn marking_it_makes_the_folder_and_is_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clone-bay").join("about_seen.txt");
        mark_seen(&path).unwrap();
        assert!(seen(&path).unwrap());
        // Marking again changes nothing.
        mark_seen(&path).unwrap();
        assert!(seen(&path).unwrap());
    }

    #[test]
    fn a_folder_that_cannot_be_made_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let not_a_folder = dir.path().join("clone-bay");
        fs::write(&not_a_folder, b"a file").unwrap();
        assert!(mark_seen(&not_a_folder.join("about_seen.txt")).is_err());
    }
}
