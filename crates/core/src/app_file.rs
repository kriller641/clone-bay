//! Writing the app's own small files, in its own folder. Settings files never
//! come through here: they go through [`crate::safe_write`].

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Replaces the file at `path` with `text` in one step, so a crash leaves the
/// old file or the new one and never part of either. Makes the folder if it
/// isn't there.
pub(crate) fn replace(path: &Path, text: &str) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    let written = fs::File::create(&temp)
        .and_then(|mut file| {
            file.write_all(text.as_bytes())?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&temp, path));
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}
