//! The only code that changes a settings file on disk.
//!
//! Every write goes through a [`BackupSession`], which makes sure a verified
//! backup of the file as it was before the app changed it exists, checks
//! that the file is still what was examined, replaces it atomically, and
//! reads it back.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use chrono::{DateTime, Local};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::discovery::file_name;
use crate::inspect::{read_settings_bytes, InspectError};

/// The folder inside a profile folder that holds every backup.
const BACKUPS_FOLDER: &str = "clone-bay-backups";
const TEMP_SUFFIX: &str = ".clone-bay.tmp";

/// A SHA-256 of a file's contents, used to tell whether it has changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
}

/// Why a file was not written, or a backup not made.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WriteError {
    #[error("it changed after it was checked (is EVE running?), so it was left untouched")]
    ChangedSinceCheck,
    #[error("it can't be read any more ({0}), so it was left untouched")]
    CannotRead(String),
    #[error("it couldn't be written ({0}), so it was left untouched")]
    CannotWrite(String),
    #[error("IT WAS WRITTEN BUT DOESN'T READ BACK CORRECTLY. Restore it from the backup")]
    ReadBackMismatch,
    #[error("it has no backup of what it holds now, so it was left untouched")]
    NotBackedUp,
    #[error("couldn't create the backup folder ({0})")]
    BackupFolder(String),
    #[error("couldn't back up {file} ({reason})")]
    BackupFailed { file: String, reason: String },
    #[error("{0} changed after it was checked, or its backup doesn't match (is EVE running?)")]
    BackupMismatch(String),
}

/// The time part of a backup folder's name, in local time.
pub fn backup_stamp(now: DateTime<Local>) -> String {
    now.format("%Y%m%d-%H%M%S").to_string()
}

/// [`backup_stamp`] for the current moment.
pub fn backup_stamp_now() -> String {
    backup_stamp(Local::now())
}

/// The backups made during one run of the app, and the only way to change a
/// file.
///
/// A file is backed up the first time it is written. Later writes keep that
/// backup for as long as the file still holds what was last written through
/// this session; a file that something else has changed since is backed up
/// again. No backup is ever overwritten or deleted.
#[derive(Debug)]
pub struct BackupSession {
    stamp: String,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    /// The folder this run's backups go in, for each profile folder.
    folders: HashMap<PathBuf, PathBuf>,
    /// The backup of each file, by the file's path.
    kept: HashMap<PathBuf, Kept>,
}

#[derive(Debug)]
struct Kept {
    folder: PathBuf,
    /// What the backup holds.
    original: Fingerprint,
    /// What the file held when it was backed up, or was last given here.
    current: Fingerprint,
}

impl BackupSession {
    /// A session whose backup folders are named with `stamp`.
    pub fn new(stamp: impl Into<String>) -> Self {
        Self {
            stamp: stamp.into(),
            state: Mutex::default(),
        }
    }

    /// A session named with the current time.
    pub fn starting_now() -> Self {
        Self::new(backup_stamp_now())
    }

    fn state(&self) -> MutexGuard<'_, State> {
        // The records are only ever changed in one step, so they are still
        // sound if another thread panicked while holding them.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Makes sure there is a backup of `file` as it was before this run
    /// changed it, and returns the folder holding it.
    ///
    /// `expected` is the contents that were examined. If the file still
    /// holds what this session last wrote, and its backup is still exactly
    /// what was saved, that backup stands. Otherwise the file is copied and
    /// the copy proved to be exactly those contents.
    pub fn back_up(&self, file: &Path, expected: Fingerprint) -> Result<PathBuf, WriteError> {
        let mut state = self.state();
        let name = file_name(file);
        if let Some(kept) = state.kept.get(file) {
            if kept.current == expected && holds(&kept.folder.join(&name), kept.original) {
                return Ok(kept.folder.clone());
            }
        }
        let profile_dir = file.parent().unwrap_or(Path::new(".")).to_path_buf();
        let folder = match state.folders.get(&profile_dir) {
            Some(folder) if has_room_for(folder, &name) => folder.clone(),
            _ => {
                let folder = create_backup_folder(&profile_dir, &self.stamp)?;
                state.folders.insert(profile_dir, folder.clone());
                folder
            }
        };
        copy_verified(file, &folder, expected)?;
        state.kept.insert(
            file.to_path_buf(),
            Kept {
                folder: folder.clone(),
                original: expected,
                current: expected,
            },
        );
        Ok(folder)
    }

    /// Replaces the contents of `path` with `data`.
    ///
    /// Refused unless [`back_up`](Self::back_up) has covered the contents
    /// being replaced. The file is left untouched unless it still has the
    /// contents that were examined earlier. The new contents go to a
    /// temporary file beside it, which is flushed to disk and then swapped
    /// in atomically, so the file is never seen half-written. Finally the
    /// file is read back and compared.
    pub fn replace_file(
        &self,
        path: &Path,
        data: &[u8],
        expected: Fingerprint,
    ) -> Result<(), WriteError> {
        let mut state = self.state();
        let Some(kept) = state
            .kept
            .get_mut(path)
            .filter(|kept| kept.current == expected)
        else {
            return Err(WriteError::NotBackedUp);
        };
        replace_file(path, data, expected)?;
        kept.current = Fingerprint::of(data);
        Ok(())
    }
}

/// Whether the file at `path` can be read and has exactly these contents.
fn holds(path: &Path, contents: Fingerprint) -> bool {
    read_settings_bytes(path).is_ok_and(|bytes| Fingerprint::of(&bytes) == contents)
}

/// Whether `folder` is still there and has nothing called `name` in it, so
/// a backup can be added without replacing one.
fn has_room_for(folder: &Path, name: &str) -> bool {
    folder.is_dir() && matches!(folder.join(name).try_exists(), Ok(false))
}

/// Creates a new, empty backup folder inside `profile_dir`'s backups folder.
///
/// The name is `stamp`; if that folder already exists a number is added, so
/// an existing backup is never reused or overwritten.
fn create_backup_folder(profile_dir: &Path, stamp: &str) -> Result<PathBuf, WriteError> {
    let all = profile_dir.join(BACKUPS_FOLDER);
    fs::create_dir_all(&all).map_err(|error| WriteError::BackupFolder(error.to_string()))?;
    for attempt in 1..=100 {
        let name = match attempt {
            1 => stamp.to_owned(),
            n => format!("{stamp}-{n}"),
        };
        let folder = all.join(name);
        match fs::create_dir(&folder) {
            Ok(()) => return Ok(folder),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(WriteError::BackupFolder(error.to_string())),
        }
    }
    Err(WriteError::BackupFolder(
        "too many backups with the same time".to_owned(),
    ))
}

/// Copies `file` into `backup_dir` and proves the copy is exactly the
/// contents that were examined earlier.
///
/// The copy is a new file (an existing one is never replaced) and is on the
/// disk, not just in the system's cache, before this returns, so a power cut
/// during the write that follows can't take the backup with it.
fn copy_verified(file: &Path, backup_dir: &Path, expected: Fingerprint) -> Result<(), WriteError> {
    let name = file_name(file);
    let failed = |reason: String| WriteError::BackupFailed {
        file: name.clone(),
        reason,
    };
    let original = read_settings_bytes(file).map_err(|error| failed(inspect_reason(error)))?;
    if Fingerprint::of(&original) != expected {
        return Err(WriteError::BackupMismatch(name));
    }
    let destination = backup_dir.join(&name);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .and_then(|mut copy| {
            copy.write_all(&original)?;
            copy.sync_all()
        })
        .map_err(|error| failed(error.to_string()))?;
    let copied =
        read_settings_bytes(&destination).map_err(|error| failed(inspect_reason(error)))?;
    if Fingerprint::of(&copied) != expected {
        return Err(WriteError::BackupMismatch(name));
    }
    Ok(())
}

fn inspect_reason(error: InspectError) -> String {
    match error {
        InspectError::Read(reason) => reason,
        other => other.to_string(),
    }
}

/// Replaces the contents of `path` with `data`, with every check but the
/// backup. See [`BackupSession::replace_file`].
fn replace_file(path: &Path, data: &[u8], expected: Fingerprint) -> Result<(), WriteError> {
    let current =
        read_settings_bytes(path).map_err(|error| WriteError::CannotRead(inspect_reason(error)))?;
    if Fingerprint::of(&current) != expected {
        return Err(WriteError::ChangedSinceCheck);
    }

    let temp = temp_path(path);
    if let Err(error) = write_then_swap(&temp, path, data) {
        // Best effort: the original is intact either way.
        let _ = fs::remove_file(&temp);
        return Err(WriteError::CannotWrite(error.to_string()));
    }

    confirm_written(path, data)
}

/// Reads a file that was just written and checks it holds exactly `data`.
fn confirm_written(path: &Path, data: &[u8]) -> Result<(), WriteError> {
    match read_settings_bytes(path) {
        Ok(written) if written == data => Ok(()),
        _ => Err(WriteError::ReadBackMismatch),
    }
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(TEMP_SUFFIX);
    path.with_file_name(name)
}

fn write_then_swap(temp: &Path, path: &Path, data: &[u8]) -> io::Result<()> {
    let mut file = fs::File::create(temp)?;
    file.write_all(data)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temp, path)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn folder_with(name: &str, contents: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        fs::write(&path, contents).unwrap();
        (dir, path)
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn make_read_only(path: &Path, read_only: bool) {
        let mut permissions = fs::metadata(path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(read_only);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[test]
    fn fingerprints_tell_contents_apart() {
        assert_eq!(Fingerprint::of(b"abc"), Fingerprint::of(b"abc"));
        assert_ne!(Fingerprint::of(b"abc"), Fingerprint::of(b"abd"));
        assert_ne!(Fingerprint::of(b""), Fingerprint::of(b"\0"));
        // The standard SHA-256 test vector for "abc" starts ba7816bf.
        assert_eq!(&Fingerprint::of(b"abc").0[..4], &[0xBA, 0x78, 0x16, 0xBF]);
    }

    #[test]
    fn backup_stamp_is_sortable_local_time() {
        let time = Local.with_ymd_and_hms(2026, 3, 7, 4, 5, 9).unwrap();
        assert_eq!(backup_stamp(time), "20260307-040509");
    }

    #[test]
    fn the_current_stamp_is_a_real_date_and_time() {
        let stamp = backup_stamp_now();
        let parsed = chrono::NaiveDateTime::parse_from_str(&stamp, "%Y%m%d-%H%M%S");
        assert!(parsed.is_ok(), "{stamp}");
        assert_eq!(stamp.len(), 15);
    }

    #[test]
    fn a_file_that_does_not_read_back_as_written_is_reported() {
        let (dir, path) = folder_with("core_user_1.dat", b"what was meant");
        assert_eq!(confirm_written(&path, b"what was meant"), Ok(()));
        assert_eq!(
            confirm_written(&path, b"something else"),
            Err(WriteError::ReadBackMismatch)
        );
        assert_eq!(
            confirm_written(&path, b"what was mean"),
            Err(WriteError::ReadBackMismatch)
        );
        let missing = dir.path().join("core_user_2.dat");
        assert_eq!(
            confirm_written(&missing, b""),
            Err(WriteError::ReadBackMismatch)
        );
    }

    #[test]
    fn backup_folders_never_reuse_an_existing_one() {
        let dir = tempfile::tempdir().unwrap();
        let first = create_backup_folder(dir.path(), "S").unwrap();
        let second = create_backup_folder(dir.path(), "S").unwrap();
        let third = create_backup_folder(dir.path(), "S").unwrap();
        // One folder in the profile holds them all.
        assert_eq!(names(dir.path()), ["clone-bay-backups"]);
        let all = dir.path().join("clone-bay-backups");
        assert_eq!(names(&all), ["S", "S-2", "S-3"]);
        assert_eq!(first, all.join("S"));
        assert_eq!(second, all.join("S-2"));
        assert_eq!(third, all.join("S-3"));
        assert!(names(&first).is_empty());
    }

    #[test]
    fn a_backup_folder_that_cannot_be_created_is_an_error() {
        // A file where the profile folder should be.
        let (_dir, not_a_folder) = folder_with("core_user_1.dat", b"x");
        let result = create_backup_folder(&not_a_folder, "S");
        let Err(WriteError::BackupFolder(reason)) = &result else {
            panic!("{result:?}");
        };
        // The real reason, straight away, not "too many backups" after
        // a hundred tries.
        assert!(reason.contains("os error"), "{reason}");

        // The same when the backups folder is fine but this run's folder
        // can't be made in it: here its name points into a missing folder.
        let dir = tempfile::tempdir().unwrap();
        let result = create_backup_folder(dir.path(), "missing/S");
        let Err(WriteError::BackupFolder(reason)) = &result else {
            panic!("{result:?}");
        };
        assert!(reason.contains("os error"), "{reason}");
        assert!(names(&dir.path().join("clone-bay-backups")).is_empty());
    }

    #[test]
    fn giving_up_after_too_many_same_named_backups() {
        let dir = tempfile::tempdir().unwrap();
        for _ in 0..100 {
            create_backup_folder(dir.path(), "S").unwrap();
        }
        let result = create_backup_folder(dir.path(), "S");
        assert_eq!(
            result,
            Err(WriteError::BackupFolder(
                "too many backups with the same time".to_owned()
            ))
        );
        assert_eq!(names(&dir.path().join("clone-bay-backups")).len(), 100);
    }

    fn session() -> BackupSession {
        BackupSession::new("S")
    }

    fn backups(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("clone-bay-backups")
    }

    #[test]
    fn a_backup_is_an_exact_copy_in_this_runs_folder() {
        let (dir, path) = folder_with("core_user_1.dat", b"original");
        let folder = session()
            .back_up(&path, Fingerprint::of(b"original"))
            .unwrap();
        assert_eq!(folder, backups(&dir).join("S"));
        assert_eq!(
            fs::read(folder.join("core_user_1.dat")).unwrap(),
            b"original"
        );
        assert_eq!(fs::read(&path).unwrap(), b"original");
    }

    #[test]
    fn a_backup_of_a_file_that_changed_is_refused() {
        let (_dir, path) = folder_with("core_user_1.dat", b"changed since");
        assert_eq!(
            session().back_up(&path, Fingerprint::of(b"original")),
            Err(WriteError::BackupMismatch("core_user_1.dat".to_owned()))
        );
    }

    #[test]
    fn a_file_that_changed_is_not_copied_into_the_backups() {
        let (dir, path) = folder_with("core_user_1.dat", b"changed since");
        let folder = backups(&dir).join("S");
        fs::create_dir_all(&folder).unwrap();
        assert_eq!(
            copy_verified(&path, &folder, Fingerprint::of(b"original")),
            Err(WriteError::BackupMismatch("core_user_1.dat".to_owned()))
        );
        assert!(names(&folder).is_empty());
    }

    #[test]
    fn a_backup_never_replaces_a_file_already_there() {
        let (dir, path) = folder_with("core_user_1.dat", b"original");
        let folder = backups(&dir).join("S");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("core_user_1.dat"), b"an older backup").unwrap();
        let result = copy_verified(&path, &folder, Fingerprint::of(b"original"));
        let Err(WriteError::BackupFailed { file, reason }) = &result else {
            panic!("{result:?}");
        };
        assert_eq!(file, "core_user_1.dat");
        assert!(reason.contains("os error"), "{reason}");
        assert_eq!(
            fs::read(folder.join("core_user_1.dat")).unwrap(),
            b"an older backup"
        );
    }

    #[test]
    fn a_read_only_file_can_be_backed_up_and_its_backup_is_not_read_only() {
        let (dir, path) = folder_with("core_user_1.dat", b"original");
        make_read_only(&path, true);
        let result = session().back_up(&path, Fingerprint::of(b"original"));
        make_read_only(&path, false);
        let folder = result.unwrap();
        assert_eq!(folder, backups(&dir).join("S"));
        let backup = folder.join("core_user_1.dat");
        assert_eq!(fs::read(&backup).unwrap(), b"original");
        assert!(!fs::metadata(&backup).unwrap().permissions().readonly());
    }

    #[test]
    fn a_backup_of_a_missing_file_is_an_error_that_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let result = session().back_up(&dir.path().join("core_user_9.dat"), Fingerprint::of(b""));
        let Err(WriteError::BackupFailed { file, reason }) = result else {
            panic!("{result:?}");
        };
        assert_eq!(file, "core_user_9.dat");
        assert!(reason.contains("os error"), "{reason}");
    }

    #[test]
    fn later_writes_in_the_same_run_keep_the_one_backup() {
        let (dir, path) = folder_with("core_user_1.dat", b"original");
        let session = session();
        let first = session
            .back_up(&path, Fingerprint::of(b"original"))
            .unwrap();
        session
            .replace_file(&path, b"edit one", Fingerprint::of(b"original"))
            .unwrap();
        let second = session
            .back_up(&path, Fingerprint::of(b"edit one"))
            .unwrap();
        session
            .replace_file(&path, b"edit two", Fingerprint::of(b"edit one"))
            .unwrap();
        let third = session
            .back_up(&path, Fingerprint::of(b"edit two"))
            .unwrap();

        assert_eq!(second, first);
        assert_eq!(third, first);
        assert_eq!(names(&backups(&dir)), ["S"]);
        assert_eq!(names(&first), ["core_user_1.dat"]);
        assert_eq!(
            fs::read(first.join("core_user_1.dat")).unwrap(),
            b"original"
        );
        assert_eq!(fs::read(&path).unwrap(), b"edit two");
    }

    #[test]
    fn a_file_something_else_changed_is_backed_up_again() {
        let (dir, path) = folder_with("core_user_1.dat", b"original");
        let session = session();
        let first = session
            .back_up(&path, Fingerprint::of(b"original"))
            .unwrap();
        session
            .replace_file(&path, b"edited", Fingerprint::of(b"original"))
            .unwrap();
        fs::write(&path, b"the game saved this").unwrap();

        let second = session
            .back_up(&path, Fingerprint::of(b"the game saved this"))
            .unwrap();
        // Same file name, so it needs a folder of its own.
        assert_eq!(second, backups(&dir).join("S-2"));
        assert_eq!(
            fs::read(second.join("core_user_1.dat")).unwrap(),
            b"the game saved this"
        );
        assert_eq!(
            fs::read(first.join("core_user_1.dat")).unwrap(),
            b"original"
        );
    }

    #[test]
    fn a_backup_that_was_never_followed_by_a_write_is_reused() {
        // A copy refused part-way through its backups, then tried again.
        let (dir, path) = folder_with("core_user_1.dat", b"original");
        let session = session();
        let first = session
            .back_up(&path, Fingerprint::of(b"original"))
            .unwrap();
        let second = session
            .back_up(&path, Fingerprint::of(b"original"))
            .unwrap();
        assert_eq!(second, first);
        assert_eq!(names(&backups(&dir)), ["S"]);
    }

    #[test]
    fn a_backup_that_has_gone_is_made_again() {
        let (dir, path) = folder_with("core_user_1.dat", b"original");
        let session = session();
        let folder = session
            .back_up(&path, Fingerprint::of(b"original"))
            .unwrap();
        session
            .replace_file(&path, b"edited", Fingerprint::of(b"original"))
            .unwrap();
        fs::remove_file(folder.join("core_user_1.dat")).unwrap();

        let again = session.back_up(&path, Fingerprint::of(b"edited")).unwrap();
        assert_eq!(again, folder);
        assert_eq!(fs::read(again.join("core_user_1.dat")).unwrap(), b"edited");
        assert_eq!(names(&backups(&dir)), ["S"]);
    }

    #[test]
    fn a_backup_that_was_altered_is_not_trusted_and_not_overwritten() {
        let (dir, path) = folder_with("core_user_1.dat", b"original");
        let session = session();
        let folder = session
            .back_up(&path, Fingerprint::of(b"original"))
            .unwrap();
        session
            .replace_file(&path, b"edited", Fingerprint::of(b"original"))
            .unwrap();
        fs::write(folder.join("core_user_1.dat"), b"tampered").unwrap();

        let again = session.back_up(&path, Fingerprint::of(b"edited")).unwrap();
        assert_eq!(again, backups(&dir).join("S-2"));
        assert_eq!(fs::read(again.join("core_user_1.dat")).unwrap(), b"edited");
        assert_eq!(
            fs::read(folder.join("core_user_1.dat")).unwrap(),
            b"tampered"
        );
    }

    #[test]
    fn a_write_that_failed_is_not_remembered_as_written() {
        let (dir, path) = folder_with("core_user_1.dat", b"original");
        let session = session();
        session
            .back_up(&path, Fingerprint::of(b"original"))
            .unwrap();
        fs::write(&path, b"changed under us").unwrap();
        assert_eq!(
            session.replace_file(&path, b"edited", Fingerprint::of(b"original")),
            Err(WriteError::ChangedSinceCheck)
        );
        // Even if the file then happens to hold what would have been
        // written, the app didn't write it, so it is backed up.
        fs::write(&path, b"edited").unwrap();
        let again = session.back_up(&path, Fingerprint::of(b"edited")).unwrap();
        assert_eq!(again, backups(&dir).join("S-2"));
        assert_eq!(fs::read(again.join("core_user_1.dat")).unwrap(), b"edited");
    }

    #[test]
    fn files_backed_up_in_one_run_share_a_folder() {
        let (dir, first) = folder_with("core_user_1.dat", b"one");
        let second = dir.path().join("core_char_2.dat");
        fs::write(&second, b"two").unwrap();
        let session = session();
        let a = session.back_up(&first, Fingerprint::of(b"one")).unwrap();
        let b = session.back_up(&second, Fingerprint::of(b"two")).unwrap();
        assert_eq!(a, b);
        assert_eq!(names(&a), ["core_char_2.dat", "core_user_1.dat"]);
        assert_eq!(names(&backups(&dir)), ["S"]);
    }

    #[test]
    fn each_profile_folder_gets_its_own_backups() {
        let (one, first) = folder_with("core_user_1.dat", b"one");
        let (two, second) = folder_with("core_user_1.dat", b"two");
        let session = session();
        let a = session.back_up(&first, Fingerprint::of(b"one")).unwrap();
        let b = session.back_up(&second, Fingerprint::of(b"two")).unwrap();
        assert_eq!(a, backups(&one).join("S"));
        assert_eq!(b, backups(&two).join("S"));
        assert_eq!(fs::read(a.join("core_user_1.dat")).unwrap(), b"one");
        assert_eq!(fs::read(b.join("core_user_1.dat")).unwrap(), b"two");
    }

    #[test]
    fn a_new_run_never_reuses_an_earlier_runs_backup() {
        let (dir, path) = folder_with("core_user_1.dat", b"original");
        let earlier = session();
        let first = earlier
            .back_up(&path, Fingerprint::of(b"original"))
            .unwrap();
        earlier
            .replace_file(&path, b"edited", Fingerprint::of(b"original"))
            .unwrap();

        let later = BackupSession::new("T");
        let second = later.back_up(&path, Fingerprint::of(b"edited")).unwrap();
        assert_eq!(second, backups(&dir).join("T"));
        assert_eq!(fs::read(second.join("core_user_1.dat")).unwrap(), b"edited");
        assert_eq!(
            fs::read(first.join("core_user_1.dat")).unwrap(),
            b"original"
        );
        // Even one started in the same second.
        let third = session()
            .back_up(&path, Fingerprint::of(b"edited"))
            .unwrap();
        assert_eq!(third, backups(&dir).join("S-2"));
    }

    #[test]
    fn a_file_with_no_backup_is_never_written() {
        let (dir, path) = folder_with("core_user_1.dat", b"original");
        let session = session();
        assert_eq!(
            session.replace_file(&path, b"new", Fingerprint::of(b"original")),
            Err(WriteError::NotBackedUp)
        );
        // Nor one whose backup is of something other than what it holds now.
        session
            .back_up(&path, Fingerprint::of(b"original"))
            .unwrap();
        fs::write(&path, b"changed since").unwrap();
        assert_eq!(
            session.replace_file(&path, b"new", Fingerprint::of(b"changed since")),
            Err(WriteError::NotBackedUp)
        );
        assert_eq!(fs::read(&path).unwrap(), b"changed since");
        assert!(WriteError::NotBackedUp
            .to_string()
            .ends_with("left untouched"));
        assert_eq!(names(dir.path()), ["clone-bay-backups", "core_user_1.dat"]);
    }

    #[test]
    fn replacing_writes_the_new_contents_and_leaves_no_temp_file() {
        let (dir, path) = folder_with("core_user_1.dat", b"old");
        replace_file(&path, b"new contents", Fingerprint::of(b"old")).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new contents");
        assert_eq!(names(dir.path()), ["core_user_1.dat"]);
    }

    #[test]
    fn a_file_that_changed_since_the_check_is_left_untouched() {
        let (dir, path) = folder_with("core_user_1.dat", b"someone else wrote this");
        assert_eq!(
            replace_file(&path, b"new", Fingerprint::of(b"old")),
            Err(WriteError::ChangedSinceCheck)
        );
        assert_eq!(fs::read(&path).unwrap(), b"someone else wrote this");
        assert_eq!(names(dir.path()), ["core_user_1.dat"]);
    }

    #[test]
    fn a_file_that_has_gone_is_reported_and_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core_user_1.dat");
        let result = replace_file(&path, b"new", Fingerprint::of(b"old"));
        let Err(WriteError::CannotRead(reason)) = &result else {
            panic!("{result:?}");
        };
        // The system's reason on its own, not wrapped a second time.
        assert!(reason.contains("os error"), "{reason}");
        assert!(!reason.contains("can't read it"), "{reason}");
        assert!(names(dir.path()).is_empty());
    }

    #[test]
    fn a_failed_swap_leaves_the_original_and_removes_the_temp_file() {
        // A read-only file can't be replaced on Windows.
        let (dir, path) = folder_with("core_user_1.dat", b"old");
        make_read_only(&path, true);
        let result = replace_file(&path, b"new", Fingerprint::of(b"old"));
        make_read_only(&path, false);
        if cfg!(windows) {
            assert!(
                matches!(result, Err(WriteError::CannotWrite(_))),
                "{result:?}"
            );
            assert_eq!(fs::read(&path).unwrap(), b"old");
        }
        assert_eq!(names(dir.path()), ["core_user_1.dat"]);
    }

    #[test]
    fn a_stale_temp_file_is_overwritten_not_trusted() {
        let (dir, path) = folder_with("core_user_1.dat", b"old");
        fs::write(
            dir.path().join("core_user_1.dat.clone-bay.tmp"),
            b"junk from a crash",
        )
        .unwrap();
        replace_file(&path, b"new", Fingerprint::of(b"old")).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(names(dir.path()), ["core_user_1.dat"]);
    }

    #[test]
    fn the_temp_file_sits_beside_the_target() {
        let path = Path::new("folder").join("core_char_5.dat");
        assert_eq!(
            temp_path(&path),
            Path::new("folder").join("core_char_5.dat.clone-bay.tmp")
        );
    }

    #[test]
    fn messages_say_what_happened_to_the_file() {
        assert!(WriteError::ChangedSinceCheck
            .to_string()
            .ends_with("left untouched"));
        assert!(WriteError::CannotWrite("denied".into())
            .to_string()
            .contains("(denied)"));
        assert!(WriteError::ReadBackMismatch
            .to_string()
            .contains("Restore it from the backup"));
        let failed = WriteError::BackupFailed {
            file: "a.dat".into(),
            reason: "denied".into(),
        };
        assert_eq!(failed.to_string(), "couldn't back up a.dat (denied)");
    }
}
