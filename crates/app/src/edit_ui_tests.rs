//! Tests that drive the settings edit screen in
//! the real UI, off-screen, against files in a temp folder.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use clone_bay_core::discovery::FileKind;
use clone_bay_core::edit::{read, settings_for, Content, Reading};
use clone_bay_core::platform::EveStatus;
use clone_bay_marshal::{decode, encode, Long, Value};
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};

use crate::app::test_support::{
    click, closing, harness_with, press_esc, press_mouse_back, press_window_close, settle, shows,
    wait_until, Ui,
};
use crate::edit_flow::EditFlow;
use crate::services::test_support::{EveGate, FakeNames};
use crate::services::Services;

fn b(text: &str) -> Value {
    Value::Bytes(text.as_bytes().to_vec())
}

fn stamped(value: Value) -> Value {
    Value::Tuple(vec![Value::Long(Long::from_i64(1)), value])
}

fn account() -> Value {
    Value::Dict(vec![
        (
            b("ui"),
            Value::Dict(vec![
                (b("cameraShakeEnabled"), stamped(Value::Int(1))),
                (b("autoTargetBack"), stamped(Value::Int(2))),
                (b("windowTransparency"), stamped(Value::Float(0.25))),
                (b("untouched"), stamped(b("leave me"))),
            ]),
        ),
        (
            b("audio"),
            Value::Dict(vec![
                // Stored in two places that disagree: can't be edited.
                (b("custom_uiclick"), stamped(Value::Float(0.5))),
                (b("soundLevel_custom_uiclick"), stamped(Value::Float(0.75))),
            ]),
        ),
    ])
}

fn character() -> Value {
    let distances = Value::Dict(vec![
        (Value::Int(0), Value::Int(5_000)),
        (Value::Int(638), Value::Int(20_000)),
        (Value::Int(999), Value::Int(750)),
    ]);
    Value::Dict(vec![(
        b("ui"),
        Value::Dict(vec![
            (b("droneAggression"), stamped(Value::Bool(true))),
            (b("defaultTypeOrbitDist"), stamped(distances)),
        ]),
    )])
}

fn profile() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let put = |name: &str, value: &Value| {
        fs::write(dir.path().join(name), encode(value, false).unwrap()).unwrap();
    };
    put("core_user_1.dat", &account());
    put("core_char_2.dat", &character());
    fs::write(dir.path().join("prefs.ini"), "debug=0\r\nport=26000").unwrap();
    dir
}

fn services(eve: EveStatus) -> Services {
    Services {
        eve_status: Arc::new(move || eve),
        type_name_source: FakeNames::knowing(&[(638, "Raven")]),
        ..Services::for_tests()
    }
}

/// The app with the edit screen open on the file whose ID is `id`.
fn edit_screen(dir: &Path, eve: EveStatus, id: &str) -> Ui {
    let mut ui = harness_with(dir, services(eve));
    click(&mut ui, id);
    click(&mut ui, "Edit settings");
    assert!(ui.state().state.edit().is_some());
    ui
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

const NOT_STORED: &str = "not stored in this file; the game is using its default";

fn flow(ui: &Ui) -> &EditFlow {
    ui.state().state.edit().unwrap()
}

fn flow_mut(ui: &mut Ui) -> &mut EditFlow {
    ui.state_mut().state.edit_mut().unwrap()
}

fn save(ui: &mut Ui) {
    click(ui, "Save changes");
    wait_until(ui, |app| app.state.edit().is_some_and(|flow| !flow.busy()));
}

/// What a setting holds in the file on disk right now.
fn on_disk(dir: &Path, file: &str, kind: FileKind, id: &str) -> Reading {
    let root = decode(&fs::read(dir.join(file)).unwrap()).unwrap().value;
    let setting = settings_for(kind)
        .iter()
        .find(|setting| setting.id == id)
        .unwrap();
    read(&root, setting)
}

fn set_draft(ui: &mut Ui, id: &str, content: Content) {
    let row = flow_mut(ui)
        .rows
        .iter_mut()
        .find(|row| row.setting.id == id)
        .unwrap();
    row.draft = Some(content);
    row.touched = true;
}

#[test]
fn the_edit_screen_lists_the_settings_under_headings() {
    let dir = profile();
    let ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    ui.get_by_label("Settings for account 1");
    for heading in [
        "Gameplay",
        "Camera",
        "Effects in space",
        "Scanning",
        "Asteroid field effects",
        "Sound levels",
    ] {
        ui.get_by_label(heading);
    }
    ui.get_by_label("Camera shake");
    ui.get_by_label("Auto target back (number of targets)");
    // Settings this file doesn't hold say so; one stored oddly can't be edited.
    assert!(
        ui.get_all_by_label("not stored in this file; the game is using its default")
            .count()
            > 5
    );
    ui.get_by_label("can't be edited: it is stored in a form this app doesn't understand");
    // Nothing has been changed yet.
    assert!(ui
        .get_by_label("Save changes")
        .accesskit_node()
        .is_disabled());
    assert!(ui
        .get_by_label("Discard changes")
        .accesskit_node()
        .is_disabled());
    assert!(!flow(&ui).is_dirty());
    assert!(!shows(&ui, "Drones"), "that is a character heading");
}

#[test]
fn a_changed_switch_is_saved_with_a_backup() {
    let dir = profile();
    let before = fs::read(dir.path().join("core_user_1.dat")).unwrap();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    click(&mut ui, "Camera shake");
    ui.get_by_label("1 change not saved");
    assert!(!ui
        .get_by_label("Save changes")
        .accesskit_node()
        .is_disabled());
    assert_eq!(
        fs::read(dir.path().join("core_user_1.dat")).unwrap(),
        before,
        "not saved yet"
    );

    save(&mut ui);
    let backup = dir.path().join("clone-bay-backups").join("T");
    ui.get_by_label(format!("Saved. The original was backed up in {}", backup.display()).as_str());
    assert_eq!(fs::read(backup.join("core_user_1.dat")).unwrap(), before);
    let kind = FileKind::Account;
    assert_eq!(
        on_disk(dir.path(), "core_user_1.dat", kind, "camera_shake"),
        Reading::Stored(Content::Switch(false))
    );
    // Everything else is as it was.
    assert_eq!(
        on_disk(dir.path(), "core_user_1.dat", kind, "auto_target_back"),
        Reading::Stored(Content::Whole(2))
    );
    // The screen now shows the saved file: nothing left to save.
    assert!(!flow(&ui).is_dirty());
    assert!(ui
        .get_by_label("Save changes")
        .accesskit_node()
        .is_disabled());
    assert!(!shows(&ui, "1 change not saved"));

    // Back at the list, which has been refreshed.
    click(&mut ui, "Back to the file list");
    assert!(ui.state().state.edit().is_none());
    ui.get_by_label("Rescan");
}

#[test]
fn numbers_and_whole_values_are_saved() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    set_draft(&mut ui, "auto_target_back", Content::Whole(5));
    set_draft(&mut ui, "window_transparency", Content::Number(0.75));
    settle(&mut ui);
    ui.get_by_label("2 changes not saved");
    save(&mut ui);
    let kind = FileKind::Account;
    assert_eq!(
        on_disk(dir.path(), "core_user_1.dat", kind, "auto_target_back"),
        Reading::Stored(Content::Whole(5))
    );
    assert_eq!(
        on_disk(dir.path(), "core_user_1.dat", kind, "window_transparency"),
        Reading::Stored(Content::Number(0.75))
    );
}

#[test]
fn a_setting_the_file_did_not_store_is_only_written_if_touched() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    // Showing a starting value for it is not a change.
    let trails = |ui: &Ui| {
        flow(ui)
            .rows
            .iter()
            .find(|row| row.setting.id == "trails")
            .cloned()
            .unwrap()
    };
    assert_eq!(trails(&ui).original, Reading::NotStored);
    assert_eq!(trails(&ui).draft, Some(Content::Switch(false)));
    assert_eq!(trails(&ui).change(), None);

    // Ticking and unticking leaves it "off", but now deliberately so.
    click(&mut ui, "Trails");
    click(&mut ui, "Trails");
    assert_eq!(trails(&ui).draft, Some(Content::Switch(false)));
    ui.get_by_label("1 change not saved");
    save(&mut ui);
    assert_eq!(
        on_disk(dir.path(), "core_user_1.dat", FileKind::Account, "trails"),
        Reading::Stored(Content::Switch(false))
    );
}

#[test]
fn discarding_puts_every_control_back() {
    let dir = profile();
    let before = fs::read(dir.path().join("core_user_1.dat")).unwrap();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    click(&mut ui, "Camera shake");
    click(&mut ui, "Trails");
    ui.get_by_label("2 changes not saved");
    click(&mut ui, "Discard changes");
    assert!(!flow(&ui).is_dirty());
    assert!(!shows(&ui, "2 changes not saved"));
    click(&mut ui, "Back to the file list");
    assert_eq!(
        fs::read(dir.path().join("core_user_1.dat")).unwrap(),
        before
    );
    assert!(!dir.path().join("clone-bay-backups").join("T").exists());
}

#[test]
fn nothing_is_saved_while_eve_is_running() {
    let dir = profile();
    let before = fs::read(dir.path().join("core_user_1.dat")).unwrap();
    let mut ui = edit_screen(dir.path(), EveStatus::Running, "1");
    click(&mut ui, "Camera shake");
    save(&mut ui);
    ui.get_by_label(
        "EVE is running. Close every EVE client first: the client saves its settings when it \
         closes and would overwrite the change. Nothing was changed.",
    );
    assert_eq!(
        fs::read(dir.path().join("core_user_1.dat")).unwrap(),
        before
    );
    // The change is still there to save once EVE is closed.
    ui.get_by_label("1 change not saved");
    assert!(!shows(&ui, "I have closed every EVE client"));
}

/// What every screen that saves one file says when the file changed after
/// the screen opened it.
pub const CHANGED_SINCE_OPENED: &str = "The file changed after it was opened (EVE may have saved \
                                        it), so nothing was saved. Press \"Discard changes\" to \
                                        load it again.";

#[test]
fn a_file_the_game_changed_after_the_screen_opened_is_not_saved_over() {
    let dir = profile();
    let path = dir.path().join("core_user_1.dat");
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    click(&mut ui, "Camera shake");
    // EVE was started, changed a setting, and closed again.
    let mut by_the_game = account();
    if let Value::Dict(sections) = &mut by_the_game {
        if let (_, Value::Dict(ui_section)) = &mut sections[0] {
            ui_section[1].1 = stamped(Value::Int(4));
        }
    }
    let game_bytes = encode(&by_the_game, false).unwrap();
    fs::write(&path, &game_bytes).unwrap();

    save(&mut ui);
    ui.get_by_label(CHANGED_SINCE_OPENED);
    assert_eq!(fs::read(&path).unwrap(), game_bytes);
    assert!(!dir.path().join("clone-bay-backups").exists());
    ui.get_by_label("1 change not saved");

    // Discarding shows what the game wrote, and that can be saved over.
    click(&mut ui, "Discard changes");
    assert!(!shows(&ui, CHANGED_SINCE_OPENED));
    assert_eq!(
        flow(&ui)
            .rows
            .iter()
            .find(|row| row.setting.id == "auto_target_back")
            .unwrap()
            .original,
        Reading::Stored(Content::Whole(4))
    );
    click(&mut ui, "Camera shake");
    save(&mut ui);
    assert_eq!(
        on_disk(
            dir.path(),
            "core_user_1.dat",
            FileKind::Account,
            "camera_shake"
        ),
        Reading::Stored(Content::Switch(false))
    );
    assert_eq!(
        on_disk(
            dir.path(),
            "core_user_1.dat",
            FileKind::Account,
            "auto_target_back"
        ),
        Reading::Stored(Content::Whole(4))
    );
}

#[test]
fn saving_twice_while_the_app_is_open_keeps_one_backup() {
    let dir = profile();
    let before = fs::read(dir.path().join("core_user_1.dat")).unwrap();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    let kind = FileKind::Account;
    for expected in [false, true] {
        click(&mut ui, "Camera shake");
        save(&mut ui);
        assert_eq!(
            on_disk(dir.path(), "core_user_1.dat", kind, "camera_shake"),
            Reading::Stored(Content::Switch(expected))
        );
    }

    let all = dir.path().join("clone-bay-backups");
    let backup = all.join("T");
    ui.get_by_label(format!("Saved. The original was backed up in {}", backup.display()).as_str());
    assert_eq!(fs::read_dir(&all).unwrap().count(), 1);
    assert_eq!(fs::read(backup.join("core_user_1.dat")).unwrap(), before);
}

#[test]
fn when_eve_cannot_be_checked_the_user_must_confirm_it_is_closed() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::Unknown, "1");
    click(&mut ui, "Camera shake");
    save(&mut ui);
    let kind = FileKind::Account;
    assert_eq!(
        on_disk(dir.path(), "core_user_1.dat", kind, "camera_shake"),
        Reading::Stored(Content::Switch(true))
    );
    assert!(!flow(&ui).can_save());
    assert!(ui
        .get_by_label("Save changes")
        .accesskit_node()
        .is_disabled());

    click(&mut ui, "I have closed every EVE client");
    assert!(flow(&ui).can_save());
    save(&mut ui);
    assert_eq!(
        on_disk(dir.path(), "core_user_1.dat", kind, "camera_shake"),
        Reading::Stored(Content::Switch(false))
    );
    assert!(!shows(&ui, "I have closed every EVE client"));
}

#[test]
fn distances_show_type_names_and_can_be_changed_and_removed() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "2");
    // The Raven is in the list built into the app; only type 999 is asked
    // about.
    wait_until(&mut ui, |app| !app.state.names.item_type_needs_lookup(999));
    ui.get_by_label("Settings for character 2");
    ui.get_by_label("Orbit (3 entries)");
    ui.get_by_label("Everything else (default)");
    ui.get_by_label("Raven");
    // A type the name service doesn't know keeps its number.
    ui.get_by_label("Type 999");
    ui.get_by_label("20 km");
    assert_eq!(flow(&ui).type_ids(), [638, 999]);

    // Change one distance and remove another entry.
    set_draft(
        &mut ui,
        "orbit_distances",
        Content::Distances(vec![(0, 7_500), (638, 20_000)]),
    );
    settle(&mut ui);
    ui.get_by_label("Orbit (2 entries)");
    ui.get_by_label("7.5 km");
    save(&mut ui);
    assert_eq!(
        on_disk(
            dir.path(),
            "core_char_2.dat",
            FileKind::Character,
            "orbit_distances"
        ),
        Reading::Stored(Content::Distances(vec![(0, 7_500), (638, 20_000)]))
    );
    assert_eq!(
        on_disk(
            dir.path(),
            "core_char_2.dat",
            FileKind::Character,
            "drone_aggression"
        ),
        Reading::Stored(Content::Switch(true))
    );
}

#[test]
fn the_remove_button_takes_an_entry_out_of_the_list() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "2");
    // Three entries, but the default for everything else has no button.
    assert_eq!(ui.get_all_by_label("Remove").count(), 2);
    let at = ui
        .get_all_by_label("Remove")
        .next()
        .unwrap()
        .rect()
        .center();
    ui.hover_at(at);
    ui.step();
    ui.drag_at(at);
    ui.step();
    ui.drop_at(at);
    settle(&mut ui);
    ui.get_by_label("Orbit (2 entries)");
    // The first ship in the list went; the default is still there.
    assert!(!shows(&ui, "Raven"), "the first ship was removed");
    ui.get_by_label("Everything else (default)");
    ui.get_by_label("1 change not saved");
}

#[test]
fn a_file_that_cannot_be_opened_says_why() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    // The file is damaged while the screen is open; discarding re-reads it.
    click(&mut ui, "Camera shake");
    fs::write(dir.path().join("core_user_1.dat"), b"damaged").unwrap();
    click(&mut ui, "Discard changes");
    ui.get_by_label(
        "Can't read core_user_1.dat: can't decode it (not a marshal stream (bad signature)).",
    );
    assert!(flow(&ui).rows.is_empty());
    assert!(!flow(&ui).can_save());
}

#[test]
fn a_damaged_file_cannot_be_edited() {
    let dir = profile();
    fs::write(dir.path().join("core_user_3.dat"), b"damaged").unwrap();
    let mut ui = harness_with(dir.path(), services(EveStatus::NotRunning));
    click(&mut ui, "3");
    assert!(ui
        .get_by_label("Edit settings")
        .accesskit_node()
        .is_disabled());
    click(&mut ui, "Edit settings");
    assert!(ui.state().state.edit().is_none());
}

// The three tests of the "Always show bracket text" tick box were removed
// with the tick box itself. The code
// that reads and writes the option is still in `core::prefs`, with its own
// tests.

#[test]
fn each_kind_of_setting_gets_its_own_control() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    // 38 settings: 3 stored, 1 unreadable, 34 not stored. All but the scan
    // filters, which say "none" instead, carry the note.
    assert_eq!(ui.get_all_by_label(NOT_STORED).count(), 33);
    // Numbers are typed into plain boxes: eleven of them (two gameplay
    // numbers, the scan range and angle, and seven readable sound levels)
    // and "auto target back". The
    // user wants nothing that changes a number by dragging, so there are no
    // sliders and no drag boxes.
    assert_eq!(ui.query_all_by_role(Role::Slider).count(), 0);
    assert_eq!(ui.query_all_by_role(Role::SpinButton).count(), 0);
    assert_eq!(ui.get_all_by_role(Role::TextInput).count(), 12);
    // The warp-to distance and the scan preset.
    assert_eq!(ui.get_all_by_role(Role::ComboBox).count(), 2);
    ui.get_by_value("0 km");
    assert!(!shows(&ui, "0 changes not saved"));

    // Once a control has been used, its "not stored" note goes.
    click(&mut ui, "Trails");
    assert_eq!(ui.get_all_by_label(NOT_STORED).count(), 32);
    // The asteroid field effects are tick boxes like the other effects.
    click(&mut ui, "God rays");
    assert_eq!(ui.get_all_by_label(NOT_STORED).count(), 31);
    ui.get_by_label("2 changes not saved");
}

#[test]
fn the_drop_down_changes_the_setting() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    let drop_down = ui.get_by_value("0 km").rect().center();
    click_at(&mut ui, drop_down);
    click(&mut ui, "30 km");
    let row = |ui: &Ui| {
        let rows = &flow(ui).rows;
        rows.iter()
            .find(|row| row.setting.id == "default_warp_to")
            .cloned()
            .unwrap()
    };
    assert_eq!(row(&ui).draft, Some(Content::Whole(30_000)));
    assert!(row(&ui).touched);
    ui.get_by_label("1 change not saved");
    save(&mut ui);
    assert_eq!(
        on_disk(
            dir.path(),
            "core_user_1.dat",
            FileKind::Account,
            "default_warp_to"
        ),
        Reading::Stored(Content::Whole(30_000))
    );
}

// The test that dragged a distance box was removed with the dragging:
// distances are typed only. What may be typed is tested with `parse_distance` in
// `edit_view.rs`.

#[test]
fn type_names_learned_on_the_edit_screen_are_saved() {
    let dir = profile();
    let data = tempfile::tempdir().unwrap();
    let file = data.path().join("names.tsv");
    let services = Services {
        names_file: Some(file.clone()),
        ..services(EveStatus::NotRunning)
    };
    let mut ui = harness_with(dir.path(), services);
    click(&mut ui, "2");
    click(&mut ui, "Edit settings");
    wait_until(&mut ui, |app| !app.state.names.item_type_needs_lookup(999));
    let saved = fs::read_to_string(&file).unwrap();
    // The type nobody knows is remembered, so it isn't asked about again.
    assert!(saved.contains("no-type\t999\n"), "{saved}");
    // The Raven is in the list built into the app, so it was never asked
    // about and isn't saved.
    assert!(!saved.contains("\t638"), "{saved}");

    // A type the name service does know is saved with its name.
    let file = data.path().join("known.tsv");
    let services = Services {
        names_file: Some(file.clone()),
        eve_status: Arc::new(|| EveStatus::NotRunning),
        type_name_source: FakeNames::knowing(&[(999, "Mystery Hull")]),
        ..Services::for_tests()
    };
    let mut ui = harness_with(dir.path(), services);
    click(&mut ui, "2");
    click(&mut ui, "Edit settings");
    wait_until(&mut ui, |app| app.state.names.item_type(999).is_some());
    let saved = fs::read_to_string(&file).unwrap();
    assert!(saved.contains("type\t999\tMystery Hull\n"), "{saved}");
    ui.get_by_label("Mystery Hull");
}

#[test]
fn the_file_list_is_refreshed_only_after_a_save() {
    let dir = profile();
    let before = fs::metadata(dir.path().join("core_user_1.dat"))
        .unwrap()
        .len();

    // Leaving without saving doesn't re-list the folder: a file that
    // appeared meanwhile isn't picked up.
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    click(&mut ui, "Camera shake");
    click(&mut ui, "Discard changes");
    fs::write(dir.path().join("core_user_7.dat"), b"new").unwrap();
    click(&mut ui, "Back to the file list");
    assert_eq!(ui.state().state.rows.len(), 2);
    fs::remove_file(dir.path().join("core_user_7.dat")).unwrap();

    // After a save the list shows the file as it is now.
    click(&mut ui, "Edit settings");
    click(&mut ui, "Trails");
    save(&mut ui);
    click(&mut ui, "Back to the file list");
    let size = fs::metadata(dir.path().join("core_user_1.dat"))
        .unwrap()
        .len();
    assert_ne!(size, before);
    let rows = &ui.state().state.rows;
    let row = rows.iter().find(|row| row.entry.id == 1).unwrap();
    assert_eq!(row.entry.size, size);
}

// The tests below were written with the "clear all", "add ship" and sorting
// changes and had not been run when they were committed.

#[test]
fn distances_are_listed_by_name_with_the_default_first() {
    use crate::edit_flow::distance_order;
    use clone_bay_core::names::{CharacterName, NameBook};

    let mut names = NameBook::default();
    names.set_item_type(638, CharacterName::Known("Raven".to_owned()));
    names.set_item_type(24_690, CharacterName::Known("abaddon".to_owned()));
    names.set_item_type(17_738, CharacterName::Known("Machariel".to_owned()));
    // Stored in no particular order, with two types nobody has named.
    let entries = [
        (638, 1),
        (999, 2),
        (0, 3),
        (17_738, 4),
        (24_690, 5),
        (50, 6),
    ];
    let shown: Vec<i64> = distance_order(&entries, &names)
        .into_iter()
        .map(|index| entries[index].0)
        .collect();
    // The default, then abaddon, Machariel, Raven, then unnamed by number.
    assert_eq!(shown, [0, 24_690, 17_738, 638, 50, 999]);
    assert!(distance_order(&[], &names).is_empty());
}

#[test]
fn clearing_a_distance_list_asks_first() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "2");
    let orbit = |ui: &Ui| {
        let rows = &flow(ui).rows;
        let row = rows
            .iter()
            .find(|row| row.setting.id == "orbit_distances")
            .unwrap();
        row.draft.clone()
    };
    // Three lists, but only the one with entries can be cleared.
    let clear = ui
        .get_all_by_label("Clear all")
        .next()
        .unwrap()
        .rect()
        .center();
    click_at(&mut ui, clear);
    // The default for everything else isn't one of the things removed.
    ui.get_by_label("Remove all 2 ships from Orbit? The default stays.");
    assert!(matches!(orbit(&ui), Some(Content::Distances(entries)) if entries.len() == 3));

    click(&mut ui, "No, keep them");
    assert!(!shows(
        &ui,
        "Remove all 2 ships from Orbit? The default stays."
    ));
    assert!(matches!(orbit(&ui), Some(Content::Distances(entries)) if entries.len() == 3));
    assert!(!flow(&ui).is_dirty());

    let clear = ui
        .get_all_by_label("Clear all")
        .next()
        .unwrap()
        .rect()
        .center();
    click_at(&mut ui, clear);
    click(&mut ui, "Yes, clear all");
    // The ships are gone; the default is still there with its distance.
    let Some(Content::Distances(left)) = orbit(&ui) else {
        panic!("not distances");
    };
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].0, 0);
    ui.get_by_label("Orbit (1 entry)");
    ui.get_by_label("Everything else (default)");
    ui.get_by_label("1 change not saved");
    // With only the default left there is nothing more to clear or remove.
    assert!(ui
        .get_all_by_label("Clear all")
        .all(|button| button.accesskit_node().is_disabled()));
    assert!(!shows(&ui, "Remove"));
    save(&mut ui);
    assert_eq!(
        on_disk(
            dir.path(),
            "core_char_2.dat",
            FileKind::Character,
            "orbit_distances"
        ),
        Reading::Stored(Content::Distances(left))
    );
}

fn orbit_row(ui: &Ui) -> (usize, crate::edit_flow::Row) {
    let rows = &flow(ui).rows;
    let index = rows
        .iter()
        .position(|row| row.setting.id == "orbit_distances")
        .unwrap();
    (index, rows[index].clone())
}

#[test]
fn a_ship_picked_from_the_built_in_list_starts_at_500_metres() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "2");
    // Nothing is offered until something is typed.
    assert!(!shows(&ui, "Machariel"));
    let (index, _) = orbit_row(&ui);
    flow_mut(&mut ui).rows[index].add_input = "machar".to_owned();
    settle(&mut ui);
    // How to add one is shown beside the box without hovering, never as a
    // tooltip that would cover the ships offered.
    let hint = "Pick a ship below to add it at 500 m, then change the distance";
    assert!(ui.query_all_by_label(hint).next().is_some());
    click(&mut ui, "Machariel");

    let (_, row) = orbit_row(&ui);
    assert_eq!(
        row.draft,
        Some(Content::Distances(vec![
            (0, 5_000),
            (638, 20_000),
            (999, 750),
            (17_738, 500)
        ]))
    );
    assert!(row.touched);
    assert_eq!(row.add_input, "", "the box is emptied for the next one");
    ui.get_by_label("Orbit (4 entries)");
    ui.get_by_label("1 change not saved");

    save(&mut ui);
    let saved = on_disk(
        dir.path(),
        "core_char_2.dat",
        FileKind::Character,
        "orbit_distances",
    );
    let Reading::Stored(Content::Distances(entries)) = saved else {
        panic!("not stored");
    };
    assert!(entries.contains(&(17_738, 500)), "{entries:?}");
}

#[test]
fn a_ship_already_in_the_list_is_not_added_twice() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "2");
    let (index, _) = orbit_row(&ui);
    flow_mut(&mut ui).add_type(index, 638, "Raven");
    let (_, row) = orbit_row(&ui);
    assert_eq!(
        row.add_note.as_deref(),
        Some("Raven is already in the list.")
    );
    assert!(matches!(&row.draft, Some(Content::Distances(entries)) if entries.len() == 3));
    assert!(!flow(&ui).is_dirty());
    // Adding to a row that isn't a distance list, or doesn't exist, does nothing.
    flow_mut(&mut ui).add_type(0, 638, "Raven");
    flow_mut(&mut ui).add_type(99, 638, "Raven");
    assert!(!flow(&ui).is_dirty());
}

#[test]
fn typing_something_no_ship_is_called_says_so() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "2");
    let (index, _) = orbit_row(&ui);
    flow_mut(&mut ui).rows[index].add_input = "zzzzqq".to_owned();
    settle(&mut ui);
    ui.get_by_label("No ship or structure has that in its name.");
}

#[test]
fn built_in_ship_names_need_no_lookup() {
    let dir = profile();
    // The name service knows nothing, yet type 638 is shown as Raven.
    let services = Services {
        type_name_source: FakeNames::knowing(&[]),
        ..services(EveStatus::NotRunning)
    };
    let mut ui = harness_with(dir.path(), services);
    click(&mut ui, "2");
    click(&mut ui, "Edit settings");
    ui.get_by_label("Raven");
}

/// Types `text` into the box that currently shows `shown`.
fn type_into(ui: &mut Ui, shown: &str, text: &str) {
    use eframe::egui::accesskit::Role;
    use egui_kittest::kittest::NodeT;
    let find = |ui: &Ui| {
        ui.get_all_by_value(shown)
            .find(|node| node.accesskit_node().role() == Role::TextInput)
            .unwrap()
            .rect()
            .center()
    };
    let at = find(ui);
    click_at(ui, at);
    ui.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    ui.step();
    ui.get_all_by_value(shown)
        .find(|node| node.accesskit_node().role() == Role::TextInput)
        .unwrap()
        .type_text(text);
    settle(ui);
}

#[test]
fn a_typed_distance_changes_the_setting_within_its_limits() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "2");
    let orbit = |ui: &Ui| {
        let row = flow(ui)
            .rows
            .iter()
            .find(|row| row.setting.id == "orbit_distances")
            .unwrap();
        row.draft.clone()
    };
    let before = orbit(&ui);
    assert!(!shows(&ui, "1 change not saved"));

    // The Raven's 20,000 m becomes 25,000 m.
    type_into(&mut ui, "20000", "25000");
    let Some(Content::Distances(now)) = orbit(&ui) else {
        panic!("not distances");
    };
    assert!(now.contains(&(638, 25_000)), "{now:?}");
    ui.get_by_label("1 change not saved");

    // Below the 500 m floor is not taken: the setting keeps what it had.
    type_into(&mut ui, "25000", "400");
    let Some(Content::Distances(now)) = orbit(&ui) else {
        panic!("not distances");
    };
    assert!(now.contains(&(638, 25_000)), "{now:?}");
    assert_ne!(orbit(&ui), before);
}

#[test]
fn number_settings_are_typed_and_held_to_their_range() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    let draft = |ui: &Ui, id: &str| {
        let row = flow(ui).rows.iter().find(|row| row.setting.id == id);
        row.unwrap().draft.clone()
    };
    let Some(Content::Number(before)) = draft(&ui, "window_transparency") else {
        panic!("not a number");
    };
    let shown = crate::edit_flow::format_decimal(before);
    // Each range is shown beside its box.
    assert!(ui.get_all_by_label("0 to 1").count() >= 1);

    // The file holds 0.25.
    assert_eq!(shown, "0.25");
    type_into(&mut ui, &shown, "0.4");
    assert_eq!(
        draft(&ui, "window_transparency"),
        Some(Content::Number(0.4))
    );
    ui.get_by_label("1 change not saved");
    // Out of range is not taken: the setting keeps what it had.
    type_into(&mut ui, "0.4", "7");
    assert_eq!(
        draft(&ui, "window_transparency"),
        Some(Content::Number(0.4))
    );

    save(&mut ui);
    assert_eq!(
        on_disk(
            dir.path(),
            "core_user_1.dat",
            FileKind::Account,
            "window_transparency"
        ),
        Reading::Stored(Content::Number(0.4))
    );
}

// Found by mutation testing before the first release: nothing typed a whole
// number, and nothing checked that typing a value again changes nothing.
#[test]
fn a_whole_number_is_typed_and_typing_the_same_value_is_not_a_change() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    let row = |ui: &Ui, id: &str| {
        let row = flow(ui).rows.iter().find(|row| row.setting.id == id);
        row.unwrap().clone()
    };
    // The file holds 2 for "auto target back" and 0.25 for transparency.
    // Typing what is already there touches nothing.
    type_into(&mut ui, "2", "2");
    type_into(&mut ui, "0.25", "0.25");
    assert!(!row(&ui, "auto_target_back").touched);
    assert!(!row(&ui, "window_transparency").touched);
    assert!(!flow(&ui).is_dirty());

    type_into(&mut ui, "2", "4");
    assert_eq!(row(&ui, "auto_target_back").draft, Some(Content::Whole(4)));
    assert!(row(&ui, "auto_target_back").touched);
    ui.get_by_label("1 change not saved");
    // Out of its range (0 to 5) is not taken.
    type_into(&mut ui, "4", "9");
    assert_eq!(row(&ui, "auto_target_back").draft, Some(Content::Whole(4)));
}

#[test]
fn typing_a_distance_that_is_already_there_is_not_a_change() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "2");
    type_into(&mut ui, "20000", "20000");
    let orbit = flow(&ui)
        .rows
        .iter()
        .find(|row| row.setting.id == "orbit_distances")
        .unwrap();
    assert!(!orbit.touched);
    assert!(!flow(&ui).is_dirty());
}

/// A profile whose character 2 has autopilot settings: the "safe" route and
/// two systems to avoid, one of which the name service knows.
fn pilot_profile() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let systems = Value::List(vec![
        Value::Int(30_000_142),
        Value::Long(Long::from_i64(30_000_318)),
    ]);
    let character = Value::Dict(vec![(
        b("ui"),
        Value::Dict(vec![
            (b("pfRouteType"), stamped(b("safe"))),
            (b("pathFinder_includeJumpGates"), stamped(Value::Bool(true))),
            (b("autopilot_avoidance2"), stamped(systems)),
        ]),
    )]);
    fs::write(
        dir.path().join("core_char_2.dat"),
        encode(&character, false).unwrap(),
    )
    .unwrap();
    dir
}

fn pilot_screen(dir: &Path) -> Ui {
    let services = Services {
        eve_status: Arc::new(|| EveStatus::NotRunning),
        type_name_source: FakeNames::knowing(&[(30_000_142, "Jita")]),
        ..Services::for_tests()
    };
    let mut ui = harness_with(dir, services);
    click(&mut ui, "2");
    click(&mut ui, "Edit settings");
    wait_until(&mut ui, |app| {
        app.state.names.item_type(30_000_142).is_some()
    });
    ui
}

fn avoided(dir: &Path) -> Vec<i64> {
    match on_disk(
        dir,
        "core_char_2.dat",
        FileKind::Character,
        "avoided_systems",
    ) {
        Reading::Stored(Content::Systems(systems)) => {
            systems.iter().map(|system| system.id).collect()
        }
        other => panic!("avoided systems read as {other:?}"),
    }
}

#[test]
fn avoided_systems_are_shown_by_name_and_can_be_removed_one_at_a_time() {
    let dir = pilot_profile();
    let mut ui = pilot_screen(dir.path());
    ui.get_by_label("Autopilot");
    ui.get_by_label("Systems to avoid (2 systems)");
    ui.get_by_label("Jita");
    // A system the name service doesn't know is shown by its number.
    ui.get_by_label("System 30000318");
    assert_eq!(flow(&ui).type_ids(), [30_000_142, 30_000_318]);
    // There is nothing to type a system into: they can only be taken off.
    assert!(!shows(&ui, "Add system"));

    let first = ui
        .get_all_by_label("Remove system")
        .next()
        .unwrap()
        .rect()
        .center();
    click_at(&mut ui, first);
    assert!(!shows(&ui, "Jita"));
    ui.get_by_label("Systems to avoid (1 system)");
    ui.get_by_label("1 change not saved");
    save(&mut ui);
    assert!(flow(&ui).problem.is_none(), "{:?}", flow(&ui).problem);
    assert_eq!(avoided(dir.path()), [30_000_318]);
}

#[test]
fn every_avoided_system_can_be_removed_at_once() {
    let dir = pilot_profile();
    let mut ui = pilot_screen(dir.path());
    click(&mut ui, "Remove all systems");
    ui.get_by_label("Systems to avoid (0 systems)");
    ui.get_by_label("none; systems are added in the game");
    assert!(!shows(&ui, "Remove system"));
    assert!(!shows(&ui, "Remove all systems"));

    // Discarding brings them back; saving takes them out of the file.
    click(&mut ui, "Discard changes");
    ui.get_by_label("Systems to avoid (2 systems)");
    click(&mut ui, "Remove all systems");
    save(&mut ui);
    assert!(flow(&ui).problem.is_none(), "{:?}", flow(&ui).problem);
    assert_eq!(avoided(dir.path()), Vec::<i64>::new());
}

#[test]
fn the_route_and_jump_gates_can_be_changed() {
    let dir = pilot_profile();
    let mut ui = pilot_screen(dir.path());
    let drop_down = ui.get_by_value("Prefer safer").rect().center();
    click_at(&mut ui, drop_down);
    click(&mut ui, "Shortest");
    click(&mut ui, "Use Ansiblex in routes");
    ui.get_by_label("2 changes not saved");
    save(&mut ui);
    assert!(flow(&ui).problem.is_none(), "{:?}", flow(&ui).problem);
    let character = FileKind::Character;
    assert_eq!(
        on_disk(dir.path(), "core_char_2.dat", character, "route_type"),
        Reading::Stored(Content::Word("shortest".to_owned()))
    );
    assert_eq!(
        on_disk(dir.path(), "core_char_2.dat", character, "route_jump_gates"),
        Reading::Stored(Content::Switch(false))
    );
    // The avoid list wasn't touched.
    assert_eq!(avoided(dir.path()), [30_000_142, 30_000_318]);
}

#[test]
fn a_character_with_no_autopilot_settings_stored_has_nothing_to_remove() {
    let dir = profile();
    let ui = edit_screen(dir.path(), EveStatus::NotRunning, "2");
    ui.get_by_label("Systems to avoid (0 systems)");
    ui.get_by_label("none; systems are added in the game");
    assert!(!shows(&ui, "Remove all systems"));
    assert!(flow(&ui).changes().is_empty());
}

/// A profile whose account 1 has scanning settings: the longest range, a
/// full-circle angle, a preset in use out of three, and two scan filters,
/// one of them switched on.
fn scanner_profile() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let filter = |name: &str, groups: &[i64]| {
        Value::Tuple(vec![
            Value::Str(name.to_owned()),
            Value::List(groups.iter().map(|group| Value::Int(*group)).collect()),
        ])
    };
    let active = Value::Reduce(Box::new(clone_bay_marshal::Reduce {
        callable: Some(Value::Global("__builtin__.set".into())),
        args: Value::Tuple(vec![Value::List(vec![Value::Int(16), Value::Int(-1)])]),
        state: None,
        list_items: vec![],
        dict_items: vec![],
    }));
    let preset = |name: &str| (Value::Str(name.to_owned()), Value::Dict(vec![]));
    let account = Value::Dict(vec![
        (
            b("overview"),
            Value::Dict(vec![(
                b("overviewProfilePresets"),
                stamped(Value::Dict(vec![
                    preset("travel"),
                    preset("<color=0xFFFF6666>Combat</color>"),
                    preset("D-scan"),
                ])),
            )]),
        ),
        (
            b("ui"),
            Value::Dict(vec![
                (b("dir_scanrange"), stamped(Value::Int(2_139_249_551))),
                (
                    b("scan_angleSlider"),
                    stamped(Value::Float(std::f64::consts::TAU)),
                ),
                (
                    b("scanner_presetInUse"),
                    stamped(Value::Str("D-scan".into())),
                ),
                (
                    b("probescanning.resultFilter.filters"),
                    stamped(Value::Dict(vec![
                        (Value::Int(13), filter("dictor", &[541])),
                        (Value::Int(16), filter("ships", &[25, 26, 27])),
                    ])),
                ),
                (
                    b("probescanning.resultFilter.activeFilterSet"),
                    stamped(active),
                ),
            ]),
        ),
    ]);
    fs::write(
        dir.path().join("core_user_1.dat"),
        encode(&account, false).unwrap(),
    )
    .unwrap();
    dir
}

fn account_value(dir: &Path, id: &str) -> Reading {
    on_disk(dir, "core_user_1.dat", FileKind::Account, id)
}

fn filter_names(dir: &Path) -> Vec<String> {
    match account_value(dir, "probe_scan_filters") {
        Reading::Stored(Content::Filters(filters)) => {
            filters.into_iter().map(|filter| filter.name).collect()
        }
        other => panic!("filters read as {other:?}"),
    }
}

#[test]
fn the_scan_range_is_typed_in_au_and_the_angle_in_degrees() {
    let dir = scanner_profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    ui.get_by_label("Scanning");
    ui.get_by_label("Directional scan range");
    ui.get_by_label("AU");
    ui.get_by_label("0 to 14.3");
    ui.get_by_label("degrees");
    ui.get_by_label("5 to 360");
    // What is stored (2,139,249,551 km and two pi) is shown as 14.3 and 360.
    assert!(ui.get_all_by_value("14.3").next().is_some());
    assert!(ui.get_all_by_value("360").next().is_some());

    type_into(&mut ui, "14.3", "2.5");
    type_into(&mut ui, "360", "30");
    ui.get_by_label("2 changes not saved");
    save(&mut ui);
    assert!(flow(&ui).problem.is_none(), "{:?}", flow(&ui).problem);
    assert_eq!(
        account_value(dir.path(), "dscan_range"),
        Reading::Stored(Content::Number(2.5))
    );
    assert_eq!(
        account_value(dir.path(), "dscan_angle"),
        Reading::Stored(Content::Number(30.0))
    );

    // A range past the limit is not taken.
    type_into(&mut ui, "2.5", "20");
    assert!(!flow(&ui).is_dirty());
}

#[test]
fn the_scan_preset_is_picked_from_the_files_own_presets() {
    let dir = scanner_profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    let drop_down = ui.get_by_value("D-scan").rect().center();
    click_at(&mut ui, drop_down);
    // Shown without the game's colour tags, in alphabetical order.
    ui.get_by_label("travel");
    click(&mut ui, "Combat");
    ui.get_by_label("1 change not saved");
    save(&mut ui);
    assert!(flow(&ui).problem.is_none(), "{:?}", flow(&ui).problem);
    // Stored under the preset's full name.
    assert_eq!(
        account_value(dir.path(), "dscan_preset"),
        Reading::Stored(Content::Word("<color=0xFFFF6666>Combat</color>".to_owned()))
    );
    assert_eq!(
        crate::edit_flow::preset_choices(&[
            "travel".to_owned(),
            "<b>Combat</b>".to_owned(),
            "<b></b>".to_owned(),
            "Docked".to_owned(),
        ]),
        [
            ("<b></b>".to_owned(), "<b></b>"),
            ("Combat".to_owned(), "<b>Combat</b>"),
            ("Docked".to_owned(), "Docked"),
            ("travel".to_owned(), "travel"),
        ]
    );
}

fn stored_filters(dir: &Path) -> Vec<clone_bay_core::edit::ScanFilter> {
    match account_value(dir, "probe_scan_filters") {
        Reading::Stored(Content::Filters(filters)) => filters,
        other => panic!("filters read as {other:?}"),
    }
}

fn filters_row(ui: &Ui) -> crate::edit_flow::Row {
    flow(ui)
        .rows
        .iter()
        .find(|row| row.setting.id == "probe_scan_filters")
        .cloned()
        .unwrap()
}

/// Clicks the `nth` widget showing `label`, scrolling it into view first.
fn click_nth(ui: &mut Ui, label: &str, nth: usize) {
    ui.get_all_by_label(label).nth(nth).unwrap().scroll_to_me();
    settle(ui);
    let at = ui.get_all_by_label(label).nth(nth).unwrap().rect().center();
    click_at(ui, at);
}

#[test]
fn scan_filters_are_renamed_and_removed() {
    let dir = scanner_profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    ui.get_by_label("Probe scan result filters (2 filters)");
    ui.get_by_label("1 kind of result");
    ui.get_by_label("3 kinds of result");
    assert!(ui.get_all_by_value("dictor").next().is_some());

    type_into(&mut ui, "ships", "all ships");
    ui.get_by_label("1 change not saved");
    click_nth(&mut ui, "Remove filter", 0);
    ui.get_by_label("Probe scan result filters (1 filter)");
    save(&mut ui);
    assert!(flow(&ui).problem.is_none(), "{:?}", flow(&ui).problem);
    assert_eq!(filter_names(dir.path()), ["all ships"]);

    // A filter with no name can't be saved; the message says which and why.
    let blank = clone_bay_core::edit::ScanFilter {
        id: 16,
        name: "  ".to_owned(),
        groups: vec![25],
    };
    set_draft(&mut ui, "probe_scan_filters", Content::Filters(vec![blank]));
    save(&mut ui);
    assert_eq!(
        flow(&ui).problem.as_deref(),
        Some("Filter '(unnamed)' needs a name. Nothing was changed")
    );
    assert_eq!(filter_names(dir.path()), ["all ships"]);
}

#[test]
fn what_a_scan_filter_shows_is_chosen_with_tick_boxes() {
    let dir = scanner_profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    // Nothing is opened until asked for.
    assert!(!shows(&ui, "Battleship"));
    assert_eq!(ui.get_all_by_label("Choose what it shows").count(), 2);

    // Open "ships" (frigates, cruisers, battleships).
    click_nth(&mut ui, "Choose what it shows", 1);
    assert_eq!(filters_row(&ui).open_filter, Some(16));
    ui.get_by_label("Done choosing");
    for heading in ["Ship", "Drone", "Deployable", "Structure"] {
        ui.get_by_label(heading);
    }
    assert!(!flow(&ui).is_dirty());

    // Frigates out, interdictors in.
    click(&mut ui, "Frigate");
    click(&mut ui, "Interdictor");
    ui.get_by_label("1 change not saved");
    // Opening another filter closes this one.
    click_nth(&mut ui, "Choose what it shows", 0);
    assert_eq!(filters_row(&ui).open_filter, Some(13));
    assert_eq!(ui.get_all_by_label("Done choosing").count(), 1);
    click(&mut ui, "Done choosing");
    assert_eq!(filters_row(&ui).open_filter, None);
    assert!(!shows(&ui, "Battleship"));

    save(&mut ui);
    assert!(flow(&ui).problem.is_none(), "{:?}", flow(&ui).problem);
    let saved = stored_filters(dir.path());
    assert_eq!(saved[0].groups, [541]);
    assert_eq!(saved[1].groups, [26, 27, 541]);
}

#[test]
fn a_scan_filter_is_added_named_and_given_something_to_show() {
    let dir = scanner_profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    click(&mut ui, "Add filter");
    ui.get_by_label("Probe scan result filters (3 filters)");
    ui.get_by_label("0 kinds of result");
    // It opens straight away, above every ID the file holds.
    assert_eq!(filters_row(&ui).open_filter, Some(17));
    ui.get_by_label("Done choosing");

    // With nothing ticked it can't be saved, and the message says why.
    save(&mut ui);
    assert_eq!(
        flow(&ui).problem.as_deref(),
        Some("Filter 'New filter' needs at least one kind of result ticked. Nothing was changed")
    );
    assert_eq!(stored_filters(dir.path()).len(), 2);

    type_into(&mut ui, "New filter", "haulers");
    click(&mut ui, "Hauler");
    click(&mut ui, "Mobile Tractor Unit");
    save(&mut ui);
    assert!(flow(&ui).problem.is_none(), "{:?}", flow(&ui).problem);
    let saved = stored_filters(dir.path());
    assert_eq!(saved.len(), 3);
    assert_eq!(saved[2].id, 17);
    assert_eq!(saved[2].name, "haulers");
    assert_eq!(saved[2].groups, [28, 1250]);
    // The two it had are as they were.
    assert_eq!(saved[0].name, "dictor");
    assert_eq!(saved[1].groups, [25, 26, 27]);
}

#[test]
fn a_removed_filters_id_is_not_handed_out_again() {
    let dir = scanner_profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    click_nth(&mut ui, "Remove filter", 1);
    click(&mut ui, "Add filter");
    click(&mut ui, "Add filter");
    let row = filters_row(&ui);
    let Some(Content::Filters(filters)) = &row.draft else {
        panic!("no filters");
    };
    let ids: Vec<i64> = filters.iter().map(|filter| filter.id).collect();
    assert_eq!(ids, [13, 17, 18]);
    assert_eq!(row.open_filter, Some(18));
    assert!(row.touched);
}

#[test]
fn an_account_with_no_filters_stored_can_be_given_its_first() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    ui.get_by_label("Probe scan result filters (0 filters)");
    assert!(!shows(&ui, "Remove filter"));
    assert!(flow(&ui).changes().is_empty());

    click(&mut ui, "Add filter");
    click(&mut ui, "Frigate");
    save(&mut ui);
    assert!(flow(&ui).problem.is_none(), "{:?}", flow(&ui).problem);
    let saved = stored_filters(dir.path());
    assert_eq!(saved.len(), 1);
    assert_eq!((saved[0].id, saved[0].name.as_str()), (1, "New filter"));
    assert_eq!(saved[0].groups, [25]);
}

#[test]
fn kinds_of_result_are_offered_by_category_with_the_filters_own_kept() {
    use crate::edit_flow::{group_choices, set_group, OTHER_GROUPS};
    let mut filter = clone_bay_core::edit::ScanFilter {
        id: 1,
        name: "mixed".to_owned(),
        // A frigate, a group the game hides, and one the app doesn't know.
        groups: vec![25, 381, 999_999],
    };
    let choices = group_choices(&filter);
    let categories: Vec<&str> = choices.iter().map(|(category, _)| *category).collect();
    assert_eq!(categories[0], "Ship");
    assert_eq!(categories.last(), Some(&OTHER_GROUPS));
    let ships = &choices[0].1;
    assert!(ships.contains(&(25, "Frigate".to_owned(), true)));
    assert!(ships.contains(&(26, "Cruiser".to_owned(), false)));
    // The hidden group is listed only because the filter holds it.
    assert!(ships.iter().any(|(id, _, ticked)| *id == 381 && *ticked));
    assert_eq!(
        choices.last().unwrap().1,
        [(999_999, "Group 999999".to_owned(), true)]
    );

    set_group(&mut filter, 381, false);
    set_group(&mut filter, 999_999, false);
    set_group(&mut filter, 26, true);
    set_group(&mut filter, 26, true);
    set_group(&mut filter, 7, false);
    assert_eq!(filter.groups, [25, 26]);
    let choices = group_choices(&filter);
    assert!(!choices
        .iter()
        .any(|(category, _)| *category == OTHER_GROUPS));
    assert!(!choices[0].1.iter().any(|(id, _, _)| *id == 381));
}

#[test]
fn esc_goes_back_to_the_file_list() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "2");
    press_esc(&mut ui);
    assert!(ui.state().state.edit().is_none());
    ui.get_by_label("Rescan");
}

#[test]
fn the_mouse_back_button_goes_back_to_the_file_list() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "2");
    press_mouse_back(&mut ui);
    assert!(ui.state().state.edit().is_none());
}

#[test]
fn leaving_with_changes_not_saved_asks_first() {
    let dir = profile();
    let before = fs::read(dir.path().join("core_user_1.dat")).unwrap();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    click(&mut ui, "Camera shake");
    assert!(flow(&ui).is_dirty());

    // Esc asks; Esc again means stay, keeping the change.
    press_esc(&mut ui);
    ui.get_by_label("Leave without saving?");
    assert!(ui.state().state.edit().is_some());
    press_esc(&mut ui);
    assert!(!shows(&ui, "Leave without saving?"));
    assert!(flow(&ui).is_dirty());

    // The button asks too; "Keep editing" stays.
    click(&mut ui, "Back to the file list");
    click(&mut ui, "Keep editing");
    assert!(!shows(&ui, "Leave without saving?"));
    assert!(flow(&ui).is_dirty());

    // The mouse's back button asks; leaving loses the change.
    press_mouse_back(&mut ui);
    click(&mut ui, "Leave without saving");
    assert!(ui.state().state.edit().is_none());
    assert!(ui.state().state.leave_prompt.is_none());
    assert_eq!(
        fs::read(dir.path().join("core_user_1.dat")).unwrap(),
        before
    );
}

#[test]
fn after_discarding_leaving_does_not_ask() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    click(&mut ui, "Camera shake");
    click(&mut ui, "Discard changes");
    press_esc(&mut ui);
    assert!(ui.state().state.edit().is_none());
}

#[test]
fn esc_while_typing_only_leaves_the_box() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "2");
    let at = ui
        .get_all_by_role(Role::TextInput)
        .next()
        .unwrap()
        .rect()
        .center();
    click_at(&mut ui, at);
    assert!(ui.ctx.memory(|memory| memory.focused().is_some()));
    ui.key_press(egui::Key::Escape);
    settle(&mut ui);
    assert!(
        ui.state().state.edit().is_some(),
        "still on the edit screen"
    );
    assert!(ui.ctx.memory(|memory| memory.focused().is_none()));
    // A second Esc, with nothing being typed, goes back.
    ui.key_press(egui::Key::Escape);
    settle(&mut ui);
    assert!(ui.state().state.edit().is_none());
}

#[test]
fn esc_with_a_list_open_only_closes_the_list() {
    let dir = profile();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    let at = ui
        .get_all_by_role(Role::ComboBox)
        .next()
        .unwrap()
        .rect()
        .center();
    click_at(&mut ui, at);
    assert!(egui::Popup::is_any_open(&ui.ctx));
    ui.key_press(egui::Key::Escape);
    settle(&mut ui);
    assert!(
        ui.state().state.edit().is_some(),
        "still on the edit screen"
    );
    assert!(!egui::Popup::is_any_open(&ui.ctx));
}

#[test]
fn esc_while_saving_neither_leaves_nor_asks() {
    let dir = profile();
    let gate = EveGate::default();
    let services = Services {
        eve_status: gate.eve_status(),
        ..services(EveStatus::NotRunning)
    };
    let mut ui = harness_with(dir.path(), services);
    click(&mut ui, "1");
    click(&mut ui, "Edit settings");
    click(&mut ui, "Camera shake");
    click(&mut ui, "Save changes");
    assert!(flow(&ui).busy());
    press_esc(&mut ui);
    press_mouse_back(&mut ui);
    assert!(ui.state().state.edit().is_some());
    assert!(ui.state().state.leave_prompt.is_none());
    gate.open();
    wait_until(&mut ui, |app| {
        app.state.edit().is_some_and(|flow| !flow.busy())
    });
    // Saved, so nothing is left to lose: Esc now leaves without asking.
    press_esc(&mut ui);
    assert!(ui.state().state.edit().is_none());
}

#[test]
fn closing_the_window_with_changes_not_saved_asks_first() {
    let dir = profile();
    let before = fs::read(dir.path().join("core_user_1.dat")).unwrap();
    let mut ui = edit_screen(dir.path(), EveStatus::NotRunning, "1");
    // Nothing changed: the window closes.
    assert!(!press_window_close(&mut ui));

    click(&mut ui, "Camera shake");
    assert!(press_window_close(&mut ui), "kept open to ask");
    ui.get_by_label("Close without saving?");
    click(&mut ui, "Keep editing");
    assert!(ui.state().state.leave_prompt.is_none());
    assert!(!closing(&ui));
    assert!(flow(&ui).is_dirty());

    assert!(press_window_close(&mut ui));
    // Esc means stay, as with any question.
    press_esc(&mut ui);
    assert!(!shows(&ui, "Close without saving?"));
    assert!(ui.state().state.edit().is_some());

    assert!(press_window_close(&mut ui));
    click(&mut ui, "Close without saving");
    assert!(closing(&ui));
    // The close that follows isn't stopped.
    assert!(!press_window_close(&mut ui));
    assert_eq!(
        fs::read(dir.path().join("core_user_1.dat")).unwrap(),
        before
    );
}

#[test]
fn the_window_stays_open_while_a_save_runs() {
    let dir = profile();
    let gate = EveGate::default();
    let services = Services {
        eve_status: gate.eve_status(),
        ..services(EveStatus::NotRunning)
    };
    let mut ui = harness_with(dir.path(), services);
    click(&mut ui, "1");
    click(&mut ui, "Edit settings");
    click(&mut ui, "Camera shake");
    click(&mut ui, "Save changes");
    assert!(press_window_close(&mut ui));
    ui.get_by_label("Clone Bay is still saving. Wait for it to finish, then close it.");
    // Not the unsaved-changes question: that choice can't be acted on yet.
    assert!(!shows(&ui, "Close without saving?"));
    click(&mut ui, "OK");
    assert!(ui.state().state.busy_notice.is_none());
    gate.open();
    wait_until(&mut ui, |app| {
        app.state.edit().is_some_and(|flow| !flow.busy())
    });
    assert!(!press_window_close(&mut ui));
}
