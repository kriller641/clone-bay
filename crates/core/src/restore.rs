//! Putting a settings file back the way one of its backups holds it.
//!
//! A restore is a write like any other: the file as it is now is backed up
//! first, so a restore can itself be undone, and the backup it comes from is
//! never changed.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;
use thiserror::Error;

use crate::discovery::file_name;
use crate::inspect::{decode_settings, read_settings_bytes};
use crate::platform::EveStatus;
use crate::safe_write::{BackupSession, Fingerprint, WriteError};

/// Folders inside a profile folder that hold one folder per backup: this
/// app's, then the one builds from before its rename used.
const BACKUP_ROOTS: [&str; 2] = ["clone-bay-backups", "eve-settings-backups"];

/// The start of the name of the folders the earliest builds made directly
/// in the profile folder, one per change.
const OLDEST_LAYOUT_PREFIX: &str = "eve-settings-backup-";

/// The length of a backup folder's time stamp, `YYYYMMDD-HHMMSS`.
const STAMP_LEN: usize = 15;

/// One backup of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    /// The backed-up file itself.
    pub path: PathBuf,
    /// When it was taken, from its folder's name, in local time. `None` when
    /// the name doesn't say.
    pub taken: Option<NaiveDateTime>,
    /// Its size in bytes when it was listed.
    pub size: u64,
}

/// Why a file was not restored.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RestoreError {
    #[error(
        "EVE is running. Close every EVE client first: the client saves its settings when it \
         closes and would overwrite the restored file. Nothing was changed"
    )]
    EveRunning,
    #[error(
        "Couldn't check whether EVE is running. Confirm that every EVE client is closed, then \
         try again. Nothing was changed"
    )]
    EveUnknown,
    #[error(
        "the file changed after this screen was opened (EVE may have saved it), so nothing was \
         restored. Go back to the file list and open this screen again"
    )]
    ChangedSinceOpened,
    #[error("that backup changed after it was listed, so nothing was restored")]
    BackupChanged,
    #[error("that backup can't be used ({0}), so nothing was restored")]
    BackupUnusable(String),
    #[error("the file already holds exactly what that backup holds, so nothing was changed")]
    AlreadyHolds,
    #[error("{0}")]
    Write(#[from] WriteError),
}

/// Every backup of `file` in its profile folder, newest first.
///
/// Folders that can't be read are passed over: a backup that can't be found
/// is simply not offered.
pub fn list_backups(file: &Path) -> Vec<Backup> {
    let name = file_name(file);
    let Some(profile) = file.parent() else {
        return Vec::new();
    };
    let mut folders: Vec<(String, PathBuf)> = Vec::new();
    for root in BACKUP_ROOTS {
        folders.extend(sub_folders(&profile.join(root)));
    }
    folders.extend(
        sub_folders(profile)
            .filter_map(|(folder, path)| {
                let stamp = folder.strip_prefix(OLDEST_LAYOUT_PREFIX)?.to_owned();
                Some((stamp, path))
            })
            .collect::<Vec<_>>(),
    );

    let mut backups: Vec<(u32, Backup)> = folders
        .into_iter()
        .filter_map(|(stamp, folder)| {
            let path = folder.join(&name);
            let metadata = fs::metadata(&path).ok().filter(fs::Metadata::is_file)?;
            let (taken, repeat) = parse_stamp(&stamp);
            Some((
                repeat,
                Backup {
                    path,
                    taken,
                    size: metadata.len(),
                },
            ))
        })
        .collect();
    // Newest first; of two taken in the same second, the later-numbered one.
    backups.sort_by(|(a_repeat, a), (b_repeat, b)| {
        (b.taken, b_repeat, &b.path).cmp(&(a.taken, a_repeat, &a.path))
    });
    backups.into_iter().map(|(_, backup)| backup).collect()
}

/// The folders directly inside `folder`, by name.
fn sub_folders(folder: &Path) -> impl Iterator<Item = (String, PathBuf)> {
    fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry.path())))
}

/// Reads a backup folder's name: the time, and the number added when more
/// than one backup was taken in the same second (1 when there is none).
fn parse_stamp(name: &str) -> (Option<NaiveDateTime>, u32) {
    let (stamp, rest) = name.split_at_checked(STAMP_LEN).unwrap_or((name, ""));
    let taken = NaiveDateTime::parse_from_str(stamp, "%Y%m%d-%H%M%S").ok();
    let repeat = match rest.strip_prefix('-').map(str::parse) {
        Some(Ok(n)) => n,
        _ => 1,
    };
    (taken, repeat)
}

/// Replaces `file` with the contents of `backup`.
///
/// `opened_as` is what the file held when the user chose to restore it; if
/// it holds anything else now, nothing is done. The backup must still be
/// the size it was listed at and be a sound settings file. What the file
/// holds now is backed up first. Returns the folder that backup is in.
pub fn restore(
    file: &Path,
    opened_as: Fingerprint,
    backup: &Backup,
    eve: EveStatus,
    user_confirmed_eve_closed: bool,
    backups: &BackupSession,
) -> Result<PathBuf, RestoreError> {
    match eve {
        EveStatus::Running => return Err(RestoreError::EveRunning),
        EveStatus::Unknown if !user_confirmed_eve_closed => return Err(RestoreError::EveUnknown),
        EveStatus::Unknown | EveStatus::NotRunning => {}
    }
    let current =
        read_settings_bytes(file).map_err(|error| WriteError::CannotRead(error.to_string()))?;
    if Fingerprint::of(&current) != opened_as {
        return Err(RestoreError::ChangedSinceOpened);
    }
    let data = read_settings_bytes(&backup.path)
        .map_err(|error| RestoreError::BackupUnusable(error.to_string()))?;
    if data.len() as u64 != backup.size {
        return Err(RestoreError::BackupChanged);
    }
    decode_settings(&data).map_err(|error| RestoreError::BackupUnusable(error.to_string()))?;
    if Fingerprint::of(&data) == opened_as {
        return Err(RestoreError::AlreadyHolds);
    }

    let folder = backups.back_up(file, opened_as)?;
    backups.replace_file(file, &data, opened_as)?;
    Ok(folder)
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn at(h: u32, m: u32, s: u32) -> Option<NaiveDateTime> {
        NaiveDate::from_ymd_opt(2026, 10, 31).and_then(|day| day.and_hms_opt(h, m, s))
    }

    #[test]
    fn a_folder_name_gives_the_time_and_the_repeat_number() {
        assert_eq!(parse_stamp("20261031-174405"), (at(17, 44, 5), 1));
        assert_eq!(parse_stamp("20261031-174405-2"), (at(17, 44, 5), 2));
        assert_eq!(parse_stamp("20261031-174405-10"), (at(17, 44, 5), 10));
        assert_eq!(parse_stamp("20261031-174405-x"), (at(17, 44, 5), 1));
        assert_eq!(parse_stamp("my backups"), (None, 1));
        assert_eq!(parse_stamp(""), (None, 1));
        // A multi-byte character where the stamp would end isn't a panic.
        assert_eq!(parse_stamp("20261031-17440é"), (None, 1));
    }
}
