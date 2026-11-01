//! Tests that drive the tab order screen in the real UI, off-screen,
//! against files in a temp folder.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use clone_bay_core::platform::EveStatus;
use clone_bay_core::tab_order::{read_file, TabLayout};
use clone_bay_marshal::{encode, Long, Value};
use egui_kittest::kittest::Queryable;

use crate::app::test_support::{click, harness_with, settle, shows, wait_until, Ui};
use crate::services::Services;
use crate::tab_flow::TabFlow;

fn b(text: &str) -> Value {
    Value::Bytes(text.as_bytes().to_vec())
}

fn s(text: &str) -> Value {
    Value::Str(text.to_owned())
}

fn dict(pairs: &[(Value, Value)]) -> Value {
    Value::Dict(pairs.to_vec())
}

fn stamped(value: Value) -> Value {
    Value::Tuple(vec![Value::Long(Long::from_i64(1)), value])
}

fn ints(ids: &[i64]) -> Value {
    Value::List(ids.iter().map(|id| Value::Int(*id)).collect())
}

fn account() -> Value {
    let tab = |id: i64, name: &str| (Value::Int(id), dict(&[(b("name"), s(name))]));
    dict(&[
        (
            b("overview"),
            dict(&[
                (
                    b("tabsByWindowInstanceID"),
                    stamped(Value::List(vec![ints(&[0, 1, 2]), ints(&[3])])),
                ),
                (
                    b("tabsettings_new"),
                    stamped(dict(&[
                        tab(0, "Combat"),
                        tab(1, "<b>Travel</b>"),
                        tab(2, "Loot"),
                        tab(3, "Gates"),
                    ])),
                ),
            ]),
        ),
        (
            b("tabgroups"),
            dict(&[(b("overviewTabs"), stamped(Value::Int(1)))]),
        ),
        (b("ui"), dict(&[(b("untouched"), stamped(b("leave me")))])),
    ])
}

fn character() -> Value {
    let channel = |kind: &str, name: &str| Value::Tuple(vec![b(kind), s("id"), s(name)]);
    dict(&[
        (
            b("ui"),
            dict(&[(
                b("chatchannels"),
                stamped(Value::List(vec![
                    channel("corp", "Corp"),
                    channel("local", "Local"),
                ])),
            )]),
        ),
        (
            b("windows"),
            dict(&[
                (
                    b("stacksWindows"),
                    stamped(dict(&[
                        (s("chatchannel_corp"), b("ChatWindowStack")),
                        (b("chatchannel_local"), b("ChatWindowStack")),
                    ])),
                ),
                (
                    b("preferredIdxInStack3"),
                    stamped(dict(&[(
                        b("ChatWindowStack"),
                        dict(&[
                            (b("chatchannel_corp"), Value::Int(1)),
                            (b("chatchannel_local"), Value::Int(0)),
                        ]),
                    )])),
                ),
            ]),
        ),
    ])
}

fn profile() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let put = |name: &str, value: &Value| {
        fs::write(dir.path().join(name), encode(value, false).unwrap()).unwrap();
    };
    put("core_user_1.dat", &account());
    put("core_char_2.dat", &character());
    // An account with nothing about the overview stored.
    put("core_user_3.dat", &dict(&[(b("ui"), dict(&[]))]));
    dir
}

fn services(eve: EveStatus) -> Services {
    Services {
        eve_status: Arc::new(move || eve),
        ..Services::for_tests()
    }
}

/// The app with the tab order screen open on the file whose ID is `id`.
fn tab_screen(dir: &Path, eve: EveStatus, id: &str, button: &str) -> Ui {
    let mut ui = harness_with(dir, services(eve));
    click(&mut ui, id);
    click(&mut ui, button);
    assert!(ui.state().state.tabs().is_some());
    ui
}

fn flow(ui: &Ui) -> &TabFlow {
    ui.state().state.tabs().unwrap()
}

fn labels(layout: &TabLayout) -> Vec<Vec<String>> {
    layout
        .groups
        .iter()
        .map(|group| group.tabs.iter().map(|tab| tab.label.clone()).collect())
        .collect()
}

fn shown(ui: &Ui) -> Vec<Vec<String>> {
    labels(flow(ui).layout.as_ref().unwrap())
}

/// A real pointer click on the `nth` widget showing `label`.
fn click_nth(ui: &mut Ui, label: &str, nth: usize) {
    let at = ui.get_all_by_label(label).nth(nth).unwrap().rect().center();
    ui.hover_at(at);
    ui.step();
    ui.drag_at(at);
    ui.step();
    ui.drop_at(at);
    settle(ui);
}

fn save(ui: &mut Ui) {
    click(ui, "Save changes");
    wait_until(ui, |app| app.state.tabs().is_some_and(|flow| !flow.busy()));
}

#[test]
fn overview_tabs_are_reordered_and_saved() {
    let dir = profile();
    let path = dir.path().join("core_user_1.dat");
    let original = fs::read(&path).unwrap();
    let mut ui = tab_screen(
        dir.path(),
        EveStatus::NotRunning,
        "1",
        "Order overview tabs",
    );

    ui.get_by_label("Overview tabs for account 1");
    ui.get_by_label("Overview window 1");
    ui.get_by_label("Overview window 2");
    ui.get_by_label("1. Combat");
    ui.get_by_label("2. Travel");
    ui.get_by_label("3. Loot");
    ui.get_by_label("1. Gates");
    assert!(!flow(&ui).is_dirty());
    assert!(!flow(&ui).can_save());
    assert!(!shows(&ui, "New order not saved"));

    // The second "Down" belongs to Travel; the first "Up" is Combat's and
    // can't be pressed.
    click_nth(&mut ui, "Up", 0);
    assert_eq!(shown(&ui)[0], ["Combat", "Travel", "Loot"]);
    click_nth(&mut ui, "Down", 1);
    assert_eq!(shown(&ui)[0], ["Combat", "Loot", "Travel"]);
    ui.get_by_label("3. Travel");
    ui.get_by_label("New order not saved");
    click_nth(&mut ui, "Up", 1);
    assert_eq!(shown(&ui)[0], ["Loot", "Combat", "Travel"]);

    save(&mut ui);
    assert!(flow(&ui).problem.is_none(), "{:?}", flow(&ui).problem);
    let backup = flow(&ui).saving.saved_to.clone().unwrap();
    assert_eq!(fs::read(backup.join("core_user_1.dat")).unwrap(), original);
    assert!(!flow(&ui).is_dirty());
    assert_eq!(
        labels(&read_file(&path).unwrap()),
        [vec!["Loot", "Combat", "Travel"], vec!["Gates"]]
    );

    click(&mut ui, "Back to the file list");
    assert!(ui.state().state.tabs().is_none());
}

#[test]
fn an_order_is_not_saved_over_a_file_the_game_changed_after_the_screen_opened() {
    let dir = profile();
    let path = dir.path().join("core_user_1.dat");
    let mut ui = tab_screen(
        dir.path(),
        EveStatus::NotRunning,
        "1",
        "Order overview tabs",
    );
    click_nth(&mut ui, "Down", 0);
    // The game saved the same settings again, with a checksum this time.
    let game_bytes = encode(&account(), true).unwrap();
    fs::write(&path, &game_bytes).unwrap();

    save(&mut ui);
    ui.get_by_label(crate::edit_ui_tests::CHANGED_SINCE_OPENED);
    assert_eq!(fs::read(&path).unwrap(), game_bytes);
    assert!(flow(&ui).saving.saved_to.is_none());
    ui.get_by_label("New order not saved");
}

#[test]
fn an_overview_tab_can_be_moved_to_another_window() {
    let dir = profile();
    let mut ui = tab_screen(
        dir.path(),
        EveStatus::NotRunning,
        "1",
        "Order overview tabs",
    );
    // Each tab has a "Move to" list; the first is Combat's.
    assert_eq!(ui.get_all_by_label("Move to").count(), 4);
    click_nth(&mut ui, "Move to", 0);
    // The one other window is offered; the tab's own is not.
    click_nth(&mut ui, "Overview window 2", 1);
    assert_eq!(
        shown(&ui),
        [vec!["Travel", "Loot"], vec!["Gates", "Combat"]]
    );
    assert!(flow(&ui).is_dirty());

    click(&mut ui, "Discard changes");
    assert_eq!(
        shown(&ui),
        [vec!["Combat", "Travel", "Loot"], vec!["Gates"]]
    );
    assert!(!flow(&ui).is_dirty());
}

#[test]
fn the_tab_the_game_opens_on_can_be_put_back_to_the_first() {
    let dir = profile();
    let path = dir.path().join("core_user_1.dat");
    let mut ui = tab_screen(
        dir.path(),
        EveStatus::NotRunning,
        "1",
        "Order overview tabs",
    );
    ui.get_by_label("Overview windows open on tab 2 of each window, not the first.");
    click(&mut ui, "Open on the first tab");
    ui.get_by_label("Overview windows open on their first tab.");
    assert!(!shows(&ui, "Open on the first tab"));
    assert!(flow(&ui).is_dirty());

    save(&mut ui);
    let saved = read_file(&path).unwrap();
    assert_eq!(saved.opens_on, Some(0));
    assert_eq!(
        labels(&saved),
        [vec!["Combat", "Travel", "Loot"], vec!["Gates"]]
    );
}

#[test]
fn chat_tabs_are_reordered_and_saved() {
    let dir = profile();
    let path = dir.path().join("core_char_2.dat");
    let mut ui = tab_screen(dir.path(), EveStatus::NotRunning, "2", "Order chat tabs");
    ui.get_by_label("Chat tabs for character 2");
    ui.get_by_label("Main chat window");
    ui.get_by_label("1. Local");
    ui.get_by_label("2. Corp");
    // Chat channels stay in their own window.
    assert!(!shows(&ui, "Move to"));
    assert!(!shows(&ui, "Open on the first tab"));

    click_nth(&mut ui, "Down", 0);
    assert_eq!(shown(&ui), [["Corp", "Local"]]);
    save(&mut ui);
    assert!(flow(&ui).problem.is_none(), "{:?}", flow(&ui).problem);
    assert_eq!(labels(&read_file(&path).unwrap()), [["Corp", "Local"]]);
}

#[test]
fn a_file_with_no_tabs_stored_says_so() {
    let dir = profile();
    let ui = tab_screen(
        dir.path(),
        EveStatus::NotRunning,
        "3",
        "Order overview tabs",
    );
    ui.get_by_label("This file doesn't store any overview windows yet.");
    assert!(!shows(&ui, "Up"));
    assert!(!flow(&ui).can_save());
}

#[test]
fn the_file_list_is_read_again_only_after_a_save() {
    let dir = profile();
    let mut ui = tab_screen(
        dir.path(),
        EveStatus::NotRunning,
        "1",
        "Order overview tabs",
    );
    let listed = ui.state().state.rows.len();
    fs::write(dir.path().join("core_user_9.dat"), b"new").unwrap();

    click(&mut ui, "Back to the file list");
    assert_eq!(ui.state().state.rows.len(), listed);

    click(&mut ui, "Order overview tabs");
    click_nth(&mut ui, "Down", 0);
    save(&mut ui);
    click(&mut ui, "Back to the file list");
    assert_eq!(ui.state().state.rows.len(), listed + 1);
}

#[test]
fn nothing_is_saved_while_eve_is_running() {
    let dir = profile();
    let path = dir.path().join("core_user_1.dat");
    let original = fs::read(&path).unwrap();
    let mut ui = tab_screen(dir.path(), EveStatus::Running, "1", "Order overview tabs");
    click_nth(&mut ui, "Down", 0);
    save(&mut ui);
    let problem = flow(&ui).problem.clone().unwrap();
    assert!(problem.starts_with("EVE is running"), "{problem}");
    // The check worked, so there is nothing for the user to vouch for.
    assert!(!flow(&ui).saving.needs_eve_confirmation);
    assert!(flow(&ui).saving.saved_to.is_none());
    assert_eq!(fs::read(&path).unwrap(), original);
    // The new order is still there to save once EVE is closed.
    assert!(flow(&ui).is_dirty());
}

#[test]
fn when_eve_cannot_be_checked_the_user_has_to_vouch_for_it() {
    let dir = profile();
    let path = dir.path().join("core_user_1.dat");
    let mut ui = tab_screen(dir.path(), EveStatus::Unknown, "1", "Order overview tabs");
    click_nth(&mut ui, "Down", 0);
    save(&mut ui);
    assert!(flow(&ui).saving.needs_eve_confirmation);
    assert!(!flow(&ui).can_save());

    click(&mut ui, "I have closed every EVE client");
    assert!(flow(&ui).can_save());
    save(&mut ui);
    assert!(flow(&ui).problem.is_none(), "{:?}", flow(&ui).problem);
    assert!(!flow(&ui).saving.needs_eve_confirmation);
    assert_eq!(
        labels(&read_file(&path).unwrap())[0],
        ["Travel", "Combat", "Loot"]
    );
}

#[test]
fn a_file_that_cannot_be_read_shows_why_and_offers_nothing_to_change() {
    let dir = profile();
    let mut ui = harness_with(dir.path(), services(EveStatus::NotRunning));
    click(&mut ui, "1");
    // The file is damaged after the list was made.
    fs::write(dir.path().join("core_user_1.dat"), b"not a settings file").unwrap();
    click(&mut ui, "Order overview tabs");
    let flow = flow(&ui);
    assert!(flow.layout.is_none());
    assert!(flow.problem.is_some());
    assert!(!flow.is_dirty());
    assert!(!shows(&ui, "Up"));
}

#[test]
fn moves_that_cannot_be_made_change_nothing() {
    let dir = profile();
    let mut ui = tab_screen(
        dir.path(),
        EveStatus::NotRunning,
        "1",
        "Order overview tabs",
    );
    let tabs = ui.state_mut().state.tabs_mut().unwrap();
    tabs.move_up(0, 0);
    tabs.move_down(0, 2);
    tabs.move_to_group(1, 0, 0);
    assert!(!tabs.is_dirty());
    tabs.move_down(0, 0);
    tabs.move_up(0, 1);
    assert!(!tabs.is_dirty());
    tabs.move_to_group(0, 0, 1);
    assert!(tabs.is_dirty());
    tabs.discard();
    assert!(!tabs.is_dirty());
    tabs.open_on_first_tab();
    assert!(tabs.is_dirty());
}
