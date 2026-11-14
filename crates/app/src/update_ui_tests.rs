//! The update notice and the tick box, driven through the real UI.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clone_bay_core::update_prefs::UpdatePrefs;

use crate::app::test_support::{click, closing, harness_with, settle, shows, wait_until, Ui};
use crate::services::test_support::{fake_updater, UpdaterLog};
use crate::services::Services;
use crate::state::test_support::eve_root;
use crate::state::Screen;

const TICK_BOX: &str = "Check for updates when Clone Bay opens";

/// The app running 0.1.4 with `offered` as the newest release, once the
/// check has finished.
fn app_offered(dir: &Path, root: &Path, offered: &str) -> (Ui, Arc<UpdaterLog>, PathBuf) {
    let (updater, _, log) = fake_updater(dir, offered);
    let exe = updater.exe.clone();
    let services = Services {
        updater: Some(updater),
        ..Services::for_tests()
    };
    let mut harness = harness_with(root, services);
    wait_until(&mut harness, update_settled);
    (harness, log, exe)
}

fn update_settled(app: &crate::app::App) -> bool {
    app.state
        .update
        .as_ref()
        .is_some_and(|update| !update.busy())
}

#[test]
fn the_status_bar_shows_the_version_and_the_tick_box() {
    let (dir, root) = (tempfile::tempdir().unwrap(), eve_root());
    let (harness, _, _) = app_offered(dir.path(), root.path(), "0.1.4");
    assert!(shows(&harness, "Version 0.1.4"));
    assert!(shows(&harness, TICK_BOX));
    assert!(!shows(&harness, "Update"));
}

#[test]
fn unticking_the_box_is_saved() {
    let (dir, root) = (tempfile::tempdir().unwrap(), eve_root());
    let (mut harness, _, _) = app_offered(dir.path(), root.path(), "0.1.4");
    click(&mut harness, TICK_BOX);
    let saved = UpdatePrefs::load(&dir.path().join("update.tsv")).unwrap();
    assert!(!saved.check);
}

#[test]
fn one_click_updates_and_closes_the_window() {
    let (dir, root) = (tempfile::tempdir().unwrap(), eve_root());
    let (mut harness, log, exe) = app_offered(dir.path(), root.path(), "0.1.5");
    assert!(shows(&harness, "Clone Bay 0.1.5 is available."));
    click(&mut harness, "Update");
    wait_until(&mut harness, |app| app.state.close_window);
    assert!(closing(&harness));
    assert_eq!(fs::read(&exe).unwrap(), b"new exe");
    assert_eq!(*log.started.lock().unwrap(), ["0.1.4"]);
}

#[test]
fn whats_new_opens_the_download_page() {
    let (dir, root) = (tempfile::tempdir().unwrap(), eve_root());
    let (mut harness, log, _) = app_offered(dir.path(), root.path(), "0.1.5");
    click(&mut harness, "What's new");
    assert_eq!(*log.pages_opened.lock().unwrap(), 1);
}

#[test]
fn a_failed_update_says_so_and_offers_the_download_page() {
    let (dir, root) = (tempfile::tempdir().unwrap(), eve_root());
    let (mut harness, log, exe) = app_offered(dir.path(), root.path(), "0.1.5");
    *log.self_test_fails.lock().unwrap() = true;
    click(&mut harness, "Update");
    wait_until(&mut harness, update_settled);
    assert!(shows(
        &harness,
        "Couldn't update: the new version didn't run properly when it was checked. \
         Clone Bay is unchanged."
    ));
    assert_eq!(fs::read(&exe).unwrap(), b"old");
    assert!(!harness.state().state.close_window);
    click(&mut harness, "Open the download page");
    assert_eq!(*log.pages_opened.lock().unwrap(), 1);
    click(&mut harness, "Dismiss");
    assert!(!shows(&harness, "Open the download page"));
}

#[test]
fn an_update_that_finishes_on_another_screen_waits_for_the_file_list() {
    let (dir, root) = (tempfile::tempdir().unwrap(), eve_root());
    let (mut harness, log, exe) = app_offered(dir.path(), root.path(), "0.1.5");
    // On another screen when the download starts, so it can't finish
    // while the file list is still showing (a click would race it).
    {
        let state = &mut harness.state_mut().state;
        let source = state.rows.iter().find(|row| row.entry.id == 11).unwrap();
        state.selected_file = Some(source.entry.path.clone());
        state.begin_copy();
        assert!(matches!(state.screen, Screen::Copy(_)));
        state.update.as_mut().unwrap().start_update();
    }
    wait_until(&mut harness, update_settled);
    // Downloaded and checked, but nothing swapped and the window stays.
    assert_eq!(fs::read(&exe).unwrap(), b"old");
    assert!(!harness.state().state.close_window);
    // Back on the file list the user restarts when ready.
    harness.state_mut().state.go_back();
    settle(&mut harness);
    assert!(shows(&harness, "Clone Bay 0.1.5 is ready."));
    click(&mut harness, "Restart to update");
    assert!(harness.state().state.close_window);
    assert_eq!(*log.started.lock().unwrap(), ["0.1.4"]);
}
