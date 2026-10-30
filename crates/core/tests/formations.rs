//! Probe formations: reading, changing and saving them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use clone_bay_core::edit::EditError;
use clone_bay_core::formations::{
    differs, open_file, read, read_file, save, Axis, Direction, Formation, FormationError,
    Formations, Preset, Probe, View, AU, BALANCE_TOLERANCE, KM, MAX_OFFSET, MAX_PROBES,
    MIN_DISTANCE, RANGE_OPTIONS,
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

fn probe_value(x: f64, y: f64, z: f64, range: f64) -> Value {
    let position = Value::Tuple(vec![Value::Float(x), Value::Float(y), Value::Float(z)]);
    Value::Tuple(vec![position, Value::Float(range)])
}

fn formation_value(name: Value, probes: &[Value]) -> Value {
    Value::Tuple(vec![name, Value::List(probes.to_vec())])
}

/// The stored formations of the sample file: the client's scratch entry, a
/// formation named with text and one named with bytes.
fn stored_formations() -> Value {
    dict(&[
        (
            Value::Int(-4),
            formation_value(
                b("tempFormation"),
                &[probe_value(1.0, -0.0, 3.0, 32.0 * AU)],
            ),
        ),
        (
            Value::Int(2),
            formation_value(
                Value::Str("skatter".into()),
                &[
                    probe_value(250.0 * KM, 0.0, 0.0, 0.25 * AU),
                    probe_value(-250.0 * KM, 0.0, 0.0, 0.25 * AU),
                ],
            ),
        ),
        (
            Value::Int(13),
            formation_value(b("long"), &[probe_value(0.0, 0.0, 0.0, 32.0 * AU)]),
        ),
    ])
}

fn account_with(formations: Option<Value>, selected: Option<Value>) -> Value {
    let mut ui = vec![(b("before"), stamped(1, Value::None))];
    if let Some(formations) = formations {
        ui.push((b("probescanning.customFormations"), stamped(1, formations)));
    }
    if let Some(selected) = selected {
        ui.push((b("probescanning.selectedFormationID"), stamped(1, selected)));
    }
    ui.push((b("after"), stamped(1, Value::None)));
    dict(&[(b("overview"), dict(&[])), (b("ui"), Value::Dict(ui))])
}

fn account() -> Value {
    account_with(Some(stored_formations()), Some(Value::Int(13)))
}

fn ui_value<'a>(root: &'a Value, key: &str) -> Option<&'a Value> {
    let Value::Dict(sections) = root else {
        panic!("not a dict");
    };
    let Value::Dict(ui) = &sections.iter().find(|(name, _)| *name == b("ui"))?.1 else {
        panic!("ui isn't a dict");
    };
    ui.iter()
        .find(|(name, _)| *name == b(key))
        .map(|(_, value)| value)
}

fn file_with(root: &Value) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core_user_1.dat");
    fs::write(&path, encode(root, false).unwrap()).unwrap();
    (dir, path)
}

fn now() -> Long {
    Long::from_i64(999)
}

fn save_to(
    path: &std::path::Path,
    wanted: &Formations,
) -> Result<std::path::PathBuf, FormationError> {
    save(
        path,
        Fingerprint::of(&fs::read(path).unwrap()),
        wanted,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &now(),
    )
}

#[test]
fn formations_are_read_with_names_probes_and_the_one_in_use() {
    let formations = read(&account()).unwrap();
    assert_eq!(formations.selected, Some(13));
    let list = formations.list();
    // The client's scratch entry isn't one of the user's formations.
    assert_eq!(
        list.iter()
            .map(|f| (f.id, f.name.as_str()))
            .collect::<Vec<_>>(),
        [(2, "skatter"), (13, "long")]
    );
    assert_eq!(
        list[0].probes,
        [
            Probe {
                x: 250.0 * KM,
                y: 0.0,
                z: 0.0,
                range: 0.25 * AU
            },
            Probe {
                x: -250.0 * KM,
                y: 0.0,
                z: 0.0,
                range: 0.25 * AU
            },
        ]
    );
    assert_eq!(formations.get(13).unwrap().name, "long");
    assert!(formations.get(-4).is_none());
    assert!(formations.get(99).is_none());
}

#[test]
fn a_file_with_no_formations_reads_as_empty() {
    let formations = read(&account_with(None, None)).unwrap();
    assert!(formations.list().is_empty());
    assert_eq!(formations.selected, None);
    assert_eq!(
        read(&dict(&[(b("overview"), dict(&[]))])).unwrap(),
        Formations::default()
    );
    let empty = read(&account_with(Some(dict(&[])), Some(Value::Int(0)))).unwrap();
    assert!(empty.list().is_empty());
    assert_eq!(empty.selected, Some(0));
}

#[test]
fn unchanged_formations_would_write_nothing() {
    let root = account();
    let formations = read(&root).unwrap();
    assert_eq!(differs(&root, &formations), Ok(false));
    let (_dir, path) = file_with(&root);
    assert_eq!(
        save_to(&path, &formations),
        Err(FormationError::Edit(EditError::NothingToSave))
    );
}

#[test]
fn anything_stored_in_an_unexpected_form_is_refused() {
    let good_probe = probe_value(0.0, 0.0, 0.0, AU);
    let odd_formations = [
        Value::List(vec![]),
        // A key that isn't a whole number.
        dict(&[(
            b("2"),
            formation_value(b("x"), std::slice::from_ref(&good_probe)),
        )]),
        // A name that isn't text, or isn't valid text.
        dict(&[(
            Value::Int(2),
            formation_value(Value::Int(5), std::slice::from_ref(&good_probe)),
        )]),
        dict(&[(
            Value::Int(2),
            formation_value(Value::Bytes(vec![0xFF]), std::slice::from_ref(&good_probe)),
        )]),
        // Probes that aren't ((x, y, z), range) of floats.
        dict(&[(Value::Int(2), formation_value(b("x"), &[Value::None]))]),
        dict(&[(
            Value::Int(2),
            formation_value(b("x"), &[probe_value(0.0, 0.0, 0.0, AU), Value::Int(1)]),
        )]),
        dict(&[(
            Value::Int(2),
            formation_value(
                b("x"),
                &[Value::Tuple(vec![
                    Value::Tuple(vec![Value::Int(0), Value::Float(0.0), Value::Float(0.0)]),
                    Value::Float(AU),
                ])],
            ),
        )]),
        dict(&[(
            Value::Int(2),
            formation_value(
                b("x"),
                &[Value::Tuple(vec![
                    Value::Tuple(vec![Value::Float(0.0), Value::Float(0.0)]),
                    Value::Float(AU),
                ])],
            ),
        )]),
        dict(&[(
            Value::Int(2),
            Value::Tuple(vec![b("x"), Value::Tuple(vec![])]),
        )]),
        dict(&[(Value::Int(2), Value::Tuple(vec![b("x")]))]),
        dict(&[(Value::Int(2), Value::None)]),
    ];
    for odd in odd_formations {
        let root = account_with(Some(odd.clone()), None);
        assert_eq!(read(&root), Err(FormationError::Unreadable), "{odd:?}");
    }
    // The choice of formation isn't a whole number.
    let root = account_with(Some(stored_formations()), Some(b("13")));
    assert_eq!(read(&root), Err(FormationError::Unreadable));
    // Not a (timestamp, value) pair, the key twice, or 'ui' isn't a dict.
    let Value::Dict(mut sections) = account() else {
        unreachable!()
    };
    let Value::Dict(ui) = &mut sections[1].1 else {
        unreachable!()
    };
    ui[1].1 = stored_formations();
    assert_eq!(
        read(&Value::Dict(sections.clone())),
        Err(FormationError::Unreadable)
    );
    let Value::Dict(ui) = &mut sections[1].1 else {
        unreachable!()
    };
    ui[1].1 = stamped(1, stored_formations());
    ui.push((b("probescanning.customFormations"), stamped(1, dict(&[]))));
    assert_eq!(
        read(&Value::Dict(sections)),
        Err(FormationError::Unreadable)
    );
    assert_eq!(
        read(&dict(&[(b("ui"), Value::None)])),
        Err(FormationError::Unreadable)
    );
    assert_eq!(
        read(&dict(&[(b("ui"), dict(&[])), (b("ui"), dict(&[]))])),
        Err(FormationError::Unreadable)
    );
    assert_eq!(read(&Value::None), Err(FormationError::Unreadable));
}

#[test]
fn the_clients_scratch_entry_is_kept_exactly_even_if_it_looks_odd() {
    let odd_scratch = dict(&[
        (
            Value::Int(-4),
            Value::Str("whatever the client put here".into()),
        ),
        (
            Value::Int(2),
            formation_value(b("x"), &[probe_value(0.0, 0.0, 0.0, AU)]),
        ),
    ]);
    let root = account_with(Some(odd_scratch), Some(Value::Int(2)));
    let mut formations = read(&root).unwrap();
    assert_eq!(formations.list().len(), 1);
    formations.get_mut(2).unwrap().name = "renamed".into();

    let (_dir, path) = file_with(&root);
    save_to(&path, &formations).unwrap();
    let written = decode(&fs::read(&path).unwrap()).unwrap().value;
    let Value::Tuple(pair) = ui_value(&written, "probescanning.customFormations").unwrap() else {
        panic!("not a pair");
    };
    let Value::Dict(entries) = &pair[1] else {
        panic!("not a dict");
    };
    assert_eq!(
        entries[0],
        (
            Value::Int(-4),
            Value::Str("whatever the client put here".into())
        )
    );
    // The edited name is still stored as bytes, as it was.
    assert_eq!(
        entries[1].1,
        formation_value(b("renamed"), &[probe_value(0.0, 0.0, 0.0, AU)])
    );
}

#[test]
fn a_change_is_saved_with_a_backup_and_touches_nothing_else() {
    let root = account();
    let (dir, path) = file_with(&root);
    let before = fs::read(&path).unwrap();
    let mut formations = read_file(&path).unwrap();
    let id = formations.add("New one", Preset::Pinpoint.probes());
    assert_eq!(id, 14);
    assert_eq!(differs(&root, &formations), Ok(true));

    let backup = save_to(&path, &formations).unwrap();
    assert_eq!(backup, dir.path().join("clone-bay-backups").join("T"));
    assert_eq!(fs::read(backup.join("core_user_1.dat")).unwrap(), before);

    let written = decode(&fs::read(&path).unwrap()).unwrap().value;
    assert_eq!(read(&written).unwrap(), formations);
    // The formations got a new timestamp; the untouched choice kept its own.
    let Value::Tuple(pair) = ui_value(&written, "probescanning.customFormations").unwrap() else {
        panic!("not a pair");
    };
    assert_eq!(pair[0], Value::Long(now()));
    assert_eq!(
        ui_value(&written, "probescanning.selectedFormationID"),
        Some(&stamped(1, Value::Int(13)))
    );
    // Everything around them is as it was, in the same order.
    assert_eq!(ui_value(&written, "before"), Some(&stamped(1, Value::None)));
    assert_eq!(ui_value(&written, "after"), Some(&stamped(1, Value::None)));
    let Value::Dict(sections) = &written else {
        unreachable!()
    };
    assert_eq!(sections[0], (b("overview"), dict(&[])));
    let Value::Dict(ui) = &sections[1].1 else {
        unreachable!()
    };
    assert_eq!(ui.len(), 4);
}

#[test]
fn removing_the_formation_in_use_moves_the_choice_to_the_first_one_left() {
    let root = account();
    let (_dir, path) = file_with(&root);
    let mut formations = read(&root).unwrap();
    formations.remove(13);
    assert_eq!(formations.selected_after_save(), Some(2));
    save_to(&path, &formations).unwrap();
    let written = decode(&fs::read(&path).unwrap()).unwrap().value;
    assert_eq!(
        ui_value(&written, "probescanning.selectedFormationID"),
        Some(&stamped(999, Value::Int(2)))
    );

    // With none left the choice becomes 0, as EVE Wrench does it.
    let mut formations = read(&written).unwrap();
    formations.remove(2);
    assert_eq!(formations.selected_after_save(), Some(0));
    save_to(&path, &formations).unwrap();
    let written = decode(&fs::read(&path).unwrap()).unwrap().value;
    assert!(read(&written).unwrap().list().is_empty());
    assert_eq!(read(&written).unwrap().selected, Some(0));
    // Removing one that isn't in use leaves the choice alone.
    let mut formations = read(&root).unwrap();
    formations.remove(2);
    assert_eq!(formations.selected_after_save(), Some(13));
    formations.remove(99);
    assert_eq!(formations.list().len(), 1);
}

#[test]
fn a_file_that_stores_no_choice_is_not_given_one() {
    let root = account_with(Some(stored_formations()), None);
    let (_dir, path) = file_with(&root);
    let mut formations = read(&root).unwrap();
    assert_eq!(formations.selected_after_save(), None);
    formations.remove(13);
    save_to(&path, &formations).unwrap();
    let written = decode(&fs::read(&path).unwrap()).unwrap().value;
    assert_eq!(
        ui_value(&written, "probescanning.selectedFormationID"),
        None
    );
}

#[test]
fn formations_can_be_added_to_a_file_that_has_none() {
    let root = account_with(None, None);
    let (_dir, path) = file_with(&root);
    let mut formations = read(&root).unwrap();
    assert_eq!(formations.next_id(), 1);
    assert_eq!(formations.add("First", Preset::Spread.probes()), 1);
    assert_eq!(formations.add("Second", Preset::Pinpoint.probes()), 2);
    save_to(&path, &formations).unwrap();
    let written = decode(&fs::read(&path).unwrap()).unwrap().value;
    let reread = read(&written).unwrap();
    assert_eq!(
        reread
            .list()
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        ["First", "Second"]
    );
    // New formations are named with text.
    let Value::Tuple(pair) = ui_value(&written, "probescanning.customFormations").unwrap() else {
        panic!("not a pair");
    };
    let Value::Dict(entries) = &pair[1] else {
        panic!("not a dict")
    };
    let Value::Tuple(first) = &entries[0].1 else {
        panic!("not a tuple")
    };
    assert_eq!(first[0], Value::Str("First".into()));
}

fn invalid(formations: &Formations) -> String {
    match formations.validate() {
        Err(FormationError::Invalid(message)) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn formations_the_game_could_not_use_are_refused() {
    let base = read(&account()).unwrap();
    assert_eq!(base.validate(), Ok(()));
    let with = |change: &dyn Fn(&mut Formation)| {
        let mut formations = base.clone();
        change(formations.get_mut(2).unwrap());
        invalid(&formations)
    };
    assert_eq!(
        with(&|f| f.name = "  ".into()),
        "Formation '(unnamed)': it needs a name"
    );
    assert_eq!(
        with(&|f| f.name = "x".repeat(101)),
        format!(
            "Formation '{}': its name is too long or has characters that can't be used",
            "x".repeat(101)
        )
    );
    assert!(with(&|f| f.name = "a\tb".into()).ends_with("characters that can't be used"));
    assert_eq!(
        with(&|f| f.probes.clear()),
        "Formation 'skatter': it must have between 1 and 8 probes"
    );
    assert!(with(&|f| f.probes = vec![f.probes[0]; 9]).ends_with("between 1 and 8 probes"));
    assert!(with(&|f| f.probes[0].x = f64::NAN).ends_with("a probe is placed too far away"));
    assert!(with(&|f| f.probes[0].y = f64::INFINITY).ends_with("a probe is placed too far away"));
    assert!(with(&|f| f.probes[0].z = -100.1 * AU).ends_with("a probe is placed too far away"));
    assert!(
        with(&|f| f.probes[1].range = 3.0 * AU).ends_with("a scan range the game doesn't offer")
    );
    assert!(with(&|f| f.probes[1].range = 0.0).ends_with("a scan range the game doesn't offer"));
    assert!(
        with(&|f| f.probes[1].range = 64.0 * AU).ends_with("a scan range the game doesn't offer")
    );
    assert_eq!(
        with(&|f| f.id = 13),
        "Formation 'long': its ID is already in use"
    );
    assert!(with(&|f| f.id = -1).ends_with("its ID is already in use"));

    // The limits themselves are fine.
    let mut edge = base.clone();
    let formation = edge.get_mut(2).unwrap();
    formation.name = "x".repeat(100);
    formation.probes = vec![
        Probe {
            x: 100.0 * AU,
            y: -100.0 * AU,
            z: 0.0,
            range: 32.0 * AU
        };
        MAX_PROBES
    ];
    assert_eq!(edge.validate(), Ok(()));
    for option in RANGE_OPTIONS {
        edge.get_mut(2).unwrap().set_range(option);
        assert_eq!(edge.validate(), Ok(()), "{option} AU");
    }
}

#[test]
fn an_invalid_formation_is_never_saved() {
    let root = account();
    let (dir, path) = file_with(&root);
    let before = fs::read(&path).unwrap();
    let mut formations = read(&root).unwrap();
    formations.get_mut(2).unwrap().probes.clear();
    assert!(matches!(
        save_to(&path, &formations),
        Err(FormationError::Invalid(_))
    ));
    assert!(matches!(
        differs(&root, &formations),
        Err(FormationError::Invalid(_))
    ));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        1,
        "no backup folder was made"
    );
}

#[test]
fn nothing_is_saved_while_eve_is_or_might_be_running() {
    let root = account();
    let (_dir, path) = file_with(&root);
    let before = fs::read(&path).unwrap();
    let mut formations = read(&root).unwrap();
    formations.add("New", Preset::Spread.probes());
    let attempt = |eve, confirmed| {
        save(
            &path,
            Fingerprint::of(&fs::read(&path).unwrap()),
            &formations,
            eve,
            confirmed,
            &BackupSession::new("T"),
            &now(),
        )
    };
    assert_eq!(
        attempt(EveStatus::Running, true),
        Err(FormationError::Edit(EditError::EveRunning))
    );
    assert_eq!(
        attempt(EveStatus::Unknown, false),
        Err(FormationError::Edit(EditError::EveUnknown))
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(attempt(EveStatus::Unknown, true).is_ok());
}

#[test]
fn formations_belong_to_account_files_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core_char_1.dat");
    fs::write(&path, encode(&account(), false).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    assert_eq!(read_file(&path), Err(FormationError::NotAnAccountFile));
    let mut formations = read(&account()).unwrap();
    formations.add("New", Preset::Spread.probes());
    assert_eq!(
        save_to(&path, &formations),
        Err(FormationError::NotAnAccountFile)
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(matches!(
        read_file(&dir.path().join("core_user_9.dat")),
        Err(FormationError::Edit(EditError::Unreadable { .. }))
    ));
}

#[test]
fn a_file_whose_formations_cannot_be_read_is_never_overwritten() {
    let root = account_with(Some(Value::List(vec![])), None);
    let (_dir, path) = file_with(&root);
    let before = fs::read(&path).unwrap();
    let mut formations = Formations::default();
    formations.add("New", Preset::Spread.probes());
    assert_eq!(save_to(&path, &formations), Err(FormationError::Unreadable));
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn formations_are_not_saved_over_a_file_that_changed_after_it_was_opened() {
    // A formation saved in the game after the editor opened would be lost
    // if the editor's list replaced the file's.
    let (dir, path) = file_with(&account());
    let (mut formations, opened_as) = open_file(&path).unwrap();
    assert_eq!(formations, read(&account()).unwrap());
    assert_eq!(opened_as, Fingerprint::of(&fs::read(&path).unwrap()));
    // The game saved the same settings again, with a checksum this time.
    let game_bytes = encode(&account(), true).unwrap();
    fs::write(&path, &game_bytes).unwrap();

    formations.add("Made in the editor", Preset::Spread.probes());
    let result = save(
        &path,
        opened_as,
        &formations,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &now(),
    );
    assert_eq!(
        result,
        Err(FormationError::Edit(EditError::ChangedSinceOpened))
    );
    assert_eq!(fs::read(&path).unwrap(), game_bytes);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn the_centre_is_the_average_position() {
    let mut formation = Formation::new(1, "f", vec![]);
    assert_eq!(formation.centre(), (0.0, 0.0, 0.0));
    assert!(formation.is_balanced());
    formation.probes = vec![
        Probe::at_km(100.0, 0.0, -30.0, 1.0),
        Probe::at_km(300.0, 40.0, -30.0, 1.0),
    ];
    let (x, y, z) = formation.centre();
    assert!(close(x, 200.0 * KM) && close(y, 20.0 * KM) && close(z, -30.0 * KM));
    let expected = (200.0_f64.powi(2) + 20.0_f64.powi(2) + 30.0_f64.powi(2)).sqrt() * KM;
    assert!(close(formation.launch_shift(), expected));
    assert!(!formation.is_balanced());
}

#[test]
fn balance_has_a_tolerance_of_half_a_kilometre() {
    assert_eq!(BALANCE_TOLERANCE, 500.0);
    let at = |metres: f64| {
        Formation::new(
            1,
            "f",
            vec![Probe {
                x: metres,
                y: 0.0,
                z: 0.0,
                range: AU,
            }],
        )
    };
    assert!(at(499.0).is_balanced());
    assert!(!at(500.0).is_balanced());
    assert!(!at(-501.0).is_balanced());
}

#[test]
fn balancing_adds_a_counterweight_when_there_is_room() {
    let mut formation = Formation::new(
        1,
        "f",
        vec![
            Probe::at_km(100.0, 0.0, 0.0, 4.0),
            Probe::at_km(300.0, 50.0, -20.0, 8.0),
        ],
    );
    formation.balance();
    assert_eq!(formation.probes.len(), 3);
    let weight = formation.probes[2];
    assert!(
        close(weight.x, -400.0 * KM) && close(weight.y, -50.0 * KM) && close(weight.z, 20.0 * KM)
    );
    assert_eq!(weight.range, 8.0 * AU, "it takes the last probe's range");
    assert!(formation.is_balanced());
    // Already balanced: nothing is added.
    formation.balance();
    assert_eq!(formation.probes.len(), 3);
    // Nothing to balance.
    let mut empty = Formation::new(1, "f", vec![]);
    empty.balance();
    assert!(empty.probes.is_empty());
}

#[test]
fn balancing_a_full_formation_repurposes_the_last_probe() {
    let mut probes = vec![Probe::at_km(100.0, 0.0, 0.0, 2.0); 7];
    probes.push(Probe::at_km(900.0, 900.0, 900.0, 16.0));
    let mut formation = Formation::new(1, "f", probes);
    formation.balance();
    assert_eq!(formation.probes.len(), MAX_PROBES);
    let weight = formation.probes[7];
    assert!(close(weight.x, -700.0 * KM) && close(weight.y, 0.0) && close(weight.z, 0.0));
    assert_eq!(weight.range, 16.0 * AU, "it keeps its own range");
    assert!(formation.is_balanced());
    assert!(formation.probes[..7]
        .iter()
        .all(|probe| *probe == Probe::at_km(100.0, 0.0, 0.0, 2.0)));
}

#[test]
fn scaling_multiplies_positions_and_leaves_ranges_alone() {
    let mut formation = Formation::new(1, "f", vec![Probe::at_km(100.0, -50.0, 20.0, 4.0)]);
    formation.scale(2.0);
    assert_eq!(formation.probes[0], Probe::at_km(200.0, -100.0, 40.0, 4.0));
    formation.scale(0.5);
    assert_eq!(formation.probes[0], Probe::at_km(100.0, -50.0, 20.0, 4.0));
    for ignored in [0.0, -2.0, f64::NAN, f64::INFINITY] {
        formation.scale(ignored);
        assert_eq!(
            formation.probes[0],
            Probe::at_km(100.0, -50.0, 20.0, 4.0),
            "{ignored}"
        );
    }
    formation.set_range(0.25);
    assert_eq!(formation.probes[0].range, 0.25 * AU);
    assert_eq!(formation.probes[0].x, 100.0 * KM);
}

#[test]
fn every_preset_is_eight_balanced_probes_the_game_accepts() {
    let mut formations = Formations::default();
    for preset in Preset::ALL {
        let probes = preset.probes();
        assert_eq!(probes.len(), MAX_PROBES, "{}", preset.label());
        let formation = Formation::new(1, preset.label(), probes.clone());
        assert!(
            formation.is_balanced(),
            "{} drifts {} m",
            preset.label(),
            formation.launch_shift()
        );
        assert!(probes.iter().all(|probe| probe.range == 4.0 * AU));
        formations.add(preset.label(), probes);
    }
    assert_eq!(formations.validate(), Ok(()));
    let labels: std::collections::BTreeSet<&str> = Preset::ALL.iter().map(|p| p.label()).collect();
    assert_eq!(labels.len(), Preset::ALL.len());
    // Written with the blank preset and not yet run when committed.
    assert_eq!(Preset::ALL[0], Preset::Blank);
    assert_eq!(Preset::Blank.label(), "Blank");
    assert_eq!(
        Preset::Blank.probes(),
        vec![Probe::at_km(0.0, 0.0, 0.0, 4.0); 8]
    );

    assert_eq!(
        Preset::Pinpoint.probes()[0],
        Probe::at_km(500.0, 0.0, 0.0, 4.0)
    );
    assert_eq!(
        Preset::Pinpoint.probes()[7],
        Probe::at_km(0.0, -1000.0, 0.0, 4.0)
    );
    assert_eq!(
        Preset::Spread.probes()[2],
        Probe::at_km(0.0, 0.0, 250.0, 4.0)
    );
    let line = Preset::Stack(Axis::ZMinus).probes();
    assert_eq!(line[0], Probe::at_km(0.0, 0.0, -200.0, 4.0));
    assert_eq!(line[6], Probe::at_km(0.0, 0.0, -1400.0, 4.0));
    assert_eq!(line[7], Probe::at_km(0.0, 0.0, 5600.0, 4.0));
    assert_eq!(
        Preset::Stack(Axis::XPlus).probes()[1],
        Probe::at_km(400.0, 0.0, 0.0, 4.0)
    );
    assert_eq!(
        Preset::Stack(Axis::XMinus).probes()[1],
        Probe::at_km(-400.0, 0.0, 0.0, 4.0)
    );
    assert_eq!(
        Preset::Stack(Axis::YPlus).probes()[1],
        Probe::at_km(0.0, 400.0, 0.0, 4.0)
    );
    assert_eq!(
        Preset::Stack(Axis::YMinus).probes()[1],
        Probe::at_km(0.0, -400.0, 0.0, 4.0)
    );
    assert_eq!(
        Preset::Stack(Axis::ZPlus).probes()[1],
        Probe::at_km(0.0, 0.0, 400.0, 4.0)
    );
}

#[test]
fn a_formation_is_found_by_its_own_id_and_no_other() {
    let mut formations = read(&account()).unwrap();
    assert_eq!(formations.get_mut(13).unwrap().name, "long");
    assert_eq!(formations.get_mut(2).unwrap().name, "skatter");
    assert!(formations.get_mut(99).is_none());
    assert!(
        formations.get_mut(-4).is_none(),
        "scratch entries can't be edited"
    );
}

#[test]
fn a_formation_numbered_zero_is_as_good_as_any_other() {
    // Files are read with IDs from 0 up, so 0 must be savable too.
    let stored = dict(&[(
        Value::Int(0),
        formation_value(b("zero"), &[probe_value(0.0, 0.0, 0.0, AU)]),
    )]);
    let formations = read(&account_with(Some(stored), None)).unwrap();
    assert_eq!(formations.list()[0].id, 0);
    assert_eq!(formations.validate(), Ok(()));
    assert_eq!(formations.next_id(), 1);
}

fn positions_km(formation: &Formation) -> Vec<(f64, f64, f64)> {
    formation
        .probes
        .iter()
        .map(|probe| (probe.x / KM, probe.y / KM, probe.z / KM))
        .collect()
}

/// One probe 100 km north, one 200 km west and one 300 km up. Positions are
/// stored as (west, up, north).
fn compass_formation() -> Formation {
    Formation::new(
        1,
        "f",
        vec![
            Probe::at_km(0.0, 0.0, 100.0, 1.0),
            Probe::at_km(200.0, 0.0, 0.0, 2.0),
            Probe::at_km(0.0, 300.0, 0.0, 4.0),
        ],
    )
}

#[test]
fn a_quarter_turn_seen_from_above_takes_north_to_east() {
    let mut formation = compass_formation();
    formation.turn(View::Above, 90.0);
    assert_eq!(
        positions_km(&formation),
        [
            // North went to east, which is minus west.
            (-100.0, 0.0, 0.0),
            // West went to north.
            (0.0, 0.0, 200.0),
            // Up is the axis of the turn, so it stayed.
            (0.0, 300.0, 0.0),
        ]
    );
    // Ranges and the order of the probes are untouched.
    let ranges: Vec<f64> = formation.probes.iter().map(|p| p.range / AU).collect();
    assert_eq!(ranges, [1.0, 2.0, 4.0]);
    // Exact, and with no "-0" left behind.
    for probe in &formation.probes {
        for coordinate in [probe.x, probe.y, probe.z] {
            assert!(coordinate != 0.0 || coordinate.is_sign_positive());
        }
    }
}

#[test]
fn a_quarter_turn_seen_from_the_south_takes_up_to_east() {
    let mut formation = compass_formation();
    formation.turn(View::South, 90.0);
    assert_eq!(
        positions_km(&formation),
        [
            // North is the axis of the turn.
            (0.0, 0.0, 100.0),
            // West went to up.
            (0.0, 200.0, 0.0),
            // Up went to east.
            (-300.0, 0.0, 0.0),
        ]
    );
}

#[test]
fn a_quarter_turn_seen_from_the_east_takes_up_to_north() {
    let mut formation = compass_formation();
    formation.turn(View::East, 90.0);
    assert_eq!(
        positions_km(&formation),
        [
            // North went to down.
            (0.0, -100.0, 0.0),
            // West is the axis of the turn.
            (200.0, 0.0, 0.0),
            // Up went to north.
            (0.0, 0.0, 300.0),
        ]
    );
}

#[test]
fn turning_back_and_all_the_way_round_changes_nothing() {
    for view in [View::Above, View::South, View::East] {
        let original = compass_formation();
        let mut formation = original.clone();
        formation.turn(view, 90.0);
        assert_ne!(formation, original, "{view:?}");
        formation.turn(view, -90.0);
        assert_eq!(formation, original, "{view:?}");
        formation.turn(view, 360.0);
        assert_eq!(formation, original, "{view:?}");
        formation.turn(view, -720.0);
        assert_eq!(formation, original, "{view:?}");
        formation.turn(view, 0.0);
        assert_eq!(formation, original, "{view:?}");
        // Half a turn is two quarter turns, and 270 is one the other way.
        let mut half = original.clone();
        half.turn(view, 180.0);
        let mut twice = original.clone();
        twice.turn(view, 90.0);
        twice.turn(view, 90.0);
        assert_eq!(half, twice, "{view:?}");
        let mut three = original.clone();
        three.turn(view, 270.0);
        let mut back = original.clone();
        back.turn(view, -90.0);
        assert_eq!(three, back, "{view:?}");
        for ignored in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            formation.turn(view, ignored);
            assert_eq!(formation, original, "{view:?}");
        }
    }
}

#[test]
fn other_angles_are_rounded_to_the_millimetre() {
    let mut formation = Formation::new(1, "f", vec![Probe::at_km(0.0, 0.0, 100.0, 1.0)]);
    // 30 degrees clockwise from north: half the distance east, and
    // 86.6025403... km north.
    formation.turn(View::Above, 30.0);
    let probe = formation.probes[0];
    assert_eq!(probe.x, -50_000.0);
    assert_eq!(probe.y, 0.0);
    assert_eq!(probe.z, 86_602.54);
    // The other way from north goes west.
    let mut formation = Formation::new(1, "f", vec![Probe::at_km(0.0, 0.0, 100.0, 1.0)]);
    formation.turn(View::Above, -30.0);
    assert_eq!(formation.probes[0].x, 50_000.0);
    // The distance from the middle is kept, to the millimetre.
    let mut formation = Formation::new(1, "f", vec![Probe::at_km(30.0, -40.0, 120.0, 1.0)]);
    for view in [View::Above, View::South, View::East] {
        formation.turn(view, 37.5);
        let probe = formation.probes[0];
        let distance = (probe.x * probe.x + probe.y * probe.y + probe.z * probe.z).sqrt();
        assert!((distance - 130_000.0).abs() < 0.01, "{view:?}: {distance}");
    }
}

#[test]
fn mirroring_swaps_the_two_ends_of_one_direction() {
    let original = Formation::new(
        1,
        "f",
        vec![
            Probe::at_km(1.0, 2.0, 3.0, 1.0),
            Probe::at_km(0.0, 0.0, 0.0, 2.0),
        ],
    );
    let mirrored = |direction: Direction| {
        let mut formation = original.clone();
        formation.mirror(direction);
        positions_km(&formation)
    };
    assert_eq!(
        mirrored(Direction::NorthSouth),
        [(1.0, 2.0, -3.0), (0.0, 0.0, 0.0)]
    );
    assert_eq!(
        mirrored(Direction::WestEast),
        [(-1.0, 2.0, 3.0), (0.0, 0.0, 0.0)]
    );
    assert_eq!(
        mirrored(Direction::UpDown),
        [(1.0, -2.0, 3.0), (0.0, 0.0, 0.0)]
    );
    // Twice is back where it started, zeros keep their sign, and ranges
    // are untouched.
    for direction in [
        Direction::NorthSouth,
        Direction::WestEast,
        Direction::UpDown,
    ] {
        let mut formation = original.clone();
        formation.mirror(direction);
        let zero = formation.probes[1];
        assert!(zero.x.is_sign_positive() && zero.y.is_sign_positive());
        assert!(zero.z.is_sign_positive());
        assert_eq!(formation.probes[1].range, 2.0 * AU);
        formation.mirror(direction);
        assert_eq!(formation, original);
    }
}

#[test]
fn recentring_moves_the_formation_onto_its_middle_without_changing_its_shape() {
    let mut formation = Formation::new(
        1,
        "f",
        vec![
            Probe::at_km(100.0, 0.0, 0.0, 1.0),
            Probe::at_km(300.0, 60.0, -90.0, 2.0),
        ],
    );
    assert!(!formation.is_balanced());
    formation.recentre();
    assert_eq!(
        positions_km(&formation),
        [(-100.0, -30.0, 45.0), (100.0, 30.0, -45.0)]
    );
    assert!(formation.is_balanced());
    assert_eq!(formation.launch_shift(), 0.0);
    assert_eq!(formation.probes.len(), 2);
    assert_eq!(formation.probes[1].range, 2.0 * AU);
    // Already centred, or empty: nothing happens.
    let again = formation.clone();
    formation.recentre();
    assert_eq!(formation, again);
    let mut empty = Formation::new(1, "f", vec![]);
    empty.recentre();
    assert!(empty.probes.is_empty());
}

#[test]
fn a_probe_knows_how_far_it_is_from_the_centre() {
    assert_eq!(Probe::at_km(0.0, 0.0, 0.0, 1.0).distance(), 0.0);
    assert_eq!(Probe::at_km(-300.0, 0.0, 0.0, 1.0).distance(), 300.0 * KM);
    assert_eq!(Probe::at_km(30.0, -40.0, 120.0, 1.0).distance(), 130.0 * KM);
}

#[test]
fn setting_a_distance_moves_a_probe_straight_in_or_out() {
    // Along one direction only.
    let mut probe = Probe::at_km(300.0, 0.0, 0.0, 2.0);
    assert!(probe.set_distance(250.0 * KM));
    assert_eq!(probe, Probe::at_km(250.0, 0.0, 0.0, 2.0));
    let mut probe = Probe::at_km(0.0, 0.0, -100.0, 2.0);
    assert!(probe.set_distance(700.0 * KM));
    assert_eq!(probe, Probe::at_km(0.0, 0.0, -700.0, 2.0));

    // At an angle: every part grows by the same factor, and the range and
    // the direction stay as they were.
    let mut probe = Probe::at_km(30.0, -40.0, 120.0, 4.0);
    assert!(probe.set_distance(260.0 * KM));
    assert_eq!(probe, Probe::at_km(60.0, -80.0, 240.0, 4.0));
    assert!(probe.set_distance(65.0 * KM));
    assert_eq!(probe, Probe::at_km(15.0, -20.0, 60.0, 4.0));

    // Positions come out rounded to the millimetre.
    let mut probe = Probe::at_km(100.0, 100.0, 100.0, 1.0);
    assert!(probe.set_distance(100.0 * KM));
    assert_eq!(probe.x, 57_735.027);
    assert_eq!((probe.y, probe.z), (probe.x, probe.x));
    assert!((probe.distance() - 100.0 * KM).abs() < 0.01);
}

#[test]
fn a_distance_that_cannot_be_set_changes_nothing() {
    // A probe at the centre has no direction to move in.
    let mut centre = Probe::at_km(0.0, 0.0, 0.0, 1.0);
    assert!(!centre.set_distance(100.0 * KM));
    assert_eq!(centre, Probe::at_km(0.0, 0.0, 0.0, 1.0));

    let original = Probe::at_km(30.0, -40.0, 120.0, 1.0);
    for refused in [
        0.0,
        -5.0,
        MIN_DISTANCE - 0.001,
        MAX_OFFSET + 1.0,
        f64::NAN,
        f64::INFINITY,
    ] {
        let mut probe = original;
        assert!(!probe.set_distance(refused), "{refused}");
        assert_eq!(probe, original, "{refused}");
    }
    // The limits themselves are allowed.
    let mut probe = Probe::at_km(0.0, 5.0, 0.0, 1.0);
    assert!(probe.set_distance(MIN_DISTANCE));
    assert_eq!(probe.y, 1.0);
    assert!(probe.set_distance(MAX_OFFSET));
    assert_eq!(probe.y, MAX_OFFSET);
}
