//! Checks against real settings files in `fixtures/real/` (not committed).
//!
//! Ignored by default so a normal test run doesn't depend on private files.
//! Run with:
//!
//!     cargo test -p clone-bay-core --test real_files -- --ignored --nocapture
//!
//! Unlike a skipped test, this fails loudly when the files are missing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use clone_bay_core::discovery::{list_settings_files, FileKind};
use clone_bay_core::inspect::{read_settings_bytes, summarize};
use clone_bay_core::safe_write::{BackupSession, Fingerprint};
use clone_bay_marshal::{decode, encode};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/real")
}

#[test]
#[ignore = "needs real settings files in fixtures/real/"]
fn real_files_decode_cleanly_and_survive_a_rewrite() {
    let folder = fixtures();
    let files = list_settings_files(&folder).unwrap();
    assert!(
        files.iter().any(|file| file.kind == FileKind::Character)
            && files.iter().any(|file| file.kind == FileKind::Account),
        "put one core_char_*.dat and one core_user_*.dat in {}",
        folder.display()
    );

    for file in files {
        let name = file
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let bytes = read_settings_bytes(&file.path).unwrap();
        let summary = summarize(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!summary.sections.is_empty(), "{name}: no sections");
        assert!(
            summary.sections.iter().any(|section| section.name == "ui"),
            "{name}: no 'ui' section"
        );

        // Writing the file out and reading it back must give the same data,
        // and writing that again must give the same bytes.
        let original = decode(&bytes).unwrap();
        let rewritten = encode(&original.value, original.had_checksum).unwrap();
        let reread = decode(&rewritten).unwrap();
        assert_eq!(
            reread.value, original.value,
            "{name}: data changed on rewrite"
        );
        assert_eq!(reread.trailing_bytes, 0, "{name}");
        assert_eq!(
            encode(&reread.value, reread.had_checksum).unwrap(),
            rewritten,
            "{name}"
        );

        // The blue-marshal crate (what EVE Wrench writes with) must produce
        // the same bytes from the same file. The one legitimate difference
        // would be a negative zero, whose sign only this codec keeps.
        let theirs = blue_marshal::decode(&bytes).unwrap();
        let options = blue_marshal::EncodeOptions {
            checksum: theirs.had_crc,
            ..Default::default()
        };
        let theirs = blue_marshal::encode(&theirs.value, &options).unwrap();
        assert!(
            theirs == rewritten,
            "{name}: output differs from blue-marshal"
        );

        println!(
            "{name}: {} bytes, format v{}, {} sections, rewrites to {} bytes",
            bytes.len(),
            summary.format_version,
            summary.sections.len(),
            rewritten.len()
        );
    }
}

/// Copies each real file onto a second copy of itself in a temp folder
/// (never in `fixtures/real/`) whose protected data has been changed, and
/// checks the result with the `blue-marshal` crate as an independent reader.
#[test]
#[ignore = "needs real settings files in fixtures/real/"]
fn real_files_copy_with_protection() {
    use clone_bay_core::copy::{execute_copy, plan_copy, Action};
    use clone_bay_core::platform::EveStatus;
    use clone_bay_core::protect::{protected_parts, strip_protected, PROTECTED};
    use clone_bay_marshal::Value;

    let files = list_settings_files(&fixtures()).unwrap();
    assert!(
        !files.is_empty(),
        "no settings files in {}",
        fixtures().display()
    );
    for file in files {
        let prefix = match file.kind {
            FileKind::Character => "core_char",
            FileKind::Account => "core_user",
        };
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join(format!("{prefix}_1.dat"));
        let target = dir.path().join(format!("{prefix}_2.dat"));
        let bytes = read_settings_bytes(&file.path).unwrap();
        std::fs::write(&source, &bytes).unwrap();

        // The target: the same file with its first section dropped, so its
        // settings differ from the source's while its protected data is its own.
        let Value::Dict(mut sections) = decode(&bytes).unwrap().value else {
            panic!("top level isn't a dict");
        };
        let original_root = Value::Dict(sections.clone());
        let dropped = sections
            .iter()
            .position(|(_, value)| matches!(value, Value::Dict(inner) if !inner.is_empty()))
            .unwrap();
        let dropped_is_protected =
            protected_parts(&Value::Dict(vec![sections[dropped].clone()]), PROTECTED)
                .unwrap()
                .iter()
                .any(|part| part.key.is_none());
        sections.remove(dropped);
        let target_root = Value::Dict(sections);
        std::fs::write(&target, encode(&target_root, false).unwrap()).unwrap();

        let started = std::time::Instant::now();
        let plan = plan_copy(&source, std::slice::from_ref(&target), &|_| {}).unwrap();
        let planned = started.elapsed();
        assert!(
            matches!(plan.targets[0].action, Action::KeepOwn { .. }),
            "{:?}",
            plan.targets[0]
        );
        let report = execute_copy(
            &plan,
            EveStatus::NotRunning,
            false,
            &BackupSession::new("T"),
            &|_| {},
        )
        .unwrap();
        assert_eq!(report.written(), 1);

        let written = std::fs::read(&target).unwrap();
        let now = decode(&written).unwrap().value;
        assert_eq!(
            strip_protected(&now, PROTECTED).unwrap(),
            strip_protected(&original_root, PROTECTED).unwrap()
        );
        // The same protected items, whatever order the copy stores them
        // in: a whole protected section is written after the others.
        let in_one_order = |root: &Value| {
            let mut parts = protected_parts(root, PROTECTED).unwrap();
            parts.sort_by_key(|part| format!("{} {:?}", part.section, part.key));
            parts
        };
        assert_eq!(in_one_order(&now), in_one_order(&target_root));
        if dropped_is_protected {
            // The dropped section was a protected one, so the target had
            // none and must still have none.
            assert!(
                protected_parts(&now, PROTECTED).unwrap().len()
                    < protected_parts(&original_root, PROTECTED).unwrap().len()
            );
        }
        // An independent reader accepts the file and agrees on its bytes.
        let theirs = blue_marshal::decode(&written).unwrap();
        // (The crate's default options add a checksum; keep the file's own choice.)
        let options = blue_marshal::EncodeOptions {
            checksum: theirs.had_crc,
            ..Default::default()
        };
        let theirs = blue_marshal::encode(&theirs.value, &options).unwrap();
        if theirs != written {
            let at = theirs.iter().zip(&written).position(|(a, b)| a != b);
            panic!(
                "output differs from blue-marshal: lengths {} vs {}, first difference at {at:?}: {:02X?} vs {:02X?}",
                theirs.len(),
                written.len(),
                at.map(|at| &theirs[at.saturating_sub(24)..(at + 8).min(theirs.len())]),
                at.map(|at| &written[at.saturating_sub(24)..(at + 8).min(written.len())]),
            );
        }

        println!(
            "{prefix}: {:?} -> planned in {planned:?}, total {:?}, kept: {:?}",
            plan.targets[0].action,
            started.elapsed(),
            protected_parts(&now, PROTECTED).unwrap().len()
        );
    }
}

/// Copies a whole profile's worth of files, one source onto all the others
/// of its kind, and checks every result.
///
/// Set `CLONE_BAY_BULK_DIR` to a folder of settings files. Point it at a
/// *copy* of a profile. The files are copied into a temp folder first and
/// only those copies are written, so the folder given is only ever read.
#[test]
#[ignore = "needs CLONE_BAY_BULK_DIR set to a folder of settings files"]
fn bulk_copy_of_a_whole_profile() {
    use clone_bay_core::copy::{execute_copy, plan_copy, Action, Outcome};
    use clone_bay_core::platform::EveStatus;
    use clone_bay_core::protect::{protected_parts, strip_protected, PROTECTED};

    let given = std::env::var_os("CLONE_BAY_BULK_DIR").expect("CLONE_BAY_BULK_DIR not set");
    let dir = tempfile::tempdir().unwrap();
    let originals = list_settings_files(std::path::Path::new(&given)).unwrap();
    assert!(originals.len() >= 2, "need at least two settings files");
    for file in &originals {
        std::fs::copy(&file.path, dir.path().join(file.path.file_name().unwrap())).unwrap();
    }

    for kind in [FileKind::Character, FileKind::Account] {
        let files: Vec<PathBuf> = list_settings_files(dir.path())
            .unwrap()
            .into_iter()
            .filter(|file| file.kind == kind)
            .map(|file| file.path)
            .collect();
        let Some((source, targets)) = files.split_first() else {
            continue;
        };
        let source_root = decode(&read_settings_bytes(source).unwrap()).unwrap().value;
        // A copy carries the source's settings with one difference: the
        // overview opens on its first tab, whatever tab the source was on.
        let source_root = match kind {
            FileKind::Account => {
                clone_bay_core::tab_order::opening_on_first_tab(&source_root).unwrap_or(source_root)
            }
            FileKind::Character => source_root,
        };
        let source_tabs = clone_bay_core::tab_order::read(&source_root, kind).unwrap();
        let source_settings = strip_protected(&source_root, PROTECTED).unwrap();

        let started = std::time::Instant::now();
        let plan = plan_copy(source, targets, &|_| {}).unwrap();
        let planned = started.elapsed();
        let (mut exact, mut keep, mut skip) = (0, 0, 0);
        for target in &plan.targets {
            match &target.action {
                Action::ExactCopy => exact += 1,
                Action::KeepOwn { .. } | Action::Chosen { .. } => keep += 1,
                Action::Skip { reason } => {
                    skip += 1;
                    println!("  skipped {}: {reason}", target.path.display());
                }
            }
        }
        let stamp = format!("{kind:?}");
        let started = std::time::Instant::now();
        let report = execute_copy(
            &plan,
            EveStatus::NotRunning,
            false,
            &BackupSession::new(stamp),
            &|_| {},
        )
        .unwrap();
        let executed = started.elapsed();
        assert_eq!(report.failed(), 0);
        assert_eq!(report.written(), exact + keep);

        for (path, outcome) in &report.outcomes {
            if *outcome != Outcome::Written {
                continue;
            }
            let name = path.file_name().unwrap();
            let before = read_settings_bytes(&report.backup_dir.join(name)).unwrap();
            let before = decode(&before).unwrap().value;
            let now = decode(&read_settings_bytes(path).unwrap()).unwrap();
            assert_eq!(now.trailing_bytes, 0);
            assert!(
                strip_protected(&now.value, PROTECTED).unwrap() == source_settings,
                "{name:?}: settings differ from the source"
            );
            if kind == FileKind::Account && source_tabs.opens_on.is_some() {
                let tabs = clone_bay_core::tab_order::read(&now.value, kind).unwrap();
                assert_eq!(tabs.opens_on, Some(0), "{name:?}: not on its first tab");
            }
            // The same protected items, whatever order the copy stores
            // them in.
            let in_one_order = |root: &clone_bay_marshal::Value| {
                let mut parts = protected_parts(root, PROTECTED).unwrap();
                parts.sort_by_key(|part| format!("{} {:?}", part.section, part.key));
                parts
            };
            assert!(
                in_one_order(&now.value) == in_one_order(&before),
                "{name:?}: protected data differs from its own original"
            );
        }
        println!(
            "{kind:?}: {} targets ({exact} exact, {keep} keeping their own, {skip} skipped); \
             planned in {planned:.1?}, copied in {executed:.1?}",
            targets.len()
        );
    }
}

/// Copies chosen groups between two copies of each real file in a temp
/// folder. The target is the real file with exactly the chosen groups
/// removed, so after the copy it must hold the same settings as the
/// original again.
#[test]
#[ignore = "needs real settings files in fixtures/real/"]
fn real_files_copy_chosen_groups() {
    use clone_bay_core::copy::{execute_copy, plan_copy_scoped, Action};
    use clone_bay_core::groups::{chat_channels, same_settings, Scope, Selection};
    use clone_bay_core::platform::EveStatus;
    use clone_bay_marshal::Value;

    let files = list_settings_files(&fixtures()).unwrap();
    assert!(
        !files.is_empty(),
        "no settings files in {}",
        fixtures().display()
    );
    for file in files {
        let bytes = read_settings_bytes(&file.path).unwrap();
        let original = decode(&bytes).unwrap().value;
        let Value::Dict(mut sections) = original.clone() else {
            panic!("top level isn't a dict");
        };
        let is = |key: &Value, name: &str| *key == Value::Bytes(name.as_bytes().to_vec());
        let ui_keys_dropped = |sections: &mut Vec<(Value, Value)>, drop: &dyn Fn(&str) -> bool| {
            let (_, ui) = sections.iter_mut().find(|(key, _)| is(key, "ui")).unwrap();
            let Value::Dict(ui) = ui else {
                panic!("ui isn't a dict");
            };
            let before = ui.len();
            ui.retain(|(key, _)| match key {
                Value::Bytes(name) => !drop(&String::from_utf8_lossy(name)),
                Value::Str(name) => !drop(name),
                _ => true,
            });
            before - ui.len()
        };

        let (prefix, selection, expect_channels) = match file.kind {
            FileKind::Account => {
                sections.retain(|(key, _)| !is(key, "cmd"));
                let dropped =
                    ui_keys_dropped(&mut sections, &|name| name.starts_with("probescanning."));
                assert!(
                    dropped > 0,
                    "the account file has no probe scanning settings"
                );
                let selection = Selection {
                    groups: ["keybinds", "probes"].map(str::to_owned).into(),
                    ..Selection::default()
                };
                ("core_user", selection, 0)
            }
            FileKind::Character => {
                sections.retain(|(key, _)| !is(key, "windows"));
                // Remove the copyable chat channels from the target.
                let copyable: Vec<String> = chat_channels(&original)
                    .unwrap()
                    .into_iter()
                    .filter(|channel| channel.can_be_copied())
                    .map(|channel| channel.id)
                    .collect();
                let (_, ui) = sections.iter_mut().find(|(key, _)| is(key, "ui")).unwrap();
                let Value::Dict(ui) = ui else {
                    panic!("ui isn't a dict");
                };
                for (key, value) in ui.iter_mut() {
                    if !is(key, "chatchannels") {
                        continue;
                    }
                    let Value::Tuple(pair) = value else {
                        panic!("chatchannels isn't a pair");
                    };
                    let Value::List(entries) = &mut pair[1] else {
                        panic!("chatchannels isn't a list");
                    };
                    entries.retain(|entry| match entry {
                        Value::Tuple(fields) => {
                            !matches!(&fields[1], Value::Str(id) if copyable.contains(id))
                        }
                        _ => true,
                    });
                }
                let selection = Selection {
                    groups: ["windows".to_owned()].into(),
                    chat_channels: Some(copyable.iter().cloned().collect()),
                    ..Selection::default()
                };
                ("core_char", selection, copyable.len())
            }
        };
        let target_root = Value::Dict(sections);
        assert!(!same_settings(&target_root, &original, file.kind).unwrap());

        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join(format!("{prefix}_1.dat"));
        let target = dir.path().join(format!("{prefix}_2.dat"));
        std::fs::write(&source, &bytes).unwrap();
        std::fs::write(&target, encode(&target_root, false).unwrap()).unwrap();

        let scope = Scope::Selected(selection);
        let plan =
            plan_copy_scoped(&source, std::slice::from_ref(&target), scope, &|_| {}).unwrap();
        assert_eq!(
            plan.targets[0].action,
            Action::Chosen {
                channels_added: expect_channels
            }
        );
        let report = execute_copy(
            &plan,
            EveStatus::NotRunning,
            false,
            &BackupSession::new("T"),
            &|_| {},
        )
        .unwrap();
        assert_eq!(report.written(), 1);

        let written = std::fs::read(&target).unwrap();
        let now = decode(&written).unwrap().value;
        assert!(
            same_settings(&now, &original, file.kind).unwrap(),
            "{prefix}: the result differs from the original"
        );
        let theirs = blue_marshal::decode(&written).unwrap();
        let options = blue_marshal::EncodeOptions {
            checksum: theirs.had_crc,
            ..Default::default()
        };
        let theirs = blue_marshal::encode(&theirs.value, &options).unwrap();
        assert!(
            theirs == written,
            "{prefix}: output differs from blue-marshal"
        );
        println!("{prefix}: chosen groups copied, {expect_channels} chat channel(s) added");
    }
}

/// Every setting the editor offers must be readable in the real files: its
/// key and stored type were taken from these files, so "unreadable" would
/// mean the definition is wrong. Also edits one setting in a temp copy.
#[test]
#[ignore = "needs real settings files in fixtures/real/"]
fn real_files_settings_are_readable_and_editable() {
    use clone_bay_core::edit::{read, save, settings_for, Change, Content, Reading};
    use clone_bay_core::platform::EveStatus;
    use clone_bay_marshal::Long;

    let files = list_settings_files(&fixtures()).unwrap();
    assert!(
        !files.is_empty(),
        "no settings files in {}",
        fixtures().display()
    );
    for file in files {
        let bytes = read_settings_bytes(&file.path).unwrap();
        let root = decode(&bytes).unwrap().value;
        let mut stored = 0;
        let mut first_switch = None;
        for setting in settings_for(file.kind) {
            match read(&root, setting) {
                Reading::Stored(content) => {
                    stored += 1;
                    if let (None, Content::Switch(on)) = (&first_switch, &content) {
                        first_switch = Some((setting, *on));
                    }
                }
                Reading::NotStored => println!("  not stored in this file: {}", setting.label),
                Reading::Unreadable => panic!("{:?}: '{}' is unreadable", file.kind, setting.label),
            }
        }
        println!(
            "{:?}: {stored} of {} settings stored",
            file.kind,
            settings_for(file.kind).len()
        );
        assert!(stored > 0);

        // Flip one switch in a temp copy and read it back.
        let (setting, on) = first_switch.expect("no on/off setting stored");
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join(file.path.file_name().unwrap());
        std::fs::write(&copy, &bytes).unwrap();
        let changes = [Change {
            setting: setting.id,
            content: Content::Switch(!on),
        }];
        let now = Long::from_i64(134_000_000_000_000_000);
        save(
            &copy,
            Fingerprint::of(&std::fs::read(&copy).unwrap()),
            &changes,
            EveStatus::NotRunning,
            false,
            &BackupSession::new("T"),
            &now,
        )
        .unwrap();
        let written = std::fs::read(&copy).unwrap();
        let edited = decode(&written).unwrap().value;
        assert_eq!(
            read(&edited, setting),
            Reading::Stored(Content::Switch(!on))
        );
        let theirs = blue_marshal::decode(&written).unwrap();
        let options = blue_marshal::EncodeOptions {
            checksum: theirs.had_crc,
            ..Default::default()
        };
        let theirs = blue_marshal::encode(&theirs.value, &options).unwrap();
        assert!(theirs == written, "output differs from blue-marshal");
    }
}

/// The real account file's probe formations must read, be judged valid, and
/// rebuild to exactly what is stored. Then one is added in a temp copy.
#[test]
#[ignore = "needs real settings files in fixtures/real/"]
fn real_file_formations_are_readable_and_editable() {
    use clone_bay_core::formations::{differs, read, save, Preset};
    use clone_bay_core::platform::EveStatus;
    use clone_bay_marshal::Long;

    let files = list_settings_files(&fixtures()).unwrap();
    let account = files
        .iter()
        .find(|file| file.kind == FileKind::Account)
        .expect("put a core_user_*.dat in fixtures/real/");
    let bytes = read_settings_bytes(&account.path).unwrap();
    let root = decode(&bytes).unwrap().value;
    let mut formations = read(&root).unwrap();
    formations.validate().unwrap();
    assert!(
        !differs(&root, &formations).unwrap(),
        "rebuilding the stored formations changed them"
    );
    println!(
        "{} formations, in use: {:?}",
        formations.list().len(),
        formations.selected
    );
    for formation in formations.list() {
        println!(
            "  {:>3} {:<16} {} probes, launch shift {:.1} km",
            formation.id,
            formation.name,
            formation.probes.len(),
            formation.launch_shift() / 1000.0
        );
    }

    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("core_user_1.dat");
    std::fs::write(&copy, &bytes).unwrap();
    let id = formations.add("Added by test", Preset::Pinpoint.probes());
    let now = Long::from_i64(134_000_000_000_000_000);
    save(
        &copy,
        Fingerprint::of(&std::fs::read(&copy).unwrap()),
        &formations,
        EveStatus::NotRunning,
        false,
        &BackupSession::new("T"),
        &now,
    )
    .unwrap();
    let written = std::fs::read(&copy).unwrap();
    let reread = read(&decode(&written).unwrap().value).unwrap();
    assert_eq!(reread, formations);
    assert_eq!(reread.get(id).unwrap().probes.len(), 8);
    let theirs = blue_marshal::decode(&written).unwrap();
    let options = blue_marshal::EncodeOptions {
        checksum: theirs.had_crc,
        ..Default::default()
    };
    let theirs = blue_marshal::encode(&theirs.value, &options).unwrap();
    assert!(theirs == written, "output differs from blue-marshal");
}

/// The real files' tab orders must read, and rebuilding them unchanged must
/// write nothing. Then each is reordered in a temp copy.
#[test]
#[ignore = "needs real settings files in fixtures/real/"]
fn real_file_tab_orders_are_readable_and_editable() {
    use clone_bay_core::platform::EveStatus;
    use clone_bay_core::tab_order::{differs, read, read_file, save};
    use clone_bay_marshal::Long;

    for file in list_settings_files(&fixtures()).unwrap() {
        let bytes = read_settings_bytes(&file.path).unwrap();
        let root = decode(&bytes).unwrap().value;
        let mut layout = read(&root, file.kind).unwrap();
        assert!(
            !differs(&root, file.kind, &layout).unwrap(),
            "rebuilding the stored tab order changed it"
        );
        println!("{:?}: opens on {:?}", file.kind, layout.opens_on);
        for group in &layout.groups {
            let tabs: Vec<&str> = group.tabs.iter().map(|tab| tab.label.as_str()).collect();
            println!("  {}: {}", group.title, tabs.join(" | "));
        }
        let Some(group) = layout.groups.iter().position(|group| group.tabs.len() > 1) else {
            panic!("{:?}: no window with two tabs to reorder", file.kind);
        };

        let dir = tempfile::tempdir().unwrap();
        let name = match file.kind {
            FileKind::Account => "core_user_1.dat",
            FileKind::Character => "core_char_1.dat",
        };
        let copy = dir.path().join(name);
        std::fs::write(&copy, &bytes).unwrap();
        layout.move_down(group, 0);
        save(
            &copy,
            Fingerprint::of(&std::fs::read(&copy).unwrap()),
            &layout,
            EveStatus::NotRunning,
            false,
            &BackupSession::new("T"),
            &Long::from_i64(134_000_000_000_000_000),
        )
        .unwrap();
        assert_eq!(read_file(&copy).unwrap(), layout);
    }
}
