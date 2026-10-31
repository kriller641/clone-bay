//! Putting a file back from one of its backups.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use clone_bay_core::platform::EveStatus;
use clone_bay_core::restore::{list_backups, restore, Backup, RestoreError};
use clone_bay_core::safe_write::{BackupSession, Fingerprint, WriteError};
use clone_bay_marshal::{encode, Value};

const NAME: &str = "core_user_1.dat";

/// A small, sound settings file whose one setting is `marker`.
fn settings(marker: i64) -> Vec<u8> {
    let ui = Value::Dict(vec![(Value::Bytes(b"x".to_vec()), Value::Int(marker))]);
    encode(
        &Value::Dict(vec![(Value::Bytes(b"ui".to_vec()), ui)]),
        false,
    )
    .unwrap()
}

/// A profile folder holding the file, with `contents`.
fn profile(contents: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(NAME);
    fs::write(&file, contents).unwrap();
    (dir, file)
}

/// Puts a backup of the file holding `contents` in `folder` (relative to the
/// profile folder).
fn backup_in(profile: &Path, folder: &str, contents: &[u8]) -> PathBuf {
    let folder = profile.join(folder);
    fs::create_dir_all(&folder).unwrap();
    let path = folder.join(NAME);
    fs::write(&path, contents).unwrap();
    path
}

fn folders(backups: &[Backup]) -> Vec<String> {
    backups
        .iter()
        .map(|backup| {
            let folder = backup.path.parent().unwrap();
            let parent = folder.parent().unwrap().file_name().unwrap();
            format!(
                "{}/{}",
                parent.to_string_lossy(),
                folder.file_name().unwrap().to_string_lossy()
            )
        })
        .collect()
}

fn the_one(file: &Path) -> Backup {
    let backups = list_backups(file);
    assert_eq!(backups.len(), 1, "{backups:?}");
    backups.into_iter().next().unwrap()
}

#[test]
fn backups_are_listed_newest_first_from_every_layout() {
    let (dir, file) = profile(&settings(0));
    let root = dir.path();
    let folder = root.file_name().unwrap().to_string_lossy().into_owned();
    backup_in(root, "clone-bay-backups/20261031-120000", &settings(1));
    backup_in(root, "clone-bay-backups/20261031-120000-2", &settings(2));
    backup_in(root, "clone-bay-backups/20261031-120000-10", &settings(3));
    backup_in(root, "eve-settings-backups/20261020-090000", &settings(4));
    backup_in(root, "eve-settings-backup-20261009-080000", &settings(5));
    backup_in(root, "clone-bay-backups/not a date", &settings(6));
    // Folders without this file, and other files, aren't backups of it.
    fs::create_dir_all(root.join("clone-bay-backups/20261031-130000")).unwrap();
    fs::write(
        root.join("clone-bay-backups/20261031-130000/core_user_2.dat"),
        b"x",
    )
    .unwrap();
    // Nor is a folder that happens to share the file's name.
    fs::create_dir_all(root.join("clone-bay-backups/20261031-140000").join(NAME)).unwrap();
    fs::create_dir_all(root.join("some-other-folder")).unwrap();
    backup_in(root, "some-other-folder", &settings(7));

    let backups = list_backups(&file);
    assert_eq!(
        folders(&backups),
        [
            "clone-bay-backups/20261031-120000-10",
            "clone-bay-backups/20261031-120000-2",
            "clone-bay-backups/20261031-120000",
            "eve-settings-backups/20261020-090000",
            &format!("{folder}/eve-settings-backup-20261009-080000"),
            "clone-bay-backups/not a date",
        ]
    );
    assert_eq!(backups[3].taken.unwrap().to_string(), "2026-10-20 09:00:00");
    assert_eq!(backups[5].taken, None);
    assert_eq!(backups[0].size, settings(3).len() as u64);
}

#[test]
fn a_file_with_no_backups_has_none_listed() {
    let (_dir, file) = profile(&settings(0));
    assert!(list_backups(&file).is_empty());
}

#[test]
fn restoring_puts_the_backup_back_and_backs_up_what_was_there() {
    let (dir, file) = profile(&settings(0));
    backup_in(
        dir.path(),
        "clone-bay-backups/20261001-100000",
        &settings(1),
    );
    let backup = the_one(&file);
    let session = BackupSession::new("20261031-150000");

    let folder = restore(
        &file,
        Fingerprint::of(&settings(0)),
        &backup,
        EveStatus::NotRunning,
        false,
        &session,
    )
    .unwrap();

    assert_eq!(fs::read(&file).unwrap(), settings(1));
    assert_eq!(folder, dir.path().join("clone-bay-backups/20261031-150000"));
    assert_eq!(fs::read(folder.join(NAME)).unwrap(), settings(0));
    // The backup it came from is untouched.
    assert_eq!(fs::read(&backup.path).unwrap(), settings(1));
    // And the restore can itself be undone from the list.
    assert_eq!(list_backups(&file).len(), 2);
}

#[test]
fn a_damaged_file_can_be_restored() {
    let (dir, file) = profile(b"not a settings file");
    backup_in(
        dir.path(),
        "clone-bay-backups/20261001-100000",
        &settings(1),
    );
    let backup = the_one(&file);
    restore(
        &file,
        Fingerprint::of(b"not a settings file"),
        &backup,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("S"),
    )
    .unwrap();
    assert_eq!(fs::read(&file).unwrap(), settings(1));
    assert_eq!(
        fs::read(dir.path().join("clone-bay-backups/S").join(NAME)).unwrap(),
        b"not a settings file"
    );
}

/// Tries a restore that must be refused, and checks nothing was touched.
fn refused(
    file: &Path,
    opened_as: Fingerprint,
    backup: &Backup,
    eve: EveStatus,
    confirmed: bool,
) -> RestoreError {
    let before = fs::read(file).ok();
    let backup_before = fs::read(&backup.path).ok();
    let session = BackupSession::new("REFUSED");
    let result = restore(file, opened_as, backup, eve, confirmed, &session);
    assert_eq!(fs::read(file).ok(), before);
    assert_eq!(fs::read(&backup.path).ok(), backup_before);
    let profile = file.parent().unwrap();
    assert!(!profile.join("clone-bay-backups/REFUSED").exists());
    result.unwrap_err()
}

#[test]
fn nothing_is_restored_while_eve_may_be_running() {
    let (dir, file) = profile(&settings(0));
    backup_in(
        dir.path(),
        "clone-bay-backups/20261001-100000",
        &settings(1),
    );
    let backup = the_one(&file);
    let opened = Fingerprint::of(&settings(0));
    assert_eq!(
        refused(&file, opened, &backup, EveStatus::Running, true),
        RestoreError::EveRunning
    );
    assert_eq!(
        refused(&file, opened, &backup, EveStatus::Unknown, false),
        RestoreError::EveUnknown
    );
    // Once the user vouches that EVE is closed, it goes ahead.
    restore(
        &file,
        opened,
        &backup,
        EveStatus::Unknown,
        true,
        &BackupSession::new("S"),
    )
    .unwrap();
    assert_eq!(fs::read(&file).unwrap(), settings(1));
}

#[test]
fn a_file_that_changed_since_the_screen_opened_is_left_alone() {
    let (dir, file) = profile(&settings(9));
    backup_in(
        dir.path(),
        "clone-bay-backups/20261001-100000",
        &settings(1),
    );
    let backup = the_one(&file);
    assert_eq!(
        refused(
            &file,
            Fingerprint::of(&settings(0)),
            &backup,
            EveStatus::NotRunning,
            false
        ),
        RestoreError::ChangedSinceOpened
    );
}

#[test]
fn a_file_that_has_gone_is_not_created() {
    let (dir, file) = profile(&settings(0));
    backup_in(
        dir.path(),
        "clone-bay-backups/20261001-100000",
        &settings(1),
    );
    let backup = the_one(&file);
    fs::remove_file(&file).unwrap();
    let error = refused(
        &file,
        Fingerprint::of(&settings(0)),
        &backup,
        EveStatus::NotRunning,
        false,
    );
    assert!(
        matches!(error, RestoreError::Write(WriteError::CannotRead(_))),
        "{error:?}"
    );
    assert!(!file.exists());
}

#[test]
fn a_backup_that_changed_since_it_was_listed_is_not_used() {
    let (dir, file) = profile(&settings(0));
    let path = backup_in(
        dir.path(),
        "clone-bay-backups/20261001-100000",
        &settings(1),
    );
    let backup = the_one(&file);
    fs::write(&path, settings(100_000)).unwrap();
    assert_eq!(
        refused(
            &file,
            Fingerprint::of(&settings(0)),
            &backup,
            EveStatus::NotRunning,
            false
        ),
        RestoreError::BackupChanged
    );
}

#[test]
fn a_backup_that_has_gone_is_reported() {
    let (dir, file) = profile(&settings(0));
    let path = backup_in(
        dir.path(),
        "clone-bay-backups/20261001-100000",
        &settings(1),
    );
    let backup = the_one(&file);
    fs::remove_file(&path).unwrap();
    let error = refused(
        &file,
        Fingerprint::of(&settings(0)),
        &backup,
        EveStatus::NotRunning,
        false,
    );
    assert!(
        matches!(&error, RestoreError::BackupUnusable(reason) if reason.contains("os error")),
        "{error:?}"
    );
}

#[test]
fn a_damaged_backup_is_never_restored() {
    let (dir, file) = profile(&settings(0));
    let mut damaged = settings(1);
    damaged.truncate(damaged.len() - 1);
    backup_in(dir.path(), "clone-bay-backups/20261001-100000", &damaged);
    let backup = the_one(&file);
    let error = refused(
        &file,
        Fingerprint::of(&settings(0)),
        &backup,
        EveStatus::NotRunning,
        false,
    );
    assert!(
        matches!(error, RestoreError::BackupUnusable(_)),
        "{error:?}"
    );
    assert!(error.to_string().ends_with("so nothing was restored"));
}

#[test]
fn a_backup_identical_to_the_file_changes_nothing() {
    let (dir, file) = profile(&settings(0));
    backup_in(
        dir.path(),
        "clone-bay-backups/20261001-100000",
        &settings(0),
    );
    let backup = the_one(&file);
    assert_eq!(
        refused(
            &file,
            Fingerprint::of(&settings(0)),
            &backup,
            EveStatus::NotRunning,
            false
        ),
        RestoreError::AlreadyHolds
    );
}

#[test]
fn restoring_after_an_edit_in_the_same_run_keeps_the_first_backup() {
    // The file was edited earlier in this run, then the user restores the
    // backup that edit made. That backup is the original and stays as it is.
    let (dir, file) = profile(&settings(0));
    let session = BackupSession::new("S");
    session
        .back_up(&file, Fingerprint::of(&settings(0)))
        .unwrap();
    session
        .replace_file(&file, &settings(5), Fingerprint::of(&settings(0)))
        .unwrap();

    let backup = the_one(&file);
    let folder = restore(
        &file,
        Fingerprint::of(&settings(5)),
        &backup,
        EveStatus::NotRunning,
        false,
        &session,
    )
    .unwrap();
    assert_eq!(folder, dir.path().join("clone-bay-backups/S"));
    assert_eq!(fs::read(&file).unwrap(), settings(0));
    assert_eq!(fs::read(&backup.path).unwrap(), settings(0));
}
