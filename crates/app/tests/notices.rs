//! Keeps `THIRD-PARTY-NOTICES.txt` from going stale without anyone noticing.
//!
//! The file is written by `cargo about` (the command is at the top of
//! `about.toml`) and committed, and the release build attaches it as it is.
//! The release build also runs these tests, so a notices file that names a
//! library version the program is no longer built with stops a release.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn root_file(name: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::read_to_string(root.join(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// Every `name version` pair in `Cargo.lock`.
fn locked() -> BTreeSet<(String, String)> {
    let lock = root_file("Cargo.lock").replace("\r\n", "\n");
    let quoted = |line: &str, key: &str| {
        let value = line.strip_prefix(key)?.strip_prefix(" = \"")?;
        Some(value.strip_suffix('"')?.to_owned())
    };
    let mut packages = BTreeSet::new();
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if let Some(name) = quoted(line, "name") {
            let version = lines.next().and_then(|line| quoted(line, "version"));
            packages.insert((name, version.expect("a version follows each name")));
        }
    }
    packages
}

/// Every `name version` pair the notices file lists under a "Used by:".
fn credited() -> BTreeSet<(String, String)> {
    let notices = root_file("THIRD-PARTY-NOTICES.txt").replace("\r\n", "\n");
    let mut libraries = BTreeSet::new();
    let mut in_list = false;
    for line in notices.lines() {
        if line == "Used by:" {
            in_list = true;
        } else if in_list && line.starts_with("  ") {
            let mut words = line.split_whitespace();
            let name = words.next().expect("a name").to_owned();
            libraries.insert((name, words.next().expect("a version").to_owned()));
        } else {
            in_list = false;
        }
    }
    libraries
}

#[test]
fn every_library_credited_is_one_the_program_is_built_with() {
    let locked = locked();
    let credited = credited();
    assert!(
        locked.len() > 100 && credited.len() > 100,
        "nothing was read"
    );
    let stale: Vec<_> = credited.difference(&locked).collect();
    assert!(
        stale.is_empty(),
        "THIRD-PARTY-NOTICES.txt names versions that are not in Cargo.lock; regenerate it \
         (the command is at the top of about.toml): {stale:?}"
    );
}

#[test]
fn the_libraries_the_app_asks_for_by_name_are_credited() {
    let credited: BTreeSet<String> = credited().into_iter().map(|(name, _)| name).collect();
    for library in [
        "eframe",
        "egui",
        "egui_extras",
        "arboard",
        "ureq",
        "thiserror",
        "epaint_default_fonts",
        "clone-bay",
        "clone-bay-core",
        "clone-bay-marshal",
    ] {
        assert!(credited.contains(library), "{library} is not credited");
    }
    // Libraries only the tests use are not part of the program.
    for test_only in ["blue-marshal", "egui_kittest", "tempfile"] {
        assert!(!credited.contains(test_only), "{test_only} is credited");
    }
}

#[test]
fn the_notices_credit_the_work_the_app_builds_on() {
    let notices = root_file("THIRD-PARTY-NOTICES.txt");
    for credit in [
        "trademarks or registered trademarks of\n    CCP hf.",
        "Static Data Export",
        "https://github.com/carbonengine/blue",
        "Copyright (c) 2026 CCP Games",
        "https://github.com/TrueBrain/blue-marshal-rs",
        "Copyright (c) 2026 TrueBrain",
        "https://github.com/eve-wrench/eve-wrench-app",
        "Copyright (c) Tim Kunze",
        "https://github.com/FontaineRiant/EANM",
        "Copyright (c) 2017 Bertral",
        "SIL Open Font License",
        "Ubuntu Font Licence",
    ] {
        assert!(
            notices.replace("\r\n", "\n").contains(credit),
            "missing: {credit}"
        );
    }
}
