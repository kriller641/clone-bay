//! Editing individual settings in one file.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fs;
use std::time::Duration;

use clone_bay_core::discovery::FileKind;
use clone_bay_core::edit::{
    apply, open, read, read_file, save, settings_for, timestamp_from_unix, timestamp_now, verify,
    Change, Content, EditError, Kind, Reading, Setting,
};
use clone_bay_core::platform::EveStatus;
use clone_bay_core::safe_write::{BackupSession, Fingerprint};
use clone_bay_marshal::{decode, encode, Long, Value};

fn b(text: &str) -> Value {
    Value::Bytes(text.as_bytes().to_vec())
}

fn dict(pairs: &[(Value, Value)]) -> Value {
    Value::Dict(pairs.to_vec())
}

fn stamped(stamp: i64, value: Value) -> Value {
    Value::Tuple(vec![Value::Long(Long::from_i64(stamp)), value])
}

/// What the edit screen would have been given on opening the file, which
/// is what the file holds now.
fn as_opened(path: &std::path::Path) -> Fingerprint {
    Fingerprint::of(&fs::read(path).unwrap_or_default())
}

fn now() -> Long {
    Long::from_i64(999)
}

fn setting(kind: FileKind, id: &str) -> &'static Setting {
    settings_for(kind)
        .iter()
        .find(|setting| setting.id == id)
        .unwrap()
}

fn account() -> Value {
    dict(&[
        (
            b("overview"),
            dict(&[(b("profile"), stamped(1, Value::Int(1)))]),
        ),
        (
            b("ui"),
            dict(&[
                (b("before"), stamped(1, Value::None)),
                (b("defaultwarpto"), stamped(1, Value::Int(0))),
                (b("autoTargetBack"), stamped(1, Value::Int(2))),
                (b("timestampchat"), stamped(1, Value::Bool(true))),
                (b("tooltipDelayGeneric"), stamped(1, Value::Float(171.5))),
                (b("windowTransparency"), stamped(1, Value::Int(0))),
                (b("cameraShakeEnabled"), stamped(1, Value::Int(1))),
                (Value::Int(7), stamped(1, Value::None)),
                (b("after"), stamped(1, Value::None)),
            ]),
        ),
        (
            b("audio"),
            dict(&[
                (b("custom_turrets"), stamped(1, Value::Float(0.25))),
                (
                    b("soundLevel_custom_turrets"),
                    stamped(1, Value::Float(0.25)),
                ),
                (b("custom_uiclick"), stamped(1, Value::Float(0.5))),
                (
                    b("soundLevel_custom_uiclick"),
                    stamped(1, Value::Float(0.75)),
                ),
            ]),
        ),
    ])
}

fn character() -> Value {
    let distances = dict(&[
        (Value::Int(0), Value::Int(5_000)),
        (Value::Int(11_393), Value::Int(599)),
    ]);
    dict(&[(
        b("ui"),
        dict(&[
            (b("droneAggression"), stamped(1, Value::Bool(true))),
            (b("defaultTypeOrbitDist"), stamped(1, distances)),
        ]),
    )])
}

fn reading(root: &Value, kind: FileKind, id: &str) -> Reading {
    read(root, setting(kind, id))
}

fn change(id: &'static str, content: Content) -> Change {
    Change {
        setting: id,
        content,
    }
}

fn edited(root: &Value, kind: FileKind, changes: &[Change]) -> Value {
    let result = apply(root, kind, changes, &now()).unwrap();
    let data = encode(&result, false).unwrap();
    verify(&data, root, kind, changes, &now()).unwrap();
    result
}

#[test]
fn the_setting_lists_are_well_formed() {
    for kind in [FileKind::Account, FileKind::Character] {
        let settings = settings_for(kind);
        let ids: BTreeSet<&str> = settings.iter().map(|setting| setting.id).collect();
        assert_eq!(ids.len(), settings.len(), "duplicate setting id");
        let mut keys = BTreeSet::new();
        for setting in settings {
            assert!(
                !setting.label.is_empty() && !setting.heading.is_empty(),
                "{}",
                setting.id
            );
            assert!(!setting.keys.is_empty(), "{}", setting.id);
            for key in setting.keys {
                assert!(
                    keys.insert((setting.section, *key)),
                    "{key} is edited by two settings"
                );
                // Protected data is never edited here.
                assert!(!key.starts_with("slotOrder") && !key.starts_with("linkedWeapons_"));
            }
            assert!(!["autorepeat", "autoreload"].contains(&setting.section));
            match setting.kind {
                Kind::Whole { min, max } => assert!(min < max),
                Kind::Number { min, max } => assert!(min < max),
                Kind::Choice(options) => assert!(options.len() > 1),
                Kind::Distances { min, max } => assert!(0 <= min && min < max),
                Kind::Word(options) => assert!(options.len() > 1),
                Kind::Scaled {
                    min, max, per_unit, ..
                } => assert!(min < max && per_unit > 0.0),
                Kind::Bool | Kind::Flag | Kind::Systems | Kind::Preset | Kind::ScanFilters => {}
            }
        }
    }
}

#[test]
fn each_kind_of_setting_is_read_from_its_stored_form() {
    let root = account();
    let account = FileKind::Account;
    assert_eq!(
        reading(&root, account, "default_warp_to"),
        Reading::Stored(Content::Whole(0))
    );
    assert_eq!(
        reading(&root, account, "auto_target_back"),
        Reading::Stored(Content::Whole(2))
    );
    assert_eq!(
        reading(&root, account, "timestamp_chat"),
        Reading::Stored(Content::Switch(true))
    );
    assert_eq!(
        reading(&root, account, "tooltip_delay"),
        Reading::Stored(Content::Number(171.5))
    );
    // A number the game happened to store as a whole number.
    assert_eq!(
        reading(&root, account, "window_transparency"),
        Reading::Stored(Content::Number(0.0))
    );
    assert_eq!(
        reading(&root, account, "camera_shake"),
        Reading::Stored(Content::Switch(true))
    );
    assert_eq!(
        reading(&root, account, "sound_turrets"),
        Reading::Stored(Content::Number(0.25))
    );
    assert_eq!(
        reading(&character(), FileKind::Character, "orbit_distances"),
        Reading::Stored(Content::Distances(vec![(0, 5_000), (11_393, 599)]))
    );
    assert_eq!(
        reading(&character(), FileKind::Character, "drone_aggression"),
        Reading::Stored(Content::Switch(true))
    );
}

#[test]
fn a_setting_the_file_does_not_hold_is_not_stored() {
    let root = account();
    assert_eq!(
        reading(&root, FileKind::Account, "trails"),
        Reading::NotStored
    );
    assert_eq!(
        reading(&root, FileKind::Account, "sound_atmosphere"),
        Reading::NotStored
    );
    assert_eq!(
        reading(&character(), FileKind::Character, "warp_to_distances"),
        Reading::NotStored
    );
    // No section at all.
    let bare = dict(&[(b("overview"), dict(&[]))]);
    assert_eq!(
        reading(&bare, FileKind::Account, "trails"),
        Reading::NotStored
    );
    assert_eq!(
        reading(&bare, FileKind::Account, "sound_turrets"),
        Reading::NotStored
    );
}

fn with_ui(key: &str, value: Value) -> Value {
    dict(&[(b("ui"), dict(&[(b(key), value)]))])
}

#[test]
fn anything_stored_in_an_unexpected_form_is_unreadable() {
    let account = FileKind::Account;
    let unreadable = |id: &str, key: &str, value: Value| {
        assert_eq!(
            reading(&with_ui(key, value.clone()), account, id),
            Reading::Unreadable,
            "{value:?}"
        );
    };
    // A switch stored as a number that isn't 0 or 1, or as the wrong type.
    unreadable(
        "camera_shake",
        "cameraShakeEnabled",
        stamped(1, Value::Int(2)),
    );
    unreadable(
        "camera_shake",
        "cameraShakeEnabled",
        stamped(1, Value::Bool(true)),
    );
    unreadable("timestamp_chat", "timestampchat", stamped(1, Value::Int(1)));
    unreadable(
        "auto_target_back",
        "autoTargetBack",
        stamped(1, Value::Float(2.0)),
    );
    unreadable("tooltip_delay", "tooltipDelayGeneric", stamped(1, b("171")));
    unreadable(
        "tooltip_delay",
        "tooltipDelayGeneric",
        stamped(1, Value::Float(f64::NAN)),
    );
    // Not a (timestamp, value) pair.
    unreadable("camera_shake", "cameraShakeEnabled", Value::Int(1));
    unreadable(
        "camera_shake",
        "cameraShakeEnabled",
        Value::Tuple(vec![Value::Int(1), Value::Int(1)]),
    );
    unreadable(
        "camera_shake",
        "cameraShakeEnabled",
        Value::Tuple(vec![
            Value::Long(Long::from_i64(1)),
            Value::Int(1),
            Value::Int(1),
        ]),
    );

    // The same key twice.
    let pair = (b("cameraShakeEnabled"), stamped(1, Value::Int(1)));
    let twice = dict(&[(b("ui"), dict(&[pair.clone(), pair]))]);
    assert_eq!(
        reading(&twice, account, "camera_shake"),
        Reading::Unreadable
    );
    // The section isn't a dict, or is there twice, or the file isn't a dict.
    let odd = dict(&[(b("ui"), Value::List(vec![]))]);
    assert_eq!(reading(&odd, account, "camera_shake"), Reading::Unreadable);
    let two = dict(&[(b("ui"), dict(&[])), (b("ui"), dict(&[]))]);
    assert_eq!(reading(&two, account, "camera_shake"), Reading::Unreadable);
    assert_eq!(
        reading(&Value::None, account, "camera_shake"),
        Reading::Unreadable
    );

    // Distances with anything but whole-number pairs.
    let bad = dict(&[(Value::Int(0), Value::Float(5.0))]);
    let root = with_ui("defaultTypeOrbitDist", stamped(1, bad));
    assert_eq!(
        reading(&root, FileKind::Character, "orbit_distances"),
        Reading::Unreadable
    );
    let root = with_ui("defaultTypeOrbitDist", stamped(1, Value::List(vec![])));
    assert_eq!(
        reading(&root, FileKind::Character, "orbit_distances"),
        Reading::Unreadable
    );
}

#[test]
fn a_setting_stored_in_two_places_must_agree() {
    let root = account();
    // custom_uiclick is 0.5 but soundLevel_custom_uiclick is 0.75.
    assert_eq!(
        reading(&root, FileKind::Account, "sound_ui_click"),
        Reading::Unreadable
    );
    // One of the two missing also counts as disagreeing.
    let half = dict(&[(
        b("audio"),
        dict(&[(b("custom_turrets"), stamped(1, Value::Float(0.25)))]),
    )]);
    assert_eq!(
        reading(&half, FileKind::Account, "sound_turrets"),
        Reading::Unreadable
    );
    let changes = [change("sound_ui_click", Content::Number(0.1))];
    assert_eq!(
        apply(&root, FileKind::Account, &changes, &now()),
        Err(EditError::CannotChange("UI clicks"))
    );
}

#[test]
fn an_edit_changes_the_value_and_timestamp_and_nothing_else() {
    let root = account();
    let changes = [
        change("camera_shake", Content::Switch(false)),
        change("auto_target_back", Content::Whole(5)),
    ];
    let result = edited(&root, FileKind::Account, &changes);
    assert_eq!(
        reading(&result, FileKind::Account, "camera_shake"),
        Reading::Stored(Content::Switch(false))
    );
    assert_eq!(
        reading(&result, FileKind::Account, "auto_target_back"),
        Reading::Stored(Content::Whole(5))
    );

    // Put the two old pairs back and the file is the original, entry for
    // entry and in the same order.
    let Value::Dict(mut sections) = result.clone() else {
        unreachable!()
    };
    let Value::Dict(ui) = &mut sections[1].1 else {
        unreachable!()
    };
    assert_eq!(
        ui[6],
        (b("cameraShakeEnabled"), stamped(999, Value::Int(0)))
    );
    assert_eq!(ui[2], (b("autoTargetBack"), stamped(999, Value::Int(5))));
    ui[6].1 = stamped(1, Value::Int(1));
    ui[2].1 = stamped(1, Value::Int(2));
    assert_eq!(Value::Dict(sections), root);
}

#[test]
fn a_setting_under_two_keys_is_written_to_both() {
    let result = edited(
        &account(),
        FileKind::Account,
        &[change("sound_turrets", Content::Number(1.0))],
    );
    let Value::Dict(sections) = &result else {
        unreachable!()
    };
    let Value::Dict(audio) = &sections[2].1 else {
        unreachable!()
    };
    assert_eq!(audio[0].1, stamped(999, Value::Float(1.0)));
    assert_eq!(audio[1].1, stamped(999, Value::Float(1.0)));
    // The disagreeing pair next to it is left exactly as it was.
    assert_eq!(audio[2].1, stamped(1, Value::Float(0.5)));
    assert_eq!(audio[3].1, stamped(1, Value::Float(0.75)));
}

#[test]
fn numbers_keep_the_form_the_game_stored_them_in() {
    let root = account();
    let transparency = |x: f64| {
        let result = edited(
            &root,
            FileKind::Account,
            &[change("window_transparency", Content::Number(x))],
        );
        let Value::Dict(sections) = result else {
            unreachable!()
        };
        let Value::Dict(ui) = &sections[1].1 else {
            unreachable!()
        };
        ui[5].1.clone()
    };
    // Stored as a whole number: a whole value stays whole, a fraction can't.
    assert_eq!(transparency(1.0), stamped(999, Value::Int(1)));
    assert_eq!(transparency(0.5), stamped(999, Value::Float(0.5)));
    // Stored as a float: stays a float even when the value is whole.
    let result = edited(
        &root,
        FileKind::Account,
        &[change("tooltip_delay", Content::Number(200.0))],
    );
    let Value::Dict(sections) = result else {
        unreachable!()
    };
    let Value::Dict(ui) = &sections[1].1 else {
        unreachable!()
    };
    assert_eq!(ui[4].1, stamped(999, Value::Float(200.0)));
}

#[test]
fn a_setting_that_was_not_stored_is_added_at_the_end() {
    let root = account();
    let result = edited(
        &root,
        FileKind::Account,
        &[change("trails", Content::Switch(true))],
    );
    let Value::Dict(sections) = &result else {
        unreachable!()
    };
    let Value::Dict(ui) = &sections[1].1 else {
        unreachable!()
    };
    assert_eq!(ui.len(), 10);
    assert_eq!(ui[9], (b("trailsEnabled"), stamped(999, Value::Int(1))));

    // A missing section is created, after the existing ones.
    let bare = dict(&[(b("overview"), dict(&[]))]);
    let result = edited(
        &bare,
        FileKind::Account,
        &[change("sound_turrets", Content::Number(0.5))],
    );
    assert_eq!(
        result,
        dict(&[
            (b("overview"), dict(&[])),
            (
                b("audio"),
                dict(&[
                    (b("custom_turrets"), stamped(999, Value::Float(0.5))),
                    (
                        b("soundLevel_custom_turrets"),
                        stamped(999, Value::Float(0.5))
                    ),
                ])
            ),
        ])
    );
}

#[test]
fn distances_can_be_changed_removed_and_added() {
    let root = character();
    let new = vec![(0, 7_500), (638, 20_000)];
    let changes = [change("orbit_distances", Content::Distances(new.clone()))];
    let result = edited(&root, FileKind::Character, &changes);
    assert_eq!(
        reading(&result, FileKind::Character, "orbit_distances"),
        Reading::Stored(Content::Distances(new))
    );
    assert_eq!(
        reading(&result, FileKind::Character, "drone_aggression"),
        Reading::Stored(Content::Switch(true))
    );
    // An empty list is allowed: every type goes back to the game's default.
    let changes = [change("orbit_distances", Content::Distances(vec![]))];
    let result = edited(&root, FileKind::Character, &changes);
    assert_eq!(
        reading(&result, FileKind::Character, "orbit_distances"),
        Reading::Stored(Content::Distances(vec![]))
    );
}

#[test]
fn values_outside_what_a_setting_allows_are_refused() {
    let root = account();
    let refused = |kind: FileKind, root: &Value, id: &'static str, content: Content| {
        let label = setting(kind, id).label;
        assert_eq!(
            apply(root, kind, &[change(id, content.clone())], &now()),
            Err(EditError::NotAllowed(label)),
            "{id}: {content:?}"
        );
    };
    let account = FileKind::Account;
    refused(account, &root, "auto_target_back", Content::Whole(-1));
    refused(account, &root, "auto_target_back", Content::Whole(6));
    refused(account, &root, "default_warp_to", Content::Whole(15_000));
    refused(account, &root, "default_warp_to", Content::Whole(-10_000));
    refused(account, &root, "window_transparency", Content::Number(1.01));
    refused(
        account,
        &root,
        "window_transparency",
        Content::Number(-0.01),
    );
    refused(
        account,
        &root,
        "window_transparency",
        Content::Number(f64::NAN),
    );
    refused(
        account,
        &root,
        "sound_turrets",
        Content::Number(f64::INFINITY),
    );
    // The wrong kind of value altogether.
    refused(account, &root, "camera_shake", Content::Whole(1));
    refused(account, &root, "timestamp_chat", Content::Number(1.0));
    refused(account, &root, "auto_target_back", Content::Switch(true));
    refused(account, &root, "tooltip_delay", Content::Whole(100));
    refused(account, &root, "camera_shake", Content::Distances(vec![]));

    let character = character();
    let kind = FileKind::Character;
    refused(
        kind,
        &character,
        "orbit_distances",
        Content::Distances(vec![(-1, 5)]),
    );
    refused(
        kind,
        &character,
        "orbit_distances",
        Content::Distances(vec![(5, -1)]),
    );
    refused(
        kind,
        &character,
        "orbit_distances",
        Content::Distances(vec![(5, 1_000_001)]),
    );
    refused(
        kind,
        &character,
        "orbit_distances",
        Content::Distances(vec![(5, 1), (5, 2)]),
    );
    refused(kind, &character, "orbit_distances", Content::Switch(true));

    // The edges are allowed.
    for content in [Content::Whole(0), Content::Whole(5)] {
        assert!(apply(
            &root,
            account,
            &[change("auto_target_back", content)],
            &now()
        )
        .is_ok());
    }
    assert!(apply(
        &root,
        account,
        &[change("default_warp_to", Content::Whole(100_000))],
        &now()
    )
    .is_ok());
    let edge = Content::Distances(vec![(0, 500), (5, 1_000_000)]);
    assert!(apply(&character, kind, &[change("orbit_distances", edge)], &now()).is_ok());
}

#[test]
fn unknown_settings_and_double_edits_are_refused() {
    let root = account();
    // A character setting in an account file.
    assert_eq!(
        apply(
            &root,
            FileKind::Account,
            &[change("drone_aggression", Content::Switch(true))],
            &now()
        ),
        Err(EditError::UnknownSetting("drone_aggression".to_owned()))
    );
    assert_eq!(
        apply(
            &root,
            FileKind::Account,
            &[change("nonsense", Content::Switch(true))],
            &now()
        ),
        Err(EditError::UnknownSetting("nonsense".to_owned()))
    );
    let twice = [
        change("camera_shake", Content::Switch(false)),
        change("camera_shake", Content::Switch(true)),
    ];
    assert_eq!(
        apply(&root, FileKind::Account, &twice, &now()),
        Err(EditError::NotAllowed("Camera shake"))
    );
    assert!(apply(&Value::None, FileKind::Account, &[], &now()).is_err());
    // No changes is simply the same file.
    assert_eq!(apply(&root, FileKind::Account, &[], &now()), Ok(root));
}

#[test]
fn the_original_is_not_modified() {
    let root = account();
    let before = root.clone();
    apply(
        &root,
        FileKind::Account,
        &[change("camera_shake", Content::Switch(false))],
        &now(),
    )
    .unwrap();
    assert_eq!(root, before);
}

#[test]
fn verification_catches_any_other_difference() {
    let root = account();
    let kind = FileKind::Account;
    let changes = [change("camera_shake", Content::Switch(false))];
    let good = apply(&root, kind, &changes, &now()).unwrap();
    let check = |output: &Value| {
        verify(
            &encode(output, false).unwrap(),
            &root,
            kind,
            &changes,
            &now(),
        )
    };
    assert_eq!(check(&good), Ok(()));

    let self_check = |message| Err(EditError::SelfCheck(message));
    // The edit wasn't made, or was made with the wrong value or time.
    assert_eq!(
        check(&root),
        self_check("an edited setting doesn't hold the new value")
    );
    let wrong = apply(
        &root,
        kind,
        &[change("camera_shake", Content::Switch(true))],
        &now(),
    )
    .unwrap();
    assert_eq!(
        check(&wrong),
        self_check("an edited setting doesn't hold the new value")
    );
    let late = apply(&root, kind, &changes, &Long::from_i64(1_000)).unwrap();
    assert_eq!(
        check(&late),
        self_check("an edited setting doesn't hold the new value")
    );
    // Something else changed too.
    let extra = [
        changes[0].clone(),
        change("auto_target_back", Content::Whole(0)),
    ];
    let too_much = apply(&root, kind, &extra, &now()).unwrap();
    assert_eq!(
        check(&too_much),
        self_check("a setting that wasn't edited has changed")
    );
    // Entries reordered, or one lost.
    let Value::Dict(mut sections) = good.clone() else {
        unreachable!()
    };
    sections.swap(0, 2);
    assert_eq!(
        check(&Value::Dict(sections)),
        self_check("a setting that wasn't edited has changed")
    );
    let Value::Dict(mut sections) = good.clone() else {
        unreachable!()
    };
    sections.remove(0);
    assert_eq!(
        check(&Value::Dict(sections)),
        self_check("a setting that wasn't edited has changed")
    );
    // The edited key stored twice.
    let Value::Dict(mut sections) = good.clone() else {
        unreachable!()
    };
    let Value::Dict(ui) = &mut sections[1].1 else {
        unreachable!()
    };
    ui.push((b("cameraShakeEnabled"), stamped(999, Value::Int(0))));
    assert_eq!(
        check(&Value::Dict(sections)),
        self_check("an edited setting doesn't hold the new value")
    );
    // Not a clean file.
    let mut data = encode(&good, false).unwrap();
    data.push(1);
    assert_eq!(
        verify(&data, &root, kind, &changes, &now()),
        self_check("the output can't be read back")
    );
    assert_eq!(
        verify(&data[..30], &root, kind, &changes, &now()),
        self_check("the output can't be read back")
    );
}

#[test]
fn timestamps_are_windows_file_times() {
    // 1970-01-01 as a FILETIME.
    assert_eq!(
        timestamp_from_unix(Duration::ZERO),
        Long::from_i64(116_444_736_000_000_000)
    );
    assert_eq!(
        timestamp_from_unix(Duration::new(1, 500)),
        Long::from_i64(116_444_736_010_000_005)
    );
    // 2021-11-11 21:07:01.0349344 UTC, the timestamp seen in a real file.
    assert_eq!(
        timestamp_from_unix(Duration::new(1_636_664_821, 34_934_400)),
        Long::from_i64(132_811_384_210_349_344)
    );
    let recent = timestamp_now().to_i64().unwrap();
    // After 2026-01-01 and before 2100.
    assert!(
        recent > 134_117_000_000_000_000 && recent < 157_470_000_000_000_000,
        "{recent}"
    );
}

fn folder_with_account() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core_user_1.dat");
    fs::write(&path, encode(&account(), false).unwrap()).unwrap();
    (dir, path)
}

#[test]
fn saving_backs_up_then_writes_the_change() {
    let (dir, path) = folder_with_account();
    let before = fs::read(&path).unwrap();
    let opened = open(&path).unwrap();
    assert_eq!(
        (opened.kind, opened.root, opened.fingerprint),
        (FileKind::Account, account(), Fingerprint::of(&before))
    );

    let changes = [change("camera_shake", Content::Switch(false))];
    let backup = save(
        &path,
        as_opened(&path),
        &changes,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &now(),
    )
    .unwrap();
    assert_eq!(backup, dir.path().join("clone-bay-backups").join("T"));
    assert_eq!(fs::read(backup.join("core_user_1.dat")).unwrap(), before);

    let written = decode(&fs::read(&path).unwrap()).unwrap().value;
    assert_eq!(
        written,
        apply(&account(), FileKind::Account, &changes, &now()).unwrap()
    );
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        2,
        "no temp file left"
    );
}

#[test]
fn saves_made_in_one_run_keep_one_backup_of_the_original() {
    let (dir, path) = folder_with_account();
    let before = fs::read(&path).unwrap();
    let run = BackupSession::new("T");
    let attempt = |on: bool| {
        let changes = [change("camera_shake", Content::Switch(on))];
        save(
            &path,
            as_opened(&path),
            &changes,
            EveStatus::NotRunning,
            false,
            &run,
            &now(),
        )
    };

    let first = attempt(false).unwrap();
    let after_first = fs::read(&path).unwrap();
    let second = attempt(true).unwrap();
    assert_ne!(fs::read(&path).unwrap(), after_first, "it was saved again");

    assert_eq!(second, first);
    let all = dir.path().join("clone-bay-backups");
    assert_eq!(fs::read_dir(&all).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&first).unwrap().count(), 1);
    assert_eq!(fs::read(first.join("core_user_1.dat")).unwrap(), before);
}

#[test]
fn a_file_with_a_checksum_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core_user_1.dat");
    fs::write(&path, encode(&account(), true).unwrap()).unwrap();
    let changes = [change("camera_shake", Content::Switch(false))];
    save(
        &path,
        as_opened(&path),
        &changes,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &now(),
    )
    .unwrap();
    assert!(decode(&fs::read(&path).unwrap()).unwrap().had_checksum);
}

#[test]
fn nothing_is_saved_while_eve_is_or_might_be_running() {
    let (dir, path) = folder_with_account();
    let before = fs::read(&path).unwrap();
    let changes = [change("camera_shake", Content::Switch(false))];
    for confirmed in [false, true] {
        assert_eq!(
            save(
                &path,
                as_opened(&path),
                &changes,
                EveStatus::Running,
                confirmed,
                &BackupSession::new("T"),
                &now()
            ),
            Err(EditError::EveRunning)
        );
    }
    assert_eq!(
        save(
            &path,
            as_opened(&path),
            &changes,
            EveStatus::Unknown,
            false,
            &BackupSession::new("T"),
            &now()
        ),
        Err(EditError::EveUnknown)
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        1,
        "no backup folder was made"
    );
    for error in [EditError::EveRunning, EditError::EveUnknown] {
        assert!(error.to_string().ends_with("Nothing was changed"));
    }

    assert!(save(
        &path,
        as_opened(&path),
        &changes,
        EveStatus::Unknown,
        true,
        &BackupSession::new("T"),
        &now()
    )
    .is_ok());
}

#[test]
fn a_file_that_changed_after_it_was_opened_is_not_saved_over() {
    // The screen was opened, then the game saved the file before the user
    // pressed Save. Whatever the game wrote must not be undone.
    let (dir, path) = folder_with_account();
    let opened = open(&path).unwrap();
    let by_the_game = encode(
        &with_ui("timestampchat", stamped(5, Value::Bool(true))),
        false,
    )
    .unwrap();
    fs::write(&path, &by_the_game).unwrap();

    let changes = [change("camera_shake", Content::Switch(false))];
    assert_eq!(
        save(
            &path,
            opened.fingerprint,
            &changes,
            EveStatus::NotRunning,
            false,
            &BackupSession::new("T"),
            &now(),
        ),
        Err(EditError::ChangedSinceOpened)
    );
    assert_eq!(fs::read(&path).unwrap(), by_the_game);
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        1,
        "no backup folder was made"
    );
    assert_eq!(
        EditError::ChangedSinceOpened.to_string(),
        "the file changed after it was opened (EVE may have saved it), so nothing was saved. Press \
         \"Discard changes\" to load it again"
    );
}

#[test]
fn refused_saves_leave_the_file_and_folder_untouched() {
    let (dir, path) = folder_with_account();
    let before = fs::read(&path).unwrap();
    let save_with = |changes: &[Change]| {
        save(
            &path,
            as_opened(&path),
            changes,
            EveStatus::NotRunning,
            false,
            &BackupSession::new("T"),
            &now(),
        )
    };

    assert_eq!(save_with(&[]), Err(EditError::NothingToSave));
    assert_eq!(
        save_with(&[change("auto_target_back", Content::Whole(99))]),
        Err(EditError::NotAllowed(
            "Auto target back (number of targets)"
        ))
    );
    assert_eq!(
        save_with(&[change("sound_ui_click", Content::Number(0.5))]),
        Err(EditError::CannotChange("UI clicks"))
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);

    let changes = [change("camera_shake", Content::Switch(false))];
    let other = dir.path().join("notes.txt");
    assert_eq!(
        save(
            &other,
            as_opened(&other),
            &changes,
            EveStatus::NotRunning,
            false,
            &BackupSession::new("T"),
            &now()
        ),
        Err(EditError::NotASettingsFile("notes.txt".to_owned()))
    );
    assert_eq!(
        open(&other),
        Err(EditError::NotASettingsFile("notes.txt".to_owned()))
    );
    let missing = dir.path().join("core_user_9.dat");
    let result = save(
        &missing,
        as_opened(&missing),
        &changes,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &now(),
    );
    assert!(
        matches!(result, Err(EditError::Unreadable { .. })),
        "{result:?}"
    );
    assert!(matches!(open(&missing), Err(EditError::Unreadable { .. })));
    fs::write(&missing, b"damaged").unwrap();
    let result = save(
        &missing,
        as_opened(&missing),
        &changes,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &now(),
    );
    assert_eq!(
        result,
        Err(EditError::Unreadable {
            file: "core_user_9.dat".to_owned(),
            reason: "can't decode it (not a marshal stream (bad signature))".to_owned(),
        })
    );
}

#[test]
fn a_number_that_is_not_finite_is_unreadable() {
    for odd in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        let root = with_ui("tooltipDelayGeneric", stamped(1, Value::Float(odd)));
        assert_eq!(
            reading(&root, FileKind::Account, "tooltip_delay"),
            Reading::Unreadable,
            "{odd}"
        );
        let root = with_ui("dir_scanrange", stamped(1, Value::Float(odd)));
        assert_eq!(
            reading(&root, FileKind::Account, "dscan_range"),
            Reading::Unreadable,
            "{odd}"
        );
    }
}

#[test]
fn a_true_or_false_setting_is_written_as_true_or_false() {
    let changes = [change("timestamp_chat", Content::Switch(false))];
    let result = edited(&account(), FileKind::Account, &changes);
    let Value::Dict(sections) = &result else {
        unreachable!()
    };
    let Value::Dict(ui) = &sections[1].1 else {
        unreachable!()
    };
    assert_eq!(
        ui[3],
        (b("timestampchat"), stamped(999, Value::Bool(false)))
    );
}

#[test]
fn every_setting_of_a_file_can_be_read_in_one_go() {
    let (_dir, path) = folder_with_account();
    let readings = read_file(&path).unwrap();
    assert_eq!(readings.len(), settings_for(FileKind::Account).len());
    for ((setting, reading), expected) in readings.iter().zip(settings_for(FileKind::Account)) {
        assert_eq!(setting.id, expected.id);
        assert_eq!(*reading, read(&account(), expected), "{}", setting.id);
    }
    assert!(readings
        .iter()
        .any(|(_, reading)| matches!(reading, Reading::Stored(_))));
    assert!(read_file(&path.with_file_name("core_user_9.dat")).is_err());
}

#[test]
fn verification_notices_an_empty_section_going_missing() {
    let kind = FileKind::Account;
    let changes = [change("camera_shake", Content::Switch(false))];
    let Value::Dict(mut sections) = account() else {
        unreachable!()
    };
    sections.push((b("empty"), dict(&[])));
    let original = Value::Dict(sections);
    // The edit applied to a file that lacks the empty section.
    let without = apply(&account(), kind, &changes, &now()).unwrap();
    let data = encode(&without, false).unwrap();
    assert_eq!(
        verify(&data, &original, kind, &changes, &now()),
        Err(EditError::SelfCheck(
            "a setting that wasn't edited has changed"
        ))
    );
    let with = apply(&original, kind, &changes, &now()).unwrap();
    let data = encode(&with, false).unwrap();
    assert_eq!(verify(&data, &original, kind, &changes, &now()), Ok(()));
}

// Written with the distance limits and not yet run when committed.
#[test]
fn each_distance_list_has_its_own_limits() {
    let kind = FileKind::Character;
    let root = dict(&[(b("ui"), dict(&[]))]);
    let attempt = |id: &'static str, metres: i64| {
        apply(
            &root,
            kind,
            &[change(id, Content::Distances(vec![(638, metres)]))],
            &now(),
        )
        .is_ok()
    };
    // Orbit and keep at range: 500 m to 1,000 km.
    for id in ["orbit_distances", "keep_at_range_distances"] {
        assert!(!attempt(id, 499), "{id}");
        assert!(attempt(id, 500), "{id}");
        assert!(attempt(id, 1_000_000), "{id}");
        assert!(!attempt(id, 1_000_001), "{id}");
        assert!(!attempt(id, 0), "{id}");
    }
    // Warp to: 0 to 100 km.
    assert!(attempt("warp_to_distances", 0));
    assert!(attempt("warp_to_distances", 100_000));
    assert!(!attempt("warp_to_distances", 100_001));
    assert!(!attempt("warp_to_distances", -1));
}

#[test]
fn a_distance_already_outside_the_limits_may_stay_but_not_be_set() {
    let kind = FileKind::Character;
    // The file holds an orbit distance of 100 m, below the 500 m floor.
    let stored = dict(&[
        (Value::Int(0), Value::Int(100)),
        (Value::Int(638), Value::Int(2_000)),
    ]);
    let root = dict(&[(
        b("ui"),
        dict(&[(b("defaultTypeOrbitDist"), stamped(1, stored))]),
    )]);
    let attempt = |entries: Vec<(i64, i64)>| {
        apply(
            &root,
            kind,
            &[change("orbit_distances", Content::Distances(entries))],
            &now(),
        )
    };
    // Changing the other entry is fine; the old one is left as it was.
    assert!(attempt(vec![(0, 100), (638, 3_000)]).is_ok());
    // Setting something new below the floor is not, nor moving the old
    // value to a different type.
    assert!(attempt(vec![(0, 200), (638, 2_000)]).is_err());
    assert!(attempt(vec![(638, 100)]).is_err());
}

/// A character with autopilot settings as the game stores them: the route
/// as a byte string, and avoided systems as a mix of the two kinds of whole
/// number.
fn pilot(route: Value, systems: &[Value]) -> Value {
    dict(&[(
        b("ui"),
        dict(&[
            (b("before"), stamped(1, Value::None)),
            (b("pfRouteType"), stamped(1, route)),
            (
                b("pathFinder_includeJumpGates"),
                stamped(1, Value::Bool(true)),
            ),
            (
                b("autopilot_avoidance2"),
                stamped(1, Value::List(systems.to_vec())),
            ),
            (b("after"), stamped(1, Value::None)),
        ]),
    )])
}

fn long(id: i64) -> Value {
    Value::Long(Long::from_i64(id))
}

fn systems(root: &Value) -> Vec<clone_bay_core::edit::ListedId> {
    match reading(root, FileKind::Character, "avoided_systems") {
        Reading::Stored(Content::Systems(systems)) => systems,
        other => panic!("avoided systems read as {other:?}"),
    }
}

fn stored_in_ui<'a>(root: &'a Value, key: &str) -> &'a Value {
    let Value::Dict(sections) = root else {
        panic!("not a dict");
    };
    let Value::Dict(entries) = &sections[0].1 else {
        panic!("not a dict");
    };
    &entries.iter().find(|(name, _)| *name == b(key)).unwrap().1
}

#[test]
fn autopilot_settings_are_read_as_stored() {
    let character = FileKind::Character;
    let root = pilot(b("safe"), &[Value::Int(30_000_455), long(30_000_318)]);
    assert_eq!(
        reading(&root, character, "route_type"),
        Reading::Stored(Content::Word("safe".to_owned()))
    );
    assert_eq!(
        reading(&root, character, "route_jump_gates"),
        Reading::Stored(Content::Switch(true))
    );
    let ids: Vec<i64> = systems(&root).iter().map(|system| system.id).collect();
    assert_eq!(ids, [30_000_455, 30_000_318]);
    assert!(systems(&pilot(b("safe"), &[])).is_empty());

    // The route as text rather than a byte string reads the same.
    assert_eq!(
        reading(
            &pilot(Value::Str("shortest".into()), &[]),
            character,
            "route_type"
        ),
        Reading::Stored(Content::Word("shortest".to_owned()))
    );
    // A route this app has no name for, or anything that isn't a word, and
    // a list holding something that isn't an ID, can't be edited.
    for odd in [b("unheard of"), Value::Int(1), Value::None] {
        assert_eq!(
            reading(&pilot(odd, &[]), character, "route_type"),
            Reading::Unreadable
        );
    }
    assert_eq!(
        reading(
            &pilot(b("safe"), &[b("Jita")]),
            character,
            "avoided_systems"
        ),
        Reading::Unreadable
    );
    assert_eq!(
        reading(&dict(&[(b("ui"), dict(&[]))]), character, "avoided_systems"),
        Reading::NotStored
    );
}

#[test]
fn the_route_type_is_changed_and_stays_the_kind_of_string_it_was() {
    let character = FileKind::Character;
    let chosen = |word: &str| [change("route_type", Content::Word(word.to_owned()))];
    let root = pilot(b("shortest"), &[]);
    let result = edited(&root, character, &chosen("safe"));
    assert_eq!(
        stored_in_ui(&result, "pfRouteType"),
        &Value::Tuple(vec![Value::Long(now()), b("safe")])
    );
    let as_text = pilot(Value::Str("shortest".into()), &[]);
    let result = edited(&as_text, character, &chosen("safe"));
    assert_eq!(
        stored_in_ui(&result, "pfRouteType"),
        &Value::Tuple(vec![Value::Long(now()), Value::Str("safe".into())])
    );
    // A file that doesn't store it gets a byte string, as the game writes.
    let bare = dict(&[(b("ui"), dict(&[]))]);
    let result = edited(&bare, character, &chosen("safe"));
    assert_eq!(
        stored_in_ui(&result, "pfRouteType"),
        &Value::Tuple(vec![Value::Long(now()), b("safe")])
    );

    // Only the words the app knows are accepted.
    for refused in [
        Content::Word("unsafe".to_owned()),
        Content::Word(String::new()),
        Content::Whole(1),
    ] {
        assert_eq!(
            apply(&root, character, &[change("route_type", refused)], &now()),
            Err(EditError::NotAllowed("Route"))
        );
    }
}

#[test]
fn systems_can_be_taken_off_the_avoid_list_but_not_put_on() {
    let character = FileKind::Character;
    let stored = [
        Value::Int(30_000_455),
        long(30_000_318),
        Value::Int(30_001_010),
    ];
    let root = pilot(b("safe"), &stored);
    let all = systems(&root);
    let saved = |wanted: &[clone_bay_core::edit::ListedId]| {
        let changes = [change("avoided_systems", Content::Systems(wanted.to_vec()))];
        let result = edited(&root, character, &changes);
        stored_in_ui(&result, "autopilot_avoidance2").clone()
    };
    let list =
        |items: &[Value]| Value::Tuple(vec![Value::Long(now()), Value::List(items.to_vec())]);

    // Each system left on the list is written back as the kind of number
    // it was.
    assert_eq!(
        saved(&[all[1], all[2]]),
        list(&[long(30_000_318), Value::Int(30_001_010)])
    );
    assert_eq!(saved(&[all[0]]), list(&[Value::Int(30_000_455)]));
    assert_eq!(saved(&[]), list(&[]));
    // Nothing else in the file is touched.
    let changes = [change(
        "avoided_systems",
        Content::Systems(vec![all[0], all[2]]),
    )];
    let result = edited(&root, character, &changes);
    assert_eq!(
        reading(&result, character, "route_type"),
        Reading::Stored(Content::Word("safe".to_owned()))
    );
    assert_eq!(
        stored_in_ui(&result, "pfRouteType"),
        stored_in_ui(&root, "pfRouteType")
    );

    // Refused: a system the file doesn't hold, the same system as another
    // kind of number, one listed twice, a changed order, and a list for a
    // file that stores none.
    let other = systems(&pilot(b("safe"), &[Value::Int(30_000_142)]))[0];
    let as_int = systems(&pilot(b("safe"), &[Value::Int(30_000_318)]))[0];
    let refused = |wanted: Vec<clone_bay_core::edit::ListedId>, root: &Value| {
        let changes = [change("avoided_systems", Content::Systems(wanted))];
        assert_eq!(
            apply(root, character, &changes, &now()),
            Err(EditError::NotAllowed("Systems to avoid"))
        );
    };
    refused(vec![all[0], other], &root);
    refused(vec![other], &root);
    refused(vec![as_int], &root);
    refused(vec![all[0], all[0]], &root);
    refused(vec![all[1], all[0]], &root);
    refused(vec![], &dict(&[(b("ui"), dict(&[]))]));
    refused(vec![all[0]], &dict(&[(b("ui"), dict(&[]))]));

    // A system the file holds twice may be left on twice.
    let twice = pilot(b("safe"), &[Value::Int(5), Value::Int(5)]);
    let both = systems(&twice);
    let changes = [change("avoided_systems", Content::Systems(both.clone()))];
    assert!(apply(&twice, character, &changes, &now()).is_ok());
    let changes = [change(
        "avoided_systems",
        Content::Systems(vec![both[0]; 3]),
    )];
    assert!(apply(&twice, character, &changes, &now()).is_err());
}

#[test]
fn asteroid_field_effects_are_switches_stored_as_one_or_nought() {
    let account = FileKind::Account;
    let root = dict(&[(
        b("ui"),
        dict(&[
            (b("UI_ASTEROID_ATMOSPHERICS"), stamped(1, Value::Int(0))),
            (b("UI_ASTEROID_FOG"), stamped(1, Value::Int(1))),
            (b("UI_ASTEROID_GODRAYS"), stamped(1, Value::Bool(true))),
        ]),
    )]);
    assert_eq!(
        reading(&root, account, "asteroid_atmospherics"),
        Reading::Stored(Content::Switch(false))
    );
    assert_eq!(
        reading(&root, account, "asteroid_fog"),
        Reading::Stored(Content::Switch(true))
    );
    assert_eq!(
        reading(&root, account, "asteroid_cloud_field"),
        Reading::NotStored
    );
    assert_eq!(
        reading(&root, account, "asteroid_particles"),
        Reading::NotStored
    );
    // Stored as true/false rather than 1/0: not the form the game uses.
    assert_eq!(
        reading(&root, account, "asteroid_god_rays"),
        Reading::Unreadable
    );

    let changes = [
        change("asteroid_atmospherics", Content::Switch(true)),
        change("asteroid_particles", Content::Switch(false)),
    ];
    let result = edited(&root, account, &changes);
    let Value::Dict(sections) = &result else {
        panic!("not a dict");
    };
    let Value::Dict(entries) = &sections[0].1 else {
        panic!("not a dict");
    };
    let held = |key: &str| &entries.iter().find(|(name, _)| *name == b(key)).unwrap().1;
    assert_eq!(
        held("UI_ASTEROID_ATMOSPHERICS"),
        &Value::Tuple(vec![Value::Long(now()), Value::Int(1)])
    );
    assert_eq!(
        held("UI_ASTEROID_PARTICLES"),
        &Value::Tuple(vec![Value::Long(now()), Value::Int(0)])
    );
    assert_eq!(held("UI_ASTEROID_FOG"), &stamped(1, Value::Int(1)));
}

/// An account with scanning settings as the game stores them.
fn scanner(range: Value, active: Option<Value>) -> Value {
    let filter = |name: Value, groups: &[i64]| {
        Value::Tuple(vec![
            name,
            Value::List(groups.iter().map(|group| Value::Int(*group)).collect()),
        ])
    };
    let mut ui = vec![
        (b("dir_scanrange"), stamped(1, range)),
        (
            b("scan_angleSlider"),
            stamped(1, Value::Float(std::f64::consts::TAU)),
        ),
        (
            b("scanner_presetInUse"),
            stamped(1, Value::Str("Travel".into())),
        ),
        (
            b("probescanning.resultFilter.filters"),
            stamped(
                1,
                dict(&[
                    (
                        Value::Int(12),
                        filter(Value::Str("boosher".into()), &[1534]),
                    ),
                    (Value::Int(13), filter(b("dictor"), &[541])),
                    (
                        Value::Int(16),
                        filter(Value::Str("ships".into()), &[25, 26, 27]),
                    ),
                ]),
            ),
        ),
    ];
    if let Some(active) = active {
        ui.push((
            b("probescanning.resultFilter.activeFilterSet"),
            stamped(1, active),
        ));
    }
    dict(&[
        (
            b("overview"),
            dict(&[(
                b("overviewProfilePresets"),
                stamped(
                    1,
                    dict(&[
                        (b("DefaultPreset_1"), dict(&[])),
                        (Value::Str("Travel".into()), dict(&[])),
                        (Value::Str("<b>Combat</b>".into()), dict(&[])),
                    ]),
                ),
            )]),
        ),
        (b("ui"), Value::Dict(ui)),
    ])
}

fn set_of(ids: &[i64]) -> Value {
    Value::Reduce(Box::new(clone_bay_marshal::Reduce {
        callable: Some(Value::Global("__builtin__.set".into())),
        args: Value::Tuple(vec![Value::List(
            ids.iter().map(|id| Value::Int(*id)).collect(),
        )]),
        state: None,
        list_items: vec![],
        dict_items: vec![],
    }))
}

fn held<'a>(root: &'a Value, section: &str, key: &str) -> &'a Value {
    let Value::Dict(sections) = root else {
        panic!("not a dict");
    };
    let (_, entries) = sections
        .iter()
        .find(|(name, _)| *name == b(section))
        .unwrap();
    let Value::Dict(entries) = entries else {
        panic!("not a dict");
    };
    &entries.iter().find(|(name, _)| *name == b(key)).unwrap().1
}

fn at(value: Value) -> Value {
    Value::Tuple(vec![Value::Long(now()), value])
}

#[test]
fn the_scan_range_is_shown_in_au_and_stored_in_kilometres() {
    let account = FileKind::Account;
    let range = |root: &Value| reading(root, account, "dscan_range");
    // The game's longest range, as a whole number and as a decimal.
    for stored in [Value::Int(2_139_249_551), Value::Float(2_139_249_551.0)] {
        assert_eq!(
            range(&scanner(stored, None)),
            Reading::Stored(Content::Number(14.3))
        );
    }
    assert_eq!(
        range(&scanner(Value::Int(13_000_000), None)),
        Reading::Stored(Content::Number(0.087))
    );
    // The "as far as it goes" value is past the limit; it is shown as it is.
    assert_eq!(
        range(&scanner(Value::Float(2_147_483_647.0), None)),
        Reading::Stored(Content::Number(14.355))
    );
    assert_eq!(
        range(&scanner(Value::Float(f64::NAN), None)),
        Reading::Unreadable
    );
    assert_eq!(range(&scanner(b("far"), None)), Reading::Unreadable);

    // Saved as whole kilometres, in the kind of number the file held.
    let set = |au: f64| [change("dscan_range", Content::Number(au))];
    let root = scanner(Value::Int(13_000_000), None);
    let result = edited(&root, account, &set(14.3));
    assert_eq!(
        held(&result, "ui", "dir_scanrange"),
        &at(Value::Int(2_139_249_551))
    );
    let root = scanner(Value::Float(2_147_483_647.0), None);
    let result = edited(&root, account, &set(14.3));
    assert_eq!(
        held(&result, "ui", "dir_scanrange"),
        &at(Value::Float(2_139_249_551.0))
    );
    let result = edited(&root, account, &set(1.0));
    assert_eq!(
        held(&result, "ui", "dir_scanrange"),
        &at(Value::Float(149_597_871.0))
    );
    let result = edited(&root, account, &set(0.0));
    assert_eq!(held(&result, "ui", "dir_scanrange"), &at(Value::Float(0.0)));
    // A file that doesn't store it gets a decimal, as most files hold.
    let bare = dict(&[(b("ui"), dict(&[]))]);
    let result = edited(&bare, account, &set(5.0));
    assert_eq!(
        held(&result, "ui", "dir_scanrange"),
        &at(Value::Float(747_989_354.0))
    );
    for refused in [14.31, -0.1, f64::NAN] {
        assert_eq!(
            apply(&root, account, &set(refused), &now()),
            Err(EditError::NotAllowed("Directional scan range"))
        );
    }
}

#[test]
fn the_scan_angle_is_shown_in_degrees_and_stored_in_radians() {
    let account = FileKind::Account;
    let root = scanner(Value::Int(1), None);
    assert_eq!(
        reading(&root, account, "dscan_angle"),
        Reading::Stored(Content::Number(360.0))
    );
    let set = |degrees: f64| [change("dscan_angle", Content::Number(degrees))];
    let result = edited(&root, account, &set(90.0));
    assert_eq!(
        held(&result, "ui", "scan_angleSlider"),
        &at(Value::Float(std::f64::consts::FRAC_PI_2))
    );
    assert_eq!(
        reading(&result, account, "dscan_angle"),
        Reading::Stored(Content::Number(90.0))
    );
    let result = edited(&root, account, &set(360.0));
    assert_eq!(
        held(&result, "ui", "scan_angleSlider"),
        &at(Value::Float(std::f64::consts::TAU))
    );
    for refused in [4.9, 360.1] {
        assert_eq!(
            apply(&root, account, &set(refused), &now()),
            Err(EditError::NotAllowed("Directional scan angle"))
        );
    }
}

#[test]
fn the_scan_preset_is_one_of_the_presets_the_file_holds() {
    use clone_bay_core::edit::{options, preset_names};
    let account = FileKind::Account;
    let root = scanner(Value::Int(1), None);
    assert_eq!(
        preset_names(&root),
        ["DefaultPreset_1", "Travel", "<b>Combat</b>"]
    );
    assert_eq!(options(&root, setting(account, "dscan_preset")).len(), 3);
    assert!(options(&root, setting(account, "dscan_range")).is_empty());
    assert!(preset_names(&dict(&[])).is_empty());
    assert_eq!(
        reading(&root, account, "dscan_preset"),
        Reading::Stored(Content::Word("Travel".to_owned()))
    );

    // Stored as the preset's own name, in the kind of string it is.
    let chosen = |name: &str| [change("dscan_preset", Content::Word(name.to_owned()))];
    let result = edited(&root, account, &chosen("DefaultPreset_1"));
    assert_eq!(
        held(&result, "ui", "scanner_presetInUse"),
        &at(b("DefaultPreset_1"))
    );
    let result = edited(&root, account, &chosen("<b>Combat</b>"));
    assert_eq!(
        held(&result, "ui", "scanner_presetInUse"),
        &at(Value::Str("<b>Combat</b>".into()))
    );
    // A name the file has no preset for, or something that isn't a name.
    assert_eq!(
        apply(&root, account, &chosen("Combat"), &now()),
        Err(EditError::NotAllowed("Directional scan overview preset"))
    );
    assert_eq!(
        apply(
            &root,
            account,
            &[change("dscan_preset", Content::Whole(1))],
            &now()
        ),
        Err(EditError::NotAllowed("Directional scan overview preset"))
    );
    assert_eq!(
        apply(
            &dict(&[(b("ui"), dict(&[]))]),
            account,
            &chosen("Travel"),
            &now()
        ),
        Err(EditError::NotAllowed("Directional scan overview preset"))
    );
}

fn scan_filters(root: &Value) -> Vec<clone_bay_core::edit::ScanFilter> {
    match reading(root, FileKind::Account, "probe_scan_filters") {
        Reading::Stored(Content::Filters(filters)) => filters,
        other => panic!("filters read as {other:?}"),
    }
}

fn named(id: i64, name: &str, groups: &[i64]) -> clone_bay_core::edit::ScanFilter {
    clone_bay_core::edit::ScanFilter {
        id,
        name: name.to_owned(),
        groups: groups.to_vec(),
    }
}

const FILTERS_KEY: &str = "probescanning.resultFilter.filters";
const ACTIVE_KEY: &str = "probescanning.resultFilter.activeFilterSet";

fn group_list(ids: &[i64]) -> Value {
    Value::List(ids.iter().map(|id| Value::Int(*id)).collect())
}

fn save_filters(root: &Value, wanted: Vec<clone_bay_core::edit::ScanFilter>) -> Value {
    edited(
        root,
        FileKind::Account,
        &[change("probe_scan_filters", Content::Filters(wanted))],
    )
}

fn filters_result(
    root: &Value,
    wanted: Vec<clone_bay_core::edit::ScanFilter>,
) -> Result<Value, EditError> {
    apply(
        root,
        FileKind::Account,
        &[change("probe_scan_filters", Content::Filters(wanted))],
        &now(),
    )
}

#[test]
fn scan_filters_can_be_renamed_and_removed() {
    let root = scanner(Value::Int(1), Some(set_of(&[16, -5, 12])));
    let all = scan_filters(&root);
    assert_eq!(
        all,
        [
            named(12, "boosher", &[1534]),
            named(13, "dictor", &[541]),
            named(16, "ships", &[25, 26, 27]),
        ]
    );

    // Renaming: the name changes, what it shows does not. A name stored as
    // a byte string stays one while it can.
    let mut renamed = all.clone();
    renamed[1].name = "  hic and dictor ".into();
    renamed[2].name = "sh\u{ef}ps".into();
    let result = save_filters(&root, renamed);
    assert_eq!(
        held(&result, "ui", FILTERS_KEY),
        &at(dict(&[
            (
                Value::Int(12),
                Value::Tuple(vec![Value::Str("boosher".into()), group_list(&[1534])])
            ),
            (
                Value::Int(13),
                Value::Tuple(vec![b("hic and dictor"), group_list(&[541])])
            ),
            (
                Value::Int(16),
                Value::Tuple(vec![
                    Value::Str("sh\u{ef}ps".into()),
                    group_list(&[25, 26, 27])
                ])
            ),
        ]))
    );
    // Nothing was removed, so the filters switched on are left alone.
    assert_eq!(
        held(&result, "ui", ACTIVE_KEY),
        held(&root, "ui", ACTIVE_KEY)
    );
    // A byte-string name given a letter it can't hold becomes text.
    let mut unicode = all.clone();
    unicode[1].name = "d\u{ef}ctor".into();
    assert_eq!(
        held(&save_filters(&root, unicode), "ui", FILTERS_KEY),
        &at(dict(&[
            (
                Value::Int(12),
                Value::Tuple(vec![Value::Str("boosher".into()), group_list(&[1534])])
            ),
            (
                Value::Int(13),
                Value::Tuple(vec![Value::Str("d\u{ef}ctor".into()), group_list(&[541])])
            ),
            (
                Value::Int(16),
                Value::Tuple(vec![Value::Str("ships".into()), group_list(&[25, 26, 27])])
            ),
        ]))
    );

    // Removing: the filter goes, and so does its place among those
    // switched on. The game's own filters (negative) stay.
    let result = save_filters(&root, vec![all[1].clone()]);
    assert_eq!(scan_filters(&result), [all[1].clone()]);
    assert_eq!(held(&result, "ui", ACTIVE_KEY), &at(set_of(&[-5])));
    // Removing one that isn't switched on leaves the set as it was.
    let result = save_filters(&root, vec![all[0].clone(), all[2].clone()]);
    assert_eq!(
        held(&result, "ui", ACTIVE_KEY),
        held(&root, "ui", ACTIVE_KEY)
    );
    let result = save_filters(&root, vec![]);
    assert_eq!(held(&result, "ui", FILTERS_KEY), &at(dict(&[])));
    assert_eq!(held(&result, "ui", ACTIVE_KEY), &at(set_of(&[-5])));
}

#[test]
fn what_a_scan_filter_shows_can_be_changed() {
    let root = scanner(Value::Int(1), Some(set_of(&[16])));
    let all = scan_filters(&root);

    // Frigates (25) out, interdictors (541) and mobile tractor units (1250)
    // in. The other filters are written back exactly as they were.
    let mut changed = all.clone();
    changed[2].groups = vec![26, 27, 541, 1250];
    let result = save_filters(&root, changed.clone());
    assert_eq!(scan_filters(&result), changed);
    let pairs = |root: &Value| match held(root, "ui", FILTERS_KEY) {
        Value::Tuple(pair) => match &pair[1] {
            Value::Dict(pairs) => pairs.clone(),
            other => panic!("filters are {other:?}"),
        },
        other => panic!("filters are {other:?}"),
    };
    assert_eq!(pairs(&result)[..2], pairs(&root)[..2]);
    assert_eq!(
        pairs(&result)[2],
        (
            Value::Int(16),
            Value::Tuple(vec![
                Value::Str("ships".into()),
                group_list(&[26, 27, 541, 1250])
            ])
        )
    );
    // A change to what a filter shows doesn't switch it on or off.
    assert_eq!(
        held(&result, "ui", ACTIVE_KEY),
        held(&root, "ui", ACTIVE_KEY)
    );

    // A kind the app doesn't offer may stay where the file already has it
    // (381 is a group the game hides; 999999 is unknown), but can't be
    // added to another filter.
    let odd = |groups: &[i64]| {
        let filter = Value::Tuple(vec![b("odd"), group_list(groups)]);
        dict(&[(
            b("ui"),
            dict(&[(
                b(FILTERS_KEY),
                stamped(
                    1,
                    dict(&[
                        (Value::Int(3), filter),
                        (
                            Value::Int(4),
                            Value::Tuple(vec![b("plain"), group_list(&[25])]),
                        ),
                    ]),
                ),
            )]),
        )])
    };
    let root = odd(&[381, 999_999, 25]);
    let all = scan_filters(&root);
    let mut kept = all.clone();
    kept[0].groups = vec![999_999, 381, 26];
    assert_eq!(scan_filters(&save_filters(&root, kept.clone())), kept);
    for unknown in [381, 999_999, 18, 0, -25] {
        let mut added = all.clone();
        added[1].groups = vec![25, unknown];
        assert_eq!(
            filters_result(&root, added),
            Err(EditError::NotAllowed("Probe scan result filters"))
        );
    }
    // The same kind twice, and no kinds at all.
    let mut twice = all.clone();
    twice[1].groups = vec![25, 26, 25];
    assert_eq!(
        filters_result(&root, twice),
        Err(EditError::NotAllowed("Probe scan result filters"))
    );
    let mut none = all.clone();
    none[1].groups = vec![];
    assert_eq!(
        filters_result(&root, none),
        Err(EditError::InvalidFilter(
            "Filter 'plain' needs at least one kind of result ticked".into()
        ))
    );
}

#[test]
fn scan_filters_can_be_added_after_the_ones_the_file_holds() {
    let root = scanner(Value::Int(1), Some(set_of(&[16])));
    let all = scan_filters(&root);

    let mut wanted = all.clone();
    wanted.push(named(17, " haulers ", &[28, 380]));
    wanted.push(named(40, "caps", &[485]));
    let result = save_filters(&root, wanted);
    let saved = scan_filters(&result);
    assert_eq!(saved[..3], all[..]);
    assert_eq!(saved[3], named(17, "haulers", &[28, 380]));
    assert_eq!(saved[4], named(40, "caps", &[485]));
    // A new filter isn't switched on; that is done in the game.
    assert_eq!(
        held(&result, "ui", ACTIVE_KEY),
        held(&root, "ui", ACTIVE_KEY)
    );
    let Value::Tuple(pair) = held(&result, "ui", FILTERS_KEY) else {
        panic!("not a pair");
    };
    let Value::Dict(pairs) = &pair[1] else {
        panic!("not a dict");
    };
    assert_eq!(
        pairs[3],
        (
            Value::Int(17),
            Value::Tuple(vec![Value::Str("haulers".into()), group_list(&[28, 380])])
        )
    );

    // Adding while removing: the new one still takes an ID above every ID
    // the file holds, so a removed filter's ID is never handed out again.
    let result = save_filters(&root, vec![all[0].clone(), named(17, "new", &[25])]);
    assert_eq!(
        scan_filters(&result),
        [all[0].clone(), named(17, "new", &[25])]
    );

    // A file that stores no filters can be given its first.
    let bare = dict(&[(b("ui"), dict(&[]))]);
    let result = save_filters(&bare, vec![named(1, "first", &[25])]);
    assert_eq!(scan_filters(&result), [named(1, "first", &[25])]);

    // Refused: an ID the file already uses or has passed, IDs out of
    // order, a new filter ahead of a stored one, and a stored one twice or
    // out of order.
    let refused = |wanted: Vec<clone_bay_core::edit::ScanFilter>| {
        assert_eq!(
            filters_result(&root, wanted),
            Err(EditError::NotAllowed("Probe scan result filters"))
        );
    };
    for id in [16, 14, 1, 0, -3] {
        if all.iter().any(|filter| filter.id == id) {
            continue;
        }
        refused(vec![all[0].clone(), named(id, "new", &[25])]);
    }
    refused(vec![named(18, "b", &[25]), named(17, "a", &[25])]);
    refused(vec![named(17, "a", &[25]), named(17, "b", &[25])]);
    refused(vec![named(17, "new", &[25]), all[0].clone()]);
    refused(vec![all[0].clone(), all[0].clone()]);
    refused(vec![all[2].clone(), all[0].clone()]);
    refused((1..=201).map(|id| named(100 + id, "many", &[25])).collect());
    assert_eq!(
        filters_result(&bare, vec![named(0, "zero", &[25])]),
        Err(EditError::NotAllowed("Probe scan result filters"))
    );
}

#[test]
fn a_filter_that_cannot_be_saved_says_which_and_why() {
    let root = scanner(Value::Int(1), None);
    let all = scan_filters(&root);
    let problem = |name: &str| {
        let mut changed = all.clone();
        changed[0].name = name.to_owned();
        match filters_result(&root, changed) {
            Err(EditError::InvalidFilter(problem)) => problem,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(problem(""), "Filter '(unnamed)' needs a name");
    assert_eq!(problem("   "), "Filter '(unnamed)' needs a name");
    assert_eq!(
        problem("two\nlines"),
        "Filter 'two\nlines' has a name that is too long or has characters that can't be used"
    );
    assert!(problem(&"x".repeat(101)).contains("too long"));
    let mut longest = all.clone();
    longest[0].name = "x".repeat(100);
    assert_eq!(
        scan_filters(&save_filters(&root, longest))[0].name.len(),
        100
    );
    assert_eq!(
        EditError::InvalidFilter("Filter 'a' needs a name".into()).to_string(),
        "Filter 'a' needs a name. Nothing was changed"
    );
}

#[test]
fn as_many_filters_as_are_allowed_can_be_saved_and_no_more() {
    let root = scanner(Value::Int(1), None);
    let mut wanted = scan_filters(&root);
    let mut id = 100;
    while wanted.len() < clone_bay_core::edit::MAX_FILTERS {
        wanted.push(named(id, &format!("new {id}"), &[25]));
        id += 1;
    }
    assert!(filters_result(&root, wanted.clone()).is_ok());
    wanted.push(named(id, "one too many", &[25]));
    assert_eq!(
        filters_result(&root, wanted),
        Err(EditError::NotAllowed("Probe scan result filters"))
    );
}

#[test]
fn scan_filters_in_an_unexpected_form_are_left_alone() {
    let account = FileKind::Account;
    let with = |filters: Value| dict(&[(b("ui"), dict(&[(b(FILTERS_KEY), stamped(1, filters))]))]);
    let good = Value::Tuple(vec![b("a"), Value::List(vec![])]);
    for odd in [
        Value::Int(1),
        dict(&[(b("12"), good.clone())]),
        dict(&[(Value::Int(1), Value::Int(1))]),
        dict(&[(Value::Int(1), Value::Tuple(vec![b("a")]))]),
        dict(&[(
            Value::Int(1),
            Value::Tuple(vec![Value::Int(1), Value::List(vec![])]),
        )]),
        dict(&[(Value::Int(1), Value::Tuple(vec![b("a"), Value::Int(1)]))]),
        // A kind of result that isn't a number.
        dict(&[(
            Value::Int(1),
            Value::Tuple(vec![b("a"), Value::List(vec![b("25")])]),
        )]),
        dict(&[(Value::Int(1), good.clone()), (Value::Int(1), good.clone())]),
    ] {
        let root = with(odd);
        assert_eq!(
            reading(&root, account, "probe_scan_filters"),
            Reading::Unreadable
        );
        assert_eq!(
            filters_result(&root, vec![named(50, "new", &[25])]),
            Err(EditError::CannotChange("Probe scan result filters"))
        );
    }
    let bare = dict(&[(b("ui"), dict(&[]))]);
    assert_eq!(
        reading(&bare, account, "probe_scan_filters"),
        Reading::NotStored
    );

    // A filter can't be removed when the set of filters switched on isn't
    // one this app can change to match. Renaming and adding need nothing
    // from the set, so they still go through.
    let mut strange = set_of(&[12]);
    if let Value::Reduce(reduce) = &mut strange {
        reduce.args = Value::Tuple(vec![Value::List(vec![b("12")])]);
    }
    // Nor is a set carrying anything beyond its members.
    let with_extra = |change: fn(&mut clone_bay_marshal::Reduce)| {
        let mut set = set_of(&[12]);
        if let Value::Reduce(reduce) = &mut set {
            change(reduce);
        }
        set
    };
    let odd_sets = [
        Value::Int(1),
        strange,
        with_extra(|reduce| reduce.state = Some(Value::None)),
        with_extra(|reduce| reduce.list_items = vec![Value::Int(1)]),
        with_extra(|reduce| reduce.dict_items = vec![(Value::Int(1), Value::Int(1))]),
    ];
    for odd_set in odd_sets {
        let root = scanner(Value::Int(1), Some(odd_set));
        let all = scan_filters(&root);
        assert_eq!(
            filters_result(&root, vec![all[0].clone()]),
            Err(EditError::CannotChange("Probe scan result filters"))
        );
        let mut renamed = all.clone();
        renamed[0].name = "other".into();
        renamed.push(named(17, "new", &[25]));
        assert!(filters_result(&root, renamed).is_ok());
    }
    // An empty set, or no set at all, is fine.
    for active in [Some(set_of(&[])), None] {
        let root = scanner(Value::Int(1), active);
        let all = scan_filters(&root);
        assert!(filters_result(&root, vec![all[0].clone()]).is_ok());
    }
}
