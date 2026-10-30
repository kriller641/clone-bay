//! End-to-end tests of copying settings between files in a temp folder.
//!
//! These cover the same ground as the old `reference/dev_tools/e2e.py`:
//! planning changes nothing, backups are exact, protected data stays put,
//! and every refusal leaves the folder untouched.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use clone_bay_core::copy::{
    execute_copy, plan_copy, plan_copy_scoped, source_chat_channels, Action, ExecuteError, Outcome,
    PlanError,
};
use clone_bay_core::discovery::FileKind;
use clone_bay_core::groups::{chat_channels, same_settings, Scope, Selection};
use clone_bay_core::platform::EveStatus;
use clone_bay_core::protect::{protected_parts, strip_protected, PROTECTED};
use clone_bay_core::safe_write::BackupSession;
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

fn layout(stamp: i64, ships: &[i64]) -> Value {
    let slots = Value::List((11..35).map(Value::Int).collect());
    let ships: Vec<(Value, Value)> = ships
        .iter()
        .map(|&ship| (Value::Long(Long::from_i64(ship)), slots.clone()))
        .collect();
    stamped(stamp, Value::Dict(ships))
}

fn modules(ids: &[i64]) -> Value {
    Value::Dict(
        ids.iter()
            .map(|&id| {
                (
                    Value::Long(Long::from_i64(id)),
                    stamped(9, Value::Int(1000)),
                )
            })
            .collect(),
    )
}

/// An account file. `ships` of `None` means no rack layout stored at all.
fn account(tag: i64, ships: Option<&[i64]>) -> Value {
    let mut ui = vec![(b("defaultwarpto"), stamped(tag, Value::Int(tag)))];
    if let Some(ships) = ships {
        ui.push((b("slotOrder"), layout(tag, ships)));
    }
    dict(&[
        (
            b("overview"),
            dict(&[(b("profile"), stamped(tag, Value::Int(tag)))]),
        ),
        (b("ui"), Value::Dict(ui)),
    ])
}

/// A character file. `repeat` of `None` means no module settings at all.
fn character(tag: i64, repeat: Option<&[i64]>) -> Value {
    let mut sections = vec![(
        b("windows"),
        dict(&[(b("open"), stamped(tag, Value::Int(tag)))]),
    )];
    if let Some(repeat) = repeat {
        sections.push((b("autorepeat"), modules(repeat)));
        sections.push((b("autoreload"), modules(&[])));
    }
    Value::Dict(sections)
}

struct Folder {
    dir: tempfile::TempDir,
}

impl Folder {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn put(&self, name: &str, value: &Value) -> PathBuf {
        self.put_bytes(name, &encode(value, false).unwrap())
    }

    fn put_bytes(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn value(&self, name: &str) -> Value {
        decode(&fs::read(self.path(name)).unwrap()).unwrap().value
    }

    /// Every file under the folder, by relative path, with its contents.
    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        fn walk(root: &Path, dir: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(root, &path, into);
                } else {
                    let name = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    into.insert(name, fs::read(&path).unwrap());
                }
            }
        }
        let mut files = BTreeMap::new();
        walk(self.dir.path(), self.dir.path(), &mut files);
        files
    }
}

fn set_read_only(path: &Path, read_only: bool) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(read_only);
    fs::set_permissions(path, permissions).unwrap();
}

fn quiet(_: usize) {}

/// The account folder used by most tests.
///
/// 111 is the source. 222 has its own layout. 333 has none. 444 is damaged.
/// 555 is read-only. 666 carries a checksum. There is also a character file
/// and an unrelated file, which must never be touched.
fn account_folder() -> (Folder, PathBuf, Vec<PathBuf>) {
    let folder = Folder::new();
    let source = folder.put("core_user_111.dat", &account(1, Some(&[10, 11])));
    let mut targets = vec![
        folder.put("core_user_222.dat", &account(2, Some(&[20, 21, 22]))),
        folder.put("core_user_333.dat", &account(3, None)),
    ];
    let good = encode(&account(4, Some(&[40])), false).unwrap();
    targets.push(folder.put_bytes("core_user_444.dat", &good[..good.len() / 2]));
    targets.push(folder.put("core_user_555.dat", &account(5, Some(&[50]))));
    set_read_only(&targets[3], true);
    let with_checksum = encode(&account(6, Some(&[60])), true).unwrap();
    targets.push(folder.put_bytes("core_user_666.dat", &with_checksum));
    folder.put("core_char_1.dat", &character(1, Some(&[1])));
    folder.put_bytes("notes.txt", b"unrelated");
    (folder, source, targets)
}

fn actions(plan: &clone_bay_core::copy::CopyPlan) -> Vec<(String, Action)> {
    plan.targets
        .iter()
        .map(|target| {
            let name = target
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            (name, target.action.clone())
        })
        .collect()
}

#[test]
fn planning_changes_nothing_and_says_what_would_happen() {
    let (folder, source, targets) = account_folder();
    let before = folder.snapshot();
    let plan = plan_copy(&source, &targets, &quiet).unwrap();
    assert_eq!(folder.snapshot(), before);

    let kept = |text: &str| Action::KeepOwn {
        kept: vec![text.to_owned()],
    };
    let skip = |reason: &str| Action::Skip {
        reason: reason.to_owned(),
    };
    assert_eq!(
        actions(&plan),
        [
            (
                "core_user_222.dat".to_owned(),
                kept("module rack layout for 3 ships")
            ),
            (
                "core_user_333.dat".to_owned(),
                Action::KeepOwn { kept: vec![] }
            ),
            (
                "core_user_444.dat".to_owned(),
                skip("can't decode it (unexpected end of data)")
            ),
            (
                "core_user_555.dat".to_owned(),
                skip("the file is read-only")
            ),
            (
                "core_user_666.dat".to_owned(),
                kept("module rack layout for 1 ship")
            ),
        ]
    );
    assert_eq!(plan.writable(), 3);
    set_read_only(&targets[3], false);
}

#[test]
fn progress_counts_every_target_examined() {
    let (_folder, source, targets) = account_folder();
    let seen = std::sync::Mutex::new(Vec::new());
    plan_copy(&source, &targets, &|done| seen.lock().unwrap().push(done)).unwrap();
    let mut seen = seen.into_inner().unwrap();
    seen.sort_unstable();
    assert_eq!(seen, [1, 2, 3, 4, 5]);
    set_read_only(&targets[3], false);
}

#[test]
fn many_targets_are_planned_in_the_order_given() {
    let folder = Folder::new();
    let source = folder.put("core_user_1.dat", &account(1, Some(&[10])));
    let targets: Vec<PathBuf> = (0..60_i64)
        .map(|n| {
            // Every third target is damaged.
            let name = format!("core_user_{}.dat", 100 + n);
            if n % 3 == 0 {
                folder.put_bytes(&name, b"damaged")
            } else {
                folder.put(&name, &account(n, Some(&[n])))
            }
        })
        .collect();
    let plan = plan_copy(&source, &targets, &quiet).unwrap();
    let paths: Vec<&PathBuf> = plan.targets.iter().map(|target| &target.path).collect();
    assert_eq!(paths, targets.iter().collect::<Vec<_>>());
    for (n, target) in plan.targets.iter().enumerate() {
        let skipped = matches!(target.action, Action::Skip { .. });
        assert_eq!(skipped, n % 3 == 0, "target {n}: {:?}", target.action);
    }
    assert_eq!(plan.writable(), 40);
    assert!(plan_copy(&source, &[], &quiet).unwrap().targets.is_empty());
}

#[test]
fn a_copy_writes_the_source_settings_and_each_target_keeps_its_own_layout() {
    let (folder, source, targets) = account_folder();
    let before = folder.snapshot();
    let plan = plan_copy(&source, &targets, &quiet).unwrap();
    let written = RefCell::new(Vec::new());
    let report = execute_copy(
        &plan,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &|n| {
            written.borrow_mut().push(n);
        },
    )
    .unwrap();
    set_read_only(&targets[3], false);
    let after = folder.snapshot();

    // The report.
    assert_eq!(report.backup_dir, folder.path("clone-bay-backups/T"));
    assert_eq!((report.written(), report.failed()), (3, 0));
    assert_eq!(*written.borrow(), [1, 2, 3]);
    let outcome = |index: usize| &report.outcomes[index].1;
    assert_eq!(outcome(0), &Outcome::Written);
    assert_eq!(outcome(1), &Outcome::Written);
    assert_eq!(
        outcome(2),
        &Outcome::Skipped("can't decode it (unexpected end of data)".to_owned())
    );
    assert_eq!(
        outcome(3),
        &Outcome::Skipped("the file is read-only".to_owned())
    );
    assert_eq!(outcome(4), &Outcome::Written);
    let paths: Vec<&PathBuf> = report.outcomes.iter().map(|(path, _)| path).collect();
    assert_eq!(paths, targets.iter().collect::<Vec<_>>());

    // Backups: exactly the written targets, byte for byte as they were.
    let backups: Vec<&String> = after
        .keys()
        .filter(|name| name.contains("backup"))
        .collect();
    assert_eq!(
        backups,
        [
            "clone-bay-backups/T/core_user_222.dat",
            "clone-bay-backups/T/core_user_333.dat",
            "clone-bay-backups/T/core_user_666.dat",
        ]
    );
    for name in [
        "core_user_222.dat",
        "core_user_333.dat",
        "core_user_666.dat",
    ] {
        assert_eq!(
            after[&format!("clone-bay-backups/T/{name}")],
            before[name],
            "{name}"
        );
        assert_ne!(
            after[name], before[name],
            "{name} should have been rewritten"
        );
    }

    // Everything that wasn't a written target is untouched, and no temp
    // files are left behind.
    for name in [
        "core_user_111.dat",
        "core_user_444.dat",
        "core_user_555.dat",
        "core_char_1.dat",
        "notes.txt",
    ] {
        assert_eq!(after[name], before[name], "{name}");
    }
    assert_eq!(after.len(), before.len() + 3);

    // Each written file: the source's settings, its own layout.
    let source_settings = strip_protected(&account(1, Some(&[10, 11])), PROTECTED).unwrap();
    for (name, own) in [
        ("core_user_222.dat", account(2, Some(&[20, 21, 22]))),
        ("core_user_333.dat", account(3, None)),
        ("core_user_666.dat", account(6, Some(&[60]))),
    ] {
        let now = folder.value(name);
        assert_eq!(
            strip_protected(&now, PROTECTED).unwrap(),
            source_settings,
            "{name}"
        );
        assert_eq!(
            protected_parts(&now, PROTECTED).unwrap(),
            protected_parts(&own, PROTECTED).unwrap(),
            "{name}"
        );
    }
    // A target with no layout still has none; it did not inherit the source's.
    assert!(
        protected_parts(&folder.value("core_user_333.dat"), PROTECTED)
            .unwrap()
            .is_empty()
    );
    // A target that carried a checksum still does.
    assert!(decode(&after["core_user_666.dat"]).unwrap().had_checksum);
    assert!(!decode(&after["core_user_222.dat"]).unwrap().had_checksum);
}

#[test]
fn character_files_without_module_settings_are_copied_byte_for_byte() {
    let folder = Folder::new();
    let source = folder.put("core_char_1.dat", &character(1, None));
    let plain = folder.put("core_char_2.dat", &character(2, None));
    let with_modules = folder.put("core_char_3.dat", &character(3, Some(&[30, 31])));
    let source_bytes = fs::read(&source).unwrap();

    let plan = plan_copy(&source, &[plain.clone(), with_modules.clone()], &quiet).unwrap();
    assert_eq!(plan.targets[0].action, Action::ExactCopy);
    assert_eq!(
        plan.targets[1].action,
        Action::KeepOwn {
            kept: vec!["auto-repeat settings for 2 modules".to_owned()]
        }
    );
    let report = execute_copy(
        &plan,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &quiet,
    )
    .unwrap();
    assert_eq!(report.written(), 2);

    assert_eq!(fs::read(&plain).unwrap(), source_bytes);
    let kept = folder.value("core_char_3.dat");
    assert_eq!(
        strip_protected(&kept, PROTECTED).unwrap(),
        strip_protected(&character(1, None), PROTECTED).unwrap()
    );
    assert_eq!(
        protected_parts(&kept, PROTECTED).unwrap(),
        protected_parts(&character(3, Some(&[30, 31])), PROTECTED).unwrap()
    );
}

#[test]
fn a_character_source_with_module_settings_never_passes_them_on() {
    let folder = Folder::new();
    let source = folder.put("core_char_1.dat", &character(1, Some(&[10, 11, 12])));
    let target = folder.put("core_char_2.dat", &character(2, None));
    let plan = plan_copy(&source, &[target], &quiet).unwrap();
    // Not an exact copy: the source's module settings must be left out.
    assert_eq!(plan.targets[0].action, Action::KeepOwn { kept: vec![] });
    execute_copy(
        &plan,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &quiet,
    )
    .unwrap();
    let now = folder.value("core_char_2.dat");
    assert!(protected_parts(&now, PROTECTED).unwrap().is_empty());
    assert_eq!(
        now,
        strip_protected(&character(1, None), PROTECTED).unwrap()
    );
}

#[test]
fn nothing_is_written_while_eve_is_running() {
    let (folder, source, targets) = account_folder();
    let plan = plan_copy(&source, &targets, &quiet).unwrap();
    let before = folder.snapshot();
    for confirmed in [false, true] {
        assert_eq!(
            execute_copy(
                &plan,
                EveStatus::Running,
                confirmed,
                &BackupSession::new("T"),
                &quiet
            ),
            Err(ExecuteError::EveRunning)
        );
    }
    assert_eq!(folder.snapshot(), before);
    set_read_only(&targets[3], false);
}

#[test]
fn an_unknown_eve_status_needs_the_user_to_confirm() {
    let (folder, source, targets) = account_folder();
    let plan = plan_copy(&source, &targets, &quiet).unwrap();
    let before = folder.snapshot();
    assert_eq!(
        execute_copy(
            &plan,
            EveStatus::Unknown,
            false,
            &BackupSession::new("T"),
            &quiet
        ),
        Err(ExecuteError::EveUnknown)
    );
    assert_eq!(folder.snapshot(), before);

    let report = execute_copy(
        &plan,
        EveStatus::Unknown,
        true,
        &BackupSession::new("T"),
        &quiet,
    )
    .unwrap();
    assert_eq!(report.written(), 3);
    set_read_only(&targets[3], false);
}

#[test]
fn an_unusable_source_stops_the_plan() {
    let (folder, _source, targets) = account_folder();
    let before = folder.snapshot();

    let damaged = folder.path("core_user_444.dat");
    assert_eq!(
        plan_copy(&damaged, &targets, &quiet),
        Err(PlanError::SourceUnusable {
            file: "core_user_444.dat".to_owned(),
            reason: "can't decode it (unexpected end of data)".to_owned(),
        })
    );
    let missing = folder.path("core_user_999.dat");
    assert!(matches!(
        plan_copy(&missing, &targets, &quiet),
        Err(PlanError::SourceUnusable { .. })
    ));
    assert_eq!(
        plan_copy(&folder.path("notes.txt"), &targets, &quiet),
        Err(PlanError::NotASettingsFile("notes.txt".to_owned()))
    );
    let two_ui = dict(&[(b("ui"), dict(&[])), (b("ui"), dict(&[]))]);
    let odd = folder.put("core_user_777.dat", &two_ui);
    assert_eq!(
        plan_copy(&odd, &targets, &quiet),
        Err(PlanError::SourceUnusable {
            file: "core_user_777.dat".to_owned(),
            reason: "file has more than one 'ui' section".to_owned(),
        })
    );
    fs::remove_file(odd).unwrap();
    assert_eq!(folder.snapshot(), before);
    set_read_only(&targets[3], false);
}

#[test]
fn the_source_and_files_of_the_other_kind_are_never_targets() {
    let (_folder, source, targets) = account_folder();
    let character = targets[0].with_file_name("core_char_1.dat");
    let plan = plan_copy(&source, &[source.clone(), character], &quiet).unwrap();
    let reasons: Vec<&Action> = plan.targets.iter().map(|target| &target.action).collect();
    assert_eq!(
        reasons,
        [
            &Action::Skip {
                reason: "it is the file being copied from".to_owned()
            },
            &Action::Skip {
                reason: "it isn't the same kind of settings file as the source".to_owned()
            },
        ]
    );
    assert_eq!(plan.writable(), 0);
    assert_eq!(
        execute_copy(
            &plan,
            EveStatus::NotRunning,
            false,
            &BackupSession::new("T"),
            &quiet
        ),
        Err(ExecuteError::NothingToWrite)
    );
    set_read_only(&targets[3], false);
}

#[test]
fn a_target_with_an_odd_layout_is_skipped_not_guessed_at() {
    let folder = Folder::new();
    let source = folder.put("core_user_1.dat", &account(1, Some(&[10])));
    let two_ui = dict(&[(b("ui"), dict(&[])), (b("ui"), dict(&[]))]);
    let odd = folder.put("core_user_2.dat", &two_ui);
    let list = folder.put("core_user_3.dat", &Value::List(vec![]));
    let plan = plan_copy(&source, &[odd, list], &quiet).unwrap();
    assert_eq!(
        plan.targets[0].action,
        Action::Skip {
            reason: "file has more than one 'ui' section".to_owned()
        }
    );
    assert_eq!(
        plan.targets[1].action,
        Action::Skip {
            reason: "unexpected structure (top level isn't a dict)".to_owned()
        }
    );
}

#[test]
fn a_target_that_changes_after_the_plan_stops_everything_before_any_write() {
    let (folder, source, targets) = account_folder();
    let plan = plan_copy(&source, &targets, &quiet).unwrap();
    // As if EVE saved its settings while the plan was on screen.
    folder.put("core_user_333.dat", &account(33, Some(&[330])));
    let before = folder.snapshot();

    let result = execute_copy(
        &plan,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &quiet,
    );
    let Err(ExecuteError::Backup(error)) = &result else {
        panic!("{result:?}");
    };
    assert!(error
        .to_string()
        .starts_with("core_user_333.dat changed after it was checked"));
    assert!(result
        .unwrap_err()
        .to_string()
        .ends_with("Nothing was changed"));

    // No settings file changed. Only the backup folder is new.
    let after = folder.snapshot();
    for (name, contents) in &before {
        assert_eq!(&after[name], contents, "{name}");
    }
    assert!(after
        .keys()
        .all(|name| before.contains_key(name) || name.contains("backup")));
    set_read_only(&targets[3], false);
}

#[test]
fn a_source_that_changes_after_the_plan_does_not_change_what_is_written() {
    let folder = Folder::new();
    let source = folder.put("core_user_1.dat", &account(1, Some(&[10])));
    let target = folder.put("core_user_2.dat", &account(2, Some(&[20])));
    let plan = plan_copy(&source, &[target], &quiet).unwrap();
    folder.put("core_user_1.dat", &account(99, Some(&[990])));

    execute_copy(
        &plan,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &quiet,
    )
    .unwrap();
    let now = folder.value("core_user_2.dat");
    assert_eq!(
        strip_protected(&now, PROTECTED).unwrap(),
        strip_protected(&account(1, None), PROTECTED).unwrap()
    );
}

#[test]
fn running_the_same_plan_twice_refuses_the_second_time() {
    // After the first run the targets no longer match what was reviewed.
    let folder = Folder::new();
    let source = folder.put("core_user_1.dat", &account(1, Some(&[10])));
    let target = folder.put("core_user_2.dat", &account(2, Some(&[20])));
    let plan = plan_copy(&source, &[target], &quiet).unwrap();
    execute_copy(
        &plan,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &quiet,
    )
    .unwrap();
    let after_first = folder.snapshot();

    let second = execute_copy(
        &plan,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &quiet,
    );
    assert!(matches!(second, Err(ExecuteError::Backup(_))), "{second:?}");
    let after_second = folder.snapshot();
    for (name, contents) in &after_first {
        assert_eq!(&after_second[name], contents, "{name}");
    }
}

#[test]
fn copying_again_with_a_fresh_plan_gives_the_same_result() {
    let folder = Folder::new();
    let source = folder.put("core_user_1.dat", &account(1, Some(&[10])));
    let target = folder.put("core_user_2.dat", &account(2, Some(&[20])));
    // Two runs of the app.
    for stamp in ["A", "B"] {
        let plan = plan_copy(&source, std::slice::from_ref(&target), &quiet).unwrap();
        let run = BackupSession::new(stamp);
        execute_copy(&plan, EveStatus::NotRunning, false, &run, &quiet).unwrap();
    }
    let files = folder.snapshot();
    // The second backup holds the result of the first copy, which is also
    // what the file holds now: the copy is stable.
    assert_eq!(
        files["clone-bay-backups/B/core_user_2.dat"],
        files["core_user_2.dat"]
    );
    assert_ne!(
        files["clone-bay-backups/A/core_user_2.dat"],
        files["core_user_2.dat"]
    );
}

#[test]
fn copies_made_in_one_run_keep_one_backup_of_each_original() {
    let folder = Folder::new();
    let first = folder.put("core_user_1.dat", &account(1, Some(&[10])));
    let other = folder.put("core_user_3.dat", &account(3, Some(&[30])));
    let target = folder.put("core_user_2.dat", &account(2, Some(&[20])));
    let before = folder.snapshot();
    let run = BackupSession::new("T");

    for source in [&first, &other] {
        let plan = plan_copy(source, std::slice::from_ref(&target), &quiet).unwrap();
        let report = execute_copy(&plan, EveStatus::NotRunning, false, &run, &quiet).unwrap();
        assert_eq!(report.written(), 1);
        assert_eq!(report.backup_dir, folder.path("clone-bay-backups/T"));
    }

    let after = folder.snapshot();
    let backups: Vec<&String> = after
        .keys()
        .filter(|name| name.contains("backup"))
        .collect();
    assert_eq!(backups, ["clone-bay-backups/T/core_user_2.dat"]);
    assert_eq!(
        after["clone-bay-backups/T/core_user_2.dat"],
        before["core_user_2.dat"]
    );
    // The second copy did happen.
    assert_eq!(
        strip_protected(&folder.value("core_user_2.dat"), PROTECTED).unwrap(),
        strip_protected(&account(3, None), PROTECTED).unwrap()
    );
}

#[test]
fn a_target_the_game_changed_between_copies_is_backed_up_again() {
    let folder = Folder::new();
    let source = folder.put("core_user_1.dat", &account(1, Some(&[10])));
    let target = folder.put("core_user_2.dat", &account(2, Some(&[20])));
    let untouched = folder.put("core_user_3.dat", &account(3, Some(&[30])));
    let run = BackupSession::new("T");
    let plan = plan_copy(&source, std::slice::from_ref(&target), &quiet).unwrap();
    execute_copy(&plan, EveStatus::NotRunning, false, &run, &quiet).unwrap();

    // As if EVE was opened and closed while the app stayed open.
    folder.put("core_user_2.dat", &account(22, Some(&[220])));
    let saved_by_game = folder.snapshot();
    let plan = plan_copy(&source, &[untouched, target], &quiet).unwrap();
    let report = execute_copy(&plan, EveStatus::NotRunning, false, &run, &quiet).unwrap();
    assert_eq!(report.written(), 2);

    let after = folder.snapshot();
    assert_eq!(
        after["clone-bay-backups/T/core_user_2.dat"],
        encode(&account(2, Some(&[20])), false).unwrap()
    );
    assert_eq!(
        after["clone-bay-backups/T-2/core_user_2.dat"],
        saved_by_game["core_user_2.dat"]
    );
    assert_eq!(
        after["clone-bay-backups/T/core_user_3.dat"],
        saved_by_game["core_user_3.dat"]
    );
    // The backups are in two folders, so the report names the one above.
    assert_eq!(report.backup_dir, folder.path("clone-bay-backups"));
}

#[test]
fn a_write_that_fails_is_reported_and_the_rest_still_happen() {
    let folder = Folder::new();
    let source = folder.put("core_user_1.dat", &account(1, Some(&[10])));
    let locked = folder.put("core_user_2.dat", &account(2, Some(&[20])));
    let fine = folder.put("core_user_3.dat", &account(3, Some(&[30])));
    let plan = plan_copy(&source, &[locked.clone(), fine], &quiet).unwrap();
    assert_eq!(plan.writable(), 2);
    // Read-only only after the plan, so the plan didn't skip it. On Windows
    // a read-only file can't be replaced.
    set_read_only(&locked, true);
    let before = fs::read(&locked).unwrap();

    let report = execute_copy(
        &plan,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &quiet,
    )
    .unwrap();
    set_read_only(&locked, false);
    if cfg!(windows) {
        assert_eq!((report.written(), report.failed()), (1, 1));
        let Outcome::Failed(reason) = &report.outcomes[0].1 else {
            panic!("{:?}", report.outcomes[0]);
        };
        assert!(reason.starts_with("it couldn't be written ("), "{reason}");
        assert!(reason.ends_with("so it was left untouched"), "{reason}");
        assert_eq!(fs::read(&locked).unwrap(), before);
    }
    assert_eq!(report.outcomes[1].1, Outcome::Written);
    // Both were backed up before either write was attempted.
    let files = folder.snapshot();
    assert!(files.contains_key("clone-bay-backups/T/core_user_2.dat"));
    assert!(files.contains_key("clone-bay-backups/T/core_user_3.dat"));
    assert!(files.keys().all(|name| !name.ends_with(".tmp")));
}

#[test]
fn refusals_say_that_nothing_was_changed() {
    for error in [
        ExecuteError::EveRunning,
        ExecuteError::EveUnknown,
        ExecuteError::NothingToWrite,
    ] {
        assert!(
            error.to_string().ends_with("Nothing was changed"),
            "{error}"
        );
    }
}

fn chosen_groups(ids: &[&str]) -> Scope {
    Scope::Selected(Selection {
        groups: ids.iter().map(|id| (*id).to_owned()).collect(),
        ..Selection::default()
    })
}

/// An account file with keyboard shortcuts as well as the usual sections.
fn account_with_shortcuts(tag: i64, ships: &[i64]) -> Value {
    let Value::Dict(mut sections) = account(tag, Some(ships)) else {
        unreachable!()
    };
    sections.push((
        b("cmd"),
        dict(&[(b("customCmds"), stamped(tag, Value::Int(tag)))]),
    ));
    Value::Dict(sections)
}

fn section<'a>(root: &'a Value, name: &str) -> &'a Value {
    let Value::Dict(sections) = root else {
        panic!("not a dict");
    };
    &sections.iter().find(|(key, _)| *key == b(name)).unwrap().1
}

#[test]
fn a_chosen_group_is_copied_and_the_rest_of_the_target_is_untouched() {
    let folder = Folder::new();
    let source = folder.put("core_user_1.dat", &account_with_shortcuts(1, &[10]));
    let target = folder.put("core_user_2.dat", &account_with_shortcuts(2, &[20, 21]));
    let before = folder.snapshot();

    let scope = chosen_groups(&["keybinds"]);
    let plan = plan_copy_scoped(&source, &[target], scope.clone(), &quiet).unwrap();
    assert_eq!(plan.scope, scope);
    assert_eq!(plan.targets[0].action, Action::Chosen { channels_added: 0 });
    assert_eq!(folder.snapshot(), before, "planning changes nothing");

    let report = execute_copy(
        &plan,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &quiet,
    )
    .unwrap();
    assert_eq!(report.written(), 1);
    let after = folder.snapshot();
    assert_eq!(
        after["clone-bay-backups/T/core_user_2.dat"],
        before["core_user_2.dat"]
    );
    assert_eq!(after["core_user_1.dat"], before["core_user_1.dat"]);

    let now = folder.value("core_user_2.dat");
    let (from_source, own) = (
        account_with_shortcuts(1, &[10]),
        account_with_shortcuts(2, &[20, 21]),
    );
    assert_eq!(section(&now, "cmd"), section(&from_source, "cmd"));
    assert_eq!(section(&now, "overview"), section(&own, "overview"));
    assert_eq!(
        protected_parts(&now, PROTECTED).unwrap(),
        protected_parts(&own, PROTECTED).unwrap()
    );
    // Everything but the shortcuts is exactly as the target had it.
    let mut expected = own.clone();
    let Value::Dict(sections) = &mut expected else {
        unreachable!()
    };
    sections.retain(|(key, _)| *key != b("cmd"));
    sections.push((b("cmd"), section(&from_source, "cmd").clone()));
    assert!(same_settings(&now, &expected, FileKind::Account).unwrap());
}

#[test]
fn a_target_that_already_has_the_chosen_settings_is_skipped() {
    let folder = Folder::new();
    let source = folder.put("core_user_1.dat", &account_with_shortcuts(1, &[10]));
    // Same shortcuts as the source, everything else different.
    let Value::Dict(mut sections) = account(2, Some(&[20])) else {
        unreachable!()
    };
    sections.push((
        b("cmd"),
        dict(&[(b("customCmds"), stamped(1, Value::Int(1)))]),
    ));
    let target = folder.put("core_user_2.dat", &Value::Dict(sections));

    let plan = plan_copy_scoped(&source, &[target], chosen_groups(&["keybinds"]), &quiet).unwrap();
    assert_eq!(
        plan.targets[0].action,
        Action::Skip {
            reason: "it already has these settings".to_owned()
        }
    );
    assert_eq!(
        execute_copy(
            &plan,
            EveStatus::NotRunning,
            false,
            &BackupSession::new("T"),
            &quiet
        ),
        Err(ExecuteError::NothingToWrite)
    );
}

#[test]
fn choosing_nothing_is_refused_before_any_file_is_read() {
    let folder = Folder::new();
    let missing = folder.path("core_user_1.dat");
    let empty = Scope::Selected(Selection::default());
    assert_eq!(
        plan_copy_scoped(&missing, &[], empty, &quiet),
        Err(PlanError::NothingChosen)
    );
    assert_eq!(
        PlanError::NothingChosen.to_string(),
        "nothing was chosen to copy"
    );
    // "Everything" is never empty.
    assert!(matches!(
        plan_copy_scoped(&missing, &[], Scope::Everything, &quiet),
        Err(PlanError::SourceUnusable { .. })
    ));
}

#[test]
fn chat_channels_are_added_and_counted_in_the_plan() {
    let folder = Folder::new();
    let channel = |kind: &str, id: &str| {
        Value::Tuple(vec![
            b(kind),
            Value::Str(id.to_owned()),
            Value::Str(id.to_uppercase()),
        ])
    };
    let with_channels = |tag: i64, channels: Vec<Value>| {
        let Value::Dict(mut sections) = character(tag, Some(&[tag])) else {
            unreachable!()
        };
        let chat = (b("chatchannels"), stamped(tag, Value::List(channels)));
        sections.push((b("ui"), dict(&[chat])));
        Value::Dict(sections)
    };
    let source = folder.put(
        "core_char_1.dat",
        &with_channels(
            1,
            vec![
                channel("corp", "corp_1"),
                channel("player_a", "player_a"),
                channel("player_b", "player_b"),
            ],
        ),
    );
    let target = folder.put(
        "core_char_2.dat",
        &with_channels(2, vec![channel("corp", "corp_2")]),
    );
    let listed: Vec<String> = source_chat_channels(&source)
        .unwrap()
        .into_iter()
        .map(|channel| channel.id)
        .collect();
    assert_eq!(listed, ["corp_1", "player_a", "player_b"]);
    assert!(source_chat_channels(&folder.path("core_char_9.dat")).is_err());

    let scope = Scope::Selected(Selection {
        chat_channels: Some(["player_a", "player_b"].map(str::to_owned).into()),
        ..Selection::default()
    });
    let plan = plan_copy_scoped(&source, &[target], scope, &quiet).unwrap();
    assert_eq!(plan.targets[0].action, Action::Chosen { channels_added: 2 });
    execute_copy(
        &plan,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &quiet,
    )
    .unwrap();

    let now = folder.value("core_char_2.dat");
    let ids: Vec<String> = chat_channels(&now)
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(ids, ["corp_2", "player_a", "player_b"]);
    // Its window layout and module settings are still its own.
    assert_eq!(
        section(&now, "windows"),
        section(&character(2, Some(&[2])), "windows")
    );
    // The chat channel list is protected too, and is the one protected part
    // this copy was asked to add to.
    let apart_from_chat = |value: &Value| {
        let mut parts = protected_parts(value, PROTECTED).unwrap();
        parts.retain(|part| part.key != Some(b("chatchannels")));
        parts
    };
    assert_eq!(
        apart_from_chat(&now),
        apart_from_chat(&character(2, Some(&[2])))
    );
    assert_eq!(apart_from_chat(&now).len(), 2);
}

/// An account whose overview is set to open on the tab at `selected`.
fn account_on_tab(tag: i64, ships: Option<&[i64]>, selected: i64) -> Value {
    let Value::Dict(mut sections) = account(tag, ships) else {
        panic!("not a dict");
    };
    sections.push((
        b("tabgroups"),
        dict(&[
            (b("overviewTabs"), stamped(tag, Value::Int(selected))),
            // Another window's selected tab, which a copy leaves alone.
            (b("marketTabs"), stamped(tag, Value::Int(7))),
        ]),
    ));
    Value::Dict(sections)
}

#[test]
fn a_copy_never_passes_on_the_tab_the_source_overview_was_left_on() {
    let folder = Folder::new();
    let source = folder.put("core_user_1.dat", &account_on_tab(1, Some(&[10]), 1));
    let source_bytes = fs::read(&source).unwrap();
    let with_layout = folder.put("core_user_2.dat", &account_on_tab(2, Some(&[20]), 0));
    let plan = plan_copy(&source, &[with_layout], &quiet).unwrap();
    assert!(plan.resets_overview_tab());
    let report = execute_copy(
        &plan,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &quiet,
    )
    .unwrap();
    assert_eq!(report.outcomes[0].1, Outcome::Written);

    // The target has the source's settings, opening on the first tab; the
    // setting keeps the source's timestamp, and other tabs are as copied.
    let written = folder.value("core_user_2.dat");
    assert_eq!(
        section(&written, "tabgroups"),
        &dict(&[
            (b("overviewTabs"), stamped(1, Value::Int(0))),
            (b("marketTabs"), stamped(1, Value::Int(7))),
        ])
    );
    assert_eq!(
        strip_protected(&written, PROTECTED).unwrap(),
        strip_protected(&account_on_tab(1, None, 0), PROTECTED).unwrap()
    );
    assert_eq!(
        protected_parts(&written, PROTECTED).unwrap(),
        protected_parts(&account_on_tab(2, Some(&[20]), 0), PROTECTED).unwrap()
    );
    // The source file itself is never changed.
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
}

#[test]
fn a_source_on_another_tab_is_not_copied_byte_for_byte() {
    // Neither file has anything protected, so this would be an exact copy.
    let folder = Folder::new();
    let source = folder.put("core_user_1.dat", &account_on_tab(1, None, 3));
    let target = folder.put("core_user_2.dat", &account(2, None));
    let plan = plan_copy(&source, &[target], &quiet).unwrap();
    assert_eq!(plan.targets[0].action, Action::KeepOwn { kept: vec![] });
    execute_copy(
        &plan,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &quiet,
    )
    .unwrap();
    assert_eq!(folder.value("core_user_2.dat"), account_on_tab(1, None, 0));

    // A source already on its first tab still is.
    let folder = Folder::new();
    let source = folder.put("core_user_1.dat", &account_on_tab(1, None, 0));
    let target = folder.put("core_user_2.dat", &account(2, None));
    let plan = plan_copy(&source, &[target], &quiet).unwrap();
    assert!(!plan.resets_overview_tab());
    assert_eq!(plan.targets[0].action, Action::ExactCopy);
    execute_copy(
        &plan,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &quiet,
    )
    .unwrap();
    assert_eq!(
        fs::read(folder.path("core_user_2.dat")).unwrap(),
        fs::read(&source).unwrap()
    );
}

#[test]
fn a_group_copy_resets_the_overview_tab_only_when_it_copies_the_selected_tabs() {
    let folder = Folder::new();
    let source = folder.put("core_user_1.dat", &account_on_tab(1, Some(&[10]), 1));
    let target = folder.put("core_user_2.dat", &account_on_tab(2, Some(&[20]), 4));

    // "Overview" alone leaves the target's own selected tab as it was.
    let plan = plan_copy_scoped(
        &source,
        std::slice::from_ref(&target),
        chosen_groups(&["overview"]),
        &quiet,
    )
    .unwrap();
    assert!(!plan.resets_overview_tab());
    execute_copy(
        &plan,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &quiet,
    )
    .unwrap();
    assert_eq!(
        section(&folder.value("core_user_2.dat"), "tabgroups"),
        section(&account_on_tab(2, None, 4), "tabgroups")
    );

    // With the selected tabs copied too, it opens on the first.
    let plan = plan_copy_scoped(&source, &[target], chosen_groups(&["tabgroups"]), &quiet).unwrap();
    assert!(plan.resets_overview_tab());
    execute_copy(
        &plan,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &quiet,
    )
    .unwrap();
    assert_eq!(
        section(&folder.value("core_user_2.dat"), "tabgroups"),
        section(&account_on_tab(1, None, 0), "tabgroups")
    );
}

#[test]
fn a_character_copy_is_not_changed_by_a_selected_tab_setting() {
    let with_tabs = |tag: i64| {
        let Value::Dict(mut sections) = character(tag, None) else {
            panic!("not a dict");
        };
        sections.push((
            b("tabgroups"),
            dict(&[(b("overviewTabs"), stamped(tag, Value::Int(1)))]),
        ));
        Value::Dict(sections)
    };
    let folder = Folder::new();
    let source = folder.put("core_char_1.dat", &with_tabs(1));
    let target = folder.put("core_char_2.dat", &character(2, None));
    let plan = plan_copy(&source, &[target], &quiet).unwrap();
    assert!(!plan.resets_overview_tab());
    assert_eq!(plan.targets[0].action, Action::ExactCopy);
}
