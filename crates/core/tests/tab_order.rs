//! Tab order: saving a new order through the safe write path.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use clone_bay_core::edit::EditError;
use clone_bay_core::platform::EveStatus;
use clone_bay_core::safe_write::{BackupSession, Fingerprint};
use clone_bay_core::tab_order::{open_file, read_file, save, TabLayout, TabOrderError};
use clone_bay_marshal::{decode, encode, Long, Value};

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
    dict(&[
        (
            b("overview"),
            dict(&[
                (b("before"), stamped(Value::None)),
                (
                    b("tabsByWindowInstanceID"),
                    stamped(Value::List(vec![ints(&[0, 1]), ints(&[2])])),
                ),
            ]),
        ),
        (
            b("tabgroups"),
            dict(&[
                (b("overviewTabs"), stamped(Value::Int(1))),
                (b("overviewTabs_names"), stamped(s("Second"))),
            ]),
        ),
        (b("ui"), dict(&[(b("slotOrder"), stamped(dict(&[])))])),
    ])
}

fn character() -> Value {
    let channel = |kind: &str| Value::Tuple(vec![b(kind), s(&format!("{kind}_1")), s(kind)]);
    dict(&[
        (
            b("autorepeat"),
            dict(&[(Value::Int(5), stamped(Value::Int(1000)))]),
        ),
        (
            b("ui"),
            dict(&[(
                b("chatchannels"),
                stamped(Value::List(vec![channel("corp"), channel("local")])),
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

fn file_with(name: &str, root: &Value) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, encode(root, false).unwrap()).unwrap();
    (dir, path)
}

fn now() -> Long {
    Long::from_i64(99)
}

fn save_to(path: &Path, layout: &TabLayout, eve: EveStatus) -> Result<PathBuf, TabOrderError> {
    save(
        path,
        Fingerprint::of(&fs::read(path).unwrap()),
        layout,
        eve,
        false,
        &BackupSession::new("T"),
        &now(),
    )
}

/// The contents of a file with the given `(section, key)` settings taken out.
fn without(path: &Path, keys: &[(&str, &str)]) -> Value {
    let Value::Dict(sections) = decode(&fs::read(path).unwrap()).unwrap().value else {
        panic!("not a dict");
    };
    Value::Dict(
        sections
            .into_iter()
            .map(|(name, entries)| {
                let Value::Dict(entries) = entries else {
                    panic!("not a dict");
                };
                let entries = entries
                    .into_iter()
                    .filter(|(key, _)| {
                        !keys
                            .iter()
                            .any(|(section, wanted)| name == b(section) && *key == b(wanted))
                    })
                    .collect();
                (name, Value::Dict(entries))
            })
            .collect(),
    )
}

fn labels(layout: &TabLayout) -> Vec<Vec<String>> {
    layout
        .groups
        .iter()
        .map(|group| group.tabs.iter().map(|tab| tab.label.clone()).collect())
        .collect()
}

#[test]
fn an_overview_order_is_saved_with_a_backup_and_touches_nothing_else() {
    let (_dir, path) = file_with("core_user_1.dat", &account());
    let original = fs::read(&path).unwrap();
    let mut layout = read_file(&path).unwrap();
    assert_eq!(labels(&layout), [vec!["Tab 0", "Tab 1"], vec!["Tab 2"]]);

    layout.move_down(0, 0);
    layout.move_to_group(0, 1, 1);
    layout.open_on_first_tab();
    let backup = save_to(&path, &layout, EveStatus::NotRunning).unwrap();

    assert_eq!(fs::read(backup.join("core_user_1.dat")).unwrap(), original);
    let saved = read_file(&path).unwrap();
    assert_eq!(saved, layout);
    assert_eq!(labels(&saved), [vec!["Tab 1"], vec!["Tab 2", "Tab 0"]]);
    assert_eq!(saved.opens_on, Some(0));

    let edited = [
        ("overview", "tabsByWindowInstanceID"),
        ("tabgroups", "overviewTabs"),
    ];
    let (_other, untouched) = file_with("core_user_2.dat", &account());
    assert_eq!(without(&path, &edited), without(&untouched, &edited));
    // What was written carries the time of the save.
    let Value::Dict(sections) = decode(&fs::read(&path).unwrap()).unwrap().value else {
        panic!("not a dict");
    };
    let Value::Dict(tab_groups) = &sections[1].1 else {
        panic!("not a dict");
    };
    assert_eq!(
        tab_groups[0],
        (
            b("overviewTabs"),
            Value::Tuple(vec![Value::Long(now()), Value::Int(0)])
        )
    );
}

#[test]
fn a_chat_order_is_saved_and_touches_nothing_else() {
    let (_dir, path) = file_with("core_char_1.dat", &character());
    let original = fs::read(&path).unwrap();
    let mut layout = read_file(&path).unwrap();
    assert_eq!(labels(&layout), [["local", "corp"]]);

    layout.move_up(0, 1);
    let backup = save_to(&path, &layout, EveStatus::NotRunning).unwrap();
    assert_eq!(fs::read(backup.join("core_char_1.dat")).unwrap(), original);
    assert_eq!(labels(&read_file(&path).unwrap()), [["corp", "local"]]);

    let edited = [("windows", "preferredIdxInStack3")];
    let (_other, untouched) = file_with("core_char_2.dat", &character());
    assert_eq!(without(&path, &edited), without(&untouched, &edited));
}

#[test]
fn an_unchanged_order_is_not_saved() {
    for (name, root) in [
        ("core_user_1.dat", account()),
        ("core_char_1.dat", character()),
    ] {
        let (_dir, path) = file_with(name, &root);
        let original = fs::read(&path).unwrap();
        let layout = read_file(&path).unwrap();
        assert_eq!(
            save_to(&path, &layout, EveStatus::NotRunning),
            Err(TabOrderError::Edit(EditError::NothingToSave))
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}

#[test]
fn nothing_is_saved_while_eve_is_or_might_be_running() {
    let (_dir, path) = file_with("core_user_1.dat", &account());
    let original = fs::read(&path).unwrap();
    let mut layout = read_file(&path).unwrap();
    layout.move_down(0, 0);
    assert_eq!(
        save_to(&path, &layout, EveStatus::Running),
        Err(TabOrderError::Edit(EditError::EveRunning))
    );
    assert_eq!(
        save_to(&path, &layout, EveStatus::Unknown),
        Err(TabOrderError::Edit(EditError::EveUnknown))
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    // Once the user vouches that EVE is closed, it goes ahead.
    save(
        &path,
        Fingerprint::of(&fs::read(&path).unwrap()),
        &layout,
        EveStatus::Unknown,
        true,
        &BackupSession::new("T"),
        &now(),
    )
    .unwrap();
    assert_eq!(read_file(&path).unwrap(), layout);
}

#[test]
fn an_order_is_not_saved_onto_a_file_whose_tabs_have_changed() {
    let (_dir, path) = file_with("core_user_1.dat", &account());
    let mut layout = read_file(&path).unwrap();
    layout.move_down(0, 0);

    // The game gave the first window another tab in the meantime.
    let Value::Dict(mut sections) = account() else {
        panic!("not a dict");
    };
    sections[0].1 = dict(&[(
        b("tabsByWindowInstanceID"),
        stamped(Value::List(vec![ints(&[0, 1, 7]), ints(&[2])])),
    )]);
    let changed = encode(&Value::Dict(sections), false).unwrap();
    fs::write(&path, &changed).unwrap();

    assert_eq!(
        save_to(&path, &layout, EveStatus::NotRunning),
        Err(TabOrderError::TabsChanged)
    );
    assert_eq!(fs::read(&path).unwrap(), changed);
}

#[test]
fn an_order_is_not_saved_over_a_file_that_changed_after_it_was_opened() {
    let (dir, path) = file_with("core_user_1.dat", &account());
    let (mut layout, opened_as) = open_file(&path).unwrap();
    assert_eq!(layout, read_file(&path).unwrap());
    // The game saved the same settings again, with a checksum this time.
    let game_bytes = encode(&account(), true).unwrap();
    fs::write(&path, &game_bytes).unwrap();

    layout.move_down(0, 0);
    let result = save(
        &path,
        opened_as,
        &layout,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &now(),
    );
    assert_eq!(
        result,
        Err(TabOrderError::Edit(EditError::ChangedSinceOpened))
    );
    assert_eq!(fs::read(&path).unwrap(), game_bytes);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn a_file_that_cannot_be_read_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("core_user_1.dat");
    assert!(matches!(
        read_file(&missing),
        Err(TabOrderError::Edit(EditError::Unreadable { .. }))
    ));
    let other = dir.path().join("notes.txt");
    fs::write(&other, b"hello").unwrap();
    assert!(matches!(
        read_file(&other),
        Err(TabOrderError::Edit(EditError::NotASettingsFile(_)))
    ));
}
