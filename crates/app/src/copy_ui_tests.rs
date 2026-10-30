//! Tests that click through the copy screens in the real UI, off-screen,
//! against settings files in a temp folder.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use clone_bay_core::platform::EveStatus;
use clone_bay_marshal::{decode, encode, Long, Value};
use eframe::egui;
use egui_kittest::kittest::{NodeT, Queryable};

use crate::app::test_support::{click, harness_with, settle, shows, wait_until, Ui};
use crate::copy_flow::Stage;
use crate::copy_view::{action_text, outcome_text};
use crate::services::Services;

fn b(text: &str) -> Value {
    Value::Bytes(text.as_bytes().to_vec())
}

fn stamped(stamp: i64, value: Value) -> Value {
    Value::Tuple(vec![Value::Long(Long::from_i64(stamp)), value])
}

/// An account file whose settings and rack layout are both marked with `tag`.
fn account(tag: i64, ships: &[i64]) -> Value {
    let slots = Value::List((11..35).map(Value::Int).collect());
    let ships = ships
        .iter()
        .map(|&ship| (Value::Long(Long::from_i64(ship)), slots.clone()))
        .collect();
    Value::Dict(vec![(
        b("ui"),
        Value::Dict(vec![
            (b("defaultwarpto"), stamped(tag, Value::Int(tag))),
            (b("slotOrder"), stamped(tag, Value::Dict(ships))),
        ]),
    )])
}

/// Accounts 1 (one ship), 2 (two ships) and 4 (three ships), a damaged
/// account 3, and a character file.
fn profile() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let put = |name: &str, value: &Value| {
        fs::write(dir.path().join(name), encode(value, false).unwrap()).unwrap();
    };
    put("core_user_1.dat", &account(1, &[10]));
    put("core_user_2.dat", &account(2, &[20, 21]));
    put("core_user_4.dat", &account(4, &[40, 41, 42]));
    fs::write(dir.path().join("core_user_3.dat"), b"damaged").unwrap();
    put(
        "core_char_9.dat",
        &Value::Dict(vec![(b("ui"), Value::Dict(vec![]))]),
    );
    dir
}

fn services(eve: EveStatus) -> Services {
    Services {
        eve_status: Arc::new(move || eve),
        ..Services::for_tests()
    }
}

/// The app with account 1 selected and the copy screens open.
fn copy_screen(dir: &Path, eve: EveStatus) -> Ui {
    let mut ui = harness_with(dir, services(eve));
    click(&mut ui, "1");
    click(&mut ui, "Copy to other files");
    assert!(ui.state().state.copy().is_some());
    ui
}

fn flow(ui: &Ui) -> &crate::copy_flow::CopyFlow {
    ui.state().state.copy().unwrap()
}

fn stage_is(ui: &Ui, want: fn(&Stage) -> bool) -> bool {
    ui.state()
        .state
        .copy()
        .is_some_and(|flow| want(&flow.stage))
}

fn check_files(ui: &mut Ui) {
    click(ui, "Check files");
    wait_until(ui, |app| app.state.copy().is_some_and(|flow| !flow.busy()));
}

fn copy_now(ui: &mut Ui) {
    click(ui, "Copy now");
    wait_until(ui, |app| app.state.copy().is_some_and(|flow| !flow.busy()));
}

/// The `defaultwarpto` and number of ships now stored in an account file.
fn contents(dir: &Path, name: &str) -> (i64, usize) {
    let root = decode(&fs::read(dir.join(name)).unwrap()).unwrap().value;
    let Value::Dict(sections) = root else {
        panic!("not a dict");
    };
    let Value::Dict(ui) = &sections[0].1 else {
        panic!("ui isn't a dict");
    };
    let setting = |name: &str| {
        let (_, value) = ui.iter().find(|(key, _)| *key == b(name)).unwrap();
        let Value::Tuple(items) = value else {
            panic!("not a (timestamp, value) pair");
        };
        items[1].clone()
    };
    let Value::Int(warp) = setting("defaultwarpto") else {
        panic!("defaultwarpto isn't an int");
    };
    let Value::Dict(ships) = setting("slotOrder") else {
        panic!("slotOrder isn't a dict");
    };
    (warp, ships.len())
}

#[test]
fn the_whole_copy_flow_from_picking_to_result() {
    let dir = profile();
    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);

    // Picking: the other accounts are offered, the damaged one can't be ticked.
    ui.get_by_label("Copy settings from account 1");
    ui.get_by_label(
        "Choose what to copy and which files to copy it onto. Each file always keeps its own \
         module rack layout, weapon groups and drone groups.",
    );
    ui.get_by_label("0 files ticked");
    // The damaged account is marked. Why is in the marker's tooltip.
    ui.get_by_label("Can't use");
    ui.get_by_label("1 Choose");
    assert!(
        !shows(&ui, "9"),
        "a character file was offered for an account copy"
    );
    click(&mut ui, "3");
    ui.get_by_label("0 files ticked");
    assert!(ui
        .get_by_label("Check files")
        .accesskit_node()
        .is_disabled());
    click(&mut ui, "Check files");
    assert!(
        stage_is(&ui, |stage| matches!(stage, Stage::Picking)),
        "nothing is ticked yet"
    );

    click(&mut ui, "2");
    ui.get_by_label("1 file ticked");
    click(&mut ui, "4");
    ui.get_by_label("2 files ticked");

    // Reviewing: says what will happen, and nothing has changed yet.
    assert!(!ui
        .get_by_label("Check files")
        .accesskit_node()
        .is_disabled());
    check_files(&mut ui);
    assert_eq!(flow(&ui).progress(), (2, 2));
    ui.get_by_label("2 files will be overwritten.");
    ui.get_by_label("Nothing has been changed yet.");
    // Nothing will be skipped, so no file is listed until they are asked for.
    ui.get_by_label("No problems found: every file ticked will be copied onto.");
    assert!(!shows(&ui, "core_user_2.dat") && !shows(&ui, "core_user_4.dat"));
    assert!(!shows(
        &ui,
        "Only the files that will be skipped are listed."
    ));
    click(&mut ui, "Show every file");
    ui.get_by_label("core_user_2.dat");
    ui.get_by_label("core_user_4.dat");
    ui.get_by_label("Copy, keeping its own module rack layout for 2 ships");
    ui.get_by_label("Copy, keeping its own module rack layout for 3 ships");
    assert_eq!(contents(dir.path(), "core_user_2.dat"), (2, 2));
    assert!(!dir.path().join("clone-bay-backups").join("T").exists());

    // Done: the settings came from account 1, the layouts stayed put.
    copy_now(&mut ui);
    assert_eq!(flow(&ui).progress(), (2, 2));
    ui.get_by_label("Copied to 2 files.");
    // "Show every file" is still ticked from the review.
    ui.get_by_label("core_user_2.dat");
    ui.get_by_label("The originals were saved in:");
    ui.get_by_label(
        dir.path()
            .join("clone-bay-backups")
            .join("T")
            .display()
            .to_string()
            .as_str(),
    );
    assert_eq!(ui.get_all_by_label("Copied").count(), 2);
    assert_eq!(contents(dir.path(), "core_user_2.dat"), (1, 2));
    assert_eq!(contents(dir.path(), "core_user_4.dat"), (1, 3));
    assert_eq!(contents(dir.path(), "core_user_1.dat"), (1, 1));
    assert_eq!(
        contents(
            &dir.path().join("clone-bay-backups").join("T"),
            "core_user_2.dat"
        ),
        (2, 2)
    );
    assert_eq!(
        contents(
            &dir.path().join("clone-bay-backups").join("T"),
            "core_user_4.dat"
        ),
        (4, 3)
    );
    assert_eq!(
        fs::read(dir.path().join("core_user_3.dat")).unwrap(),
        b"damaged"
    );

    // Back at the list, which has been refreshed.
    click(&mut ui, "Back to the file list");
    assert!(ui.state().state.copy().is_none());
    wait_until(&mut ui, |app| {
        app.state
            .rows
            .iter()
            .all(|row| row.status != crate::list_model::Status::Pending)
    });
    ui.get_by_label("5 files, 1 can't be used");
    // The list shows the files as they are now, not as they were.
    let listed = |id: u64| {
        let row = ui.state().state.rows.iter().find(|row| row.entry.id == id);
        row.unwrap().entry.size
    };
    let on_disk = |name: &str| fs::metadata(dir.path().join(name)).unwrap().len();
    assert_eq!(listed(2), on_disk("core_user_2.dat"));
    assert_eq!(listed(4), on_disk("core_user_4.dat"));
}

#[test]
fn tick_all_only_ticks_files_that_can_be_used_and_respects_the_search() {
    let dir = profile();
    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "Tick all shown");
    ui.get_by_label("2 files ticked");
    click(&mut ui, "Untick all shown");
    ui.get_by_label("0 files ticked");

    ui.state_mut().state.copy_mut().unwrap().search = "4".to_owned();
    settle(&mut ui);
    assert!(!shows(&ui, "2"));
    click(&mut ui, "Tick all shown");
    ui.get_by_label("1 file ticked");
    let flow = ui.state().state.copy().unwrap();
    let ticked: Vec<u64> = flow
        .candidates
        .iter()
        .filter(|candidate| candidate.ticked)
        .map(|candidate| candidate.entry.id)
        .collect();
    assert_eq!(ticked, [4]);
}

#[test]
fn nothing_is_copied_while_eve_is_running() {
    let dir = profile();
    let mut ui = copy_screen(dir.path(), EveStatus::Running);
    click(&mut ui, "2");
    check_files(&mut ui);
    copy_now(&mut ui);

    // Still on the review screen, with the reason, and nothing touched.
    assert!(stage_is(&ui, |stage| matches!(stage, Stage::Reviewing(_))));
    ui.get_by_label(
        "EVE is running. Close every EVE client first: the client saves its settings when it \
         closes and would overwrite the result. Nothing was changed.",
    );
    assert!(!shows(&ui, "I have closed every EVE client"));
    assert_eq!(contents(dir.path(), "core_user_2.dat"), (2, 2));
    assert!(!dir.path().join("clone-bay-backups").join("T").exists());
}

#[test]
fn when_eve_cannot_be_checked_the_user_must_confirm_it_is_closed() {
    let dir = profile();
    let mut ui = copy_screen(dir.path(), EveStatus::Unknown);
    click(&mut ui, "2");
    check_files(&mut ui);
    copy_now(&mut ui);
    assert!(stage_is(&ui, |stage| matches!(stage, Stage::Reviewing(_))));
    assert_eq!(contents(dir.path(), "core_user_2.dat"), (2, 2));

    // "Copy now" does nothing until the box is ticked.
    assert!(!flow(&ui).can_copy());
    assert!(ui.get_by_label("Copy now").accesskit_node().is_disabled());
    click(&mut ui, "Copy now");
    assert!(stage_is(&ui, |stage| matches!(stage, Stage::Reviewing(_))));
    assert_eq!(contents(dir.path(), "core_user_2.dat"), (2, 2));

    click(&mut ui, "I have closed every EVE client");
    assert!(flow(&ui).can_copy());
    copy_now(&mut ui);
    ui.get_by_label("Copied to 1 file.");
    assert_eq!(contents(dir.path(), "core_user_2.dat"), (1, 2));
}

#[test]
fn back_and_cancel_change_nothing() {
    let dir = profile();
    let before = fs::read(dir.path().join("core_user_2.dat")).unwrap();
    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "2");
    check_files(&mut ui);

    click(&mut ui, "Back");
    assert!(stage_is(&ui, |stage| matches!(stage, Stage::Picking)));
    // The ticks are remembered.
    ui.get_by_label("1 file ticked");

    check_files(&mut ui);
    // A file that appears meanwhile isn't listed: cancelling changed
    // nothing, so the list isn't refreshed.
    fs::write(dir.path().join("core_user_7.dat"), b"new").unwrap();
    click(&mut ui, "Cancel");
    assert!(ui.state().state.copy().is_none());
    assert_eq!(ui.state().state.rows.len(), 5);
    fs::remove_file(dir.path().join("core_user_7.dat")).unwrap();
    ui.get_by_label("Rescan");
    assert_eq!(
        fs::read(dir.path().join("core_user_2.dat")).unwrap(),
        before
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 5);
}

#[test]
fn a_file_that_changes_during_review_stops_the_copy() {
    let dir = profile();
    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "Tick all shown");
    check_files(&mut ui);
    // As if EVE saved account 4 while the review was on screen.
    let changed = encode(&account(44, &[1, 2, 3, 4]), false).unwrap();
    fs::write(dir.path().join("core_user_4.dat"), changed).unwrap();

    copy_now(&mut ui);
    ui.get_by_label(
        "Backup failed: core_user_4.dat changed after it was checked, or its backup doesn't \
         match (is EVE running?). Nothing was changed.",
    );
    assert_eq!(contents(dir.path(), "core_user_2.dat"), (2, 2));
    assert_eq!(contents(dir.path(), "core_user_4.dat"), (44, 4));
}

#[test]
fn files_that_cannot_be_written_are_shown_as_skipped_before_anything_happens() {
    let dir = profile();
    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "Tick all shown");
    // Account 4 is damaged after it was listed as fine.
    fs::write(dir.path().join("core_user_4.dat"), b"damaged").unwrap();
    check_files(&mut ui);
    ui.get_by_label("1 file will be overwritten, 1 skipped.");
    ui.get_by_label("Skipped: can't decode it (not a marshal stream (bad signature))");
    assert!(flow(&ui).can_copy());

    // With account 2 damaged as well, nothing is left to do.
    click(&mut ui, "Back");
    fs::write(dir.path().join("core_user_2.dat"), b"damaged").unwrap();
    check_files(&mut ui);
    ui.get_by_label("None of the chosen files can be overwritten, 2 skipped.");
    assert!(!flow(&ui).can_copy());
    assert!(ui.get_by_label("Copy now").accesskit_node().is_disabled());
    click(&mut ui, "Copy now");
    assert!(stage_is(&ui, |stage| matches!(stage, Stage::Reviewing(_))));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 5);
}

#[test]
fn steps_cannot_be_started_out_of_turn() {
    let dir = profile();
    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);
    let flow_mut = |ui: &mut Ui| ui.state_mut().state.copy_mut().unwrap().start_plan();

    // Nothing ticked: there is nothing to check.
    flow_mut(&mut ui);
    assert!(stage_is(&ui, |stage| matches!(stage, Stage::Picking)));
    // Copying can't start before a review.
    ui.state_mut().state.copy_mut().unwrap().start_copy();
    assert!(stage_is(&ui, |stage| matches!(stage, Stage::Picking)));

    // Once reviewing, asking for a new check does nothing.
    click(&mut ui, "2");
    check_files(&mut ui);
    flow_mut(&mut ui);
    assert!(stage_is(&ui, |stage| matches!(stage, Stage::Reviewing(_))));
    assert!(!flow(&ui).busy());
}

#[test]
fn a_damaged_file_cannot_be_copied_from() {
    let dir = profile();
    let mut ui = harness_with(dir.path(), services(EveStatus::NotRunning));
    click(&mut ui, "3");
    click(&mut ui, "Copy to other files");
    assert!(ui.state().state.copy().is_none());
}

#[test]
fn a_file_with_no_others_of_its_kind_has_nothing_to_copy_onto() {
    let dir = profile();
    let mut ui = harness_with(dir.path(), services(EveStatus::NotRunning));
    click(&mut ui, "9");
    click(&mut ui, "Copy to other files");
    ui.get_by_label("Copy settings from character 9");
    ui.get_by_label("There are no other files of this kind in this profile.");
    assert!(!shows(&ui, "Check files"));
    click(&mut ui, "Cancel");
    assert!(ui.state().state.copy().is_none());
}

#[test]
fn every_action_and_outcome_has_its_own_wording() {
    use clone_bay_core::copy::{Action, Outcome};
    use clone_bay_core::discovery::FileKind;

    assert_eq!(
        action_text(&Action::ExactCopy, FileKind::Account),
        "Exact copy"
    );
    let none = Action::KeepOwn { kept: vec![] };
    assert_eq!(
        action_text(&none, FileKind::Account),
        "Copy. It has no module rack layout, weapon groups and drone groups stored, so it gets \
         none"
    );
    assert_eq!(
        action_text(&none, FileKind::Character),
        "Copy. It has no auto-repeat and auto-reload settings and chat channels stored, so it gets \
         none"
    );
    let some = Action::KeepOwn {
        kept: vec!["a".to_owned(), "b".to_owned()],
    };
    assert_eq!(
        action_text(&some, FileKind::Account),
        "Copy, keeping its own a, b"
    );
    let skip = Action::Skip {
        reason: "the file is read-only".to_owned(),
    };
    assert_eq!(
        action_text(&skip, FileKind::Account),
        "Skipped: the file is read-only"
    );

    assert_eq!(outcome_text(&Outcome::Written), "Copied");
    assert_eq!(outcome_text(&Outcome::Skipped("x".into())), "Skipped: x");
    assert_eq!(outcome_text(&Outcome::Failed("x".into())), "NOT copied: x");
}

/// An account file with keyboard shortcuts, sound and an ungrouped setting,
/// all marked with `tag`.
fn grouped_account(tag: i64) -> Value {
    let section = |key: &str| Value::Dict(vec![(b(key), stamped(tag, Value::Int(tag)))]);
    Value::Dict(vec![
        (b("cmd"), section("customCmds")),
        (b("audio"), section("volume")),
        (b("notifications"), section("n")),
        (b("ui"), section("slotOrder")),
    ])
}

/// The tag stored under `section` in an account file made by
/// [`grouped_account`].
fn tag_in(dir: &Path, name: &str, section: &str) -> i64 {
    let root = decode(&fs::read(dir.join(name)).unwrap()).unwrap().value;
    let Value::Dict(sections) = root else {
        panic!("not a dict");
    };
    let (_, value) = sections.iter().find(|(key, _)| *key == b(section)).unwrap();
    let Value::Dict(inner) = value else {
        panic!("section isn't a dict");
    };
    let Value::Tuple(pair) = &inner[0].1 else {
        panic!("not stamped");
    };
    match pair[1] {
        Value::Int(tag) => tag,
        _ => panic!("no tag"),
    }
}

/// Unticks every box under "Only what I tick" by clicking it. They all start
/// ticked, so a test that wants a few chosen clears them first.
fn untick_all(ui: &mut Ui) {
    let scope = &flow(ui).scope;
    assert!(!scope.everything, "pick \"Only what I tick\" first");
    let mut labels: Vec<String> = scope
        .groups
        .iter()
        .filter(|(_, ticked)| *ticked)
        .map(|(group, _)| group.label.to_owned())
        .collect();
    if scope.chat.as_ref().is_some_and(|chat| chat.ticked) {
        labels.push("Chat channels".to_owned());
    }
    if scope.everything_else {
        labels.push("Everything else".to_owned());
    }
    for label in labels {
        click(ui, &label);
    }
    assert_eq!(flow(ui).scope.summary(), "");
}

fn grouped_profile() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for tag in [1, 2] {
        let bytes = encode(&grouped_account(tag), false).unwrap();
        fs::write(dir.path().join(format!("core_user_{tag}.dat")), bytes).unwrap();
    }
    dir
}

#[test]
fn only_the_ticked_groups_are_copied() {
    let dir = grouped_profile();
    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "2");

    // "Everything" is the default; the groups appear once it is turned off.
    assert!(!shows(&ui, "Keyboard shortcuts"));
    click(&mut ui, "Only what I tick");
    ui.get_by_label("Keyboard shortcuts");
    ui.get_by_label("Sound");
    ui.get_by_label("Everything else");
    assert!(
        !shows(&ui, "Chat channels"),
        "accounts have no chat channels"
    );
    assert!(!shows(&ui, "Window layout"), "that is a character group");
    // Every box starts ticked, so the user unticks what they don't want.
    assert!(flow(&ui).can_check());
    assert!(flow(&ui).scope.groups.iter().all(|(_, ticked)| *ticked));
    assert!(flow(&ui).scope.everything_else);

    // With nothing chosen there is nothing to check.
    untick_all(&mut ui);
    ui.get_by_label("Nothing is chosen to copy.");
    assert!(!flow(&ui).can_check());
    assert!(ui
        .get_by_label("Check files")
        .accesskit_node()
        .is_disabled());

    click(&mut ui, "Keyboard shortcuts");
    assert!(!shows(&ui, "Nothing is chosen to copy."));
    check_files(&mut ui);
    ui.get_by_label("1 file will be overwritten.");
    ui.get_by_label("Copying: keyboard shortcuts.");
    click(&mut ui, "Show every file");
    ui.get_by_label("Copy the chosen settings");

    copy_now(&mut ui);
    ui.get_by_label("Copied to 1 file.");
    assert_eq!(
        tag_in(dir.path(), "core_user_2.dat", "cmd"),
        1,
        "shortcuts came across"
    );
    assert_eq!(
        tag_in(dir.path(), "core_user_2.dat", "audio"),
        2,
        "sound stayed"
    );
    assert_eq!(
        tag_in(dir.path(), "core_user_2.dat", "notifications"),
        2,
        "the rest stayed"
    );
    assert_eq!(
        tag_in(dir.path(), "core_user_2.dat", "ui"),
        2,
        "the rack layout stayed"
    );
}

#[test]
fn everything_else_and_several_groups_can_be_combined() {
    let dir = grouped_profile();
    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "2");
    click(&mut ui, "Only what I tick");
    untick_all(&mut ui);
    click(&mut ui, "Sound");
    click(&mut ui, "Everything else");
    check_files(&mut ui);
    ui.get_by_label("Copying: sound, everything else.");
    copy_now(&mut ui);
    assert_eq!(tag_in(dir.path(), "core_user_2.dat", "cmd"), 2);
    assert_eq!(tag_in(dir.path(), "core_user_2.dat", "audio"), 1);
    assert_eq!(tag_in(dir.path(), "core_user_2.dat", "notifications"), 1);
    assert_eq!(tag_in(dir.path(), "core_user_2.dat", "ui"), 2);
}

#[test]
fn a_file_that_already_has_the_chosen_settings_is_skipped() {
    let dir = grouped_profile();
    // Account 3 has the same shortcuts as account 1 but different sound.
    let mut same = grouped_account(3);
    let Value::Dict(sections) = &mut same else {
        unreachable!()
    };
    let shortcuts = Value::Dict(vec![(b("customCmds"), stamped(1, Value::Int(1)))]);
    sections[0] = (b("cmd"), shortcuts);
    fs::write(
        dir.path().join("core_user_3.dat"),
        encode(&same, false).unwrap(),
    )
    .unwrap();
    let before = fs::read(dir.path().join("core_user_3.dat")).unwrap();

    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "Tick all shown");
    click(&mut ui, "Only what I tick");
    untick_all(&mut ui);
    click(&mut ui, "Keyboard shortcuts");
    check_files(&mut ui);
    ui.get_by_label("1 file will be overwritten, 1 skipped.");
    // The skipped file is listed without asking; the other one is not.
    ui.get_by_label("Skipped: it already has these settings");
    ui.get_by_label("Only the files that will be skipped are listed.");
    ui.get_by_label("core_user_3.dat");
    assert!(!shows(&ui, "core_user_2.dat"));
    assert!(!shows(&ui, "Copy the chosen settings"));
    assert!(!flow(&ui).show_every_file);
    click(&mut ui, "Show every file");
    ui.get_by_label("core_user_2.dat");
    ui.get_by_label("Copy the chosen settings");
    ui.get_by_label("Skipped: it already has these settings");
    assert!(!shows(
        &ui,
        "Only the files that will be skipped are listed."
    ));
    copy_now(&mut ui);
    assert_eq!(
        fs::read(dir.path().join("core_user_3.dat")).unwrap(),
        before
    );
    assert_eq!(tag_in(dir.path(), "core_user_2.dat", "cmd"), 1);
}

fn character_with_channels(tag: i64, channels: &[(&str, &str, &str)]) -> Value {
    let channels = channels
        .iter()
        .map(|(kind, id, name)| {
            Value::Tuple(vec![
                b(kind),
                Value::Str((*id).into()),
                Value::Str((*name).into()),
            ])
        })
        .collect();
    Value::Dict(vec![
        (
            b("windows"),
            Value::Dict(vec![(b("open"), stamped(tag, Value::Int(tag)))]),
        ),
        (
            b("ui"),
            Value::Dict(vec![(
                b("chatchannels"),
                stamped(tag, Value::List(channels)),
            )]),
        ),
    ])
}

fn channel_names(dir: &Path, name: &str) -> Vec<String> {
    let root = decode(&fs::read(dir.join(name)).unwrap()).unwrap().value;
    clone_bay_core::groups::chat_channels(&root)
        .unwrap()
        .into_iter()
        .map(|channel| channel.name)
        .collect()
}

#[test]
fn chosen_chat_channels_are_added_to_a_character() {
    let dir = tempfile::tempdir().unwrap();
    let put = |name: &str, value: &Value| {
        fs::write(dir.path().join(name), encode(value, false).unwrap()).unwrap();
    };
    let source = [
        ("corp", "corp_1", "Corp"),
        ("local", "local_1", "Local"),
        ("player_a", "player_a", "Traders"),
        ("player_b", "player_b", "Scouts"),
        ("player_c", "player_c", "Haulers"),
    ];
    put("core_char_1.dat", &character_with_channels(1, &source));
    let target = [
        ("corp", "corp_2", "Corp"),
        ("player_b", "player_b", "Scouts"),
    ];
    put("core_char_2.dat", &character_with_channels(2, &target));

    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "2");
    click(&mut ui, "Only what I tick");
    // Chat starts ticked, with its channels listed. The list is only open
    // while chat is ticked, and never offers the source's own corp or local.
    ui.get_by_label("Traders");
    untick_all(&mut ui);
    assert!(!shows(&ui, "Traders"));
    click(&mut ui, "Chat channels");
    ui.get_by_label("Traders");
    ui.get_by_label("Scouts");
    ui.get_by_label("Haulers");
    assert!(!shows(&ui, "Corp") && !shows(&ui, "Local"));
    // Chat ticked with no channel chosen is the same as choosing nothing.
    let tick_all = |ui: &mut Ui, on: bool| {
        let flow = ui.state_mut().state.copy_mut().unwrap();
        let channels = &mut flow.scope.chat.as_mut().unwrap().channels;
        channels.iter_mut().for_each(|(_, ticked)| *ticked = on);
    };
    tick_all(&mut ui, false);
    assert_eq!(flow(&ui).scope.summary(), "");
    assert!(!flow(&ui).can_check());
    tick_all(&mut ui, true);
    settle(&mut ui);
    // Every channel starts ticked; leave one out.
    click(&mut ui, "Haulers");

    check_files(&mut ui);
    ui.get_by_label("Copying: 2 chat channels.");
    click(&mut ui, "Show every file");
    ui.get_by_label("Copy the chosen settings, adding 1 chat channel");
    copy_now(&mut ui);
    // Its own channels first and untouched, then the one it lacked.
    assert_eq!(
        channel_names(dir.path(), "core_char_2.dat"),
        ["Corp", "Scouts", "Traders"]
    );
    assert_eq!(
        channel_names(dir.path(), "core_char_1.dat").len(),
        5,
        "source untouched"
    );
}

#[test]
fn a_character_with_no_player_channels_cannot_have_chat_ticked() {
    let dir = tempfile::tempdir().unwrap();
    for tag in [1, 2] {
        let value = character_with_channels(tag, &[("corp", "corp_1", "Corp")]);
        let bytes = encode(&value, false).unwrap();
        fs::write(dir.path().join(format!("core_char_{tag}.dat")), bytes).unwrap();
    }
    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "Only what I tick");
    ui.get_by_label("This character has no player channels to copy.");
    assert!(ui
        .get_by_label("Chat channels")
        .accesskit_node()
        .is_disabled());
    ui.get_by_label("Window layout");
}

#[test]
fn the_chosen_scope_is_described_in_a_few_words() {
    let dir = grouped_profile();
    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);
    assert_eq!(flow(&ui).scope.summary(), "everything");
    assert!(flow(&ui).scope.chooses_something());
    let scope = &mut ui.state_mut().state.copy_mut().unwrap().scope;
    scope.everything = false;
    // Every box starts ticked.
    assert_eq!(
        scope.summary(),
        "overview, keyboard shortcuts, probe scanning, \"don't show again\" choices, sound, \
         camera and graphics, market, tab groups, everything else"
    );
    assert!(scope.chooses_something());
    scope
        .groups
        .iter_mut()
        .for_each(|(_, ticked)| *ticked = false);
    scope.everything_else = false;
    assert_eq!(scope.summary(), "");
    assert!(!scope.chooses_something());
    scope.groups[0].1 = true;
    scope.groups[1].1 = true;
    assert_eq!(scope.summary(), "overview, keyboard shortcuts");
    scope.everything_else = true;
    assert_eq!(
        scope.summary(),
        "overview, keyboard shortcuts, everything else"
    );
    assert!(scope.chooses_something());
}

/// The IDs of the files to copy onto, in the order the table lists them.
fn listed_ids(ui: &Ui) -> Vec<u64> {
    let flow = flow(ui);
    flow.visible()
        .iter()
        .map(|&index| flow.candidates[index].entry.id)
        .collect()
}

#[test]
fn the_files_to_copy_onto_can_be_sorted_by_clicking_a_column() {
    let dir = profile();
    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);
    assert_eq!(flow(&ui).sort, crate::list_model::Sort::default());

    click(&mut ui, "ID");
    assert_eq!(listed_ids(&ui), [2, 3, 4]);
    click(&mut ui, "ID \u{2B06}");
    assert_eq!(listed_ids(&ui), [4, 3, 2]);
    ui.get_by_label("ID \u{2B07}");

    // Account 4 holds the most ships, so it is the biggest; the damaged
    // account 3 is the smallest.
    click(&mut ui, "Size");
    assert_eq!(listed_ids(&ui), [3, 2, 4]);
}

#[test]
fn ticks_stay_with_their_files_when_the_list_is_sorted_again() {
    let dir = profile();
    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "ID");
    click(&mut ui, "4");
    click(&mut ui, "ID \u{2B06}");
    assert_eq!(listed_ids(&ui), [4, 3, 2]);
    let ticked: Vec<u64> = flow(&ui)
        .candidates
        .iter()
        .filter(|candidate| candidate.ticked)
        .map(|candidate| candidate.entry.id)
        .collect();
    assert_eq!(ticked, [4]);

    // Clicking the row again unticks it.
    click(&mut ui, "4");
    ui.get_by_label("0 files ticked");
}

#[test]
fn sorting_by_name_puts_files_without_a_label_last() {
    let dir = profile();
    let mut ui = harness_with(dir.path(), services(EveStatus::NotRunning));
    {
        let state = &mut ui.state_mut().state;
        *state.label_draft(4) = "Alpha".to_owned();
        state.commit_label_draft();
        *state.label_draft(2) = "beta".to_owned();
        state.commit_label_draft();
    }
    click(&mut ui, "1");
    click(&mut ui, "Copy to other files");
    click(&mut ui, "Name");
    assert_eq!(listed_ids(&ui), [4, 2, 3]);
    ui.get_by_label("Alpha");
    ui.get_by_label("no label");
}

#[test]
fn each_copy_screen_shows_which_step_it_is() {
    let dir = profile();
    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);
    assert_eq!(flow(&ui).stage.step(), 1);
    click(&mut ui, "2");
    check_files(&mut ui);
    assert_eq!(flow(&ui).stage.step(), 2);
    copy_now(&mut ui);
    assert_eq!(flow(&ui).stage.step(), 3);
    ui.get_by_label("3 Done");
}

/// Account 1 to copy from, and accounts 2 to 7 to copy onto, of which
/// account 4 is damaged.
fn seven_accounts() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for id in 1..=7_i64 {
        let path = dir.path().join(format!("core_user_{id}.dat"));
        if id == 4 {
            fs::write(path, b"damaged").unwrap();
        } else {
            fs::write(path, encode(&account(id, &[id]), false).unwrap()).unwrap();
        }
    }
    dir
}

/// The copy screen for [`seven_accounts`], listed by ID: 2, 3, 4, 5, 6, 7.
fn range_screen(dir: &Path) -> Ui {
    let mut ui = copy_screen(dir, EveStatus::NotRunning);
    click(&mut ui, "ID");
    assert_eq!(listed_ids(&ui), [2, 3, 4, 5, 6, 7]);
    ui
}

fn ticked_ids(ui: &Ui) -> Vec<u64> {
    let flow = flow(ui);
    let mut ids: Vec<u64> = flow
        .candidates
        .iter()
        .filter(|candidate| candidate.ticked)
        .map(|candidate| candidate.entry.id)
        .collect();
    ids.sort_unstable();
    ids
}

/// A click on the row showing `label` with the Shift key held.
fn shift_click(ui: &mut Ui, label: &str) {
    ui.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
    click(ui, label);
    ui.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    settle(ui);
}

/// A click on the row of the file with this ID, as the flow sees it.
fn click_id(ui: &mut Ui, id: u64, shift: bool) {
    let flow = ui.state_mut().state.copy_mut().unwrap();
    let index = flow
        .candidates
        .iter()
        .position(|candidate| candidate.entry.id == id)
        .unwrap();
    flow.click_row(index, shift);
}

#[test]
fn shift_click_ticks_every_file_between_the_last_click_and_this_one() {
    let dir = seven_accounts();
    let mut ui = range_screen(dir.path());
    ui.get_by_label("Shift-click ticks a range");
    click(&mut ui, "2");
    shift_click(&mut ui, "6");
    // The damaged account in the middle can't be ticked and stays out.
    assert_eq!(ticked_ids(&ui), [2, 3, 5, 6]);
    ui.get_by_label("4 files ticked");
    // Shift is no longer held: the next click is an ordinary one.
    click(&mut ui, "7");
    assert_eq!(ticked_ids(&ui), [2, 3, 5, 6, 7]);
}

#[test]
fn a_range_can_run_upwards_and_continues_from_the_last_click() {
    let dir = seven_accounts();
    let mut ui = range_screen(dir.path());
    click_id(&mut ui, 6, false);
    click_id(&mut ui, 3, true);
    assert_eq!(ticked_ids(&ui), [3, 5, 6]);
    // The next range starts from the row just Shift-clicked.
    click_id(&mut ui, 2, true);
    assert_eq!(ticked_ids(&ui), [2, 3, 5, 6]);
    click_id(&mut ui, 7, true);
    assert_eq!(ticked_ids(&ui), [2, 3, 5, 6, 7]);
}

#[test]
fn a_range_from_an_unticked_file_unticks() {
    let dir = seven_accounts();
    let mut ui = range_screen(dir.path());
    click(&mut ui, "Tick all shown");
    assert_eq!(ticked_ids(&ui), [2, 3, 5, 6, 7]);
    // Unticking 3 and Shift-clicking 6 unticks everything between them.
    click_id(&mut ui, 3, false);
    click_id(&mut ui, 6, true);
    assert_eq!(ticked_ids(&ui), [2, 7]);
}

#[test]
fn a_range_follows_the_order_and_search_on_screen() {
    let dir = seven_accounts();
    let mut ui = range_screen(dir.path());
    // Listed from the highest ID down, so 7 to 5 is the top three rows.
    click(&mut ui, "ID \u{2B06}");
    assert_eq!(listed_ids(&ui), [7, 6, 5, 4, 3, 2]);
    click_id(&mut ui, 7, false);
    click_id(&mut ui, 5, true);
    assert_eq!(ticked_ids(&ui), [5, 6, 7]);

    // Files searched out of view aren't part of a range.
    click(&mut ui, "Untick all shown");
    // Back to lowest ID first.
    click(&mut ui, "ID \u{2B07}");
    let flow = ui.state_mut().state.copy_mut().unwrap();
    for (candidate, name) in flow
        .candidates
        .iter_mut()
        .zip(["a", "b", "x", "a", "b", "a"])
    {
        candidate.name = Some(name.to_owned());
    }
    flow.search = "a".to_owned();
    assert_eq!(listed_ids(&ui), [2, 5, 7]);
    click_id(&mut ui, 2, false);
    click_id(&mut ui, 7, true);
    assert_eq!(ticked_ids(&ui), [2, 5, 7]);
}

#[test]
fn shift_click_with_nowhere_to_start_from_is_an_ordinary_click() {
    let dir = seven_accounts();
    let mut ui = range_screen(dir.path());
    // Nothing has been clicked yet.
    click_id(&mut ui, 5, true);
    assert_eq!(ticked_ids(&ui), [5]);
    // The file last clicked has been searched out of view.
    ui.state_mut().state.copy_mut().unwrap().search = "7".to_owned();
    click_id(&mut ui, 7, true);
    assert_eq!(ticked_ids(&ui), [5, 7]);
    // Shift-clicking the same row again leaves it as it is.
    click_id(&mut ui, 7, true);
    assert_eq!(ticked_ids(&ui), [5, 7]);
}

#[test]
fn a_file_that_cannot_be_used_is_never_ticked_or_started_from() {
    let dir = seven_accounts();
    let mut ui = range_screen(dir.path());
    click_id(&mut ui, 2, false);
    // Clicks on the damaged file do nothing, with or without Shift.
    click_id(&mut ui, 4, false);
    click_id(&mut ui, 4, true);
    assert_eq!(ticked_ids(&ui), [2]);
    // The range still starts from account 2, not from the damaged one.
    click_id(&mut ui, 6, true);
    assert_eq!(ticked_ids(&ui), [2, 3, 5, 6]);
    // A row that doesn't exist is ignored.
    ui.state_mut().state.copy_mut().unwrap().click_row(99, true);
    assert_eq!(ticked_ids(&ui), [2, 3, 5, 6]);
}

#[test]
fn the_review_says_when_the_source_overview_was_left_on_another_tab() {
    // Without it, an ordinary copy says nothing about tabs.
    let dir = profile();
    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "2");
    check_files(&mut ui);
    assert!(!shows(&ui, crate::copy_view::FIRST_TAB_NOTE));

    // Account 1 was last closed with its second overview tab selected.
    let dir = profile();
    let Value::Dict(mut sections) = account(1, &[10]) else {
        panic!("not a dict");
    };
    sections.push((
        b("tabgroups"),
        Value::Dict(vec![(b("overviewTabs"), stamped(1, Value::Int(1)))]),
    ));
    let source = encode(&Value::Dict(sections), false).unwrap();
    fs::write(dir.path().join("core_user_1.dat"), &source).unwrap();
    let mut ui = copy_screen(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "2");
    check_files(&mut ui);
    ui.get_by_label(crate::copy_view::FIRST_TAB_NOTE);
    copy_now(&mut ui);

    let written = decode(&fs::read(dir.path().join("core_user_2.dat")).unwrap())
        .unwrap()
        .value;
    let Value::Dict(sections) = written else {
        panic!("not a dict");
    };
    let tab_groups = sections
        .iter()
        .find(|(name, _)| *name == b("tabgroups"))
        .map(|(_, value)| value.clone());
    assert_eq!(
        tab_groups,
        Some(Value::Dict(vec![(
            b("overviewTabs"),
            stamped(1, Value::Int(0))
        )]))
    );
    // The source file is as it was.
    assert_eq!(
        fs::read(dir.path().join("core_user_1.dat")).unwrap(),
        source
    );
}
