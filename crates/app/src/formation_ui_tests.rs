//! Tests that drive the probe formation editor in the real UI, off-screen,
//! against files in a temp folder.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use clone_bay_core::formations::{read, Formations, Probe, AU, KM};
use clone_bay_core::platform::EveStatus;
use clone_bay_marshal::{decode, encode, Long, Value};
use eframe::egui;
use eframe::egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT, Queryable};

use crate::app::test_support::{click, harness_with, settle, shows, wait_until, Ui};
use crate::formation_flow::FormationFlow;
use crate::services::Services;

fn b(text: &str) -> Value {
    Value::Bytes(text.as_bytes().to_vec())
}

fn stamped(value: Value) -> Value {
    Value::Tuple(vec![Value::Long(Long::from_i64(1)), value])
}

fn probe(x_km: f64, y_km: f64, z_km: f64, range_au: f64) -> Value {
    let position = Value::Tuple(vec![
        Value::Float(x_km * KM),
        Value::Float(y_km * KM),
        Value::Float(z_km * KM),
    ]);
    Value::Tuple(vec![position, Value::Float(range_au * AU)])
}

fn formation(name: &str, probes: Vec<Value>) -> Value {
    Value::Tuple(vec![Value::Str(name.to_owned()), Value::List(probes)])
}

/// An account with the client's scratch entry, an off-centre two-probe
/// formation ("skatter", ID 2) and a one-probe formation in use ("long", 13).
fn account(formations: Option<Value>) -> Value {
    let formations = formations.unwrap_or_else(|| {
        Value::Dict(vec![
            (
                Value::Int(-4),
                formation("tempFormation", vec![probe(0.0, 0.0, 0.0, 32.0)]),
            ),
            (
                Value::Int(2),
                formation(
                    "skatter",
                    vec![
                        probe(300.0, 0.0, 0.0, 0.25),
                        probe(-100.0, 400.0, 0.0, 0.25),
                    ],
                ),
            ),
            (
                Value::Int(13),
                formation("long", vec![probe(0.0, 0.0, 0.0, 32.0)]),
            ),
        ])
    });
    Value::Dict(vec![(
        b("ui"),
        Value::Dict(vec![
            (b("probescanning.customFormations"), stamped(formations)),
            (
                b("probescanning.selectedFormationID"),
                stamped(Value::Int(13)),
            ),
        ]),
    )])
}

fn profile_with(account: &Value) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("core_user_1.dat"),
        encode(account, false).unwrap(),
    )
    .unwrap();
    let character = Value::Dict(vec![(b("ui"), Value::Dict(vec![]))]);
    fs::write(
        dir.path().join("core_char_2.dat"),
        encode(&character, false).unwrap(),
    )
    .unwrap();
    dir
}

fn profile() -> tempfile::TempDir {
    profile_with(&account(None))
}

fn services(eve: EveStatus) -> Services {
    Services {
        eve_status: Arc::new(move || eve),
        ..Services::for_tests()
    }
}

/// The app with the formation editor open on account 1.
fn editor(dir: &Path, eve: EveStatus) -> Ui {
    let mut ui = harness_with(dir, services(eve));
    click(&mut ui, "1");
    click(&mut ui, "Edit probe formations");
    assert!(ui.state().state.formations().is_some());
    ui
}

fn flow(ui: &Ui) -> &FormationFlow {
    ui.state().state.formations().unwrap()
}

fn flow_mut(ui: &mut Ui) -> &mut FormationFlow {
    ui.state_mut().state.formations_mut().unwrap()
}

fn save(ui: &mut Ui) {
    click(ui, "Save changes");
    wait_until(ui, |app| {
        app.state.formations().is_some_and(|flow| !flow.busy())
    });
}

fn on_disk(dir: &Path) -> Formations {
    let root = decode(&fs::read(dir.join("core_user_1.dat")).unwrap())
        .unwrap()
        .value;
    read(&root).unwrap()
}

fn names(formations: &Formations) -> Vec<String> {
    formations
        .list()
        .iter()
        .map(|formation| formation.name.clone())
        .collect()
}

/// A real pointer click at a point on the screen.
fn click_at(ui: &mut Ui, at: egui::Pos2) {
    ui.hover_at(at);
    ui.step();
    ui.drag_at(at);
    ui.step();
    ui.drop_at(at);
    settle(ui);
}

/// Opens the drop-down showing `shown` and picks `option`.
fn pick(ui: &mut Ui, shown: &str, option: &str) {
    let at = ui.get_all_by_value(shown).next().unwrap().rect().center();
    click_at(ui, at);
    click(ui, option);
}

#[test]
fn the_editor_lists_the_formations_and_opens_the_one_in_use() {
    let dir = profile();
    let ui = editor(dir.path(), EveStatus::NotRunning);
    ui.get_by_label("Probe formations for account 1");
    ui.get_by_label("skatter  (2 probes)");
    ui.get_by_label("long  (1 probe)  default");
    assert!(
        !shows(&ui, "tempFormation  (1 probe)"),
        "the client's scratch entry is hidden"
    );
    assert_eq!(flow(&ui).current, Some(13));
    ui.get_by_label("Launches exactly as drawn.");
    ui.get_by_label("Formation view");
    // It is already the one in use, so there is nothing to switch to.
    assert!(!shows(&ui, "Default in game"));
    assert!(ui
        .get_by_label("Save changes")
        .accesskit_node()
        .is_disabled());
    assert!(!flow(&ui).is_dirty());
    assert!(!shows(&ui, "Changes not saved"));
}

#[test]
fn a_formation_saved_in_the_game_after_the_editor_opened_is_not_lost() {
    let dir = profile();
    let path = dir.path().join("core_user_1.dat");
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    flow_mut(&mut ui).add_preset(clone_bay_core::formations::Preset::Spread);
    settle(&mut ui);
    // Meanwhile the game saved a formation of its own.
    let with_game_formation = Value::Dict(vec![
        (
            Value::Int(13),
            formation("long", vec![probe(0.0, 0.0, 0.0, 32.0)]),
        ),
        (
            Value::Int(20),
            formation("made in the game", vec![probe(0.0, 0.0, 0.0, 4.0)]),
        ),
    ]);
    let game_bytes = encode(&account(Some(with_game_formation)), false).unwrap();
    fs::write(&path, &game_bytes).unwrap();

    save(&mut ui);
    ui.get_by_label(crate::edit_ui_tests::CHANGED_SINCE_OPENED);
    assert_eq!(fs::read(&path).unwrap(), game_bytes);
    assert!(flow(&ui).is_dirty(), "the editor's changes are still there");

    click(&mut ui, "Discard changes");
    assert_eq!(names(&flow(&ui).formations), ["long", "made in the game"]);
}

#[test]
fn only_accounts_have_probe_formations() {
    let dir = profile();
    let mut ui = harness_with(dir.path(), services(EveStatus::NotRunning));
    click(&mut ui, "2");
    assert!(!shows(&ui, "Edit probe formations"));
    ui.state_mut().state.begin_formations();
    assert!(ui.state().state.formations().is_none());
}

#[test]
fn clicking_a_formation_shows_it() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "skatter  (2 probes)");
    assert_eq!(flow(&ui).current, Some(2));
    // (300, 0, 0) and (-100, 400, 0) average to (100, 200, 0): 224 km off.
    ui.get_by_label(
        "The game centres a formation on its probes' average position, so this one will launch \
         shifted by 224 km.",
    );
    ui.get_by_label("Default in game");
    assert!(
        !flow(&ui).is_dirty(),
        "looking at a formation changes nothing"
    );
}

#[test]
fn a_formation_added_from_a_preset_is_saved_with_a_backup() {
    let dir = profile();
    let before = fs::read(dir.path().join("core_user_1.dat")).unwrap();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    pick(&mut ui, "Add a formation", "Pinpoint (500 km)");
    ui.get_by_label("Pinpoint (500 km)  (8 probes)");
    assert_eq!(flow(&ui).current, Some(14));
    ui.get_by_label("Changes not saved");
    // A second one gets a name of its own.
    pick(&mut ui, "Add a formation", "Pinpoint (500 km)");
    ui.get_by_label("Pinpoint (500 km) 2  (8 probes)");

    save(&mut ui);
    let backup = dir.path().join("clone-bay-backups").join("T");
    ui.get_by_label(format!("Saved. The original was backed up in {}", backup.display()).as_str());
    assert_eq!(fs::read(backup.join("core_user_1.dat")).unwrap(), before);
    let saved = on_disk(dir.path());
    assert_eq!(
        names(&saved),
        [
            "skatter",
            "long",
            "Pinpoint (500 km)",
            "Pinpoint (500 km) 2"
        ]
    );
    assert_eq!(
        saved.get(14).unwrap().probes[0],
        Probe::at_km(500.0, 0.0, 0.0, 4.0)
    );
    assert_eq!(saved.selected, Some(13));
    assert!(!flow(&ui).is_dirty());
    assert_eq!(
        flow(&ui).current,
        Some(15),
        "still showing the one just added"
    );

    click(&mut ui, "Back to the file list");
    assert!(ui.state().state.formations().is_none());
    let size = fs::metadata(dir.path().join("core_user_1.dat"))
        .unwrap()
        .len();
    let rows = &ui.state().state.rows;
    assert_eq!(
        rows.iter()
            .find(|row| row.entry.id == 1)
            .unwrap()
            .entry
            .size,
        size
    );
}

#[test]
fn formations_can_be_duplicated_and_deleted() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "skatter  (2 probes)");
    click(&mut ui, "Duplicate");
    ui.get_by_label("skatter copy  (2 probes)");
    assert_eq!(flow(&ui).current, Some(14));
    click(&mut ui, "Delete");
    assert!(!shows(&ui, "skatter copy  (2 probes)"));
    assert_eq!(flow(&ui).current, Some(2), "the first one left is shown");
    assert!(
        !flow(&ui).is_dirty(),
        "adding then deleting leaves things as they were"
    );

    // Deleting the one in use moves the choice when saved.
    click(&mut ui, "long  (1 probe)  default");
    click(&mut ui, "Delete");
    save(&mut ui);
    let saved = on_disk(dir.path());
    assert_eq!(names(&saved), ["skatter"]);
    assert_eq!(saved.selected, Some(2));
    ui.get_by_label("skatter  (2 probes)  default");
}

#[test]
fn deleting_every_formation_leaves_an_empty_editor() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "Delete");
    click(&mut ui, "Delete");
    assert_eq!(flow(&ui).current, None);
    ui.get_by_label("This account has no custom formations yet.");
    ui.get_by_label("Add a formation, or pick one on the left.");
    assert!(ui.get_by_label("Delete").accesskit_node().is_disabled());
    assert!(ui.get_by_label("Duplicate").accesskit_node().is_disabled());
    click(&mut ui, "Delete");
    click(&mut ui, "Duplicate");
    assert!(flow(&ui).formations.list().is_empty());
}

#[test]
fn balancing_adds_a_counterweight_so_the_formation_launches_as_drawn() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "skatter  (2 probes)");
    assert!(!ui.get_by_label("Balance").accesskit_node().is_disabled());
    click(&mut ui, "Balance");
    ui.get_by_label("skatter  (3 probes)");
    ui.get_by_label("Launches exactly as drawn.");
    assert!(ui.get_by_label("Balance").accesskit_node().is_disabled());
    let probes = &flow(&ui).formations.get(2).unwrap().probes;
    assert_eq!(probes[2], Probe::at_km(-200.0, -400.0, 0.0, 0.25));
}

#[test]
fn a_formation_can_be_resized_and_given_one_range_throughout() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "skatter  (2 probes)");
    click(&mut ui, "Double size");
    let probes = |ui: &Ui| flow(ui).formations.get(2).unwrap().probes.clone();
    assert_eq!(probes(&ui)[0], Probe::at_km(600.0, 0.0, 0.0, 0.25));
    click(&mut ui, "Half size");
    click(&mut ui, "Half size");
    assert_eq!(probes(&ui)[1], Probe::at_km(-50.0, 200.0, 0.0, 0.25));

    pick(&mut ui, "Set every range", "4 AU");
    assert!(probes(&ui).iter().all(|probe| probe.range == 4.0 * AU));
    // One probe's own range.
    pick(&mut ui, "4 AU", "16 AU");
    assert_eq!(probes(&ui)[0].range, 16.0 * AU);
    assert_eq!(probes(&ui)[1].range, 4.0 * AU);
}

#[test]
fn probes_can_be_added_up_to_eight_and_removed() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "skatter  (2 probes)");
    click(&mut ui, "Add a probe");
    let probes = |ui: &Ui| flow(ui).formations.get(2).unwrap().probes.clone();
    // The new probe sits at the centre with the same range as the others.
    assert_eq!(probes(&ui)[2], Probe::at_km(0.0, 0.0, 0.0, 0.25));
    for _ in 0..5 {
        click(&mut ui, "Add a probe");
    }
    ui.get_by_label("skatter  (8 probes)");
    assert!(ui
        .get_by_label("Add a probe")
        .accesskit_node()
        .is_disabled());
    click(&mut ui, "Add a probe");
    assert_eq!(probes(&ui).len(), 8);

    let first_remove = ui
        .get_all_by_label("Remove")
        .next()
        .unwrap()
        .rect()
        .center();
    click_at(&mut ui, first_remove);
    assert_eq!(probes(&ui).len(), 7);
    assert_eq!(
        probes(&ui)[0],
        Probe::at_km(-100.0, 400.0, 0.0, 0.25),
        "the first one went"
    );
}

// The test that dragged a coordinate box was removed with the dragging:
// positions are typed only.
// What may be typed is tested with `parse_km` in `formation_view.rs`.

#[test]
fn another_formation_can_be_made_the_one_in_use() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "skatter  (2 probes)");
    click(&mut ui, "Default in game");
    ui.get_by_label("skatter  (2 probes)  default");
    ui.get_by_label("long  (1 probe)");
    save(&mut ui);
    assert_eq!(on_disk(dir.path()).selected, Some(2));
}

#[test]
fn a_formation_the_game_could_not_use_cannot_be_saved() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    flow_mut(&mut ui).formations.get_mut(13).unwrap().name = " ".to_owned();
    settle(&mut ui);
    ui.get_by_label("Formation '(unnamed)': it needs a name.");
    assert!(flow(&ui).is_dirty());
    assert!(!flow(&ui).can_save());
    assert!(ui
        .get_by_label("Save changes")
        .accesskit_node()
        .is_disabled());
    click(&mut ui, "Save changes");
    assert_eq!(names(&on_disk(dir.path())), ["skatter", "long"]);

    click(&mut ui, "Discard changes");
    assert!(!flow(&ui).is_dirty());
    assert!(!shows(&ui, "Formation '(unnamed)': it needs a name."));
    ui.get_by_label("long  (1 probe)  default");
}

#[test]
fn nothing_is_saved_while_eve_is_running() {
    let dir = profile();
    let before = fs::read(dir.path().join("core_user_1.dat")).unwrap();
    let mut ui = editor(dir.path(), EveStatus::Running);
    click(&mut ui, "Duplicate");
    save(&mut ui);
    ui.get_by_label(
        "EVE is running. Close every EVE client first: the client saves its settings when it \
         closes and would overwrite the change. Nothing was changed.",
    );
    assert_eq!(
        fs::read(dir.path().join("core_user_1.dat")).unwrap(),
        before
    );
    assert!(
        flow(&ui).is_dirty(),
        "the changes are still there to save later"
    );
    assert!(!shows(&ui, "I have closed every EVE client"));
}

#[test]
fn when_eve_cannot_be_checked_the_user_must_confirm_it_is_closed() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::Unknown);
    click(&mut ui, "Duplicate");
    save(&mut ui);
    assert_eq!(names(&on_disk(dir.path())), ["skatter", "long"]);
    assert!(!flow(&ui).can_save());
    click(&mut ui, "I have closed every EVE client");
    assert!(flow(&ui).can_save());
    save(&mut ui);
    assert_eq!(
        names(&on_disk(dir.path())),
        ["skatter", "long", "long copy"]
    );
}

#[test]
fn formations_stored_in_an_unexpected_form_cannot_be_edited() {
    let dir = profile_with(&account(Some(Value::List(vec![]))));
    let before = fs::read(dir.path().join("core_user_1.dat")).unwrap();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    ui.get_by_label("The probe formations are stored in a form this app doesn't understand.");
    assert!(!flow(&ui).readable());
    assert!(!shows(&ui, "Formations"));
    assert!(!shows(&ui, "Add a formation, or pick one on the left."));
    // Nothing can be added or saved over what couldn't be read.
    flow_mut(&mut ui).add_preset(clone_bay_core::formations::Preset::Spread);
    assert!(flow(&ui).formations.list().is_empty());
    assert!(!flow(&ui).can_save());
    click(&mut ui, "Back to the file list");
    assert_eq!(
        fs::read(dir.path().join("core_user_1.dat")).unwrap(),
        before
    );
}

#[test]
fn leaving_without_saving_does_not_relist_the_folder() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "Duplicate");
    fs::write(dir.path().join("core_user_7.dat"), b"new").unwrap();
    click(&mut ui, "Back to the file list");
    assert!(ui.state().state.formations().is_none());
    assert_eq!(ui.state().state.rows.len(), 2);
    assert_eq!(names(&on_disk(dir.path())), ["skatter", "long"]);
}

#[test]
fn an_account_with_no_formations_can_be_given_one() {
    let bare = Value::Dict(vec![(b("ui"), Value::Dict(vec![]))]);
    let dir = profile_with(&bare);
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    ui.get_by_label("This account has no custom formations yet.");
    assert_eq!(flow(&ui).current, None);
    pick(&mut ui, "Add a formation", "Spread (250 km)");
    // The file stores no choice of formation, so none is offered.
    assert!(!shows(&ui, "Default in game"));
    save(&mut ui);
    let saved = on_disk(dir.path());
    assert_eq!(names(&saved), ["Spread (250 km)"]);
    assert_eq!(saved.selected, None);
}

#[test]
fn the_formation_being_shown_is_the_one_highlighted() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    // A highlighted row reports itself as "on" to assistive technology.
    let selected = |ui: &Ui, label: &str| {
        let node = ui.get_by_label(label).accesskit_node();
        node.toggled().map(|state| state == Toggled::True)
    };
    assert_eq!(selected(&ui, "long  (1 probe)  default"), Some(true));
    assert_eq!(selected(&ui, "skatter  (2 probes)"), Some(false));
    click(&mut ui, "skatter  (2 probes)");
    assert_eq!(selected(&ui, "skatter  (2 probes)"), Some(true));
    assert_eq!(selected(&ui, "long  (1 probe)  default"), Some(false));
}

#[test]
fn probes_are_numbered_from_one_and_shown_in_kilometres() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "skatter  (2 probes)");
    // Row numbers in the table (the pictures draw theirs, they aren't text).
    ui.get_by_label("1");
    ui.get_by_label("2");
    assert!(!shows(&ui, "0") && !shows(&ui, "3"));
    // 300,000 m is shown as 300, under a heading that says kilometres.
    // A box and the text inside it both carry the value, so count the boxes.
    let boxes_holding = |value: &str| {
        ui.get_all_by_value(value)
            .filter(|node| node.accesskit_node().role() == Role::TextInput)
            .count()
    };
    // The first probe is 300 km west, so 300 km from the centre too.
    assert_eq!(boxes_holding("300"), 2);
    // The second is 412.311 km from the centre.
    assert_eq!(boxes_holding("412.311"), 1);
    assert_eq!(boxes_holding("-100"), 1);
    assert_eq!(boxes_holding("400"), 1);
    // Three zeros: the first probe's N/S and U/D, the second's N/S.
    assert_eq!(boxes_holding("0"), 3);
}

#[test]
fn a_ninth_probe_is_never_added() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "skatter  (2 probes)");
    for _ in 0..10 {
        flow_mut(&mut ui).add_probe();
    }
    assert_eq!(flow(&ui).formations.get(2).unwrap().probes.len(), 8);
}

// The tests below were written with Undo/Redo and the compass labels and
// had not been run when they were committed.

#[test]
fn undo_takes_back_one_change_at_a_time_and_redo_puts_it_back() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    assert!(ui.get_by_label("Undo").accesskit_node().is_disabled());
    assert!(ui.get_by_label("Redo").accesskit_node().is_disabled());

    click(&mut ui, "skatter  (2 probes)");
    click(&mut ui, "Double size");
    click(&mut ui, "Balance");
    let probes = |ui: &Ui| flow(ui).formations.get(2).unwrap().probes.clone();
    assert_eq!(probes(&ui).len(), 3);

    click(&mut ui, "Undo");
    assert_eq!(probes(&ui).len(), 2, "the balance was taken back");
    assert_eq!(
        probes(&ui)[0],
        Probe::at_km(600.0, 0.0, 0.0, 0.25),
        "the doubling was not"
    );
    click(&mut ui, "Undo");
    assert_eq!(probes(&ui)[0], Probe::at_km(300.0, 0.0, 0.0, 0.25));
    assert!(!flow(&ui).is_dirty(), "back to what the file holds");
    assert!(ui.get_by_label("Undo").accesskit_node().is_disabled());

    click(&mut ui, "Redo");
    assert_eq!(probes(&ui)[0], Probe::at_km(600.0, 0.0, 0.0, 0.25));
    click(&mut ui, "Redo");
    assert_eq!(probes(&ui).len(), 3);
    assert!(ui.get_by_label("Redo").accesskit_node().is_disabled());
}

#[test]
fn a_new_change_after_undoing_clears_what_could_be_redone() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "skatter  (2 probes)");
    click(&mut ui, "Double size");
    click(&mut ui, "Undo");
    assert!(flow(&ui).can_redo());
    click(&mut ui, "Half size");
    assert!(!flow(&ui).can_redo());
    assert!(flow(&ui).can_undo());
}

#[test]
fn deleting_and_adding_formations_can_be_undone() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "Delete");
    assert_eq!(names(&flow(&ui).formations), ["skatter"]);
    click(&mut ui, "Undo");
    assert_eq!(names(&flow(&ui).formations), ["skatter", "long"]);
    assert_eq!(flow(&ui).formations.selected, Some(13));

    pick(&mut ui, "Add a formation", "Line to the north");
    assert_eq!(flow(&ui).current, Some(14));
    click(&mut ui, "Undo");
    assert_eq!(names(&flow(&ui).formations), ["skatter", "long"]);
    // The formation that was being shown is gone, so the first one is shown.
    assert_eq!(flow(&ui).current, Some(2));
}

#[test]
fn a_run_of_typing_is_one_step_but_separate_runs_are_separate() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    let state = |ui: &Ui| flow(ui).formations.clone();
    let original = state(&ui);
    let rename = |ui: &mut Ui, name: &str, at: f64| {
        let before = state(ui);
        flow_mut(ui).formations.get_mut(13).unwrap().name = name.to_owned();
        flow_mut(ui).record_change(before, true, at);
    };
    // Three keystrokes within a second of each other, then one much later.
    rename(&mut ui, "l", 10.0);
    rename(&mut ui, "lo", 10.3);
    rename(&mut ui, "lon", 10.9);
    rename(&mut ui, "lonX", 20.0);
    flow_mut(&mut ui).undo();
    assert_eq!(flow(&ui).formations.get(13).unwrap().name, "lon");
    flow_mut(&mut ui).undo();
    assert_eq!(state(&ui), original, "the whole first run went in one step");
    assert!(!flow(&ui).can_undo());
}

#[test]
fn discarding_can_be_undone() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "Duplicate");
    click(&mut ui, "Discard changes");
    assert_eq!(names(&flow(&ui).formations), ["skatter", "long"]);
    click(&mut ui, "Undo");
    assert_eq!(
        names(&flow(&ui).formations),
        ["skatter", "long", "long copy"]
    );
}

#[test]
fn directions_are_named_by_compass_point() {
    let dir = profile();
    let ui = editor(dir.path(), EveStatus::NotRunning);
    for heading in ["N/S (km)", "W/E (km)", "U/D (km)", "Formation view"] {
        ui.get_by_label(heading);
    }
    assert!(!shows(&ui, "X") && !shows(&ui, "Y (up)") && !shows(&ui, "Z"));
}

// Written with the turnable picture and not yet run when committed.
#[test]
fn the_view_buttons_turn_the_picture() {
    use crate::viewer::Camera;
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    assert_eq!(flow(&ui).camera, Camera::default());
    click(&mut ui, "Top");
    assert_eq!(flow(&ui).camera, Camera::TOP);
    click(&mut ui, "Front");
    assert_eq!(flow(&ui).camera, Camera::FRONT);
    click(&mut ui, "Side");
    assert_eq!(flow(&ui).camera, Camera::SIDE);
    click(&mut ui, "Reset view");
    assert_eq!(flow(&ui).camera, Camera::default());
    // Turning the picture is not a change to the formations.
    assert!(!flow(&ui).is_dirty() && !flow(&ui).can_undo());
}

#[test]
fn a_probe_number_picks_the_probe_out_and_lets_it_go_again() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    // "skatter" (ID 2) has two probes.
    click(&mut ui, "skatter  (2 probes)");
    assert_eq!(flow(&ui).picked_probe, None);
    click(&mut ui, "2");
    assert_eq!(flow(&ui).picked_probe, Some((2, 1)));
    click(&mut ui, "1");
    assert_eq!(flow(&ui).picked_probe, Some((2, 0)));
    click(&mut ui, "1");
    assert_eq!(flow(&ui).picked_probe, None);
    // A probe picked in one formation isn't picked in another.
    click(&mut ui, "1");
    click(&mut ui, "long  (1 probe)  default");
    click(&mut ui, "1");
    assert_eq!(flow(&ui).picked_probe, Some((13, 0)));
    assert!(!flow(&ui).is_dirty());
}

// Written with the arrow buttons and not yet run when committed.
#[test]
fn the_arrows_beside_a_position_move_the_probe_fifty_kilometres() {
    use crate::formation_view::{LEFT_ARROW, RIGHT_ARROW};
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    // "long" has one probe at the centre: three boxes, an arrow each side.
    let before = flow(&ui).formations.get(13).unwrap().probes[0];
    let lefts: Vec<_> = ui
        .get_all_by_label(LEFT_ARROW)
        .map(|arrow| arrow.rect().center())
        .collect();
    let rights: Vec<_> = ui
        .get_all_by_label(RIGHT_ARROW)
        .map(|arrow| arrow.rect().center())
        .collect();
    assert_eq!((lefts.len(), rights.len()), (3, 3));
    // Left of the first box: north, which is +z.
    click_at(&mut ui, lefts[0]);
    // Right of the second box: east, which is -x.
    click_at(&mut ui, rights[1]);
    // Left of the third box, twice: up, which is +y.
    click_at(&mut ui, lefts[2]);
    click_at(&mut ui, lefts[2]);
    let after = flow(&ui).formations.get(13).unwrap().probes[0];
    assert_eq!(after.z, before.z + 50_000.0);
    assert_eq!(after.x, before.x - 50_000.0);
    assert_eq!(after.y, before.y + 100_000.0);
    assert_eq!(after.range, before.range);
    // Each click is a step of its own to undo.
    click(&mut ui, "Undo");
    let undone = flow(&ui).formations.get(13).unwrap().probes[0];
    assert_eq!(undone.y, before.y + 50_000.0);
}

/// Types `text` into the box that currently shows `shown`.
fn type_into(ui: &mut Ui, shown: &str, text: &str) {
    let node = ui
        .get_all_by_value(shown)
        .find(|node| node.accesskit_node().role() == Role::TextInput)
        .unwrap();
    node.focus();
    ui.step();
    let node = ui
        .get_all_by_value(shown)
        .find(|node| node.accesskit_node().role() == Role::TextInput)
        .unwrap();
    ui.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    node.type_text(text);
    settle(ui);
}

#[test]
fn a_formation_can_be_scaled_by_a_typed_factor() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "skatter  (2 probes)");
    let probes = |ui: &Ui| flow(ui).formations.get(2).unwrap().probes.clone();
    // The box starts at one and a half.
    click(&mut ui, "Scale");
    assert_eq!(probes(&ui)[0], Probe::at_km(450.0, 0.0, 0.0, 0.25));
    type_into(&mut ui, "1.5", "0.2");
    assert_eq!(flow(&ui).tools.scale_factor, 0.2);
    // Typing a factor is not an edit; only the button is.
    assert_eq!(probes(&ui)[0], Probe::at_km(450.0, 0.0, 0.0, 0.25));
    click(&mut ui, "Scale");
    assert_eq!(probes(&ui)[0], Probe::at_km(90.0, 0.0, 0.0, 0.25));
    assert_eq!(probes(&ui)[1], Probe::at_km(-30.0, 120.0, 0.0, 0.25));
    // Each click is a step of its own to undo.
    click(&mut ui, "Undo");
    assert_eq!(probes(&ui)[0], Probe::at_km(450.0, 0.0, 0.0, 0.25));
    click(&mut ui, "Undo");
    assert_eq!(probes(&ui)[0], Probe::at_km(300.0, 0.0, 0.0, 0.25));
    assert!(!flow(&ui).can_undo());
}

#[test]
fn a_formation_can_be_turned_and_mirrored() {
    use clone_bay_core::formations::View;
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "skatter  (2 probes)");
    let probes = |ui: &Ui| flow(ui).formations.get(2).unwrap().probes.clone();
    // Stored as (west, up, north): 300 km west, and 100 km east 400 km up.
    assert_eq!(probes(&ui)[0], Probe::at_km(300.0, 0.0, 0.0, 0.25));
    assert_eq!(probes(&ui)[1], Probe::at_km(-100.0, 400.0, 0.0, 0.25));

    // A quarter turn clockwise seen from above: west goes north, east south.
    click(&mut ui, "Turn");
    assert_eq!(probes(&ui)[0], Probe::at_km(0.0, 0.0, 300.0, 0.25));
    assert_eq!(probes(&ui)[1], Probe::at_km(0.0, 400.0, -100.0, 0.25));

    click(&mut ui, "Flip N/S");
    assert_eq!(probes(&ui)[0], Probe::at_km(0.0, 0.0, -300.0, 0.25));
    assert_eq!(probes(&ui)[1], Probe::at_km(0.0, 400.0, 100.0, 0.25));
    click(&mut ui, "Flip U/D");
    assert_eq!(probes(&ui)[1], Probe::at_km(0.0, -400.0, 100.0, 0.25));
    click(&mut ui, "Flip W/E");
    assert_eq!(probes(&ui)[1], Probe::at_km(0.0, -400.0, 100.0, 0.25));

    // Seen from the east, by a typed angle the other way: north goes up.
    pick(&mut ui, "above", "the east");
    assert_eq!(flow(&ui).tools.turn_view, View::East);
    type_into(&mut ui, "90", "-90");
    assert_eq!(flow(&ui).tools.turn_degrees, -90.0);
    click(&mut ui, "Turn");
    assert_eq!(probes(&ui)[0], Probe::at_km(0.0, -300.0, 0.0, 0.25));
    assert_eq!(probes(&ui)[1], Probe::at_km(0.0, 100.0, 400.0, 0.25));
}

#[test]
fn centre_moves_an_off_balance_formation_onto_its_middle() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "skatter  (2 probes)");
    let probes = |ui: &Ui| flow(ui).formations.get(2).unwrap().probes.clone();
    assert!(!flow(&ui).formations.get(2).unwrap().is_balanced());
    click(&mut ui, "Centre");
    // The middle was 100 km west and 200 km up.
    assert_eq!(probes(&ui)[0], Probe::at_km(200.0, -200.0, 0.0, 0.25));
    assert_eq!(probes(&ui)[1], Probe::at_km(-200.0, 200.0, 0.0, 0.25));
    assert_eq!(probes(&ui).len(), 2, "no probe was added");
    ui.get_by_label("Launches exactly as drawn.");
    // Nothing left to do, for this button or for Balance.
    for label in ["Centre", "Balance"] {
        assert!(ui.get_by_label(label).accesskit_node().is_disabled());
    }
}

fn is_selected(ui: &Ui, label: &str) -> bool {
    ui.get_by_label(label).accesskit_node().toggled() == Some(Toggled::True)
}

#[test]
fn a_picked_probe_is_shown_as_picked_and_forgotten_when_it_goes() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "skatter  (2 probes)");
    click(&mut ui, "2");
    assert!(is_selected(&ui, "2") && !is_selected(&ui, "1"));
    assert_eq!(flow(&ui).picked_in(2), Some(1));
    // It is picked in its own formation only.
    assert_eq!(flow(&ui).picked_in(13), None);
    assert_eq!(flow(&ui).picked_in(999), None);
    // A pick that points past the end of the table isn't one.
    flow_mut(&mut ui).picked_probe = Some((2, 2));
    assert_eq!(flow(&ui).picked_in(2), None);
    flow_mut(&mut ui).picked_probe = Some((2, 1));
    assert_eq!(flow(&ui).picked_in(2), Some(1));
    flow_mut(&mut ui).picked_probe = None;
    assert_eq!(flow(&ui).picked_in(2), None);
}

#[test]
fn clicking_a_probe_in_the_picture_picks_it() {
    use crate::viewer::{east_north_up, extent, Camera, Scene};
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "skatter  (2 probes)");
    click(&mut ui, "Top");
    let rect = ui.get_by_label("Formation view").rect();
    let formation = flow(&ui).formations.get(2).unwrap().clone();
    let scene = Scene::new(rect, Camera::TOP, extent(&formation));
    let on_screen = |index: usize| {
        let probe = formation.probes[index];
        let (east, north, up) = east_north_up(probe.x, probe.y, probe.z);
        scene.place(east, north, up).0
    };
    // The first probe is 300 km west: left of the middle. The second is
    // east of it.
    assert!(on_screen(0).x < rect.center().x && on_screen(1).x > rect.center().x);

    click_at(&mut ui, on_screen(1));
    assert_eq!(flow(&ui).picked_probe, Some((2, 1)));
    assert!(is_selected(&ui, "2"));
    click_at(&mut ui, on_screen(0));
    assert_eq!(flow(&ui).picked_probe, Some((2, 0)));
    // Empty space lets go.
    click_at(&mut ui, rect.left_top() + egui::vec2(6.0, 6.0));
    assert_eq!(flow(&ui).picked_probe, None);
    // None of that is an edit.
    assert!(!flow(&ui).is_dirty() && !flow(&ui).can_undo());
}

#[test]
fn the_picture_is_turned_by_dragging_and_zoomed_by_the_wheel() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "Front");
    let middle = ui.get_by_label("Formation view").rect().center();

    // Dragging right and down turns the view and tips it towards the top.
    ui.hover_at(middle);
    ui.step();
    ui.drag_at(middle);
    ui.step();
    ui.hover_at(middle + egui::vec2(40.0, 30.0));
    ui.step();
    ui.drop_at(middle + egui::vec2(40.0, 30.0));
    settle(&mut ui);
    let camera = flow(&ui).camera;
    assert!((camera.yaw - 0.4).abs() < 0.05, "{}", camera.yaw);
    assert!((camera.pitch - 0.3).abs() < 0.05, "{}", camera.pitch);
    assert_eq!(camera.zoom, 1.0);

    // Scrolling down zooms in, gently; scrolling up zooms back out.
    ui.hover_at(middle);
    ui.step();
    ui.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -100.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    ui.run_steps(30);
    let zoomed = flow(&ui).camera.zoom;
    assert!(zoomed > 1.2 && zoomed < 2.0, "{zoomed}");
    ui.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 200.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    ui.run_steps(30);
    let back = flow(&ui).camera.zoom;
    assert!(back < 0.9 && back > 0.5, "{back}");

    assert!(!flow(&ui).is_dirty());
}

#[test]
fn a_typed_position_moves_the_probe_and_a_run_of_typing_is_one_step() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "skatter  (2 probes)");
    let probes = |ui: &Ui| flow(ui).formations.get(2).unwrap().probes.clone();
    assert_eq!(probes(&ui)[0].x, 300.0 * KM);
    type_into(&mut ui, "300", "250");
    assert_eq!(probes(&ui)[0], Probe::at_km(250.0, 0.0, 0.0, 0.25));
    assert!(flow(&ui).is_dirty());
    // Typed in one go, so one step takes it all back.
    click(&mut ui, "Undo");
    assert_eq!(probes(&ui)[0].x, 300.0 * KM);
    assert!(!flow(&ui).can_undo());

    // Typing a name is a run of typing too.
    type_into(&mut ui, "skatter", "ab");
    assert_eq!(flow(&ui).formations.get(2).unwrap().name, "ab");
    click(&mut ui, "Undo");
    assert_eq!(flow(&ui).formations.get(2).unwrap().name, "skatter");
    assert!(!flow(&ui).can_undo());
}

#[test]
fn typing_runs_end_after_a_pause_and_history_has_a_limit() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    let state = |ui: &Ui| flow(ui).formations.clone();
    let rename = |ui: &mut Ui, name: &str, typing: bool, at: f64| {
        let before = state(ui);
        flow_mut(ui).formations.get_mut(13).unwrap().name = name.to_owned();
        flow_mut(ui).record_change(before, typing, at);
    };
    let undo_count = |ui: &mut Ui| {
        let mut count = 0;
        while flow(ui).can_undo() {
            flow_mut(ui).undo();
            count += 1;
        }
        count
    };
    // Exactly a second apart is a pause; just under is the same run.
    rename(&mut ui, "a", true, 10.0);
    rename(&mut ui, "ab", true, 11.0);
    assert_eq!(undo_count(&mut ui), 2);
    rename(&mut ui, "a", true, 10.0);
    rename(&mut ui, "ab", true, 10.75);
    assert_eq!(undo_count(&mut ui), 1);
    // A button press between two keystrokes splits the run.
    rename(&mut ui, "a", true, 10.0);
    rename(&mut ui, "ab", false, 10.1);
    rename(&mut ui, "abc", true, 10.2);
    assert_eq!(undo_count(&mut ui), 3);

    // Two hundred steps are kept, and no more.
    for step in 0..205 {
        rename(&mut ui, &format!("n{step}"), false, 100.0);
    }
    assert_eq!(undo_count(&mut ui), 200);
    assert_eq!(flow(&ui).formations.get(13).unwrap().name, "n4");
}

#[test]
fn either_redo_shortcut_puts_a_change_back() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "Duplicate");
    let count = |ui: &Ui| flow(ui).formations.list().len();
    assert_eq!(count(&ui), 3);
    let command = egui::Modifiers::COMMAND;
    for redo in [
        (command, egui::Key::Y),
        (command | egui::Modifiers::SHIFT, egui::Key::Z),
    ] {
        ui.key_press_modifiers(command, egui::Key::Z);
        settle(&mut ui);
        assert_eq!(count(&ui), 2, "undone");
        ui.key_press_modifiers(redo.0, redo.1);
        settle(&mut ui);
        assert_eq!(count(&ui), 3, "{redo:?}");
    }
}

/// The editor with a clipboard the test can look at and fill.
fn editor_with_clipboard(dir: &Path) -> (Ui, Arc<crate::services::test_support::FakeClipboard>) {
    let clipboard = Arc::new(crate::services::test_support::FakeClipboard::default());
    let services = Services {
        clipboard: Arc::clone(&clipboard) as _,
        ..services(EveStatus::NotRunning)
    };
    let mut ui = harness_with(dir, services);
    click(&mut ui, "1");
    click(&mut ui, "Edit probe formations");
    (ui, clipboard)
}

fn names_of(ui: &Ui) -> Vec<String> {
    let list = flow(ui).formations.list();
    list.iter()
        .map(|formation| formation.name.clone())
        .collect()
}

#[test]
fn one_formation_or_all_of_them_can_be_copied_to_the_clipboard() {
    let dir = profile();
    let (mut ui, clipboard) = editor_with_clipboard(dir.path());
    let settings_before = fs::read(dir.path().join("core_user_1.dat")).unwrap();
    let copied = || clipboard.text.lock().unwrap().clone().unwrap();

    // The formation being shown, as edited: unsaved changes are included.
    click(&mut ui, "skatter  (2 probes)");
    click(&mut ui, "Double size");
    click(&mut ui, "Copy this one");
    // Just the formation and its probes: north, west, up, range.
    assert_eq!(
        copied(),
        "formation: skatter\nprobe: 0 600 0 0.25\nprobe: 0 -200 800 0.25\n"
    );
    ui.get_by_label("Copied 1 formation to the clipboard.");

    // All of them, in the order listed, without the client's scratch entry.
    click(&mut ui, "Copy all");
    assert_eq!(
        copied(),
        "formation: skatter\nprobe: 0 600 0 0.25\nprobe: 0 -200 800 0.25\n\n\
         formation: long\nprobe: 0 0 0 32\n"
    );
    ui.get_by_label("Copied 2 formations to the clipboard.");

    // A clipboard that can't be written to is reported.
    *clipboard.broken.lock().unwrap() = true;
    click(&mut ui, "Copy this one");
    assert_eq!(flow(&ui).share_note, None);
    ui.get_by_label("Nothing was copied: the clipboard couldn't be written to (busy).");

    // Copying never changes the formations or the settings file.
    assert_eq!(names_of(&ui), ["skatter", "long"]);
    assert_eq!(
        fs::read(dir.path().join("core_user_1.dat")).unwrap(),
        settings_before
    );
}

#[test]
fn pasted_formations_are_added_without_replacing_anything() {
    let dir = profile();
    let (mut ui, clipboard) = editor_with_clipboard(dir.path());
    let settings_before = fs::read(dir.path().join("core_user_1.dat")).unwrap();

    // An empty clipboard adds nothing and says so.
    click(&mut ui, "Paste");
    assert_eq!(names_of(&ui), ["skatter", "long"]);
    assert!(!flow(&ui).is_dirty());
    ui.get_by_label("Nothing was pasted: the clipboard has no text on it.");

    *clipboard.text.lock().unwrap() = Some(
        "formation: long\nprobe: 100 200 300 8\n\n\
         formation: Wide\nprobe: 0 0 0 4\nprobe: -50 0 25.5 4\n"
            .to_owned(),
    );
    click(&mut ui, "Paste");
    // One that shares a name with a formation here gets a name of its own.
    assert_eq!(names_of(&ui), ["skatter", "long", "long 2", "Wide"]);
    let added = flow(&ui).formations.list()[2].clone();
    // North 100, west 200, up 300 is stored as (west, up, north).
    assert_eq!(added.probes, [Probe::at_km(200.0, 300.0, 100.0, 8.0)]);
    assert_eq!(flow(&ui).formations.list()[3].probes.len(), 2);
    // The first one added is the one shown.
    assert_eq!(flow(&ui).current, Some(added.id));
    ui.get_by_label("Added 2 formations from the clipboard. Press \"Save changes\" to keep them.");
    assert!(!shows(
        &ui,
        "Nothing was pasted: the clipboard has no text on it."
    ));
    // The formation in use is still the one it was.
    assert_eq!(flow(&ui).formations.selected, Some(13));

    // Nothing reaches the settings file until it is saved...
    assert!(flow(&ui).is_dirty());
    assert_eq!(
        fs::read(dir.path().join("core_user_1.dat")).unwrap(),
        settings_before
    );
    // ...and the whole paste is one step to undo.
    click(&mut ui, "Undo");
    assert_eq!(names_of(&ui), ["skatter", "long"]);
    assert!(!flow(&ui).can_undo());
    click(&mut ui, "Redo");
    save(&mut ui);
    let saved = on_disk(dir.path());
    let saved: Vec<&str> = saved.list().iter().map(|f| f.name.as_str()).collect();
    assert_eq!(saved, ["skatter", "long", "long 2", "Wide"]);
}

#[test]
fn what_is_copied_can_be_pasted_back() {
    let dir = profile();
    let (mut ui, _clipboard) = editor_with_clipboard(dir.path());
    click(&mut ui, "Copy all");
    click(&mut ui, "Paste");
    assert_eq!(names_of(&ui), ["skatter", "long", "skatter 2", "long 2"]);
    let list = flow(&ui).formations.list();
    assert_eq!(list[2].probes, list[0].probes);
    assert_eq!(list[3].probes, list[1].probes);
}

#[test]
fn pasting_text_with_anything_wrong_in_it_adds_nothing() {
    let dir = profile();
    let (mut ui, clipboard) = editor_with_clipboard(dir.path());
    let put = |text: &str| *clipboard.text.lock().unwrap() = Some(text.to_owned());

    // The first formation is fine; the second has a range the game lacks.
    put("formation: Fine\nprobe: 0 0 0 4\nformation: Odd\nprobe: 0 0 0 3\n");
    click(&mut ui, "Paste");
    assert_eq!(names_of(&ui), ["skatter", "long"]);
    assert!(!flow(&ui).is_dirty() && !flow(&ui).can_undo());
    assert_eq!(flow(&ui).share_note, None);
    ui.get_by_label(
        "Nothing was pasted: Formation 'Odd': a probe has a scan range the game doesn't offer.",
    );

    // Whatever else happened to be on the clipboard.
    put("some other text that was copied");
    click(&mut ui, "Paste");
    ui.get_by_label("Nothing was pasted: it doesn't hold probe formations copied from Clone Bay.");
    put("formation: Fine\nprobe: 1 2 3\n");
    click(&mut ui, "Paste");
    ui.get_by_label(
        "Nothing was pasted: line 2 isn't understood: a probe needs four numbers: north, west, \
         up, range.",
    );
    assert_eq!(names_of(&ui), ["skatter", "long"]);

    // A good paste afterwards clears the message.
    put("formation: Fine\nprobe: 0 0 0 4\n");
    click(&mut ui, "Paste");
    assert_eq!(names_of(&ui), ["skatter", "long", "Fine"]);
    assert_eq!(flow(&ui).problem, None);
}

#[test]
fn the_copy_buttons_need_something_to_copy() {
    let dir = profile();
    let (mut ui, clipboard) = editor_with_clipboard(dir.path());
    let disabled = |ui: &Ui, label: &str| ui.get_by_label(label).accesskit_node().is_disabled();
    assert!(!disabled(&ui, "Copy this one") && !disabled(&ui, "Copy all"));
    click(&mut ui, "Delete");
    click(&mut ui, "Delete");
    assert!(names_of(&ui).is_empty());
    assert!(disabled(&ui, "Copy this one") && disabled(&ui, "Copy all"));
    assert!(!disabled(&ui, "Paste"));
    // Asked anyway, nothing is put on the clipboard.
    flow_mut(&mut ui).copy_all();
    flow_mut(&mut ui).copy_current();
    assert_eq!(*clipboard.text.lock().unwrap(), None);
    assert_eq!(flow(&ui).share_note, None);
}

// This test was lost when the tests after it were rewritten for the
// clipboard, and mutation testing before the first release showed it was
// missing. It is back, with the cases that run found.
#[test]
fn a_typed_distance_moves_the_probe_straight_towards_or_away_from_the_centre() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    ui.get_by_label("Distance (km)");
    // "long" has its one probe at the centre: nothing to type a distance in.
    assert_eq!(
        flow(&ui).formations.get(13).unwrap().probes[0].distance(),
        0.0
    );

    click(&mut ui, "skatter  (2 probes)");
    let probes = |ui: &Ui| flow(ui).formations.get(2).unwrap().probes.clone();
    // Typing the distance a probe already has is not an edit.
    type_into(&mut ui, "412.311", "412.311");
    assert_eq!(probes(&ui)[1], Probe::at_km(-100.0, 400.0, 0.0, 0.25));
    assert!(!flow(&ui).is_dirty() && !flow(&ui).can_undo());

    // The second probe is 100 km east and 400 km up, 412.311 km from the
    // centre. At half that it is 50 km east and 200 km up.
    type_into(&mut ui, "412.311", "206.1555");
    let moved = probes(&ui)[1];
    assert!((moved.x + 50.0 * KM).abs() < 1.0, "{}", moved.x);
    assert!((moved.y - 200.0 * KM).abs() < 1.0, "{}", moved.y);
    assert_eq!(moved.z, 0.0);
    assert_eq!(moved.range, 0.25 * AU);
    // The other probe was left alone.
    assert_eq!(probes(&ui)[0], Probe::at_km(300.0, 0.0, 0.0, 0.25));
    // Typed in one go, so one step takes it all back.
    click(&mut ui, "Undo");
    assert_eq!(probes(&ui)[1], Probe::at_km(-100.0, 400.0, 0.0, 0.25));
    assert!(!flow(&ui).can_undo());
}

#[test]
fn a_name_typed_a_letter_at_a_time_is_one_step_to_undo() {
    let dir = profile();
    let mut ui = editor(dir.path(), EveStatus::NotRunning);
    click(&mut ui, "skatter  (2 probes)");
    let name = |ui: &Ui| flow(ui).formations.get(2).unwrap().name.clone();
    type_into(&mut ui, "skatter", "a");
    assert_eq!(name(&ui), "a");
    // A second letter, in a frame of its own, straight afterwards.
    ui.get_all_by_value("a")
        .find(|node| node.accesskit_node().role() == Role::TextInput)
        .unwrap()
        .type_text("b");
    settle(&mut ui);
    assert_eq!(name(&ui), "ab");
    click(&mut ui, "Undo");
    assert_eq!(name(&ui), "skatter");
    assert!(!flow(&ui).can_undo());
}
