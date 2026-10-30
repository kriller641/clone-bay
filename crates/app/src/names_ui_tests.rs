//! Tests of character names and account labels in the real UI, off-screen.
//! The name service is always a stand-in; nothing here uses the network.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use egui_kittest::kittest::Queryable;

use crate::app::test_support::{click, harness_with, settle, shows, wait_until, Ui};
use crate::list_model::SortColumn;
use crate::services::test_support::FakeNames;
use crate::services::Services;
use crate::state::test_support::SMALL_SETTINGS;

/// Characters 101 and 102, a character nobody has heard of (103), and
/// account 5.
fn profile() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "core_char_101.dat",
        "core_char_102.dat",
        "core_char_103.dat",
        "core_user_5.dat",
    ] {
        fs::write(dir.path().join(name), SMALL_SETTINGS).unwrap();
    }
    dir
}

fn known() -> Arc<FakeNames> {
    FakeNames::knowing(&[(101, "Ann Able"), (102, "bob Baker")])
}

fn open(dir: &Path, names: &Arc<FakeNames>, names_file: Option<&Path>) -> Ui {
    let services = Services {
        name_source: Arc::clone(names) as _,
        names_file: names_file.map(Path::to_path_buf),
        ..Services::for_tests()
    };
    let mut ui = harness_with(dir, services);
    wait_until(&mut ui, |app| {
        app.state.names_note().as_deref() != Some("Looking up character names…")
    });
    ui
}

#[test]
fn character_names_are_looked_up_once_and_shown() {
    let dir = profile();
    let names = known();
    let ui = open(dir.path(), &names, None);
    ui.get_by_label("Ann Able");
    ui.get_by_label("bob Baker");
    ui.get_by_label("no label");
    assert_eq!(ui.state().state.names_note(), None);
    // One request, for the characters only: account IDs can't be looked up.
    assert_eq!(*names.requests.lock().unwrap(), [vec![101, 102, 103]]);
}

#[test]
fn a_character_nobody_knows_is_not_asked_about_again() {
    let dir = profile();
    let names = known();
    let mut ui = open(dir.path(), &names, None);
    click(&mut ui, "Rescan");
    wait_until(&mut ui, |app| app.state.names_note().is_none());
    assert_eq!(names.request_count(), 1);
}

#[test]
fn names_are_remembered_between_runs() {
    let dir = profile();
    let data = tempfile::tempdir().unwrap();
    let file = data.path().join("clone-bay").join("names.tsv");
    drop(open(dir.path(), &known(), Some(&file)));
    assert!(file.is_file(), "names were not saved");

    // A second run with no connection still shows them, without asking.
    let offline = FakeNames::offline();
    let ui = open(dir.path(), &offline, Some(&file));
    ui.get_by_label("Ann Able");
    ui.get_by_label("bob Baker");
    assert_eq!(offline.request_count(), 0);
    assert_eq!(ui.state().state.names_note(), None);
}

#[test]
fn without_a_connection_the_list_still_works_and_says_why_names_are_missing() {
    let dir = profile();
    let offline = FakeNames::offline();
    let mut ui = open(dir.path(), &offline, None);
    ui.get_by_label("Couldn't reach EVE's name service (offline)");
    ui.get_by_label("101");
    assert!(!shows(&ui, "Ann Able"));
    // It tries again the next time the files are listed.
    click(&mut ui, "Rescan");
    wait_until(&mut ui, |app| {
        app.state.names_note().as_deref() != Some("Looking up character names…")
    });
    assert_eq!(offline.request_count(), 2);
}

#[test]
fn searching_and_sorting_work_on_names() {
    let dir = profile();
    let mut ui = open(dir.path(), &known(), None);
    ui.state_mut().state.search = "BAKER".to_owned();
    settle(&mut ui);
    ui.get_by_label("bob Baker");
    assert!(!shows(&ui, "Ann Able"));
    assert!(!shows(&ui, "103"));

    ui.state_mut().state.search.clear();
    settle(&mut ui);
    click(&mut ui, "Name");
    let sort = ui.state().state.sort;
    assert_eq!((sort.column, sort.descending), (SortColumn::Name, false));
}

#[test]
fn an_account_can_be_given_a_label_which_is_saved() {
    let dir = profile();
    let data = tempfile::tempdir().unwrap();
    let file = data.path().join("names.tsv");
    let mut ui = open(dir.path(), &known(), Some(&file));

    // Only accounts have a label to edit.
    click(&mut ui, "101");
    assert!(!shows(&ui, "Label"));
    click(&mut ui, "5");
    ui.get_by_label("Label");

    let state = &mut ui.state_mut().state;
    *state.label_draft(5) = "Main account ".to_owned();
    state.commit_label_draft();
    settle(&mut ui);
    // Shown trimmed in the list and as the heading beside it, and no longer
    // "no label".
    assert_eq!(ui.query_all_by_label("Main account").count(), 2);
    assert!(!shows(&ui, "no label"));
    // The box still holds exactly what was typed, trailing space and all.
    assert_eq!(ui.state_mut().state.label_draft(5), "Main account ");

    drop(ui);
    let ui = open(dir.path(), &FakeNames::offline(), Some(&file));
    ui.get_by_label("Main account");
}

#[test]
fn the_label_box_follows_the_selected_account() {
    let dir = profile();
    fs::write(dir.path().join("core_user_6.dat"), SMALL_SETTINGS).unwrap();
    let mut ui = open(dir.path(), &known(), None);
    let state = &mut ui.state_mut().state;
    *state.label_draft(5) = "Five".to_owned();
    state.commit_label_draft();
    // Another account starts from its own saved label (none), not the
    // text left over from the last one.
    assert_eq!(state.label_draft(6), "");
    assert_eq!(state.label_draft(5), "Five");
    // Clearing the box removes the label.
    state.label_draft(5).clear();
    state.commit_label_draft();
    assert_eq!(state.names.account_label(5), None);
}

#[test]
fn the_copy_screens_show_names() {
    let dir = profile();
    let mut ui = open(dir.path(), &known(), None);
    click(&mut ui, "101");
    click(&mut ui, "Copy to other files");
    ui.get_by_label("Copy settings from Ann Able (character 101)");
    ui.get_by_label("bob Baker");

    // Searching by name narrows the choice.
    let flow = ui.state_mut().state.copy_mut().unwrap();
    flow.search = "bob".to_owned();
    assert_eq!(flow.visible().len(), 1);
    settle(&mut ui);
    assert!(!shows(&ui, "103"));

    click(&mut ui, "Tick all shown");
    click(&mut ui, "Check files");
    wait_until(&mut ui, |app| {
        app.state.copy().is_some_and(|flow| !flow.busy())
    });
    click(&mut ui, "Show every file");
    ui.get_by_label("core_char_102.dat");
    ui.get_by_label("bob Baker");
}

#[test]
fn a_saved_names_file_that_cannot_be_read_is_reported_not_fatal() {
    let dir = profile();
    let data = tempfile::tempdir().unwrap();
    // A folder where the file should be.
    let ui = open(dir.path(), &FakeNames::offline(), Some(data.path()));
    assert!(ui.state().state.rows.len() == 4);
    let note = ui.state().state.names_note().unwrap();
    assert!(note.starts_with("Couldn't re"), "{note}");
}

#[test]
fn a_file_can_be_marked_as_main_and_changing_it_asks_first() {
    use clone_bay_core::discovery::FileKind::{Account, Character};
    let dir = profile();
    let data = tempfile::tempdir().unwrap();
    let file = data.path().join("names.tsv");
    let mut ui = open(dir.path(), &known(), Some(&file));
    let main = |ui: &Ui, kind| ui.state().state.main_of(kind);
    assert_eq!(main(&ui, Character), None);

    // With no main yet, marking one happens at once and is saved.
    click(&mut ui, "102");
    click(&mut ui, "Mark as main");
    assert_eq!(main(&ui, Character), Some(102));
    assert_eq!(main(&ui, Account), None);
    assert_eq!(ui.state().state.main_prompt_for_selected(), None);
    ui.get_by_label("bob Baker (Main)");
    assert!(!shows(&ui, "Yes"));
    let saved = fs::read_to_string(&file).unwrap();
    assert!(saved.contains("main-character\t102\n"), "{saved}");

    // Marking another asks first, and "No" leaves things as they were.
    click(&mut ui, "101");
    click(&mut ui, "Mark as main");
    assert_eq!(main(&ui, Character), Some(102));
    assert_eq!(
        ui.state().state.main_prompt_for_selected(),
        Some((Character, 101, true))
    );
    ui.get_by_label("bob Baker (Main) is your main character now. Make this one the main instead?");
    click(&mut ui, "No");
    assert_eq!(main(&ui, Character), Some(102));
    assert!(!shows(&ui, "Yes"));

    // The question belongs to the file it was asked about.
    click(&mut ui, "Mark as main");
    click(&mut ui, "103");
    assert_eq!(ui.state().state.main_prompt_for_selected(), None);
    assert!(!shows(&ui, "Yes"));
    click(&mut ui, "101");
    click(&mut ui, "Yes");
    assert_eq!(main(&ui, Character), Some(101));
    ui.get_by_label("Ann Able (Main)");
    assert!(!shows(&ui, "bob Baker (Main)"));
    let saved = fs::read_to_string(&file).unwrap();
    assert!(saved.contains("main-character\t101\n"), "{saved}");

    // Unmarking the main asks too.
    click(&mut ui, "Mark as main");
    ui.get_by_label("Stop marking this as your main character?");
    assert_eq!(
        ui.state().state.main_prompt_for_selected(),
        Some((Character, 101, false))
    );
    assert_eq!(main(&ui, Character), Some(101));
    click(&mut ui, "Yes");
    assert_eq!(main(&ui, Character), None);
    let saved = fs::read_to_string(&file).unwrap();
    assert!(!saved.contains("main-character"), "{saved}");

    // Accounts have a main of their own.
    click(&mut ui, "5");
    click(&mut ui, "Mark as main");
    assert_eq!(main(&ui, Account), Some(5));
    assert_eq!(main(&ui, Character), None);
    ui.get_by_label("5 (Main)");
    let saved = fs::read_to_string(&file).unwrap();
    assert!(saved.contains("main-account\t5\n"), "{saved}");
}
