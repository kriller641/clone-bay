//! Tests that drive the restore screen in the real UI, off-screen, against
//! files in a temp folder.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use clone_bay_core::platform::EveStatus;
use clone_bay_core::restore::list_backups;
use clone_bay_marshal::{encode, Value};
use egui_kittest::kittest::Queryable;

use crate::app::test_support::{click, harness_with, shows, wait_until, Ui};
use crate::restore_flow::{backup_label, RestoreFlow};
use crate::services::Services;
use crate::state::Screen;

/// A small, sound settings file whose one setting is `marker`.
fn settings(marker: i64) -> Vec<u8> {
    let ui = Value::Dict(vec![(Value::Bytes(b"x".to_vec()), Value::Int(marker))]);
    encode(
        &Value::Dict(vec![(Value::Bytes(b"ui".to_vec()), ui)]),
        false,
    )
    .unwrap()
}

fn put_backup(profile: &Path, folder: &str, name: &str, contents: &[u8]) {
    let folder = profile.join("clone-bay-backups").join(folder);
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(name), contents).unwrap();
}

/// Account 1 is sound and has one backup; character 2 is damaged and has
/// two; account 3 has none.
fn profile() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("core_user_1.dat"), settings(0)).unwrap();
    fs::write(dir.path().join("core_char_2.dat"), b"damaged").unwrap();
    fs::write(dir.path().join("core_user_3.dat"), settings(3)).unwrap();
    put_backup(
        dir.path(),
        "20261001-100000",
        "core_user_1.dat",
        &settings(1),
    );
    put_backup(
        dir.path(),
        "20261001-100000",
        "core_char_2.dat",
        &settings(21),
    );
    put_backup(
        dir.path(),
        "20261002-100000",
        "core_char_2.dat",
        &settings(22),
    );
    dir
}

fn services(eve: EveStatus) -> Services {
    Services {
        eve_status: Arc::new(move || eve),
        ..Services::for_tests()
    }
}

/// The app with the restore screen open on the file whose ID is `id`.
fn restore_screen(dir: &Path, eve: EveStatus, id: &str) -> Ui {
    let mut ui = harness_with(dir, services(eve));
    click(&mut ui, id);
    click(&mut ui, "Restore from a backup");
    assert!(ui.state().state.restore().is_some());
    ui
}

fn flow(ui: &Ui) -> &RestoreFlow {
    ui.state().state.restore().unwrap()
}

/// The label the list shows for the `nth` backup of `file`.
fn label_of(file: &Path, nth: usize) -> String {
    backup_label(&list_backups(file)[nth])
}

fn restore(ui: &mut Ui) {
    click(ui, "Restore this backup");
    wait_until(ui, |app| {
        app.state.restore().is_some_and(|flow| !flow.busy())
    });
}

#[test]
fn a_chosen_backup_is_put_back() {
    let dir = profile();
    let file = dir.path().join("core_user_1.dat");
    let mut ui = restore_screen(dir.path(), EveStatus::NotRunning, "1");
    ui.get_by_label("Restore account 1");
    let label = label_of(&file, 0);
    assert!(label.starts_with("1 Oct 2026, 10:00:00"), "{label}");
    // Nothing is picked yet, so there is nothing to restore.
    assert!(!flow(&ui).can_restore());

    click(&mut ui, &label);
    assert_eq!(flow(&ui).selected, Some(0));
    assert!(flow(&ui).can_restore());
    restore(&mut ui);

    assert!(flow(&ui).problem.is_none(), "{:?}", flow(&ui).problem);
    assert_eq!(fs::read(&file).unwrap(), settings(1));
    let saved_to = flow(&ui).saving.saved_to.clone().unwrap();
    assert_eq!(
        fs::read(saved_to.join("core_user_1.dat")).unwrap(),
        settings(0)
    );
    assert!(shows(
        &ui,
        &format!(
            "Restored. What the file held before was backed up in {}",
            saved_to.display()
        )
    ));
    // The list now holds the backup just made too, and nothing is picked.
    assert_eq!(flow(&ui).backups.len(), 2);
    assert_eq!(flow(&ui).selected, None);

    click(&mut ui, "Back to the file list");
    assert!(ui.state().state.restore().is_none());
}

#[test]
fn a_file_that_cant_be_used_can_still_be_restored() {
    let dir = profile();
    let file = dir.path().join("core_char_2.dat");
    let mut ui = restore_screen(dir.path(), EveStatus::NotRunning, "2");
    // Newest first.
    let newest = label_of(&file, 0);
    assert!(newest.starts_with("2 Oct 2026"), "{newest}");
    ui.get_by_label(&newest);
    ui.get_by_label(&label_of(&file, 1));

    click(&mut ui, &newest);
    restore(&mut ui);
    assert!(flow(&ui).problem.is_none(), "{:?}", flow(&ui).problem);
    assert_eq!(fs::read(&file).unwrap(), settings(22));
}

#[test]
fn a_file_with_no_backups_says_so() {
    let dir = profile();
    let ui = restore_screen(dir.path(), EveStatus::NotRunning, "3");
    ui.get_by_label(
        "There are no backups of this file. One is made the first time the app changes it.",
    );
    assert!(!flow(&ui).can_restore());
}

#[test]
fn a_backup_past_the_end_of_the_list_cannot_be_picked() {
    let dir = profile();
    let mut ui = restore_screen(dir.path(), EveStatus::NotRunning, "1");
    let Screen::Restore(flow) = &mut ui.state_mut().state.screen else {
        panic!("the restore screen should be open");
    };
    flow.select(1);
    assert_eq!(flow.selected, None);
    flow.select(0);
    assert_eq!(flow.selected, Some(0));
}

#[test]
fn the_file_list_is_read_again_only_after_a_restore() {
    let dir = profile();
    let file = dir.path().join("core_user_1.dat");
    let mut ui = restore_screen(dir.path(), EveStatus::NotRunning, "1");
    let listed = ui.state().state.rows.len();
    fs::write(dir.path().join("core_user_9.dat"), settings(9)).unwrap();

    click(&mut ui, "Back to the file list");
    assert_eq!(ui.state().state.rows.len(), listed);

    click(&mut ui, "Restore from a backup");
    click(&mut ui, &label_of(&file, 0));
    restore(&mut ui);
    click(&mut ui, "Back to the file list");
    assert_eq!(ui.state().state.rows.len(), listed + 1);
}

#[test]
fn nothing_is_restored_while_eve_is_running() {
    let dir = profile();
    let file = dir.path().join("core_user_1.dat");
    let mut ui = restore_screen(dir.path(), EveStatus::Running, "1");
    click(&mut ui, &label_of(&file, 0));
    restore(&mut ui);
    assert!(flow(&ui)
        .problem
        .as_deref()
        .unwrap()
        .starts_with("EVE is running"));
    // The check worked, so there is nothing for the user to vouch for.
    assert!(!flow(&ui).saving.needs_eve_confirmation);
    assert_eq!(fs::read(&file).unwrap(), settings(0));
    assert!(!dir.path().join("clone-bay-backups/T").exists());
}

#[test]
fn when_eve_cant_be_checked_the_user_vouches_that_it_is_closed() {
    let dir = profile();
    let file = dir.path().join("core_user_1.dat");
    let mut ui = restore_screen(dir.path(), EveStatus::Unknown, "1");
    click(&mut ui, &label_of(&file, 0));
    restore(&mut ui);
    assert_eq!(fs::read(&file).unwrap(), settings(0));
    // Restoring waits for the box to be ticked.
    assert!(!flow(&ui).can_restore());
    click(&mut ui, "I have closed every EVE client");
    assert!(flow(&ui).can_restore());
    restore(&mut ui);
    assert!(flow(&ui).problem.is_none(), "{:?}", flow(&ui).problem);
    assert_eq!(fs::read(&file).unwrap(), settings(1));
}
